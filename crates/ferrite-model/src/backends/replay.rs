//! Serves committed fixtures instead of calling a provider (§10.3).
//!
//! This is the other half of R7's bargain: unit tests script a
//! [`MockProvider`](super::MockProvider), integration tests and CI replay
//! real recorded responses, and neither needs a network or a key.
//!
//! A replay provider is built *for* a recorded backend —
//! `ReplayProvider::new(dir, ProviderId::Ollama)` serves the Ollama corpus —
//! because the fixture key includes the provider that answered. What it
//! reports as its own [`id`](ModelProvider::id) is
//! [`ProviderId::Replay`](crate::ProviderId::Replay), and the provenance on
//! every response it returns says `replay` too: a record must never be able
//! to claim a live call happened when it did not.
//!
//! A miss is a hard, typed failure with the key printed. There is
//! deliberately no fallthrough-to-network path in this file, so "the CI
//! suite quietly started spending quota" is not a state this code can reach.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use crate::cache_key::CacheKey;
use crate::error::ModelError;
use crate::fixtures;
use crate::guard;
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::CompletionResponse;

/// Serves `tests/fixtures/model/<hash>.json`.
#[derive(Debug)]
pub struct ReplayProvider {
    dir: PathBuf,
    records: ProviderId,
    tier: ModelTier,
    max_response_bytes: usize,
    served: AtomicU64,
}

impl ReplayProvider {
    /// Serves fixtures from `dir` that were recorded from `records`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, records: ProviderId) -> Self {
        Self {
            dir: dir.into(),
            records,
            tier: ModelTier::Small,
            max_response_bytes: crate::config::DEFAULT_MAX_RESPONSE_BYTES,
            served: AtomicU64::new(0),
        }
    }

    /// Declares which tier the recorded corpus belongs to.
    #[must_use]
    pub fn with_tier(mut self, tier: ModelTier) -> Self {
        self.tier = tier;
        self
    }

    /// Sets the response ceiling applied to replayed bodies.
    ///
    /// Applied to fixtures too, not just live responses: a fixture is still
    /// provider-derived data, and a suite that skips the ceiling on replay
    /// would not be testing the code that runs in production.
    #[must_use]
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    /// The fixture directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Which backend the fixtures were recorded from.
    #[must_use]
    pub fn records(&self) -> ProviderId {
        self.records
    }

    /// How many fixtures have been served.
    #[must_use]
    pub fn served(&self) -> u64 {
        self.served.load(Ordering::Relaxed)
    }

    /// The key a given request will be looked up under — what `just record`
    /// would name the file, and what a replay miss prints.
    #[must_use]
    pub fn key_for(&self, req: &CompletionRequest) -> CacheKey {
        CacheKey::compute(self.records, req)
    }
}

#[async_trait]
impl ModelProvider for ReplayProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Replay
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let key = self.key_for(&req);
        let fixture = fixtures::load(&self.dir, &key)?;

        // The recorded body goes through the *live* parser, so replaying
        // exercises the wire format rather than a summary of it.
        let body =
            serde_json::to_vec(&fixture.wire_response).map_err(|e| ModelError::MalformedJson {
                provider: ProviderId::Replay,
                detail: guard::bounded(&e.to_string()),
            })?;
        let raw = match fixture.provider {
            ProviderId::Ollama => super::ollama::parse_chat_response(&body)?,
            ProviderId::Gemini => super::gemini::parse_generate_response(&body)?,
            other => {
                return Err(ModelError::Config(format!(
                    "fixture {key} records provider '{other}', which has no wire format to replay; \
                     fixtures are recorded from a live backend only"
                )));
            }
        };

        self.served.fetch_add(1, Ordering::Relaxed);
        guard::finalize(ProviderId::Replay, &req, raw, self.max_response_bytes)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_json_schema: true,
            context_window_tokens: 8192,
            tier: self.tier,
            // The property the whole of R7 rests on, stated mechanically.
            reaches_network: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use ferrite_core::FixedClock;

    use super::*;
    use crate::request::Message;
    use crate::testing::TempDir;

    fn req() -> CompletionRequest {
        CompletionRequest::new("gemma3:27b", ModelTier::Small, vec![Message::user("hi")])
            .with_format_schema(serde_json::json!({"type": "array"}))
    }

    fn record(dir: &Path, provider: ProviderId, wire: serde_json::Value) {
        fixtures::write(dir, provider, &req(), wire, &FixedClock::at_epoch()).expect("writes");
    }

    fn ollama_wire(content: &str) -> serde_json::Value {
        serde_json::json!({
            "model": "gemma3:27b",
            "message": { "role": "assistant", "content": content },
            "done": true,
            "prompt_eval_count": 19,
            "eval_count": 5
        })
    }

    #[tokio::test]
    async fn a_replay_provider_never_reaches_the_network() {
        let dir = TempDir::new("replay-offline");
        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);
        assert!(!provider.capabilities().reaches_network);
    }

    #[tokio::test]
    async fn a_recorded_ollama_response_replays_through_the_live_parser() {
        let dir = TempDir::new("replay-ollama");
        record(
            dir.path(),
            ProviderId::Ollama,
            ollama_wire(r#"["read_page"]"#),
        );

        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);
        let response = provider.complete(req()).await.expect("served");

        assert_eq!(response.content, r#"["read_page"]"#);
        assert_eq!(response.usage.prompt_eval_count, 19);
        assert_eq!(response.usage.eval_count, 5);
        assert_eq!(response.structured, Some(serde_json::json!(["read_page"])));
        assert_eq!(provider.served(), 1);
    }

    #[tokio::test]
    async fn a_recorded_gemini_response_replays_through_its_own_parser() {
        let dir = TempDir::new("replay-gemini");
        record(
            dir.path(),
            ProviderId::Gemini,
            serde_json::json!({
                "candidates": [{"content": {"parts": [{"text": "[\"navigate\"]"}], "role": "model"}}],
                "usageMetadata": {"promptTokenCount": 30, "candidatesTokenCount": 4}
            }),
        );

        let provider = ReplayProvider::new(dir.path(), ProviderId::Gemini);
        let response = provider.complete(req()).await.expect("served");
        assert_eq!(response.content, r#"["navigate"]"#);
        assert_eq!(response.usage.prompt_eval_count, 30);
    }

    #[tokio::test]
    async fn provenance_says_replay_so_no_record_can_claim_a_live_call() {
        let dir = TempDir::new("replay-provenance");
        record(dir.path(), ProviderId::Ollama, ollama_wire("[]"));
        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);
        let response = provider.complete(req()).await.expect("served");
        assert_eq!(response.provenance.provider, ProviderId::Replay);
        assert_eq!(provider.id(), ProviderId::Replay);
    }

    #[tokio::test]
    async fn a_miss_is_a_hard_failure_with_the_key_printed() {
        // §10.3: "never a silent fallthrough to the network."
        let dir = TempDir::new("replay-miss");
        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);
        let err = provider
            .complete(req())
            .await
            .expect_err("nothing recorded");

        let expected = provider.key_for(&req());
        match &err {
            ModelError::ReplayMiss { key, fixture_dir } => {
                assert_eq!(key, expected.as_str());
                assert_eq!(fixture_dir, dir.path());
            }
            other => panic!("expected ReplayMiss, got {other:?}"),
        }
        assert!(err.to_string().contains(expected.as_str()));
        assert!(
            err.to_string().contains("just record"),
            "the message should say how to fix it: {err}"
        );
    }

    #[tokio::test]
    async fn a_different_request_misses_rather_than_serving_a_near_match() {
        let dir = TempDir::new("replay-near-match");
        record(dir.path(), ProviderId::Ollama, ollama_wire("[]"));
        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);

        let mut other = req();
        other.messages = vec![Message::user("a different question entirely")];
        assert!(matches!(
            provider.complete(other).await.expect_err("must miss"),
            ModelError::ReplayMiss { .. }
        ));
    }

    #[tokio::test]
    async fn a_fixture_recorded_from_one_backend_is_not_served_for_another() {
        let dir = TempDir::new("replay-wrong-backend");
        record(dir.path(), ProviderId::Ollama, ollama_wire("[]"));
        let as_gemini = ReplayProvider::new(dir.path(), ProviderId::Gemini);
        assert!(matches!(
            as_gemini.complete(req()).await.expect_err("different key"),
            ModelError::ReplayMiss { .. }
        ));
    }

    #[tokio::test]
    async fn a_replayed_body_still_passes_through_the_trust_boundary() {
        let dir = TempDir::new("replay-guard");
        record(
            dir.path(),
            ProviderId::Ollama,
            ollama_wire(r#"["truncated"#),
        );
        let provider = ReplayProvider::new(dir.path(), ProviderId::Ollama);
        let err = provider.complete(req()).await.expect_err("malformed");
        assert!(matches!(err, ModelError::MalformedJson { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn the_response_ceiling_applies_to_fixtures_too() {
        let dir = TempDir::new("replay-ceiling");
        record(
            dir.path(),
            ProviderId::Ollama,
            ollama_wire(&"x".repeat(5000)),
        );
        let provider =
            ReplayProvider::new(dir.path(), ProviderId::Ollama).with_max_response_bytes(100);
        assert!(matches!(
            provider.complete(req()).await.expect_err("too large"),
            ModelError::ResponseTooLarge { .. }
        ));
    }

    #[tokio::test]
    async fn a_fixture_claiming_a_non_wire_provider_is_rejected() {
        let dir = TempDir::new("replay-bad-provider");
        record(dir.path(), ProviderId::Mock, ollama_wire("[]"));
        // Looked up under the Mock key, so the file is found — and then
        // refused, because there is no Mock wire format to replay.
        let provider = ReplayProvider::new(dir.path(), ProviderId::Mock);
        let err = provider.complete(req()).await.expect_err("no wire format");
        assert!(matches!(err, ModelError::Config(_)), "{err:?}");
    }
}
