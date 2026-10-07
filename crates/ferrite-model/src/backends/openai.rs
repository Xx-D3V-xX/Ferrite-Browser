//! OpenAI-compatible — any server that speaks `/chat/completions` (T-277).
//!
//! ```text
//! POST {base}/chat/completions          Authorization: Bearer $FERRITE_OPENAI_API_KEY
//! {
//!   "model": "...",
//!   "messages": [{ "role": "system", ... }, { "role": "user", ... }],
//!   "max_completion_tokens": 128, "temperature": 0, "seed": 42,
//!   "response_format": { "type": "json_schema",
//!                        "json_schema": { "name": "answer", "schema": {...} } }
//! }
//! ```
//!
//! One backend for OpenAI itself and the many servers that copy its wire
//! format (OpenRouter, Groq, Together, vLLM, LM Studio, llama.cpp's server…).
//! `base` is the API root including its version, e.g. `https://api.openai.com/v1`
//! or `http://localhost:1234/v1`. A server on this computer may take no key
//! and is sent none if none is configured; a remote one needs a key.
//!
//! Those servers disagree about which knobs they accept: OpenAI's reasoning
//! models refuse `max_tokens`, `temperature` and `seed`; older servers know
//! `max_tokens` but not `max_completion_tokens`; some have no structured
//! output. Rather than guess per server, a 400 that names one of those
//! parameters is answered by sending the request again without it (or with
//! the older spelling), and the adjustment is remembered for this provider's
//! later calls. Each knob can be given up once, so a call makes at most five
//! requests and an unrelated 400 is returned as it came.

use std::sync::atomic::{AtomicU8, Ordering};

use async_trait::async_trait;

use crate::config::{ModelConfig, is_local_url};
use crate::error::ModelError;
use crate::guard::{self, RawCompletion, bounded};
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, TokenUsage};
use crate::secret::{SecretStore, Token};

use super::http;

/// The env var (and keyring account) holding an OpenAI-compatible key.
pub const OPENAI_API_KEY_VAR: &str = "FERRITE_OPENAI_API_KEY";

/// Which knobs this server has refused, as bits.
const OLD_MAX_TOKENS: u8 = 1;
const NO_TEMPERATURE: u8 = 2;
const NO_SEED: u8 = 4;
const NO_SCHEMA: u8 = 8;

/// Builds a `/chat/completions` body, leaving out what `refused` names.
#[must_use]
pub(crate) fn build_chat_body(req: &CompletionRequest, refused: u8) -> serde_json::Value {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(system) = &req.system_prompt {
        messages.push(serde_json::json!({ "role": "system", "content": system }));
    }
    messages.extend(
        req.messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role.as_str(), "content": m.content })),
    );
    let mut body = serde_json::json!({ "model": req.model_tag, "messages": messages });
    let cap = if refused & OLD_MAX_TOKENS == 0 {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    body[cap] = serde_json::json!(req.options.num_predict);
    if refused & NO_TEMPERATURE == 0 {
        body["temperature"] = serde_json::json!(req.options.temperature);
    }
    if refused & NO_SEED == 0 {
        body["seed"] = serde_json::json!(req.options.seed);
    }
    if let Some(schema) = req
        .format_schema
        .as_ref()
        .filter(|_| refused & NO_SCHEMA == 0)
    {
        body["response_format"] = serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": "answer", "schema": schema }
        });
    }
    body
}

/// Which knob a 400's body says the server refused, if it names one this
/// backend can give up (and has not already).
fn refused_knob(body_excerpt: &str, refused: u8) -> Option<u8> {
    let text = body_excerpt.to_ascii_lowercase();
    // `max_completion_tokens` first: its name contains `max_tokens`.
    [
        ("max_completion_tokens", OLD_MAX_TOKENS),
        ("max_tokens", 0),
        ("temperature", NO_TEMPERATURE),
        ("seed", NO_SEED),
        ("response_format", NO_SCHEMA),
        ("json_schema", NO_SCHEMA),
    ]
    .into_iter()
    .find(|(name, _)| text.contains(name))
    .map(|(_, bit)| bit)
    .filter(|bit| *bit != 0 && refused & bit == 0)
}

/// Reads the answer and token counts out of a `/chat/completions` response.
///
/// # Errors
///
/// [`ModelError::MalformedJson`] if the body is not JSON or has no choice.
pub(crate) fn parse_chat_response(body: &[u8]) -> Result<RawCompletion, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::OpenAi,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;
    let message = json
        .pointer("/choices/0/message")
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::OpenAi,
            detail: format!(
                "no /choices/0/message (body: {})",
                bounded(&json.to_string())
            ),
        })?;
    // A refusal (OpenAI's structured-output `refusal` field) has no content:
    // the guard turns that into an empty answer, as for any other backend.
    let content = message
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let count = |field: &str| {
        json.pointer(&format!("/usage/{field}"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0)
    };
    Ok(RawCompletion {
        content,
        usage: TokenUsage {
            prompt_eval_count: count("prompt_tokens"),
            eval_count: count("completion_tokens"),
        },
    })
}

/// Parses `GET {base}/models`: every model id, sorted.
pub(crate) fn parse_models_response(body: &[u8]) -> Result<Vec<String>, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::OpenAi,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;
    let data = json
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::OpenAi,
            detail: format!("no \"data\" array (body: {})", bounded(&json.to_string())),
        })?;
    let mut ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The OpenAI-compatible backend.
#[derive(Debug)]
pub struct OpenAiProvider {
    base_url: String,
    auth: Option<Token>,
    model_tier: ModelTier,
    max_response_bytes: usize,
    refused: AtomicU8,
    client: reqwest::Client,
}

impl OpenAiProvider {
    /// Builds a provider directly. `auth` is `None` for a server that takes
    /// no key.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        auth: Option<Token>,
        model_tier: ModelTier,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth,
            model_tier,
            max_response_bytes,
            refused: AtomicU8::new(0),
            client: reqwest::Client::new(),
        }
    }

    /// Builds a provider from loaded configuration. A remote server needs a
    /// key from the environment or the OS keyring; a server on this computer
    /// gets one only if one is configured.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingApiKey`] for a remote server with no key.
    pub fn from_config(
        config: &ModelConfig,
        tier: ModelTier,
        env: &dyn crate::config::EnvSource,
        store: &dyn SecretStore,
    ) -> Result<Self, ModelError> {
        let key = crate::secret::resolve(env, store, OPENAI_API_KEY_VAR);
        let auth = if is_local_url(&config.openai_base_url) {
            key.ok()
        } else {
            Some(key?)
        };
        Ok(Self::new(
            &config.openai_base_url,
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

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let builder = self
            .client
            .request(method, format!("{}{path}", self.base_url));
        match &self.auth {
            Some(token) => builder.bearer_auth(token.expose()),
            None => builder,
        }
    }

    fn transport(e: &reqwest::Error) -> ModelError {
        ModelError::Transport {
            provider: ProviderId::OpenAi,
            detail: bounded(&e.to_string()),
        }
    }

    /// Every model id the server offers, sorted. Doubles as the key check.
    ///
    /// # Errors
    ///
    /// Transport, status or parse failures, each typed.
    pub async fn list_models(&self) -> Result<Vec<String>, ModelError> {
        let response = self
            .request(reqwest::Method::GET, "/models")
            .send()
            .await
            .map_err(|e| Self::transport(&e))?;
        let response =
            http::classify(ProviderId::OpenAi, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::OpenAi, response, self.max_response_bytes).await?;
        parse_models_response(&body)
    }
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OpenAi
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        loop {
            let refused = self.refused.load(Ordering::Relaxed);
            let response = self
                .request(reqwest::Method::POST, "/chat/completions")
                .json(&build_chat_body(&req, refused))
                .send()
                .await
                .map_err(|e| Self::transport(&e))?;
            match http::classify(ProviderId::OpenAi, response, self.max_response_bytes).await {
                Ok(response) => {
                    let body =
                        http::read_bounded(ProviderId::OpenAi, response, self.max_response_bytes)
                            .await?;
                    let raw = parse_chat_response(&body)?;
                    return guard::finalize(ProviderId::OpenAi, &req, raw, self.max_response_bytes);
                }
                Err(ModelError::ClientError {
                    status: 400,
                    body_excerpt,
                    provider,
                }) => match refused_knob(&body_excerpt, refused) {
                    Some(bit) => {
                        self.refused.fetch_or(bit, Ordering::Relaxed);
                    }
                    None => {
                        return Err(ModelError::ClientError {
                            provider,
                            status: 400,
                            body_excerpt,
                        });
                    }
                },
                Err(other) => return Err(other),
            }
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_json_schema: self.refused.load(Ordering::Relaxed) & NO_SCHEMA == 0,
            context_window_tokens: 32_768,
            tier: self.model_tier,
            reaches_network: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_OPENAI_BASE_URL, MapEnv};
    use crate::request::{Message, SamplingOptions};
    use crate::secret::{KEYRING_SERVICE, MapSecretStore, NoSecretStore};

    fn req() -> CompletionRequest {
        CompletionRequest::new(
            "some-model",
            ModelTier::Small,
            vec![
                Message::user("labels?"),
                Message::assistant("ok"),
                Message::user("now"),
            ],
        )
        .with_system_prompt("You are Ferrite.", 3)
        .with_options(SamplingOptions::default())
    }

    #[test]
    fn the_system_prompt_leads_the_messages_and_every_knob_is_sent_at_first() {
        let schema = serde_json::json!({"type": "array", "items": {"type": "string"}});
        let body = build_chat_body(&req().with_format_schema(schema.clone()), 0);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are Ferrite.");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["max_completion_tokens"], 128);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["seed"], 42);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    }

    #[test]
    fn refused_knobs_are_left_out_or_respelled() {
        let schema = serde_json::json!({"type": "object"});
        let body = build_chat_body(
            &req().with_format_schema(schema),
            OLD_MAX_TOKENS | NO_TEMPERATURE | NO_SEED | NO_SCHEMA,
        );
        assert_eq!(body["max_tokens"], 128);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("seed").is_none());
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn a_400_naming_a_knob_gives_up_that_knob_once_and_nothing_else() {
        let openai = r#"{"error":{"message":"Unsupported parameter: 'max_completion_tokens' is not supported with this model. Use 'max_tokens' instead.","type":"invalid_request_error","param":"max_completion_tokens"}}"#;
        assert_eq!(refused_knob(openai, 0), Some(OLD_MAX_TOKENS));
        let reasoning = r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0 with this model."}}"#;
        assert_eq!(refused_knob(reasoning, 0), Some(NO_TEMPERATURE));
        assert_eq!(refused_knob(reasoning, NO_TEMPERATURE), None, "only once");
        assert_eq!(
            refused_knob(
                r#"{"error":"response_format json_schema is not supported"}"#,
                0
            ),
            Some(NO_SCHEMA)
        );
        // Naming the old spelling is not a reason to give anything up.
        assert_eq!(refused_knob("max_tokens must be at least 1", 0), None);
        assert_eq!(refused_knob(r#"{"error":"model not found"}"#, 0), None);
    }

    #[test]
    fn a_chat_response_parses_into_content_and_token_counts() {
        let body = br#"{"id":"c1","object":"chat.completion","choices":[{"index":0,
            "message":{"role":"assistant","content":"[\"read_page\"]"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":40,"completion_tokens":6,"total_tokens":46}}"#;
        let raw = parse_chat_response(body).expect("parses");
        assert_eq!(raw.content, r#"["read_page"]"#);
        assert_eq!(raw.usage.prompt_eval_count, 40);
        assert_eq!(raw.usage.eval_count, 6);
    }

    #[test]
    fn a_refusal_or_null_content_is_an_empty_answer_not_a_panic() {
        let body = br#"{"choices":[{"message":{"role":"assistant","content":null,"refusal":"I can't help with that."}}]}"#;
        let raw = parse_chat_response(body).expect("parses");
        assert!(matches!(
            guard::finalize(ProviderId::OpenAi, &req(), raw, 1024),
            Err(ModelError::EmptyResponse { .. })
        ));
        assert!(matches!(
            parse_chat_response(br#"{"choices":[]}"#),
            Err(ModelError::MalformedJson { .. })
        ));
    }

    #[test]
    fn the_model_list_is_every_id_sorted() {
        let body = br#"{"object":"list","data":[{"id":"zeta","object":"model"},{"id":"alpha","object":"model"},{"id":"alpha"}]}"#;
        assert_eq!(
            parse_models_response(body).expect("parses"),
            vec!["alpha", "zeta"]
        );
        assert!(parse_models_response(br#"{"models":[]}"#).is_err());
    }

    fn config(base: &str) -> ModelConfig {
        ModelConfig::load(
            &MapEnv::new()
                .with("FERRITE_MODEL_SMALL", "s")
                .with("FERRITE_MODEL_MAIN", "m")
                .with("FERRITE_OPENAI_BASE_URL", base)
                .with("FERRITE_MODEL_CACHE_DIR", "/tmp/ferrite-model-openai-test"),
        )
        .expect("loads")
    }

    #[test]
    fn a_remote_server_needs_a_key_and_a_local_one_does_not() {
        let remote = config(DEFAULT_OPENAI_BASE_URL);
        let err =
            OpenAiProvider::from_config(&remote, ModelTier::Small, &MapEnv::new(), &NoSecretStore)
                .expect_err("no key");
        assert!(err.to_string().contains(OPENAI_API_KEY_VAR), "{err}");
        let keyed = OpenAiProvider::from_config(
            &remote,
            ModelTier::Small,
            &MapEnv::new(),
            &MapSecretStore::new().with(KEYRING_SERVICE, OPENAI_API_KEY_VAR, "k"),
        )
        .expect("keyring");
        assert!(keyed.auth.is_some());

        let local = config("http://localhost:1234/v1");
        let keyless =
            OpenAiProvider::from_config(&local, ModelTier::Main, &MapEnv::new(), &NoSecretStore)
                .expect("local needs none");
        assert!(keyless.auth.is_none());
        assert_eq!(keyless.base_url(), "http://localhost:1234/v1");
    }

    #[test]
    fn the_provider_debug_output_never_contains_the_key() {
        let provider = OpenAiProvider::new(
            DEFAULT_OPENAI_BASE_URL,
            Some(Token::new("sk-do-not-log-me")),
            ModelTier::Small,
            1024,
        );
        assert!(!format!("{provider:?}").contains("sk-do-not-log-me"));
    }

    #[tokio::test]
    async fn a_refused_knob_is_given_up_once_and_remembered() {
        use super::super::fake_server::{body_of, scripted};
        let ok = r#"{"choices":[{"message":{"role":"assistant","content":"done"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
        let server = scripted(vec![
            (400, r#"{"error":{"message":"Unsupported parameter: 'max_completion_tokens'. Use 'max_tokens'.","param":"max_completion_tokens"}}"#.into()),
            (400, r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0"}}"#.into()),
            (200, ok.into()),
        ])
        .await;
        let provider = OpenAiProvider::new(
            &server.url,
            Some(Token::new("sk-x")),
            ModelTier::Small,
            1 << 20,
        );
        let first = provider.complete(req()).await.expect("third try works");
        assert_eq!(first.content, "done");
        provider.complete(req()).await.expect("again");
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "the second call goes straight through");
        assert!(body_of(&seen[0]).get("max_completion_tokens").is_some());
        assert!(body_of(&seen[1]).get("max_tokens").is_some());
        assert!(body_of(&seen[2]).get("temperature").is_none());
        assert!(
            body_of(&seen[3]).get("temperature").is_none()
                && body_of(&seen[3]).get("max_tokens").is_some()
        );
        assert!(
            seen[0]
                .to_lowercase()
                .contains("authorization: bearer sk-x")
        );
    }

    #[tokio::test]
    async fn an_unrelated_400_is_returned_as_it_came() {
        use super::super::fake_server::scripted;
        let server = scripted(vec![(400, r#"{"error":"model not found"}"#.into())]).await;
        let provider = OpenAiProvider::new(&server.url, None, ModelTier::Small, 1 << 20);
        let err = provider.complete(req()).await.expect_err("400");
        assert!(
            matches!(err, ModelError::ClientError { status: 400, .. }),
            "{err:?}"
        );
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            !seen[0].to_lowercase().contains("authorization"),
            "no key, no header"
        );
    }
}
