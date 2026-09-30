//! Client for a locally running Laya decision server (`laya-serve`-compatible
//! `POST /v1/systemone`): typed `choice`/`score`/`noul` decisions in one
//! non-autoregressive forward pass. See `docs/DECISIONS.md` for what Laya is,
//! and is not, trusted with.
//!
//! # What this module is
//!
//! A thin, strict transport for `choice` questions: an ordered request
//! builder ([`SystemOneRequest`], [`ChoiceQuestion`]), an HTTP client
//! ([`LayaClient`]) and a response validator ([`SystemOneResponse::choice`])
//! that refuses anything it cannot fully account for. It knows nothing about
//! browsers; the agent-side policy that turns a page into questions lives in
//! `ferrite_agent::decider`.
//!
//! # What this module is not (trust boundary)
//!
//! Laya is a fast System-1 accelerator for *ordinary browsing decisions*. It
//! is **never** part of the injection defense: nothing here is, or may be,
//! imported by `ferrite-ipi` (fingerprint prediction, comparator and consent
//! stay LLM-or-rule based and unaffected). Its published base checkpoints are
//! near chance zero-shot and a fine-tuned head can be confidently wrong off
//! its training distribution, which is why every consumer gates on the
//! returned probabilities and falls back to the normal LLM path on anything
//! else. Failure semantics follow the project's fail-to-empty rule: every
//! error here means "no fast decision", never "guess" and never "skip a
//! check".
//!
//! # Configuration (all optional; unset `FERRITE_LAYA_URL` disables Laya)
//!
//! | variable | meaning | default |
//! |---|---|---|
//! | `FERRITE_LAYA_URL` | base URL of the Laya server, e.g. `http://127.0.0.1:8000` | unset = disabled |
//! | `FERRITE_LAYA_API_KEY` | sent as `Authorization: Bearer`; env only, never logged | none |
//! | `FERRITE_LAYA_TIMEOUT_MS` | whole-request timeout | [`DEFAULT_TIMEOUT_MS`] |
//! | `FERRITE_LAYA_MODEL` | pin the `model` field of a request | omitted (server routes) |
//! | `FERRITE_LAYA_OP_GATE` | minimum operation probability, in (0, 1] | [`DEFAULT_OP_GATE`] |
//! | `FERRITE_LAYA_TARGET_GATE` | minimum target probability, in (0, 1] | [`DEFAULT_TARGET_GATE`] |
//!
//! # Privacy
//!
//! A request carries the page URL, title, visible text and element labels of
//! the page being decided about. Point `FERRITE_LAYA_URL` at a machine you
//! trust (the intended deployment is `laya-serve` on loopback). Password
//! fields never reach a digest, and an API key is refused over cleartext HTTP
//! to a non-loopback host.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;
use thiserror::Error;

use crate::config::{DEFAULT_MAX_RESPONSE_BYTES, EnvSource};
use crate::error::ModelError;
use crate::secret::Token;
use crate::trace::{self, TraceBackend, TraceEvent};

/// `FERRITE_LAYA_URL` — enables Laya when set.
pub const LAYA_URL_VAR: &str = "FERRITE_LAYA_URL";
/// `FERRITE_LAYA_API_KEY` — optional bearer token (environment only).
pub const LAYA_API_KEY_VAR: &str = "FERRITE_LAYA_API_KEY";
/// `FERRITE_LAYA_TIMEOUT_MS`.
pub const LAYA_TIMEOUT_VAR: &str = "FERRITE_LAYA_TIMEOUT_MS";
/// `FERRITE_LAYA_MODEL`.
pub const LAYA_MODEL_VAR: &str = "FERRITE_LAYA_MODEL";
/// `FERRITE_LAYA_OP_GATE`.
pub const LAYA_OP_GATE_VAR: &str = "FERRITE_LAYA_OP_GATE";
/// `FERRITE_LAYA_TARGET_GATE`.
pub const LAYA_TARGET_GATE_VAR: &str = "FERRITE_LAYA_TARGET_GATE";

/// Default whole-request timeout. Latency is the entire point of System 1: a
/// server that has not answered in this long is worse than the LLM it was
/// meant to beat, so the caller falls back.
pub const DEFAULT_TIMEOUT_MS: u64 = 1_500;
/// Default per-request `max_len`: the 1,024-token window of the published
/// multilingual/typed-decisions checkpoints.
pub const DEFAULT_MAX_LEN: u32 = 1_024;
/// Default per-request `head_max_len`: 768, the option-prompt budget the
/// `cklxx/laya-browser` checkpoints were trained with (their README: "the
/// config records the input format too").
pub const DEFAULT_HEAD_MAX_LEN: u32 = 768;
/// Default minimum probability of the chosen *operation*.
///
/// A conservative starting point, **not** a measured optimum: it has not been
/// tuned on real Ferrite traces. Raise it to make the fast lane rarer.
pub const DEFAULT_OP_GATE: f32 = 0.80;
/// Default minimum probability of the chosen *target*. Lower than the
/// operation gate because a target head spreads probability over up to ~45
/// candidates; like [`DEFAULT_OP_GATE`] it is an untuned starting point.
pub const DEFAULT_TARGET_GATE: f32 = 0.60;

/// The server rejects a `choice` question with more options than this
/// (`MAX_CHOICE_OPTIONS` in `laya.serve`); checked locally so a caller bug is
/// not a round trip.
pub const MAX_CHOICE_OPTIONS: usize = 100;
/// `laya.serve`'s `MAX_QUESTIONS`.
const MAX_QUESTIONS: usize = 64;
/// `laya.serve`'s `MAX_TOTAL_OPTIONS`.
const MAX_TOTAL_OPTIONS: usize = 512;
/// A health probe must never hold up a step.
const HEALTH_TIMEOUT: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Everything needed to talk to a Laya server, and the confidence gates the
/// agent applies to its answers.
///
/// [`Debug`] is derived and safe: the key is a [`Token`], whose `Debug`
/// prints `Token(<redacted>)`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayaConfig {
    /// Server base URL without a trailing slash.
    pub base_url: String,
    /// Optional bearer token. Read only from the environment.
    pub api_key: Option<Token>,
    /// Whole-request timeout.
    pub timeout: Duration,
    /// Value of the request's `model` field; omitted when `None` so the
    /// server routes.
    pub model: Option<String>,
    /// Per-request `max_len`.
    pub max_len: u32,
    /// Per-request `head_max_len`.
    pub head_max_len: u32,
    /// Minimum operation probability for the fast lane, in (0, 1].
    pub op_gate: f32,
    /// Minimum target probability for the fast lane, in (0, 1].
    pub target_gate: f32,
}

impl LayaConfig {
    /// A config for `base_url` with every documented default.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            api_key: None,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            model: None,
            max_len: DEFAULT_MAX_LEN,
            head_max_len: DEFAULT_HEAD_MAX_LEN,
            op_gate: DEFAULT_OP_GATE,
            target_gate: DEFAULT_TARGET_GATE,
        }
    }

    /// Loads from an [`EnvSource`]. `Ok(None)` means Laya is disabled (the
    /// URL is unset or empty) and the agent must behave exactly as if this
    /// module did not exist.
    ///
    /// # Errors
    ///
    /// [`ModelError::Config`] when Laya *is* enabled but a value is unusable:
    /// a URL without an `http(s)://` scheme, a non-numeric or zero timeout, a
    /// gate outside (0, 1], or an API key configured for a cleartext,
    /// non-loopback URL. A typo must not silently disable a safety gate.
    pub fn from_env(env: &dyn EnvSource) -> Result<Option<Self>, ModelError> {
        let Some(url) = env.get(LAYA_URL_VAR) else {
            return Ok(None);
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ModelError::Config(format!(
                "{LAYA_URL_VAR}={url:?} must start with http:// or https:// \
                 (for example http://127.0.0.1:8000)"
            )));
        }
        let mut config = Self::new(url);
        config.api_key = env.get(LAYA_API_KEY_VAR).map(Token::new);
        config.model = env.get(LAYA_MODEL_VAR);
        if let Some(raw) = env.get(LAYA_TIMEOUT_VAR) {
            let ms: u64 = raw.parse().map_err(|e| {
                ModelError::Config(format!(
                    "{LAYA_TIMEOUT_VAR}={raw:?} is not a valid value: {e}"
                ))
            })?;
            if ms == 0 {
                return Err(ModelError::Config(format!(
                    "{LAYA_TIMEOUT_VAR}=0 would time out every request; use a positive number of milliseconds"
                )));
            }
            config.timeout = Duration::from_millis(ms);
        }
        config.op_gate = parse_gate(env, LAYA_OP_GATE_VAR, DEFAULT_OP_GATE)?;
        config.target_gate = parse_gate(env, LAYA_TARGET_GATE_VAR, DEFAULT_TARGET_GATE)?;
        if config.api_key.is_some()
            && config.base_url.starts_with("http://")
            && !is_loopback_url(&config.base_url)
        {
            return Err(ModelError::Config(format!(
                "{LAYA_API_KEY_VAR} is set but {LAYA_URL_VAR} is cleartext http:// to a non-loopback host; \
                 refusing to send a bearer token unencrypted. Use https:// or a loopback address."
            )));
        }
        Ok(Some(config))
    }
}

fn parse_gate(env: &dyn EnvSource, key: &str, default: f32) -> Result<f32, ModelError> {
    let Some(raw) = env.get(key) else {
        return Ok(default);
    };
    let value: f32 = raw
        .parse()
        .map_err(|e| ModelError::Config(format!("{key}={raw:?} is not a valid value: {e}")))?;
    // `!(x > 0.0 && x <= 1.0)` rather than a range check so NaN is rejected.
    if !(value > 0.0 && value <= 1.0) {
        return Err(ModelError::Config(format!(
            "{key}={raw:?} must be a probability in (0, 1]"
        )));
    }
    Ok(value)
}

/// Whether `url`'s host is a loopback literal. Whole-host match, never a
/// prefix (`localhost.evil.example` is remote); same rule as
/// `ModelConfig::ollama_is_local`.
fn is_loopback_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(after) => match after.split_once(']') {
            Some((inner, _port)) => format!("[{inner}]"),
            None => return false,
        },
        None => authority.split(':').next().unwrap_or("").to_string(),
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Every way a Laya call fails. All of them mean "no fast decision".
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LayaError {
    /// No server is configured (`FERRITE_LAYA_URL` unset).
    #[error("Laya is not configured ({LAYA_URL_VAR} is unset)")]
    Disabled,
    /// The request did not finish within the configured timeout.
    #[error("Laya request timed out after {0:?}")]
    Timeout(Duration),
    /// The server could not be reached, or the connection broke.
    #[error("could not reach the Laya server: {0}")]
    Connect(String),
    /// The server answered with a non-2xx status.
    #[error("Laya server returned HTTP {0}")]
    Http(u16),
    /// The body was not a response this module can fully account for:
    /// garbage JSON, a chosen label that was not offered, probabilities that
    /// do not cover the offered labels or sum to one, an oversized body.
    #[error("invalid Laya response: {0}")]
    InvalidResponse(String),
    /// The *request* was refused locally before sending (over the server's
    /// option limits, duplicate ids, nothing to ask). A caller bug.
    #[error("refusing to send an invalid Laya request: {0}")]
    InvalidRequest(String),
}

// ---------------------------------------------------------------------------
// Insertion-ordered JSON
// ---------------------------------------------------------------------------
//
// Laya renders `state` and every criterion with `json.dumps(..., sort_keys=False)`
// (laya/router.py: "insertion order significant at every nesting level"), and
// option order is option order. `serde_json::Map` is a BTreeMap unless the
// workspace enables `preserve_order` — which would also change every other
// serialization in the workspace, including hash-chained audit records — so
// the wire format gets its own tiny ordered tree instead.

/// A JSON value whose objects keep insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A leaf (or a subtree whose key order does not matter).
    Value(Value),
    /// An array of ordered nodes.
    Array(Vec<Node>),
    /// An insertion-ordered object.
    Object(OrderedMap),
}

/// An insertion-ordered JSON object. Setting an existing key replaces its
/// value in place, keeping its position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrderedMap(Vec<(String, Node)>);

impl OrderedMap {
    /// An empty object.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key` (appending it if new, replacing in place otherwise).
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Node>) -> Self {
        self.set(key, value);
        self
    }

    /// In-place [`OrderedMap::with`].
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<Node>) {
        let key = key.into();
        let value = value.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }

    /// Number of keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }

    /// The value at `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl Serialize for OrderedMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(v) => v.serialize(serializer),
            Self::Object(m) => m.serialize(serializer),
            Self::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
        }
    }
}

impl From<&str> for Node {
    fn from(s: &str) -> Self {
        Self::Value(Value::String(s.to_string()))
    }
}
impl From<String> for Node {
    fn from(s: String) -> Self {
        Self::Value(Value::String(s))
    }
}
impl From<bool> for Node {
    fn from(b: bool) -> Self {
        Self::Value(Value::Bool(b))
    }
}
impl From<Value> for Node {
    fn from(v: Value) -> Self {
        Self::Value(v)
    }
}
impl From<OrderedMap> for Node {
    fn from(m: OrderedMap) -> Self {
        Self::Object(m)
    }
}
impl From<Vec<Node>> for Node {
    fn from(items: Vec<Node>) -> Self {
        Self::Array(items)
    }
}
impl<T: Into<Node>> From<Option<T>> for Node {
    fn from(o: Option<T>) -> Self {
        o.map_or(Self::Value(Value::Null), Into::into)
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// One `choice` question: what to decide (`instructions`, a string or an
/// object such as `{goal, rules}`) and the ordered options.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceQuestion {
    instructions: Node,
    criteria: OrderedMap,
}

impl ChoiceQuestion {
    /// A question with no options yet.
    #[must_use]
    pub fn new(instructions: impl Into<Node>) -> Self {
        Self {
            instructions: instructions.into(),
            criteria: OrderedMap::new(),
        }
    }

    /// Appends an option: `id` is the label Laya answers with, `description`
    /// what it stands for (a string, or an ordered object).
    #[must_use]
    pub fn option(mut self, id: impl Into<String>, description: impl Into<Node>) -> Self {
        self.criteria.set(id, description);
        self
    }

    /// The offered ids, in order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.criteria.keys().collect()
    }

    /// Number of options.
    #[must_use]
    pub fn len(&self) -> usize {
        self.criteria.len()
    }

    /// Whether no option was added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.criteria.is_empty()
    }
}

impl Serialize for ChoiceQuestion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("type", "choice")?;
        map.serialize_entry("criteria", &self.criteria)?;
        map.serialize_entry("instructions", &self.instructions)?;
        map.end()
    }
}

/// The body of `POST /v1/systemone`.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemOneRequest {
    model: Option<String>,
    state: Node,
    questions: Vec<(String, ChoiceQuestion)>,
    max_len: Option<u32>,
    head_max_len: Option<u32>,
}

impl SystemOneRequest {
    /// A request about `state` with no questions yet.
    #[must_use]
    pub fn new(state: impl Into<Node>) -> Self {
        Self {
            model: None,
            state: state.into(),
            questions: Vec::new(),
            max_len: None,
            head_max_len: None,
        }
    }

    /// Adds a question; answers come back under `id`.
    #[must_use]
    pub fn with_question(mut self, id: impl Into<String>, question: ChoiceQuestion) -> Self {
        self.questions.push((id.into(), question));
        self
    }

    /// Pins the `model` field (`None` omits it).
    #[must_use]
    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }

    /// Sets the per-request token budgets.
    #[must_use]
    pub fn with_budgets(mut self, max_len: u32, head_max_len: u32) -> Self {
        self.max_len = Some(max_len);
        self.head_max_len = Some(head_max_len);
        self
    }

    /// The question asked under `id`.
    #[must_use]
    pub fn question(&self, id: &str) -> Option<&ChoiceQuestion> {
        self.questions.iter().find(|(q, _)| q == id).map(|(_, q)| q)
    }

    /// Checks the request against the server's published limits.
    ///
    /// # Errors
    ///
    /// [`LayaError::InvalidRequest`] for no questions, too many, a duplicate
    /// question id, an empty question, or more options than the server
    /// accepts.
    pub fn validate(&self) -> Result<(), LayaError> {
        let bad = |m: String| Err(LayaError::InvalidRequest(m));
        if self.questions.is_empty() {
            return bad("no questions".to_string());
        }
        if self.questions.len() > MAX_QUESTIONS {
            return bad(format!(
                "{} questions (limit {MAX_QUESTIONS})",
                self.questions.len()
            ));
        }
        let mut total = 0usize;
        for (i, (id, q)) in self.questions.iter().enumerate() {
            if self.questions[..i].iter().any(|(other, _)| other == id) {
                return bad(format!("duplicate question id {id:?}"));
            }
            if q.is_empty() {
                return bad(format!("question {id:?} has no options"));
            }
            if q.len() > MAX_CHOICE_OPTIONS {
                return bad(format!(
                    "question {id:?} has {} options (limit {MAX_CHOICE_OPTIONS})",
                    q.len()
                ));
            }
            total += q.len();
        }
        if total > MAX_TOTAL_OPTIONS {
            return bad(format!(
                "{total} options in total (limit {MAX_TOTAL_OPTIONS})"
            ));
        }
        Ok(())
    }
}

struct QuestionsSer<'a>(&'a [(String, ChoiceQuestion)]);

impl Serialize for QuestionsSer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, q) in self.0 {
            map.serialize_entry(id, q)?;
        }
        map.end()
    }
}

impl Serialize for SystemOneRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(model) = &self.model {
            map.serialize_entry("model", model)?;
        }
        map.serialize_entry("state", &self.state)?;
        map.serialize_entry("questions", &QuestionsSer(&self.questions))?;
        if let Some(n) = self.max_len {
            map.serialize_entry("max_len", &n)?;
        }
        if let Some(n) = self.head_max_len {
            map.serialize_entry("head_max_len", &n)?;
        }
        map.end()
    }
}

// ---------------------------------------------------------------------------
// Response and its validator
// ---------------------------------------------------------------------------

/// A validated answer to one `choice` question.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAnswer {
    /// The chosen option id (always one that was offered).
    pub choice: String,
    /// The server's `confidence`, validated to be finite and in `[0, 1]`.
    pub confidence: f64,
    probabilities: Vec<(String, f64)>,
}

impl ChoiceAnswer {
    /// The probability the server assigned to `id`.
    #[must_use]
    pub fn probability(&self, id: &str) -> Option<f64> {
        self.probabilities
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, p)| *p)
    }

    /// The probability of the chosen option.
    #[must_use]
    pub fn chosen_probability(&self) -> f64 {
        self.probability(&self.choice).unwrap_or(0.0)
    }
}

/// A parsed `/v1/systemone` response. Answers are held raw and only become
/// usable through [`SystemOneResponse::choice`], which validates them.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemOneResponse {
    /// Which checkpoint answered (informational).
    pub model: String,
    answers: BTreeMap<String, Value>,
}

impl SystemOneResponse {
    /// Parses a response body.
    ///
    /// # Errors
    ///
    /// [`LayaError::InvalidResponse`] if it is not a JSON object with an
    /// `answers` object.
    pub fn from_slice(body: &[u8]) -> Result<Self, LayaError> {
        let value: Value = serde_json::from_slice(body)
            .map_err(|e| LayaError::InvalidResponse(format!("body is not JSON: {e}")))?;
        let answers = value
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(|| LayaError::InvalidResponse("no `answers` object".to_string()))?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(Self { model, answers })
    }

    /// The validated answer to `question_id`, checked against `question`'s
    /// offered ids. This is a port of `validate_choice` from
    /// `browser-use/jev-ultrafast` (`jev_ultrafast/model.py`, MIT): the
    /// chosen label must be one of the offered ids; the probabilities must
    /// cover exactly the offered ids; every probability and the confidence
    /// must be a finite JSON number in `[0, 1]` (booleans are not numbers);
    /// they must sum to 1 within 0.02; and the chosen label must be
    /// (within 1e-6) the argmax. Anything else is an error, never a guess.
    ///
    /// # Errors
    ///
    /// [`LayaError::InvalidResponse`] on any deviation.
    pub fn choice(
        &self,
        question_id: &str,
        question: &ChoiceQuestion,
    ) -> Result<ChoiceAnswer, LayaError> {
        validate_choice(self.answers.get(question_id), &question.ids())
            .map_err(|why| LayaError::InvalidResponse(format!("answer {question_id:?}: {why}")))
    }
}

fn unit_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1.0).contains(x)),
        _ => None,
    }
}

fn validate_choice(answer: Option<&Value>, ids: &[&str]) -> Result<ChoiceAnswer, String> {
    let obj = answer
        .and_then(Value::as_object)
        .ok_or("missing or not an object")?;
    let choice = obj
        .get("choice")
        .and_then(Value::as_str)
        .ok_or("no string `choice`")?;
    if !ids.contains(&choice) {
        return Err(format!("chose {choice:?}, which was not offered"));
    }
    let probs = obj
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or("no `probabilities` object")?;
    if probs.len() != ids.len() || !ids.iter().all(|id| probs.contains_key(*id)) {
        return Err("probabilities do not cover exactly the offered ids".to_string());
    }
    let mut ordered = Vec::with_capacity(ids.len());
    for id in ids {
        let p = probs
            .get(*id)
            .and_then(unit_number)
            .ok_or_else(|| format!("probability of {id:?} is not a finite number in [0, 1]"))?;
        ordered.push(((*id).to_string(), p));
    }
    let confidence = obj
        .get("confidence")
        .and_then(unit_number)
        .ok_or("`confidence` is missing or not a finite number in [0, 1]")?;
    let sum: f64 = ordered.iter().map(|(_, p)| p).sum();
    if (sum - 1.0).abs() >= 0.02 {
        return Err(format!("probabilities sum to {sum}, not 1"));
    }
    let max = ordered.iter().map(|(_, p)| *p).fold(0.0_f64, f64::max);
    let chosen = ordered
        .iter()
        .find(|(k, _)| k == choice)
        .map_or(0.0, |(_, p)| *p);
    if chosen < max - 1e-6 {
        return Err(format!(
            "chosen {choice:?} has probability {chosen}, below the maximum {max}"
        ));
    }
    Ok(ChoiceAnswer {
        choice: choice.to_string(),
        confidence,
        probabilities: ordered,
    })
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for one Laya server.
#[derive(Debug, Clone)]
pub struct LayaClient {
    config: LayaConfig,
    http: reqwest::Client,
}

impl LayaClient {
    /// A client for `config`.
    ///
    /// Redirects are not followed (a bearer token must not be replayed to
    /// wherever a redirect points), and a loopback server is contacted
    /// directly even when the environment configures an HTTP proxy.
    #[must_use]
    pub fn new(config: LayaConfig) -> Self {
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if is_loopback_url(&config.base_url) {
            builder = builder.no_proxy();
        }
        Self {
            http: builder.build().unwrap_or_else(|_| reqwest::Client::new()),
            config,
        }
    }

    /// A client from an optional config.
    ///
    /// # Errors
    ///
    /// [`LayaError::Disabled`] for `None`.
    pub fn from_config(config: Option<LayaConfig>) -> Result<Self, LayaError> {
        config.map(Self::new).ok_or(LayaError::Disabled)
    }

    /// The configuration this client was built from.
    #[must_use]
    pub fn config(&self) -> &LayaConfig {
        &self.config
    }

    /// Whether the server answers `GET /health` within 500 ms. Cheap enough
    /// to call before enabling the fast lane; never sends the API key.
    pub async fn health(&self) -> bool {
        let url = format!("{}/health", self.config.base_url);
        match self.http.get(url).timeout(HEALTH_TIMEOUT).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Sends one `POST /v1/systemone`. `stage` names what the call is for in
    /// the [activity trace](crate::trace), which records the request, the
    /// raw answer and the round-trip time of every call.
    ///
    /// No retries except a single immediate retry of a *connection* error
    /// (latency is the point; a timeout or an HTTP error is never retried).
    ///
    /// # Errors
    ///
    /// [`LayaError::InvalidRequest`] before anything is sent;
    /// [`LayaError::Timeout`], [`LayaError::Connect`], [`LayaError::Http`],
    /// or [`LayaError::InvalidResponse`] (unparsable or over
    /// [`DEFAULT_MAX_RESPONSE_BYTES`]) afterwards.
    pub async fn systemone(
        &self,
        stage: &str,
        req: &SystemOneRequest,
    ) -> Result<SystemOneResponse, LayaError> {
        let started = std::time::Instant::now();
        let result = self.systemone_raw(req).await;
        let mut event = TraceEvent::new(
            TraceBackend::Laya,
            stage,
            self.config.model.clone().unwrap_or_default(),
        );
        event.latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        event.request = serde_json::to_string(req).unwrap_or_default();
        match &result {
            Ok((response, raw)) => {
                event.response = String::from_utf8_lossy(raw).into_owned();
                if !response.model.is_empty() {
                    event.model.clone_from(&response.model);
                }
            }
            Err(e) => {
                event.ok = false;
                event.response = e.to_string();
                event.note = "Laya call failed".to_string();
            }
        }
        trace::global().record(event);
        result.map(|(response, _)| response)
    }

    async fn systemone_raw(
        &self,
        req: &SystemOneRequest,
    ) -> Result<(SystemOneResponse, Vec<u8>), LayaError> {
        req.validate()?;
        let body = serde_json::to_vec(req)
            .map_err(|e| LayaError::InvalidRequest(format!("request does not serialize: {e}")))?;
        let url = format!("{}/v1/systemone", self.config.base_url);

        let mut retried = false;
        let mut response = loop {
            let mut builder = self
                .http
                .post(&url)
                .timeout(self.config.timeout)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone());
            if let Some(key) = &self.config.api_key {
                builder = builder.bearer_auth(key.expose());
            }
            match builder.send().await {
                Ok(response) => break response,
                Err(e) if e.is_timeout() => return Err(LayaError::Timeout(self.config.timeout)),
                Err(e) if e.is_connect() && !retried => retried = true,
                Err(e) => return Err(self.transport_error(e)),
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Err(LayaError::Http(status.as_u16()));
        }

        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if bytes.len() + chunk.len() > DEFAULT_MAX_RESPONSE_BYTES {
                        return Err(LayaError::InvalidResponse(format!(
                            "body exceeded {DEFAULT_MAX_RESPONSE_BYTES} bytes"
                        )));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) if e.is_timeout() => return Err(LayaError::Timeout(self.config.timeout)),
                Err(e) => return Err(self.transport_error(e)),
            }
        }
        SystemOneResponse::from_slice(&bytes).map(|response| (response, bytes))
    }

    fn transport_error(&self, e: reqwest::Error) -> LayaError {
        // `without_url`: a base URL may carry userinfo, and an error message
        // must never be a way to print it.
        let text = e.without_url().to_string();
        LayaError::Connect(text.chars().take(200).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // R7: the "no live network" rule is about leaving the machine. Every
    // socket in this module is a loopback listener owned by the test itself,
    // bound to 127.0.0.1:0, so nothing here can reach anything else, and no
    // provider key is involved.
    use tokio::net::TcpListener;

    use super::*;
    use crate::config::MapEnv;

    const KEY: &str = "sk-laya-super-secret-value";

    // -- config -------------------------------------------------------------

    fn env() -> MapEnv {
        MapEnv::new().with(LAYA_URL_VAR, "http://127.0.0.1:8000/")
    }

    #[test]
    fn an_unset_or_blank_url_disables_laya() {
        assert_eq!(LayaConfig::from_env(&MapEnv::new()).expect("ok"), None);
        let blank = MapEnv::new().with(LAYA_URL_VAR, "   ");
        assert_eq!(LayaConfig::from_env(&blank).expect("ok"), None);
        // Other Laya variables alone never enable it.
        let only_key = MapEnv::new().with(LAYA_API_KEY_VAR, KEY);
        assert_eq!(LayaConfig::from_env(&only_key).expect("ok"), None);
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let cfg = LayaConfig::from_env(&env()).expect("ok").expect("enabled");
        assert_eq!(
            cfg.base_url, "http://127.0.0.1:8000",
            "trailing slash trimmed"
        );
        assert_eq!(cfg.timeout, Duration::from_millis(1_500));
        assert_eq!((cfg.max_len, cfg.head_max_len), (1_024, 768));
        assert!((cfg.op_gate - 0.80).abs() < f32::EPSILON);
        assert!((cfg.target_gate - 0.60).abs() < f32::EPSILON);
        assert_eq!(cfg.model, None);
        assert_eq!(cfg.api_key, None);
    }

    #[test]
    fn every_default_is_overridable() {
        let e = env()
            .with(LAYA_TIMEOUT_VAR, "250")
            .with(LAYA_MODEL_VAR, "typed-decisions")
            .with(LAYA_OP_GATE_VAR, "0.9")
            .with(LAYA_TARGET_GATE_VAR, "1")
            .with(LAYA_API_KEY_VAR, KEY);
        let cfg = LayaConfig::from_env(&e).expect("ok").expect("enabled");
        assert_eq!(cfg.timeout, Duration::from_millis(250));
        assert_eq!(cfg.model.as_deref(), Some("typed-decisions"));
        assert!((cfg.op_gate - 0.9).abs() < f32::EPSILON);
        assert!((cfg.target_gate - 1.0).abs() < f32::EPSILON);
        assert_eq!(cfg.api_key.as_ref().map(Token::expose), Some(KEY));
    }

    #[test]
    fn bad_overrides_are_actionable_config_errors_naming_the_variable() {
        for (var, value) in [
            (LAYA_TIMEOUT_VAR, "soon"),
            (LAYA_TIMEOUT_VAR, "0"),
            (LAYA_OP_GATE_VAR, "high"),
            (LAYA_OP_GATE_VAR, "0"),
            (LAYA_OP_GATE_VAR, "1.5"),
            (LAYA_OP_GATE_VAR, "-0.2"),
            (LAYA_OP_GATE_VAR, "NaN"),
            (LAYA_TARGET_GATE_VAR, "2"),
        ] {
            let err = LayaConfig::from_env(&env().with(var, value))
                .expect_err("a typo must not silently change a gate");
            let msg = err.to_string();
            assert!(msg.contains(var), "{var}={value}: {msg}");
        }
        let err = LayaConfig::from_env(&MapEnv::new().with(LAYA_URL_VAR, "127.0.0.1:8000"))
            .expect_err("no scheme");
        assert!(err.to_string().contains("http://"), "{err}");
    }

    #[test]
    fn a_key_is_refused_over_cleartext_to_a_remote_host_but_not_loopback_or_tls() {
        let remote = MapEnv::new()
            .with(LAYA_URL_VAR, "http://laya.example.com:8000")
            .with(LAYA_API_KEY_VAR, KEY);
        let err = LayaConfig::from_env(&remote).expect_err("cleartext bearer");
        assert!(!err.to_string().contains(KEY), "the key is never echoed");
        for ok in [
            "https://laya.example.com",
            "http://localhost:8000",
            "http://[::1]:8000",
        ] {
            let e = MapEnv::new()
                .with(LAYA_URL_VAR, ok)
                .with(LAYA_API_KEY_VAR, KEY);
            assert!(LayaConfig::from_env(&e).is_ok(), "{ok}");
        }
        let lookalike = MapEnv::new()
            .with(LAYA_URL_VAR, "http://localhost.evil.example")
            .with(LAYA_API_KEY_VAR, KEY);
        assert!(LayaConfig::from_env(&lookalike).is_err());
    }

    #[test]
    fn the_api_key_never_appears_in_debug_output() {
        let cfg = LayaConfig::from_env(&env().with(LAYA_API_KEY_VAR, KEY))
            .expect("ok")
            .expect("enabled");
        let rendered = format!("{cfg:?} {:?}", LayaClient::new(cfg.clone()));
        assert!(!rendered.contains(KEY), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }

    #[test]
    fn a_client_needs_a_config() {
        assert_eq!(
            LayaClient::from_config(None).expect_err("disabled"),
            LayaError::Disabled
        );
        assert!(LayaClient::from_config(Some(LayaConfig::new("http://x"))).is_ok());
    }

    // -- request format -----------------------------------------------------

    #[test]
    fn a_request_keeps_key_and_option_order_at_every_level() {
        // Deliberately not alphabetical anywhere: "10" before "2", "z" before "a".
        let state = OrderedMap::new()
            .with(
                "page",
                OrderedMap::new()
                    .with("url", "https://a.example/")
                    .with("title", "T")
                    .with("text", "hello"),
            )
            .with(
                "recent_actions",
                vec![Node::from(
                    OrderedMap::new()
                        .with("action", "Go")
                        .with("text", Option::<String>::None)
                        .with("page_changed", Some(true)),
                )],
            );
        let q = ChoiceQuestion::new(
            OrderedMap::new()
                .with("goal", "find z")
                .with("rules", vec![Node::from("r1"), Node::from("r2")]),
        )
        .option("10", OrderedMap::new().with("z", "1").with("a", "2"))
        .option("2", "plain");
        let req = SystemOneRequest::new(state)
            .with_question("operation", q)
            .with_model(Some("m".to_string()))
            .with_budgets(1024, 768);
        let json = serde_json::to_string(&req).expect("serializes");
        assert_eq!(
            json,
            concat!(
                r#"{"model":"m","state":{"page":{"url":"https://a.example/","title":"T","text":"hello"},"#,
                r#""recent_actions":[{"action":"Go","text":null,"page_changed":true}]},"#,
                r#""questions":{"operation":{"type":"choice","criteria":{"10":{"z":"1","a":"2"},"2":"plain"},"#,
                r#""instructions":{"goal":"find z","rules":["r1","r2"]}}},"max_len":1024,"head_max_len":768}"#
            )
        );
    }

    #[test]
    fn setting_an_existing_key_keeps_its_position() {
        let m = OrderedMap::new()
            .with("b", "1")
            .with("a", "2")
            .with("b", "3");
        assert_eq!(m.keys().collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(m.get("b"), Some(&Node::from("3")));
    }

    #[test]
    fn requests_over_the_servers_limits_are_refused_before_sending() {
        let q = |n: usize| {
            (0..n).fold(ChoiceQuestion::new("i"), |q, i| {
                q.option(i.to_string(), "d")
            })
        };
        assert!(
            SystemOneRequest::new("s").validate().is_err(),
            "no questions"
        );
        let ok = SystemOneRequest::new("s").with_question("a", q(100));
        assert!(ok.validate().is_ok());
        let too_many = SystemOneRequest::new("s").with_question("a", q(101));
        assert!(matches!(
            too_many.validate(),
            Err(LayaError::InvalidRequest(m)) if m.contains("101")
        ));
        let dup = SystemOneRequest::new("s")
            .with_question("a", q(2))
            .with_question("a", q(2));
        assert!(dup.validate().is_err());
        let empty = SystemOneRequest::new("s").with_question("a", q(0));
        assert!(empty.validate().is_err());
    }

    // -- validator (port of jev's validate_choice) --------------------------

    fn question() -> ChoiceQuestion {
        ChoiceQuestion::new("i")
            .option("A", "a")
            .option("B", "b")
            .option("C", "c")
    }

    fn answer(choice: &str, probs: Value, confidence: Value) -> Value {
        serde_json::json!({"choice": choice, "probabilities": probs, "confidence": confidence})
    }

    fn respond(a: Value) -> SystemOneResponse {
        let body = serde_json::json!({"model": "m", "answers": {"q": a}}).to_string();
        SystemOneResponse::from_slice(body.as_bytes()).expect("shape ok")
    }

    fn check(a: Value) -> Result<ChoiceAnswer, LayaError> {
        respond(a).choice("q", &question())
    }

    #[test]
    fn a_well_formed_answer_validates_and_exposes_probabilities_in_offered_order() {
        let a = check(answer(
            "B",
            serde_json::json!({"C": 0.1, "A": 0.1, "B": 0.8}),
            serde_json::json!(0.8),
        ))
        .expect("valid");
        assert_eq!(a.choice, "B");
        assert!((a.chosen_probability() - 0.8).abs() < 1e-9);
        assert!((a.probability("A").expect("A") - 0.1).abs() < 1e-9);
        assert_eq!(a.probability("Z"), None);
    }

    #[test]
    fn every_way_an_answer_can_be_wrong_is_rejected() {
        let good = serde_json::json!({"A": 0.1, "B": 0.8, "C": 0.1});
        let cases: Vec<(&str, Value)> = vec![
            (
                "chosen label not offered",
                answer("Z", good.clone(), serde_json::json!(0.8)),
            ),
            (
                "chosen label is not argmax",
                answer("A", good.clone(), serde_json::json!(0.8)),
            ),
            (
                "does not sum to 1",
                answer(
                    "B",
                    serde_json::json!({"A": 0.1, "B": 0.8, "C": 0.5}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "missing an offered id",
                answer(
                    "B",
                    serde_json::json!({"A": 0.2, "B": 0.8}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "extra id",
                answer(
                    "B",
                    serde_json::json!({"A": 0.1, "B": 0.8, "C": 0.1, "D": 0.0}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "probability above 1",
                answer(
                    "B",
                    serde_json::json!({"A": -0.1, "B": 1.1, "C": 0.0}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "boolean is not a number",
                answer(
                    "B",
                    serde_json::json!({"A": 0.0, "B": true, "C": 0.0}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "string probability",
                answer(
                    "B",
                    serde_json::json!({"A": "0.1", "B": 0.8, "C": 0.1}),
                    serde_json::json!(0.8),
                ),
            ),
            (
                "confidence out of range",
                answer("B", good.clone(), serde_json::json!(1.5)),
            ),
            (
                "confidence missing",
                serde_json::json!({"choice": "B", "probabilities": good.clone()}),
            ),
            (
                "no probabilities",
                serde_json::json!({"choice": "B", "confidence": 0.8}),
            ),
            (
                "choice not a string",
                answer("B", good.clone(), serde_json::json!(0.8))
                    .as_object()
                    .map(|o| {
                        let mut o = o.clone();
                        o.insert("choice".into(), serde_json::json!(1));
                        Value::Object(o)
                    })
                    .expect("object"),
            ),
            ("not an object", serde_json::json!("B")),
        ];
        for (why, a) in cases {
            assert!(
                matches!(check(a), Err(LayaError::InvalidResponse(_))),
                "must reject: {why}"
            );
        }
        // An absent question is also invalid, not a default.
        let r = SystemOneResponse::from_slice(br#"{"answers":{}}"#).expect("shape ok");
        assert!(r.choice("q", &question()).is_err());
    }

    #[test]
    fn ties_and_rounding_slack_are_accepted_but_a_real_margin_is_not() {
        // Exact tie: either label is a valid argmax.
        let tie = serde_json::json!({"A": 0.5, "B": 0.5, "C": 0.0});
        assert!(check(answer("A", tie.clone(), serde_json::json!(0.5))).is_ok());
        assert!(check(answer("B", tie, serde_json::json!(0.5))).is_ok());
        // Sum within 0.02 of 1 (float32 rounding on the server) is accepted.
        let slack = serde_json::json!({"A": 0.005, "B": 0.985, "C": 0.005});
        assert!(check(answer("B", slack, serde_json::json!(0.98))).is_ok());
    }

    #[test]
    fn a_body_without_an_answers_object_is_invalid() {
        for body in [&b"not json"[..], b"[]", b"{}", br#"{"answers":[]}"#, b""] {
            assert!(
                matches!(
                    SystemOneResponse::from_slice(body),
                    Err(LayaError::InvalidResponse(_))
                ),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    // -- client against a loopback server -----------------------------------

    struct Server {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    /// A one-shot-per-connection HTTP/1.1 responder: reads a whole request
    /// (headers plus `Content-Length` body), records it, waits `delay`, then
    /// answers `status` with `body` and closes.
    async fn serve(status: u16, body: String, delay: Duration) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let base_url = format!("http://{}", listener.local_addr().expect("addr"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let log = Arc::clone(&log);
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                            let want = head
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if rest.len() >= want {
                                break;
                            }
                        }
                    }
                    log.lock()
                        .expect("log")
                        .push(String::from_utf8_lossy(&buf).to_string());
                    tokio::time::sleep(delay).await;
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Server { base_url, requests }
    }

    fn one_question_request() -> SystemOneRequest {
        SystemOneRequest::new("s").with_question("q", question())
    }

    fn ok_body() -> String {
        serde_json::json!({
            "model": "typed-decisions",
            "answers": {"q": {
                "choice": "B",
                "probabilities": {"A": 0.1, "B": 0.8, "C": 0.1},
                "confidence": 0.8
            }},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string()
    }

    #[tokio::test]
    async fn happy_path_round_trips_and_validates() {
        let server = serve(200, ok_body(), Duration::ZERO).await;
        let client = LayaClient::new(LayaConfig::new(&server.base_url));
        let req = one_question_request();
        let resp = client.systemone("test", &req).await.expect("answers");
        assert_eq!(resp.model, "typed-decisions");
        let a = resp
            .choice("q", req.question("q").expect("q"))
            .expect("valid");
        assert_eq!(a.choice, "B");

        let seen = server.requests.lock().expect("log").clone();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /v1/systemone HTTP/1.1"),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains(r#""type":"choice""#));
    }

    #[tokio::test]
    async fn a_non_2xx_status_is_a_typed_http_error() {
        for status in [401u16, 422, 500, 503] {
            let server = serve(status, "{}".to_string(), Duration::ZERO).await;
            let client = LayaClient::new(LayaConfig::new(&server.base_url));
            assert_eq!(
                client
                    .systemone("test", &one_question_request())
                    .await
                    .expect_err("http"),
                LayaError::Http(status)
            );
            assert_eq!(
                server.requests.lock().expect("log").len(),
                1,
                "an HTTP error is never retried"
            );
        }
    }

    #[tokio::test]
    async fn garbage_json_is_an_invalid_response() {
        let server = serve(200, "<html>oops</html>".to_string(), Duration::ZERO).await;
        let client = LayaClient::new(LayaConfig::new(&server.base_url));
        assert!(matches!(
            client.systemone("test", &one_question_request()).await,
            Err(LayaError::InvalidResponse(_))
        ));
    }

    #[tokio::test]
    async fn answers_that_do_not_validate_are_rejected_not_repaired() {
        // Probabilities that do not sum to one.
        let bad_sum = serde_json::json!({"model": "m", "answers": {"q": {
            "choice": "B", "probabilities": {"A": 0.5, "B": 0.9, "C": 0.5}, "confidence": 0.9}}})
        .to_string();
        // A chosen label that was never offered.
        let bad_label = serde_json::json!({"model": "m", "answers": {"q": {
            "choice": "Z", "probabilities": {"A": 0.1, "B": 0.8, "C": 0.1}, "confidence": 0.9}}})
        .to_string();
        for body in [bad_sum, bad_label] {
            let server = serve(200, body, Duration::ZERO).await;
            let client = LayaClient::new(LayaConfig::new(&server.base_url));
            let req = one_question_request();
            let resp = client
                .systemone("test", &req)
                .await
                .expect("transport is fine");
            assert!(matches!(
                resp.choice("q", req.question("q").expect("q")),
                Err(LayaError::InvalidResponse(_))
            ));
        }
    }

    #[tokio::test]
    async fn an_oversized_body_is_abandoned() {
        let big = "x".repeat(DEFAULT_MAX_RESPONSE_BYTES + 10);
        let server = serve(200, big, Duration::ZERO).await;
        let client = LayaClient::new(LayaConfig::new(&server.base_url));
        assert!(matches!(
            client.systemone("test", &one_question_request()).await,
            Err(LayaError::InvalidResponse(m)) if m.contains("exceeded")
        ));
    }

    #[tokio::test]
    async fn a_slow_server_times_out() {
        let server = serve(200, ok_body(), Duration::from_secs(5)).await;
        let mut config = LayaConfig::new(&server.base_url);
        config.timeout = Duration::from_millis(100);
        let client = LayaClient::new(config);
        assert_eq!(
            client
                .systemone("test", &one_question_request())
                .await
                .expect_err("slow"),
            LayaError::Timeout(Duration::from_millis(100))
        );
    }

    #[tokio::test]
    async fn a_server_that_is_down_is_a_connect_error_and_health_is_false() {
        // Bind then drop: the port is closed, so the connection is refused.
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            format!("http://{}", l.local_addr().expect("addr"))
        };
        let client = LayaClient::new(LayaConfig::new(closed));
        assert!(matches!(
            client.systemone("test", &one_question_request()).await,
            Err(LayaError::Connect(_))
        ));
        assert!(!client.health().await);
    }

    #[tokio::test]
    async fn health_is_true_only_for_a_2xx() {
        let up = serve(200, r#"{"status":"ok"}"#.to_string(), Duration::ZERO).await;
        assert!(
            LayaClient::new(LayaConfig::new(&up.base_url))
                .health()
                .await
        );
        let seen = up.requests.lock().expect("log").clone();
        assert!(seen[0].starts_with("GET /health"), "{}", seen[0]);

        let broken = serve(500, "{}".to_string(), Duration::ZERO).await;
        assert!(
            !LayaClient::new(LayaConfig::new(&broken.base_url))
                .health()
                .await
        );
    }

    #[tokio::test]
    async fn the_bearer_header_is_sent_only_when_configured() {
        let with = serve(200, ok_body(), Duration::ZERO).await;
        let mut config = LayaConfig::new(&with.base_url);
        config.api_key = Some(Token::new(KEY));
        LayaClient::new(config)
            .systemone("test", &one_question_request())
            .await
            .expect("ok");
        let sent = with.requests.lock().expect("log")[0].to_ascii_lowercase();
        assert!(
            sent.contains(&format!(
                "authorization: bearer {}",
                KEY.to_ascii_lowercase()
            )),
            "{sent}"
        );

        let without = serve(200, ok_body(), Duration::ZERO).await;
        LayaClient::new(LayaConfig::new(&without.base_url))
            .systemone("test", &one_question_request())
            .await
            .expect("ok");
        let sent = without.requests.lock().expect("log")[0].to_ascii_lowercase();
        assert!(!sent.contains("authorization"), "{sent}");

        // /health never carries the key.
        let health = serve(200, "{}".to_string(), Duration::ZERO).await;
        let mut config = LayaConfig::new(&health.base_url);
        config.api_key = Some(Token::new(KEY));
        assert!(LayaClient::new(config).health().await);
        let sent = health.requests.lock().expect("log")[0].to_ascii_lowercase();
        assert!(!sent.contains("authorization"), "{sent}");
    }

    #[tokio::test]
    async fn an_invalid_request_never_reaches_the_network() {
        let server = serve(200, ok_body(), Duration::ZERO).await;
        let client = LayaClient::new(LayaConfig::new(&server.base_url));
        assert!(matches!(
            client.systemone("test", &SystemOneRequest::new("s")).await,
            Err(LayaError::InvalidRequest(_))
        ));
        assert!(server.requests.lock().expect("log").is_empty());
    }
}
