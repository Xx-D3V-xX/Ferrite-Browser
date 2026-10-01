//! `just eval`'s report generator: a markdown table + CSV over a completed
//! corpus run, plus the mode-pair McNemar/Holm/Cohen's-h comparison table and
//! a sample of audit-chain anchors (O2's attribution objective — citing real
//! `ferrite-audit-log` entries the run actually produced).
//!
//! Deliberately pure: takes `cases`/`executions` already produced by
//! `harness::run_case` (against `MockProvider`/`ReplayProvider` or a live
//! backend — this module does not care which), and returns strings. No I/O
//! here; the `eval` binary/example writes them to disk.

use ferrite_ipi::dataset::{CaseDefinition, Corpus, ExecutionRecord, Tier};
use ferrite_ipi::tool_decision::DefenseMode;

use crate::adjudication::{self, ConsentPolicy};
use crate::metrics::{
    adr, asr, benign_false_flag_rate, compare_modes_holm_corrected, cr, discordant_pairs,
    latency_overhead, residual_r, sdr, ModeComparison, ProportionMetric,
};

/// A stratum this report is honest about not being powered for at all — the
/// charter's own example (T-227): Tier 3 AgentDojo has 3 cases. Below this
/// many cases in a cell, the report prints "insufficient data" instead of a
/// number that LOOKS precise but isn't.
const UNDERPOWERED_FLOOR: usize = 5;

pub struct EvalReport {
    pub markdown: String,
    pub csv: String,
}

const ALL_MODES: [DefenseMode; 4] = [
    DefenseMode::Off,
    DefenseMode::SanitizerOnly,
    DefenseMode::LoopOnly,
    DefenseMode::On,
];

fn fmt_pct(p: f64) -> String {
    if p.is_nan() {
        "n/a".to_string()
    } else {
        format!("{:.1}%", p * 100.0)
    }
}

fn fmt_metric(m: &ProportionMetric) -> String {
    if m.n == 0 {
        return "n/a (n=0)".to_string();
    }
    if m.is_underpowered(UNDERPOWERED_FLOOR) {
        format!(
            "{}/{} = {} [95% CI {}–{}] — **insufficient data (n<{})**",
            m.k,
            m.n,
            fmt_pct(m.ci.point),
            fmt_pct(m.ci.lower),
            fmt_pct(m.ci.upper),
            UNDERPOWERED_FLOOR
        )
    } else {
        format!(
            "{}/{} = {} [95% CI {}–{}]",
            m.k,
            m.n,
            fmt_pct(m.ci.point),
            fmt_pct(m.ci.lower),
            fmt_pct(m.ci.upper)
        )
    }
}

fn corpus_composition_table(cases: &[CaseDefinition]) -> String {
    let mut out = String::new();
    out.push_str("| Stratum | Corpus | n |\n|---|---|---|\n");
    for tier in [
        Tier::Tier1,
        Tier::Tier2,
        Tier::Tier3Teammate,
        Tier::Tier3Professor,
        Tier::Tier3AgentDojo,
    ] {
        for corpus in [Corpus::Attack, Corpus::Benign] {
            let n = cases
                .iter()
                .filter(|c| c.tier == tier && c.corpus == corpus)
                .count();
            if n > 0 {
                out.push_str(&format!("| {tier:?} | {corpus:?} | {n} |\n"));
            }
        }
    }
    out.push_str(&format!("| **Total** | | **{}** |\n", cases.len()));
    out
}

/// Per-`(tier, mode)` ASR — the charter's own explicit example of why pooled
/// numbers alone are not enough: a pooled Off-mode ASR silently averages
/// over Tier 3 AgentDojo's 3 cases and Tier 1's 12, hiding that the
/// AgentDojo cell alone is nowhere near powered for its own claim. Every
/// cell under `UNDERPOWERED_FLOOR` is flagged, not smoothed over.
fn stratified_asr_table(cases: &[CaseDefinition], executions: &[ExecutionRecord]) -> String {
    let mut out = String::new();
    out.push_str("| Tier | Mode | ASR |\n|---|---|---|\n");
    for tier in [
        Tier::Tier1,
        Tier::Tier2,
        Tier::Tier3Teammate,
        Tier::Tier3Professor,
        Tier::Tier3AgentDojo,
    ] {
        let tier_cases: Vec<CaseDefinition> = cases
            .iter()
            .filter(|c| c.tier == tier && c.corpus == Corpus::Attack)
            .cloned()
            .collect();
        if tier_cases.is_empty() {
            continue;
        }
        for mode in ALL_MODES {
            let m = asr(&tier_cases, executions, mode);
            if m.n == 0 {
                continue;
            }
            out.push_str(&format!("| {tier:?} | {mode:?} | {} |\n", fmt_metric(&m)));
        }
    }
    out
}

fn per_mode_metrics_table(cases: &[CaseDefinition], executions: &[ExecutionRecord]) -> String {
    let mut out = String::new();
    out.push_str("| Mode | ASR | CR | ADR | SDR | FGR / FSR-proxy |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for mode in ALL_MODES {
        let a = asr(cases, executions, mode);
        let c = cr(cases, executions, mode);
        // ADR/SDR are only meaningful for modes that actually run the layer
        // being measured (ADR-007: "per-layer power is read from the
        // isolated modes, never from" a mode that didn't run that layer at
        // all). Printing a "0.0%" ADR for `Off` (which never runs the loop)
        // would read as "the loop tried and failed" rather than "not
        // applicable" — an examiner-facing report must not imply that.
        let loop_active = matches!(mode, DefenseMode::LoopOnly | DefenseMode::On);
        let sanitizer_active = matches!(mode, DefenseMode::SanitizerOnly | DefenseMode::On);
        let adr_cell = if loop_active {
            fmt_metric(&adr(cases, executions, mode))
        } else {
            "n/a (loop inactive in this mode)".to_string()
        };
        let sdr_cell = if sanitizer_active {
            fmt_metric(&sdr(cases, executions, mode))
        } else {
            "n/a (sanitizer inactive in this mode)".to_string()
        };
        let f = benign_false_flag_rate(cases, executions, mode);
        out.push_str(&format!(
            "| {mode:?} | {} | {} | {} | {} | {} |\n",
            fmt_metric(&a),
            fmt_metric(&c),
            adr_cell,
            sdr_cell,
            fmt_metric(&f),
        ));
    }
    out
}

fn mode_comparison_table(comparisons: &[ModeComparison]) -> String {
    let mut out = String::new();
    out.push_str("| Mode A | Mode B | b | c | n_pairs | p (raw) | p (Holm) | Cohen's h |\n");
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    for m in comparisons {
        let note = if m.n_pairs < UNDERPOWERED_FLOOR {
            " — **insufficient paired data**"
        } else {
            ""
        };
        out.push_str(&format!(
            "| {:?} | {:?} | {} | {} | {} | {:.4} | {:.4} | {:.3}{} |\n",
            m.mode_a, m.mode_b, m.b, m.c, m.n_pairs, m.p_raw, m.p_holm, m.cohens_h, note
        ));
    }
    out
}

fn consent_sweep_table(cases: &[CaseDefinition], executions: &[ExecutionRecord]) -> String {
    // Re-derive ASR/CR under each consent policy via `reconsent` (post-hoc,
    // no dry run re-run) for the two modes consent policy actually affects.
    let by_id: std::collections::HashMap<_, _> = cases.iter().map(|c| (c.case_id, c)).collect();

    let policies = [
        (
            "RejectFlagged (upper bound on human vigilance)",
            ConsentPolicy::RejectFlagged,
        ),
        ("ApproveAll (lower bound)", ConsentPolicy::ApproveAll),
        (
            "RandomP(p=0.5, seed=42) (middle estimate)",
            ConsentPolicy::RandomP { p: 0.5, seed: 42 },
        ),
    ];

    let mut out = String::new();
    out.push_str("| Consent policy | Mode | ASR |\n|---|---|---|\n");
    for (label, policy) in policies {
        for mode in [DefenseMode::LoopOnly, DefenseMode::On] {
            let mut n = 0usize;
            let mut executed = 0usize;
            for exec in executions.iter().filter(|e| e.defense_mode == mode) {
                let Some(case) = by_id.get(&exec.case_id) else {
                    continue;
                };
                if case.corpus != Corpus::Attack {
                    continue;
                }
                let (_, final_outcome, _) = adjudication::reconsent(case, exec, policy);
                n += 1;
                if final_outcome == ferrite_ipi::dataset::FinalOutcome::Executed {
                    executed += 1;
                }
            }
            let m = ProportionMetric::compute(executed, n);
            out.push_str(&format!("| {label} | {mode:?} | {} |\n", fmt_metric(&m)));
        }
    }
    out
}

fn audit_anchors_sample(executions: &[ExecutionRecord], limit: usize) -> String {
    let mut out = String::new();
    out.push_str("| exec_id | case_id | mode | audit_log_anchor |\n|---|---|---|---|\n");
    for exec in executions.iter().take(limit) {
        out.push_str(&format!(
            "| {} | {} | {:?} | `{}` |\n",
            exec.exec_id, exec.case_id, exec.defense_mode, exec.audit_log_anchor
        ));
    }
    if executions.len() > limit {
        out.push_str(&format!(
            "\n_(+{} more execution rows, all with their own anchor — see the CSV)_\n",
            executions.len() - limit
        ));
    }
    out
}

fn csv_report(cases: &[CaseDefinition], executions: &[ExecutionRecord]) -> String {
    let by_id: std::collections::HashMap<_, _> = cases.iter().map(|c| (c.case_id, c)).collect();
    let mut out = String::new();
    out.push_str(
        "case_id,corpus,tier,defense_mode,sanitizer_caught,fingerprint_caught,consent_gated,final_outcome,residual_risk,total_ms,audit_log_anchor\n",
    );
    for exec in executions {
        let (corpus, tier) = match by_id.get(&exec.case_id) {
            Some(c) => (format!("{:?}", c.corpus), format!("{:?}", c.tier)),
            None => ("unknown".to_string(), "unknown".to_string()),
        };
        out.push_str(&format!(
            "{},{},{},{:?},{:?},{:?},{:?},{:?},{:?},{},{}\n",
            exec.case_id,
            corpus,
            tier,
            exec.defense_mode,
            exec.sanitizer_caught,
            exec.fingerprint_caught,
            exec.consent_gated,
            exec.final_outcome,
            exec.residual_risk,
            exec.timing.total_ms,
            exec.audit_log_anchor,
        ));
    }
    out
}

/// Generates the full report (markdown + CSV) over a completed corpus run.
/// Number of cases `docs/REBUILD_DIRECTIVE.md` §13.3's power calculation asks for.
const TARGET_CASES: usize = 360;

/// The honest caveat that goes with however many cases were run. Below the
/// target every interval is wide; above it, the intervals are still computed as
/// if the cases were independent draws, which a generated matrix (many cases
/// share a task, a page template or a payload) is not, and the cases are
/// self-authored by the defense's author (ADR-008), so they are tuned-against
/// data, not an independent slice.
fn scope_note(n: usize) -> String {
    if n < TARGET_CASES {
        format!(
            "**Scope note: this corpus ({n} cases) is smaller than `docs/REBUILD_DIRECTIVE.md` \
             §13.3's ~{TARGET_CASES}-case target (see `docs/TO-DO.md` T-227) — every interval below \
             is honestly wide, not a rounding artifact.**"
        )
    } else {
        format!(
            "**Scope note: {n} cases meets `docs/REBUILD_DIRECTIVE.md` §13.3's ~{TARGET_CASES}-case \
             size target, but size is not independence. Most cases are generated from a matrix \
             (`scripts/gen_redteam_corpus.py`), so they share tasks, page templates and payloads, and the \
             Wilson/McNemar figures below treat them as independent draws, which they are not; they are \
             self-authored by the defense's author and therefore tuned-against data (ADR-008). Only the 3 \
             AgentDojo cases are an external slice. Read the intervals as lower bounds on the uncertainty.**"
        )
    }
}

#[must_use]
pub fn generate_report(cases: &[CaseDefinition], executions: &[ExecutionRecord]) -> EvalReport {
    let mut md = String::new();
    md.push_str("# Ferrite evaluation report\n\n");
    md.push_str(&format!(
        "Generated over {} cases, {} executions. {}\n\n",
        cases.len(),
        executions.len(),
        scope_note(cases.len())
    ));

    md.push_str("## Corpus composition\n\n");
    md.push_str(&corpus_composition_table(cases));
    md.push('\n');

    md.push_str("## Per-mode metrics (§13.2)\n\n");
    md.push_str(&per_mode_metrics_table(cases, executions));
    md.push('\n');

    md.push_str(
        "## Per-tier ASR stratification\n\n\
         Pooled ASR above averages over every tier; a per-tier breakdown is \
         what actually shows which claims are powered at this corpus size \
         (§13.3's own worked example — Tier 3 AgentDojo has 3 cases).\n\n",
    );
    md.push_str(&stratified_asr_table(cases, executions));
    md.push('\n');

    let r = residual_r(cases);
    md.push_str(&format!(
        "**Residual R** (structural floor on ASR, corpus-level, mode-independent): {}\n\n",
        fmt_metric(&r)
    ));

    let overhead = latency_overhead(executions);
    md.push_str(&format!(
        "**Overhead ΔL** (On − Off total_ms, paired, n_pairs={}): p50 = {} ms, p95 = {} ms.\n\n",
        overhead.n_pairs, overhead.delta_p50_ms, overhead.delta_p95_ms
    ));
    md.push_str(
        "**ΔT (extra model tokens):** not computed — `docs/TO-DO.md` T-226, documented \
         limitation, see `docs/EVALUATION.md`.\n\n",
    );

    md.push_str("## Mode-pair comparisons (paired McNemar exact test, Holm-Bonferroni corrected across the whole family, Cohen's h)\n\n");
    let comparisons = compare_modes_holm_corrected(cases, executions, &ALL_MODES);
    md.push_str(&mode_comparison_table(&comparisons));
    md.push('\n');

    md.push_str("## Consent-policy sweep (T-010/D10)\n\n");
    md.push_str(&consent_sweep_table(cases, executions));
    md.push('\n');

    md.push_str("## Audit-chain anchors (sample)\n\n");
    md.push_str(&audit_anchors_sample(executions, 10));

    let csv = csv_report(cases, executions);

    // Silence an unused-import lint if a future edit removes a call site
    // without removing the import — cheap insurance, not a real dependency.
    let _ = discordant_pairs;

    EvalReport { markdown: md, csv }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ferrite_core::OriginScope;
    use ferrite_ipi::comparator::FingerprintDiff;
    use ferrite_ipi::dataset::{
        Author, CarrierVector, ConsentOutcome, FinalOutcome, GroundTruth, LayerOutcome, Model,
        ResidualRisk, RunLabel, Timing, WebContentVector,
    };
    use ferrite_ipi::dry_run::ToolEvent;
    use uuid::Uuid;

    fn exact(url: &str) -> OriginScope {
        OriginScope::exact([ferrite_core::Origin::parse(url).unwrap()]).unwrap()
    }

    fn case(corpus: Corpus, tier: Tier) -> CaseDefinition {
        CaseDefinition {
            case_id: Uuid::new_v4(),
            corpus,
            tier,
            author: Author::SelfAuthored,
            carrier_vector: CarrierVector::WebContent(WebContentVector::HtmlComment),
            attack_category: None,
            attack_techniques: vec![],
            in_scope: true,
            user_task: "task".to_string(),
            attacker_goal: None,
            expected_origins: exact("https://news.example"),
            scope_rationale: None,
            ground_truth: match corpus {
                Corpus::Attack => GroundTruth::Deviation {
                    expected_extra_primitives: Default::default(),
                    expected_out_of_scope_origins: Default::default(),
                },
                Corpus::Benign => GroundTruth::None,
            },
            taxonomy_anchor: None,
            expected_finding: None,
        }
    }

    fn exec(case_id: Uuid, mode: DefenseMode, final_outcome: FinalOutcome) -> ExecutionRecord {
        ExecutionRecord {
            exec_id: Uuid::new_v4(),
            case_id,
            timestamp: Utc::now(),
            run_label: RunLabel::R2,
            model: Model::Gemini,
            defense_mode: mode,
            expected_fingerprint: None,
            expected_realization: None,
            actual_events: vec![ToolEvent {
                primitive: ferrite_core::Primitive::DomRead,
                origin: None,
            }],
            computed_diff: FingerprintDiff::default(),
            sanitizer_caught: LayerOutcome::NotApplicable,
            fingerprint_caught: LayerOutcome::Missed,
            consent_gated: match final_outcome {
                FinalOutcome::ContainedViaConsent | FinalOutcome::BenignFalseFlag => {
                    ConsentOutcome::Gated
                }
                _ => ConsentOutcome::NotGated,
            },
            data_fields_accessed: vec![],
            network_attempts: vec![],
            final_outcome,
            residual_risk: ResidualRisk::NotApplicable,
            timing: Timing {
                total_ms: 10,
                dry_run_ms: 5,
                predict_ms: 2,
            },
            audit_log_anchor: "deadbeef".to_string(),
        }
    }

    #[test]
    fn generate_report_produces_well_formed_markdown_and_csv() {
        let c1 = case(Corpus::Attack, Tier::Tier1);
        let c2 = case(Corpus::Benign, Tier::Tier1);
        let cases = vec![c1.clone(), c2.clone()];
        let executions = vec![
            exec(c1.case_id, DefenseMode::Off, FinalOutcome::Executed),
            exec(
                c1.case_id,
                DefenseMode::On,
                FinalOutcome::ContainedViaConsent,
            ),
            exec(c2.case_id, DefenseMode::On, FinalOutcome::BenignNoFlag),
        ];

        let report = generate_report(&cases, &executions);

        // Markdown has every required section.
        for heading in [
            "# Ferrite evaluation report",
            "## Corpus composition",
            "## Per-mode metrics",
            "## Per-tier ASR stratification",
            "## Mode-pair comparisons",
            "## Consent-policy sweep",
            "## Audit-chain anchors",
            "Residual R",
            "Overhead ΔL",
            "ΔT (extra model tokens)",
        ] {
            assert!(
                report.markdown.contains(heading),
                "missing section {heading:?} in:\n{}",
                report.markdown
            );
        }

        // CSV: header + exactly one row per execution, comma-separated.
        let mut lines = report.csv.lines();
        let header = lines.next().unwrap();
        assert!(header.starts_with("case_id,corpus,tier,defense_mode"));
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), executions.len());
        for row in &rows {
            assert_eq!(row.matches(',').count(), header.matches(',').count());
        }
    }

    #[test]
    fn generate_report_flags_insufficient_data_for_a_tiny_stratum() {
        let c1 = case(Corpus::Attack, Tier::Tier3AgentDojo);
        let cases = vec![c1.clone()];
        let executions = vec![exec(c1.case_id, DefenseMode::On, FinalOutcome::Executed)];
        let report = generate_report(&cases, &executions);
        assert!(report.markdown.contains("insufficient data"));
    }

    #[test]
    fn generate_report_handles_an_empty_corpus_without_panicking() {
        let report = generate_report(&[], &[]);
        assert!(report.markdown.contains("# Ferrite evaluation report"));
        assert_eq!(report.csv.lines().count(), 1); // header only
    }
}
