//! The content-addressed key §10.3 specifies:
//!
//! ```text
//! SHA256(provider, model_tag, temperature, seed, system_prompt_version,
//!        canonical_messages_json, format_schema)
//! ```
//!
//! It is the same key for the on-disk response cache and for the committed
//! replay fixtures, deliberately: a fixture recorded by `just record` is
//! exactly the cache entry that call would have written, so the two
//! mechanisms can never disagree about what "the same request" means.
//!
//! Three properties the implementation has to guarantee, each with a test:
//!
//! - **Canonical.** JSON object keys are emitted in sorted order, so a
//!   schema built by two different callers hashes the same (R8).
//! - **Unambiguous.** Fields are length-prefixed, so no pair of different
//!   requests can serialise to the same byte stream by concatenation.
//! - **Domain-separated.** The preimage starts with a version tag, so a
//!   future change to the key's shape cannot collide with today's entries —
//!   it produces a cold cache, which is correct, rather than a wrong hit.

use sha2::{Digest, Sha256};

use crate::provider::ProviderId;
use crate::request::CompletionRequest;

/// Prefix on every preimage. Bump the version when the key's *shape*
/// changes; every existing entry then misses rather than being misread.
const DOMAIN: &[u8] = b"ferrite-model-cache-v1";

/// A hex-encoded SHA256 over a request's cacheable identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    /// Computes the key for `req` as answered by `provider`.
    ///
    /// The provider is an input rather than being read off the request
    /// because the same request routed to Ollama and to Gemini are two
    /// different questions with two different answers.
    #[must_use]
    pub fn compute(provider: ProviderId, req: &CompletionRequest) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN);

        field(&mut hasher, provider.as_str().as_bytes());
        field(&mut hasher, req.model_tag.as_bytes());
        // Bit pattern, not a formatted float: `0.0` and `-0.0` are distinct
        // bit patterns and distinct keys, which is harmless, whereas a
        // formatted `0` and `0.0` would be the same request hashing two ways.
        field(
            &mut hasher,
            &req.options.temperature.to_bits().to_be_bytes(),
        );
        field(&mut hasher, &req.options.seed.to_be_bytes());
        field(&mut hasher, &req.options.num_predict.to_be_bytes());
        field(&mut hasher, &req.system_prompt_version.to_be_bytes());
        field(
            &mut hasher,
            req.system_prompt.as_deref().unwrap_or("").as_bytes(),
        );
        field(&mut hasher, canonical_messages(req).as_bytes());
        field(&mut hasher, canonical_schema(req).as_bytes());

        Self(hex::encode(hasher.finalize()))
    }

    /// The hex digest — the fixture filename stem and the cache filename stem.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wraps an already-computed digest, for reading a fixture directory
    /// back without recomputing it.
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Length-prefixes a field so `("ab", "c")` and `("a", "bc")` cannot hash
/// alike.
fn field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

/// The message list as canonical JSON.
///
/// `serde_json`'s default map is a `BTreeMap`, so object keys serialise in
/// sorted order; the crate is deliberately *not* built with `preserve_order`
/// anywhere in this workspace, and
/// [`object_keys_are_emitted_in_sorted_order`] pins that.
fn canonical_messages(req: &CompletionRequest) -> String {
    serde_json::to_string(&req.messages)
        .expect("messages are plain strings and enums; serialisation cannot fail")
}

fn canonical_schema(req: &CompletionRequest) -> String {
    match &req.format_schema {
        None => String::new(),
        Some(schema) => {
            serde_json::to_string(schema).expect("a serde_json::Value always re-serialises")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ModelTier;
    use crate::request::{Message, SamplingOptions};

    fn base() -> CompletionRequest {
        CompletionRequest::new("tag:1b", ModelTier::Small, vec![Message::user("hello")])
            .with_system_prompt("be terse", 1)
    }

    fn key(req: &CompletionRequest) -> String {
        CacheKey::compute(ProviderId::Ollama, req)
            .as_str()
            .to_string()
    }

    #[test]
    fn the_same_request_always_produces_the_same_key() {
        assert_eq!(key(&base()), key(&base()));
        assert_eq!(key(&base()).len(), 64, "hex-encoded SHA256");
    }

    #[test]
    fn object_keys_are_emitted_in_sorted_order() {
        // The whole soundness argument for a content-addressed cache rests
        // on two spellings of the same schema hashing alike. If anything in
        // this workspace ever enables serde_json's `preserve_order`, this is
        // the test that says so.
        let one: serde_json::Value =
            serde_json::from_str(r#"{"zebra": 1, "apple": 2}"#).expect("parses");
        let other: serde_json::Value =
            serde_json::from_str(r#"{"apple": 2, "zebra": 1}"#).expect("parses");
        assert_eq!(
            serde_json::to_string(&one).expect("serialises"),
            serde_json::to_string(&other).expect("serialises")
        );
        assert_eq!(
            key(&base().clone().with_format_schema(one)),
            key(&base().clone().with_format_schema(other))
        );
    }

    #[test]
    fn every_specified_input_changes_the_key() {
        let baseline = key(&base());

        let cases: Vec<(&str, CompletionRequest)> = vec![
            ("model tag", {
                let mut r = base();
                r.model_tag = "other:31b".to_string();
                r
            }),
            (
                "temperature",
                base().with_options(SamplingOptions {
                    temperature: 0.5,
                    ..SamplingOptions::default()
                }),
            ),
            (
                "seed",
                base().with_options(SamplingOptions {
                    seed: 43,
                    ..SamplingOptions::default()
                }),
            ),
            (
                "num_predict",
                base().with_options(SamplingOptions::default().with_num_predict(256)),
            ),
            (
                "system prompt version",
                base().with_system_prompt("be terse", 2),
            ),
            (
                "system prompt text",
                base().with_system_prompt("be wordy", 1),
            ),
            ("messages", {
                let mut r = base();
                r.messages = vec![Message::user("goodbye")];
                r
            }),
            (
                "format schema",
                base().with_format_schema(serde_json::json!({"type": "array"})),
            ),
        ];

        for (what, req) in cases {
            assert_ne!(
                key(&req),
                baseline,
                "changing the {what} must change the key"
            );
        }
    }

    #[test]
    fn the_provider_is_part_of_the_key() {
        let req = base();
        assert_ne!(
            CacheKey::compute(ProviderId::Ollama, &req),
            CacheKey::compute(ProviderId::Gemini, &req),
            "the same prompt asked of two backends is two questions"
        );
    }

    #[test]
    fn fields_are_length_prefixed_so_a_shift_between_them_cannot_collide() {
        let mut a = base();
        a.model_tag = "ab".to_string();
        a.messages = vec![Message::user("c")];

        let mut b = base();
        b.model_tag = "a".to_string();
        b.messages = vec![Message::user("bc")];

        assert_ne!(key(&a), key(&b));
    }

    #[test]
    fn the_tier_is_not_part_of_the_key() {
        // Deliberate: the tier selects *which tag* to use, and the tag is
        // already hashed. Two tiers configured to the same tag ask the same
        // question and should share one cache entry — which is exactly the
        // collapse §10.3's arithmetic depends on.
        let small = base();
        let mut main = base();
        main.tier = ModelTier::Main;
        assert_eq!(key(&small), key(&main));
    }

    #[test]
    fn a_key_round_trips_through_its_hex_form() {
        let k = CacheKey::compute(ProviderId::Ollama, &base());
        assert_eq!(CacheKey::from_hex(k.as_str()), k);
        assert_eq!(k.to_string(), k.as_str());
    }
}
