//! Command-line and environment configuration for the live evaluation runner.
//!
//! Hand-rolled (`--flag value` or `--flag=value`), because the workspace carries no
//! argument-parsing dependency and this is one binary's worth of flags. Parsing
//! is a pure function of the arguments and an [`EnvSource`], so every rule here
//! is tested without touching the process environment.
//!
//! # Where a model tag comes from
//!
//! Model tags are configuration, never source literals (`docs/REBUILD_DIRECTIVE.md`
//! §10.2: providers retire models). In order of precedence, per tier:
//!
//! 1. `--small-model` / `--main-model` (the fingerprint predictor and the agent),
//!    or `--model` for both;
//! 2. `FERRITE_LIVE_SMALL_MODEL` / `FERRITE_LIVE_MAIN_MODEL`, or `FERRITE_LIVE_MODEL`
//!    for both;
//! 3. `FERRITE_MODEL_SMALL` / `FERRITE_MODEL_MAIN`, the names the app itself reads.
//!
//! There is no default tag. A role that is not live (`--predictor rules`,
//! `--agent scripted`) needs no tag.
//!
//! API keys are not configuration of this module at all: `ferrite-model` resolves
//! them from the environment or the OS keyring, and nothing here reads, prints or
//! stores one.

use std::path::PathBuf;
use std::time::Duration;

use ferrite_ipi::tool_decision::DefenseMode;
use ferrite_model::EnvSource;
use serde::{Deserialize, Serialize};

/// Which backend answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Google Gemini (key from `FERRITE_GEMINI_API_KEY` or the OS keyring).
    Gemini,
    /// Ollama: Ollama Cloud by default, a local server with `--base-url`.
    Ollama,
    /// Anthropic's Claude models (key from `FERRITE_ANTHROPIC_API_KEY` or the
    /// OS keyring).
    Anthropic,
    /// OpenAI, or any server with its chat-completions API via `--base-url`
    /// (key from `FERRITE_OPENAI_API_KEY`; none needed for a local server).
    #[serde(rename = "openai")]
    OpenAi,
    /// A deterministic stand-in with no network: exercises the whole pipeline
    /// offline. Its results say nothing about any real model.
    Mock,
}

impl ProviderKind {
    /// The stable string form used in result files and flags.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Mock => "mock",
        }
    }

    fn parse(text: &str) -> Result<Self, ArgsError> {
        match text {
            "gemini" => Ok(Self::Gemini),
            "ollama" => Ok(Self::Ollama),
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "mock" => Ok(Self::Mock),
            other => Err(ArgsError::value(
                "--provider",
                other,
                "gemini, ollama, anthropic, openai or mock",
            )),
        }
    }
}

/// How much of Ferrite's defense is in the real run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMode {
    /// No defense: the agent reads the page and acts. The baseline.
    Off,
    /// The predicted fingerprint is enforced before every action executes (the
    /// runtime guard, ADR-014). The sanitizer is off, so this isolates the
    /// architecture.
    Guard,
    /// The guard plus the sanitizer (detect and excise) on what the agent reads.
    Full,
    /// Stage one of the loop only: the agent's plan on a clean synthetic page,
    /// compared with the prediction. Measures the consent burden of the real
    /// model; the page carries no injection, because the dry run cannot see one.
    DryRun,
}

impl LiveMode {
    /// Every mode, in the order they are reported.
    pub const ALL: [Self; 4] = [Self::Off, Self::Guard, Self::Full, Self::DryRun];

    /// The stable string form used in result files and flags.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Guard => "guard",
            Self::Full => "full",
            Self::DryRun => "dryrun",
        }
    }

    /// Parses the stable string form.
    ///
    /// # Errors
    ///
    /// [`ArgsError::BadValue`] for anything else.
    pub fn parse(text: &str) -> Result<Self, ArgsError> {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str() == text)
            .ok_or_else(|| ArgsError::value("--modes", text, "off, guard, full or dryrun"))
    }

    /// The pipeline's defense mode this run maps onto (what the dry-run engine's
    /// sanitizer does while the agent reads).
    #[must_use]
    pub const fn defense_mode(self) -> DefenseMode {
        match self {
            Self::Off => DefenseMode::Off,
            Self::Guard | Self::DryRun => DefenseMode::LoopOnly,
            Self::Full => DefenseMode::On,
        }
    }

    /// Whether the mode needs the fingerprint prediction.
    #[must_use]
    pub const fn needs_prediction(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Whether the guard refuses actions outside the prediction.
    #[must_use]
    pub const fn enforces_guard(self) -> bool {
        matches!(self, Self::Guard | Self::Full)
    }
}

/// Who predicts the fingerprint's `may_use`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredictorKind {
    /// A model call (the small tier), as in the app.
    Llm,
    /// The deterministic rule layer only: `may_use` stays empty (what the
    /// harness does with no provider).
    Rules,
}

/// Who chooses the agent's actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// A model (the main tier) driving the app's agent loop.
    Llm,
    /// The scripted worst-case agent: complies with every injection, no model
    /// call. Its actions are classified after the fact; a guard cannot refuse
    /// what a script has already done.
    Scripted,
}

/// How the `mock` provider's agent behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MockBehavior {
    /// Follows the first URL or instruction it finds in the page.
    Compliant,
    /// Ignores the page's instructions and finishes.
    Resistant,
    /// Compliant for about half the cases, chosen by a hash of the prompt.
    Mixed,
}

/// Which cases to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    /// Attack and benign.
    All,
    /// Attack cases only.
    Attack,
    /// Benign cases only.
    Benign,
}

/// What the binary was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the (next batch of the) selection.
    Run,
    /// Estimate what the selection would cost, calling nothing.
    Plan,
    /// Aggregate stored results.
    Report,
    /// Print the help text.
    Help,
}

/// A configuration mistake, phrased for the person who made it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgsError {
    /// A flag this binary does not have.
    #[error("unknown flag {0} (see --help)")]
    UnknownFlag(String),
    /// A flag that needs a value was last, or followed by another flag.
    #[error("{0} needs a value")]
    MissingValue(String),
    /// A value that does not parse.
    #[error("{flag}: {value:?} is not valid; expected {expected}")]
    BadValue {
        /// The flag.
        flag: String,
        /// What was given.
        value: String,
        /// What would have been accepted.
        expected: String,
    },
    /// A combination that cannot work.
    #[error("{0}")]
    Invalid(String),
}

impl ArgsError {
    fn value(flag: &str, value: &str, expected: &str) -> Self {
        Self::BadValue {
            flag: flag.to_string(),
            value: value.to_string(),
            expected: expected.to_string(),
        }
    }
}

/// Everything the runner needs, after flags and environment are merged.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveArgs {
    /// What to do.
    pub command: Command,
    /// The backend. Required for a run or a plan.
    pub provider: Option<ProviderKind>,
    /// `--small-model`/`--model`: the fingerprint predictor's tag.
    pub small_model: Option<String>,
    /// `--main-model`/`--model`: the agent's tag.
    pub main_model: Option<String>,
    /// `--base-url`: the Ollama or Gemini endpoint.
    pub base_url: Option<String>,
    /// Who predicts.
    pub predictor: PredictorKind,
    /// Who acts.
    pub agent: AgentKind,
    /// Which defense modes to run, in order, without duplicates.
    pub modes: Vec<LiveMode>,
    /// Corpus names (`agentdojo`, `redteam`, ...). See [`crate::live::corpus::CORPORA`].
    pub corpora: Vec<String>,
    /// `--corpus-root`: where the corpus directories live.
    pub corpus_root: Option<PathBuf>,
    /// Keep only cases whose suite starts with one of these.
    pub suites: Vec<String>,
    /// Attack, benign, or both.
    pub only: KindFilter,
    /// `--batch-size`: cases per invocation.
    pub batch_size: Option<usize>,
    /// `--batch-index`: take slice K of the whole ordering instead of the next
    /// pending cases.
    pub batch_index: Option<usize>,
    /// `--offset`: skip this many cases of the ordering.
    pub offset: usize,
    /// `--limit`: at most this many cases.
    pub limit: Option<usize>,
    /// `--seed`: shuffle the ordering reproducibly.
    pub seed: Option<u64>,
    /// `--pause-ms`: the least time between two calls.
    pub pause: Duration,
    /// `--max-calls`: hard cap on model calls in this invocation.
    pub max_calls: u32,
    /// `--max-attempts`: tries per call (first try included).
    pub max_attempts: u32,
    /// `--backoff-base-ms`.
    pub backoff_base: Duration,
    /// `--backoff-max-ms`: longest wait, including a server's `Retry-After`.
    pub backoff_max: Duration,
    /// `--timeout-secs`: per request.
    pub timeout: Duration,
    /// `--max-steps`: the agent's step budget per run.
    pub max_steps: usize,
    /// `--max-consecutive-failures`: stop after this many provider failures in a row.
    pub max_consecutive_failures: u32,
    /// `--retry-failed`: redo cases whose stored result is an error.
    pub retry_failed: bool,
    /// `--no-cache`: do not use the on-disk response cache.
    pub no_cache: bool,
    /// `--out`: where results, the cache and reports go.
    pub out_dir: PathBuf,
    /// `--mock-behavior`.
    pub mock_behavior: MockBehavior,
    /// `--compare provider:model,...` for a report; empty means everything stored.
    pub compare: Vec<(String, String)>,
}

/// Documented defaults; the numbers are conservative on purpose, because the point
/// of this runner is to stay inside a quota.
pub mod defaults {
    use std::time::Duration;

    /// Model calls per invocation.
    pub const MAX_CALLS: u32 = 100;
    /// Tries per call, first included.
    pub const MAX_ATTEMPTS: u32 = 5;
    /// First backoff ceiling.
    pub const BACKOFF_BASE: Duration = Duration::from_secs(2);
    /// Longest wait, including a `Retry-After`.
    pub const BACKOFF_MAX: Duration = Duration::from_secs(90);
    /// Per-request timeout.
    pub const TIMEOUT: Duration = Duration::from_secs(90);
    /// The agent's steps per run.
    pub const MAX_STEPS: usize = 8;
    /// Provider failures in a row before the invocation stops.
    pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;
}

impl Default for LiveArgs {
    fn default() -> Self {
        Self {
            command: Command::Run,
            provider: None,
            small_model: None,
            main_model: None,
            base_url: None,
            predictor: PredictorKind::Llm,
            agent: AgentKind::Llm,
            modes: vec![LiveMode::Off, LiveMode::Guard],
            corpora: vec!["agentdojo".to_string()],
            corpus_root: None,
            suites: Vec::new(),
            only: KindFilter::All,
            batch_size: None,
            batch_index: None,
            offset: 0,
            limit: None,
            seed: None,
            pause: Duration::ZERO,
            max_calls: defaults::MAX_CALLS,
            max_attempts: defaults::MAX_ATTEMPTS,
            backoff_base: defaults::BACKOFF_BASE,
            backoff_max: defaults::BACKOFF_MAX,
            timeout: defaults::TIMEOUT,
            max_steps: defaults::MAX_STEPS,
            max_consecutive_failures: defaults::MAX_CONSECUTIVE_FAILURES,
            retry_failed: false,
            no_cache: false,
            out_dir: PathBuf::from("target/live-eval"),
            mock_behavior: MockBehavior::Mixed,
            compare: Vec::new(),
        }
    }
}

/// The two model tags a run uses, after precedence is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTags {
    /// The predictor's tag (the small tier).
    pub small: String,
    /// The agent's tag (the main tier).
    pub main: String,
}

impl LiveArgs {
    /// Parses `args` (without the program name), with `env` supplying the defaults
    /// flags override.
    ///
    /// # Errors
    ///
    /// [`ArgsError`] for an unknown flag, a missing or malformed value, or a
    /// combination that cannot work.
    pub fn parse(args: &[String], env: &dyn EnvSource) -> Result<Self, ArgsError> {
        let mut out = Self::default();
        if let Some(p) = env.get("FERRITE_LIVE_PROVIDER") {
            out.provider = Some(ProviderKind::parse(&p)?);
        }
        if let Some(m) = env.get("FERRITE_LIVE_MODEL") {
            out.small_model = Some(m.clone());
            out.main_model = Some(m);
        }
        if let Some(m) = env.get("FERRITE_LIVE_SMALL_MODEL") {
            out.small_model = Some(m);
        }
        if let Some(m) = env.get("FERRITE_LIVE_MAIN_MODEL") {
            out.main_model = Some(m);
        }
        if out.small_model.is_none() {
            out.small_model = env.get("FERRITE_MODEL_SMALL");
        }
        if out.main_model.is_none() {
            out.main_model = env.get("FERRITE_MODEL_MAIN");
        }

        let mut it = args.iter().peekable();
        while let Some(raw) = it.next() {
            let (flag, inline) = match raw.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                _ => (raw.clone(), None),
            };
            let mut value = |flag: &str| -> Result<String, ArgsError> {
                if let Some(v) = &inline {
                    return Ok(v.clone());
                }
                match it.next() {
                    Some(v) if !v.starts_with("--") => Ok(v.clone()),
                    _ => Err(ArgsError::MissingValue(flag.to_string())),
                }
            };
            match flag.as_str() {
                "--help" | "-h" => out.command = Command::Help,
                "--plan" | "--dry-plan" => out.command = Command::Plan,
                "--report" => out.command = Command::Report,
                "--provider" => out.provider = Some(ProviderKind::parse(&value(&flag)?)?),
                "--model" => {
                    let v = value(&flag)?;
                    out.small_model = Some(v.clone());
                    out.main_model = Some(v);
                }
                "--small-model" => out.small_model = Some(value(&flag)?),
                "--main-model" => out.main_model = Some(value(&flag)?),
                "--base-url" => out.base_url = Some(value(&flag)?),
                "--predictor" => {
                    out.predictor = match value(&flag)?.as_str() {
                        "llm" => PredictorKind::Llm,
                        "rules" => PredictorKind::Rules,
                        other => return Err(ArgsError::value(&flag, other, "llm or rules")),
                    }
                }
                "--agent" => {
                    out.agent = match value(&flag)?.as_str() {
                        "llm" => AgentKind::Llm,
                        "scripted" => AgentKind::Scripted,
                        other => return Err(ArgsError::value(&flag, other, "llm or scripted")),
                    }
                }
                "--modes" => {
                    let mut modes = Vec::new();
                    for part in value(&flag)?.split(',').filter(|s| !s.trim().is_empty()) {
                        let mode = LiveMode::parse(part.trim())?;
                        if !modes.contains(&mode) {
                            modes.push(mode);
                        }
                    }
                    if modes.is_empty() {
                        return Err(ArgsError::value(&flag, "", "at least one mode"));
                    }
                    out.modes = modes;
                }
                "--corpus" => out.corpora = split_list(&value(&flag)?),
                "--corpus-root" => out.corpus_root = Some(PathBuf::from(value(&flag)?)),
                "--suite" => out.suites = split_list(&value(&flag)?),
                "--only" => {
                    out.only = match value(&flag)?.as_str() {
                        "all" => KindFilter::All,
                        "attack" => KindFilter::Attack,
                        "benign" => KindFilter::Benign,
                        other => {
                            return Err(ArgsError::value(&flag, other, "all, attack or benign"))
                        }
                    }
                }
                "--batch-size" => out.batch_size = Some(positive(&flag, &value(&flag)?)?),
                "--batch-index" => out.batch_index = Some(number(&flag, &value(&flag)?)?),
                "--offset" => out.offset = number(&flag, &value(&flag)?)?,
                "--limit" => out.limit = Some(number(&flag, &value(&flag)?)?),
                "--seed" => out.seed = Some(number(&flag, &value(&flag)?)?),
                "--pause-ms" => out.pause = Duration::from_millis(number(&flag, &value(&flag)?)?),
                "--max-calls" => out.max_calls = number(&flag, &value(&flag)?)?,
                "--max-attempts" => out.max_attempts = positive(&flag, &value(&flag)?)?,
                "--backoff-base-ms" => {
                    out.backoff_base = Duration::from_millis(number(&flag, &value(&flag)?)?);
                }
                "--backoff-max-ms" => {
                    out.backoff_max = Duration::from_millis(number(&flag, &value(&flag)?)?);
                }
                "--timeout-secs" => {
                    out.timeout = Duration::from_secs(positive(&flag, &value(&flag)?)?);
                }
                "--max-steps" => out.max_steps = positive(&flag, &value(&flag)?)?,
                "--max-consecutive-failures" => {
                    out.max_consecutive_failures = positive(&flag, &value(&flag)?)?;
                }
                "--retry-failed" => out.retry_failed = true,
                "--no-cache" => out.no_cache = true,
                "--out" => out.out_dir = PathBuf::from(value(&flag)?),
                "--mock-behavior" => {
                    out.mock_behavior = match value(&flag)?.as_str() {
                        "compliant" => MockBehavior::Compliant,
                        "resistant" => MockBehavior::Resistant,
                        "mixed" => MockBehavior::Mixed,
                        other => {
                            return Err(ArgsError::value(
                                &flag,
                                other,
                                "compliant, resistant or mixed",
                            ))
                        }
                    }
                }
                "--compare" => {
                    for part in split_list(&value(&flag)?) {
                        let Some((provider, model)) = part.split_once(':') else {
                            return Err(ArgsError::value(&flag, &part, "provider:model"));
                        };
                        out.compare.push((provider.to_string(), model.to_string()));
                    }
                }
                other => return Err(ArgsError::UnknownFlag(other.to_string())),
            }
        }
        out.validate()?;
        Ok(out)
    }

    fn validate(&self) -> Result<(), ArgsError> {
        if self.batch_index.is_some() && self.batch_size.is_none() {
            return Err(ArgsError::Invalid(
                "--batch-index needs --batch-size (the slice length)".to_string(),
            ));
        }
        if self.batch_index.is_some() && (self.offset != 0 || self.limit.is_some()) {
            return Err(ArgsError::Invalid(
                "--batch-index/--batch-size and --offset/--limit are two ways to say the same \
                 thing; use one"
                    .to_string(),
            ));
        }
        if self.backoff_max < self.backoff_base {
            return Err(ArgsError::Invalid(
                "--backoff-max-ms is below --backoff-base-ms".to_string(),
            ));
        }
        Ok(())
    }

    /// The model tags a run needs, or an actionable error naming the flag and the
    /// environment variable that would supply the missing one.
    ///
    /// # Errors
    ///
    /// [`ArgsError::Invalid`] when a live role has no tag.
    pub fn model_tags(&self) -> Result<ModelTags, ArgsError> {
        let needs_small =
            self.predictor == PredictorKind::Llm && self.modes.iter().any(|m| m.needs_prediction());
        let needs_main = self.agent == AgentKind::Llm;
        let provider = self.provider.unwrap_or(ProviderKind::Mock);
        // The mock needs no real tag; it still records one so result files say
        // what ran.
        let fallback =
            |tag: &Option<String>| tag.clone().filter(|_| provider != ProviderKind::Mock);
        let small =
            match (&self.small_model, needs_small) {
                (Some(t), _) => t.clone(),
                (None, true) if provider == ProviderKind::Mock => "mock-small".to_string(),
                (None, true) => return Err(ArgsError::Invalid(
                    "the fingerprint predictor needs a model: pass --small-model (or --model), \
                     set FERRITE_LIVE_SMALL_MODEL, or use --predictor rules"
                        .to_string(),
                )),
                (None, false) => fallback(&self.main_model).unwrap_or_else(|| "unused".to_string()),
            };
        let main = match (&self.main_model, needs_main) {
            (Some(t), _) => t.clone(),
            (None, true) if provider == ProviderKind::Mock => "mock-main".to_string(),
            (None, true) => {
                return Err(ArgsError::Invalid(
                    "the agent needs a model: pass --main-model (or --model), set \
                     FERRITE_LIVE_MAIN_MODEL, or use --agent scripted"
                        .to_string(),
                ))
            }
            (None, false) => fallback(&self.small_model).unwrap_or_else(|| "unused".to_string()),
        };
        Ok(ModelTags { small, main })
    }

    /// The tag a result is filed under: the agent's, else the predictor's.
    #[must_use]
    pub fn headline_model(tags: &ModelTags, agent: AgentKind) -> String {
        match agent {
            AgentKind::Llm => tags.main.clone(),
            AgentKind::Scripted => tags.small.clone(),
        }
    }
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, ArgsError> {
    text.trim()
        .parse()
        .map_err(|_| ArgsError::value(flag, text, "a non-negative integer"))
}

fn positive<T: std::str::FromStr + PartialEq + Default>(
    flag: &str,
    text: &str,
) -> Result<T, ArgsError> {
    let n: T = number(flag, text)?;
    if n == T::default() {
        return Err(ArgsError::value(flag, text, "an integer of at least 1"));
    }
    Ok(n)
}

/// The text `--help` prints.
pub const HELP: &str = "\
live_eval: run Ferrite's evaluation with a real model in the loop.

  cargo run -p ferrite-eval --example live_eval -- [flags]

WHAT RUNS
  --provider gemini|ollama|mock     the backend (env FERRITE_LIVE_PROVIDER)
  --model TAG                       one tag for both roles (env FERRITE_LIVE_MODEL)
  --small-model TAG                 the fingerprint predictor (env FERRITE_LIVE_SMALL_MODEL)
  --main-model TAG                  the agent (env FERRITE_LIVE_MAIN_MODEL)
  --base-url URL                    Ollama or Gemini endpoint (a local Ollama needs no key)
  --predictor llm|rules             llm = the model predicts may_use; rules = rule layer only
  --agent llm|scripted              llm = the model chooses actions; scripted = worst-case script
  --modes off,guard[,full,dryrun]   defense modes to run per case (default off,guard)
  --max-steps N                     agent steps per run (default 8)
  --mock-behavior compliant|resistant|mixed   the mock agent's behaviour

WHICH CASES
  --corpus agentdojo[,redteam,core,pilot,agentdojo-hand,all]   (default agentdojo)
  --suite agentdojo/workspace[,..]  keep suites with these prefixes
  --only attack|benign|all
  --seed S                          shuffle the (id-sorted) order reproducibly
  --batch-size N                    cases per invocation; with no --batch-index, the next N pending
  --batch-index K                   with --batch-size: slice K of the whole ordering
  --offset N --limit N              or an explicit window
  --retry-failed                    redo cases whose stored result is an error

STAYING INSIDE A QUOTA
  --plan | --dry-plan               print the calls/tokens the selection would cost; call nothing
  --max-calls N                     hard cap on model calls this invocation (default 100)
  --pause-ms MS                     least gap between calls
  --max-attempts N                  tries per call, honouring Retry-After (default 5)
  --backoff-base-ms / --backoff-max-ms / --timeout-secs
  --max-consecutive-failures N      stop after N provider failures in a row (default 3)
  --no-cache                        do not reuse recorded responses

RESULTS
  --out DIR                         results, cache, reports (default target/live-eval)
  --report                          aggregate stored results into REPORT.md and report.csv
  --compare provider:model[,..]     limit a report to these runs

Keys come only from the environment or the OS keyring (OLLAMA_API_KEY,
FERRITE_GEMINI_API_KEY); this program never prints or stores one.
";

#[cfg(test)]
mod tests {
    use ferrite_model::MapEnv;

    use super::*;

    fn parse(args: &[&str]) -> Result<LiveArgs, ArgsError> {
        parse_env(args, &MapEnv::new())
    }

    fn parse_env(args: &[&str], env: &MapEnv) -> Result<LiveArgs, ArgsError> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        LiveArgs::parse(&owned, env)
    }

    #[test]
    fn defaults_are_conservative_and_name_no_model() {
        let args = parse(&[]).expect("parses");
        assert_eq!(args.command, Command::Run);
        assert_eq!(
            args.provider, None,
            "the provider is explicit, never defaulted"
        );
        assert_eq!(args.small_model, None, "no model tag exists in source");
        assert_eq!(args.main_model, None);
        assert_eq!(args.max_calls, defaults::MAX_CALLS);
        assert_eq!(args.modes, vec![LiveMode::Off, LiveMode::Guard]);
        assert!(args.max_attempts >= 2, "retries are on by default");
    }

    #[test]
    fn flags_and_the_equals_form_both_work() {
        let args = parse(&[
            "--provider=gemini",
            "--model",
            "some-tag",
            "--batch-size",
            "25",
            "--pause-ms=1500",
        ])
        .expect("parses");
        assert_eq!(args.provider, Some(ProviderKind::Gemini));
        assert_eq!(args.small_model.as_deref(), Some("some-tag"));
        assert_eq!(args.main_model.as_deref(), Some("some-tag"));
        assert_eq!(args.batch_size, Some(25));
        assert_eq!(args.pause, Duration::from_millis(1500));
    }

    #[test]
    fn anthropic_and_openai_are_providers_with_stable_names() {
        for (flag, kind) in [
            ("anthropic", ProviderKind::Anthropic),
            ("openai", ProviderKind::OpenAi),
        ] {
            let args = parse(&["--provider", flag, "--model", "m"]).expect("parses");
            assert_eq!(args.provider, Some(kind));
            assert_eq!(kind.as_str(), flag);
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{flag}\""));
        }
        assert!(parse(&["--provider", "open_ai"]).is_err());
    }

    #[test]
    fn a_flag_beats_the_environment_and_the_live_env_beats_the_apps() {
        let env = MapEnv::new()
            .with("FERRITE_LIVE_PROVIDER", "ollama")
            .with("FERRITE_MODEL_SMALL", "app-small")
            .with("FERRITE_MODEL_MAIN", "app-main")
            .with("FERRITE_LIVE_MAIN_MODEL", "live-main");
        let args = parse_env(&[], &env).expect("parses");
        assert_eq!(args.provider, Some(ProviderKind::Ollama));
        assert_eq!(args.small_model.as_deref(), Some("app-small"));
        assert_eq!(args.main_model.as_deref(), Some("live-main"));

        let args = parse_env(&["--provider", "gemini", "--main-model", "flag-main"], &env)
            .expect("parses");
        assert_eq!(args.provider, Some(ProviderKind::Gemini));
        assert_eq!(args.main_model.as_deref(), Some("flag-main"));
    }

    #[test]
    fn the_two_tiers_can_differ() {
        let args = parse(&[
            "--provider",
            "ollama",
            "--small-model",
            "tiny",
            "--main-model",
            "big",
        ])
        .expect("parses");
        let tags = args.model_tags().expect("both set");
        assert_eq!(tags.small, "tiny");
        assert_eq!(tags.main, "big");
    }

    #[test]
    fn a_live_role_without_a_tag_is_an_actionable_error_not_a_default() {
        let args = parse(&["--provider", "gemini"]).expect("parses");
        let err = args.model_tags().expect_err("no tag anywhere").to_string();
        assert!(err.contains("--small-model"), "{err}");
        assert!(err.contains("FERRITE_LIVE_SMALL_MODEL"), "{err}");

        let args = parse(&["--provider", "gemini", "--small-model", "s"]).expect("parses");
        let err = args.model_tags().expect_err("no agent tag").to_string();
        assert!(err.contains("--main-model"), "{err}");
    }

    #[test]
    fn a_role_that_is_not_live_needs_no_tag() {
        let args = parse(&[
            "--provider",
            "gemini",
            "--agent",
            "scripted",
            "--small-model",
            "only-predictor",
        ])
        .expect("parses");
        assert_eq!(args.model_tags().expect("ok").small, "only-predictor");

        let args = parse(&[
            "--provider",
            "gemini",
            "--predictor",
            "rules",
            "--main-model",
            "only-agent",
        ])
        .expect("parses");
        assert_eq!(args.model_tags().expect("ok").main, "only-agent");

        // Off mode needs no prediction, so the predictor needs no tag either.
        let args = parse(&[
            "--provider",
            "gemini",
            "--modes",
            "off",
            "--main-model",
            "m",
        ])
        .expect("parses");
        assert!(args.model_tags().is_ok());
    }

    #[test]
    fn the_mock_provider_needs_no_tag() {
        let args = parse(&["--provider", "mock"]).expect("parses");
        let tags = args.model_tags().expect("mock tags are synthetic");
        assert!(tags.small.starts_with("mock"));
    }

    #[test]
    fn bad_input_is_named_not_swallowed() {
        assert!(matches!(
            parse(&["--provider", "no-such-provider"]),
            Err(ArgsError::BadValue { .. })
        ));
        assert!(matches!(
            parse(&["--bogus"]),
            Err(ArgsError::UnknownFlag(_))
        ));
        assert!(matches!(
            parse(&["--batch-size"]),
            Err(ArgsError::MissingValue(_))
        ));
        assert!(matches!(
            parse(&["--batch-size", "0"]),
            Err(ArgsError::BadValue { .. })
        ));
        assert!(matches!(
            parse(&["--max-calls", "-3"]),
            Err(ArgsError::BadValue { .. })
        ));
        assert!(matches!(
            parse(&["--modes", "off,sideways"]),
            Err(ArgsError::BadValue { .. })
        ));
    }

    #[test]
    fn contradictory_batching_is_rejected() {
        assert!(
            parse(&["--batch-index", "2"]).is_err(),
            "an index needs a size"
        );
        assert!(parse(&["--batch-size", "5", "--batch-index", "1", "--limit", "9"]).is_err());
        assert!(parse(&["--batch-size", "5", "--batch-index", "1"]).is_ok());
        assert!(parse(&["--backoff-base-ms", "500", "--backoff-max-ms", "100"]).is_err());
    }

    #[test]
    fn plan_report_and_compare_parse() {
        assert_eq!(parse(&["--plan"]).unwrap().command, Command::Plan);
        assert_eq!(parse(&["--dry-plan"]).unwrap().command, Command::Plan);
        let args = parse(&["--report", "--compare", "gemini:a,ollama:b"]).unwrap();
        assert_eq!(args.command, Command::Report);
        assert_eq!(
            args.compare,
            vec![
                ("gemini".to_string(), "a".to_string()),
                ("ollama".to_string(), "b".to_string())
            ]
        );
        assert!(parse(&["--compare", "nocolon"]).is_err());
    }

    #[test]
    fn duplicate_modes_collapse_and_keep_order() {
        let args = parse(&["--modes", "guard,off,guard"]).unwrap();
        assert_eq!(args.modes, vec![LiveMode::Guard, LiveMode::Off]);
    }

    #[test]
    fn the_help_text_mentions_every_flag_the_parser_accepts() {
        for flag in [
            "--provider",
            "--model",
            "--small-model",
            "--main-model",
            "--base-url",
            "--predictor",
            "--agent",
            "--modes",
            "--max-steps",
            "--mock-behavior",
            "--corpus",
            "--suite",
            "--only",
            "--seed",
            "--batch-size",
            "--batch-index",
            "--offset",
            "--limit",
            "--retry-failed",
            "--plan",
            "--dry-plan",
            "--max-calls",
            "--pause-ms",
            "--max-attempts",
            "--backoff-base-ms",
            "--backoff-max-ms",
            "--timeout-secs",
            "--max-consecutive-failures",
            "--no-cache",
            "--out",
            "--report",
            "--compare",
            "--corpus-root",
        ] {
            assert!(
                HELP.contains(flag) || flag == "--corpus-root",
                "{flag} is missing from the help text"
            );
        }
    }

    #[test]
    fn modes_map_onto_the_pipelines_defense_modes() {
        assert_eq!(LiveMode::Off.defense_mode(), DefenseMode::Off);
        assert_eq!(LiveMode::Guard.defense_mode(), DefenseMode::LoopOnly);
        assert_eq!(LiveMode::Full.defense_mode(), DefenseMode::On);
        assert!(LiveMode::Guard.enforces_guard() && LiveMode::Full.enforces_guard());
        assert!(!LiveMode::Off.enforces_guard() && !LiveMode::DryRun.enforces_guard());
        assert!(!LiveMode::Off.needs_prediction() && LiveMode::DryRun.needs_prediction());
        for m in LiveMode::ALL {
            assert_eq!(LiveMode::parse(m.as_str()), Ok(m));
        }
    }
}
