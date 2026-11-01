//! The per-case companion to the report: which runs failed, and why.
//!
//! [`super::report`] says *how many* benign tasks had an action refused; this says
//! *which*, with what was refused, what the predictor allowed against what the task
//! ideally needs, and a cause. It reads only stored [`LiveRecord`]s, so it needs no
//! model and no network.
//!
//! Per provider/model group and per mode it lists:
//!
//! - benign cases with an action refused (a false positive of the guard);
//! - attacks that executed under a defended mode;
//! - attacks that were attempted and not blocked (a superset of the previous
//!   list, with an `executed` column, so a run the guard let through without
//!   running it is not hidden);
//! - runs whose prediction was empty or degraded to the rule layer;
//! - runs that errored (excluded from every rate in the report).
//!
//! # Causes
//!
//! A refused benign action is given exactly one cause, tested in this order:
//! `unusable model answer` (the predictor's answer was discarded), `empty
//! fingerprint` (nothing predicted, so everything is gated), `missing capability`
//! (a capability the task ideally needs was not predicted), else `out-of-scope
//! origin or primitive` (the capabilities were all predicted, so the refusal came
//! from where or how the action ran). `no prediction stored` is reported as such
//! rather than guessed at.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::config::LiveMode;
use super::record::{LiveRecord, Verdict};
use super::report::{matches, modes_present, GroupKey, ReportOptions};
use super::store::Results;

/// The finished failure report.
#[derive(Debug, Clone)]
pub struct FailureReport {
    /// The Markdown document.
    pub markdown: String,
    /// One CSV row per listed failure.
    pub csv: String,
    /// How many failures are listed in total.
    pub rows: usize,
}

const CSV_HEADER: &str = "group,mode,category,case_id,suite,primitive,origin,cause,stop_reason,must_use,may_use,ideal_capabilities,detail";

/// Why a benign action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Cause {
    UnusableModelAnswer,
    EmptyFingerprint,
    MissingCapability,
    OutOfScope,
    NoPrediction,
}

impl Cause {
    fn label(self) -> &'static str {
        match self {
            Self::UnusableModelAnswer => "unusable model answer",
            Self::EmptyFingerprint => "empty fingerprint",
            Self::MissingCapability => "missing capability",
            Self::OutOfScope => "out-of-scope origin or primitive",
            Self::NoPrediction => "no prediction stored",
        }
    }
}

fn cause_of(r: &LiveRecord) -> Cause {
    let Some(p) = &r.prediction else {
        return Cause::NoPrediction;
    };
    if p.degraded {
        return Cause::UnusableModelAnswer;
    }
    if p.must_use.is_empty() && p.may_use.is_empty() {
        return Cause::EmptyFingerprint;
    }
    let missing = r.ideal_capabilities.as_ref().is_some_and(|ideal| {
        ideal
            .iter()
            .any(|c| !p.must_use.contains(c) && !p.may_use.contains(c))
    });
    if missing {
        Cause::MissingCapability
    } else {
        Cause::OutOfScope
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    BenignRefused,
    AttackExecuted,
    AttackNotBlocked,
    PredictionUnusable,
    Errored,
}

impl Category {
    const ALL: [Self; 5] = [
        Self::BenignRefused,
        Self::AttackExecuted,
        Self::AttackNotBlocked,
        Self::PredictionUnusable,
        Self::Errored,
    ];

    fn slug(self) -> &'static str {
        match self {
            Self::BenignRefused => "benign_refused",
            Self::AttackExecuted => "attack_executed",
            Self::AttackNotBlocked => "attack_attempted_not_blocked",
            Self::PredictionUnusable => "empty_or_degraded_prediction",
            Self::Errored => "errored",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::BenignRefused => "Benign cases with an action refused",
            Self::AttackExecuted => "Attacks that executed under the defense",
            Self::AttackNotBlocked => {
                "Attacks attempted but not blocked (includes the executed ones)"
            }
            Self::PredictionUnusable => "Empty or degraded predictions",
            Self::Errored => "Errored runs (excluded from every rate)",
        }
    }
}

struct Row {
    category: Category,
    case_id: String,
    suite: String,
    primitive: String,
    origin: String,
    cause: String,
    stop_reason: String,
    must: String,
    may: String,
    ideal: String,
    detail: String,
}

fn join(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join("; ")
    }
}

fn row(
    r: &LiveRecord,
    category: Category,
    primitive: String,
    origin: String,
    cause: String,
) -> Row {
    let (must, may) = r.prediction.as_ref().map_or_else(
        || ("-".to_string(), "-".to_string()),
        |p| (join(&p.must_use), join(&p.may_use)),
    );
    Row {
        category,
        case_id: r.case_id.clone(),
        suite: r.suite.clone(),
        primitive,
        origin,
        cause,
        stop_reason: r.stop_reason.clone(),
        must,
        may,
        ideal: r
            .ideal_capabilities
            .as_deref()
            .map_or_else(|| "-".to_string(), join),
        detail: r
            .error
            .as_ref()
            .map(|e| format!("{}: {}", e.class, e.message))
            .unwrap_or_default(),
    }
}

/// The primitives and origins of the actions picked by `keep`, `-` when none.
fn actions_where(
    r: &LiveRecord,
    keep: impl Fn(&super::record::ActionRecord) -> bool,
) -> (String, String) {
    let picked: Vec<_> = r.actions.iter().filter(|a| keep(a)).collect();
    let field = |f: fn(&super::record::ActionRecord) -> &Option<String>| {
        let mut v: Vec<String> = picked.iter().filter_map(|a| f(a).clone()).collect();
        v.dedup();
        join(&v)
    };
    (field(|a| &a.primitive), field(|a| &a.origin))
}

fn rows_for(r: &LiveRecord) -> Vec<Row> {
    let mut out = Vec::new();
    if r.error.is_some() {
        out.push(row(
            r,
            Category::Errored,
            "-".into(),
            "-".into(),
            "run failed".into(),
        ));
        return out;
    }
    if r.kind == "benign" && r.outcome.benign_blocked {
        let cause = cause_of(r).label().to_string();
        let blocked: Vec<_> = r
            .actions
            .iter()
            .filter(|a| a.verdict == Verdict::Blocked)
            .collect();
        if blocked.is_empty() {
            out.push(row(
                r,
                Category::BenignRefused,
                "-".into(),
                "-".into(),
                cause.clone(),
            ));
        }
        for a in blocked {
            out.push(row(
                r,
                Category::BenignRefused,
                a.primitive.clone().unwrap_or_else(|| "-".into()),
                a.origin.clone().unwrap_or_else(|| "-".into()),
                cause.clone(),
            ));
        }
    }
    if r.kind == "attack" && r.mode != LiveMode::Off {
        let (prim, origin) = actions_where(r, |a| a.attack_hit.is_some());
        if r.outcome.attack_executed {
            out.push(row(
                r,
                Category::AttackExecuted,
                prim.clone(),
                origin.clone(),
                "-".into(),
            ));
        }
        if r.outcome.attack_attempted && !r.outcome.attack_blocked {
            out.push(row(r, Category::AttackNotBlocked, prim, origin, "-".into()));
        }
    }
    if let Some(p) = &r.prediction {
        let empty = p.must_use.is_empty() && p.may_use.is_empty();
        if p.degraded || empty {
            let why = match (&p.degraded_reason, p.degraded, empty) {
                (Some(reason), true, _) => format!("degraded: {reason}"),
                (None, true, _) => "degraded".to_string(),
                _ => "empty fingerprint".to_string(),
            };
            let mut x = row(r, Category::PredictionUnusable, "-".into(), "-".into(), why);
            x.detail = String::new();
            out.push(x);
        }
    }
    out
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn section(md: &mut String, category: Category, rows: &[&Row]) {
    let _ = writeln!(md, "#### {} ({})\n", category.title(), rows.len());
    if rows.is_empty() {
        md.push_str("None.\n\n");
        return;
    }
    if category == Category::BenignRefused {
        let mut by_cause: BTreeMap<&str, usize> = BTreeMap::new();
        for r in rows {
            *by_cause.entry(r.cause.as_str()).or_default() += 1;
        }
        let parts: Vec<String> = by_cause.iter().map(|(c, n)| format!("{c}: {n}")).collect();
        let _ = writeln!(md, "Refused actions by cause: {}.\n", parts.join(", "));
    }
    md.push_str(
        "| case | suite | primitive | origin | cause | stop | must_use | may_use | ideal | detail |\n|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for r in rows {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&r.case_id),
            cell(&r.suite),
            cell(&r.primitive),
            cell(&r.origin),
            cell(&r.cause),
            cell(&r.stop_reason),
            cell(&r.must),
            cell(&r.may),
            cell(&r.ideal),
            cell(&r.detail)
        );
    }
    md.push('\n');
}

/// Builds the failure report from stored results.
#[must_use]
pub fn generate_failures(results: &Results, options: &ReportOptions) -> FailureReport {
    let mut by_group: BTreeMap<GroupKey, Vec<&LiveRecord>> = BTreeMap::new();
    for r in results.by_key.values() {
        let key = GroupKey::of(r);
        if matches(&key, &options.compare) {
            by_group.entry(key).or_default().push(r);
        }
    }
    let mut md = String::from("# Ferrite live evaluation: failing cases\n\n");
    let mut csv = String::from(CSV_HEADER);
    csv.push('\n');
    let mut total = 0;
    if by_group.is_empty() {
        md.push_str("No stored results match. Run `live_eval` first.\n");
        return FailureReport {
            markdown: md,
            csv,
            rows: 0,
        };
    }
    md.push_str("Each list is the individual cases behind a count in REPORT.md. `must_use` / `may_use` are what the predictor allowed; `ideal` is what the task needs. Read the cause rules in the `failures` module doc before grouping by cause.\n\n");
    for (key, records) in &by_group {
        let group = key.label();
        let _ = writeln!(md, "## {group}\n");
        for mode in modes_present(records) {
            let mut rows: Vec<Row> = records
                .iter()
                .filter(|r| r.mode == mode)
                .flat_map(|r| rows_for(r))
                .collect();
            rows.sort_by(|a, b| (a.category, &a.case_id).cmp(&(b.category, &b.case_id)));
            let _ = writeln!(md, "### mode `{}`\n", mode.as_str());
            for category in Category::ALL {
                let in_cat: Vec<&Row> = rows.iter().filter(|r| r.category == category).collect();
                section(&mut md, category, &in_cat);
            }
            for r in &rows {
                total += 1;
                let fields = [
                    group.as_str(),
                    mode.as_str(),
                    r.category.slug(),
                    &r.case_id,
                    &r.suite,
                    &r.primitive,
                    &r.origin,
                    &r.cause,
                    &r.stop_reason,
                    &r.must,
                    &r.may,
                    &r.ideal,
                    &r.detail,
                ];
                let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
                csv.push_str(&line.join(","));
                csv.push('\n');
            }
        }
    }
    FailureReport {
        markdown: md,
        csv,
        rows: total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::config::PredictorKind;
    use crate::live::record::{ActionRecord, Outcome, PredictionRecord, RecordError};
    use crate::live::testing::record;

    fn add(results: &mut Results, id: &str, mode: LiveMode, f: impl FnOnce(&mut LiveRecord)) {
        let key = format!("{id}|{}", mode.as_str());
        let mut r = record(&key, "2026-01-01T00:00:00Z");
        r.case_id = id.to_string();
        r.mode = mode;
        f(&mut r);
        results.by_key.insert(key, r);
    }

    fn pred(must: &[&str], may: &[&str], degraded: bool) -> Option<PredictionRecord> {
        Some(PredictionRecord {
            must_use: must.iter().map(|s| (*s).to_string()).collect(),
            may_use: may.iter().map(|s| (*s).to_string()).collect(),
            degraded,
            degraded_reason: degraded.then(|| "malformed".to_string()),
            predictor: PredictorKind::Llm,
        })
    }

    fn blocked_action(prim: &str, origin: &str) -> ActionRecord {
        ActionRecord {
            step: 0,
            action: "{}".into(),
            primitive: Some(prim.into()),
            origin: Some(origin.into()),
            verdict: Verdict::Blocked,
            executed: false,
            attack_hit: None,
        }
    }

    fn benign_refused(
        prediction: Option<PredictionRecord>,
        ideal: &[&str],
    ) -> impl FnOnce(&mut LiveRecord) {
        let ideal: Vec<String> = ideal.iter().map(|s| (*s).to_string()).collect();
        move |r| {
            r.kind = "benign".into();
            r.prediction = prediction;
            r.ideal_capabilities = Some(ideal);
            r.actions = vec![blocked_action("form.fill", "https://a.example")];
            r.outcome = Outcome {
                benign_blocked: true,
                ..Outcome::default()
            };
            r.stop_reason = "blocked".into();
        }
    }

    fn report_for(results: &Results) -> FailureReport {
        generate_failures(results, &ReportOptions::default())
    }

    #[test]
    fn each_refused_benign_case_gets_the_first_matching_cause() {
        let mut res = Results::default();
        add(
            &mut res,
            "degraded",
            LiveMode::Guard,
            benign_refused(pred(&["web.read"], &[], true), &["web.read"]),
        );
        add(
            &mut res,
            "empty",
            LiveMode::Guard,
            benign_refused(pred(&[], &[], false), &["web.read"]),
        );
        add(
            &mut res,
            "missing",
            LiveMode::Guard,
            benign_refused(
                pred(&["web.read"], &[], false),
                &["web.read", "web.interact"],
            ),
        );
        add(
            &mut res,
            "scope",
            LiveMode::Guard,
            benign_refused(
                pred(&["web.read", "web.interact"], &[], false),
                &["web.interact"],
            ),
        );
        add(
            &mut res,
            "none",
            LiveMode::Guard,
            benign_refused(None, &["web.read"]),
        );
        let rep = report_for(&res);
        for (id, cause) in [
            ("degraded", "unusable model answer"),
            ("empty", "empty fingerprint"),
            ("missing", "missing capability"),
            ("scope", "out-of-scope origin or primitive"),
            ("none", "no prediction stored"),
        ] {
            let line = rep
                .csv
                .lines()
                .find(|l| l.contains(&format!(",benign_refused,{id},")))
                .unwrap_or_else(|| panic!("{id}: {}", rep.csv));
            assert!(line.contains(cause), "{id}: {line}");
        }
        assert!(rep.markdown.contains("Refused actions by cause:"));
    }

    #[test]
    fn a_refusal_names_the_refused_primitive_and_origin() {
        let mut res = Results::default();
        add(
            &mut res,
            "b1",
            LiveMode::Guard,
            benign_refused(pred(&["web.read"], &[], false), &["web.read"]),
        );
        let rep = report_for(&res);
        assert!(
            rep.markdown
                .contains("| b1 | s | form.fill | https://a.example |"),
            "{}",
            rep.markdown
        );
        assert_eq!(rep.rows, 1);
    }

    #[test]
    fn attacks_are_listed_only_under_a_defended_mode() {
        let mut res = Results::default();
        for mode in [LiveMode::Off, LiveMode::Guard] {
            add(&mut res, "atk", mode, |r| {
                r.outcome = Outcome {
                    measurable: true,
                    attack_attempted: true,
                    attack_executed: true,
                    ..Outcome::default()
                };
                r.actions = vec![ActionRecord {
                    step: 0,
                    action: "{}".into(),
                    primitive: Some("net.send".into()),
                    origin: Some("https://evil.example".into()),
                    verdict: Verdict::Expected,
                    executed: true,
                    attack_hit: Some("effect".into()),
                }];
            });
        }
        let rep = report_for(&res);
        let guard_rows: Vec<_> = rep
            .csv
            .lines()
            .filter(|l| l.contains(",attack_executed,"))
            .collect();
        assert_eq!(guard_rows.len(), 1, "{}", rep.csv);
        assert!(guard_rows[0].contains(",guard,"));
        assert!(
            guard_rows[0].contains("net.send") && guard_rows[0].contains("https://evil.example")
        );
        assert_eq!(
            rep.csv
                .lines()
                .filter(|l| l.contains(",attack_attempted_not_blocked,"))
                .count(),
            1
        );
    }

    #[test]
    fn a_blocked_attack_is_not_a_failure() {
        let mut res = Results::default();
        add(&mut res, "atk", LiveMode::Guard, |r| {
            r.outcome = Outcome {
                measurable: true,
                attack_attempted: true,
                attack_blocked: true,
                ..Outcome::default()
            };
        });
        let rep = report_for(&res);
        assert_eq!(rep.rows, 0, "{}", rep.csv);
        assert!(rep.markdown.contains("None."));
    }

    #[test]
    fn empty_and_degraded_predictions_and_errored_runs_are_listed() {
        let mut res = Results::default();
        add(&mut res, "e", LiveMode::Guard, |r| {
            r.kind = "benign".into();
            r.prediction = pred(&[], &[], false);
        });
        add(&mut res, "d", LiveMode::Guard, |r| {
            r.kind = "benign".into();
            r.prediction = pred(&["web.read"], &[], true);
        });
        add(&mut res, "x", LiveMode::Guard, |r| {
            r.error = Some(RecordError {
                class: "timeout".into(),
                message: "slow, \"very\"".into(),
                retryable: true,
            });
        });
        let rep = report_for(&res);
        assert!(
            rep.csv.contains(",empty_or_degraded_prediction,e,"),
            "{}",
            rep.csv
        );
        assert!(rep.csv.contains("degraded: malformed"));
        assert!(rep.csv.contains(",errored,x,"));
        assert!(
            rep.csv.contains("\"timeout: slow, \"\"very\"\"\""),
            "csv fields are quoted: {}",
            rep.csv
        );
    }

    #[test]
    fn a_clean_run_lists_nothing_and_an_empty_store_says_so() {
        let mut res = Results::default();
        add(&mut res, "ok", LiveMode::Guard, |r| {
            r.kind = "benign".into();
            r.prediction = pred(&["web.read"], &[], false);
        });
        assert_eq!(report_for(&res).rows, 0);
        let none = report_for(&Results::default());
        assert!(none.markdown.contains("No stored results"));
        assert_eq!(none.csv.lines().count(), 1);
    }

    #[test]
    fn pipes_in_a_cell_cannot_break_the_table() {
        assert_eq!(cell("a|b\nc"), "a\\|b c");
    }
}
