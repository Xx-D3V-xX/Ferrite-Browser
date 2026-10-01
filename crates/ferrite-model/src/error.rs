//! Every way a model call can fail.
//!
//! One enum, not one per layer, because §10.4's rule is stated over the set
//! of *outcomes* ("timeout, 429, malformed JSON, schema violation, budget
//! exhaustion, empty body — every one yields an empty `may_use` set"), and a
//! caller that has to match across four error types to implement one rule
//! will eventually miss a variant.
//!
//! Every variant carries bounded data. A 10 MB response body does not become
//! a 10 MB error message: [`crate::guard::bounded`] truncates anything
//! derived from a provider's bytes before it reaches a variant.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

use crate::provider::ProviderId;

/// A model call failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ModelError {
    /// The request never reached the provider, or the connection broke.
    #[error("transport failure talking to {provider}: {detail}")]
    Transport {
        /// Which backend.
        provider: ProviderId,
        /// A bounded description.
        detail: String,
    },

    /// The per-request timeout elapsed (§10.3's fourth mechanism).
    #[error("request to {provider} timed out after {after:?}")]
    Timeout {
        /// Which backend.
        provider: ProviderId,
        /// The configured timeout that elapsed.
        after: Duration,
    },

    /// HTTP 429. Carries `Retry-After` when the provider sent one.
    ///
    /// Reaching a caller *as* this variant means throttling already gave
    /// up: [`Throttle`](crate::decorators::Throttle) retries a 429 with
    /// backoff and only surfaces the last one.
    #[error("rate limited by {provider} (retry after {retry_after:?})")]
    RateLimited {
        /// Which backend.
        provider: ProviderId,
        /// The parsed `Retry-After`, if present.
        retry_after: Option<Duration>,
    },

    /// HTTP 5xx.
    #[error("{provider} returned server error {status}: {body_excerpt}")]
    ServerError {
        /// Which backend.
        provider: ProviderId,
        /// The status code.
        status: u16,
        /// A bounded excerpt of the body.
        body_excerpt: String,
    },

    /// HTTP 4xx other than 429 — a bad request, a bad key, a retired tag.
    /// Not retried: retrying a request the provider rejected on its merits
    /// spends quota to get the same answer.
    #[error("{provider} rejected the request with {status}: {body_excerpt}")]
    ClientError {
        /// Which backend.
        provider: ProviderId,
        /// The status code.
        status: u16,
        /// A bounded excerpt of the body.
        body_excerpt: String,
    },

    /// The body was not the JSON it claimed to be — truncated, or not JSON.
    #[error("{provider} returned a body that is not valid JSON: {detail}")]
    MalformedJson {
        /// Which backend.
        provider: ProviderId,
        /// A bounded parser message.
        detail: String,
    },

    /// The provider answered with nothing at all.
    #[error("{provider} returned an empty response body")]
    EmptyResponse {
        /// Which backend.
        provider: ProviderId,
    },

    /// The body exceeded the configured ceiling and was abandoned *while
    /// streaming*, so the bytes past the limit were never allocated.
    #[error(
        "{provider} returned at least {seen_bytes} bytes, over the {limit_bytes}-byte ceiling; \
         the body was abandoned rather than buffered"
    )]
    ResponseTooLarge {
        /// Which backend.
        provider: ProviderId,
        /// How many bytes had been read when the read was abandoned.
        seen_bytes: usize,
        /// The configured ceiling.
        limit_bytes: usize,
    },

    /// Valid JSON that does not fit the shape the caller asked for.
    #[error("the structured response did not fit the requested schema: {detail}")]
    SchemaViolation {
        /// A bounded description.
        detail: String,
    },

    /// `FERRITE_MODEL_CALL_BUDGET` is spent (§10.3's fourth mechanism).
    /// A partial-results artifact was written before aborting.
    #[error(
        "model call budget of {budget} exhausted; partial results written to {partial_results_path}"
    )]
    BudgetExhausted {
        /// The configured budget.
        budget: u32,
        /// Where the partial-results artifact landed.
        partial_results_path: PathBuf,
    },

    /// [`ReplayProvider`](crate::backends::ReplayProvider) has no fixture
    /// for this request.
    ///
    /// §10.3: "a replay miss is a hard failure with the missing key
    /// printed, never a silent fallthrough to the network." The key is in
    /// the message so that `just record` can be pointed at it directly.
    #[error(
        "replay miss: no fixture {key}.json in {fixture_dir}. \
         Record it with `just record` (needs a live key), or add the fixture by hand."
    )]
    ReplayMiss {
        /// The content-addressed key that was looked up.
        key: String,
        /// Where it was looked for.
        fixture_dir: PathBuf,
    },

    /// No API key in the environment and none in the OS keyring (§10.1).
    #[error(
        "no API key: set {env_var}, or store it in the OS keyring under service \
         '{keyring_service}' / account '{keyring_account}'. \
         It may not go in a config file, a .env, or a default."
    )]
    MissingApiKey {
        /// The environment variable that was checked first.
        env_var: &'static str,
        /// The keyring service that was checked second.
        keyring_service: &'static str,
        /// The keyring account that was checked second.
        keyring_account: String,
    },

    /// The startup preflight (§10.2) found a configured tag the provider
    /// does not serve — caught before a run rather than as a mid-run 404.
    #[error(
        "configured model tag '{requested}' ({tier_env_var}) is not available. \
         Available tags: {available}"
    )]
    ModelTagUnavailable {
        /// The tag that was configured.
        requested: String,
        /// Which env var configured it, so the fix is actionable.
        tier_env_var: &'static str,
        /// The tags the provider actually serves, sorted (R8).
        available: String,
    },

    /// A configuration value was missing or unusable (T-213).
    #[error("model configuration: {0}")]
    Config(String),

    /// The on-disk cache could not be read or written. Never fatal to a
    /// call on its own — the cache decorator logs and falls through to the
    /// wrapped provider — but it is a real error worth surfacing.
    #[error("response cache at {path}: {detail}")]
    Cache {
        /// The cache path involved.
        path: PathBuf,
        /// A bounded description.
        detail: String,
    },

    /// A [`MockProvider`](crate::backends::MockProvider) ran out of script.
    /// A test that calls more times than it scripted is a test bug, and
    /// this says so rather than hanging or returning a made-up answer.
    #[error("mock provider script exhausted after {calls_made} call(s)")]
    MockScriptExhausted {
        /// How many calls the script did cover.
        calls_made: usize,
    },
}

impl ModelError {
    /// Whether retrying this failure could plausibly succeed.
    ///
    /// §10.3 names 429 and 5xx as the retryable pair. A timeout joins them:
    /// it is the client-side shape of the same "the server is struggling"
    /// condition. Everything else — a 4xx, a malformed body, a spent budget
    /// — returns the same answer next time and retrying only spends quota.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::ServerError { .. } | Self::Timeout { .. }
        )
    }

    /// The `Retry-After` the provider asked for, if it asked for one.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_transient_failures_are_retryable() {
        let retryable = [
            ModelError::RateLimited {
                provider: ProviderId::Ollama,
                retry_after: None,
            },
            ModelError::ServerError {
                provider: ProviderId::Ollama,
                status: 503,
                body_excerpt: String::new(),
            },
            ModelError::Timeout {
                provider: ProviderId::Ollama,
                after: Duration::from_secs(1),
            },
        ];
        for e in retryable {
            assert!(e.is_retryable(), "{e:?} should be retryable");
        }

        let terminal = [
            ModelError::ClientError {
                provider: ProviderId::Ollama,
                status: 401,
                body_excerpt: String::new(),
            },
            ModelError::MalformedJson {
                provider: ProviderId::Ollama,
                detail: String::new(),
            },
            ModelError::EmptyResponse {
                provider: ProviderId::Ollama,
            },
            ModelError::BudgetExhausted {
                budget: 1,
                partial_results_path: PathBuf::from("/tmp/x.json"),
            },
        ];
        for e in terminal {
            assert!(
                !e.is_retryable(),
                "{e:?} returns the same answer next time; retrying only spends quota"
            );
        }
    }

    #[test]
    fn a_replay_miss_prints_the_missing_key() {
        let err = ModelError::ReplayMiss {
            key: "abc123".to_string(),
            fixture_dir: PathBuf::from("tests/fixtures/model"),
        };
        let rendered = err.to_string();
        assert!(
            rendered.contains("abc123"),
            "§10.3 requires the missing key be printed: {rendered}"
        );
    }

    #[test]
    fn a_missing_key_error_never_suggests_a_config_file() {
        let err = ModelError::MissingApiKey {
            env_var: "OLLAMA_API_KEY",
            keyring_service: "ferrite",
            keyring_account: "OLLAMA_API_KEY".to_string(),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("OLLAMA_API_KEY"));
        assert!(
            rendered.contains("may not go in a config file"),
            "§10.1 is explicit that env var or keyring are the only two sources"
        );
    }

    #[test]
    fn only_a_rate_limit_carries_a_retry_after() {
        let with = ModelError::RateLimited {
            provider: ProviderId::Ollama,
            retry_after: Some(Duration::from_secs(30)),
        };
        assert_eq!(with.retry_after(), Some(Duration::from_secs(30)));
        assert_eq!(
            ModelError::Timeout {
                provider: ProviderId::Ollama,
                after: Duration::from_secs(1)
            }
            .retry_after(),
            None
        );
    }
}
