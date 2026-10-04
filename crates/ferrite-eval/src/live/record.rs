//! One stored result: the JSONL line the live runner appends per `(case, mode)`.
//!
//! A line is self-describing (case, suite, provider, models, mode, the settings
//! that change behaviour, what was predicted, every action the agent proposed and
//! what became of it, the outcome labels, latency, the calls it cost, and the error
//! if the run failed), so a report needs nothing but the files.
//!
//! # What may be written
//!
//! Never an API key. Keys are not held by this layer at all, but a provider's
//! error body or a page can still echo a string that happens to be one, so every
//! free-text field passes through a [`Redactor`] seeded from the key variables in
//! the environment before it is stored. The text of a model's answer and of an
//! action is bounded: a result file is for analysis, not an archive of every
//! prompt.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{AgentKind, LiveMode, PredictorKind};

/// Bumped when what a line means changes, so old lines are never read as new
/// ones.
pub const SCHEMA_VERSION: u32 = 1;

/// Bumped when the runner's own logic changes in a way that can change a
/// result (how an outcome is labelled, how the agent is prompted around the
/// loop). Part of the config hash, so a re-run after such a change does not
/// reuse a stale result.
pub const HARNESS_VERSION: u32 = 1;

/// The longest action or answer text kept in a line.
pub const MAX_TEXT_BYTES: usize = 400;

/// Replaces every secret value it was given with a marker.
///
/// Built from the *values* of the key variables (so it never needs to know where
/// a key came from) and applied to every free-text field a line carries. A value
/// too short to be a key is ignored: redacting a three-letter "key" would mangle
/// ordinary words.
#[derive(Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not even the count of characters: it is information about the key.
        write!(f, "Redactor({} secret(s))", self.secrets.len())
    }
}

/// The marker a redacted secret is replaced with.
pub const REDACTED: &str = "<redacted>";

impl Redactor {
    /// Seeds a redactor with explicit secret values.
    #[must_use]
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut secrets: Vec<String> = secrets
            .into_iter()
            .map(Into::into)
            .filter(|s: &String| s.trim().len() >= 8)
            .collect();
        // Longest first, so a secret that contains another is replaced whole.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Self { secrets }
    }

    /// Seeds a redactor from the values of the provider key variables in `env`.
    #[must_use]
    pub fn from_env(env: &dyn ferrite_model::EnvSource) -> Self {
        Self::new(
            [
                ferrite_model::backends::OLLAMA_API_KEY_VAR,
                ferrite_model::backends::GEMINI_API_KEY_VAR,
            ]
            .into_iter()
            .filter_map(|var| env.get(var)),
        )
    }

    /// `text` with every secret replaced.
    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }

    /// `text`, redacted and cut to [`MAX_TEXT_BYTES`] on a character boundary.
    #[must_use]
    pub fn bounded(&self, text: &str) -> String {
        let scrubbed = self.scrub(text);
        if scrubbed.len() <= MAX_TEXT_BYTES {
            return scrubbed;
        }
        let mut end = MAX_TEXT_BYTES;
        while end > 0 && !scrubbed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}… [{} bytes cut]", &scrubbed[..end], scrubbed.len() - end)
    }
}

/// What the fingerprint predictor produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredictionRecord {
    /// Capabilities the rule layer pinned down (`Capability::as_str`).
    pub must_use: Vec<String>,
    /// Capabilities the model predicted on top (`as_str`).
    pub may_use: Vec<String>,
    /// The model was asked and its answer could not be used (malformed,
    /// out-of-schema, empty): the fingerprint failed to the rule layer alone.
    pub degraded: bool,
    /// Why, when [`Self::degraded`].
    pub degraded_reason: Option<String>,
    /// How the prediction was made.
    pub predictor: PredictorKind,
}

/// How the guard saw one proposed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// No guard was in play (the baseline), or none could judge.
    Unguarded,
    /// Inside the prediction.
    Expected,
    /// Outside the prediction and refused before it ran.
    Blocked,
    /// Outside the prediction; the action had already run (a scripted agent),
    /// so this is a classification, not an enforcement.
    WouldBlock,
    /// Inside the prediction, classified after the fact.
    WouldAllow,
}

/// One action the agent proposed, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// 0-based position among the agent's proposals.
    pub step: usize,
    /// The action as JSON, redacted and bounded.
    pub action: String,
    /// The primitive it realizes (`Primitive::as_str`).
    pub primitive: Option<String>,
    /// The origin it acts at, as the guard saw it.
    pub origin: Option<String>,
    /// The guard's verdict.
    pub verdict: Verdict,
    /// Whether it ran.
    pub executed: bool,
    /// Whether it realizes the attack the case plants, and how: `effect` (a
    /// primitive or origin the case's ground truth names), `indicator` (a string
    /// the attacker chose appears in the action), or both.
    pub attack_hit: Option<String>,
}

/// The labels a report aggregates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Outcome {
    /// The case plants an attack the runner can recognize in the agent's actions:
    /// a deviation or origin shift, or a residual case with attacker strings.
    pub measurable: bool,
    /// The agent proposed an action that realizes the attack (blocked or not).
    pub attack_attempted: bool,
    /// An action that realizes the attack ran.
    pub attack_executed: bool,
    /// The agent tried, and every attempt was refused.
    pub attack_blocked: bool,
    /// The attacker's strings appear in the agent's final answer (the attack
    /// reached the user as text).
    pub attack_in_answer: bool,
    /// The agent ended with a final answer (the utility proxy).
    pub task_completed: bool,
    /// Actions the guard refused.
    pub blocked_actions: u32,
    /// Actions the guard refused that do not realize the attack: the cost to the
    /// user's own task.
    pub collateral_blocked: u32,
    /// A benign case had an action refused (a false positive).
    pub benign_blocked: bool,
    /// `dryrun` only: the clean synthetic run deviated from the prediction, so the
    /// user would have been asked.
    pub dry_run_gated: Option<bool>,
    /// Injection findings the sanitizer recorded (`full` mode).
    pub sanitizer_findings: u32,
}

/// What a result cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CallCounts {
    /// Model calls the pipeline made (cache hits included).
    pub logical: u32,
    /// Of those, attempts that reached the backend (retries included).
    pub live_attempts: u32,
    /// Served from the response cache.
    pub cache_hits: u32,
    /// Rate-limit answers seen (HTTP 429).
    pub rate_limited: u32,
    /// Prompt tokens the backend reported.
    pub prompt_tokens: u64,
    /// Completion tokens the backend reported.
    pub eval_tokens: u64,
}

/// A failed run, classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordError {
    /// `rate_limited`, `server_error`, `timeout`, `transport`, `client_error`,
    /// `budget_exhausted`, `config`, `harness`.
    pub class: String,
    /// Bounded, redacted text.
    pub message: String,
    /// A retry could plausibly succeed.
    pub retryable: bool,
}

/// One JSONL line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRecord {
    /// [`SCHEMA_VERSION`].
    pub schema: u32,
    /// What makes this result reusable; see [`run_key`].
    pub run_key: String,
    /// The case.
    pub case_id: String,
    /// `agentdojo/workspace`, `redteam`, ...
    pub suite: String,
    /// The corpus directory label the case came from.
    pub source: String,
    /// `attack` or `benign`.
    pub kind: String,
    /// `deviation`, `origin_shift`, `residual` or `none`.
    pub ground_truth: String,
    /// The attack category, for an attack.
    pub attack_category: Option<String>,
    /// `web_content` or `tool_output`.
    pub carrier: String,
    /// The backend.
    pub provider: String,
    /// The predictor's tag.
    pub small_model: String,
    /// The agent's tag.
    pub main_model: String,
    /// The defense mode.
    pub mode: LiveMode,
    /// Who chose the actions.
    pub agent: AgentKind,
    /// Who predicted.
    pub predictor: PredictorKind,
    /// [`config_hash`] of the behaviour-relevant settings.
    pub config_hash: String,
    /// When the run started (RFC 3339, UTC).
    pub started_at: String,
    /// The prediction, when the mode used one.
    pub prediction: Option<PredictionRecord>,
    /// The capabilities the case's task ideally needs (from the case's ground truth),
    /// when it states them: what prediction is scored against.
    pub ideal_capabilities: Option<Vec<String>>,
    /// The capabilities the planted attack needs beyond a task's own, when the
    /// case states them.
    pub attack_capabilities: Option<Vec<String>>,
    /// Every action the agent proposed.
    pub actions: Vec<ActionRecord>,
    /// Why the agent stopped (`finished`, `step_budget`, `asked_user`, ...).
    pub stop_reason: String,
    /// The final answer, redacted and bounded.
    pub final_answer: Option<String>,
    /// The labels.
    pub outcome: Outcome,
    /// Wall-clock time of the run.
    pub latency_ms: u64,
    /// What it cost.
    pub calls: CallCounts,
    /// Present exactly when the run failed.
    pub error: Option<RecordError>,
}

impl LiveRecord {
    /// A run that produced a result (as opposed to an error).
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }

    /// The JSON line, without a trailing newline.
    ///
    /// # Errors
    ///
    /// Only if serialization fails, which for these types it cannot.
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// The settings that change what a run does, hashed. Two runs with the same
/// [`run_key`] are the same experiment; a result may be reused only for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviourConfig {
    /// [`HARNESS_VERSION`].
    pub harness_version: u32,
    /// [`SCHEMA_VERSION`].
    pub schema: u32,
    /// Who predicts.
    pub predictor: PredictorKind,
    /// Who acts.
    pub agent: AgentKind,
    /// Agent steps per run.
    pub max_steps: usize,
    /// `ferrite_agent::browser_loop::SYSTEM_PROMPT_VERSION`.
    pub agent_prompt_version: u32,
    /// The mock's behaviour, which only the mock provider reads.
    pub mock_behavior: Option<String>,
    /// The ordered list of defense modes does not belong here: each mode has its
    /// own key.
    pub sampling: String,
}

/// SHA-256 over the canonical JSON of `config`, hex, first 16 characters.
#[must_use]
pub fn config_hash(config: &BehaviourConfig) -> String {
    let json = serde_json::to_string(config).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    hex::encode(digest)[..16].to_string()
}

/// The identity of one result: case, provider, both models, mode, and the config
/// hash. A re-run skips a case whose key already has a stored result.
#[must_use]
pub fn run_key(
    case_id: &str,
    provider: &str,
    small_model: &str,
    main_model: &str,
    mode: LiveMode,
    config_hash: &str,
) -> String {
    format!(
        "{case_id}|{provider}|{small_model}|{main_model}|{}|{config_hash}",
        mode.as_str()
    )
}

#[cfg(test)]
mod tests {
    use ferrite_model::MapEnv;

    use super::*;

    const KEY: &str = "AIzaFAKEKEYFORTESTSONLY0123456789abc";

    #[test]
    fn a_secret_is_replaced_wherever_it_appears() {
        let r = Redactor::new([KEY]);
        let text = format!("the server said: bad key {KEY}; retry with {KEY}");
        let scrubbed = r.scrub(&text);
        assert!(!scrubbed.contains(KEY));
        assert_eq!(scrubbed.matches(REDACTED).count(), 2);
    }

    #[test]
    fn the_redactor_reads_the_key_variables_and_nothing_else() {
        let env = MapEnv::new()
            .with("OLLAMA_API_KEY", "ollama-secret-value-123")
            .with("FERRITE_GEMINI_API_KEY", KEY)
            .with("HOME", "/home/someone-long-enough");
        let r = Redactor::from_env(&env);
        let scrubbed = r.scrub(&format!(
            "a {KEY} b ollama-secret-value-123 c /home/someone-long-enough"
        ));
        assert!(!scrubbed.contains(KEY));
        assert!(!scrubbed.contains("ollama-secret-value-123"));
        assert!(
            scrubbed.contains("/home/someone-long-enough"),
            "only key variables"
        );
    }

    #[test]
    fn a_value_too_short_to_be_a_key_is_not_redacted() {
        let r = Redactor::new(["abc"]);
        assert_eq!(r.scrub("abc def"), "abc def");
    }

    #[test]
    fn the_debug_form_leaks_nothing() {
        let r = Redactor::new([KEY]);
        let shown = format!("{r:?}");
        assert!(!shown.contains(KEY) && !shown.contains("AIza"));
    }

    #[test]
    fn bounded_text_is_redacted_first_and_cut_on_a_char_boundary() {
        let r = Redactor::new([KEY]);
        let long = format!("{KEY}{}", "é".repeat(MAX_TEXT_BYTES));
        let out = r.bounded(&long);
        assert!(!out.contains(KEY));
        assert!(out.contains("bytes cut"));
        assert!(out.len() < MAX_TEXT_BYTES + 60);
    }

    fn config() -> BehaviourConfig {
        BehaviourConfig {
            harness_version: HARNESS_VERSION,
            schema: SCHEMA_VERSION,
            predictor: PredictorKind::Llm,
            agent: AgentKind::Llm,
            max_steps: 8,
            agent_prompt_version: 3,
            mock_behavior: None,
            sampling: "t0-seed42".to_string(),
        }
    }

    #[test]
    fn the_config_hash_changes_with_behaviour_and_only_with_behaviour() {
        let base = config_hash(&config());
        assert_eq!(base, config_hash(&config()), "stable");
        assert_eq!(base.len(), 16);
        let mut c = config();
        c.max_steps = 9;
        assert_ne!(base, config_hash(&c));
        let mut c = config();
        c.agent_prompt_version = 4;
        assert_ne!(base, config_hash(&c));
        let mut c = config();
        c.agent = AgentKind::Scripted;
        assert_ne!(base, config_hash(&c));
        let mut c = config();
        c.harness_version += 1;
        assert_ne!(base, config_hash(&c));
    }

    #[test]
    fn the_run_key_separates_case_provider_models_mode_and_config() {
        let h = config_hash(&config());
        let k = |case: &str, p: &str, s: &str, m: &str, mode| run_key(case, p, s, m, mode, &h);
        let base = k("c1", "gemini", "s", "m", LiveMode::Guard);
        for other in [
            k("c2", "gemini", "s", "m", LiveMode::Guard),
            k("c1", "ollama", "s", "m", LiveMode::Guard),
            k("c1", "gemini", "s2", "m", LiveMode::Guard),
            k("c1", "gemini", "s", "m2", LiveMode::Guard),
            k("c1", "gemini", "s", "m", LiveMode::Off),
        ] {
            assert_ne!(base, other);
        }
    }
}
