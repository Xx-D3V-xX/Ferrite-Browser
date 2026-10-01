//! The provider-agnostic seam: [`ModelProvider`] and the metadata a caller
//! needs in order to use one without knowing which one it is.
//!
//! `docs/REBUILD_DIRECTIVE.md` §10.5 is explicit that no concrete provider
//! type may appear in a `ferrite-ipi` or `ferrite-agent` signature. That is
//! why [`ProviderId`] is a closed enum of *names* rather than a type
//! parameter: downstream code can record and compare which backend answered
//! without being able to name the backend's type.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::ModelError, request::CompletionRequest, response::CompletionResponse};

/// Which backend produced a response.
///
/// Closed on purpose. A new backend is a new variant here and a new file in
/// `backends/`; there is no "other provider" escape hatch, because an
/// unnamed provider in an `ExecutionRecord` (§10.2) would be exactly the
/// silent confound that recording provenance exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    /// [`MockProvider`](crate::backends::MockProvider) — scripted, in-process.
    Mock,
    /// [`ReplayProvider`](crate::backends::ReplayProvider) — committed fixtures.
    Replay,
    /// [`OllamaProvider`](crate::backends::OllamaProvider) — cloud or local.
    Ollama,
    /// [`GeminiProvider`](crate::backends::GeminiProvider).
    Gemini,
}

impl ProviderId {
    /// The stable string form used in cache keys, fixtures and logs.
    ///
    /// Written by hand rather than derived from [`Debug`], because a cache
    /// key built from a `Debug` rendering would silently change if anyone
    /// ever renamed a variant, invalidating every committed fixture.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Replay => "replay",
            Self::Ollama => "ollama",
            Self::Gemini => "gemini",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which of the two configured workloads a call belongs to (§10.2).
///
/// The tier is not a model name — it selects *which configured tag* to use,
/// so no model name ever appears in Rust source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// `FERRITE_MODEL_SMALL` — the per-task fingerprint `may_use` prediction.
    Small,
    /// `FERRITE_MODEL_MAIN` — the agent loop's plan/act reasoning.
    Main,
}

impl ModelTier {
    /// The stable string form used in cache keys, fixtures and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Main => "main",
        }
    }

    /// The environment variable that configures this tier's model tag.
    #[must_use]
    pub const fn env_var(self) -> &'static str {
        match self {
            Self::Small => "FERRITE_MODEL_SMALL",
            Self::Main => "FERRITE_MODEL_MAIN",
        }
    }
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a backend can do, so a caller can adapt without type-checking it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    /// Whether [`CompletionRequest::format_schema`] is honoured as a
    /// *constraint* on the model rather than politely ignored.
    ///
    /// A provider that returns `false` here will still be asked, and the
    /// response still goes through the same parse-and-bound guard — §10.4's
    /// trust boundary does not depend on the model cooperating.
    pub supports_json_schema: bool,
    /// Context window in tokens, as documented by the backend. Advisory: it
    /// is not enforced here, it is what a caller sizes a prompt against.
    pub context_window_tokens: u32,
    /// Which configured tier this instance was built for.
    pub tier: ModelTier,
    /// Whether a call can reach the network.
    ///
    /// R7's mechanical check: a test harness asserts this is `false` for
    /// every provider it wires up, so "this test does not make a live call"
    /// is a property the type system reports rather than a claim a reviewer
    /// has to verify by reading.
    pub reaches_network: bool,
}

/// A text/structured-JSON completion backend.
///
/// Deliberately *narrow*: one request in, one response out. Tool/function
/// calling is not here — the agent loop that needs it (A9) builds it on top
/// of this trait rather than inside it, so a backend never has to model
/// someone else's control flow.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Which backend this is.
    fn id(&self) -> ProviderId;

    /// One completion round-trip.
    ///
    /// Every failure mode is an `Err`, bounded and typed: §10.4 requires
    /// that a timeout, a 429, malformed JSON, an empty body, an oversized
    /// body and budget exhaustion each produce a value the caller can act
    /// on, never a panic and never an unbounded allocation.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError>;

    /// What this backend supports.
    fn capabilities(&self) -> ProviderCapabilities;
}

#[async_trait]
impl<P: ModelProvider + ?Sized> ModelProvider for std::sync::Arc<P> {
    fn id(&self) -> ProviderId {
        (**self).id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        (**self).complete(req).await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        (**self).capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_tier_names_are_stable_cache_key_inputs() {
        // These strings feed SHA256 cache keys and committed fixture
        // filenames. Pinning them here means a rename shows up as a failing
        // test rather than as a silently cold cache.
        assert_eq!(ProviderId::Mock.as_str(), "mock");
        assert_eq!(ProviderId::Replay.as_str(), "replay");
        assert_eq!(ProviderId::Ollama.as_str(), "ollama");
        assert_eq!(ProviderId::Gemini.as_str(), "gemini");
        assert_eq!(ModelTier::Small.as_str(), "small");
        assert_eq!(ModelTier::Main.as_str(), "main");
    }

    #[test]
    fn each_tier_names_its_own_env_var_so_no_model_name_is_hardcoded() {
        assert_eq!(ModelTier::Small.env_var(), "FERRITE_MODEL_SMALL");
        assert_eq!(ModelTier::Main.env_var(), "FERRITE_MODEL_MAIN");
    }
}
