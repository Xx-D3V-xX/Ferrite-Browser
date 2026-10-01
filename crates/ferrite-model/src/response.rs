//! What comes back: [`CompletionResponse`], its token counts, and the
//! provenance §10.2 requires every decision to carry.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::provider::{ModelTier, ProviderId};

/// Token counts, named after Ollama's wire fields (§10.1) because those are
/// what the overhead metric in §13.2 is defined against.
///
/// Gemini's `usageMetadata.promptTokenCount` / `candidatesTokenCount` map
/// onto the same two numbers, so the metric does not have to know which
/// backend produced a record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Ollama's `prompt_eval_count`.
    pub prompt_eval_count: u32,
    /// Ollama's `eval_count`.
    pub eval_count: u32,
}

impl TokenUsage {
    /// Prompt plus completion.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.prompt_eval_count.saturating_add(self.eval_count)
    }
}

/// Which backend, tag and tier actually produced a response, and whether it
/// came off disk.
///
/// §10.2: "record which tier and tag produced every decision ... so a model
/// swap mid-corpus is visible in the data instead of being a silent
/// confound." Writing that into the `ExecutionRecord` is A12's job; making
/// it *available* to write is this struct's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The backend that answered. For a cached response this is the
    /// provider that *originally* answered, not the cache — a cache is a
    /// decorator, not a provider, and recording `cache` here would lose the
    /// only fact the metric cares about.
    pub provider: ProviderId,
    /// The exact model tag.
    pub model_tag: String,
    /// Which configured tier the tag came from.
    pub tier: ModelTier,
    /// Whether this response was served from the on-disk cache rather than
    /// by a fresh call. Lets a run report calls-used vs cache-hits at exit
    /// (§10.3) without the cache having to be consulted separately.
    pub cache_hit: bool,
}

/// One completion result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The raw text the model produced. For a structured request this is
    /// the JSON document as a string, kept verbatim so a fixture records
    /// exactly what the model said.
    pub content: String,
    /// The parsed document, present exactly when the request carried a
    /// `format_schema` and the body parsed as JSON.
    ///
    /// Parsing happens at the boundary rather than in the caller so that a
    /// malformed body becomes [`ModelError::MalformedJson`] once, here,
    /// instead of becoming a `serde_json` panic somewhere downstream.
    pub structured: Option<serde_json::Value>,
    /// Token counts.
    pub usage: TokenUsage,
    /// Which backend/tag/tier answered.
    pub provenance: Provenance,
}

impl CompletionResponse {
    /// Deserializes the structured body into a typed value.
    ///
    /// This is the typed hand-off A4's fingerprint engine consumes. What it
    /// deliberately does *not* do is filter the parsed labels against the
    /// closed 7-capability allowlist — that is policy, it lives in A4, and
    /// putting it here would make this crate know about the taxonomy's
    /// meaning rather than its shape.
    ///
    /// # Errors
    ///
    /// [`ModelError::SchemaViolation`] if there is no structured body, or
    /// if it does not fit `T`.
    pub fn parse_structured<T: DeserializeOwned>(&self) -> Result<T, ModelError> {
        let value = self
            .structured
            .as_ref()
            .ok_or_else(|| ModelError::SchemaViolation {
                detail: "no structured body: the request carried no format schema".to_string(),
            })?;
        serde_json::from_value(value.clone()).map_err(|e| ModelError::SchemaViolation {
            detail: crate::guard::bounded(&e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(structured: Option<serde_json::Value>) -> CompletionResponse {
        CompletionResponse {
            content: "[]".to_string(),
            structured,
            usage: TokenUsage {
                prompt_eval_count: 11,
                eval_count: 2,
            },
            provenance: Provenance {
                provider: ProviderId::Mock,
                model_tag: "tag".to_string(),
                tier: ModelTier::Small,
                cache_hit: false,
            },
        }
    }

    #[test]
    fn token_counts_add_up_and_do_not_overflow() {
        assert_eq!(response(None).usage.total(), 13);
        let maxed = TokenUsage {
            prompt_eval_count: u32::MAX,
            eval_count: 5,
        };
        assert_eq!(maxed.total(), u32::MAX, "saturating, never a panic");
    }

    #[test]
    fn a_structured_body_parses_into_a_typed_value() {
        let r = response(Some(serde_json::json!(["read_page", "navigate"])));
        let labels: Vec<String> = r.parse_structured().expect("parses");
        assert_eq!(labels, vec!["read_page", "navigate"]);
    }

    #[test]
    fn a_body_that_does_not_fit_the_type_is_an_error_not_a_panic() {
        let r = response(Some(serde_json::json!({"not": "an array"})));
        let err = r
            .parse_structured::<Vec<String>>()
            .expect_err("must not panic");
        assert!(matches!(err, ModelError::SchemaViolation { .. }), "{err:?}");
    }

    #[test]
    fn asking_for_a_structured_body_that_was_never_requested_is_an_error() {
        let err = response(None)
            .parse_structured::<Vec<String>>()
            .expect_err("must not panic");
        assert!(matches!(err, ModelError::SchemaViolation { .. }), "{err:?}");
    }
}
