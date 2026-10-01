//! The trust boundary §10.4 describes, implemented once so every backend
//! gets the same answer.
//!
//! "Model output is never trusted as text": a body arriving from a provider
//! — mock, replay, Ollama or Gemini — passes through [`finalize`] before it
//! becomes a [`CompletionResponse`]. That is the single place where an empty
//! body, an oversized body and a body that is not the JSON it promised turn
//! into typed, bounded, logged errors.
//!
//! **Mechanism, not policy.** This module guarantees *shape*: that a caller
//! receives either a parsed value or an `Err`, never a panic, a hang or an
//! unbounded allocation. What the parsed labels are allowed to *mean* — the
//! closed 7-capability allowlist — is A4's filter, deliberately not here.

use crate::error::ModelError;
use crate::provider::ProviderId;
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, Provenance, TokenUsage};

/// How much of a provider-controlled string may reach an error message or a
/// log line. Model output is attacker-influenced (that is the whole premise
/// of the project), so it is truncated before it is ever formatted.
pub(crate) const MAX_EXCERPT_BYTES: usize = 512;

/// Truncates provider-controlled text to [`MAX_EXCERPT_BYTES`], on a UTF-8
/// character boundary, with an explicit marker so a reader knows it is a
/// fragment rather than the whole story.
#[must_use]
pub(crate) fn bounded(s: &str) -> String {
    if s.len() <= MAX_EXCERPT_BYTES {
        return s.to_string();
    }
    let mut end = MAX_EXCERPT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [{} bytes truncated]", &s[..end], s.len() - end)
}

/// The raw material a backend extracts from its own wire format, before the
/// trust boundary is applied.
#[derive(Debug, Clone)]
pub(crate) struct RawCompletion {
    /// The model's text, verbatim.
    pub content: String,
    /// Token counts, already mapped onto Ollama's names.
    pub usage: TokenUsage,
}

/// Turns a backend's raw output into a [`CompletionResponse`] or a typed
/// error.
///
/// The order matters and is deliberate:
/// 1. **Size** first, so an oversized body is rejected before anything tries
///    to parse it. (A streaming backend will already have abandoned the read
///    — this is the belt to that braces, and the only check a non-streaming
///    backend like the mock has.)
/// 2. **Emptiness** second, because an empty body is a distinct, actionable
///    condition and "unexpected end of input" is a worse message for it.
/// 3. **JSON parse** last, and only when a schema was actually requested. A
///    free-text completion is not required to be JSON.
pub(crate) fn finalize(
    provider: ProviderId,
    req: &CompletionRequest,
    raw: RawCompletion,
    limit_bytes: usize,
) -> Result<CompletionResponse, ModelError> {
    if raw.content.len() > limit_bytes {
        return Err(ModelError::ResponseTooLarge {
            provider,
            seen_bytes: raw.content.len(),
            limit_bytes,
        });
    }
    if raw.content.trim().is_empty() {
        return Err(ModelError::EmptyResponse { provider });
    }

    let structured = match req.format_schema {
        None => None,
        Some(_) => Some(
            serde_json::from_str::<serde_json::Value>(&raw.content).map_err(|e| {
                ModelError::MalformedJson {
                    provider,
                    detail: format!("{} (body: {})", e, bounded(&raw.content)),
                }
            })?,
        ),
    };

    Ok(CompletionResponse {
        content: raw.content,
        structured,
        usage: raw.usage,
        provenance: Provenance {
            provider,
            model_tag: req.model_tag.clone(),
            tier: req.tier,
            cache_hit: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ModelTier;
    use crate::request::Message;

    fn req(schema: bool) -> CompletionRequest {
        let r = CompletionRequest::new("tag", ModelTier::Small, vec![Message::user("hi")]);
        if schema {
            r.with_format_schema(serde_json::json!({"type": "array"}))
        } else {
            r
        }
    }

    fn raw(content: &str) -> RawCompletion {
        RawCompletion {
            content: content.to_string(),
            usage: TokenUsage::default(),
        }
    }

    #[test]
    fn bounded_leaves_short_text_alone() {
        assert_eq!(bounded("short"), "short");
    }

    #[test]
    fn bounded_truncates_long_text_and_says_so() {
        let long = "a".repeat(MAX_EXCERPT_BYTES * 4);
        let out = bounded(&long);
        assert!(out.len() < long.len());
        assert!(out.contains("bytes truncated"), "{out}");
    }

    #[test]
    fn bounded_never_splits_a_utf8_character() {
        // A 3-byte character straddling the cut point would panic a naive
        // `&s[..N]`. The whole string is multi-byte so the boundary walk is
        // exercised on every possible offset.
        let s = "☃".repeat(MAX_EXCERPT_BYTES);
        let out = bounded(&s);
        assert!(out.starts_with('☃'));
        assert!(out.contains("bytes truncated"));
    }

    #[test]
    fn an_oversized_body_is_rejected_before_it_is_parsed() {
        let huge = "x".repeat(1024);
        let err = finalize(ProviderId::Mock, &req(true), raw(&huge), 100).expect_err("bounded");
        match err {
            ModelError::ResponseTooLarge {
                seen_bytes,
                limit_bytes,
                ..
            } => {
                assert_eq!(seen_bytes, 1024);
                assert_eq!(limit_bytes, 100);
            }
            other => panic!("expected ResponseTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_body_is_its_own_error_not_a_parse_failure() {
        for body in ["", "   ", "\n\t "] {
            let err = finalize(ProviderId::Mock, &req(true), raw(body), 1024).expect_err("bounded");
            assert!(
                matches!(err, ModelError::EmptyResponse { .. }),
                "body {body:?} gave {err:?}"
            );
        }
    }

    #[test]
    fn truncated_json_is_a_malformed_json_error_with_a_bounded_excerpt() {
        let err =
            finalize(ProviderId::Mock, &req(true), raw("[\"read_pa"), 1024).expect_err("bounded");
        match err {
            ModelError::MalformedJson { detail, .. } => {
                assert!(detail.len() <= MAX_EXCERPT_BYTES * 2, "excerpt is bounded");
            }
            other => panic!("expected MalformedJson, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_json_array_is_a_valid_parse_and_is_already_the_fail_to_empty_state() {
        // §10.4's "fail to empty" terminal state, reached legitimately: the
        // model said "no capabilities". It is not an error, and treating it
        // as one would be indistinguishable from the model saying nothing.
        let ok = finalize(ProviderId::Mock, &req(true), raw("[]"), 1024).expect("valid JSON");
        assert_eq!(ok.structured, Some(serde_json::json!([])));
        let labels: Vec<String> = ok.parse_structured().expect("empty array is a Vec");
        assert!(labels.is_empty());
    }

    #[test]
    fn a_free_text_completion_is_not_required_to_be_json() {
        let ok = finalize(ProviderId::Mock, &req(false), raw("just prose"), 1024)
            .expect("no schema was requested");
        assert_eq!(ok.content, "just prose");
        assert_eq!(ok.structured, None);
    }

    #[test]
    fn provenance_is_stamped_from_the_request() {
        let ok = finalize(ProviderId::Ollama, &req(false), raw("hi"), 1024).expect("ok");
        assert_eq!(ok.provenance.provider, ProviderId::Ollama);
        assert_eq!(ok.provenance.model_tag, "tag");
        assert_eq!(ok.provenance.tier, ModelTier::Small);
        assert!(!ok.provenance.cache_hit);
    }
}
