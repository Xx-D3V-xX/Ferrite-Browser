//! The dry-run event log: [`ToolEvent`], [`DryRunRecord`], and the
//! sanitizer-finding bookkeeping recorded alongside it.
//!
//! # Ordering
//!
//! `docs/REBUILD_DIRECTIVE.md` §6/A6 asks for an *ordered*
//! `Vec<ToolEvent { primitive, origin, seq }>`. Here, ordering is the
//! `Vec<ToolEvent>`'s own push order (guaranteed by
//! [`DryRunRecord::record_tool`] appending, never inserting/reordering), and
//! [`DryRunRecord::events_with_seq`] is a derived view that pairs each event
//! with its 0-based position — a computed `seq`, not a stored field.
//!
//! # `primitive: ferrite_core::Primitive`, direct — the B1 fix
//!
//! Prior to this charter, this struct kept `tool: ToolId` (a stringly wire
//! id matching `ferrite_agent::BrowserTool::tool_id()`) plus a best-effort,
//! fallible `ToolEvent::primitive()` bridge to `ferrite_core::Primitive`,
//! because the dry-run executor was itself typed against
//! `ferrite_agent::{BrowserTool, ToolExecutor}` (see `docs/TO-DO.md` T-221).
//! That bridge returned `None` for exactly one case (`"download.file"` vs.
//! `Primitive::Download`'s `"download"` wire string, T-216) and needed a
//! second, comparator-local patch table to close.
//!
//! B1 replaced the dry-run's executor with [`super::engine::DryRunEngine`],
//! which implements [`ferrite_engine::BrowserEngine`] directly — every
//! action is already tagged with its real `ferrite_core::Primitive` via
//! [`ferrite_engine::Call::primitive`], with no second, independently
//! authored string vocabulary to drift out of sync (see that trait's own
//! module docs). So `ToolEvent` now stores the `Primitive` the engine
//! actually recorded, directly: no bridge, no `Option`, no per-case patch
//! table. `ferrite_core::Primitive::JsExecute` is representable here (the
//! *observed* vocabulary always could — see `ferrite_core::taxonomy`'s
//! module docs) even though no *expected*-side type can ever name it; the
//! comparator is what enforces that asymmetry, not this struct.
//!
//! `crate::dataset` (A11's schema) and `ferrite-eval`/`ferrite-ui` construct
//! `ToolEvent` literals directly too — `docs/handoffs/b01.md` documents the
//! exact mechanical fix each of those calls needed (a literal `ToolId::new("dom.read")`
//! becomes `Primitive::DomRead`, etc. — a 1:1 wire-string correspondence, not
//! a redesign).

use std::collections::HashSet;

use ferrite_core::Primitive;

use crate::sanitizer::Finding;
use crate::tool_decision::ToolId;

/// Where a recorded finding came from, so the harness can match it against a
/// case's declared expected finding (pattern + optional location).
#[derive(Debug, Clone, PartialEq)]
pub enum FindingCarrier {
    /// T1a web content. `channel` is "visible_text" | "comment" | "script".
    WebContent { channel: String },
    /// T1b tool output. `json_path` is the LocatedFinding path (e.g. "results[0].description", "$", "error").
    ToolOutput { json_path: String },
}

/// A sanitizer finding recorded during the dry run, with the context needed
/// to adjudicate it against a case's declared expected finding (W2).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedFinding {
    pub finding: Finding,
    pub carrier: FindingCarrier,
    /// The tool whose result carried this finding, as the pre-existing
    /// `ToolId` wire string — kept as `ToolId` (not retyped to `Primitive`)
    /// because this field is purely informational/audit (it plays no part
    /// in `compare()`'s security-relevant classification, unlike
    /// `ToolEvent::primitive`) and `ferrite-eval::adjudication` constructs it
    /// directly; retyping it would not be a minimal mechanical fix for a
    /// field with no comparator role. Built from `ToolId::new(primitive.as_str())`
    /// at the point of recording — see `super::engine::DryRunEngine`.
    pub tool: ToolId,
    /// The origin the agent was on when this result was produced.
    pub origin: Option<String>,
}

/// One recorded tool invocation, bound to the origin the agent was on when
/// it happened. Ordering is the containing `Vec<ToolEvent>`'s push order —
/// see [`DryRunRecord::events_with_seq`] for an explicit sequence number.
///
/// `origin` is `None` when no origin context has been established yet
/// (e.g. the very first call before any navigation/context_url).
///
/// `primitive` is the real, direct `ferrite_core::Primitive` the engine
/// recorded — see the [module docs](self) for why this is no longer a
/// stringly `ToolId` needing a bridge.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolEvent {
    pub primitive: Primitive,
    pub origin: Option<String>,
}

/// Accumulates everything the agent did during the dry run.
#[derive(Debug, Default, Clone)]
pub struct DryRunRecord {
    pub session_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub tool_events: Vec<ToolEvent>,
    pub origins_touched: HashSet<String>,
    pub data_fields_accessed: HashSet<String>,
    pub network_attempts: Vec<String>,
    pub completed: bool,
    pub sanitizer_findings: Vec<RecordedFinding>,
}

impl DryRunRecord {
    pub fn new(session_id: uuid::Uuid, task_id: uuid::Uuid) -> Self {
        Self {
            session_id,
            task_id,
            ..Default::default()
        }
    }

    /// Derives the set of distinct primitives called from the ordered event
    /// log. Convenience for callers that only need set membership, not
    /// ordering/origin.
    pub fn tools_called(&self) -> HashSet<Primitive> {
        self.tool_events.iter().map(|e| e.primitive).collect()
    }

    /// Records one tool invocation, appended to the end of `tool_events` —
    /// the sole source of call ordering (see module docs).
    pub fn record_tool(&mut self, primitive: Primitive, origin: Option<String>) {
        self.tool_events.push(ToolEvent { primitive, origin });
    }

    /// Pairs each recorded event with its 0-based position in call order —
    /// the explicit `seq` the directive's `ToolEvent { primitive, origin,
    /// seq }` shape asks for, derived rather than stored (see module docs).
    pub fn events_with_seq(&self) -> impl Iterator<Item = (u64, &ToolEvent)> {
        self.tool_events
            .iter()
            .enumerate()
            .map(|(i, e)| (i as u64, e))
    }

    pub fn record_finding(&mut self, finding: RecordedFinding) {
        self.sanitizer_findings.push(finding);
    }

    pub fn record_network_attempt(&mut self, url: String) {
        let origin = extract_origin(&url);
        self.origins_touched.insert(origin);
        self.network_attempts.push(url);
    }
}

/// Normalizes a URL to its origin (`scheme://host`), falling back to the
/// literal input when it doesn't parse as `scheme://host` — this is the
/// T-211 handling for opaque-scheme URLs (`about:blank`, `data:`, `blob:`):
/// they are recorded as their literal string rather than a normalized
/// origin, since `ferrite_core::Origin::parse` structurally rejects every
/// scheme but `http`/`https` (by design — an opaque origin is one no
/// `OriginScope` could ever admit). A literal opaque string will likewise
/// fail `Origin::parse`, which is the *correct* downstream effect: the
/// comparator can treat "this event's origin string does not parse as an
/// `Origin`" as itself the signal that no scope can ever admit it — the
/// same always-a-deviation treatment `js.execute` gets, achieved by type
/// mismatch rather than a dedicated `Origin::Opaque` variant. See
/// `docs/TO-DO.md` T-211 and `docs/handoffs/a06.md` for the full reasoning
/// and why a real `Origin::Opaque` variant (in `ferrite-core`, out of this
/// session's file scope) is still the more principled long-term fix.
pub(crate) fn extract_origin(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_string();
            // A non-default port is part of the origin (`Origin::parse` keeps
            // it); dropping it made a local server on `:8099` look like a
            // different origin from the one the task named.
            Some(match u.port() {
                Some(port) => format!("{}://{}:{}", u.scheme(), host, port),
                None => format!("{}://{}", u.scheme(), host),
            })
        })
        .unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_default_port_is_part_of_the_origin_and_a_default_port_is_not() {
        assert_eq!(
            extract_origin("http://127.0.0.1:8099/a?b=1"),
            "http://127.0.0.1:8099"
        );
        assert_eq!(
            extract_origin("https://Example.com:443/x"),
            "https://example.com"
        );
        assert_eq!(
            extract_origin("https://example.com/x"),
            "https://example.com"
        );
    }

    #[test]
    fn seq_is_monotonic_and_matches_call_order() {
        let mut rec = DryRunRecord::new(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        rec.record_tool(Primitive::Navigate, Some("https://a.example".into()));
        rec.record_tool(Primitive::DomRead, Some("https://a.example".into()));
        rec.record_tool(Primitive::DomRead, Some("https://a.example".into()));

        let seqs: Vec<u64> = rec.events_with_seq().map(|(seq, _)| seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
        assert_eq!(rec.tool_events[0].primitive, Primitive::Navigate);
        assert_eq!(rec.tool_events[1].primitive, Primitive::DomRead);

        // seq tracks position even after the record is cloned/filtered
        // elsewhere — it is a view over whatever Vec you call it on, not
        // state that can drift from the Vec it describes.
        let (first_seq, first_event) = rec.events_with_seq().next().unwrap();
        assert_eq!(first_seq, 0);
        assert_eq!(first_event.primitive, Primitive::Navigate);
    }

    #[test]
    fn extract_origin_normalizes_scheme_and_host_only() {
        assert_eq!(
            extract_origin("https://Example.com:443/inbox?x=1"),
            "https://example.com"
        );
    }

    #[test]
    fn extract_origin_falls_back_to_literal_for_opaque_schemes() {
        assert_eq!(extract_origin("about:blank"), "about:blank");
        assert!(extract_origin("data:text/html,hi").starts_with("data:"));
    }

    /// B1: `ToolEvent.primitive` is a real, direct `ferrite_core::Primitive`
    /// — no bridge, no `Option`, no per-case patch table. This is the
    /// concrete proof the T-216 mismatch (`"download.file"` vs.
    /// `Primitive::Download`'s `"download"` wire string) cannot recur here:
    /// there is no wire-string comparison in the recording path at all for
    /// `compare()` to get wrong.
    #[test]
    fn tool_event_primitive_is_a_real_direct_value_download_included() {
        let event = ToolEvent {
            primitive: Primitive::Download,
            origin: Some("https://files.example".to_string()),
        };
        assert_eq!(event.primitive, Primitive::Download);

        // js.execute is representable on the *observed* side (this struct),
        // even though no *expected*-side type can ever name it — that
        // asymmetry belongs to the comparator, not to whether this field can
        // hold the value at all.
        let js = ToolEvent {
            primitive: Primitive::JsExecute,
            origin: None,
        };
        assert_eq!(js.primitive, Primitive::JsExecute);
        assert!(
            js.primitive.as_scopable().is_none(),
            "js.execute must still have no ScopablePrimitive counterpart"
        );
    }
}
