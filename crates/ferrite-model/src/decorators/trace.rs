//! Records every call in the [activity trace](crate::trace).
//!
//! Outermost decorator: it sees each request the caller made, whether it was
//! answered by the model, by the cache, or with an error, and how long the
//! caller waited. It never alters the request or the response.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::error::ModelError;
use crate::provider::{ModelProvider, ProviderCapabilities, ProviderId, TextSink};
use crate::request::CompletionRequest;
use crate::response::CompletionResponse;
use crate::trace::{TraceBackend, TraceEvent, TraceLog};

/// A provider that records every call to a [`TraceLog`].
pub struct Trace<P> {
    inner: P,
    log: Arc<TraceLog>,
}

impl<P> std::fmt::Debug for Trace<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trace").finish_non_exhaustive()
    }
}

impl<P> Trace<P> {
    /// Wraps `inner`, recording into `log`.
    pub fn new(inner: P, log: Arc<TraceLog>) -> Self {
        Self { inner, log }
    }
}

/// The request as the model saw it: system prompt, then each message.
fn request_text(req: &CompletionRequest) -> String {
    let mut text = String::new();
    if let Some(system) = &req.system_prompt {
        text.push_str(&format!(
            "[system prompt v{}]\n{system}\n\n",
            req.system_prompt_version
        ));
    }
    for message in &req.messages {
        text.push_str(&format!("[{:?}]\n{}\n\n", message.role, message.content));
    }
    text
}

#[async_trait]
impl<P: ModelProvider> ModelProvider for Trace<P> {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let (event, started) = start_event(&req);
        let result = self.inner.complete(req).await;
        self.finish(event, started, result)
    }

    async fn complete_streaming(
        &self,
        req: CompletionRequest,
        sink: &TextSink<'_>,
    ) -> Result<CompletionResponse, ModelError> {
        let (event, started) = start_event(&req);
        let result = self.inner.complete_streaming(req, sink).await;
        self.finish(event, started, result)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

fn start_event(req: &CompletionRequest) -> (TraceEvent, Instant) {
    let stage = if req.label.is_empty() {
        format!("{:?} tier", req.tier)
    } else {
        req.label.clone()
    };
    let mut event = TraceEvent::new(TraceBackend::Llm, stage, req.model_tag.clone());
    event.request = request_text(req);
    (event, Instant::now())
}

impl<P: ModelProvider> Trace<P> {
    fn finish(
        &self,
        mut event: TraceEvent,
        started: Instant,
        result: Result<CompletionResponse, ModelError>,
    ) -> Result<CompletionResponse, ModelError> {
        event.latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match &result {
            Ok(response) => {
                event.response = response.content.clone();
                event.cached = response.provenance.cache_hit;
                if event.cached {
                    event.note = "served from the response cache".to_string();
                }
                event.tokens_in = Some(response.usage.prompt_eval_count);
                event.tokens_out = Some(response.usage.eval_count);
            }
            Err(e) => {
                event.ok = false;
                event.response = e.to_string();
                event.note = "model call failed".to_string();
            }
        }
        self.log.record(event);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::MockProvider;
    use crate::provider::ModelTier;
    use crate::request::Message;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag-x", ModelTier::Main, vec![Message::user("hello there")])
            .with_system_prompt("be brief", 3)
    }

    #[tokio::test]
    async fn a_successful_call_is_recorded_with_its_prompt_answer_and_timing() {
        let log = Arc::new(TraceLog::new());
        let provider = Trace::new(MockProvider::new().push_content("hi"), log.clone());
        let response = provider
            .complete(req().with_label("agent step"))
            .await
            .unwrap();
        assert_eq!(response.content, "hi");
        let events = log.snapshot();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.backend, TraceBackend::Llm);
        assert_eq!(e.stage, "agent step");
        assert_eq!(e.model, "tag-x");
        assert!(e.request.contains("be brief") && e.request.contains("hello there"));
        assert!(e.request.contains("system prompt v3"));
        assert_eq!(e.response, "hi");
        assert!(e.ok);
    }

    #[tokio::test]
    async fn an_unlabelled_call_is_named_after_its_tier() {
        let log = Arc::new(TraceLog::new());
        let provider = Trace::new(MockProvider::new().push_content("x"), log.clone());
        provider.complete(req()).await.unwrap();
        assert_eq!(log.snapshot()[0].stage, "Main tier");
    }

    #[tokio::test]
    async fn a_failed_call_is_recorded_and_still_returned_as_an_error() {
        let log = Arc::new(TraceLog::new());
        let provider = Trace::new(MockProvider::new(), log.clone());
        let result = provider.complete(req()).await;
        assert!(result.is_err(), "an exhausted mock script errors");
        let events = log.snapshot();
        assert_eq!(events.len(), 1);
        assert!(!events[0].ok);
        assert!(!events[0].response.is_empty());
    }
}
