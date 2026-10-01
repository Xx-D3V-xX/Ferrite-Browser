> HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS. DO NOT USE AS CONTEXT.
> Superseded by `docs/EVALUATION.md`; genuinely-true decisions extracted to
> `docs/DECISIONS.md`. Kept for the reasoning trail only. See `docs/archive/README.md`.

# Ferrite Browser — Evaluation Methodology Plan

> **Status of this document — READ FIRST.**
> This is a **living working document** and is *expected to churn*. Corpus sizes, category
> lists, the AgentDojo slice scope, and metric thresholds will change as the work proceeds
> and as early results feed back. Do **not** treat anything here as settled fact the way
> `PROJECT_REFERENCE.md` should be treated. This is the design we are building toward, not a
> record of what exists.
>
> Companion documents:
> - `PROJECT_REFERENCE.md` (repo root) — stable narrative hub. What Ferrite is.
> - `Browser/CLAUDE.md` — coding rules for the Claude Code agent.
> - `Browser/PROGRESS.md` — dated change log.
> - `Browser/TO-DO.md` — task/block breakdown. (Evaluation work will be decomposed into
>   tasks here *after* this plan is approved — not before.)
>
> **Provisional academic framing.** Research questions and venue ambitions may shift. The
> evaluation is being designed to a high bar (publishable quality), but the committed
> deliverable floor is a working demo + report.

---

## 1. Purpose and Bar

This document defines *how Ferrite's IPI defense will be evaluated* — what we measure, on
what data, against what baseline, and how we make the result credible rather than
self-serving.

**Quality bar:** maximize rigor; there is no external deadline forcing compromise. The plan
is written toward a publishable standard. The minimum acceptable outcome is a working demo
plus a report carrying real evaluation numbers; everything beyond that is upside we are
deliberately reaching for.

**What a defense evaluation must prove** (not "it runs," but four measured claims):
1. **Security** — it contains attacks (true-positive / containment rate).
2. **Utility** — it does not wreck normal use (false-positive consent rate).
3. **Cost** — the performance overhead of the dry-run is acceptable and quantified.
4. **Verifiability** — the audit trail can reconstruct, after the fact, that containment held.

Claims 1 and 2 are a pair and **require two different corpora** (attacks vs. benign). This is
the backbone of the whole plan: a defense that looks perfect on attacks but is never tested
on benign tasks is not evaluated, it is advertised.

---

## 2. Threat Model

### 2.1 In scope

**Indirect Prompt Injection (IPI)** — malicious instructions embedded in untrusted content
that the agent ingests during a task, causing behaviour the task never called for. Ferrite
ingests *all* external content through a single channel — every tool result arrives to the
agent as `AgentToolResult.data` — so injection has **one ingestion channel with two
carriers**, both in scope:

- **T1a — Web-content carrier.** Malicious instructions hidden in page content the agent
  reads (via the `dom.read`/page-read path): hidden/`display:none` elements, off-screen text,
  HTML comments, `alt` attributes, CSS-concealed spans, script-borne text. The page-read
  tool's result carries the injection. This is Ferrite's home turf and the primary carrier
  its sanitizer (component 2) targets.
- **T1b — Tool-output carrier.** Malicious instructions embedded in the text/JSON any *other*
  tool returns to the agent. Same logical attack, same channel, different carrier.

> **Why "carrier" not "surface."** In Ferrite's architecture both arrive identically — as
> `AgentToolResult.data` — differing only in *which tool* produced the result (a page-read
> vs. another tool). They are not two architecturally distinct attack surfaces; they are two
> carriers over one mediated channel. This is why one defense covers both.

> **Why both are in scope with one architecture.** Ferrite's defense is *delivery-agnostic*:
> the predict→dry-run→compare→consent loop watches *what the agent does* (tools called,
> origins touched) — not *where the instruction came from*. Whether an injection arrives via
> a rendered `<div>` or a tool's JSON response, if it provokes an unexpected tool call or
> origin, the comparator catches it identically. T1b is therefore a near-free extension of
> the existing architecture and a genuine selling point: *one mechanism, multiple delivery
> carriers.* (See §8 for the one code addition this implies.)

### 2.2 Explicitly deferred (out of scope, future work)

- **T2 — Malicious or compromised tools.** A tool whose *implementation* is adversarial and
  abuses authority *entirely within its declared contract*, reaching no new tool or origin.
  Ferrite's current fingerprint granularity catches malicious-tool behaviour only when it
  produces observable deviation (new tool/origin). Fully covering T2's in-contract abuse
  requires finer-grained capability-lineage enforcement — i.e. the **deferred capability
  broker**. T2 is named in the paper's threat model and future-work as the natural next
  stage, with the deferred components as the intended mechanism. It is **not** evaluated now.

### 2.3 Attacker assumptions (to be grounded in literature, not invented)

Attack categories are **objective-primary** — organized by what the injection tries to
achieve — with attack *technique* recorded as a secondary tag. The five categories are
informed by published IPI taxonomies (e.g. Greshake et al. 2023, and the attack types
catalogued in AgentDojo / InjecAgent); per-category anchoring to specific published
equivalents is fixed at authoring time, not pre-committed here. Category 5 is partly novel
(browser-specific) and may have no clean published twin. The attacker is assumed to control
untrusted content (web pages, tool-returned data) but not the agent's internals or the
trusted UI consent surface.

---

## 3. The Two Corpora

We construct **two** corpora. Both are browser-native (the Option B foundation). The attack
corpus carries the security result; the benign corpus carries the utility result.

### 3.1 Attack corpus (browser-native, primary contribution + artifact)

Each attack case is a labelled record:

- **carrier** — T1a (web content) or T1b (tool output)
- **attack category** — one of five objective-primary categories:
  (1) data_exfiltration, (2) unauthorized_action, (3) agent_redirection,
  (4) scope_escalation — all *in-scope* for the fingerprint mechanism — and
  (5) within_fingerprint_abuse — *out-of-scope by design* (uses only expected
  tools/origins; see §7 stratified scoring)
- **attack technique(s)** — secondary tag(s) from a closed vocabulary (e.g.
  instruction-override, obfuscation, context-manipulation); multi-valued, descriptive only
- **carrier vector** — the concrete vector (hidden div, comment, alt-text, tool-JSON field, …)
- **user task** — the legitimate task the agent was asked to do
- **attacker goal** — what the injection tries to make the agent do
- **expected_origins** — the origins the legitimate task is authorized to touch (authored
  ground truth; see §7)
- **ground-truth deviation label** — the tool(s)/origin(s) a *successful* injection would
  trigger that the legitimate task would not (this is what "caught" is checked against)

This corpus is **the artifact**: a labelled, reusable, *browser-native* IPI dataset.
Existing benchmarks are tool-runtime simulations; a browser-grounded corpus is a
contribution in its own right. Component 7 (the dataset pipeline, Task 19) is the
*instrument that generates and labels this corpus* — which is precisely why its schema must
be designed against this section (see §7).

**Tiering.** The attack corpus is built in tiers so partial completion still yields a
result:
- **Tier 1 (core):** T1a web-content injections across the taxonomy categories. The minimum
  viable security corpus. Carries the bulk of cases.
- **Tier 2 (generality):** T1b tool-output injections — same categories, second carrier.
  Proves the delivery-agnostic claim. Smaller (generality demonstration, same harness).
- **Tier 3 (independence slices):** held-out attacks authored independently (see §6). Small
  (10–20 each); independence value is provenance, not volume.

**Sizing.** Target **Option M — 60–100 attack cases** total. Proportions: Tier 1 heavy;
benign sized comparably to Tier 1; Tier 2 smaller; independence slices small. Exact N is
**deferred to a ~10-case authoring pilot** that measures realizable throughput of
well-formed cases (a "well-formed" attack must actually succeed with defense OFF). Decide the
rule now, set the number from the pilot — premature precision is rejected.

### 3.2 Benign corpus (non-negotiable — the utility half)

Comparable user tasks on comparable pages/tool-outputs with **no injection present**. Run
through the identical pipeline to measure how often the defense *still* fires a consent
prompt or blocks an action. This produces the **false-positive consent rate** — the cost
side of the ledger.

Design requirement: the benign corpus must be *plausibly matched* to the attack corpus
(similar task types, similar page complexity), so the false-positive rate is measured under
conditions comparable to where attacks are caught. A benign corpus of trivially simple tasks
would understate friction and is not acceptable. Benign size is sized comparably to Tier 1
(the utility half needs as much statistical weight as the security half).

> **Note on empty fingerprints.** Recall (per `PROJECT_REFERENCE.md` §2.3) that vague /
> open-ended prompts correctly produce empty fingerprints, routing everything through
> consent. The benign corpus must use *reasonably specific* tasks — otherwise we measure the
> known empty-fingerprint behaviour rather than the defense's real-world false-positive rate.
> Task specificity is a controlled variable, to be stated explicitly in the writeup.

---

## 4. Baseline Protocol

A defense number is meaningless without a comparison. The baseline is **Ferrite with the IPI
defense disabled** — identical browser, identical agent, identical inputs, with the
predict→dry-run→compare→consent loop turned off.

- **Defense OFF:** the agent runs the task directly; an injection that succeeds demonstrates
  the attack is real and the surface is genuinely vulnerable.
- **Defense ON:** the same case runs through the full loop; we record whether the deviation
  was contained (consent-gated or blocked).
- **Loop-only:** the sanitizer is bypassed and the agent's raw (un-sanitized) content runs
  through the full dry-run/compare/consent loop. This is the **RQ1 number** — it measures the
  architecture's standalone containment with the content filter explicitly off, which is the
  cleanest possible evidence that containment comes from the architecture rather than from
  passive stripping. Supported by `DefenseMode::LoopOnly` (TO-DO Task 18).
- **Sanitizer-only:** the sanitizer runs but the dry-run/compare/consent loop is bypassed.
  Measures the filter's standalone containment. Supported by `DefenseMode::SanitizerOnly`.

> **Per-layer power is read from the isolated modes, not from ON.** In ON, the sanitizer
> strips first and the loop runs only on the residue, so each layer sees only what upstream
> passed through — ON therefore *understates* each layer's standalone power and must not be
> read as a per-layer number. The sanitizer's power is M1a (Sanitizer-only); the
> architecture's power is M1_loop (Loop-only); ON is the composed, deployed-system number.
> The two deltas are first-class results: **M1 − M1_loop** = the sanitizer's marginal
> contribution to the shipped stack; **M1 − M1a** = the architecture's marginal contribution
> beyond passive stripping.

**The result is the delta.** "Injection succeeds X% with defense off → contained Y% with
defense on" *is* the headline. Without the OFF condition, a high ON containment number could
just mean the attacks were weak. Every attack case is run in both conditions.

The committed four conditions are OFF, ON, Loop-only, and Sanitizer-only — together they
isolate each layer and measure their composition.

(Optional, stretch) One further condition is upside, not required:
- A **model-level** defense (e.g. a prompt-based "ignore injected instructions" system
  instruction) to strengthen the "architectural beats model-level" claim. Run only if effort
  budget allows; the committed result stands on the four conditions above.

---

## 5. Metrics

| # | Metric | Corpus | Definition | Answers RQ |
|---|--------|--------|------------|------------|
| M1 | **Containment rate (stratified)** | attack, `in_scope` only | % of injection attempts caught (consent-gated or blocked) vs. executed silently, computed over in-scope attacks (categories 1–4) only | RQ1 (architectural containment) |
| M1a | **Sanitizer-only containment** | attack | % caught with the sanitizer active but the dry-run/compare/consent loop bypassed (the filter's standalone power) | RQ1 (attribution) |
| M1_loop | **Loop-only containment** | attack, `in_scope` only | % caught by the full loop running on UN-sanitized content (the architecture's standalone power, sanitizer off) — the most direct RQ1 evidence | RQ1 (architectural containment, isolated) |
| M2 | **Attack success rate (baseline)** | attack, defense OFF | % of injections that succeed with defense disabled | establishes attacks are real |
| M3 | **False-positive consent rate** | benign | % of benign tasks that still triggered a consent prompt / block | RQ2 (cost / friction) |
| M3a | **Sanitizer benign false-strip rate** | benign | % of benign cases the sanitizer flags/excises when run sanitizer-only (loop bypassed), i.e. benign × SanitizerOnly (A5) | RQ2 (sanitizer cost, isolated) |
| M4 | **Dry-run latency overhead** | both | added wall-clock time from the contained dry-run (the agent plan runs twice on clean tasks) | RQ2 (cost) |
| M5 | **Fingerprint discrimination** | both | how cleanly expected-vs-actual fingerprinting separates genuine injections from benign deviation (e.g. precision/recall, or a confusion matrix over M1/M3) | RQ3 (discrimination) |
| M6 | **Verifiability demonstration** | sampled | for sampled events, show the audit log reconstructs the containment decision and chain integrity holds | RQ4 (post-hoc verifiability) |

M3 and M3a are distinct benign false-positive numbers (full stack vs. sanitizer alone) and are
never combined.

M5 is the synthesis metric: M1 (catch attacks) and M3 (don't flag benign) are the two axes
of the same discrimination question, and reporting them together (rather than M1 alone) is
what makes the evaluation honest.
**Per-layer attribution and the meaning of ON (four-mode design).** Containment is measured
under four defense modes (§4): OFF (M2, attacks are real), Sanitizer-only (M1a, filter alone),
Loop-only (M1_loop, architecture alone), and ON (M1, composed stack). **Standalone layer power
is read from the isolated modes, never from ON.** In ON the sanitizer strips before the loop, so
the loop acts only on the sanitizer's misses; ON's per-layer catch rates therefore *understate*
each layer and are not reported as standalone numbers. ON is the deployed-system number — the
right metric for "how well does the shipped product contain attacks," and the wrong metric for
"how good is layer X." The two deltas are reported as results: **M1 − M1_loop** (sanitizer's
marginal contribution to the shipped stack) and **M1 − M1a** (architecture's marginal
contribution beyond passive stripping). Note this design isolates each layer and measures their
composition, but does not decompose a single ON run into per-layer credit — that decomposition
comes from comparing the isolated-mode records case-by-case, not from instrumenting ON.

**Scope-tightness stratification (Decision 4).** M1 is additionally reported broken down by the
case's `scope_type` (`exact` / `domain_suffix` / `task_open`). Tight scopes carry strong
containment guarantees; `task_open` cases carry a measured, disclosed residual. Reporting the
gradient (e.g. "M1 on exact-scoped = X%, on task_open = Y%") turns the loose-scope soft spot into
a quantified result rather than a hidden weakness. This is a breakdown of existing runs, not new
runs.

**Category-5 (out-of-scope) accounting.** Category 5 is excluded from M1 by the `in_scope`
filter and reported separately as bounded residual risk with per-layer attribution (see §7.4).
Containment of model behaviour is **model-independent by construction** (the defense consumes
only provider-neutral types; see §10 item 4); friction metrics (M3/M5) are reported as
Gemini-conditioned.

---

## 6. Credibility Strategy (defeating "you graded your own homework")

Option B's central risk: we author the attacks, the defense, and the labels — so a reviewer
can dismiss the result as circular. We defeat this with **three layers of independence plus
two structural safeguards.** The layers are ordered by how much independence they provide.

### 6.1 Three independence layers

1. **Teammate-authored held-out slice (internal independence).** A teammate who did not build
   the defense authors a slice of attacks. Partial independence — same team, may know the
   architecture — but a genuinely different author.
2. **External professor slice (external human independence).** A faculty member outside the
   project authors or reviews a slice. This is the high-value layer: no stake in the defense,
   not on the build team. **Treated as *planned upside*, not a guarantee** — faculty time is
   uncertain, so the evaluation must stand without it (see floor below). The ask is deferred
   until after the corpus exists and the defense is frozen (firewall timing); author identity
   (supervisor vs. non-supervising faculty) is open, with a non-supervising author marginally
   more independent.
3. **AgentDojo adapted slice (external benchmark independence).** A slice of a recognized
   third-party benchmark we did not author at all (the "C" in B-leaning-C). **Scoped to the
   Slack suite only** — the one AgentDojo suite whose web-browsing threat model maps to
   Ferrite's `BrowserTool` surface; the other suites (banking, workspace, travel) assume
   domain APIs the browser agent lacks and would require fabricating capabilities, so they
   are excluded. Selection is **complete-suite-with-exclusions** (adapt every mappable case,
   report non-mapping ones with reason). Because Ferrite ingests web content as tool output
   (§2.1), the Slack cases validate Ferrite's **unified ingestion channel**, not a separate
   surface. Exact mapped-case count is set by a local mappability spike (read the Slack suite
   against `BrowserTool`). This is the independence substitute that is fully within our control.

**Credibility floor (what we can guarantee):** teammate slice + AgentDojo (Slack) slice. If
the professor contributes, that is added independence, not a load-bearing dependency. The plan
is defensible on the floor alone.

### 6.2 Two structural safeguards (hard rules)

- **The held-out firewall.** The defense is built and tuned **only** against our own
  (non-held-out) attack set. The teammate slice, professor slice, and AgentDojo slice are run
  **once, at the very end, untouched.** We do **not** fix the defense in response to anything
  those slices expose mid-development — doing so silently folds them into the training set and
  destroys their value. This mirrors a train/test split: no peeking.
- **The briefing boundary.** Independent authors receive a **threat-model brief** (what the
  target is, what tools exist, what an attack looks like) but **not a defense brief** (how the
  fingerprint loop, sanitizer, or consent gate work). The moment an author knows the
  mechanism, their attacks get unconsciously shaped by it and independence degrades. §6.3
  pins down exactly what is shared vs. withheld.

### 6.3 Independent-author briefing packages (deliverable — produced at corpus stage)

When we reach corpus construction, this plan commits to producing **two standalone briefing
documents**, each self-contained so the author needs no further context:

- **Teammate brief (held-out internal slice).**
- **External professor brief (external slice).** Produced only if/when the professor ask is
  made — wording stays conditional.

Each brief will contain:
- the threat model (T1a + T1b), in plain language;
- the agent's tool surface (the nine `BrowserTool` operations) — *what* the agent can do;
- the shape of a valid attack case (user task + attacker goal + carrier), with examples;
- the labelling format they should submit;
- scope boundaries (what's in/out — e.g. T2 is out);
- an explicit "what we are *not* telling you, and why" note, so the independence is
  transparent rather than feeling like withheld information.

Each brief will **deliberately exclude**: how Ferrite detects or contains attacks, what the
sanitizer strips, how fingerprints are computed, how consent is triggered. This withholding
*is* the methodology — it is what makes their attacks independent evidence.

> These briefs are listed here as committed deliverables so they are not forgotten. They are
> written later (at the corpus-construction stage), because they must reference the finalized
> corpus schema and case format from §3 and §7.

### 6.4 Additional non-author levers

- **Published-taxonomy grounding (§2.3):** attack categories are informed by the literature,
  so the threat model is not purely self-defined.
- **Pre-registration:** the attack category list and success criteria are written down
  *before* runs, so it is provable we did not curate to a flattering number.
- **Honest failure reporting:** attacks Ferrite *fails* to catch are included and reported.
  A defense paper that reports only successes is less believable, not more.

---

## 7. Dataset Schema (finalized contract)

This is the reason the evaluation is designed *before* `dataset.rs` is implemented. The schema
is **two-layer**: a *case definition* (authored once, invariant per case) and an *execution
record* (one row per case-run, since a case runs in multiple defense modes). The vocabulary it
references (capabilities, primitives, technique/carrier tags) is canonical in `CLAUDE.md` →
*Tool Vocabulary and Capability Model*; this section pins the dataset fields. Full design
rationale: `FINALIZED_DECISIONS.md` Decisions 5–6.

### 7.0 Capability model (compact reference; full canon in CLAUDE.md)

A **capability** = (**action class** × **origin scope**). Six action classes (`read`,
`interact`, `navigate`, `download`, `clipboard`, `execute`) over the eight real `BrowserTool`
primitives. There is no `email.read` tool — "email" is a property of the origin scope, authored
per case. The comparator **lowers** semantic capabilities to expected (primitives + origin
scopes) and compares against actual, with **per-origin attribution** (group by origin → admit
against the most-specific admitting capability → unadmitted origin = deviation). `js.execute`
(`execute` class) is **unscopable** → always a deviation, always consent-gated. `expected_origins`
is authored, trusted, and never written from run data (integrity rule). Origin scope is **typed**:
`exact | domain_suffix | task_open`; containment is reported stratified by this tightness (§5).

### 7.1 Case definition (authored, trusted, invariant)

| field | type | notes |
|---|---|---|
| `case_id` | Uuid | PK |
| `corpus` | enum Attack \| Benign | |
| `tier` | enum Tier1 \| Tier2 \| Tier3Teammate \| Tier3Professor \| Tier3AgentDojo | |
| `author` | enum SelfAuthored \| Teammate \| Professor \| AgentDojo | provenance |
| `carrier` | enum WebContent \| ToolOutput | T1a / T1b |
| `carrier_vector` | enum (closed; partition matches `carrier` — see CLAUDE.md) | exactly one |
| `attack_category` | Option&lt;enum&gt; (5 categories) | None for benign |
| `attack_techniques` | Vec&lt;enum&gt; (7 closed tags) | ≥1 for attacks; empty for benign |
| `in_scope` | bool (STORED) | true for categories 1–4, false for 5; drives stratified M1 |
| `user_task` | String | |
| `attacker_goal` | Option&lt;String&gt; | None for benign |
| `expected_origins` | OriginScope `{ exact: Vec&lt;String&gt;, domain_suffix: Vec&lt;String&gt; }` | authored, trusted |
| `scope_type` | enum Exact \| DomainSuffix \| TaskOpen | tightest-fit obligation |
| `scope_rationale` | Option&lt;String&gt; | required iff TaskOpen |
| `ground_truth` | GroundTruth enum (§7.3) | what "caught" is judged against |
| `taxonomy_anchor` | Option&lt;String&gt; | filled at authoring |

### 7.2 Execution record (one per case-run)

| field | type | notes |
|---|---|---|
| `exec_id` | Uuid | PK |
| `case_id` | Uuid | FK |
| `timestamp` | DateTime&lt;Utc&gt; | |
| `run_label` | enum R1..R10 \| A1..A5 | A1/A2 sanitizer-only, A3/A4 loop-only, A5 benign sanitizer-only (M3a) |
| `model` | enum Gemini (extensible) | stored so a future model slice is append-only |
| `defense_mode` | enum On \| SanitizerOnly \| Off | Task 18 |
| `expected_fingerprint` | Option&lt;SemanticFingerprint&gt; | None in Off; capabilities (action-class × scope) |
| `expected_realization` | Option&lt;{ primitives, origin_scopes }&gt; | lowered form; None in Off |
| `actual_events` | Vec&lt;{ primitive: ToolId, origin: Option&lt;String&gt; }&gt; | ORDERED, origin-bound (replaces lossy set) |
| `computed_diff` | { extra_primitives, out_of_scope_origins } | two deviation kinds |
| `sanitizer_caught` | enum Caught \| Missed \| NotApplicable | |
| `fingerprint_caught` | enum Caught \| Missed \| NotApplicable | |
| `consent_gated` | enum Gated \| NotGated \| NotApplicable | |
| `data_fields_accessed` | Vec&lt;String&gt; | recorded attempt (populated by origin-binding work) |
| `network_attempts` | Vec&lt;String&gt; | recorded attempt |
| `final_outcome` | enum ContainedViaConsent \| Blocked \| Executed \| BenignNoFlag \| BenignFalseFlag | derived |
| `residual_risk` | enum None \| BlastRadiusContained \| RealHarm \| NotApplicable | |
| `timing` | { total_ms, dry_run_ms, predict_ms } | M4 |
| `audit_log_anchor` | String (hash/id) | M6 |

Derived at analysis time (NOT stored): `unscopable_primitive_invoked` (= extra_primitives ∋
js.execute); `production_residual` (= fingerprint_caught==Missed && consent_gated==NotGated).

### 7.3 GroundTruth (the label "caught" is judged against)

A four-variant tagged enum (avoids null-soup; makes the category-5 caught-vs-residual split
auditable in the data):
- `Deviation { expected_extra_primitives, expected_out_of_scope_origins }` — categories 1–4.
- `WithinFingerprintOriginShift { legitimate_origin, attack_origin }` — category 5, origin-
  distinguishable (caught by per-origin attribution; recorded so the "blind spot shrank" claim
  is auditable).
- `WithinFingerprintDataOnly { legitimate_data_ref, attack_data_ref }` — category 5, same-
  origin/same-primitive/data-only: the irreducible residual. `attack_data_ref` is authored;
  whether the system can *observe* it depends on Tier-A `data_fields_accessed` instrumentation
  (pilot-gated). If Tier B, still recorded as ground truth and reported as documented residual.
- `None` — benign cases (keeps benign in the same schema without a nullable-deviation hack).

### 7.4 Synthetic-data containment and the category-5 residual

Synthetic-data containment is a **structural invariant, not a measured variable**:
`RecordingExecutor` returns only synthetic data and the containment layer logs rather than sends
network attempts — no dry-run path exposes real data. So the schema records the *attempt*
(`data_fields_accessed`, `network_attempts`) against an invariant of zero real-data exposure,
not a contained/leaked flag. The honest varying category-5 number is the **production residual**
(derived above). Per-origin attribution and the `js.execute` rule shrink the category-5 blind
spot to the narrow same-origin/data-only residual; finer measurement of that residual (via
`data_fields_accessed`) is **pilot-gated** (Tier A only if such cases prove non-trivially
frequent; else documented as honest residual, Tier B).

Every M1–M6 query is a filter/aggregate over these two layers (M1 stratified over `in_scope`;
also reported stratified by `scope_type`, §5). Any metric-needed field omitted means re-running
experiments — hence designing from this list. Storage: rusqlite 0.37 (bundled), flat
`src/dataset.rs`, temp paths via `std::env::temp_dir()`. The legacy `IpiEvent`/`IpiLabel` in
`comparator.rs` are superseded.

> **Coupling note.** The §8 sanitizer T1b extension, the capability-mapping / comparator rewrite
> (§8 item 6), and this schema are coupled — all concern "what we capture about an injection."
> They are built together against this finalized list.


---


## 8. Implied Code Work (beyond the current implemented state)

The evaluation as designed implies a bounded set of new code, decomposed into TO-DO tasks:

1. **`dataset.rs` (Task 19)** — implement component 7 to the §7 schema. The single existing
   stub.
2. **Sanitizer extension for T1b (Task 20)** — extend component 2 to scan/strip injection
   patterns in *tool-output* text, not just HTML/JS. Defense-in-depth (containment already
   holds without it via the loop), but needed for full T1b coverage and for the sanitizer to
   be exercised on the second carrier. Bounded — a plumbing extension of existing pattern logic.
3. **Defense mode toggle (Task 18)** — a clean switch with four modes (On / SanitizerOnly /
   LoopOnly / Off) to run the agent as the full composed stack, sanitizer-only, loop-only
   (sanitizer bypassed — isolates the architecture for RQ1), or fully disabled, for the §4
   baseline and the per-layer ablation. Implemented as `DefenseMode`. (The base three-mode
   toggle shipped first; `LoopOnly` was added once the four-condition measurement design in
   §4/§5 was settled.)
4. **Evaluation harness (Task 21)** — a runner that takes a corpus, executes each case across
   defense modes, and emits dataset records. Lives in a dedicated **`ferrite-eval` crate**,
   Servo-free by default (depends only on `ferrite-ipi` + `ferrite-agent`; the eval runs
   entirely through the dry-run path, which never touches Servo), with a feature-flag fallback
   if any corpus case needs a real page fetch.
5. **AgentDojo adapter (Task 22, small)** — maps the Slack suite onto Ferrite's tool surface
   and content path for the §6.1 external slice. **Timeboxed** — explicitly scoped to a
   validation slice, not a full benchmark port, to avoid scope-creep into Option A.
6. **Capability mapping + comparator rewrite (pre-Task-19 vocab-fix block).** The fingerprint
   vocabulary (semantic capabilities) and the dry-run record (browser primitives) must be
   reconciled before the schema means anything. This block: (a) authors the thin
   capability→(primitives, origin-scope) mapping; (b) trims `rule_based_must_use` + the
   predictor allowlist to mapped capabilities only (cutting phantom tools with no primitive
   realization); (c) adds origin-binding instrumentation to the dry-run record (an ordered
   event log binding each primitive to the origin it acted on, replacing the lossy tool set);
   (d) rewrites `compare()` as lower-then-compare with per-origin attribution + the
   `js.execute`-always-deviation rule. Contained to `tool_decision`, `dry_run.rs`,
   `comparator.rs`. No Servo. This is upstream of the schema and of Task 19.

> **CI reality.** All build verification runs through GitHub Actions (Windows + macOS), not
> local builds (the dev laptop cannot do full Servo builds). The `ferrite-eval` crate is
> Servo-free and therefore locally buildable — a deliberate choice to relieve the CI-only
> bottleneck. Linux-only paths (the network-namespace half of component 4) are not CI-covered
> and must be noted as a coverage limitation in the writeup.

---

## 9. Experiment Matrix (what gets run against what)

| Run | Corpus | Condition | Produces |
|-----|--------|-----------|----------|
| R1 | Attack — Tier 1 (T1a) | OFF | M2 baseline (web-content attacks are real) |
| R2 | Attack — Tier 1 (T1a) | ON | M1 containment (web content) |
| R3 | Attack — Tier 2 (T1b) | OFF | M2 baseline (tool-output attacks are real) |
| R4 | Attack — Tier 2 (T1b) | ON | M1 containment (tool output) → generality claim |
| R5 | Benign corpus | ON | M3 false-positive consent rate |
| R6 | Benign + attack (timed) | ON | M4 dry-run latency overhead |
| R7 | Held-out: teammate slice | ON | independent-author containment (run once) |
| R8 | Held-out: professor slice | ON | external-author containment (run once, if available) |
| R9 | AgentDojo Slack slice | ON (+ OFF) | external-benchmark validation (run once) |
| R10 | Sampled events from R1–R9 | — | M6 verifiability / audit-chain integrity |

*Per-layer ablation rows (isolate each layer; see §4/§5). Run after the freeze (they touch the
same frozen defense):*

| Run | Corpus | Condition | Produces |
|-----|--------|-----------|----------|
| A1 | Attack — Tier 1 (T1a) | Sanitizer-only | M1a sanitizer-only containment (filter alone, web content) |
| A2 | Attack — Tier 2 (T1b) | Sanitizer-only | M1a on the tool-output carrier (T1b sanitizer extension alone) |
| A3 | Attack — Tier 1 (T1a) | Loop-only | M1_loop architecture-alone containment (sanitizer off, web content) — RQ1 |
| A4 | Attack — Tier 2 (T1b) | Loop-only | M1_loop on the tool-output carrier (architecture alone) |
| A5 | Benign | Sanitizer-only | M3a sanitizer benign false-strip rate |

A5 runs post-freeze with the other ablations.

The deltas are the findings: **M1 − M1_loop** (sanitizer's marginal contribution) and
**M1 − M1a** (architecture's marginal contribution). A3 (Loop-only on T1a) is the single most
direct RQ1 result — the architecture containing injection with the content filter off.

M5 (discrimination) is computed across R2/R4 (attacks caught) vs. R5 (benign flagged).
R7–R9 are the firewall-protected independence runs — executed only after the defense is
frozen. A1–A4 likewise run only after the freeze.

M1 from R2/R4 is additionally tabulated by `scope_type` (Decision 4) — a breakdown of these same runs, not extra runs.

---

## 10. Resolved Decisions (record)

The seven open decisions from the prior revision are now resolved. Recorded here as the
authoritative decision log; some carry a measurement deferred to a pilot/spike (the *rule* is
fixed, the *number* is set by measurement).

1. **Corpus sizes — RESOLVED.** Option M (60–100 attack). Proportions locked: Tier-1-heavy;
   benign ≈ Tier 1; Tier 2 smaller; independence slices small (10–20). Exact N deferred to a
   ~10-case authoring pilot measuring throughput of well-formed cases (must succeed with
   defense OFF). See §3.1.

2. **Category list — RESOLVED.** Objective-primary, five categories, technique as a secondary
   closed-vocab tag. Categories 1–4 in-scope; category 5 (within_fingerprint_abuse)
   out-of-scope by design, scored stratified (M1 over in-scope only) and reported as bounded
   residual with per-layer attribution. Per-category anchoring to published taxonomies fixed
   at authoring. See §2.3, §3.1, §7.

3. **AgentDojo slice scope — RESOLVED.** Slack suite only (others excluded — domain-API tools
   would require fabricated capabilities). Complete-suite-with-exclusions. Framed as
   unified-ingestion-channel validation (§2.1). Exact mapped-case count + web-paths-only vs.
   model-Slack-UI deferred to a local mappability spike. See §6.1.

4. **Models under test — RESOLVED.** Gemini only. Containment is **model-independent by
   construction** — the defense consumes only provider-neutral types via the `AgentRuntime`
   trait boundary; model-specificity is confined to `gemini.rs`. Friction metrics (M3/M5)
   reported as Gemini-conditioned. A second backend is low-cost future work (isolated to
   `ferrite-agent`).

5. **Professor slice logistics — RESOLVED.** Planned upside, conditional, off the critical
   path. Ask deferred until after corpus exists + defense frozen (firewall timing). Same brief
   template as the teammate slice (threat model only). Author identity open. See §6.1, §6.3.

6. **Harness home — RESOLVED.** Dedicated `ferrite-eval` crate, Servo-free by default,
   feature-flag fallback for any live-page cases. Makes the workspace seven crates
   (PROJECT_REFERENCE §5 updated accordingly). See §8 item 4.

7. **Defense-mode conditions — RESOLVED (revised).** Four committed modes (OFF / ON /
   Sanitizer-only / Loop-only), isolating each layer and measuring their composition (§4/§5).
   Per-layer power is read from the isolated modes; ON is the deployed-stack number, never a
   per-layer number. Ablation runs A1–A4 (§9) are committed (not upside), run post-freeze.
   The **model-level-defense baseline** (§4) remains optional upside, decided at run time.
   (Supersedes the prior "both optional" resolution — the loop-only isolation is RQ1's most
   direct evidence and cannot be optional.)

**Tool-vocabulary decision (the schema gate) — RESOLVED.** Two-layer vocabulary: semantic
capabilities (fingerprint + consent UI) over browser primitives (execution + dry-run record),
connected by a thin mapping where each capability = (primitives + origin-scope). Comparator =
lower-then-compare with per-origin attribution and specificity precedence. `js.execute` is
always a deviation. Origin scope is per-case authored for evaluation (with a disclosed
deployment story: static config, future prompt-inference). The full canonical vocabulary (eight primitives, six action classes, capabilities, technique and carrier_vector tags, the unscopable rule) now lives in `CLAUDE.md` → *Tool Vocabulary and Capability Model*, with rationale in `FINALIZED_DECISIONS.md`. This decision creates the pre-Task-19 vocab-fix block (§8 item 6). See §7.0.

---

## 11. Sequencing (recommended order of work)

1. **Approve this plan.** (§8 is decomposed into TO-DO tasks 18–22.)
2. **§10 decisions are resolved** (sizes, categories, AgentDojo scope, etc.).
3. **Execute the pre-Task-19 vocab-fix block (§8 item 6):** author the capability mapping,
   trim the rule engine + predictor allowlist, add origin-binding instrumentation, rewrite
   `compare()` as lower-then-compare. The schema depends on this — it pins the field meanings.
4. **Finalize the Task 19 dataset schema** against §7 (two-layer, origin-scoped).
5. **Implement:** the defense mode toggle (Task 18, `DefenseMode`) → `dataset.rs` (Task 19) +
   sanitizer T1b extension (Task 20).
6. **Build the evaluation harness** (Task 21, `ferrite-eval`).
7. **Construct corpora:** ~10-case pilot first (locks exact N), then Tier 1 + benign (core
   result), then Tier 2. Run the AgentDojo mappability spike to lock the Slack slice count.
8. **Produce independent-author briefs** (§6.3); teammate authors slice, walled off.
9. **Freeze the defense.** Run R1–R6 (own corpora).
10. **Run R7–R9** (independence slices, once, no tuning afterward); ask the professor slice
    here if pursued.
11. **Run R10** (verifiability sampling).
12. **Results → report** (separate later effort; this plan feeds its evaluation section).

The ordering exists to prevent the known failure modes: building the dataset/sanitizer before
the vocabulary and schema are settled (rework), and contaminating the independence slices by
running them before the defense is frozen (lost credibility).

---

## 12. Mapping to Research Questions

- **RQ1** (architectural containment without content detection) → M1_loop (A3/A4 — the
  architecture isolated, sanitizer off) as the primary evidence, M1 (stratified, R2/R4) for
  the composed stack, and the deltas vs. M2 (attacks real) and vs. M1a (beyond stripping).
- **RQ2** (cost of architectural enforcement) → M3 (friction) + M3a (sanitizer-isolated
  companion to M3) + M4 (latency).
- **RQ3** (fingerprint discrimination of injection vs. benign deviation) → M5.
- **RQ4** (post-hoc verifiability) → M6.

If a research question moves (they are provisional), the affected rows here move with it. Keep
this section in sync — a metric with no RQ, or an RQ with no metric, is a gap to close.