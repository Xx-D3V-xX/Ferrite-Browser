//! Ollama — the default backend (§10.1), cloud *and* local.
//!
//! One struct covers both, because they are the same wire protocol at
//! different base URLs:
//!
//! ```text
//! POST https://ollama.com/api/chat        Authorization: Bearer $OLLAMA_API_KEY
//! POST http://localhost:11434/api/chat    (no Authorization header)
//! ```
//!
//! The body, the answer's location (`message.content`) and the token counts
//! (`prompt_eval_count` / `eval_count`) are identical. Making local a
//! *configuration* of the same backend rather than a second backend is what
//! keeps the offline fallback exercised by the same code the cloud path uses.
//!
//! # What is deliberately not here
//!
//! Tool/function calling. §10.5's trait is text and structured JSON; the
//! agent loop that needs tools (A9) builds on this rather than inside it.
//!
//! # Wire parsing is separated from HTTP
//!
//! [`build_chat_body`] and [`parse_chat_response`] are pure functions over
//! JSON, so the format is tested against realistic payloads with no network
//! and no key (R7). The HTTP method is the thin part that cannot be tested
//! offline, and it is thin on purpose.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::ModelConfig;
use crate::error::ModelError;
use crate::guard::{self, RawCompletion, bounded};
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::{CompletionRequest, Role};
use crate::response::{CompletionResponse, TokenUsage};
use crate::secret::{SecretStore, Token};

use super::http;

/// The env var §10.1 names as the first of exactly two key sources.
pub const OLLAMA_API_KEY_VAR: &str = "OLLAMA_API_KEY";

/// Builds the `/api/chat` request body of §10.1.
///
/// `stream` is always `false`: this layer returns one complete response, and
/// a streaming parser would be a second wire format to keep correct for no
/// benefit to any current caller.
#[must_use]
pub(crate) fn build_chat_body(req: &CompletionRequest) -> serde_json::Value {
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(system) = &req.system_prompt {
        messages.push(serde_json::json!({ "role": Role::System.as_str(), "content": system }));
    }
    for message in &req.messages {
        messages.push(serde_json::json!({
            "role": message.role.as_str(),
            "content": message.content,
        }));
    }

    let mut body = serde_json::json!({
        "model": req.model_tag,
        "messages": messages,
        "stream": false,
        "options": {
            "temperature": req.options.temperature,
            "seed": req.options.seed,
            "num_predict": req.options.num_predict,
        },
    });

    if let Some(schema) = &req.format_schema {
        body["format"] = schema.clone();
    }
    body
}

/// Reads `message.content` and the token counts out of an `/api/chat`
/// response.
///
/// Missing token counts are zeros rather than an error: they feed a metric
/// (§13.2), and losing a metric is not a reason to fail a call that
/// otherwise succeeded.
///
/// # Errors
///
/// [`ModelError::MalformedJson`] if the body is not JSON or has no
/// `message.content` string.
pub(crate) fn parse_chat_response(body: &[u8]) -> Result<RawCompletion, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Ollama,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;

    let content = json
        .pointer("/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::Ollama,
            detail: format!(
                "no string at /message/content (body: {})",
                bounded(&json.to_string())
            ),
        })?
        .to_string();

    Ok(RawCompletion {
        content,
        usage: TokenUsage {
            prompt_eval_count: count(&json, "prompt_eval_count"),
            eval_count: count(&json, "eval_count"),
        },
    })
}

fn count(json: &serde_json::Value, field: &str) -> u32 {
    json.get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}

/// Reads the tag list out of an `/api/tags` response, sorted.
///
/// Sorted because the list is printed by `just models` and quoted back in
/// the preflight failure, and R8 forbids output whose order depends on what
/// a server happened to send first.
///
/// # Errors
///
/// [`ModelError::MalformedJson`] if the body is not the documented shape.
pub(crate) fn parse_tags_response(body: &[u8]) -> Result<Vec<String>, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Ollama,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;

    let models = json
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::Ollama,
            detail: format!("no \"models\" array (body: {})", bounded(&json.to_string())),
        })?;

    let mut tags: Vec<String> = models
        .iter()
        .filter_map(|m| m.get("name").and_then(serde_json::Value::as_str))
        .map(ToString::to_string)
        .collect();
    tags.sort_unstable();
    tags.dedup();
    Ok(tags)
}

/// The startup preflight of §10.2, as a pure function.
///
/// > validates each configured tag against `/api/tags` and fails with the
/// > available list rather than dying mid-run on a 404.
///
/// # Errors
///
/// [`ModelError::ModelTagUnavailable`], naming the tag, the env var that
/// configured it, and every tag that *is* available.
pub fn validate_tag(
    requested: &str,
    tier_env_var: &'static str,
    available: &[String],
) -> Result<(), ModelError> {
    if available.iter().any(|tag| tag == requested) {
        return Ok(());
    }
    Err(ModelError::ModelTagUnavailable {
        requested: requested.to_string(),
        tier_env_var,
        available: if available.is_empty() {
            "(none — the endpoint served an empty list)".to_string()
        } else {
            available.join(", ")
        },
    })
}

/// The Ollama backend.
#[derive(Debug)]
pub struct OllamaProvider {
    base_url: String,
    auth: Option<Token>,
    tier: ModelTier,
    max_response_bytes: usize,
    client: reqwest::Client,
}

impl OllamaProvider {
    /// Builds a provider directly. `auth` is `None` for a local endpoint,
    /// which takes no `Authorization` header.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        auth: Option<Token>,
        tier: ModelTier,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth,
            tier,
            max_response_bytes,
            client: reqwest::Client::new(),
        }
    }

    /// Builds a provider from loaded configuration, resolving the key from
    /// the environment or the OS keyring — but only when the endpoint is
    /// remote.
    ///
    /// A local endpoint deliberately does *not* require a key, and just as
    /// deliberately never *sends* one: a bearer token must not leave the
    /// machine towards something that is not the cloud service.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingApiKey`] if the endpoint is remote and neither
    /// source has a key.
    pub fn from_config(
        config: &ModelConfig,
        tier: ModelTier,
        env: &dyn crate::config::EnvSource,
        store: &dyn SecretStore,
    ) -> Result<Self, ModelError> {
        let auth = if config.ollama_is_local() {
            None
        } else {
            Some(crate::secret::resolve(env, store, OLLAMA_API_KEY_VAR)?)
        };
        Ok(Self::new(
            &config.ollama_base_url,
            auth,
            tier,
            config.max_response_bytes,
        ))
    }

    /// The configured endpoint.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Whether this provider will send an `Authorization` header.
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.auth.is_some()
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let builder = self
            .client
            .request(method, format!("{}{path}", self.base_url));
        match &self.auth {
            Some(token) => builder.bearer_auth(token.expose()),
            None => builder,
        }
    }

    /// Every tag the endpoint serves, sorted — what `just models` prints.
    ///
    /// # Errors
    ///
    /// Transport, status or parse failures, each typed.
    pub async fn list_tags(&self) -> Result<Vec<String>, ModelError> {
        let response = self
            .request(reqwest::Method::GET, "/api/tags")
            .send()
            .await
            .map_err(|e| ModelError::Transport {
                provider: ProviderId::Ollama,
                detail: bounded(&e.to_string()),
            })?;
        let response =
            http::classify(ProviderId::Ollama, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Ollama, response, self.max_response_bytes).await?;
        parse_tags_response(&body)
    }

    /// The §10.2 startup preflight: validates both configured tags against
    /// `/api/tags` before a run starts.
    ///
    /// # Errors
    ///
    /// [`ModelError::ModelTagUnavailable`] for the first tag that is not
    /// served, listing what is.
    pub async fn preflight(&self, config: &ModelConfig) -> Result<Vec<String>, ModelError> {
        let available = self.list_tags().await?;
        validate_tag(&config.small, ModelTier::Small.env_var(), &available)?;
        validate_tag(&config.main, ModelTier::Main.env_var(), &available)?;
        Ok(available)
    }

    /// The provider's raw, unparsed wire body for `req` — what `just record`
    /// hands to [`crate::fixtures::write`].
    ///
    /// [`ModelProvider::complete`] digests the same call into a
    /// [`CompletionResponse`] and discards the original bytes once parsed; a
    /// committed fixture is defined (`fixtures.rs`'s module doc) to store
    /// the wire body verbatim so replay exercises the real parser, so this
    /// exists to keep that promise true rather than reconstructing an
    /// approximation of the wire shape from the digested response.
    ///
    /// # Errors
    ///
    /// The same transport/status/size errors [`ModelProvider::complete`]
    /// can return, plus [`ModelError::MalformedJson`] if the body is not a
    /// JSON object at all (`complete` would fail identically, later, inside
    /// its own parser).
    pub async fn fetch_wire(
        &self,
        req: &CompletionRequest,
    ) -> Result<serde_json::Value, ModelError> {
        let response = self
            .request(reqwest::Method::POST, "/api/chat")
            .json(&build_chat_body(req))
            .send()
            .await
            .map_err(|e| ModelError::Transport {
                provider: ProviderId::Ollama,
                detail: bounded(&e.to_string()),
            })?;
        let response =
            http::classify(ProviderId::Ollama, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Ollama, response, self.max_response_bytes).await?;
        serde_json::from_slice(&body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Ollama,
            detail: bounded(&e.to_string()),
        })
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Ollama
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let response = self
            .request(reqwest::Method::POST, "/api/chat")
            .json(&build_chat_body(&req))
            .send()
            .await
            .map_err(|e| ModelError::Transport {
                provider: ProviderId::Ollama,
                detail: bounded(&e.to_string()),
            })?;

        let response =
            http::classify(ProviderId::Ollama, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Ollama, response, self.max_response_bytes).await?;
        let raw = parse_chat_response(&body)?;
        guard::finalize(ProviderId::Ollama, &req, raw, self.max_response_bytes)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // Ollama's `format` field constrains decoding rather than asking
            // the model nicely — that is the mechanism §10.4 is built on.
            supports_json_schema: true,
            // Advisory. The real window is per-tag and only `/api/show`
            // knows it; this is a floor that every tag worth configuring
            // clears, not a claim about a specific model.
            context_window_tokens: 8192,
            tier: self.tier,
            reaches_network: true,
        }
    }
}

/// A shared, preconfigured Ollama provider for the tier `tier`.
///
/// Convenience for the CLI, which builds the same thing three times.
///
/// # Errors
///
/// Whatever [`OllamaProvider::from_config`] returns.
pub fn shared(
    config: &ModelConfig,
    tier: ModelTier,
    env: &dyn crate::config::EnvSource,
    store: &dyn SecretStore,
) -> Result<Arc<OllamaProvider>, ModelError> {
    Ok(Arc::new(OllamaProvider::from_config(
        config, tier, env, store,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_OLLAMA_BASE_URL, LOCAL_OLLAMA_BASE_URL, MapEnv};
    use crate::request::{Message, SamplingOptions};
    use crate::secret::{KEYRING_SERVICE, MapSecretStore, NoSecretStore};

    fn req() -> CompletionRequest {
        CompletionRequest::new(
            "gemma3:27b",
            ModelTier::Small,
            vec![Message::user("what does this page let me do?")],
        )
        .with_system_prompt("Answer with a JSON array of capability labels.", 1)
    }

    fn config(base_url: &str) -> ModelConfig {
        ModelConfig::load(
            &MapEnv::new()
                .with("FERRITE_MODEL_SMALL", "gemma3:27b")
                .with("FERRITE_MODEL_MAIN", "gemma3:27b-it")
                .with("FERRITE_MODEL_CACHE_DIR", "/tmp/ferrite-model-ollama-test")
                .with("FERRITE_OLLAMA_BASE_URL", base_url),
        )
        .expect("loads")
    }

    // ── wire format ──────────────────────────────────────────────────────

    #[test]
    fn the_request_body_matches_the_documented_shape() {
        let body = build_chat_body(&req());
        assert_eq!(body["model"], "gemma3:27b");
        assert_eq!(body["stream"], false, "§10.1 sends stream: false");
        assert_eq!(body["options"]["temperature"], 0.0);
        assert_eq!(body["options"]["seed"], 42);
        assert_eq!(body["options"]["num_predict"], 128);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][0]["content"],
            "Answer with a JSON array of capability labels."
        );
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(
            body.get("format").is_none(),
            "no schema requested, no format field"
        );
    }

    #[test]
    fn a_request_without_a_system_prompt_omits_the_system_message() {
        let mut bare = req();
        bare.system_prompt = None;
        let body = build_chat_body(&bare);
        assert_eq!(body["messages"].as_array().expect("array").len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn a_schema_is_sent_in_the_format_field() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let body = build_chat_body(&req().with_format_schema(schema.clone()));
        assert_eq!(body["format"], schema, "§10.4 constrains via `format`");
    }

    #[test]
    fn sampling_overrides_reach_the_options_block() {
        let body = build_chat_body(&req().with_options(SamplingOptions {
            temperature: 0.25,
            seed: 7,
            num_predict: 64,
        }));
        assert_eq!(body["options"]["seed"], 7);
        assert_eq!(body["options"]["num_predict"], 64);
    }

    #[test]
    fn a_real_chat_response_parses_into_content_and_token_counts() {
        // Shaped after an actual /api/chat reply, field for field.
        let body = br#"{
          "model": "gemma3:27b",
          "created_at": "2026-09-18T10:11:12.131415Z",
          "message": { "role": "assistant", "content": "[\"read_page\",\"navigate\"]" },
          "done_reason": "stop",
          "done": true,
          "total_duration": 1183446625,
          "load_duration": 21398250,
          "prompt_eval_count": 41,
          "prompt_eval_duration": 65000000,
          "eval_count": 12,
          "eval_duration": 1090000000
        }"#;
        let raw = parse_chat_response(body).expect("parses");
        assert_eq!(raw.content, r#"["read_page","navigate"]"#);
        assert_eq!(raw.usage.prompt_eval_count, 41);
        assert_eq!(raw.usage.eval_count, 12);
    }

    #[test]
    fn missing_token_counts_are_zero_rather_than_a_failed_call() {
        let body = br#"{"message":{"role":"assistant","content":"hi"},"done":true}"#;
        let raw = parse_chat_response(body).expect("parses");
        assert_eq!(raw.usage, TokenUsage::default());
    }

    #[test]
    fn a_response_without_message_content_is_malformed_not_empty() {
        let body = br#"{"error":"model 'nope:1b' not found"}"#;
        let err = parse_chat_response(body).expect_err("no content");
        assert!(matches!(err, ModelError::MalformedJson { .. }), "{err:?}");
    }

    #[test]
    fn a_truncated_body_is_malformed_json_with_a_bounded_excerpt() {
        let err = parse_chat_response(br#"{"message":{"content":"tru"#).expect_err("truncated");
        match err {
            ModelError::MalformedJson { detail, .. } => {
                assert!(detail.len() < 4096, "excerpts stay bounded");
            }
            other => panic!("expected MalformedJson, got {other:?}"),
        }
    }

    // ── /api/tags and the preflight ──────────────────────────────────────

    #[test]
    fn the_tag_list_parses_and_comes_back_sorted() {
        let body = br#"{
          "models": [
            {"name":"qwen3:8b","model":"qwen3:8b","size":5200000000,"digest":"aa"},
            {"name":"gemma3:27b","model":"gemma3:27b","size":17000000000,"digest":"bb"},
            {"name":"deepseek-v3.1:671b","model":"deepseek-v3.1:671b","size":4e11,"digest":"cc"}
          ]
        }"#;
        let tags = parse_tags_response(body).expect("parses");
        assert_eq!(
            tags,
            vec!["deepseek-v3.1:671b", "gemma3:27b", "qwen3:8b"],
            "R8: sorted, not server order"
        );
    }

    #[test]
    fn an_empty_tag_list_parses_to_nothing_rather_than_failing() {
        assert_eq!(
            parse_tags_response(br#"{"models":[]}"#).expect("parses"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_tag_body_of_the_wrong_shape_is_an_error() {
        let err = parse_tags_response(br#"{"tags":["a"]}"#).expect_err("wrong shape");
        assert!(matches!(err, ModelError::MalformedJson { .. }), "{err:?}");
    }

    #[test]
    fn the_preflight_accepts_a_tag_the_endpoint_serves() {
        let available = vec!["gemma3:27b".to_string(), "qwen3:8b".to_string()];
        validate_tag("gemma3:27b", "FERRITE_MODEL_SMALL", &available).expect("available");
    }

    #[test]
    fn the_preflight_fails_with_the_available_list_not_a_bare_404() {
        // §10.2's actual requirement: a retired tag is caught at startup and
        // the message tells you what you *can* use.
        let available = vec!["gemma3:27b".to_string(), "qwen3:8b".to_string()];
        let err =
            validate_tag("gemma2:9b", "FERRITE_MODEL_SMALL", &available).expect_err("retired tag");
        let message = err.to_string();
        assert!(message.contains("gemma2:9b"));
        assert!(message.contains("FERRITE_MODEL_SMALL"));
        assert!(message.contains("gemma3:27b"), "{message}");
        assert!(message.contains("qwen3:8b"), "{message}");
    }

    #[test]
    fn the_preflight_says_so_when_the_endpoint_serves_nothing() {
        let err = validate_tag("anything:1b", "FERRITE_MODEL_MAIN", &[]).expect_err("nothing");
        assert!(err.to_string().contains("none"), "{err}");
    }

    // ── construction, auth and key resolution ────────────────────────────

    #[test]
    fn a_cloud_provider_requires_a_key_and_sends_it() {
        let env = MapEnv::new().with(OLLAMA_API_KEY_VAR, "sk-test");
        let provider = OllamaProvider::from_config(
            &config(DEFAULT_OLLAMA_BASE_URL),
            ModelTier::Small,
            &env,
            &NoSecretStore,
        )
        .expect("key is in the environment");
        assert!(provider.is_authenticated());
        assert_eq!(provider.base_url(), DEFAULT_OLLAMA_BASE_URL);
    }

    #[test]
    fn a_cloud_provider_falls_back_to_the_keyring() {
        let store = MapSecretStore::new().with(KEYRING_SERVICE, OLLAMA_API_KEY_VAR, "sk-keyring");
        let provider = OllamaProvider::from_config(
            &config(DEFAULT_OLLAMA_BASE_URL),
            ModelTier::Main,
            &MapEnv::new(),
            &store,
        )
        .expect("key is in the keyring");
        assert!(provider.is_authenticated());
    }

    #[test]
    fn a_cloud_provider_without_a_key_anywhere_is_a_clear_error_not_a_panic() {
        // The exact state this sandbox is in, and the state `just probe`
        // must fail gracefully from.
        let err = OllamaProvider::from_config(
            &config(DEFAULT_OLLAMA_BASE_URL),
            ModelTier::Small,
            &MapEnv::new(),
            &NoSecretStore,
        )
        .expect_err("no key anywhere");
        assert!(matches!(err, ModelError::MissingApiKey { .. }), "{err:?}");
        assert!(err.to_string().contains(OLLAMA_API_KEY_VAR));
    }

    #[test]
    fn a_local_provider_needs_no_key_and_never_sends_one() {
        let env = MapEnv::new().with(OLLAMA_API_KEY_VAR, "sk-must-not-leave-this-machine");
        let provider = OllamaProvider::from_config(
            &config(LOCAL_OLLAMA_BASE_URL),
            ModelTier::Small,
            &env,
            &NoSecretStore,
        )
        .expect("no key needed locally");
        assert!(
            !provider.is_authenticated(),
            "a bearer token must not be sent to a local endpoint"
        );
    }

    #[test]
    fn a_trailing_slash_in_the_base_url_does_not_double_up_the_path() {
        let provider = OllamaProvider::new(
            "https://ollama.com/",
            Some(Token::new("k")),
            ModelTier::Small,
            1024,
        );
        assert_eq!(provider.base_url(), "https://ollama.com");
    }

    #[test]
    fn the_provider_reports_that_it_reaches_the_network() {
        // R7's mechanical check: a test harness asserting no live provider
        // is wired up relies on this being honest.
        let provider = OllamaProvider::new(DEFAULT_OLLAMA_BASE_URL, None, ModelTier::Main, 1024);
        let caps = provider.capabilities();
        assert!(caps.reaches_network);
        assert!(caps.supports_json_schema);
        assert_eq!(caps.tier, ModelTier::Main);
    }

    #[test]
    fn the_provider_debug_output_never_contains_the_key() {
        let provider = OllamaProvider::new(
            DEFAULT_OLLAMA_BASE_URL,
            Some(Token::new("sk-do-not-log-me")),
            ModelTier::Small,
            1024,
        );
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("sk-do-not-log-me"), "{rendered}");
    }
}
