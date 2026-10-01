//! A fully scriptable in-process backend.
//!
//! R7's half of the bargain: unit tests use this, integration tests use
//! [`ReplayProvider`](super::ReplayProvider), and neither can reach the
//! network. `reaches_network` is `false` and there is no code path here that
//! could make it true.
//!
//! The script is a queue of [`MockStep`]s, one consumed per call, plus an
//! optional sticky step for "and every call after that". Scripting a
//! *sequence* rather than a single canned answer is what makes the
//! retry/backoff tests possible: `push_error(429); push_error(429);
//! push_content("ok")` is a provider that fails twice and then succeeds, and
//! a throttle test can assert it was called exactly three times.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::ModelError;
use crate::guard::{self, RawCompletion};
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, TokenUsage};

/// What a scripted call does.
#[derive(Debug)]
#[non_exhaustive]
pub enum MockStep {
    /// Return this body verbatim. It still goes through the §10.4 guard, so
    /// handing it truncated JSON produces a real `MalformedJson` rather than
    /// a specially-cased test error.
    Content(String),
    /// Return this body with specific token counts.
    ContentWithUsage(String, TokenUsage),
    /// Fail with this error.
    Fail(ModelError),
    /// Never return. Used to prove the per-request timeout in
    /// [`Throttle`](crate::decorators::Throttle) actually bounds a call.
    Hang,
    /// Return a body of exactly this many bytes, to exercise the response
    /// ceiling without a real provider.
    Oversized(usize),
}

type StickyStep = Box<dyn Fn(&CompletionRequest) -> MockStep + Send + Sync>;

/// A scripted [`ModelProvider`].
pub struct MockProvider {
    script: Mutex<VecDeque<MockStep>>,
    sticky: Option<StickyStep>,
    calls: Mutex<Vec<CompletionRequest>>,
    capabilities: ProviderCapabilities,
    max_response_bytes: usize,
}

impl std::fmt::Debug for MockProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockProvider")
            .field("remaining_steps", &self.script.lock().map(|s| s.len()).ok())
            .field("has_sticky_step", &self.sticky.is_some())
            .field("calls_made", &self.call_count())
            .finish()
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    /// An empty script. A call against it is
    /// [`ModelError::MockScriptExhausted`] — calling more times than a test
    /// scripted is a test bug, and saying so beats inventing an answer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
            sticky: None,
            calls: Mutex::new(Vec::new()),
            capabilities: ProviderCapabilities {
                supports_json_schema: true,
                context_window_tokens: 8192,
                tier: ModelTier::Small,
                reaches_network: false,
            },
            max_response_bytes: crate::config::DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// Appends a step.
    #[must_use]
    pub fn push(self, step: MockStep) -> Self {
        self.script
            .lock()
            .expect("MockProvider script poisoned")
            .push_back(step);
        self
    }

    /// Appends a successful body.
    #[must_use]
    pub fn push_content(self, content: impl Into<String>) -> Self {
        self.push(MockStep::Content(content.into()))
    }

    /// Appends a successful body with explicit token counts.
    #[must_use]
    pub fn push_content_with_usage(self, content: impl Into<String>, usage: TokenUsage) -> Self {
        self.push(MockStep::ContentWithUsage(content.into(), usage))
    }

    /// Appends a failure.
    #[must_use]
    pub fn push_error(self, error: ModelError) -> Self {
        self.push(MockStep::Fail(error))
    }

    /// Appends a call that never returns.
    #[must_use]
    pub fn push_hang(self) -> Self {
        self.push(MockStep::Hang)
    }

    /// Appends a call returning `bytes` bytes.
    #[must_use]
    pub fn push_oversized(self, bytes: usize) -> Self {
        self.push(MockStep::Oversized(bytes))
    }

    /// Sets the step used once the queue is empty, so a test can script "and
    /// then it keeps working" without pushing a step per expected call.
    #[must_use]
    pub fn always(
        mut self,
        step: impl Fn(&CompletionRequest) -> MockStep + Send + Sync + 'static,
    ) -> Self {
        self.sticky = Some(Box::new(step));
        self
    }

    /// Shorthand for [`MockProvider::always`] returning a fixed body.
    #[must_use]
    pub fn always_content(self, content: impl Into<String>) -> Self {
        let content = content.into();
        self.always(move |_| MockStep::Content(content.clone()))
    }

    /// Declares the tier this mock stands in for.
    #[must_use]
    pub fn with_tier(mut self, tier: ModelTier) -> Self {
        self.capabilities.tier = tier;
        self
    }

    /// Declares whether this mock honours a JSON schema.
    #[must_use]
    pub fn with_json_schema_support(mut self, supported: bool) -> Self {
        self.capabilities.supports_json_schema = supported;
        self
    }

    /// Sets the response-size ceiling this mock enforces.
    #[must_use]
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    /// How many times [`ModelProvider::complete`] has been called.
    ///
    /// This is the counter A3's exit gate names: the cache test asserts the
    /// wrapped provider was called exactly once across two identical
    /// requests.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .expect("MockProvider call log poisoned")
            .len()
    }

    /// Every request this mock has been handed, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<CompletionRequest> {
        self.calls
            .lock()
            .expect("MockProvider call log poisoned")
            .clone()
    }

    fn next_step(&self, req: &CompletionRequest) -> MockStep {
        let queued = self
            .script
            .lock()
            .expect("MockProvider script poisoned")
            .pop_front();
        match queued {
            Some(step) => step,
            None => match &self.sticky {
                Some(f) => f(req),
                None => MockStep::Fail(ModelError::MockScriptExhausted {
                    calls_made: self.call_count(),
                }),
            },
        }
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Mock
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        // Recorded before dispatch, so a hang still shows up in the call log.
        self.calls
            .lock()
            .expect("MockProvider call log poisoned")
            .push(req.clone());

        let raw = match self.next_step(&req) {
            MockStep::Content(content) => RawCompletion {
                usage: estimated_usage(&req, &content),
                content,
            },
            MockStep::ContentWithUsage(content, usage) => RawCompletion { content, usage },
            MockStep::Fail(error) => return Err(error),
            MockStep::Hang => {
                // Bounded by whoever wrapped this provider in a timeout. A
                // test that awaits an unwrapped hang deserves to hang — that
                // is the condition being modelled.
                std::future::pending::<()>().await;
                unreachable!("std::future::pending never resolves");
            }
            MockStep::Oversized(bytes) => RawCompletion {
                content: "x".repeat(bytes),
                usage: TokenUsage::default(),
            },
        };

        guard::finalize(ProviderId::Mock, &req, raw, self.max_response_bytes)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }
}

/// A plausible, deterministic token count: roughly four characters per
/// token, which is the usual rule of thumb. Deterministic matters more than
/// accurate — a test that asserts on usage must not depend on a tokenizer.
fn estimated_usage(req: &CompletionRequest, content: &str) -> TokenUsage {
    let prompt_chars: usize = req.system_prompt.as_deref().unwrap_or("").len()
        + req.messages.iter().map(|m| m.content.len()).sum::<usize>();
    TokenUsage {
        prompt_eval_count: u32::try_from(prompt_chars / 4).unwrap_or(u32::MAX),
        eval_count: u32::try_from(content.len() / 4).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Message;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag", ModelTier::Small, vec![Message::user("hello there")])
    }

    #[tokio::test]
    async fn a_mock_never_reaches_the_network() {
        assert!(
            !MockProvider::new().capabilities().reaches_network,
            "R7: the unit-test backend must report, mechanically, that it cannot call out"
        );
    }

    #[tokio::test]
    async fn scripted_steps_are_consumed_in_order() {
        let mock = MockProvider::new()
            .push_content("first")
            .push_content("second");
        assert_eq!(mock.complete(req()).await.expect("first").content, "first");
        assert_eq!(
            mock.complete(req()).await.expect("second").content,
            "second"
        );
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn an_exhausted_script_says_so_rather_than_inventing_an_answer() {
        let mock = MockProvider::new().push_content("only one");
        mock.complete(req()).await.expect("scripted");
        let err = mock.complete(req()).await.expect_err("script is spent");
        assert!(
            matches!(err, ModelError::MockScriptExhausted { calls_made: 2 }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_sticky_step_answers_every_call_after_the_queue() {
        let mock = MockProvider::new()
            .push_content("queued")
            .always_content("sticky");
        assert_eq!(mock.complete(req()).await.expect("q").content, "queued");
        for _ in 0..5 {
            assert_eq!(mock.complete(req()).await.expect("s").content, "sticky");
        }
        assert_eq!(mock.call_count(), 6);
    }

    #[tokio::test]
    async fn a_sticky_step_can_depend_on_the_request() {
        let mock = MockProvider::new().always(|r| MockStep::Content(r.model_tag.clone()));
        let mut r = req();
        r.model_tag = "echoed:1b".to_string();
        assert_eq!(mock.complete(r).await.expect("ok").content, "echoed:1b");
    }

    #[tokio::test]
    async fn every_request_is_recorded_for_later_assertions() {
        let mock = MockProvider::new().always_content("ok");
        mock.complete(req()).await.expect("ok");
        let recorded = mock.calls();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].messages[0].content, "hello there");
    }

    #[tokio::test]
    async fn a_scripted_body_still_passes_through_the_trust_boundary() {
        let mock = MockProvider::new().push_content("{\"truncated\": ");
        let err = mock
            .complete(req().with_format_schema(serde_json::json!({"type": "object"})))
            .await
            .expect_err("the guard applies to mocks too");
        assert!(matches!(err, ModelError::MalformedJson { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn token_counts_are_deterministic() {
        let mock = MockProvider::new().push_content("abcdefgh");
        let usage = mock.complete(req()).await.expect("ok").usage;
        assert_eq!(usage.eval_count, 2, "8 chars / 4");
        assert_eq!(
            usage.prompt_eval_count, 2,
            "\"hello there\" is 11 chars / 4"
        );
    }

    #[tokio::test]
    async fn explicit_token_counts_override_the_estimate() {
        let usage = TokenUsage {
            prompt_eval_count: 111,
            eval_count: 222,
        };
        let mock = MockProvider::new().push_content_with_usage("ok", usage);
        assert_eq!(mock.complete(req()).await.expect("ok").usage, usage);
    }
}
