pub mod comparator;
pub mod dataset;
pub mod dry_run;
pub mod fingerprint;
pub mod sanitizer;
pub mod tool_decision;
pub mod twin;

/// A submitted task, decoupled from `ferrite_agent::AgentTask` (T-221: this
/// crate must not depend on `ferrite-agent` — see `docs/TO-DO.md`). Every
/// field mirrors `AgentTask`'s shape, so a caller that already builds one
/// (`ferrite-eval`'s harness, `ferrite-ui`) constructs this alongside it with
/// a one-line field copy rather than a redesign — see `docs/handoffs/b01.md`
/// for the exact migration shape handed to B2/B3.
///
/// This is the vocabulary [`tool_decision::ToolDecisionEngine::prepare_task`]
/// and [`dry_run::DryRunOrchestrator::run`] both take, so a live task's
/// prompt/context reaches the sanitizer, the fingerprint predictor, and the
/// dry run through one shared, agent-crate-agnostic shape.
#[derive(Debug, Clone)]
pub struct IpiTask {
    pub session_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub prompt: String,
    pub context_url: Option<String>,
}

impl IpiTask {
    #[must_use]
    pub fn new(prompt: impl Into<String>, context_url: Option<String>) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            prompt: prompt.into(),
            context_url,
        }
    }
}
