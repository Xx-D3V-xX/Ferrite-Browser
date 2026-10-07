//! Anthropic — Claude models over the Messages API (T-277).
//!
//! ```text
//! POST {base}/v1/messages
//! x-api-key: $FERRITE_ANTHROPIC_API_KEY
//! anthropic-version: 2023-06-01
//! {
//!   "model":      "<the configured tag>",
//!   "max_tokens": 1024,
//!   "system":     "...",
//!   "messages":   [{ "role": "user", "content": "..." }],
//!   "output_config": { "format": { "type": "json_schema", "schema": {...} } }
//! }
//! ```
//!
//! Raw HTTP rather than an SDK: Anthropic publishes no Rust SDK, and this crate
//! already owns one bounded, typed HTTP path that every backend shares
//! ([`super::http`]).
//!
//! Three differences from the other backends that the mapping absorbs:
//!
//! - **No sampling knobs are sent.** Current Claude models reject a
//!   non-default `temperature` (and have no `seed`), so sending R8's
//!   `temperature = 0` would make every call a 400. The request's own
//!   options still say `temperature = 0`, which keeps it cacheable; what the
//!   cache then holds is the first answer, not a reproducible one.
//! - **`max_tokens` has a floor.** On models that think, thinking counts
//!   against `max_tokens`, so the 128-token cap the fingerprint call uses
//!   would end most answers before any text. The cap is
//!   `max(num_predict, MIN_MAX_TOKENS)`: still a bound on every call (§10.3),
//!   one a thinking model can answer inside.
//! - **A refusal is an empty answer.** `stop_reason: "refusal"` arrives as
//!   HTTP 200 with no text; it becomes [`ModelError::EmptyResponse`] through
//!   the shared guard, which the fingerprint layer already treats as "predict
//!   nothing" (fail to empty, never to a bypass).

use async_trait::async_trait;

use crate::config::ModelConfig;
use crate::error::ModelError;
use crate::guard::{self, RawCompletion, bounded};
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::{CompletionRequest, Role};
use crate::response::{CompletionResponse, TokenUsage};
use crate::secret::{SecretStore, Token};

use super::http;

/// The env var (and keyring account) holding an Anthropic key.
pub const ANTHROPIC_API_KEY_VAR: &str = "FERRITE_ANTHROPIC_API_KEY";

/// The API version every request names.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The smallest `max_tokens` sent; see the module docs.
const MIN_MAX_TOKENS: u32 = 1024;

/// Builds a Messages API request body.
#[must_use]
pub(crate) fn build_messages_body(req: &CompletionRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                // A system turn in the list is a caller's mistake; keeping its
                // text as a user turn is better than dropping it.
                Role::User | Role::System => "user",
                Role::Assistant => "assistant",
            };
            serde_json::json!({ "role": role, "content": m.content })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": req.model_tag,
        "max_tokens": req.options.num_predict.max(MIN_MAX_TOKENS),
        "messages": messages,
    });
    if let Some(system) = &req.system_prompt {
        body["system"] = serde_json::json!(system);
    }
    if let Some(schema) = &req.format_schema {
        body["output_config"] = serde_json::json!({
            "format": { "type": "json_schema", "schema": schema }
        });
    }
    body
}

/// Reads the answer and token counts out of a Messages API response.
///
/// Every `text` block is joined: an answer may come in more than one, and a
/// model that thinks puts `thinking` blocks before them, which are not the
/// answer and are skipped.
///
/// # Errors
///
/// [`ModelError::MalformedJson`] if the body is not JSON or has no `content`
/// array.
pub(crate) fn parse_messages_response(body: &[u8]) -> Result<RawCompletion, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Anthropic,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;
    let blocks = json
        .get("content")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::Anthropic,
            detail: format!(
                "no \"content\" array (body: {})",
                bounded(&json.to_string())
            ),
        })?;
    let content: String = blocks
        .iter()
        .filter(|b| b.get("type").and_then(serde_json::Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(serde_json::Value::as_str))
        .collect();
    let count = |field: &str| {
        json.pointer(&format!("/usage/{field}"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0)
    };
    Ok(RawCompletion {
        content,
        usage: TokenUsage {
            prompt_eval_count: count("input_tokens"),
            eval_count: count("output_tokens"),
        },
    })
}

/// Parses `GET /v1/models`: every model id, sorted, and the id to continue
/// after when the list has more pages.
pub(crate) fn parse_models_page(body: &[u8]) -> Result<(Vec<String>, Option<String>), ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Anthropic,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;
    let data = json
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::Anthropic,
            detail: format!("no \"data\" array (body: {})", bounded(&json.to_string())),
        })?;
    let ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();
    let more = json
        .get("has_more")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let next = more
        .then(|| json.get("last_id").and_then(serde_json::Value::as_str))
        .flatten()
        .map(str::to_string);
    Ok((ids, next))
}

/// The Anthropic backend.
#[derive(Debug)]
pub struct AnthropicProvider {
    base_url: String,
    api_key: Token,
    model_tier: ModelTier,
    max_response_bytes: usize,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// Builds a provider directly. `base_url` is the API root
    /// (`https://api.anthropic.com`), without `/v1`.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: Token,
        model_tier: ModelTier,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            model_tier,
            max_response_bytes,
            client: reqwest::Client::new(),
        }
    }

    /// Builds a provider from loaded configuration, resolving the key from
    /// the environment or the OS keyring.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingApiKey`] when neither source has a key.
    pub fn from_config(
        config: &ModelConfig,
        tier: ModelTier,
        env: &dyn crate::config::EnvSource,
        store: &dyn SecretStore,
    ) -> Result<Self, ModelError> {
        let api_key = crate::secret::resolve(env, store, ANTHROPIC_API_KEY_VAR)?;
        Ok(Self::new(
            &config.anthropic_base_url,
            api_key,
            tier,
            config.max_response_bytes,
        ))
    }

    /// The configured endpoint.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base_url))
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
    }

    fn transport(e: &reqwest::Error) -> ModelError {
        // The key is in a header, never in the URL, so a transport error
        // cannot quote it back.
        ModelError::Transport {
            provider: ProviderId::Anthropic,
            detail: bounded(&e.to_string()),
        }
    }

    /// Every model id this key may call, sorted — what the Settings screen
    /// offers. Doubles as the key check.
    ///
    /// # Errors
    ///
    /// Transport, status or parse failures, each typed.
    pub async fn list_models(&self) -> Result<Vec<String>, ModelError> {
        let mut names = Vec::new();
        let mut after: Option<String> = None;
        // A bound on pages, so a server that always says "more" cannot loop us.
        for _ in 0..20 {
            let mut request = self
                .request(reqwest::Method::GET, "/v1/models")
                .query(&[("limit", "1000")]);
            if let Some(id) = &after {
                request = request.query(&[("after_id", id.as_str())]);
            }
            let response = request.send().await.map_err(|e| Self::transport(&e))?;
            let response =
                http::classify(ProviderId::Anthropic, response, self.max_response_bytes).await?;
            let body = http::read_bounded(ProviderId::Anthropic, response, self.max_response_bytes)
                .await?;
            let (page, next) = parse_models_page(&body)?;
            names.extend(page);
            match next {
                Some(id) => after = Some(id),
                None => break,
            }
        }
        names.sort_unstable();
        names.dedup();
        Ok(names)
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let response = self
            .request(reqwest::Method::POST, "/v1/messages")
            .json(&build_messages_body(&req))
            .send()
            .await
            .map_err(|e| Self::transport(&e))?;
        let response =
            http::classify(ProviderId::Anthropic, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Anthropic, response, self.max_response_bytes).await?;
        let raw = parse_messages_response(&body)?;
        guard::finalize(ProviderId::Anthropic, &req, raw, self.max_response_bytes)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_json_schema: true,
            context_window_tokens: 200_000,
            tier: self.model_tier,
            reaches_network: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_ANTHROPIC_BASE_URL, MapEnv};
    use crate::request::{Message, SamplingOptions};
    use crate::secret::{KEYRING_SERVICE, MapSecretStore, NoSecretStore};

    fn req() -> CompletionRequest {
        CompletionRequest::new(
            "a-claude-model",
            ModelTier::Small,
            vec![
                Message::user("what does this page let me do?"),
                Message::assistant("Reading it now."),
                Message::user("just the labels, please"),
            ],
        )
        .with_system_prompt("You are Ferrite.", 3)
    }

    #[test]
    fn the_system_prompt_is_a_top_level_field_and_roles_keep_their_names() {
        let body = build_messages_body(&req());
        assert_eq!(body["system"], "You are Ferrite.");
        assert_eq!(body["model"], "a-claude-model");
        let messages = body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 3, "the system prompt is not also a message");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["content"], "just the labels, please");
    }

    #[test]
    fn no_sampling_knob_is_sent_and_max_tokens_has_a_floor() {
        let body = build_messages_body(&req().with_options(SamplingOptions {
            temperature: 0.0,
            seed: 42,
            num_predict: 128,
        }));
        assert!(
            body.get("temperature").is_none(),
            "current models reject it"
        );
        assert!(body.get("seed").is_none());
        assert_eq!(body["max_tokens"], MIN_MAX_TOKENS);
        let big = build_messages_body(
            &req().with_options(SamplingOptions::default().with_num_predict(4096)),
        );
        assert_eq!(big["max_tokens"], 4096, "a larger cap is kept as asked");
    }

    #[test]
    fn a_schema_becomes_a_json_schema_output_format() {
        let schema = serde_json::json!({"type": "array", "items": {"type": "string"}});
        let body = build_messages_body(&req().with_format_schema(schema.clone()));
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["format"]["schema"], schema);
        assert!(build_messages_body(&req()).get("output_config").is_none());
    }

    #[test]
    fn text_blocks_are_joined_and_thinking_is_skipped() {
        let body = br#"{
          "id": "msg_1", "type": "message", "role": "assistant",
          "content": [
            {"type": "thinking", "thinking": "", "signature": "x"},
            {"type": "text", "text": "[\"read_"},
            {"type": "text", "text": "page\"]"}
          ],
          "stop_reason": "end_turn",
          "usage": {"input_tokens": 57, "output_tokens": 8}
        }"#;
        let raw = parse_messages_response(body).expect("parses");
        assert_eq!(raw.content, r#"["read_page"]"#);
        assert_eq!(raw.usage.prompt_eval_count, 57);
        assert_eq!(raw.usage.eval_count, 8);
    }

    #[test]
    fn a_refusal_has_no_text_and_finalizes_as_empty() {
        let body = br#"{"content": [], "stop_reason": "refusal",
                        "stop_details": {"type": "refusal", "category": "cyber"},
                        "usage": {"input_tokens": 9, "output_tokens": 0}}"#;
        let raw = parse_messages_response(body).expect("parses");
        let err = guard::finalize(ProviderId::Anthropic, &req(), raw, 1024).expect_err("empty");
        assert!(matches!(err, ModelError::EmptyResponse { .. }), "{err:?}");
    }

    #[test]
    fn a_body_of_the_wrong_shape_is_malformed_not_a_panic() {
        assert!(matches!(
            parse_messages_response(br#"{"type":"error"}"#),
            Err(ModelError::MalformedJson { .. })
        ));
        assert!(parse_messages_response(b"not json").is_err());
    }

    #[test]
    fn a_model_page_reads_ids_and_where_to_continue() {
        let body = br#"{"data":[{"id":"b-model","type":"model"},{"id":"a-model","type":"model"}],
                        "has_more":true,"first_id":"b-model","last_id":"a-model"}"#;
        let (ids, next) = parse_models_page(body).expect("parses");
        assert_eq!(ids, vec!["b-model", "a-model"]);
        assert_eq!(next.as_deref(), Some("a-model"));
        let (_, last) =
            parse_models_page(br#"{"data":[],"has_more":false,"last_id":null}"#).expect("parses");
        assert_eq!(last, None);
        assert!(parse_models_page(br#"{"models":[]}"#).is_err());
    }

    #[test]
    fn a_key_is_required_and_resolves_from_env_or_keyring() {
        let config = ModelConfig::load(
            &MapEnv::new()
                .with("FERRITE_MODEL_SMALL", "s")
                .with("FERRITE_MODEL_MAIN", "m")
                .with(
                    "FERRITE_MODEL_CACHE_DIR",
                    "/tmp/ferrite-model-anthropic-test",
                ),
        )
        .expect("loads");
        let from_env = AnthropicProvider::from_config(
            &config,
            ModelTier::Small,
            &MapEnv::new().with(ANTHROPIC_API_KEY_VAR, "k-env"),
            &NoSecretStore,
        )
        .expect("env");
        assert_eq!(from_env.base_url(), DEFAULT_ANTHROPIC_BASE_URL);
        AnthropicProvider::from_config(
            &config,
            ModelTier::Main,
            &MapEnv::new(),
            &MapSecretStore::new().with(KEYRING_SERVICE, ANTHROPIC_API_KEY_VAR, "k-keyring"),
        )
        .expect("keyring");
        let err = AnthropicProvider::from_config(
            &config,
            ModelTier::Small,
            &MapEnv::new(),
            &NoSecretStore,
        )
        .expect_err("no key");
        assert!(err.to_string().contains(ANTHROPIC_API_KEY_VAR), "{err}");
    }

    #[test]
    fn the_provider_debug_output_never_contains_the_key() {
        let provider = AnthropicProvider::new(
            DEFAULT_ANTHROPIC_BASE_URL,
            Token::new("sk-ant-do-not-log-me"),
            ModelTier::Small,
            1024,
        );
        assert!(!format!("{provider:?}").contains("sk-ant-do-not-log-me"));
        assert!(provider.capabilities().reaches_network);
    }

    #[tokio::test]
    async fn a_completion_over_http_sends_the_key_and_version_headers() {
        use super::super::fake_server::{body_of, scripted};
        let server = scripted(vec![(
            200,
            r#"{"content":[{"type":"text","text":"[\"read_page\"]"}],"stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":3}}"#.into(),
        )])
        .await;
        let provider = AnthropicProvider::new(
            &server.url,
            Token::new("sk-ant-k"),
            ModelTier::Small,
            1 << 20,
        );
        let schema = serde_json::json!({"type":"array","items":{"type":"string"}});
        let response = provider
            .complete(req().with_format_schema(schema))
            .await
            .expect("completes");
        assert_eq!(response.structured, Some(serde_json::json!(["read_page"])));
        assert_eq!(response.provenance.provider, ProviderId::Anthropic);
        let request = server.seen.lock().unwrap()[0].clone();
        let lower = request.to_lowercase();
        assert!(lower.starts_with("post /v1/messages"), "{request}");
        assert!(lower.contains("x-api-key: sk-ant-k"), "{request}");
        assert!(lower.contains("anthropic-version: 2023-06-01"), "{request}");
        assert_eq!(
            body_of(&request)["output_config"]["format"]["type"],
            "json_schema"
        );
    }

    #[tokio::test]
    async fn model_listing_follows_the_pages() {
        use super::super::fake_server::scripted;
        let server = scripted(vec![
            (
                200,
                r#"{"data":[{"id":"m-b"}],"has_more":true,"last_id":"m-b"}"#.into(),
            ),
            (
                200,
                r#"{"data":[{"id":"m-a"}],"has_more":false,"last_id":"m-a"}"#.into(),
            ),
        ])
        .await;
        let provider =
            AnthropicProvider::new(&server.url, Token::new("k"), ModelTier::Small, 1 << 20);
        assert_eq!(
            provider.list_models().await.expect("lists"),
            vec!["m-a", "m-b"]
        );
        let seen = server.seen.lock().unwrap();
        assert!(seen[1].contains("after_id=m-b"), "{}", seen[1]);
    }
}
