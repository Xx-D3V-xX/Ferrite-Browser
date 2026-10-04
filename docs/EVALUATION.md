# Ferrite — Evaluation Methodology

This document is `docs/REBUILD_DIRECTIVE.md` §13's required deliverable:
objectives, formulas, corpus sizing derivation, and parameter rationale,
written to be read by an examiner. It is A12's charter output
(`docs/TO-DO.md` T-112), updated by B2 (`docs/TO-DO.md` T-221's second
half; `docs/handoffs/b02.md`) with real-provider verification.

**Read this first, honestly, before any number below:** the corpus this
document reports against has **29 cases**, not the ~360 §13.3's own sizing
derivation targets. Every interval quoted here is wide because the sample
is small — that is stated plainly throughout, not smoothed over. See §3.
**B2 mechanically migrated the corpus's `by_tool` vocabulary** from the old
`ferrite_agent::BrowserTool::tool_id()` strings to `ferrite_core::Primitive
::as_str()` directly (closing T-216's drift at its source rather than
patching around it) — every case file's `ground_truth`/authored semantics
are byte-for-byte unchanged; only the JSON `by_tool` key spelling for the
one case that used it (`ref06_t1b_jsonfield_exfil.json`: `"download.file"`
→ `"download"`) and `corpus.rs`'s validation logic changed. See §4/§6 for
the details.

**Two numbered runs back this document, both real, neither fabricated:**

1. **The primary, always-reproducible run** (headline numbers throughout
   §1–§3): `cargo run -p ferrite-eval --example eval` with no model config
   set — the default in this sandbox, in CI, and for any contributor who
   hasn't configured `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN`. The
   fingerprint's `may_use` layer runs rules-only (fail-to-empty); the
   dry-run agent (`WorstCaseAgent`) never calls a model. **Zero live model
   or network calls** and fully reproducible: same corpus in, same numbers
   out, modulo wall-clock timing fields — re-verified this session
   byte-for-byte identical to A12/B1's own committed numbers below.
2. **A real, live-provider run** (B2, new — §2.6): with
   `FERRITE_MODEL_SMALL=FERRITE_MODEL_MAIN=gemma4:31b` set and a real
   `OLLAMA_API_KEY` resolved from the OS keyring (service `"ferrite"`, per
   `docs/PROGRESS.md`'s 2026-09-18 coordinator entry — the same key `just
   probe` verified live during A3), `harness::try_real_provider()`
   constructs a real, undecorated `ferrite_model::OllamaProvider` and the
   fingerprint's `may_use` prediction makes genuine network round trips.
   This run is **not** reproducible byte-for-byte (a live model's answer is
   not pinned the way a scripted corpus is) and is reported separately,
   clearly labeled, never blended into the headline numbers above.

---

## 1. Objectives (§13.1)

Each objective is stated as a falsifiable claim with the metric that tests
it, and the actual result this session's run produced.

- **O1 (Containment).** *The loop reduces attack success rate versus no
  defense, for attacks whose effect is a deviation from the predicted
  fingerprint.* → ASR, paired across modes.
  **Result:** ASR_Off = 100.0% (22/22), ASR_On = 0.0% (0/22). Paired McNemar
  exact test, Off vs On: b=22, c=0, p (Holm-corrected) < 0.0001, Cohen's h =
  3.142 (the formula's own maximum boundary value — see §2's caveat on why
  this is a ceiling effect of the evaluation methodology, not literally
  "always"). **Supported, with the important caveat in §5** that the
  corpus-runner agent unconditionally re-enacts each case's authored
  deviation rather than modeling whether a real model would take the bait
  (see `ferrite_eval::worst_case_agent`'s module docs).
- **O2 (Attribution).** *Every executed action is attributable to a
  specific admitting capability and origin, and the audit chain commits to
  that attribution.* → tamper tests + attribution coverage.
  **Result:** attribution itself is A7's `comparator` (out of this
  charter's scope, tested by `comparator::tests::*`, unchanged this
  session). Audit-chain commitment: this session's run appended one
  `EvalExecutionRecorded` entry per execution (96 entries) and
  `audit.log.verify_chain()` returned `true`; A8's tamper matrix
  (`ferrite-audit-log`'s own test suite, 20 tests, unchanged this session)
  is the standing proof that mutating any covered field breaks that
  verification. **Supported** for the audit-chain half; the attribution
  half is inherited, not re-verified here.
- **O3 (Utility).** *Benign tasks complete unchanged, and consent burden
  stays below a stated threshold.* → FGR, task-completion parity (UP).
  **Result:** FGR_On (benign false-flag rate) = 3/7 = 42.9% [95% CI
  15.8%–75.0%]. **Not supported at an acceptable threshold** by this run —
  but n=7 total benign cases (2 in `Tier2`) makes this reading barely more
  than anecdotal; the CI spans from "better than one in six" to "three in
  four." UP (task-completion parity vs `Off`) is **not computable at all**:
  `ADR-007`'s own run matrix defines no `Benign`×`Off` cell, so there is no
  baseline trace to diff a benign task's `On`-mode trace against. This is a
  real gap, not a rounding matter — see §5.
- **O4 (Honest limits).** *The blind spot is characterized, not hidden:
  same-origin/same-primitive, data-only attacks
  (`WithinFingerprintDataOnly`) are undetectable by construction and are
  reported as a named residual, with their share of the corpus stated.*
  **Result:** Residual R = 1/22 = 4.5% [95% CI 0.8%–21.8%] of the attack
  corpus. **Supported** — the one `WithinFingerprintDataOnly` case
  (`crates/ferrite-eval/tests/corpus/c24_tool_json_scope_escalation_category5.json`)
  correctly shows `fingerprint_caught = Missed` in every loop-active mode
  by construction
  (`adjudication::tests::fingerprint_missed_within_fingerprint_data_only`),
  and is reported as a floor, not silently dropped from the denominator.
- **O5 (Cost).** *The defense's latency and token overhead are measured,
  not assumed.* → ΔL, ΔT.
  **Result:** ΔL (On − Off total_ms, paired, n_pairs=22): p50 = 0ms, p95 =
  1ms. **ΔT is not computed** — a documented limitation, T-226, see §5.
  ΔL's near-zero reading here is a direct consequence of the rules-only
  fallback this default run uses (no network round-trip), so ΔL here
  measures the loop's own bookkeeping overhead, not a real model-call
  latency delta. **This prediction is now verified, not hypothetical:**
  §2.6's real live-provider run measured ΔL (On − Off, paired, n_pairs=22)
  at p50 = 699 ms, p95 = 1527 ms — the materially different, non-trivial
  number a real network round-trip per loop-active case actually costs.

---

## 2. Metrics — exact formulas and this run's real numbers (§13.2)

> **Superseded numbers.** Sections 2.1–2.6 and 3 report the original n=29
> corpus and are kept as the record of that run. The current corpus (938
> cases), the evaluation fixes that changed what "caught" means, and the
> runtime-guard experiment are in **§8**; read that for current numbers.

Implemented in `crates/ferrite-eval/src/metrics.rs`. Every proportion is a
**Wilson score 95% interval**, computed exactly as specified:
`(p̂ + z²/2n ± z·√(p̂(1−p̂)/n + z²/4n²)) / (1 + z²/n)`, z = 1.96
(`metrics::wilson_interval`, hand-checked against the standard n=10/k=5
worked example in `metrics::tests::wilson_interval_matches_the_n10_k5_worked_example`).

Mode comparisons use **McNemar's exact test** on paired discordant counts
(`metrics::mcnemar_exact_p`, hand-checked against a textbook b=1/c=9 worked
example in `metrics::tests::mcnemar_matches_the_b1_c9_worked_example`),
**Holm-Bonferroni**-corrected across the full 6-comparison mode-pair family
(`metrics::holm_bonferroni`, hand-checked against a 4-p-value family in
`metrics::tests::holm_bonferroni_matches_hand_computed_adjustment`), with
**Cohen's h** effect sizes (`metrics::cohens_h`, checked at its 0 and π
boundary cases).

**A note on the exact-test formula's literal wording:** §13.2 states
McNemar's p-value as `Σ_{i≥min(b,c)}`. By the binomial symmetry
`C(n,i)=C(n,n-i)`, that sum and the standard `Σ_{i=0}^{min(b,c)}` form used
here denote the same quantity reflected around its tail — this
implementation uses the `i=0..=min(b,c)` direction because it is the one
every published reference states the formula as and the one directly
hand-checkable (see `mcnemar_exact_p`'s doc comment for the full
reasoning). A literal `i≥min(b,c)` reading (summing the OTHER, larger
tail) reports large p-values for exactly the skewed samples that should be
significant — backwards for a significance test.

### 2.1 Per-mode metrics, pooled across the whole corpus (n=29)

| Mode | ASR | CR | ADR | SDR | FGR / FSR-proxy |
|---|---|---|---|---|---|
| Off | 22/22 = 100.0% [85.1%–100.0%] | 0/22 = 0.0% [0.0%–14.9%] | n/a — loop never runs in Off | n/a — sanitizer never runs in Off | n/a (n=0, benign has no Off cell) |
| SanitizerOnly | 7/19 = 36.8% [19.1%–59.0%] | 12/19 = 63.2% [41.0%–80.9%] | n/a — loop never runs | 12/14 = 85.7% [60.1%–96.0%] | 1/7 = 14.3% [2.6%–51.3%] |
| LoopOnly | 1/19 = 5.3% [0.9%–24.6%] | 18/19 = 94.7% [75.4%–99.1%] | 16/16 = 100.0% [80.6%–100.0%] | n/a — sanitizer never runs | n/a (n=0, benign has no LoopOnly cell) |
| On | 0/22 = 0.0% [0.0%–14.9%] | 22/22 = 100.0% [85.1%–100.0%] | 19/19 = 100.0% [83.2%–100.0%] | 12/17 = 70.6% [46.9%–86.7%] | 3/7 = 42.9% [15.8%–75.0%] |

ADR/SDR are printed `n/a` for modes that never run the layer being
measured (`Off`'s ADR/SDR, `SanitizerOnly`'s ADR, `LoopOnly`'s SDR) rather
than the structurally-guaranteed `0/N` those cells would otherwise show —
ADR-007 is explicit that per-layer power is read only from the isolated
modes that actually run that layer; printing a precise-looking `0.0%` for
a mode that never ran the loop at all would misleadingly read as "the loop
tried and failed."

### 2.2 Per-tier ASR (the pooled numbers above hide exactly this)

| Tier | Mode | ASR |
|---|---|---|
| Tier1 (n=12 attack) | Off | 12/12 = 100.0% [75.7%–100.0%] |
| Tier1 | SanitizerOnly | 5/12 = 41.7% [19.3%–68.0%] |
| Tier1 | LoopOnly | 0/12 = 0.0% [0.0%–24.3%] |
| Tier1 | On | 0/12 = 0.0% [0.0%–24.3%] |
| Tier2 (n=7 attack) | Off | 7/7 = 100.0% [64.6%–100.0%] |
| Tier2 | SanitizerOnly | 2/7 = 28.6% [8.2%–64.1%] |
| Tier2 | LoopOnly | 1/7 = 14.3% [2.6%–51.3%] |
| Tier2 | On | 0/7 = 0.0% [0.0%–35.4%] |
| Tier3AgentDojo (n=3 attack) | Off | 3/3 = 100.0% [43.8%–100.0%] — **insufficient data (n<5)** |
| Tier3AgentDojo | On | 0/3 = 0.0% [0.0%–56.2%] — **insufficient data (n<5)** |

Tier 3 AgentDojo's cells are explicitly flagged rather than left to look
as precise as Tier 1's — 3 cases supports no per-category claim at all,
per T-227's own statement of this exact limitation.

### 2.3 Residual, overhead

- **Residual R** = 1/22 = 4.5% [95% CI 0.8%–21.8%] — the
  `WithinFingerprintDataOnly` share of the attack corpus, reported as a
  structural floor on ASR, not a failure (§13.2's own framing).
- **Overhead ΔL** (On − Off `total_ms`, paired, n_pairs=22): p50 = 0ms,
  p95 = 1ms. See §1/O5 and §5 for why this specific run's ΔL is not a
  meaningful cost estimate.
- **ΔT** (extra model tokens): not computed. T-226, documented limitation
  — see §5.

### 2.4 Mode-pair comparisons (paired McNemar, Holm-Bonferroni across the full 6-pair family, Cohen's h)

| Mode A | Mode B | b | c | n_pairs | p (raw) | p (Holm) | Cohen's h |
|---|---|---|---|---|---|---|---|
| Off | SanitizerOnly | 12 | 0 | 19 | 0.0005 | 0.0020 | 1.837 |
| Off | LoopOnly | 18 | 0 | 19 | <0.0001 | <0.0001 | 2.679 |
| Off | On | 22 | 0 | 22 | <0.0001 | <0.0001 | 3.142 |
| SanitizerOnly | LoopOnly | 7 | 1 | 19 | 0.0703 | 0.1406 | 0.842 |
| SanitizerOnly | On | 7 | 0 | 19 | 0.0156 | 0.0469 | 1.305 |
| LoopOnly | On | 1 | 0 | 19 | 1.0000 | 1.0000 | 0.463 |

Read this table carefully: `LoopOnly` vs `On`'s p=1.0 does **not** mean the
sanitizer contributes nothing — it means the loop alone already catches
essentially everything the worst-case scripted agent realizes (see §5), so
there is very little room left for the sanitizer to show a marginal
difference in THIS run's methodology. `SanitizerOnly` vs `LoopOnly`/`On`
being the more borderline/non-significant comparisons (p_Holm 0.14, 0.047)
is the honest signal this small a sample can support: the loop's
containment is the strong, clearly-significant result; the sanitizer's
*marginal* contribution on top of the loop is not resolvable at n=19 paired
cases.

### 2.5 Consent-policy sweep (T-010/D10)

| Consent policy | Mode | ASR |
|---|---|---|
| RejectFlagged (**upper bound on human vigilance**) | LoopOnly | 1/19 = 5.3% [0.9%–24.6%] |
| RejectFlagged (upper bound) | On | 0/22 = 0.0% [0.0%–14.9%] |
| ApproveAll (**lower bound**) | LoopOnly | 19/19 = 100.0% [83.2%–100.0%] |
| ApproveAll (lower bound) | On | 10/22 = 45.5% [26.9%–65.3%] |
| RandomP(p=0.5, seed=42) (**middle estimate**) | LoopOnly | 10/19 = 52.6% [31.7%–72.7%] |
| RandomP(p=0.5, seed=42) (middle estimate) | On | 6/22 = 27.3% [13.2%–48.2%] |

`RejectFlagged` is the headline number reported everywhere else in this
document — and it is explicitly an upper bound on what containment could
be if the user were maximally attentive, never an estimate of real human
behavior. `ApproveAll` and `RandomP(0.5)` bracket it: real human vigilance
almost certainly sits somewhere in this very wide range, which this
corpus makes no attempt to estimate more precisely (that is a
human-factors study, out of scope, per ADR-007/§13.4).

`RandomP` is deterministic given a fixed seed
(`adjudication::ConsentPolicy::deterministic_unit_draw`, tested in
`adjudication::tests::t010_random_p_is_deterministic_given_a_fixed_seed`
across 20 repeated calls) — this table is exactly reproducible from the
same corpus and the same seed.

### 2.6 The real, live-provider run (B2, new)

Everything above (§2.1–§2.5) is the **default, always-reproducible run**:
no `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN` set, so the fingerprint's
`may_use` layer runs rules-only (fail-to-empty, per `CLAUDE.md`'s own
invariant) and the corpus-wide run makes zero network calls. This section
reports a second, separately-labeled run made with a real, live
`ferrite_model::OllamaProvider` — `FERRITE_MODEL_SMALL=FERRITE_MODEL_MAIN=
gemma4:31b`, key resolved from the OS keyring (service `"ferrite"`, the
same key `just probe` verified live during A3) via
`harness::try_real_provider()`. Run twice this session, ~30 minutes apart,
with **byte-identical results both times** — real evidence of
determinism (temperature 0, seed 42), not an assumption.

Only the metrics that actually depend on the fingerprint's `may_use`
prediction move at all — everything sanitizer-only or audit-only
(`Off`, `SanitizerOnly`, and `On`'s ASR/CR/ADR/audit chain) is
**byte-identical** to §2.1's rules-only numbers, exactly as it should be
since those modes/fields never consult the fingerprint. The two real
differences, both in the expected direction for a real model versus the
rules-only fallback:

| Metric | Rules-only (§2.1) | Live provider (this run) | Why |
|---|---|---|---|
| `LoopOnly` ASR/CR | 1/19 = 5.3% / 18/19 = 94.7% | 3/19 = 15.8% / 16/19 = 84.2% | A real `may_use` prediction admits a broader (but still closed-vocabulary, still filtered) set of plausible capabilities than the keyword rules alone catch for 2 of the 19 loop-active attack cases. With no sanitizer running in this mode to independently catch the same deviation, those 2 cases' worst-case action now falls inside the (correctly) wider admitted set and is no longer flagged — a real, honest containment reduction in this one isolated mode, not a bug. |
| `On`'s FGR (false gate rate, benign) | 3/7 = 42.9% [15.8%–75.0%] | 1/7 = 14.3% [2.6%–51.3%] | The real model's `may_use` judgment is less naive than the keyword rules for benign tasks — fewer benign actions get spuriously flagged. Direction makes sense (a real model should be *better* at "is this plausible for the task" than five keyword groups) but n=7 benign×On cases is far too small to treat either number as more than illustrative. |

`On`'s headline ASR/CR (0/22 / 22/22, containment complete) and every
McNemar/Cohen's-h comparison against `Off` are **unaffected** — the
sanitizer's excision (T-105/T-003) already removes the injected
instruction before the loop runs in `On` mode, so a wider or narrower
`may_use` set has nothing left to admit or reject for the attack side.
This is exactly the defense-in-depth composition the four-mode design
(ADR-007) exists to demonstrate: the loop's fingerprint precision moves
independently of the sanitizer's own containment, and `On`'s combination
of both stays robust to either layer's individual noise.

**What this run does and does not establish.** It proves the real
`ferrite_model::ModelProvider` pipeline (A3's cache/throttle/budget
decorators, A4's `fingerprint::generate_fingerprint`, B1's rebuilt
`tool_decision::generate_fingerprint`) works end to end against a live
backend, on the real corpus, with real network calls — not just in
isolated unit tests. It does **not** change this document's headline
numbers (§2.1's rules-only run stays the one quoted everywhere else,
since it is the only one every contributor can reproduce with zero
credentials, per R7) and it does **not** grow the corpus (still n=29,
§3) or make the small-`n` caveats throughout this document any less true
— a 2-case shift in a 19-case cell is exactly the kind of change a wider
confidence interval already warns you not to over-read.

---

## 3. Corpus sizing — derived, then honestly reconciled against n=29 (§13.3)

**The derivation**, exactly as §13.3 states it: sample size per cell from
the target CI half-width *w* at 95%, worst case p̂=0.5:
`n ≈ z²·p̂(1−p̂)/w²`. At *w*=0.15: n ≈ 3.8416·0.25/0.0225 ≈ 42.7 → 43. At
*w*=0.10: n ≈ 3.8416·0.25/0.01 ≈ 96.0 → 96.

**The proposed target** (§13.3's table): Tier 1 (WebContent) 120, Tier 2
(ToolOutput) 80, Tier 3 (AgentDojo) 60, Benign 100 — total ≈ 360, chosen so
per-category CIs land near *w*≈0.10–0.20 and the pooled CI near *w*≈0.09.

**What was actually achieved** (`docs/TO-DO.md` T-227, A11's honest
accounting, unchanged by this session): **29 cases** — Tier 1: 17 of 120
(14% of target), Tier 2: 9 of 80 (11%), Tier 3 AgentDojo: 3 of 60 (5%),
Benign: 7 of 100 (7%). **0% double-authored** against the 10% requirement;
Cohen's κ is not computable and was not fabricated from a fake pair
(ADR-008's independence layers are not realized this session — no
teammate/professor slice exists).

**What this means for the numbers in §2, stated plainly:**

- The pooled attack-corpus intervals (n=19–22) are wide but not vacuous —
  e.g. `ASR_On`'s CI is [0.0%, 14.9%], which is a real, useful upper bound
  even though it is far from the ±10-point precision §13.3's derivation
  aimed for.
- The per-tier intervals (§2.2) are considerably wider (Tier 2's n=7 gives
  CI half-widths around 20–35 points), and Tier 3 AgentDojo's n=3 is
  explicitly flagged as **not supporting any claim at all** — 3 cases
  cannot distinguish "this defense generalizes to an external benchmark"
  from noise.
- The benign-side numbers (n=7 total, FGR_On=3/7) are the least powered
  claim in this whole document — a 42.9% point estimate with a CI spanning
  15.8%–75.0% supports "somewhere between rare and common," nothing
  sharper. O3's utility claim is the one this corpus size speaks to least.
- No claim in this document should be read as more precise than its stated
  CI. Where a stratum is too small to say anything (Tier 3, n<5 per §2.2's
  own floor), this document and the generated report say so explicitly
  rather than presenting a number that looks as confident as a
  well-powered one.

---

## 4. Parameter rationale (§13.4) — verified against the actual implementation, drift corrected

| Parameter | §13.4's stated value | Verified true of what `just eval` actually runs? |
|---|---|---|
| Model backend | Ollama Cloud | **Yes, as of B1/B2 — the A12-era drift this row used to describe is fixed, not just documented.** A12's original note here described `ferrite_ipi::tool_decision::LlmMayUsePredictor` making a direct `reqwest::Client` call straight to the Gemini API — that type no longer exists. B1 rebuilt `tool_decision::generate_fingerprint` to take a real `&dyn ferrite_model::ModelProvider` and call A4's `fingerprint::generate_fingerprint`; B2 wired `harness::try_real_provider()` to construct a real `ferrite_model::OllamaProvider` from `ModelConfig`/the OS keyring when `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN` are set, and verified it live against Ollama Cloud (§2.6, `gemma4:31b`, byte-identical across two separate runs). The default `just eval` run (no env vars set, §2.1–§2.5's headline numbers) still uses the rules-only fail-to-empty path deliberately, per R7 — this is a choice about what the *automated, always-reproducible* run does, not a limitation of what the pipeline can do. |
| Model tiering | small for `may_use`, mid for the agent loop | **The `may_use` half is real and verified** (§2.6 uses `ModelTier::Small` via `fingerprint::generate_fingerprint`, unchanged since A4). The agent-loop mid-tier half remains moot for `just eval` specifically: `WorstCaseAgent` (§5) is a deterministic re-enactment of authored `ground_truth`, not a model deciding a plan, by design — it makes no model call of any kind, tiered or otherwise. |
| Model temperature / seed | 0, fixed seed | **Verified, not assumed** — §2.6's two live runs, ~30 minutes apart, produced byte-identical per-mode metrics, which is only possible if the real model calls underneath are genuinely deterministic at temperature 0 with a fixed seed. `WorstCaseAgent`'s own action-decision determinism is separately a pure function of `ground_truth`, unrelated to the model call. |
| Response caching | content-addressed, on by default | Real and active on the §2.6 live path — `ferrite_model::OllamaProvider` is used through its normal `Cache`/`Throttle`/`Budget` decorator stack (A3), unchanged by B1/B2. Not exercised by the default rules-only run, which makes no model call to cache. |
| Call budget | 500 per process, hard abort | Same as response caching — real and active on the §2.6 live path via the same decorator stack, not exercised by the default run. |
| Capability allowlist | closed, 7 labels | Verified unchanged — `ferrite_core::Capability`'s closed enum, untouched this session. |
| Provider failure policy | fail to empty | Verified unchanged — `fingerprint::engine`'s fail-to-empty guards, untouched this session (read-only per this charter's scope walls). |
| `js.execute` | always unscopable | Verified unchanged — `comparator`'s structural handling, untouched this session. |
| Dry-run timeout | 30s | Verified true: `DryRunOrchestrator::new`'s `timeout_secs: 30` default, untouched this session. |
| Excision granularity | sentence segment | Verified true of A5's sanitizer, untouched this session (not independently re-audited beyond reading). |
| Excision replacement | single space, no marker | Same as above. |
| Pattern count | 5 labelled patterns | Carried forward from A5/A11's audit; not independently re-counted this session. |
| Scope precedence | exact > domain-suffix > task-open | Verified unchanged — A7's comparator, untouched this session. |
| **Consent policy (sim)** | swept; `RejectFlagged` headline | **Corrected this session (T-010/D10).** Previously hard-pinned to `RejectFlagged` (D10's defect). Now a real `ConsentPolicy` enum — `RejectFlagged`/`ApproveAll`/`RandomP(p, seed)` — actually read by `adjudicate`, with `RejectFlagged` reported everywhere as the explicit upper bound §13.4 always intended it to be. See §2.5. |
| Detection vs excision split | independently toggled | **Corrected this session (T-215).** Previously `harness.rs::run_one` called `set_detect_enabled` alone, so `strip_enabled` stayed `false` in every eval-harness mode regardless of `DefenseMode` — `On` and `LoopOnly` were behaviorally identical in the actual harness even though A5 had wired live excision in production. Now `run_one`/`ferrite-ui`'s dry-run construction both call `set_defense_mode(mode)`, which derives both flags together. This is what makes `On`'s `SanitizerOnly`-catches-and-strips path (`FinalOutcome::Stripped`, T-004) reachable at all in the eval harness. |

---

## 5. Honest limitations (this session's; A13 owns the final consolidated pass)

1. **The corpus-runner agent is scripted from ground truth, not a real
   model reasoning about whether to comply with an injected instruction.**
   `ferrite_eval::worst_case_agent::WorstCaseAgent` re-enacts each attack
   case's authored `expected_extra_primitives`/`expected_out_of_scope_origins`/
   `attack_origin` unconditionally — it does not read the (possibly
   sanitizer-stripped) content it observes and decide whether to comply.
   This is a deliberate, stated methodology choice (see that module's own
   doc comment) appropriate to THIS project's specific claim — Ferrite's
   contribution is the architectural defense (catching a *realized*
   deviation), not base-model robustness (whether a given model falls for
   a given phrasing) — but it means:
   - `ASR_Off` = 100% here measures "if the agent complied, would anything
     stop it" (answer: no, by construction of `Off`), not "does a real
     model actually comply." A live-model run would very likely show a
     lower, more realistic `Off`-mode ASR.
   - `FinalOutcome::Stripped` is proven correct by the table-driven unit
     test (`adjudication::tests::t004_table_driven_attack_final_outcome_every_combination`)
     but does not appear in this run's aggregate numbers at all (§2.4's
     `LoopOnly` vs `On` p=1.0 is a symptom of this) — because the scripted
     agent's action doesn't depend on whether content was stripped, the
     loop almost always still catches the unconditionally-issued
     deviation directly, so `ContainedViaConsent` dominates. Observing
     `Stripped` in aggregate data would need either a live-model agent or
     a scripted agent that reacts to stripped content specifically.
2. **ΔT (extra model tokens) is not computed.** `ferrite_model::response::TokenUsage`
   exists and is exactly the right shape (T-226), but threading it through
   requires changing `fingerprint::engine::generate_fingerprint`'s return
   type (explicitly off-limits — `fingerprint/` is in this charter's
   do-not-touch list beyond reading) and `ferrite_agent::AgentRuntime::run_turn`'s
   signature (a different crate, not in scope either, and — per finding
   1 above — the agent that would need to carry it doesn't call a model at
   all in this run). Given both blockers are genuinely outside this
   charter's file scope, this is reported as a documented limitation, not
   a schema addition — adding an unpopulated `Timing.tokens` field would
   itself violate `CLAUDE.md`'s no-dead-code invariant.
3. **FSR is a case-level proxy, not the literal segment-level ratio §13.2
   defines.** `ExecutionRecord` carries `sanitizer_caught: LayerOutcome`
   (one value per execution) and `final_outcome`, not a per-segment
   (excised, total) pair — instrumenting that would mean touching
   `ferrite-ipi::sanitizer`/`dry_run`, both off-limits beyond reading per
   this charter. The `FGR / FSR-proxy` column in §2.1 is the same
   computation (`|BenignFalseFlag| / |benign cases run under that mode|`)
   read under two different names depending on mode — documented, not
   silently presented as the literal metric.
4. **UP (utility preservation vs `Off`) is not computable at all** — the
   run matrix (ADR-007) defines no `Benign`×`Off` cell, so there is no
   baseline benign trace to diff against. This is a design property of the
   four-mode protocol itself, not a bug this session introduced or could
   fix within scope.
5. **`WithinFingerprintOriginShift` adjudication gap, root-caused, not
   fixed.** Inspecting `ref09_cat5_within_fingerprint.json` with the T-202
   tool (`cargo run -p ferrite-eval --example inspect_case -- crates/ferrite-eval/tests/pilot_corpus/ref09_cat5_within_fingerprint.json`)
   and its diff dump shows exactly why `fingerprint_caught = Missed` for
   this case despite `consent_gated = Gated`: with no
   `FERRITE_GEMINI_API_KEY` set (this session's deterministic default), the
   rules-only fingerprint predictor does not recognize this case's
   `user_task` phrasing as triggering any capability, so the expected
   fingerprint is legitimately empty (correct fail-to-empty behavior).
   With nothing expected, `comparator::compare` has no admitted capability
   for any primitive, so BOTH the primary read and the navigation to
   `attack_origin` land in `diff.extra_primitives`
   (`{"dom.read", "navigate"}`) rather than `diff.out_of_scope_origins` —
   and `adjudication::adjudicate`'s `WithinFingerprintOriginShift` arm only
   checks the latter bucket. Containment itself is not broken (the case is
   still correctly `Gated`/`ContainedViaConsent`); only this category's
   specific per-layer attribution claim doesn't land as designed. A real
   fix belongs in `adjudication.rs`'s own check (in `ferrite-eval`, in
   scope for a future session, not attempted this session because it is a
   genuine semantics decision, not a one-line patch — see
   `docs/TO-DO.md` T-228 for the exact reasoning).
6. **Single-machine eval, no independent double-authoring.** Every case is
   `Author::SelfAuthored` or `Author::AgentDojo`; Cohen's κ is not
   computable (§3). This is A11's finding, carried forward unchanged.
7. **§13.4's model-backend/tiering/caching/budget rows describe a design
   the eval harness's fingerprint layer does not yet run under** — see §4.
   This is `docs/TO-DO.md` T-224, found by the coordinator before this
   session, not something A12 introduced or could fix within its file
   scope (`fingerprint/` off-limits).

A13 (reconciliation & release) should fold these into the project's final
consolidated limitations section alongside its own findings (irreducible
blind spot, pattern ceiling, consent-policy upper bound, single-machine
eval — per its own charter) rather than duplicating this list; this
document is the source for the eval-specific ones.

---

## 6. Consolidated limitations — final pass (A13, `docs/TO-DO.md` T-113)

This section is the definitive limitations account for v0.1.0. It absorbs
§5 above rather than duplicating it — where a §5 finding still holds
unchanged, it is cross-referenced, not re-derived. Every number below was
re-verified this session against the real corpus files and source, not
copied forward from an earlier agent's claim (see the grep/count commands
cited inline).

1. **Irreducible blind spot.** Same-origin/same-primitive, data-only
   attacks (`GroundTruth::WithinFingerprintDataOnly`) are undetectable by
   the fingerprint/comparator layer **by construction** — there is no
   deviation from the predicted fingerprint for the loop to notice, because
   the attack never leaves the fingerprint's admitted surface. Measured
   share of the attack corpus: **R = 1/22 = 4.5%** [95% CI 0.8%–21.8%]
   (re-verified: `grep -l 'WithinFingerprintDataOnly' crates/ferrite-eval/tests/**/*.json`
   matches exactly one file,
   `crates/ferrite-eval/tests/corpus/c24_tool_json_scope_escalation_category5.json`).
   This is a structural floor on ASR, not a defect — reported as such
   everywhere in this document, never smoothed into the headline number.

2. **Pattern ceiling.** The sanitizer's detection layer is 5 general
   literal-phrase patterns (`instruction_override`,
   `instruction_override_disregard`, `system_prompt_reference`,
   `data_exfiltration_language`, `new_instructions`) plus 5 script-specific
   patterns (`js_fetch_call`, `js_websocket_construction`,
   `js_cookie_access`, `js_storage_access`, `js_send_beacon`) — re-counted
   this session directly against `crates/ferrite-ipi/src/sanitizer/patterns.rs`
   (`GENERAL_PATTERNS`/`SCRIPT_PATTERNS`), not assumed from the directive's
   parameter table. Patterns match literal phrasing; they miss paraphrase,
   synonym substitution, and any non-English injection by design — this is
   stated in the sanitizer's own module docs, not just this document. **The
   architecture (predict → dry-run → compare → consent), not sanitizer
   recall, carries the actual security argument** — `LoopOnly` mode (loop
   active, sanitizer bypassed) independently reaches CR = 94.7% in this
   corpus (§2.1), which is the evidence for that claim, not an assertion.

3. **Consent-policy upper bound.** `RejectFlagged` — the policy behind
   every headline number in this document — models a maximally attentive
   human who correctly rejects every flagged item. It is reported
   everywhere as an **upper bound on human vigilance**, never as an
   estimate of real user behavior (T-010's own finding, §2.5). The sweep
   against `ApproveAll` (lower bound, `On`-mode ASR = 45.5%) and
   `RandomP(0.5)` (middle estimate, `On`-mode ASR = 27.3%) shows how wide
   the real range plausibly is; no human-factors study narrows it further,
   and none was in scope.

4. **Single-machine eval.** Every number in this document comes from one
   run, on one machine (macOS, Apple Silicon, `rustc 1.98.1` —
   `docs/BUILD_BUDGET.md`'s recorded environment), authored and run by one
   agent session per corpus-authoring pass. There is no cross-hardware, no
   cross-platform (Windows/Linux CI never ran the eval, only `check`/
   `test`), and no cross-operator replication of these specific numbers.
   The harness is deterministic and reproducible *on this machine*
   (`just eval` twice produced byte-identical 29-case/96-execution counts,
   per A12's entry) — reproducibility and independent replication are not
   the same claim, and only the former is established here.

5. **Corpus-size shortfall (T-227).** Re-counted this session directly
   against the corpus directories (`grep`/`ls`, not trusted from prior
   claims): **29 cases** against the §13.3 target of **~360** — Tier 1
   (WebContent) 17/120 (14.2%), Tier 2 (ToolOutput) 9/80 (11.25%), Tier 3
   (AgentDojo) 3/60 (5.0%), Benign 7/100 (7.0%); 22 attack / 7 benign
   overall, confirmed via `grep -c '"corpus": "Attack"'`/`"Benign"` across
   all three corpus directories. Confidence intervals throughout this
   document are correspondingly wide: the pooled attack-mode intervals
   (n=19–22) run roughly ±15 points; per-tier intervals (n=3–12) run
   ±20–35 points; Tier 3's n=3 is flagged as supporting no per-category
   claim at all (§2.2). **0% double-authored** against the 10% requirement
   — every case is `Author::SelfAuthored` or `Author::AgentDojo` (a
   citation-derived provenance, not a second independent human) — so
   Cohen's κ is not computable and was not fabricated from a fake pair.

6. **Worst-case-agent methodology.** `ferrite_eval::worst_case_agent::WorstCaseAgent`
   re-enacts each attack case's authored deviation unconditionally — it
   does not read the (possibly sanitizer-stripped) content and decide
   whether to comply, the way a real model-driven agent would. This is a
   **deliberate scope choice**, stated in the module's own doc comment, not
   an oversight: Ferrite's claim is that the architecture contains a
   *realized* deviation, not that a given model resists a given phrasing of
   an injected instruction. Concretely, this means `ASR_Off = 100%` in this
   corpus measures "if the agent complied, would anything stop it"
   (by construction, nothing does in `Off`), not "does a real model
   actually comply" — a live-model run would very likely show a lower,
   more realistic `Off`-mode ASR, and would be a different, complementary
   experiment, not a re-run of this one.

7. **The live-app gap (T-224) — significant, not a footnote.** The actual
   running `ferrite-ui`/`ferrite-shell` application does not use the
   `ferrite-model`/`ferrite-engine`/`browser_loop` stack this rebuild built
   and tested. Verified this session by reading the call path directly:
   `ferrite-ui`'s agent-task handler still calls
   `ferrite_agent::gemini::GeminiAgent::read_api_key()` (a `gemini_key.txt`
   file or a bare `FERRITE_GEMINI_API_KEY` env var — code that predates
   `ferrite-model` and does not know the OS keyring exists), and never
   constructs a `ferrite_model::ModelProvider`, never calls
   `ferrite_engine::BrowserEngine`, and never drives
   `ferrite_agent::browser_loop::run_agent_loop`. The IPI defense loop
   itself (`ferrite-ipi`: fingerprint, sanitizer, dry-run, comparator,
   consent) **is** real, tested, and (per `docs/PROGRESS.md`'s A7 entry)
   wired into `ferrite-ui`'s own agent-task path — what is *not* wired in
   is the new model-provider and engine abstraction this rebuild's later
   phases (A3, A9) built alongside it. This is a real, material gap
   between "what was built and proven in isolation" and "what actually
   runs when you launch the app," not a minor integration detail.

8. **`ServoEngine` real-navigation gap (T-220).** The new,
   engine-agnostic path's Servo backend (`ferrite-engine-servo`) does not
   complete a real page load in this environment: `ServoEngine::new`
   succeeds against a real, successfully-built Servo, but `navigate()` to a
   loopback fixture server never observes a TCP connection attempt within
   the drive-until-loaded window (leading hypothesis: `HeadlessServoSession`
   needs a genuine winit event loop driving it, not bare
   `spin_event_loop()` calls — not confirmed, not fixed). **This is
   isolated to the new wrapper.** `docs/TO-DO.md` T-220 carries a
   2026-09-18 addendum, confirmed live by the coordinator: the live
   `ferrite-ui`/`ferrite-shell` app's own, separate Servo integration
   (`ferrite-servo::session`/`shell.rs`, built with
   `cargo run -p ferrite-shell --features ferrite-servo/servo`) uses a real
   winit event loop and was directly observed rendering a real page
   end-to-end. Real browsing in the actual product is not blocked by
   T-220; the new `BrowserEngine`-trait path's Servo backend is.

9. **Other open items material to an honest accounting, briefly:**
   - **T-221** — `ferrite-ipi` depends on `ferrite-agent`, backwards from
     the target dependency direction (`core ← {model, audit, engine} ← ipi
     ← agent ← {ui, eval, cli}`). Confirmed still present this session
     (`crates/ferrite-ipi/Cargo.toml`: `ferrite-agent = { workspace =
     true }`). Fixing it means relocating `BrowserTool`/`AgentRuntime`/
     `ToolExecutor` to a crate both sides can depend on — real, cross-crate
     work, not attempted here (out of a doc-reconciliation charter's
     remit; `CLAUDE.md`'s dependency-direction invariant now names this
     exception explicitly rather than silently contradicting it).
   - **T-222** — `FingerprintDiff`'s `extra_primitives`/`out_of_scope_origins`
     each record only one half of a `(tool, origin)` pair, so the consent
     panel can name authorized origins in general but not tie a specific
     flagged primitive to the specific origin it needed. A real fix means
     enriching the diff's shape in `ferrite-ipi::comparator`.
   - **T-223** — iced 0.13's `button` widget has no `Focusable` impl in
     this pinned version, so the consent panel's Reject button cannot get a
     literal keyboard-focus-ring default; mitigated with reject-listed-first
     plus strict completeness-gating (no silent default-approve path
     regardless of stray-keypress focus).
   - **T-228** — `WithinFingerprintOriginShift`'s adjudication check credits
     a catch only via `diff.out_of_scope_origins`; a degenerate (correctly
     empty, fail-to-empty) fingerprint routes the same deviation into
     `diff.extra_primitives` instead, where the check never looks.
     Root-caused (§5 item 5), not fixed — containment itself is not broken
     (the case is still correctly gated), only this category's specific
     per-layer attribution claim.

None of the nine items above is hidden elsewhere and stated differently
here — this section is the single place a reader should go for "what does
this project's evaluation not show."

---

## 7. `docs/REBUILD_DIRECTIVE.md` §14 — definition of done, scored item by item

Verified fresh this session (T-113), not carried forward from an earlier
agent's self-report. Evidence is cited per item; **[ ]** means genuinely
not met, stated plainly rather than rounded up.

- [x] `just check && just test` green on a clean clone, with no network and
      `OLLAMA_API_KEY` unset, under 5 minutes, without Servo. — Re-run this
      session from a freshly-recreated branch: `just check` clean (fmt,
      clippy `--all-targets -D warnings`, `cargo machete`), `just test`
      green workspace-wide. Exact wall-clock time and command are recorded
      in this session's `docs/PROGRESS.md` entry.
- [x] Every model call routes through cache + throttle + budget decorators;
      a full eval re-run on an unchanged corpus makes zero live calls;
      `just cache-stats` reports the hit rate. — True of `ferrite-model`'s
      own conformance suite and decorators (A3, unchanged), and, as of B2,
      demonstrated on a real path too: the eval harness's fingerprint layer
      now goes through a genuine `ferrite_model::OllamaProvider` when a
      backend is configured (§2.6, verified live), through the same
      Cache/Throttle/Budget stack A3 built. The *default* `just eval` run
      that every contributor and CI actually runs makes zero live calls by
      deliberate design (R7, rules-only fail-to-empty, no env vars set) —
      not because the live path doesn't exist, but because determinism and
      credential-free reproducibility are the right default.
- [ ] `just build-servo` succeeds and the action-conformance suite passes
      against `ServoEngine` at least once, with cost recorded. — **Partially
      true, not done.** `just build-servo` succeeded for real (A9: 15m31s,
      6.4GB, `docs/BUILD_BUDGET.md`). The action-conformance suite did
      **not** fully pass against real `ServoEngine`: construction succeeds,
      but `navigate()` never completes a real page load (T-220). Marked
      unmet rather than rounded up to "passed."
- [x] Every D1–D14 defect closed, each with a test that would fail if
      reverted. — All fourteen have a `docs/TO-DO.md` row citing a real SHA
      and test name (T-001 through T-014, cross-checked against
      `docs/DECISIONS.md`'s ADR history this session). D3/T-003 in
      particular took three charters (A5, A6, A12) to close for real —
      documented as `in-progress` until T-215 landed, not marked done
      early.
- [x] Zero dead code, zero `#[allow(dead_code)]` without a comment naming
      the reason and a `T-###`. — `grep -rn "allow(dead_code)" crates/`
      returns **zero matches** workspace-wide this session — there is
      nothing to check the "has a `T-###` comment" condition against
      because the crate-root `#![deny(dead_code)]` gates
      (`ferrite-core`/`ferrite-model`) made the allow-and-annotate pattern
      unnecessary; no reachable-but-uncalled function was found separately.
- [x] No doc claims anything a test or SHA doesn't back; the doc CI check
      passes. — `scripts/check_purge.sh` and `scripts/check_no_archive_links.sh`
      both run clean this session (the former updated to drop its one
      stale exclusion, see this session's `docs/PROGRESS.md` entry). Every
      SHA spot-checked this session against `git log -1 --format="%H %s"
      <sha>` matched its cited description (sample: `e474403`, `a45b2ad`,
      `6cb4c70`, `6250cb3`, `ec7ece9`, `795977a`, `d9e2c12`, `aaae747`,
      `6e6926a`).
- [x] `CLAUDE.md` has no status section; `.rules`, `commands.md`, broker
      crates and the 9222 forward are gone from the working tree. — Verified
      fresh: `find . -name ".rules" -o -name "commands.md"` (excluding
      `.git/`) — no matches; `grep -rn "9222"` outside `docs/archive/` and
      the rebuild's own record docs — no matches in any live config;
      `crates/ferrite-capability-broker`/`ferrite-policy`/`ferrite-sandbox`
      absent from disk and from `Cargo.toml` `members`. `CLAUDE.md` itself
      re-read this session — no "planned"/"not yet implemented" section,
      only the dependency-direction invariant was updated (to name T-221's
      known exception, not to add a status list).
- [ ] Corpus authored to the §13.3 targets, κ reported; `just eval` emits
      the full metrics table with Wilson CIs and McNemar results. —
      **False, stated plainly.** Corpus is 29 cases, not ~360 (item 5
      above); κ is not computed (0% double-authored). The second half is
      true in isolation — `just eval` does emit the full metrics table with
      Wilson CIs and McNemar/Holm-Bonferroni/Cohen's h results — but the
      item as a whole is not met because the corpus-sizing half is not.
- [x] `docs/EVALUATION.md` complete: objectives, formulas, sizing
      derivation, parameter rationale, honest limitations. — All five
      sections present (§1–§4 plus this §6); this session's §6/§7 are the
      final consolidation the document's own header called for.
- [x] `docs/BUILD_BUDGET.md` shows target-dir size and cold-test time at
      every phase gate. — A1 and A9 entries present with real, cited
      numbers; no phase after A9 changed the build surface enough to need
      a new entry (A10–A12 added no new heavy dependency).
- [x] No commit in `rebuild/*` or `main` mentions Claude, Anthropic, or AI
      assistance. — Verified fresh this session across full history:
      `git log --all --oneline | grep -iE "claude|anthropic|ai.assist"` and
      `git log --all --format="%H %B" | grep -iE "claude|anthropic"` both
      return only two commits (`f582dda`, `01c3f54`) whose *content*
      discusses fixing **`CLAUDE.md` the file** — no AI-attribution trailer,
      byline, or "generated with" string anywhere in the matched text. No
      breach found.

**Net: 8 of 11 items fully met, 1 partially met (Servo conformance —
build succeeds, real navigation does not), 2 not met (corpus size/κ). This
matches, item for item, what `docs/TO-DO.md` T-227 and T-220 already state
honestly — nothing in this checklist is new information, only the first
place it is scored against the directive's own checklist explicitly.**

---

## 8. The red-team corpus, the runtime guard, and what changed in the measurement (2026-10-01, ADR-014)

The owner asked for the defense to be evaluated against every real-world case
and then made as strong as possible. Attacking it found defects in every layer
and in the evaluation itself. This section is the current record; §2–§7 describe
the earlier n=29 run.

### 8.1 Corrections to what "caught" means (they change the numbers)

| Change | Why it mattered |
|---|---|
| `fingerprint_caught` for a deviation case now needs a *declared* item to be in the diff | it used "all declared primitives OR all declared origins", and `.all()` over an empty set is true, so a case that declared nothing on one side counted as caught against a **clean** diff. On the old n=29 corpus every declared deviation was in fact flagged (ADR stayed 100%), so no old figure was inflated, but the metric could not have failed |
| origin-shift and origin declarations compare normalized origins; a flagged `navigate` covers a declared origin (T-206) | a path or default port made an origin-shift case "missed" |
| a domain-suffix / `task_open` case gets its content origin as the context page | every event of such a case carried no origin, was flagged, gated the well-behaved agent and never served the attacker's page |
| the scripted agent knows `tab.open`, `cookie.read`, `storage.read`, `screenshot`, `scroll`, `wait`, `dom.query`; `dom.write` is typing; and a benign agent uses exactly the capabilities the task's words imply | a declared primitive with no script was silently never attempted; a false alarm on the benign agent is now a comparator precision error |
| the dry run records a download and a new tab at their *destination* origin | recorded at the page's origin, an agent on an allowed site could fetch from an attacker without producing a deviating event |
| the rule layer matches whole words, grants cookies/storage only when named, and knows everyday read verbs | first benign corpus: 3 of 7 gated (43%); see ADR-014 §4 |

### 8.2 The corpus

`scripts/gen_redteam_corpus.py` writes 909 cases to
`crates/ferrite-eval/tests/corpus_redteam/` (787 attack, 122 benign), run with
the 29 existing ones: **938 cases, 3,488 executions**. The axes: 13 tasks x 12
attacker goals (every primitive an attack can need) x 11 carrier vectors x 28
payload dressings (plain, 14 disguises, 5 languages, 5 structural forms, 3 that
the sanitizer cannot recognise) x 3 scope types, plus 20 origin look-alikes and
5 same-site spellings, plus benign controls (ordinary pages, attack-adjacent
text, every scope type). Labels are derived from the capability lowering table
(ADR-001), never from running the defense; `tests/corpus_redteam_validate.rs`
checks that table and each task's capability set against the real code.

**Independence.** The cases are self-authored by the defense's author and
generated from shared templates. The Wilson and McNemar figures treat them as
independent draws, which a matrix is not, so the intervals understate the
uncertainty, and the cases are tuned-against data (ADR-008). Only the 3
AgentDojo cases are an external slice (and n=3 supports no claim). No second
author exists, so Cohen's κ is still not computable (T-227).

### 8.3 Results (`just eval`, rules-only fingerprint, no network)

| Mode | ASR | ADR | SDR | FGR |
|---|---|---|---|---|
| Off | 809/809 = 100.0% | n/a | n/a | n/a |
| SanitizerOnly | 135/806 = 16.7% [14.3–19.5] | n/a | 671/673 = 99.7% | 3/129 = 2.3% (FSR-proxy) |
| LoopOnly | 122/806 = 15.1% [12.8–17.8] | 621/621 = 100% | n/a | n/a |
| On | 63/809 = 7.8% [6.1–9.8] | 624/624 = 100% | 671/676 = 99.3% | 0/129 = 0.0% [0–2.9] |

Residual R (actions the prediction admits): 122/809 = 15.1%. LoopOnly's ASR
*is* the residual, which is the point of ADR-007: the architecture contains
every attack that needs an action outside the prediction and nothing else. The
sanitizer strips 59 of the 122 (detectable wording), leaving On at 63: the
residual with wording it cannot recognise (paraphrase, social engineering,
narrative framing) on the carriers the scan cannot see.

**What these numbers do not say.** The agent is scripted from ground truth, so
they say nothing about whether a real model takes the bait (that is the base
model's robustness, which the project does not claim), and 0% FGR is the rate
for a well-behaved scripted agent under a rules-only prediction: a real agent's
false-gate rate depends on its actual actions and the LLM `may_use` layer, which
this run does not exercise. ADR = 100% is by construction of a scripted agent
issuing exactly the declared deviation; it measures the comparator, not the
agent.

### 8.4 The runtime-guard experiment (`just guard-eval`, `ferrite_eval::guard_eval`)

The corpus runner shows the dry run the attacker's content, which models a dry
run that can see the real page. The live app's cannot. This experiment runs the
compromised real run (the worst-case agent, no defense in its way) and checks
each action as the live loop does before executing it.

| Stratum | Attacks | Succeed without the guard | Succeed with the guard | Benign blocked |
|---|---|---|---|---|
| Tier1 | 539 | 539 (100%) | 96 (17.8%) | 0/90 |
| Tier2 | 267 | 267 (100%) | 26 (9.7%) | 0/39 |
| Tier3 (AgentDojo) | 3 | 3 | 0 | n/a |
| **All** | 809 | 809 (100%) | 122 (15.1%, all residual) | 0/129 |

The guard blocked no admitted action (it and `compare` classify every event
identically; `tests/guard_eval_invariants.rs` and the sweep in
`comparator/guard.rs` check it). *Without the guard the loop, facing a dry run
that cannot see the attack, ran all 809.* The sanitizer is not applied here, so
this is the architecture alone.

### 8.5 Component red-team suites

| Suite | What it pins |
|---|---|
| `ferrite-ipi/tests/red_team_sanitizer.rs` | 25 instructions x 25 disguises = 625 disguised texts, each detected and excised with the benign context around it intact; 85 benign texts untouched; the known false positives and the residual disguises pinned so the inventory stays honest; every HTML carrier shape; attributes holding `<`/`>`; CSS-generated text; 14 hostile inputs (1 MB, deep nesting, tag characters) without a panic or a hang |
| `ferrite-core/tests/red_team_scope.rs` | 16 tests: 23 look-alike hosts never admitted; same-site spellings admitted; scheme/port; IP spellings; every non-http scheme refused; public suffixes refused and registrable domains accepted |
| `fingerprint/rules.rs` | whole-word matching, inflections, noun `-ing`, explicit-only cookie/storage, everyday read verbs, hostile input |
| `comparator/guard.rs`, `ferrite-ui/src/guard_tests.rs`, `runtime_guard.rs` | the guard over every primitive x origin x fingerprint; blocks, approvals, `js.execute`, four-block stop, link clicks, opaque schemes, fixed observation text |
| `ferrite-ipi/src/dry_run/engine.rs` | download / new tab recorded at their destination |

### 8.6 Known limits (each pinned or listed in `docs/TO-DO.md` T-249)

- **Residual.** Same primitive at the task's own origin; any origin under a
  `task_open` scope; data-only abuse. By construction (O4).
- **Sanitizer.** Paraphrase, unlisted languages, syllable hyphenation, spelled-out
  markup/URLs, reversed non-Latin text, payloads split across elements. Known
  false positives: pages that discuss attacks ("attackers exfiltrate data",
  "never ignore the previous warnings") and any mention of "system prompt".
- **Guard.** CSS-selector clicks, form `action`s, server-side redirects (the
  destination is checked on the next action, after the request); a trailing-dot
  host (`example.com.`) fails closed.
- **Measurement.** Scripted agent; rules-only fingerprint; matrix-correlated
  cases; self-authored; no κ.


---

## 9. Running with a real model

Everything above is run by a scripted agent that complies with every injection. This section is the other half: a
**real model** in the agent's seat, so the question becomes what a model actually does under an injection and what
Ferrite then does about it. It is meant to be run by the owner, on their own machine, with their own keys. Nothing
here is run by `cargo test` (every behaviour is tested offline against the `mock` provider and a loopback fake
server), and no result from a real model has been produced or is quoted in this repository yet.

### 9.1 What it is

`crates/ferrite-eval/src/live/`, driven by `cargo run -p ferrite-eval --example live_eval -- <flags>`. For each case it
runs the app's own agent loop (`ferrite_agent::browser_loop`: same action schema, system prompt and parser) against the
dry-run engine, with the injection planted in the page or tool output the agent reads. Two roles can each be a model
or not, independently:

| role | `llm` (default) | other |
|---|---|---|
| predictor (`--predictor`) | the small-tier model proposes the fingerprint's `may_use`, as in the app | `rules`: rule layer only |
| agent (`--agent`) | the main-tier model chooses actions | `scripted`: the worst-case script, no calls |

And the defense, per case, in the modes of `--modes` (default `off,guard`):

| mode | what is on |
|---|---|
| `off` | nothing; the baseline |
| `guard` | the runtime guard refuses every action outside the predicted fingerprint (ADR-014), sanitizer off |
| `full` | guard plus the sanitizer on what the agent reads |
| `dryrun` | stage one only: the plan on a clean synthetic page, compared with the prediction (consent burden) |

The simulated user refuses every deviation, the best case for containment.

### 9.2 Providers and keys

The runner reuses `ferrite-model` and adds no HTTP stack, so it supports what `ferrite-model` supports:

| `--provider` | what it is | key |
|---|---|---|
| `gemini` | Google Gemini | `FERRITE_GEMINI_API_KEY` or the OS keyring (service `ferrite`) |
| `ollama` | Ollama Cloud (`https://ollama.com`), or a local server with `--base-url http://localhost:11434` | `OLLAMA_API_KEY` or the keyring; a local server needs none |
| `mock` | deterministic stand-in, no network; exercises the pipeline, says nothing about any model | none |

**OpenAI-compatible and Anthropic endpoints are not supported**, because `ferrite-model` has no backend for them. Adding
one is a `ferrite-model` change (T-277), not a runner change. A hosted OpenAI-compatible server that also speaks
Ollama's `/api/chat` can be reached with `--base-url`, nothing else can.

Keys are read from the environment or the OS keyring by `ferrite-model` and nowhere else. They are never accepted as a
flag, never printed, and never written: error text kept in a result or in the budget ledger passes through a redactor
that removes the key values the process holds (tested with a fake server that echoes the key back in a URL and in an
error body). Model tags are configuration, there is no default: `--model TAG` for both roles, or `--small-model` and
`--main-model`; environment `FERRITE_LIVE_MODEL`, `FERRITE_LIVE_SMALL_MODEL`, `FERRITE_LIVE_MAIN_MODEL`, then the app's
`FERRITE_MODEL_SMALL` / `FERRITE_MODEL_MAIN`.

### 9.3 Cost first: `--plan`

`--plan` prints what a selection would cost and calls nothing (no key needed):

```
live_eval --plan --provider gemini --model <tag> --corpus agentdojo
```

It reports the cases and runs in the window, how many are already stored, fingerprint predictions, agent steps (typical
and worst case), a rough token count (characters / 4), and how many invocations the batch cap implies. For the full
AgentDojo import at the default two modes it prints about 2,092 runs and about 7,300 model calls typical (51,000 at
most), i.e. roughly 74 invocations at the default cap of 100 calls. Those are estimates from the corpus and the step
limit; what a model really takes, what the cache absorbs and the provider's tokenizer are only known by running.

### 9.4 Batches, limits and resuming

- **Selection:** `--corpus agentdojo|redteam|core|pilot|agentdojo-hand|all`, `--suite agentdojo/banking`, `--only attack|benign`,
  `--seed S` (reproducible shuffle, so a small batch samples every suite), `--offset/--limit`.
- **Batches:** `--batch-size N` runs the next N cases that still have work to do. Run the same command again for the
  next batch; `--batch-index K` takes slice K of the whole ordering instead.
- **Calls:** `--max-calls N` (default 100) is a hard cap on requests that reach the provider, retries included. When it
  is spent the case in flight is abandoned unrecorded (it stays pending), a partial-results ledger is written, and the
  process exits 3.
- **Rate limits:** `--pause-ms` spaces calls; a 429/5xx/timeout is retried up to `--max-attempts` (default 5) with
  exponential backoff and jitter, honouring the provider's `Retry-After` (capped at `--backoff-max-ms`, default 90 s, so
  an hour-long daily quota is not slept through). After `--max-consecutive-failures` (default 3) failed calls in a row the
  invocation stops with exit 4: the provider is saying no, so stop and come back later.
- **Failures are not scores.** A call that never got an answer (rate limit, outage, rejected key, timeout) is stored as an
  error and excluded from every rate; `--retry-failed` redoes those. A model that answers badly (unparseable action,
  empty reply, unusable fingerprint) *is* a result: the fingerprint falls back to empty and the case is scored as such.
- **Resume:** results are an append-only JSONL file, one line per case and mode, synced before the next case starts, under
  `<out>/results/<provider>--<small>--<main>.jsonl` (default `--out target/live-eval`). A crash, Ctrl-C or closed laptop
  loses at most the case in flight, and a torn last line is skipped. Re-running the same command skips what is stored.
  Stored results are reused only when the settings that change behaviour (models, modes, steps, predictor, agent) hash the
  same. Model responses are also cached under `<out>/model-cache/` (temperature 0), so a re-run of an unchanged case is
  free and does not count against `--max-calls`; `--no-cache` turns that off.

Exit codes: 0 the batch finished cleanly, 1 internal error, 2 usage, 3 call cap reached, 4 provider unavailable, 5 the
batch finished but some cases failed (rerun with `--retry-failed`).

### 9.5 The report

`live_eval --report [--compare gemini:<tag>,ollama:<tag>]` writes `<out>/REPORT.md` and `<out>/report.csv` from the
stored results, one group per provider and model pair. Every rate carries a 95% Wilson interval and rows with few
observations say so. It has attack runs (attempted, executed, blocked of attempted, final answer) per mode; baseline
against defense case by case (McNemar exact, Cohen's h); benign tasks (false positives, consent burden); fingerprint
prediction precision and recall; breakdowns **by suite, by attack category (the attack class), by ground truth and by
carrier**; and cost (calls, cache hits, rate-limit answers, tokens). Residual attacks that use the task's own primitive
at its own origin are reported as a floor, not a failure, and attacks with no attacker string to recognize are counted
and excluded from attack rates.

### 9.6 What to run first

```
# 1. nothing is called; no key needed
cargo run --release -p ferrite-eval --example live_eval -- --plan --provider gemini --model <tag> \
    --corpus agentdojo --seed 1 --batch-size 10

# 2. one small real batch (key in FERRITE_GEMINI_API_KEY or the keyring)
cargo run --release -p ferrite-eval --example live_eval -- --provider gemini --model <tag> \
    --corpus agentdojo --seed 1 --batch-size 10 --max-calls 100 --pause-ms 4000

# 3. read what happened, then repeat step 2 for the next batch
cargo run --release -p ferrite-eval --example live_eval -- --report
```

Do the `mock` run once first to see the whole flow with no key (`--provider mock --model x`).

### 9.7 What this does not establish

The labels come from AgentDojo's ground-truth calls mapped onto Ferrite primitives, not from AgentDojo's own checks;
one attack template (`important_instructions`) and one sample per case at temperature 0; a synthetic environment where
"task success" is approximated by the agent giving a final answer; a simulated user who refuses every deviation. The
numbers are Ferrite's measurement, not AgentDojo's utility or security scores, and are not comparable to published
AgentDojo results. Provider tokenizers and real step counts are unmeasured until the first run.

---

## 10. AgentDojo coverage

`crates/ferrite-eval/tests/agentdojo_full/` holds 1,046 cases generated by `scripts/import_agentdojo.py` from
[AgentDojo](https://github.com/ethz-spylab/agentdojo) (MIT), commit `089ed468cf3ed0322acc66b0211f26d9d90dbf60`,
benchmark v1.2.2: every user task x every injection task of the four suites (workspace 40 x 14, travel 20 x 7, banking
16 x 9, slack 21 x 5 = 949 attack cases) with the `important_instructions` template, plus one benign twin per user task
(97). `agentdojo_full_manifest.json` records the pin, per-suite counts, the lowering and tool maps, and anything the
importer could not resolve (nothing, at this commit).

- **It parses, never executes.** The task files are read with Python's `ast` (constants folded, f-strings evaluated,
  the benchmark-version rules re-implemented); no AgentDojo code, tool, environment or utility/security check runs.
  Each task's ground-truth tool calls are mapped to the nearest Ferrite primitive and origin (`TOOL_MAP`), and the
  importer refuses to run if a suite registers a tool the map does not cover.
- **Labels follow the specification**, never the defense's output: the closed capability lowering of ADR-001 (a Rust
  test checks the importer's copy against `ferrite_core::LOWERING`).
- **Declared approximations:** injection tasks whose ground truth is text only (some workspace and travel tasks) take
  their effects from the goal text and are listed in the manifest as `injection_tasks_with_goal_text_effects`; the rest
  use the ground-truth calls.
- **Offline checks:** `cargo test -p ferrite-eval --test agentdojo_full_validate` (loads every file, counts against the
  manifest and the pinned commit, unique ids, label consistency, every attack has a benign twin).
- **Regenerating or verifying needs the real dataset, which is not in this repository's tooling**: clone the repository,
  check out the pinned commit, and run `python3 scripts/import_agentdojo.py --src <checkout> --check` (omit `--check` to
  rewrite). It prints these instructions if `--src` is missing or at another commit. Last verified byte-identical
  against a fresh clone at the pinned commit on 2026-10-04.
