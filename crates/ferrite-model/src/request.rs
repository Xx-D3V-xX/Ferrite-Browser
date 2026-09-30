//! What goes in: [`CompletionRequest`] and the knobs §10.1 puts in
//! Ollama's `options` block.

use serde::{Deserialize, Serialize};

use crate::provider::ModelTier;

/// Who said a message.
///
/// No `Tool` role: this layer does not do tool calling (§10.5's trait is
/// text/structured-JSON only; the agent loop owns that, A9/T-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The system prompt, when a backend carries it in the message list
    /// rather than in a separate field.
    System,
    /// The human/caller turn.
    User,
    /// A prior model turn, replayed back as conversation context.
    Assistant,
}

impl Role {
    /// The wire string. Ollama and Gemini agree on `user`; they disagree on
    /// the assistant turn, so the Gemini backend maps this itself.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who said it.
    pub role: Role,
    /// What was said.
    pub content: String,
}

impl Message {
    /// A user turn.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant turn, for replaying history.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// The sampling knobs, mapped to Ollama's `options` object (§10.1).
///
/// [`Default`] is `temperature = 0`, `seed = 42`, `num_predict = 128` —
/// R8's determinism defaults, and the reason the content-addressed cache is
/// sound rather than a fudge (§10.3: at `temperature = 0` the response is a
/// pure function of the key).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SamplingOptions {
    /// `options.temperature`. Anything but `0.0` makes the cache unsound;
    /// [`CompletionRequest::is_cacheable`] is what actually enforces that.
    pub temperature: f32,
    /// `options.seed`.
    pub seed: u64,
    /// `options.num_predict` — §10.3 requires this be capped on every call.
    pub num_predict: u32,
}

impl Default for SamplingOptions {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            seed: 42,
            num_predict: 128,
        }
    }
}

impl SamplingOptions {
    /// Overrides the output cap, e.g. the agent loop's per-step ceiling.
    #[must_use]
    pub const fn with_num_predict(mut self, num_predict: u32) -> Self {
        self.num_predict = num_predict;
        self
    }
}

/// One completion call.
///
/// Built through [`CompletionRequest::new`] plus the `with_*` methods so
/// that adding a field later cannot silently change an existing call site's
/// meaning the way a new positional argument would.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The concrete model tag, resolved from config — never a literal in
    /// source (§10.2).
    pub model_tag: String,
    /// Which configured tier `model_tag` came from. Carried through to
    /// [`Provenance`](crate::response::Provenance) so a mid-corpus model
    /// swap is visible in the data.
    pub tier: ModelTier,
    /// The system prompt, if any.
    pub system_prompt: Option<String>,
    /// The system prompt's version. Bumping it deliberately invalidates
    /// every cache entry built from the old one (§10.3) — which is what you
    /// want when the prompt changes, and is why the version rather than the
    /// prompt text alone is a first-class field.
    pub system_prompt_version: u32,
    /// The conversation, oldest first.
    pub messages: Vec<Message>,
    /// Sampling knobs.
    pub options: SamplingOptions,
    /// A JSON schema for structured output (§10.4). Sent as Ollama's
    /// `format` / Gemini's `responseSchema`.
    pub format_schema: Option<serde_json::Value>,
    /// What this call is for (`"agent step"`, `"fingerprint"`, ...). Purely a
    /// label for the activity trace: it is not sent to any backend and is not
    /// part of the cache key, so setting it never changes a response.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl CompletionRequest {
    /// A request with determinism defaults and no schema.
    #[must_use]
    pub fn new(model_tag: impl Into<String>, tier: ModelTier, messages: Vec<Message>) -> Self {
        Self {
            model_tag: model_tag.into(),
            tier,
            system_prompt: None,
            system_prompt_version: 1,
            messages,
            options: SamplingOptions::default(),
            format_schema: None,
            label: String::new(),
        }
    }

    /// Names what this call is for, for the activity trace only.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets the system prompt and its version together, because a prompt
    /// whose version was not bumped alongside it is a stale cache hit
    /// waiting to happen.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>, version: u32) -> Self {
        self.system_prompt = Some(prompt.into());
        self.system_prompt_version = version;
        self
    }

    /// Asks for structured output against `schema` (§10.4).
    #[must_use]
    pub fn with_format_schema(mut self, schema: serde_json::Value) -> Self {
        self.format_schema = Some(schema);
        self
    }

    /// Overrides the sampling knobs.
    #[must_use]
    pub const fn with_options(mut self, options: SamplingOptions) -> Self {
        self.options = options;
        self
    }

    /// Whether caching this request's response is *sound*.
    ///
    /// Only at `temperature == 0` is the response a pure function of the
    /// cache key. Above it the cache would be memoizing a random draw, so
    /// [`Cache`](crate::decorators::Cache) passes such a request straight
    /// through instead of poisoning the store with one sample.
    #[must_use]
    pub fn is_cacheable(&self) -> bool {
        self.options.temperature == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_determinism_defaults() {
        let opts = SamplingOptions::default();
        assert_eq!(opts.temperature, 0.0, "R8: temperature = 0 by default");
        assert_eq!(opts.seed, 42, "R8: a fixed seed by default");
        assert_eq!(
            opts.num_predict, 128,
            "§10.3 caps num_predict on every call; the small tier's cap is the default"
        );
    }

    #[test]
    fn a_request_is_cacheable_exactly_when_it_is_deterministic() {
        let req = CompletionRequest::new("m", ModelTier::Small, vec![Message::user("hi")]);
        assert!(req.is_cacheable());

        let hot = req.clone().with_options(SamplingOptions {
            temperature: 0.7,
            ..SamplingOptions::default()
        });
        assert!(
            !hot.is_cacheable(),
            "caching a sampled response would memoize one draw as if it were the function"
        );
    }

    #[test]
    fn setting_a_system_prompt_forces_its_version_to_be_stated() {
        let req = CompletionRequest::new("m", ModelTier::Small, vec![Message::user("hi")])
            .with_system_prompt("be terse", 7);
        assert_eq!(req.system_prompt.as_deref(), Some("be terse"));
        assert_eq!(req.system_prompt_version, 7);
    }

    #[test]
    fn a_request_round_trips_through_serde() {
        let req = CompletionRequest::new("tag:1b", ModelTier::Main, vec![Message::user("hi")])
            .with_system_prompt("sys", 2)
            .with_format_schema(serde_json::json!({"type": "array"}));
        let json = serde_json::to_string(&req).expect("serialize");
        let back: CompletionRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, back);
    }
}
