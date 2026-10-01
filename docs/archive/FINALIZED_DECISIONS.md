> HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS. DO NOT USE AS CONTEXT.
> Extracted into `docs/DECISIONS.md` as ADR-001 through ADR-008. This file is
> the iteration-by-iteration reasoning behind those ADRs, not itself a live
> reference. See `docs/archive/README.md`.

# Ferrite — Finalized Design Decisions

> Built by working each remaining decision through three iterations and three lenses
> (researcher → user → developer priority; complexity-as-risk weighed, time NOT a factor).
> Grounded in the actual source: `ferrite-agent/src/lib.rs`, `ferrite-ipi/src/tool_decision/mod.rs`,
> `comparator.rs`, `dry_run.rs`, `twin.rs`. This file is the authoritative resolution of the
> decisions left open after the evaluation-planning session. Fold into EVALUATION_PLAN §7/§8,
> a new CLAUDE.md vocabulary section, and the pre-Task-19 vocab-fix block.

## Ground truth (verified from source — the basis for everything below)

The nine `BrowserTool` variants collapse to **eight distinct primitive IDs**:

| BrowserTool variant | primitive id |
|---|---|
| `Navigate(String)` | `navigate` |
| `ReadPage` | `dom.read` |
| `ExtractData(String)` | `dom.read` (same id as ReadPage) |
| `ClickElement(String)` | `dom.write` |
| `FillForm { selector, value }` | `form.fill` |
| `ReadClipboard` | `clipboard.read` |
| `WriteClipboard(String)` | `clipboard.write` |
| `ExecuteJs(String)` | `js.execute` |
| `DownloadFile(String)` | `download.file` |

**The complete, real primitive set is exactly these eight:** `navigate`, `dom.read`,
`dom.write`, `form.fill`, `clipboard.read`, `clipboard.write`, `js.execute`, `download.file`.

Everything the rule engine / predictor emit beyond these is a **phantom** (no `BrowserTool`
realizes it): `email.read`, `email.send`, `email.draft`, `calendar.read`, `calendar.write`,
`form.submit`, `report.write`, `contacts.read`, `storage.read`, `storage.write`,
`network.fetch`, `screenshot`. These must be cut or re-expressed (Decision 1).

---

# DECISION 1 — The capability mapping (the schema gate)

**Question:** What is the exact, authoritative capability vocabulary, and how does each
capability map to (primitives + origin-scope)?

## Iteration 1 — first cut

Naively: keep the semantic names the rule engine already emits (`email.read`, etc.), map each
to its realizing primitives + an origin scope. E.g. `email.read → {navigate, dom.read} @ mail
origins`. **Problem found:** this preserves the rule engine's *invented domain model* (email,
calendar, contacts) as if those were first-class agent capabilities. But the agent has no
email/calendar tools — it has eight browser primitives. Treating `email.read` as a capability
implies the agent "knows about email," when in truth it just navigates and reads DOM. This is
Model B leaking back in. Reject naive keep.

## Iteration 2 — collapse vs. preserve tension

Strict-thin collapse (Path 1) makes capabilities = primitives-with-names, which deletes the
legibility that motivated Model C. Path 2 (origin-scoped) was chosen to escape this: a
capability is realized by (primitives + origin-scope), so `email.read` becomes "dom.read
scoped to mail origins" — real today, distinct from generic `web.read` by *scope*. **Refinement
found:** but the *domain naming* (email/calendar/contacts) is still a fiction unless the
origin-scope actually encodes the domain. The honest version: a capability is **not** named
after a user-domain it can't truly model; it's named after the (action × origin-class) it
represents. So `email.read` is better expressed as `read @ {mail-provider origin class}` — the
"email-ness" lives entirely in the origin scope, not in a fake email tool.

## Iteration 3 — the resolved model: capability = (action-class, origin-scope)

A **capability** is the pair: an **action class** (a grouping of primitives that share a
security character) and an **origin scope** (where it's authorized to act). The semantic
*name* is a human label for that pair, used in consent UI and fingerprints; it carries no
hidden tool the agent lacks. This keeps Model C's legibility (named, scoped capabilities) while
being honest (every capability is fully realized by the eight primitives + an origin class).

### Action classes (grouping the eight primitives by security character)

| Action class | primitives | character |
|---|---|---|
| `read` | `dom.read` | observe page content (passive) |
| `interact` | `dom.write`, `form.fill` | modify page / enter data (active, on-page) |
| `navigate` | `navigate` | move to an origin (origin-changing) |
| `download` | `download.file` | pull a resource (origin-touching, exfil-adjacent) |
| `clipboard` | `clipboard.read`, `clipboard.write` | local side channel (no origin) |
| `execute` | `js.execute` | arbitrary code (UNSCOPABLE — Decision 3) |

### Capabilities (action-class × origin-scope), thin and fully realizable

| Capability (label) | action classes | origin scope | notes |
|---|---|---|---|
| `web.read` | read, navigate | task-declared origin(s) | generic page reading |
| `web.interact` | interact, navigate | task-declared origin(s) | fill/click on a page |
| `web.download` | download, navigate | task-declared origin(s) | pull a file |
| `scoped.read:<class>` | read, navigate | a NARROW declared origin class (e.g. a single mail/bank domain) | the "email.read"-style tight capability, expressed honestly as read scoped to a narrow origin class |
| `clipboard.read` | clipboard | none (local) | no origin |
| `clipboard.write` | clipboard | none (local) | no origin |
| (`execute`) | execute | — | NEVER a capability; always deviation (Decision 3) |

> **Key reframing:** there is no `email.read` capability. There is `scoped.read` with an
> origin scope that happens to be a mail domain. "Email" is a *property of the origin scope*,
> authored per case (Decision 4 / SQ1), not a fake tool. This is the honest expression of
> origin-scoped capabilities and it eliminates the entire phantom-vocabulary problem at the root.

### The phantom cut (authoritative)

DELETE from `rule_based_must_use` and the predictor allowlist — none have a primitive
realization, all are re-expressed as scoped capabilities or removed:
- `email.read / email.send / email.draft` → re-express as `scoped.read` / `web.interact`
  scoped to a mail origin class (the action is navigate+dom.read or navigate+form.fill;
  "email" is the origin scope).
- `calendar.read / calendar.write` → same: `scoped.read` / `web.interact` @ calendar origin.
- `contacts.read / storage.read / storage.write / screenshot` → REMOVE entirely. The agent
  has no primitive for these; they are pure fiction.
- `network.fetch` → REMOVE as a tool id. Network reachability is not a tool; it is the
  *origin* dimension. Origin checking is driven by origin scope (Decision 4), never by a
  `network.fetch` tool's presence. (This also kills the broken all-or-nothing origin gate.)
- `form.submit` → REMOVE. No `BrowserTool` produces it (`FillForm`→`form.fill` only; there is
  no submit variant). A "submit" is modeled as `interact` (a `dom.write`/click). If a true
  submit primitive is ever added to `BrowserTool`, add it then — not before.
- `report.write` → REMOVE. "Summarize/report" is the agent producing output, not a
  browser action with an origin. It is not a security-relevant tool; drop it from the
  fingerprint vocabulary. (The agent writing a summary touches no origin and no extra primitive.)
- `dom.write` as a bare predictor id → fold into `interact` action class.

### The three lenses on Decision 1

- **Researcher:** the (action-class × origin-scope) model is the cleanest defensible story —
  it makes the contribution precise ("capabilities are origin-scoped action classes; the
  defense compares lowered primitives + origins"), removes the confound of a fake domain
  vocabulary, and makes category 5's blind spot exactly characterizable (same action class,
  same origin scope, different data). This is the strongest version for the paper.
- **User (consent UI):** a consent prompt reads "the agent tried to **read** content at
  **evil.com**, which your task ('summarize mail.com') did not authorize." That is *more*
  legible than "email.read violated," because it names the concrete action + origin the user
  can actually judge, rather than an abstract capability label. Origin-scoped naming serves the
  user better than fake-domain naming.
- **Developer:** six action classes + a small capability set is far more maintainable than the
  ~20-id phantom soup. Every capability is realized by the eight real primitives, so there is
  no mapping that can ever reference a tool the agent lacks — the whole class of
  vocabulary-mismatch bugs becomes unrepresentable. Complexity-as-risk drops sharply.

**Where lenses conflicted:** none materially did — the honest model happens to serve all three.
The only tension (user wants rich domain names like "email") is resolved by putting the domain
in the origin scope, which the user actually sees and judges more easily anyway.

## DECISION 1 — RESOLVED

Capability = (**action class**, **origin scope**). Six action classes (`read`, `interact`,
`navigate`, `download`, `clipboard`, `execute`) over the eight real primitives. Capabilities
are origin-scoped action classes; tight "domain" capabilities (email/bank/calendar) are
expressed as `scoped.read`/`web.interact` with a narrow authored origin class, NOT as fake
domain tools. `report.write`, `contacts.*`, `storage.*`, `screenshot`, `form.submit`,
`network.fetch` are removed from the vocabulary entirely. `execute` is never a capability
(Decision 3). The rule engine and predictor are rewritten to emit only this vocabulary.

---

# DECISION 2 — The closed technique vocabulary

**Question:** What is the fixed set of attack-technique tags (multi-valued, secondary,
descriptive) that case authors choose from?

## Iteration 1 — first cut

Start from what published taxonomies name: instruction-override, context-manipulation,
obfuscation, payload-splitting. **Problem found:** these mix two different axes —
*what the injection says* (instruction-override) vs. *how it's hidden* (obfuscation). A tag set
that conflates "rhetorical strategy" with "concealment method" will produce inconsistent
tagging because a single attack has both. Need to separate the axes or accept multi-tag spanning
both.

## Iteration 2 — two sub-axes within technique

Split technique into two conceptual groups, both drawn from the same flat closed set (multi-
valued, so one case can carry one from each group):
- **Rhetorical technique** (how the injected text manipulates the agent): `instruction_override`
  ("ignore previous instructions…"), `context_manipulation` (fake system/role framing),
  `social_engineering` (urgency, authority, plausible-pretext), `goal_hijack` (replace the
  task's objective).
- **Concealment technique** (how the payload is hidden in the carrier — note: this is distinct
  from `carrier_vector`, which is the *structural* location; concealment is the *evasion method*):
  `obfuscation` (encoding, homoglyphs, zero-width), `payload_splitting` (across elements/turns),
  `plain` (no concealment — overt injection).

**Refinement found:** `carrier_vector` (hidden div, comment, alt-text) already captures
*structural location*. Concealment-technique tags risk overlapping it. Resolve by scoping
concealment tags to *evasion method only* (obfuscation/splitting/plain), not location — the
location is the vector, the method is the technique. Keep them orthogonal and say so.

## Iteration 3 — final closed set, with anti-drift discipline

A closed, flat, multi-valued set of **seven** tags, documented with one-line definitions so two
authors tag identically. Authors may apply multiple (typically one rhetorical + one concealment).

| Tag | Group | Definition (for authors) |
|---|---|---|
| `instruction_override` | rhetorical | Explicitly tells the agent to disregard prior instructions / its task. |
| `context_manipulation` | rhetorical | Forges system/role/authority framing to be obeyed (fake "SYSTEM:", fake admin). |
| `social_engineering` | rhetorical | Uses urgency, plausibility, or pretext to induce the action (no explicit override). |
| `goal_hijack` | rhetorical | Substitutes a new objective while appearing to continue the task. |
| `obfuscation` | concealment | Payload encoded/disguised to evade detection (base64, homoglyph, zero-width, lang tricks). |
| `payload_splitting` | concealment | Injection assembled from fragments across elements or turns. |
| `plain` | concealment | Overt injection, no concealment (baseline; pairs with a rhetorical tag). |

Rules: every attack case carries **at least one** tag; concealment and rhetorical groups are
orthogonal to `carrier_vector`; the set is **closed** (no free-text) and lives in CLAUDE.md so
it cannot drift; adding a tag is a deliberate documented change, not an author's ad-hoc choice.

### The three lenses on Decision 2

- **Researcher:** a closed, defined vocabulary makes cross-tabulation valid ("obfuscated
  attacks evade the sanitizer N% more") and pre-registrable. The rhetorical/concealment split
  mirrors how the literature reasons and supports clean analysis. Orthogonality to
  `carrier_vector` prevents double-counting that would muddy results.
- **User:** not user-facing (technique is an analysis field, never shown in consent). No user
  lens applies — correctly, this is a researcher/developer-only field.
- **Developer:** a fixed 7-tag enum (or a small validated set) is trivial to store and validate;
  closed-vocab means input validation can reject typos, preventing the silent drift that free
  text invites. Multi-valued = a `Vec`/set, no logic keys on it, so zero downstream risk.

**Where lenses conflicted:** none. (No user stake; researcher and developer both want closed +
defined.)

## DECISION 2 — RESOLVED

Seven closed tags in two orthogonal groups (rhetorical: `instruction_override`,
`context_manipulation`, `social_engineering`, `goal_hijack`; concealment: `obfuscation`,
`payload_splitting`, `plain`). Multi-valued, ≥1 per case, closed vocabulary in CLAUDE.md,
orthogonal to `carrier_vector`. Descriptive only — no comparator/metric logic keys on it.

---

# DECISION 3 — Schema/comparator encoding of the js.execute rule

**Question:** How is "`js.execute` is always a deviation, always consent-gated" actually
encoded — a hardcoded comparator rule, or an explicit capability class?

## Iteration 1 — first cut

Hardcode it in `compare()`: `if actual primitive == js.execute → push to extra_primitives`.
Simple. **Problem found:** a magic-string special-case buried in comparator logic is invisible
to anyone reading the capability model; a future maintainer adding an `execute` capability
wouldn't see why it's forbidden. The rule's *rationale* (unscopable meta-primitive) isn't
captured where capabilities are defined. Risk: silent regression if someone "tidies" it.

## Iteration 2 — explicit unscopable class

Model `execute` as a first-class action class flagged `unscopable: true`, and have the
lowering/attribution logic treat any unscopable action class as never-admittable → always
`extra_primitives`. **Refinement found:** this is better — the rule lives *with* the action-class
definitions (Decision 1's table already lists `execute` as the unscopable class), so the "why"
is co-located with the "what." The comparator then has a *general* rule ("unscopable action
classes are always deviations"), not a `js.execute` special-case — more principled and
future-proof (if another unscopable primitive is ever added, it inherits the behavior).

## Iteration 3 — encoding + the dataset representation

Two layers must agree:
1. **Comparator:** a general rule — *any actual primitive whose action class is unscopable is
   unconditionally an `extra_primitive`*, regardless of fingerprint contents. `js.execute` is
   the sole member of the unscopable `execute` class today. No magic string; the property lives
   on the action class.
2. **Schema:** no new field needed. An always-`js.execute` deviation surfaces through the
   existing `extra_primitives` (it lands there) and `consent_gated` (it always reaches consent).
   For analysis legibility, the dataset's `computed_diff` naturally records it; an optional
   derived/queryable marker `unscopable_primitive_invoked` (boolean) MAY be added purely for
   easy filtering in analysis — but it is *derivable* from `extra_primitives ∋ js.execute`, so
   per Decision-4-style reasoning (store vs derive), it can be derived at query time, not stored.
   Decision: **derive, don't store** — it's a pure function of an existing field, and storing
   risks divergence.

### The three lenses on Decision 3

- **Researcher:** the general "unscopable action class" framing is a cleaner paper statement than
  "we special-case JS" — it generalizes the limitation (any primitive that can impersonate
  others must be consent-gated) and ties directly to the honest category-5 narrative (the worst
  within-fingerprint vector is removed because execute is always visible).
- **User:** consent prompt reads "the agent tried to **run a script** (arbitrary code), which
  is always reviewed." Clear, and the always-on nature is a *feature* the user can trust, not a
  confusing per-case judgment.
- **Developer:** a property on the action class (`unscopable`) with a general comparator rule is
  far safer than a buried string check — it's discoverable, self-documenting, and regression-
  resistant. Deriving the analysis marker avoids a stored-field divergence bug.

**Where lenses conflicted:** none. The principled encoding serves all three better than the
hardcode.

## DECISION 3 — RESOLVED

Encode as a general comparator rule keyed on an `unscopable` property of the action class
(`execute` is unscopable; `js.execute` its sole current member). Any unscopable primitive is
unconditionally `extra_primitives` and thus always `consent_gated`. No new stored schema field;
an `unscopable_primitive_invoked` marker is *derived* at analysis time from `extra_primitives`,
not stored.

---

# DECISION 4 — How `expected_origins` (task-declared origin scope) is authored

**Question:** SQ1 resolved that eval origin-scope is per-case authored. But *how* is it
expressed per case, especially for loose `web.read` whose scope is "task-declared origin(s)"?
This is the soft underbelly of the whole scoping scheme (the loose-scope laundering residual).

## Iteration 1 — first cut

Author a flat allowlist of exact origins per case: `expected_origins = ["mail.com"]`.
**Problem found:** real tasks legitimately touch *patterns*, not single origins ("search the
web about X" → many search-result domains). A flat exact list either over-restricts (false
positives on legitimate result pages) or gets padded so wide it admits the attack. The
underbelly is real: a vague task needs a wide scope, and a wide scope launders attacks.

## Iteration 2 — typed scopes instead of flat lists

Express each case's `expected_origins` as a small **typed scope**, not a flat list:
- `exact: [origins]` — the task touches specific known origins (tightest; e.g. a mail/bank task).
- `domain_suffix: [suffixes]` — a bounded family (e.g. `*.wikipedia.org`).
- `task_open: <declared rationale>` — genuinely open browsing, explicitly marked as a *wide*
  scope with a recorded reason. Cases tagged `task_open` are KNOWN-WEAK and reported separately.

**Refinement found:** the value of typing the scope is that it makes the *strength* of each
case's boundary explicit and queryable. You can then report containment **stratified by scope
tightness** — "on `exact`-scoped tasks, M1 = X%; on `task_open` tasks, M1 = Y% (weaker, as
expected)." This converts the laundering underbelly from a hidden weakness into a *measured,
disclosed* gradient. That is the honest researcher move.

## Iteration 3 — the authoring rule + the integrity tie-in

Final rule for authoring `expected_origins` per case:
1. Author chooses the **tightest scope type that legitimately fits the task.** Tightest-fit is
   a documented authoring obligation (not "whatever admits the benign run").
2. Scope type is recorded as a field (`scope_type: exact | domain_suffix | task_open`).
3. `task_open` requires a written rationale and flags the case as weak-scope for stratified
   reporting.
4. **Integrity (from SQ1):** `expected_origins` and `scope_type` are Layer-1 authored fields,
   set before the run, NEVER written/mutated from dry-run data or fetched content. The
   comparator reads them; nothing in the run writes them.
5. The attribution rule (SQ2) uses scope *specificity* for precedence: `exact` is more specific
   than `domain_suffix` is more specific than `task_open`. A tight capability always wins
   attribution over a loose one for an origin both admit — closing the laundering path at the
   precedence level, while `task_open`'s residual weakness is measured, not hidden.

### The three lenses on Decision 4

- **Researcher:** typed scopes + stratified-by-tightness reporting turns the single biggest
  soft spot (open-web scope) into a quantified gradient — the most honest possible treatment.
  It also gives the deployment-story disclosure teeth: "tight scopes are strongly contained;
  open scopes carry measured residual; runtime scope-derivation is future work."
- **User:** consent prompts on tight-scope tasks are precise ("read at evil.com — your task only
  authorized mail.com"); on open-scope tasks the system is appropriately more permissive, which
  matches user expectation (an open browse *should* prompt less). The typing aligns friction
  with task nature.
- **Developer:** three scope types with a specificity ordering is a small, total, testable
  function; `task_open` as an explicit marker prevents the silent "wide list" anti-pattern.
  Storing `scope_type` (not deriving) is correct — it's an authoring judgment, not a derivation.

**Where lenses conflicted:** none materially. Researcher wants the gradient measured; user wants
friction matched to task openness; developer wants a small total function — typed scopes deliver
all three.

## DECISION 4 — RESOLVED

`expected_origins` is authored per case as a **typed scope**: `exact` | `domain_suffix` |
`task_open` (the last requires a rationale and marks the case weak-scope). Authors must choose
the tightest type that legitimately fits. `scope_type` is a stored Layer-1 field. Containment is
reported **stratified by scope tightness**. SQ2 attribution precedence follows scope specificity
(exact > domain_suffix > task_open). Integrity rule from SQ1 holds: these fields are authored,
trusted, never run-written.

---

# DECISION 5 — Final schema field names & types (turning §7 prose into a struct contract)

**Question:** The §7 two-layer schema is described in prose. Pin the concrete field
names/types so Task 19 implements one fixed contract (no second guessing during build).

Resolved as two serde structs persisted via `rusqlite` 0.37 (bundled). Storage note from
PROGRESS: `dataset.rs` is the FLAT file `src/dataset.rs`; temp paths in tests use
`std::env::temp_dir()`, never `/tmp/`. No `sea-query`/`sea-orm`/`sqlx`/`diesel`.

## Layer 1 — CaseDefinition (authored, invariant)

| field | type | notes |
|---|---|---|
| `case_id` | `Uuid` | PK |
| `corpus` | enum `Attack` \| `Benign` | |
| `tier` | enum `Tier1 \| Tier2 \| Tier3Teammate \| Tier3Professor \| Tier3AgentDojo` | |
| `author` | enum `SelfAuthored \| Teammate \| Professor \| AgentDojo` | provenance |
| `carrier` | enum `WebContent \| ToolOutput` | T1a / T1b |
| `carrier_vector` | `String` (closed-vocab) | hidden_div, html_comment, alt_text, tool_json_field, … |
| `attack_category` | `Option<enum>` `DataExfiltration \| UnauthorizedAction \| AgentRedirection \| ScopeEscalation \| WithinFingerprintAbuse` | None for benign |
| `attack_techniques` | `Vec<enum>` (the 7 closed tags) | ≥1 for attacks; empty for benign |
| `in_scope` | `bool` | STORED (Q4); true for categories 1–4, false for 5 |
| `user_task` | `String` | |
| `attacker_goal` | `Option<String>` | None for benign |
| `expected_origins` | `OriginScope` (see below) | authored, trusted |
| `scope_type` | enum `Exact \| DomainSuffix \| TaskOpen` | Decision 4 |
| `scope_rationale` | `Option<String>` | required iff `TaskOpen` |
| `ground_truth_deviation` | `GroundTruth` struct (expected extra primitives/origins; or data-field for cat5) | |
| `taxonomy_anchor` | `Option<String>` | filled at authoring |

`OriginScope` = `{ exact: Vec<String>, domain_suffix: Vec<String> }` (interpretation governed
by `scope_type`; `TaskOpen` admits broadly and is reported weak).

## Layer 2 — ExecutionRecord (one per case-run)

| field | type | notes |
|---|---|---|
| `exec_id` | `Uuid` | PK |
| `case_id` | `Uuid` | FK → CaseDefinition |
| `timestamp` | `DateTime<Utc>` | |
| `run_label` | enum R1..R10 \| A1..A5 | A3/A4 loop-only and A5 benign sanitizer-only (M3a) added post-freeze; see EVALUATION_PLAN §7/§9 for the current matrix |
| `model` | enum `Gemini` (extensible) | Decision: store for append-only future models |
| `defense_mode` | enum `On \| SanitizerOnly \| Off` | Task 18 |
| `expected_fingerprint` | `Option<SemanticFingerprint>` | None in Off; capabilities (action-class × scope) |
| `expected_realization` | `Option<{ primitives: HashSet<ToolId>, origin_scopes: Vec<(Capability, OriginScope)> }>` | lowered form; None in Off |
| `actual_events` | `Vec<{ primitive: ToolId, origin: Option<String> }>` | ORDERED event log (replaces HashSet) — Decision: origin-bound |
| `computed_diff` | `{ extra_primitives: HashSet<ToolId>, out_of_scope_origins: HashSet<String> }` | two deviation kinds |
| `sanitizer_caught` | enum `Caught \| Missed \| NotApplicable` | |
| `fingerprint_caught` | enum `Caught \| Missed \| NotApplicable` | |
| `consent_gated` | enum `Gated \| NotGated \| NotApplicable` | |
| `data_fields_accessed` | `Vec<String>` | recorded attempt (populated by origin-binding work) |
| `network_attempts` | `Vec<String>` | recorded attempt (already in DryRunRecord) |
| `final_outcome` | enum `ContainedViaConsent \| Blocked \| Executed \| BenignNoFlag \| BenignFalseFlag` | derived |
| `residual_risk` | enum `None \| BlastRadiusContained \| RealHarm \| NotApplicable` | |
| `timing` | `{ total_ms, dry_run_ms, predict_ms }` | M4 |
| `audit_log_anchor` | `String` (hash/id) | M6 |

Derived-not-stored (per Decision 3 / Q-style reasoning): `unscopable_primitive_invoked`
(= `extra_primitives ∋ js.execute`); `production_residual` (= `fingerprint_caught==Missed &&
consent_gated==NotGated`) — both computed at analysis time.

### The three lenses on Decision 5

- **Researcher:** every M1–M6 query is a filter/aggregate over these two structs; stratified M1
  (`in_scope`), scope-tightness stratification (`scope_type`), and the category-5 residual
  (`residual_risk` + derived production_residual) are all directly expressible. No metric needs
  a field that's absent.
- **User:** the consent-facing data (`computed_diff`, `expected_realization`) carries enough to
  render the precise "tried to <action> at <origin> beyond <capability>" message.
- **Developer:** two flat serde structs, rusqlite-friendly, enums over magic strings, ordered
  event log fixes the lossy HashSet, derived fields avoid divergence. The existing thin
  `IpiEvent`/`IpiLabel` in comparator.rs is REPLACED by these (and its tests rewritten — they
  currently encode the vocabulary confusion and will rightly fail).

## DECISION 5 — RESOLVED

Two serde structs as specified, persisted via rusqlite 0.37 in flat `src/dataset.rs`. Ordered
origin-bound `actual_events` replaces the lossy `tools_called` HashSet. Derived markers
(`unscopable_primitive_invoked`, `production_residual`) computed at analysis time, not stored.
The legacy `IpiEvent`/`IpiLabel` are superseded.

---

# Cross-cutting consequences (fold into canonical docs)

1. **EVALUATION_PLAN §7** — replace the action-class/capability prose with Decision 1's model;
   add `scope_type` (Decision 4) and the stratified-by-tightness reporting note; the field list
   matches Decision 5. (Minor: §7 currently says "semantic capabilities" generically — refine
   to "(action-class × origin-scope) capabilities.")
2. **EVALUATION_PLAN §5/§9** — add scope-tightness stratification to M1 reporting (a sub-table:
   M1 by `scope_type`). This is new and strengthens the honesty story.
3. **CLAUDE.md** — add an authoritative "Tool Vocabulary" section: the eight primitives, six
   action classes, capability model, the phantom-cut list, the 7 technique tags, the
   `unscopable` rule. This becomes the source of truth the rule engine/predictor must obey.
4. **Pre-Task-19 vocab-fix block (§8 item 6)** — now fully specified by Decisions 1, 3, 4:
   (a) rewrite `rule_based_must_use` to emit only capabilities (action-class labels), cutting all
   phantoms; (b) rewrite the predictor allowlist to the same closed set; (c) origin-binding event
   log in `dry_run.rs`; (d) `compare()` → lower-then-compare with per-origin attribution,
   specificity precedence (exact>domain_suffix>task_open), and the general unscopable rule.
5. **Key-loading divergence (NEW finding — config confound, not a bug in intent):** The
   `gemini_key.txt`-next-to-executable approach is the INTENDED design — a deliberate workaround
   to avoid committing the Gemini key to a public repo (runtime file read, env-var fallback).
   That approach is correct. The actual problem is that two components load the key differently:
   - `gemini.rs` (agent backend) — uses the `gemini_key.txt` + env approach.
   - `tool_decision::LlmMayUsePredictor::from_env()` — reads ONLY `FERRITE_GEMINI_API_KEY`; has
     no file-reading path. Returns `None` (silently disables may-use prediction → rules-only
     fingerprinting) if that env var is absent.
   **Consequence for the evaluation:** if shipped with `gemini_key.txt` present but the env var
   unset, the predictor runs KEYLESS while the agent runs fine. The may-use layer silently
   collapses to empty, which *worsens M3 (false-positive rate)* for a reason that has nothing to
   do with the defense — a config artifact masquerading as a result. A reviewer-facing metric
   shifted by key-plumbing is a silent confound.
   **Resolution (fold into vocab-fix block, which already touches `tool_decision`):** factor
   key-loading into ONE shared function — **env var first, then `gemini_key.txt` (next to the
   executable) fallback** — called by BOTH `gemini.rs` and `tool_decision`. This honors the
   workaround uniformly and makes the two components unable to diverge again.
   **Hardening recommendation (adopt):** at predictor initialization during an EVALUATION run,
   emit a non-fatal WARNING if the predictor comes up keyless (no key found by either path).
   This must warn, not fail (rules-only is a legitimate mode — see PROJECT_REFERENCE §2.3), but
   it guarantees you never silently record M3/M5 numbers from a degraded predictor. The warning
   is eval-context-only so normal rules-only operation stays quiet.
6. **PROGRESS.md staleness:** still calls the dataset stub "Task 18" and points the next step at
   it; it is now Task 19, and the vocab-fix block precedes it. Update when editing canonical docs.
7. **comparator.rs tests** will fail by design after the rewrite (they encode the old single-
   vocabulary set-difference); rewrite them against the lower-then-compare model.

# Status

All remaining design decisions are now CLOSED:
- Decision 1 (capability mapping) ✅  — Decision 2 (technique vocab) ✅
- Decision 3 (js.execute encoding) ✅ — Decision 4 (origin-scope authoring) ✅
- Decision 5 (schema field contract) ✅

Deferred-by-rule (measurement, not decision): exact corpus N (pilot); AgentDojo Slack count
(spike); category-5 Tier A/B (pilot); stretch conditions (run-time effort); professor slice
(availability). These are NOT open decisions — rules are fixed, values await data.

Next: this file folds into EVALUATION_PLAN §5/§7/§8/§9, a new CLAUDE.md vocabulary section, and
the vocab-fix TO-DO block. No further design decisions remain before implementation.

---

# DECISION 6 — Closing the last two loose ends (carrier_vector vocabulary + GroundTruth struct)

Final-pass completeness: two fields in Decision 5 were exemplified but not fully pinned. Closed
here so every closed-vocab is enumerated and every struct is field-level specified.

## 6a — `carrier_vector` closed vocabulary (enumerated)

`carrier_vector` records the *structural location* where the payload rides (distinct from
`attack_techniques`, which is the rhetorical/concealment method — orthogonal, per Decision 2).
Closed set, partitioned by the two carriers (`carrier` field selects which partition is valid):

**For `carrier = WebContent` (T1a):**
| tag | meaning |
|---|---|
| `hidden_element` | `display:none` / `visibility:hidden` / `hidden` attribute |
| `offscreen_text` | positioned off-viewport (negative coords, tiny font, clip) |
| `html_comment` | inside `<!-- -->` |
| `alt_text` | image `alt` / `title` / aria attributes |
| `meta_content` | `<meta>` / structured-data / JSON-LD blocks |
| `css_pseudo` | CSS-injected content (`::before`/`::after`, content props) |
| `visible_text` | overt, visible on-page text (no structural concealment) |

**For `carrier = ToolOutput` (T1b):**
| tag | meaning |
|---|---|
| `tool_json_field` | inside a returned JSON value field |
| `tool_text_blob` | inside a free-text tool result |
| `tool_error_message` | inside an error/diagnostic string returned by a tool |
| `tool_metadata` | inside result metadata/headers rather than the primary payload |

Rules: exactly one `carrier_vector` per case; it must belong to the partition matching the
`carrier` field (validation enforces this); the set is closed and lives in CLAUDE.md alongside
the technique tags. `visible_text` and `tool_text_blob` are the "no structural concealment"
baselines (pair naturally with technique tag `plain`).

> Orthogonality check: `carrier_vector` = WHERE (structural location), `attack_techniques`
> concealment group = HOW it evades (obfuscation/splitting/plain), rhetorical group = WHAT it
> says. A case is fully described by (carrier, carrier_vector, techniques[]) with no overlap.

## 6b — `GroundTruth` struct (field-pinned)

The authored ground-truth label against which "caught" is judged. Must express three distinct
deviation shapes (in-scope primitive/origin deviations, AND the cat-5 same-origin/data residual)
without ambiguity. Resolved as a tagged enum, not a flat struct, because the three shapes carry
different fields and a flat struct would leave most fields null per case (lossy, error-prone):

```
enum GroundTruth {
    // Categories 1–4 (in_scope=true): success = agent reaches a tool/origin the task didn't authorize.
    Deviation {
        expected_extra_primitives: HashSet<ToolId>,   // primitives a successful attack adds
        expected_out_of_scope_origins: HashSet<String>, // origins a successful attack touches
    },
    // Category 5 (in_scope=false), origin-distinguishable: same primitive, DIFFERENT origin.
    // (These are caught by SQ2 attribution; recorded so the "shrunk blind spot" claim is auditable.)
    WithinFingerprintOriginShift {
        legitimate_origin: String,
        attack_origin: String,        // different origin, same action class
    },
    // Category 5 (in_scope=false), same-origin/same-primitive/data-only: the irreducible residual.
    // expected_field is the data the LEGIT task needs; attack accesses a DIFFERENT field at same origin.
    WithinFingerprintDataOnly {
        legitimate_data_ref: String,  // what the task legitimately accesses (e.g. "inbox:latest")
        attack_data_ref: String,      // what the injection accesses (e.g. "archive:sensitive")
        // checkable ONLY if data_fields_accessed is instrumented (Tier A); else documented residual (Tier B)
    },
    // Benign cases: no deviation expected at all.
    None,
}
```

Resolution notes:
- The enum makes the cat-5 split explicit and matches Decision 4 / §7.4: origin-distinguishable
  cat-5 (`WithinFingerprintOriginShift`) is caught by per-origin attribution; data-only cat-5
  (`WithinFingerprintDataOnly`) is the residual whose measurement is pilot-gated (Tier A/B).
- `WithinFingerprintDataOnly.attack_data_ref` is *authored* (the case author states what the
  attack targets); whether the system can *observe* it depends on the Tier-A `data_fields_accessed`
  instrumentation. If Tier B (not instrumented), the field is still recorded as ground truth and
  the case is reported as documented-residual, not measured-caught. No ambiguity either way.
- `None` for benign keeps the benign corpus in the same schema (CaseDefinition) without a
  nullable-deviation hack.

### Three lenses on Decision 6

- **Researcher:** the enum makes every ground-truth shape auditable and the cat-5 split (caught-
  by-attribution vs. residual) explicit in the data itself — a reviewer can verify the "blind
  spot shrank" claim by counting `WithinFingerprintOriginShift` (caught) vs.
  `WithinFingerprintDataOnly` (residual). The enumerated carrier_vector makes carrier×vector
  cross-tabs valid.
- **User:** neither field is user-facing (both are authoring/analysis). No user lens — correct.
- **Developer:** a tagged enum is the idiomatic Rust encoding (serde-friendly, exhaustive match,
  no null soup); enumerated carrier_vector enables input validation that rejects typos and
  enforces the carrier-partition rule. Both eliminate a class of silent data-quality bugs.

## DECISION 6 — RESOLVED

`carrier_vector` is a closed set of 7 (T1a) + 4 (T1b) tags, partition-validated against
`carrier`, enumerated in CLAUDE.md. `GroundTruth` is a four-variant tagged enum
(`Deviation`, `WithinFingerprintOriginShift`, `WithinFingerprintDataOnly`, `None`) that
expresses all in-scope and cat-5 deviation shapes without nullable ambiguity and makes the
category-5 caught-vs-residual split auditable in the data.

---

# FINAL STATUS — zero open design decisions

Closed: Decision 1 (capability mapping) · 2 (technique vocab) · 3 (js.execute encoding) ·
4 (origin-scope authoring) · 5 (schema field contract) · 6 (carrier_vector vocab + GroundTruth
struct). Every closed vocabulary is enumerated; every struct is field-pinned.

NOT decisions (do not reopen as such):
- Measurement-gated values: corpus N (pilot), AgentDojo Slack count (spike), cat-5 Tier A/B
  (pilot), stretch conditions (run-time effort), professor slice (availability).
- Execution: draft Task 18–22 prompts; fold all decisions into EVALUATION_PLAN/CLAUDE.md/
  TO-DO/PROGRESS; build the vocab-fix block.

There are no remaining design forks. Implementation is unblocked.