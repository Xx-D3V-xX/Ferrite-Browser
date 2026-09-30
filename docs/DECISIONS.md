# Ferrite — Architecture Decision Records

Numbered, dated, append-only. Extracted from `FINALIZED_DECISIONS.md` and
`EVALUATION_PLAN.md` (both now under `docs/archive/`, see the banner on each
for why they're not live references) by Agent A0, per
`docs/REBUILD_DIRECTIVE.md` §6/A0. Only decisions still believed true are
here — anything superseded by the rebuild (the D1–D14 defect list) says so
inline rather than being silently dropped.

Each entry cites the commit SHA where the decision's implementation actually
landed in the pre-rebuild codebase, per R2 (no status claim without proof).

---

## ADR-000 — Architectural defense over model-level defense

**Date:** project inception, reaffirmed throughout. **Status:** live, unchanged by the rebuild.

Prompt injection is not treated as solvable by better model training — the
labs building the models say as much themselves. Ferrite instead constrains
*what an agent is allowed to do* and makes deviation visible and
consent-gated, so a successful injection has a bounded blast radius
regardless of what the malicious text says. Mechanism: **predict → dry-run →
compare → consent.**

1. **Predict** the task's expected fingerprint (must-use + may-use tools/origins).
2. **Dry-run** the agent's plan against synthetic data, no real network reachable.
3. **Compare** actual (dry-run) behavior against the predicted fingerprint.
4. **Consent-gate** any deviation before the real run proceeds.

This is the one decision every later ADR and every A1–A13 charter serves.
Nothing in the rebuild changes it — the rebuild fixes how faithfully the
mechanism is implemented (D1–D14), not the mechanism itself.

---

## ADR-001 — Capability = (action class × origin scope)

**Date:** design fixed by commit `46b2177` (2026-07-03). **Status:** live, carried into directive §8 verbatim.

A capability is not a fake domain tool ("email.read" doesn't exist as
something the agent can do) — it's an **action class** (a grouping of
primitives sharing a security character: `read`, `interact`, `navigate`,
`download`, `clipboard`, `execute`) paired with an **origin scope** (where
it's authorized to act). "Email" is a property of the origin scope, authored
per task, never a tool the model can invent. Closed capability vocabulary:
`web.read`, `web.navigate`, `web.interact`, `web.download`, `scoped.read`,
`clipboard.read`, `clipboard.write`.

This eliminated a real historical bug: the rule engine and LLM predictor
used to emit tool IDs (`email.read`, `calendar.write`, `network.fetch`,
`report.write`, `contacts.read`, `storage.*`, `screenshot`, `form.submit`)
that no `BrowserTool` variant could ever realize — a vocabulary mismatch
between what the fingerprint claimed and what the dry-run could actually
record. All of those are permanently cut from the vocabulary, not
re-added under the rebuild.

**Carries forward with an expanded primitive set.** The original 8
primitives (`navigate`, `dom.read`, `dom.write`, `form.fill`,
`clipboard.read`, `clipboard.write`, `js.execute`, `download.file`) become a
larger set under the rebuilt `BrowserEngine` action surface (directive §8:
adds `dom.query`, `click`, `scroll`, `wait`, `tab.open`, `tab.close`,
`cookie.read`, `storage.read`, `screenshot`). The action-class × origin-scope
*model* is unchanged; the lowering table just gets more rows. A2/A4 own
updating the table; the exhaustiveness test requirement (directive §8) is
new discipline, not a decision reversal.

---

## ADR-002 — Closed attack-technique vocabulary for corpus authoring

**Date:** design fixed by commit `46b2177` (2026-07-03). **Status:** live, unchanged, feeds A11.

Two orthogonal, closed, multi-valued tag groups so two authors tag a case
identically:
- **Rhetorical** (what the injected text argues): `instruction_override`,
  `context_manipulation`, `social_engineering`, `goal_hijack`.
- **Concealment** (how the payload is hidden): `obfuscation`,
  `payload_splitting`, `plain`.

Every attack case carries ≥1 tag. Orthogonal to `carrier_vector` (ADR-006),
which is the *structural location*, not the rhetorical/concealment method.
No free text — closed vocabulary lives here, in one place, so it can't drift.

---

## ADR-003 — `js.execute` is unconditionally unscopable

**Date:** design fixed by commit `46b2177` (2026-07-03). **Status:** live, strengthened by the rebuild (D1/D2, A7).

`js.execute` can synthesize any other primitive invisibly past the
`ToolExecutor` boundary, so it can never be part of any capability's
expected realization, at any scope, ever. The comparator's rule is general
(*any primitive whose action class is unscopable is unconditionally a
deviation*), not a `js.execute` string special-case — the property lives on
the action class definition, not buried in comparator logic.

**Rebuild strengthens the encoding.** Pre-rebuild this was a runtime check
(`const UNSCOPABLE: &[&str] = &["js.execute"]` in `comparator.rs`). Directive
§8/A4 requires it be structurally unrepresentable — a separate
`ExpectedCapability` enum that simply has no variant capable of producing it
— so the invariant is enforced at compile time, not by remembering to check
a list at runtime. The decision (unscopable, always deviation, always
consent-gated) is unchanged; only the enforcement mechanism gets stronger.

---

## ADR-004 — Origin scope is authored, typed, and precedence-ordered

**Date:** design fixed by commit `46b2177` (2026-07-03). **Status:** live; implementation gap identified and scheduled for A7 (D1/D2).

`expected_origins` is authored per case as a typed scope, not a flat
allowlist, because a flat list either over-restricts (false positives on
legitimate result pages) or gets padded so wide it admits the attack:
- `exact` — specific known origins (tightest).
- `domain_suffix` — a bounded family (`*.wikipedia.org`).
- `task_open` — genuinely open browsing, requires a written rationale,
  flagged weak-scope for stratified reporting.

Specificity precedence for attribution: `exact` > `domain_suffix` >
`task_open`. Containment is reported **stratified by scope tightness** —
turning the "open-web scope is a soft spot" problem into a measured,
disclosed gradient instead of a hidden weakness.

**Known implementation gap, not a decision reversal:** the pre-rebuild
`compare()` took exactly one `OriginScope` for the *entire task*, so a
fingerprint mixing a narrow `scoped.read` and a wide `web.read` had no way
to give each its own scope — `admission_rank()`'s specificity math was
computed and never consumed. This is D1/D2 in the defect register; A7's job
is to make the comparator take **per-capability** scopes so this decision is
actually expressible, not just documented.

---

## ADR-005 — Two-layer dataset schema: CaseDefinition + ExecutionRecord

**Date:** design fixed by commit `46b2177` (2026-07-03), field set finalized same commit. **Status:** live, carried into A11.

A case is authored once (`CaseDefinition` — corpus, tier, author, carrier,
carrier_vector, attack_category, attack_techniques, `in_scope`, user_task,
expected_origins, ground_truth, ...) and run many times (`ExecutionRecord` —
one row per case × defense-mode, carrying the fingerprint used, the actual
event log, the computed diff, per-layer catch outcomes, final outcome,
timing, audit anchor). Every M1–M6-class metric is a filter/aggregate over
these two structs; a metric needing a field that's absent means re-running
experiments, hence pinning the schema before building the harness.

**Derived, never stored:** `unscopable_primitive_invoked` (pure function of
`extra_primitives`), `production_residual` (pure function of
`fingerprint_caught` + `consent_gated`). Storing either risks divergence
from the fields they're computed from — a general principle the rebuild
keeps (directive's metrics module, A12, computes these at query time too).

---

## ADR-006 — `carrier_vector` closed vocabulary + `GroundTruth` as a tagged enum

**Date:** design fixed by commit `65b6d67` (2026-08-25). **Status:** live; partition-enforcement gap identified and scheduled for A11 (D6).

`carrier_vector` records *where structurally* the payload rides, partitioned
by which carrier the case declares:
- `WebContent` (T1a): `hidden_element`, `offscreen_text`, `html_comment`,
  `alt_text`, `meta_content`, `css_pseudo`, `visible_text`.
- `ToolOutput` (T1b): `tool_json_field`, `tool_text_blob`,
  `tool_error_message`, `tool_metadata`.

`GroundTruth` is a four-variant tagged enum, not a flat struct with mostly-
null fields, because the shapes genuinely differ: `Deviation` (categories
1–4: primitive/origin the legit task didn't authorize), `WithinFingerprintOriginShift`
(category 5, origin-distinguishable — caught by per-origin attribution),
`WithinFingerprintDataOnly` (category 5, same-origin/same-primitive/data-only
— the irreducible residual, always `Missed` by the fingerprint layer by
construction), `None` (benign).

**Known implementation gap, not a decision reversal:** pre-rebuild, the
carrier/carrier_vector partition match was validated only at JSON-corpus-load
time (`corpus.rs::partition_matches`); a hand-built Rust `CaseDefinition`
literal could still violate it. D6 in the defect register; A11 moves this
into the type system (sealed constructor or carrier-specific enums) so it's
unrepresentable, not just checked.

---

## ADR-007 — Four-mode baseline protocol (Off / On / SanitizerOnly / LoopOnly), per-layer power read only from isolated modes

**Date:** design fixed by commit `46b2177` (2026-07-03, base 3-mode toggle); `LoopOnly` added same commit once the four-condition measurement design was settled. **Status:** live, carried into directive §13 headline metrics.

A defense number is meaningless without a baseline. Four committed
conditions, not one:
- **Off** — agent runs directly; measures whether the attack is real (M2/ASR baseline).
- **On** — full composed stack; the deployed-system number.
- **SanitizerOnly** — filter alone, loop bypassed; the filter's standalone power (M1a).
- **LoopOnly** — sanitizer bypassed, loop runs on raw content; the
  architecture's standalone power with content detection explicitly off —
  the cleanest evidence containment comes from architecture, not passive
  stripping (RQ1's most direct evidence).

**Per-layer power is read from the isolated modes, never from On** — in On
the sanitizer strips first, so the loop only ever sees what upstream missed,
which understates each layer if read as a standalone number. The two
deltas — `M1 − M1_loop` (sanitizer's marginal contribution) and `M1 − M1a`
(architecture's marginal contribution beyond passive stripping) — are
first-class reported results, not an afterthought.

**Known implementation gap, not a decision reversal:** pre-rebuild,
`adjudicate()`'s On-mode `final_outcome` ignored `sanitizer_caught` entirely
— once excision activates, a successfully-stripped attack would report as
`Executed` (attack succeeded), inverting the headline metric. D4 in the
defect register; A12 fixes the outcome lattice.

---

## ADR-008 — Credibility strategy: three independence layers + two structural safeguards

**Date:** design fixed by commit `46b2177` (2026-07-03). **Status:** live, unchanged, feeds A11 corpus construction.

Self-authored attacks + self-built defense is circular by default — a
reviewer can dismiss the result as graded homework. Three independence
layers, ordered by strength: (1) a teammate who didn't build the defense
authors a held-out slice; (2) an external professor slice, treated as
planned upside, not load-bearing (the plan must stand without it); (3) an
adapted external-benchmark slice (AgentDojo, Slack suite only — the one
suite whose threat model maps to a browser-tool surface without fabricating
capabilities).

Two hard rules make the independence real rather than decorative:
- **Held-out firewall** — the defense is tuned only against the
  self-authored set; independent slices run once, at the end, untouched. No
  peeking, no fixing the defense in response to what an independent slice
  exposes mid-development.
- **Briefing boundary** — independent authors get a threat-model brief
  (what the target is, what tools exist) but explicitly **not** a defense
  brief (how the fingerprint loop, sanitizer, or consent gate work). Knowing
  the mechanism unconsciously shapes attacks toward what the mechanism
  catches, degrading the independence the whole exercise exists to buy.

**Note for A11, per the amendment accepted alongside this ADR set:** the 10
existing pilot-corpus cases (`crates/ferrite-eval/tests/pilot_corpus/`) are
real, reusable assets and should seed the real corpus, but they are not
grandfathered in as already-valid: they get re-labelled under the ADR-006
`GroundTruth` enum as it exists post-rebuild, re-validated against the
type-level carrier partition (D6), and count toward the 10%
double-authoring / Cohen's κ ≥ 0.8 requirement in directive §13.3 like any
newly authored case.


---

## ADR-009 — Laya is an optional, confidence-gated accelerator for ordinary browsing steps; never part of the security boundary

**Date:** 2026-09-30, implemented by `7ba6d00`/`08152bb` (client + step decider), `ac1c872` (live-loop fast lane). **Status:** live, **off by default** (`FERRITE_LAYA_URL` unset = behaviour identical to before).

Laya ([NandhaKishorM/laya](https://github.com/NandhaKishorM/laya)) is a
non-autoregressive "System 1" decision model: typed `choice`/`score`/`noul`
answers in one forward pass (the authors report ~33 ms), no generation, so
nothing to parse or hallucinate. The question this ADR answers is *where in
this agent's pipeline that is, and is not, the right tool*.

| Stage | Decider | Why |
| --- | --- | --- |
| Fingerprint prediction, comparator, consent (the IPI defense) | LLM (small tier) + deterministic code — **never Laya** | ADR-000: the defense's premise is fail-to-empty on any model doubt. Laya's own "Honest limits" say base checkpoints are ~chance zero-shot on new label sets (0.362 vs 0.318 random on typed-decisions), are confidently wrong off-distribution, and can select the negated option (issue #377). A confident-but-wrong classifier in this position is a bypass, not an accelerator. |
| "Should the current page be context?" | Deterministic heuristic (`context::decide_page_use`) | Microseconds, auditable, 88-prompt table test. `decider::refine_page_use` exists but is deliberately **unwired**: it asks a question no head was trained on. |
| Next operation + target element on ordinary pages | **Laya browser head** (`cklxx/laya-browser`) as a gated fast lane, LLM fallback | The one documented fit: the authors report element top-1 0.66 among ~45 candidates, operation accuracy 0.88, 62% on 16 live tasks at 17–23 ms/step (their pages, their measurements; not reproduced here). Gated on operation *and* target probability, refuses to repeat the previous fast action, and any error/timeout/abstention falls through to the normal LLM step. |
| Text to type, final answers, summaries, extraction, planning, `ask_user` | LLM | Laya cannot generate text (its own browser-agent write-up uses a small LLM for `TYPE_TEXT` too). `TYPE_TEXT` = Laya picks the field, the small-tier model writes the value; a declined value falls back to the full LLM step. |
| `DONE` / `BLOCKED` | LLM | Laya's write-up states a `DONE` still needs independent outcome verification; the LLM writes the final answer anyway. |

**Consequences.** Fast-lane actions use the same vocabulary as LLM actions and
go through the identical rejection/consent/step-budget/repeat-stop path, so
Laya can steer *which* element a page-controlled label leads to but cannot
widen what is allowed. A request sends page URL, title, visible text and
element labels to `FERRITE_LAYA_URL`; the default is a loopback server and a
non-loopback URL logs a warning at startup. The locally served browser head
occupies laya-serve's `typed-decisions` router slot (only three names exist) —
a workaround documented in `scripts/laya/serve.py`. Gates (0.80 op / 0.60
target) are conservative **untuned** defaults; the authors also report that
confidence-gated escalation to a bigger LLM did *not* help on their pages, so
no claim is made here that gating improves outcomes. A real-trace A/B (fast
lane on vs off) must precede any default-on (T-234).

---

## ADR-010 — Agent memory and page context are untrusted data; only user-authored text may feed the fingerprint

**Date:** 2026-09-30, implemented by `d2f7454` (`context.rs`) and `b4157f5` (wiring). **Status:** live.

Multi-turn chats and page digests give the live loop far more context — and
far more attack surface: earlier agent answers and page text can carry
injected instructions, and persisting them into later prompts would let one
poisoned page shape every future run in the chat.

- The live loop's first message (`build_seed`) delimits **CONVERSATION SO
  FAR**, **OPEN TABS** and **CURRENT PAGE** as untrusted data, sanitizes and
  bounds every page-/agent-derived string, and never truncates the user
  request. Password values are never read into a digest.
- The IPI defense (`IpiTask`, sanitizer input, fingerprint prediction, the
  dry-run driver's prompt) receives **`trusted_task_text`** only: the new
  prompt plus up to five earlier *user* messages. Never the seed, page text,
  agent output or steps. A test enforces this structurally on the spawned
  defense task and another feeds it a hostile chat.
- The harness's per-step page observation (`observe_page`) is not an
  agent-initiated primitive: `DryRunEngine` implements it without logging a
  call. Logging it would put `dom.read` in nearly every dry-run record, flag a
  deviation on almost every task and train users to click through consent.
  The agent-initiated `read_page` goes through the logged `page_digest`.
- Typed text is not persisted in chat files (`@3 -> 7 chars`): they are plain
  text on disk and are re-read into later seeds.

**Known cost:** the dry run sees no page content and only prior user text, so
follow-ups such as "do the same for the second one" predict a looser
fingerprint and may raise more consent prompts. That is the fail-safe
direction and is accepted.

## ADR-011 — A separate model-activity trace, not the hash-chained audit log

**Date:** 2026-09-30. **Status:** live (`ferrite_model::trace`, the Audit panel's "Model calls" view).

The owner needs to see exactly what was sent to the LLM and to Laya, what came
back, and how long each took, to judge whether Laya is worth having. That is
a different record from the hash-chained audit log, which exists so the
*containment decision* is verifiable (capability granted/denied/exercised).

- **Where it is written.** One process-wide `TraceLog` (bounded ring of 500
  events, plus a JSON-lines file at `$FERRITE_HOME/logs/model-activity.jsonl`,
  rotated at 20 MB). LLM calls are recorded by the `Trace` provider decorator
  (outermost, so it sees what the caller waited for, cache hits included);
  every Laya request is recorded inside `LayaClient::systemone`; the UI adds
  the agent's own events (run start/finish, each executed action with its
  result and timing, consent requested/decided) and the Laya fast-lane
  *verdict* (accepted, or which gate sent it back to the LLM).
- **Why not the audit chain.** Prompts and page text are large, free-form and
  private, and do not fit the chain's fixed fields; putting them there would
  also make a log meant to be verifiable and shareable carry page content. The
  trace is explicitly local and every text field is cut to 6 000 characters.
- **Not in the security path.** Recording is best-effort (a write failure is
  ignored) and never influences a decision, admission or fingerprint. Nothing
  reads the trace back except the UI.
- **Labels.** `CompletionRequest::label` names a call's purpose (`agent step`,
  `fingerprint`, `field text`); it is not sent to any backend and not part of
  the cache key.
- **The "is Laya faster" answer** is computed from the events held
  (`activity::laya_effect_summary`): Laya round-trip mean vs LLM agent-step
  mean, times the number of steps whose Laya answer was actually used. It
  reports "no time saved" rather than a negative number when Laya is slower.

## ADR-012 — The browser profile persists under the data directory

**Date:** 2026-09-30. **Status:** live; Google sign-in specifically **unverified**.

`ServoBuilder` now gets `Opts::config_dir = $FERRITE_HOME/profile` (default
`~/.local/share/ferrite/profile`), so cookies, HSTS, cached HTTP credentials and
web storage survive a restart, and `dom_indexeddb_enabled` /
`dom_cookiestore_enabled` are on. Servo writes the cookie jar only when it
shuts down cleanly, so closing the window now drops every session and calls
`ferrite_servo::session::shutdown_engine()` before exiting; a crash, `kill`
or macOS Cmd+Q that bypasses the window close loses that run's new cookies.
Verified with `ferrite-servo/examples/profile_probe.rs` (a cookie set by a
response and a `localStorage` value both visible to a fresh process) and by
closing the real window. Whether a given site's login works is a different
question: Google in particular may refuse an embedded engine or a user-agent
that names Servo (`FERRITE_USER_AGENT` overrides it), and needs web-platform
features this Servo version may lack.
