//! Gemini — the second remote backend (§10.1's "Gemini is the second remote
//! backend").
//!
//! ```text
//! POST {base}/{model}:generateContent?key=$FERRITE_GEMINI_API_KEY
//! {
//!   "system_instruction": { "parts": [{ "text": ... }] },
//!   "contents":           [{ "role": "user", "parts": [{ "text": ... }] }],
//!   "generationConfig":   { "temperature": 0, "seed": 42, "maxOutputTokens": 128 }
//! }
//! ```
//!
//! Two differences from Ollama that the mapping has to absorb:
//!
//! - the assistant turn is spelled `model`, not `assistant`;
//! - the system prompt is its own top-level field rather than a message.
//!
//! Structured output is `responseMimeType: "application/json"` plus
//! `responseSchema`, which plays the role Ollama's `format` does.
//!
//! Token counts arrive as `usageMetadata.promptTokenCount` /
//! `candidatesTokenCount` and are mapped onto Ollama's names, because §13.2's
//! overhead metric is defined against those and should not have to know which
//! backend produced a record.
//!
//! **No function calling.** The pre-rebuild `ferrite-agent/src/gemini.rs`
//! carries a tool-manifest/tool-loop; that belongs to the agent loop (A9),
//! which consumes this trait rather than being implemented inside it.

use async_trait::async_trait;

use crate::config::ModelConfig;
use crate::error::ModelError;
use crate::guard::{self, RawCompletion, bounded};
use crate::provider::{ModelProvider, ModelTier, ProviderCapabilities, ProviderId};
use crate::request::{CompletionRequest, Role};
use crate::response::{CompletionResponse, TokenUsage};
use crate::secret::{SecretStore, Token};

use super::http;

/// The env var holding a Gemini key. Same two-source rule as §10.1's
/// `OLLAMA_API_KEY`: environment or OS keyring, never a file.
pub const GEMINI_API_KEY_VAR: &str = "FERRITE_GEMINI_API_KEY";

/// Gemini's spelling of a conversation role.
fn gemini_role(role: Role) -> &'static str {
    match role {
        // A system turn appearing in the message list at all is a caller
        // that put it in the wrong place; mapping it to `user` keeps its
        // text in the prompt rather than dropping it silently.
        Role::User | Role::System => "user",
        Role::Assistant => "model",
    }
}

/// Builds a `generateContent` request body.
#[must_use]
pub(crate) fn build_generate_body(req: &CompletionRequest) -> serde_json::Value {
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": gemini_role(m.role),
                "parts": [{ "text": m.content }],
            })
        })
        .collect();

    let mut generation_config = serde_json::json!({
        "temperature": req.options.temperature,
        "seed": req.options.seed,
        "maxOutputTokens": req.options.num_predict,
    });
    if let Some(schema) = &req.format_schema {
        generation_config["responseMimeType"] = serde_json::json!("application/json");
        generation_config["responseSchema"] = schema.clone();
    }

    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": generation_config,
    });
    if let Some(system) = &req.system_prompt {
        body["system_instruction"] = serde_json::json!({ "parts": [{ "text": system }] });
    }
    body
}

/// Reads the answer and token counts out of a `generateContent` response.
///
/// Every `text` part of the first candidate is concatenated: Gemini may
/// split one answer across parts, and taking only the first would silently
/// truncate a structured document into invalid JSON.
///
/// # Errors
///
/// [`ModelError::MalformedJson`] if the body is not JSON or has no candidate
/// content.
pub(crate) fn parse_generate_response(body: &[u8]) -> Result<RawCompletion, ModelError> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Gemini,
            detail: format!("{} (body: {})", e, bounded(&String::from_utf8_lossy(body))),
        })?;

    let parts = json
        .pointer("/candidates/0/content/parts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ModelError::MalformedJson {
            provider: ProviderId::Gemini,
            detail: format!(
                "no array at /candidates/0/content/parts (body: {})",
                bounded(&json.to_string())
            ),
        })?;

    let content: String = parts
        .iter()
        .filter_map(|p| p.get("text").and_then(serde_json::Value::as_str))
        .collect();

    Ok(RawCompletion {
        content,
        usage: TokenUsage {
            prompt_eval_count: count(&json, "promptTokenCount"),
            eval_count: count(&json, "candidatesTokenCount"),
        },
    })
}

fn count(json: &serde_json::Value, field: &str) -> u32 {
    json.pointer(&format!("/usageMetadata/{field}"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}

/// The Gemini backend.
#[derive(Debug)]
pub struct GeminiProvider {
    base_url: String,
    api_key: Token,
    model_tier: ModelTier,
    max_response_bytes: usize,
    client: reqwest::Client,
}

impl GeminiProvider {
    /// Builds a provider directly.
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
    /// [`ModelError::MissingApiKey`] when neither source has a key. There is
    /// no local Gemini, so unlike Ollama there is no keyless path.
    pub fn from_config(
        config: &ModelConfig,
        tier: ModelTier,
        env: &dyn crate::config::EnvSource,
        store: &dyn SecretStore,
    ) -> Result<Self, ModelError> {
        let api_key = crate::secret::resolve(env, store, GEMINI_API_KEY_VAR)?;
        Ok(Self::new(
            &config.gemini_base_url,
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

    /// The provider's raw, unparsed wire body for `req` — what `just record`
    /// hands to [`crate::fixtures::write`]. See
    /// [`OllamaProvider::fetch_wire`](crate::backends::OllamaProvider::fetch_wire)
    /// for why this exists alongside [`ModelProvider::complete`] rather than
    /// being derived from it.
    ///
    /// # Errors
    ///
    /// The same transport/status/size errors `complete` can return, plus
    /// [`ModelError::MalformedJson`] if the body is not JSON at all.
    pub async fn fetch_wire(
        &self,
        req: &CompletionRequest,
    ) -> Result<serde_json::Value, ModelError> {
        let url = format!("{}/{}:generateContent", self.base_url, req.model_tag);
        let response = self
            .client
            .post(&url)
            .query(&[("key", self.api_key.expose())])
            .json(&build_generate_body(req))
            .send()
            .await
            .map_err(|e| ModelError::Transport {
                provider: ProviderId::Gemini,
                detail: bounded(&redact(&e.to_string(), self.api_key.expose())),
            })?;
        let response =
            http::classify(ProviderId::Gemini, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Gemini, response, self.max_response_bytes).await?;
        serde_json::from_slice(&body).map_err(|e| ModelError::MalformedJson {
            provider: ProviderId::Gemini,
            detail: bounded(&e.to_string()),
        })
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Gemini
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let url = format!("{}/{}:generateContent", self.base_url, req.model_tag);
        let response = self
            .client
            .post(&url)
            // As a query parameter because that is what the API takes. It is
            // therefore in the URL, which is why no code in this crate ever
            // puts a URL in an error message.
            .query(&[("key", self.api_key.expose())])
            .json(&build_generate_body(&req))
            .send()
            .await
            .map_err(|e| ModelError::Transport {
                provider: ProviderId::Gemini,
                detail: bounded(&redact(&e.to_string(), self.api_key.expose())),
            })?;

        let response =
            http::classify(ProviderId::Gemini, response, self.max_response_bytes).await?;
        let body =
            http::read_bounded(ProviderId::Gemini, response, self.max_response_bytes).await?;
        let raw = parse_generate_response(&body)?;
        guard::finalize(ProviderId::Gemini, &req, raw, self.max_response_bytes)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_json_schema: true,
            context_window_tokens: 32_768,
            tier: self.model_tier,
            reaches_network: true,
        }
    }
}

/// Removes a key that a transport error quoted back out of the request URL.
///
/// `reqwest` includes the URL in some error messages, and Gemini's key is a
/// query parameter — so the one provider whose key *can* end up in an error
/// string gets it stripped here rather than relying on nobody logging it.
fn redact(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        return message.to_string();
    }
    message.replace(secret, "<redacted>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_GEMINI_BASE_URL, MapEnv};
    use crate::request::{Message, SamplingOptions};
    use crate::secret::{KEYRING_SERVICE, MapSecretStore, NoSecretStore};

    fn req() -> CompletionRequest {
        CompletionRequest::new(
            "gemini-2.5-flash",
            ModelTier::Small,
            vec![
                Message::user("what does this page let me do?"),
                Message::assistant("Reading it now."),
                Message::user("just the labels, please"),
            ],
        )
        .with_system_prompt("You are Ferrite.", 3)
    }

    fn config() -> ModelConfig {
        ModelConfig::load(
            &MapEnv::new()
                .with("FERRITE_MODEL_SMALL", "gemini-2.5-flash")
                .with("FERRITE_MODEL_MAIN", "gemini-2.5-pro")
                .with("FERRITE_MODEL_CACHE_DIR", "/tmp/ferrite-model-gemini-test"),
        )
        .expect("loads")
    }

    #[test]
    fn the_system_prompt_becomes_a_top_level_instruction() {
        let body = build_generate_body(&req());
        assert_eq!(
            body["system_instruction"]["parts"][0]["text"],
            "You are Ferrite."
        );
        assert_eq!(
            body["contents"].as_array().expect("array").len(),
            3,
            "the system prompt must not also appear as a message"
        );
    }

    #[test]
    fn the_assistant_turn_is_spelled_model() {
        let body = build_generate_body(&req());
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["contents"][2]["role"], "user");
    }

    #[test]
    fn sampling_knobs_map_onto_generation_config() {
        let body = build_generate_body(&req().with_options(SamplingOptions {
            temperature: 0.0,
            seed: 42,
            num_predict: 128,
        }));
        assert_eq!(body["generationConfig"]["temperature"], 0.0);
        assert_eq!(body["generationConfig"]["seed"], 42);
        assert_eq!(
            body["generationConfig"]["maxOutputTokens"], 128,
            "Ollama's num_predict is Gemini's maxOutputTokens"
        );
    }

    #[test]
    fn a_schema_sets_both_the_mime_type_and_the_schema() {
        let schema = serde_json::json!({"type": "array", "items": {"type": "string"}});
        let body = build_generate_body(&req().with_format_schema(schema.clone()));
        assert_eq!(
            body["generationConfig"]["responseMimeType"], "application/json",
            "the schema alone does not constrain decoding; the mime type is what switches it on"
        );
        assert_eq!(body["generationConfig"]["responseSchema"], schema);
    }

    #[test]
    fn no_schema_means_no_json_mime_type() {
        let body = build_generate_body(&req());
        assert!(body["generationConfig"].get("responseMimeType").is_none());
        assert!(body["generationConfig"].get("responseSchema").is_none());
    }

    #[test]
    fn a_real_generate_response_parses_into_content_and_token_counts() {
        let body = br#"{
          "candidates": [{
            "content": { "parts": [{"text": "[\"read_page\"]"}], "role": "model" },
            "finishReason": "STOP",
            "index": 0
          }],
          "usageMetadata": {
            "promptTokenCount": 57,
            "candidatesTokenCount": 8,
            "totalTokenCount": 65
          },
          "modelVersion": "gemini-2.5-flash"
        }"#;
        let raw = parse_generate_response(body).expect("parses");
        assert_eq!(raw.content, r#"["read_page"]"#);
        assert_eq!(raw.usage.prompt_eval_count, 57);
        assert_eq!(raw.usage.eval_count, 8);
    }

    #[test]
    fn an_answer_split_across_parts_is_rejoined_not_truncated() {
        // Taking parts[0] alone would turn a valid JSON document into an
        // invalid one — a malformed-JSON error caused entirely by us.
        let body = br#"{
          "candidates": [{"content": {"parts": [
            {"text": "[\"read_"},
            {"text": "page\"]"}
          ], "role": "model"}}]
        }"#;
        let raw = parse_generate_response(body).expect("parses");
        assert_eq!(raw.content, r#"["read_page"]"#);
    }

    #[test]
    fn a_blocked_response_with_no_candidates_is_malformed_not_a_panic() {
        let body = br#"{"promptFeedback":{"blockReason":"SAFETY"}}"#;
        let err = parse_generate_response(body).expect_err("no candidates");
        assert!(matches!(err, ModelError::MalformedJson { .. }), "{err:?}");
    }

    #[test]
    fn missing_usage_metadata_is_zero_rather_than_a_failed_call() {
        let body = br#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#;
        let raw = parse_generate_response(body).expect("parses");
        assert_eq!(raw.usage, TokenUsage::default());
    }

    #[test]
    fn a_key_is_required_and_resolves_from_env_or_keyring() {
        let from_env = GeminiProvider::from_config(
            &config(),
            ModelTier::Small,
            &MapEnv::new().with(GEMINI_API_KEY_VAR, "k-env"),
            &NoSecretStore,
        )
        .expect("env");
        assert_eq!(from_env.base_url(), DEFAULT_GEMINI_BASE_URL);

        GeminiProvider::from_config(
            &config(),
            ModelTier::Main,
            &MapEnv::new(),
            &MapSecretStore::new().with(KEYRING_SERVICE, GEMINI_API_KEY_VAR, "k-keyring"),
        )
        .expect("keyring");
    }

    #[test]
    fn no_key_anywhere_is_a_clear_error_not_a_panic() {
        let err = GeminiProvider::from_config(
            &config(),
            ModelTier::Small,
            &MapEnv::new(),
            &NoSecretStore,
        )
        .expect_err("no key");
        assert!(matches!(err, ModelError::MissingApiKey { .. }), "{err:?}");
        assert!(
            err.to_string().contains(GEMINI_API_KEY_VAR),
            "and it must not suggest the old gemini_key.txt fallback: {err}"
        );
    }

    #[test]
    fn the_key_is_stripped_from_anything_that_quotes_the_url_back() {
        let message = "error sending request for url \
                       (https://example.invalid/v1/m:generateContent?key=AIza-SECRET)";
        let redacted = redact(message, "AIza-SECRET");
        assert!(!redacted.contains("AIza-SECRET"), "{redacted}");
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn the_provider_debug_output_never_contains_the_key() {
        let provider = GeminiProvider::new(
            DEFAULT_GEMINI_BASE_URL,
            Token::new("AIza-do-not-log-me"),
            ModelTier::Small,
            1024,
        );
        assert!(!format!("{provider:?}").contains("AIza-do-not-log-me"));
    }

    #[test]
    fn the_provider_reports_that_it_reaches_the_network() {
        let provider = GeminiProvider::new(
            DEFAULT_GEMINI_BASE_URL,
            Token::new("k"),
            ModelTier::Main,
            1024,
        );
        assert!(provider.capabilities().reaches_network);
        assert_eq!(provider.capabilities().tier, ModelTier::Main);
    }
}
