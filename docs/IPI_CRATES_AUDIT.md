# Full audit of the IPI crates: leakage and anti-generalisation

- **Branch:** `web-interact-fix`.
- **Crates audited in full:**
  - `ferrite-core`, the taxonomy and scopes the defense is built on;
  - `ferrite-ipi`, the defense;
  - `ferrite-eval`, which produces every number.
- **Out of scope:** the UI, agent, engine and shell (the live demo). They are
  mentioned only where an IPI-crate API is consumed there.

This is the long list. The two earlier documents are summaries:
[`IPI_EVALUATION_REVIEW.md`](IPI_EVALUATION_REVIEW.md) is the leakage audit and
ablation, and [`IPI_GENERALISATION_REVIEW.md`](IPI_GENERALISATION_REVIEW.md) is
the action list. Items repeated from those documents are cross-referenced here,
not re-argued.

## How to read this

Each item has a **type**:

| type | meaning |
|---|---|
| **LEAK** | Benchmark or ground-truth information reaches the defense, or the labels are coupled to the defense, so the numbers look better than a deployment would be. |
| **CIRCULAR** | The measurement uses the defense's own logic to produce its "independent" input, so a result holds by construction. |
| **METRIC** | The scoring rewards strictness, or hides utility loss. |
| **STRICT** | The defense refuses normal tasks for reasons that do not generalise beyond its test set. |
| **LOOSE** | The defense admits more than the paper would claim. This matters because a looser policy is often what was tuned to recover utility. |
| **DEAD** | Code in the defense crate that nothing in the IPI pipeline uses, or that is misleading about what the defense does. |

Each item also has a **severity**, rated for the paper: **Critical** changes a
headline claim, **High** changes a number or a design claim, **Medium** should be
fixed or disclosed, **Low** is cleanup. Each item also has an **action**:
**Remove**, **Fix**, **Report** (keep it, but disclose or ablate it), or
**Redesign**.

Every claim marked *(probed)* was confirmed by calling the branch code on the
input shown. The offline AgentDojo ablation is in Appendix A of
`IPI_EVALUATION_REVIEW.md`.

---

## Summary table

| ID | crate | where | type | sev. | action | one line |
|---|---|---|---|---|---|---|
| E1 | eval | `scripts/import_agentdojo.py:815`, `harness.rs:249` | LEAK | **Critical** | Remove | The guard's origin scope is built from the user task's ground-truth tool calls. |
| E2 | eval | `worst_case_agent.rs:204-208` | CIRCULAR | **Critical** | Remove | The benign scripted agent acts using `rule_based_must_use`, the same function that builds the fingerprint, so a 0% false-gate rate holds by construction. |
| I9 | ipi | `dry_run/engine.rs:434-443`; `harness.rs:331` | LEAK / STRICT | **Critical** | Redesign / Report | A deployed dry run sees only `"Synthetic page"` and can never see an injection. In the corpus eval it is served the case's attack content. |
| E3 | eval | `adjudication.rs:276` | METRIC | **Critical** | Fix | "Contained" is credited when **any** deviation is flagged, even if the attack's own action was admitted. This rewards strictness. |
| E7 | eval | `agentdojo_bridge.rs:132` etc. | LEAK | **Critical** | Remove / Report | The AgentDojo recipient table, `own_identities`, the refusal text, and one origin per suite (see the generalisation review A2, A5, A6, A9). |
| I1 | ipi | `comparator/destination.rs:143-168` | LEAK / LOOSE / DEAD | High | Remove | Every prompt word is an allowed recipient. It is used only by the AgentDojo bridge. |
| I6 | ipi | `fingerprint/rules.rs:28-58` | STRICT / LOOSE | High | Redesign | The English keyword layer is grown from failure lists, cannot be overridden by the model, and gets ordinary prompts wrong in both directions *(probed)*. |
| I8 | ipi | `fingerprint/engine.rs:29` | design | High | Report | The "fingerprint" predicts only 7 capability bits. Origins and recipients are never predicted. |
| I5 | ipi | `comparator/expected.rs:136` | STRICT / LOOSE | High | Redesign | One scope is copied to every capability, so "read on A, write on B" cannot be expressed. |
| C1 | core | `taxonomy.rs:263` | STRICT / LOOSE | High | Redesign | `click` shares a capability with `form.fill` and `dom.write`. Read tasks that click must be granted the right to submit forms. |
| I14 | ipi | `comparator/guard.rs:130` | LOOSE | High | Fix | Approving a flagged primitive once admits it at **every** origin, attackers' included. |
| E4 | eval | `adjudication.rs:78` | LEAK | High | Fix | The sanitizer is credited only if it fires the **pattern id** named in the case's label, so labels are written in the detector's own vocabulary. |
| E5 | eval | `scripts/gen_redteam_corpus.py:44`; `tests/redteam_lowering.json` | LEAK | High | Fix | Red-team labels are derived from the defense's own lowering table, and this branch edits both together. |
| E6 | eval | `live/outcome.rs:155` | METRIC | High | Report | Residual (data-only) attacks with no indicator string are excluded from attack rates. |
| I13 | ipi | `sanitizer/patterns.rs` (set 3) | LEAK | High | Report | Pattern set 3 was written from the 63 corpus cases it then "solved" (disclosed in §8.3; the 0/809 claim must not be used). |
| E9 | eval | `live/failures.rs`; T-301 commits | LEAK (process) | High | Report | Policy tuning on the same split the numbers are reported on. |
| I2 | ipi | `comparator/expected.rs:207,291` | LOOSE | Medium | Fix | E-mail domains and file names become admitted sites *(probed)*. |
| I3 | ipi | `comparator/expected.rs:141,186` | STRICT | Medium | Fix | Named sites use an exact scope (`www.` and subdomains refused), and plain names ("youtube") are not recognised *(probed)*. |
| I4 | ipi | `comparator/expected.rs:239,271` | STRICT / LOOSE | Medium | Fix | The search engine is hard-coded and result pages are refused. The `http` downgrade applies to writes too. |
| I7 | ipi | `comparator/expected.rs:146-149` | design | Medium | Redesign | `must_use` and `may_use` are enforced identically. Nothing checks that expected work actually happened. |
| I10 | ipi | `dry_run/engine.rs:515,691` | STRICT | Medium | Fix / Report | The dry-run page has no structure (no links, no forms), so a benign plan diverges from the real run. |
| I15 | ipi | `comparator/guard.rs:123,183` | LOOSE | Medium | Redesign | The guard ignores arguments and content: a write in scope is allowed whatever it sends or to whom. |
| C2 | core | `taxonomy.rs:257` | LOOSE | Medium | Report | `navigate` is not treated as data-sending, but URL paths and queries carry data. |
| C3 | core | `scope.rs:229`, `taxonomy.rs` | generalisation | Medium | Report | The DOM-primitive taxonomy reduces API-tool agents (AgentDojo) to roughly one bit: read vs. write. |
| E8 | eval | `live/runner.rs`, `live/outcome.rs` | METRIC | Medium | Fix | Corpus "benign utility" means "the agent stopped with an answer", not "the task was done". |
| E10 | eval | `adjudication.rs:33-60` | METRIC | Medium | Report | `RejectFlagged` is the headline policy. `ApproveAll` is not a realistic lower bound, given I14. |
| I11 | ipi | `comparator/consent.rs:28` | DEAD / LOOSE | Low | Remove / Fix | `ConsentDecision::is_complete` ignores out-of-scope origins, and nothing in the IPI pipeline uses it. |
| I12 | ipi | `tool_decision/mod.rs:150,179` | DEAD | Low | Fix | `prepare_task` runs the sanitizer on the user's own prompt, and the constructor reads `FERRITE_DEFENSE` from the environment. |
| A7 | ipi | `expected.rs:477`; `destination.rs` tests | LEAK | Low | Fix | AgentDojo strings in the defense crate's unit tests. |
| E11 | eval | `live/mock_model.rs` | METRIC | Low | Report | The mock predictor (fixed write-verb list) is the default for `replay`, so replay output must never be cited. |
| E12 | eval | process | METRIC | Medium | Fix | Raw logs for §11.7, §11.8 and §12 are lost, so no number can be re-derived. |

---

## 1. `ferrite-core`

### C1 — `click` is grouped with `form.fill` and `dom.write` in `web.interact` (High; STRICT and LOOSE)

`taxonomy.rs:263`: `WEB_INTERACT = [DomWrite, FormFill, Click]`.

Clicking is how people **read** the web: "show more", pagination, tabs,
accordions, dismissing a cookie banner, following a JavaScript link.
`form.fill`/`dom.write` is how data **leaves** the user. Because both sit under
one capability:

- A read-only task that clicks must either be refused (STRICT) or be granted
  `web.interact` at every in-scope origin. That grant also authorises typing and
  submitting forms there (LOOSE).
- This is the root of what §11.7 measured. Predictor prompt v2 had to predict
  `web.interact` more often to recover utility, and "the prediction also admits
  the attack's capability" doubled from 24.9% to 51.1%.

**Redesign:** split the capability into at least `web.click` (UI navigation, no
typed data) and `web.write` (type, fill, submit). Better still, have a flag on
effects that carry user data. The predictor can then grant clicks freely and
still gate writes.

### C2 — `navigate` is not a data-sending primitive (Medium; LOOSE)

`taxonomy.rs:257`. A navigation to an admitted origin can carry arbitrary data
in its path or query (`https://lite.duckduckgo.com/?q=<secret>`; any GET form).
The taxonomy has no notion of "this effect carries data". The weakness is
already admitted for the search engine (§11.8). It is structural, not specific
to the search engine. **Report it** as a stated limit, or add a data-carrying
attribute to effects (see I15).

### C3 — The DOM-primitive taxonomy collapses on API-tool agents (Medium)

The taxonomy has 17 browser primitives. AgentDojo's 70 tools map onto three of
them: every write is `form.fill`, every read is `dom.read`, and `get_webpage` is
`navigate` (`agentdojo_full_manifest.json` `tool_map`). Within a suite every tool
sits at one origin (E7/A9), so for an API agent the capability layer carries
about one bit: "does the task write?". The ablation shows the consequence:
predicting `web.read`+`web.interact` blocks 0% of workspace, travel and banking
attacks. If the paper claims generality across agent types, the taxonomy needs
**action semantics** (send, delete, pay, share, modify-credentials) or per-tool
effect declarations. Otherwise, scope the claim to browser agents and say why
AgentDojo is a weak fit.

### `ferrite-core` items that are fine

- `DomainSuffix` refuses public suffixes via the PSL (`scope.rs`). This is the
  right building block for I3. It already exists but no IPI producer uses it.
- `ScopablePrimitive` has no `JsExecute` variant, so `js.execute` can never be
  admitted by construction. This is general and correct.
- `OriginScope::admits` returns the admission rank. This is fine.

---

## 2. `ferrite-ipi`

### I1 — `Destination` / `destinations_named_in` (High; LEAK, LOOSE, DEAD)

See the evaluation review F2 and F7, and the generalisation review A3, A4 and
B6.

- **Every token of two or more characters becomes a `Handle`.** *(probed)* "Please
  pay my rent to the landlord and message the team in general" yields `please`,
  `pay`, `my`, `to`, `the`, `and`, `in`, `general`, and so on.
- **Nothing in the IPI pipeline produces or consumes it.** `ExpectedFingerprint`,
  `compare` and `RuntimeGuard` never see a `Destination`. Only
  `ferrite-eval::agentdojo_bridge` turns arguments into destination origins.
- **Its unit tests use AgentDojo's attacker address and IBAN** (A7).

**Remove** it from `ferrite-ipi`. If recipients are part of the contribution,
re-add them as a typed field of the effect model (I15) with typed extraction
(e-mail, IBAN, `@handle`), not every word.

### I2 — `origins_named_in` admits things that are not sites the user named (Medium; LOOSE)

`expected.rs:207` and `:291` *(probed)*:

| prompt | admitted origins |
|---|---|
| "Email the agenda to bob@gmail.com" | `https://gmail.com`, for **every** capability |
| "Fix config.yaml and main.go, unzip data.zip" | `https://config.yaml`, `https://main.go`, `https://data.zip` |

The `NOT_HOSTS` denylist of about 30 file extensions is a patch for a missing
public-suffix check. `ferrite-core` already depends on `psl` (`scope.rs`).
**Fix:** skip e-mail-shaped tokens, and accept a bare name only if
`psl::domain_str` returns a registrable domain.

### I3 — Named sites are scoped too tightly, and plain names are missed (Medium; STRICT)

`expected.rs:141` builds `OriginScope::exact`. *(probed)* For "Check prices on
example.org" the scope admits `https://example.org` and `http://example.org`,
but **not** `https://www.example.org` or `https://shop.example.org`. "open
youtube and play lo-fi" names no site at all (`[]`).

Apex-to-www redirects, login subdomains and CDNs are normal on the real web. None
of them occur in AgentDojo, where every suite is one synthetic origin, so the
benchmark cannot reveal this.

**Fix:** use `OriginScope::DomainSuffix(registrable_domain)` for user-named sites,
and recognise plain site names with one shared parser in `ferrite-core`.

### I4 — `widen_for_task`'s two special cases (Medium; STRICT and LOOSE)

- `SEARCH_ENGINE_ORIGINS = ["https://lite.duckduckgo.com"]` (`expected.rs:239`).
  The defense crate hard-codes one engine, kept in step with the demo agent's
  prompt only by a test. Every search **result** stays out of scope, so any
  "look it up and summarise" task stops at the first result (the remaining §11.8
  refusals include tripadvisor and Google Docs). **Fix:** make the engine a
  parameter. Consider admitting origins reached from a user-requested search for
  read and navigate primitives only, with the provenance recorded.
- The `http://` form of a bare-named site is added for **every** capability,
  writes included (`expected.rs:271`). **Fix:** limit it to `web.read` and
  `web.navigate`.

Both special cases were added in response to failure lists from the evaluation
corpus (E9). They are the kind of rule that improves the measured number without
improving generalisation.

### I5 — One scope is copied to every capability (High; STRICT and LOOSE)

`expected.rs:136-150`: `scope_for_each()` gives every predicted capability the
same origin list (context URL, plus named sites, plus the I4 widenings). The core
data model (`ExpectedCapability { capability, scope }`) already allows a
different scope per capability. The producer never uses that.

So "read the report on a.example and post a summary to b.example" admits
`form.fill` on a.example and reads on b.example. The defense has to over-admit
(LOOSE) or over-refuse (STRICT). Telling the two apart is what a fingerprint is
supposed to do.

**Redesign:** predict (capability, origin) pairs, or at least derive a read scope
and a separate, narrower write scope.

### I6 — The keyword rule layer (High; STRICT and LOOSE)

`rules.rs:28-58` and `rule_based_must_use` (`:178`). Its results are pinned as
`must_use` and the model cannot remove them. *(probed)*:

| prompt | rule layer gives | correct |
|---|---|---|
| "Read the book review on this page" | read **+ interact** | read |
| "Summarise the draft policy" | read **+ interact** | read |
| "I look forward to your summary of this article" | **interact** | read |
| "What form of payment does this shop accept?" | **interact** | read |
| "Accept the cookie banner and read the page" | read **+ scoped.read** (cookie access) | read + click |
| "Is the store open today?" | **navigate** | read |
| "Add this item to my cart and check out" | read | read + interact |
| "Unsubscribe me from this newsletter" | (nothing) | interact |
| "Like this post" / "Translate this page" | (nothing) | interact / read |
| "Envoie un e-mail à Bob avec le rapport" | read | read + interact |

The layer over-admits on everyday nouns ("book", "draft", "forward", "form",
"cookie"), misses ordinary write verbs ("add to cart", "check out",
"unsubscribe", "like"), and is English-only. Its phrase list grew from evaluation
failure lists: T-301 commit `e820d20` added "send money", "post a message",
"create a calendar event" and "schedule a meeting", the verbs of AgentDojo's
suites. Because `must_use` is unconditional, a wrong rule widens the scope
permanently. The model can only add capabilities.

**Redesign, one of:**

- (a) drop the rule layer and rely on the model, with fail-to-empty;
- (b) let rules only *add read-only* capabilities;
- (c) use rules as a disagreement signal (rule and model differ → ask), not as
  a grant.

Freeze the phrase list before any further evaluation (E9).

### I7 — `must_use` and `may_use` are enforced identically (Medium; design)

`from_fingerprint` chains both sets (`expected.rs:146-149`), and nothing reads the
distinction for enforcement. The "deterministic layer" is therefore only a
widening layer.

The design also has no notion of **required** behaviour. An injection that
suppresses the user's task ("don't send the invoice") or swaps its content
produces no deviation. If `must_use` is meant to be the required set, a check
that required work actually happened would catch that class of attack. Either
implement it or stop describing `must_use` as more than a second source of
`may_use`.

### I8 — The fingerprint predicts capabilities only (High; design and claims)

`engine.rs:29` (system prompt) and `build_request`. The model returns a subset of
7 capability names. Origins come from the context URL and string matching on the
prompt (I2–I4). Recipients come from nothing inside the crate (I1). The repo
`CLAUDE.md` describes the idea as "predict the agent's expected **tool/origin**
fingerprint", but the origin half is not predicted.

**Report** this exactly in the paper, or extend the prediction to origins and
recipients. If extended, keep fail-to-empty and keep the prediction free of page
content.

### I9 — The dry run cannot see an injection in deployment (Critical; LEAK and STRICT)

`dry_run/engine.rs:434-443` and `:515`. With no case-authored content, every page
read returns `"Synthetic page. User: <twin name>"` and every extract returns
`"Extracted: <twin email>"`. A real injection lives on the real page, which the
dry run never fetches. So in deployment the dry run:

- **cannot surface any IPI**, because no attacker text ever reaches it; and
- **can only add false consent prompts.** It compares what the agent does on a
  blank page with the prediction. On a page with no links, forms or results, a
  benign agent searches, navigates elsewhere or gives up, and any of those can
  be "unexpected" (I10).

In the corpus evaluation (`harness.rs:331`, `DryRunOrchestrator::with_content`)
the dry run **is** served the case's own content, attack included.
`EVALUATION.md` §8.4 discloses this: "models a dry run that can see the real
page". The §8.3 containment for `On`/`LoopOnly` therefore credits
"dry run → compare → consent" with information a deployed dry run cannot have.

The live eval's `dryrun` mode is correct (empty content, `live/runner.rs`). §8.4
already shows that without the attack content the dry run "ran all 809".

**Report/Redesign:** in the paper, the part that contains attacks is the
**runtime guard** holding the real run to the prediction. Either drop the dry run
from the containment claim, or state its role precisely (checking the plan
against the prediction before any real action) and measure its benign cost.

### I10 — The synthetic page has no structure (Medium; STRICT)

`observe_page` (`engine.rs:691`) returns the URL only. `query` returns no
elements. Pages contain one line of text. A benign plan that needs a link, a
button or a form field cannot unfold the way it would on the real page. This
makes the dry run's diff a poor predictor of the real run in both directions.
**Fix:** give the twin realistic structure (for example a template per page
type), or report the dry run's agreement rate with the real run.

### I11 — `ConsentDecision::is_complete` ignores origins (Low; DEAD and LOOSE)

`consent.rs:28` checks only `diff.extra_primitives`, so a diff whose only
deviation is an out-of-scope origin counts as "complete" with no decision.
Nothing in the IPI pipeline calls it; the demo UI has its own
`consent_is_complete`. **Remove** it, or make it cover origins.

### I12 — `ToolDecisionEngine` plumbing (Low; DEAD)

- `prepare_task` (`tool_decision/mod.rs:179`) runs the sanitizer on the **user's
  prompt**, which is trusted text. Its own comment says there is no page body to
  act on.
- `new()` (`:150`) reads `FERRITE_DEFENSE` from the process environment. Any
  caller, the AgentDojo bridge included, therefore has an environment-dependent
  mode it never sets.

Neither affects the fingerprint, but both mislead a reader about where the
sanitizer acts. **Fix:** drop the prompt sanitisation, and take the mode as a
parameter.

### I13 — Sanitizer pattern set 3 was fitted to its own corpus (High; LEAK, disclosed)

Commit `13ab8dc` says the set was "written after reading those 63 cases".
`EVALUATION.md` §8.3 says the same, and `On` went from 63/809 to 0/809. The
patterns themselves are general English (no AgentDojo template strings; checked
in `patterns.rs`).

**Report:** the 0/809 `On` result must not appear in the paper as an evaluation
result. Use held-out text, which the golden tests partly provide, or a corpus
written by someone else.

### I14 — One approval widens far beyond the action shown (High; LOOSE)

`guard.rs:130`: when a primitive is not in the prediction at all
(`ExtraPrimitive`), a matching entry in `approved_tools` returns `Approved`
**regardless of origin**. A user who approves "`form.fill` on mail.example" has
approved `form.fill` on every origin, attackers' included, for the rest of the
task. Likewise `approved_origins` (`:140`) admits every expected primitive at an
approved origin.

Every evaluation uses "nobody approves", so this never shows up in the numbers.
The paper's statement that "a real user who approves moves the numbers toward
`off`" understates the risk. **Fix:** key approvals by (primitive, origin).

### I15 — The guard ignores arguments and content (Medium; LOOSE, the main design gap)

`guard.rs:123` and `:183`: `check` takes `(Primitive, Option<origin>)`. A
`form.fill` at an in-scope origin is allowed whatever it types, to whomever it
sends, and whatever it deletes. This is why every residual attack on AgentDojo
is a recipient-less in-app write (`delete_file`, `reserve_hotel`,
`update_password`). It is also why the AgentDojo security result had to come
from a recipient table outside the crate (E7).

**Redesign:** a typed effect, for example
`Effect { primitive, origin, recipients, carries_user_data, destructive }`, which
the guard checks and the fingerprint predicts.

### `ferrite-ipi` items that are fine

- **Fail-to-empty** in `predict_may_use` (`engine.rs:176`), enforced by control
  flow.
- **The closed capability allowlist** applied to model output.
- **`compare`'s per-event attribution** with admission rank.
- **`check_all` refuses an empty effect list** (it fails closed).
- **Recording downloads and new tabs at their destination** (`engine.rs`
  `record_call_at`).
- **`num_predict` 2048** for the fingerprint call, and `scroll`/`wait` lowered to
  `web.read`.
- **The sanitizer's folding and normalisation** against disguised text. This is
  general; I13 is about the evaluation claim, not the code.

---

## 3. `ferrite-eval`

### E1 — The guard's origin scope comes from ground truth (Critical; LEAK)

See the evaluation review F1. `import_agentdojo.py:815` `user_origins()` adds
every origin the user task's **ground-truth calls** touch. `harness.rs:249`
(`expected_fingerprint_for`) and `live/runner.rs:535` enforce the result. Six
Slack tasks get origins their prompts never name. **Remove** it from the guard's
input. Keep `expected_origins` for labels only, and build the enforced scope with
`ExpectedFingerprint::from_fingerprint(fp, context, prompt)`.

### E2 — The benign scripted agent is the rule layer (Critical; CIRCULAR)

`worst_case_agent.rs:204-208`:

```rust
fn legitimate_actions(case: &CaseDefinition) -> Vec<ExtraAction> {
    let origin = legitimate_origin(case);
    for capability in ferrite_ipi::fingerprint::rule_based_must_use(&case.user_task) { ... }
```

The "well-behaved" agent performs exactly the capabilities that
`rule_based_must_use` returns, at the case's own origin. In the rules-only §8
run, the fingerprint **is** `rule_based_must_use`. A benign false gate is
therefore impossible by construction, which explains the
`FGR = 0/129 = 0.0% [0–2.9]` in §8.3.

Attack cases mirror this. The malicious agent replays the ground truth's
declared extra primitives and out-of-scope origins, which are defined relative
to the same user-task scope the guard is given (E1). §8.3 discloses
"ADR = 100% by construction", but not that FGR is circular.

**Remove** the §8 benign FGR from the paper, or derive the benign agent's
actions from something independent of the defense, such as a human-written
action list per task or a real model.

### E3 — "Contained" is credited for any deviation (Critical; METRIC)

`adjudication.rs:276`: `consent_gated` is `Gated` whenever `!diff.is_clean()`,
meaning **any** extra primitive or out-of-scope origin anywhere in the run. The
final outcome (`attack_final_outcome`) uses `consent_gated`, not
`fingerprint_caught`. So an attack whose own action **was admitted** is still
scored `ContainedViaConsent` if something unrelated in the run deviated.

The incentive is perverse: the stricter the policy, and the more benign steps it
flags, the more attacks count as "contained". This is exactly the "strict design
that blocks everything" the paper must avoid looking like. `fingerprint_caught`
(`:208`) is the attack-specific measure, but it only feeds ADR. Also
`navigation_flagged` (`:235`, `:255`) counts **any** flagged `navigate` as
catching a declared attacker origin.

**Fix:**

- Credit containment only when a flagged event *is* the attack's effect.
- Report "contained but the run also flagged benign work" separately.
- Require `navigate` to target the declared origin before it counts.

### E4 — Sanitizer credit depends on the detector's own pattern ids (High; LEAK)

`adjudication.rs:78` `sanitizer_matches` counts a catch only if a finding's
`pattern` equals the case's `expected_finding.pattern`, optionally at the
declared channel or JSON path. The label names the detector's internal rule
(`instruction_override`, …), so cases are labelled in the defense's own
vocabulary, by the defense's author. A different but correct detection counts
as "missed". A label written to match the current rule set rewards that rule
set. **Fix:** label *whether and where* there is an injection (span or
channel), not *which rule* must fire.

### E5 — Red-team labels come from the defense's lowering table (High; LEAK)

`scripts/gen_redteam_corpus.py:44` copies `LOWER` from
`ferrite-core/src/taxonomy.rs`. The case's "extra primitives" label is computed
from it. `tests/corpus_redteam_validate.rs` checks the labels against "the real
code". This branch changed `LOWER`, `redteam_lowering.json` and the taxonomy
**in the same commit** (`e820d20`, moving `scroll`/`wait`). When the defense's
taxonomy changes, the ground truth changes with it, so the test can never show
that a taxonomy change admitted an attack. For this particular change, none of
the 909 red-team cases label `scroll`/`wait` as an extra primitive, so the
numbers did not move. The coupling is still the problem. **Fix:** freeze labels
as data and validate the taxonomy **against** them, not the other way round.

### E6 — Residual attacks are dropped from the denominator (High; METRIC)

`live/outcome.rs:155` `measurable()`: a `WithinFingerprintDataOnly` case with no
indicator string is "not measurable" and is excluded from every attack rate. That
class (same primitive, same origin, data-only) is exactly what the architecture
cannot stop. Excluding it makes rates look better. **Report** the excluded count
next to every attack rate, and count those cases as executed in a sensitivity
row.

### E7 — The AgentDojo bridge (Critical; LEAK)

See the evaluation review F2 and F8, and the generalisation review A2, A5, A6
and A9:

- `recipient_args` (`agentdojo_bridge.rs:132`) is a table of AgentDojo tool
  argument names. Without it, injection blocking is 0/240 (workspace), 0/120
  (travel) and 0/144 (banking).
- `own_identities` (`scripts/agentdojo_ferrite.py:99`) reads AgentDojo
  environment attribute paths.
- The `BLOCKED` text (`:49`) tells the agent to "Continue with the user's
  request only", which only the guarded runs see.
- One origin per suite (`agentdojo_bridge.rs:227`).

### E8 — "Benign utility" is not task success (Medium; METRIC)

Live corpus runs report "finished with an answer" (`Stop::Finished`) as the
utility proxy. In guarded modes the dry-run engine serves the case's first page
once, and any other origin gets `"Synthetic page"`. Multi-page benign tasks
therefore cannot really succeed or fail, and an early "finished" counts as
success. Only §12 (AgentDojo's own `utility()`) measures task success. **Fix:**
add a per-case success check for benign corpus tasks, or use only §12-style
utility in the paper.

### E9 — Tuning on the evaluation split (High; LEAK, process)

`live/failures.rs` writes per-case failure lists. The T-301 commits changed
`ferrite-ipi` in response to them (I4 search engine and `http`, I6 phrases,
predictor prompt v2, named sites) and re-scored the same 1,984 cases. The same
happened with sanitizer set 3 (I13). **Fix:** freeze a held-out split now and
never open its failure lists.

### E10 — Consent policies (Medium; METRIC)

`adjudication.rs:33-60`:

- `RejectFlagged`, the headline policy, is the best case for containment.
- `ApproveAll` is described as the lower bound. Because of I14, a single
  approval in practice widens beyond the action, so neither policy models a real
  user.

**Report** this, and fix I14 before using `RandomP` as a realistic middle
estimate.

### E11 — The mock predictor (Low)

`live/mock_model.rs` answers `web.read`, plus `web.interact` if a `WRITE_VERBS`
word appears. It is the default `--provider` for `agentdojo_ferrite.py replay`.
Replay output with the mock is a plumbing check, not a result. Never cite it.

### E12 — No reproducibility (Medium)

The raw logs and caches for §11.7, §11.8 and §12 are lost. **Fix:** archive them
with every published number.

### `ferrite-eval` items that are fine

- §12 runs AgentDojo's own environments, attack and `utility()`/`security()`
  unchanged.
- The live eval's `dryrun` mode uses empty content, like a real deployment.
- The live agent uses the product's own agent loop and the same
  `BLOCKED_OBSERVATION` text as the product.
- Wilson intervals throughout, and the 97/132 benign-denominator correction.

---

## 4. Removal list (in priority order)

These deletions stop the numbers from depending on the benchmark or on the
defense's own logic:

1. **E1:** delete the use of ground-truth origins as the enforced scope.
2. **E2:** delete `legitimate_actions`'s dependence on `rule_based_must_use`.
   Until the benign agent is independent, drop the §8 FGR claim.
3. **E3:** delete "any dirty diff ⇒ contained". Use attack-specific containment.
4. **E7 / I1:** delete `recipient_args`, `own_identities`, the bridge's `BLOCKED`
   text, and `Destination`/`destinations_named_in` from the Ferrite arm. Keep a
   recipient allowlist only as a separately reported baseline.
5. **I9:** delete the dry run's containment claim. Keep the runtime guard as the
   containment mechanism.
6. **I6:** delete the keyword phrases added from failure lists. Stop pinning
   rule output as non-removable `must_use`.
7. **I4:** delete the hard-coded `SEARCH_ENGINE_ORIGINS` constant (make it a
   parameter) and the `http` widening for writes.
8. **E4 / E5:** delete pattern-id labels and labels generated from the
   defense's own lowering table.
9. **A7:** delete AgentDojo strings from `ferrite-ipi` tests.
10. **I11 / I12:** delete `ConsentDecision::is_complete`, the prompt-only
    sanitizer call, and the environment read in `ToolDecisionEngine::new`.

## 5. Changes that generalise without making the defense stricter

The user's constraint is not to buy security with refusals. These changes reduce
false refusals and over-admission together:

| change | effect on benign tasks | effect on attacks |
|---|---|---|
| Split `click` from `write` (C1) | read tasks that click stop needing `web.interact` | writes stay gated even when clicks are granted |
| Per-capability scopes (I5) | reads can cover more sites | writes are narrowed to where the user asked |
| Registrable-domain scope for named sites, plus the PSL check (I2, I3) | `www.`, subdomains and redirects stop being refused | file names and e-mail domains stop being admitted |
| Typed effects with recipients and data-carrying flags (I15, C2) | writes to people the user named, or their own contacts, pass | writes to unnamed recipients and data-carrying navigations are gated |
| Approvals keyed by (primitive, origin) (I14) | none | one "yes" no longer opens every origin |
| Rules as a disagreement signal, not a grant (I6) | fewer refusals for unfamiliar verbs (the model decides) | no spurious `interact` or `scoped.read` from nouns |
| Provenance-based read admission for user-requested searches (I4) | research tasks work | writes on result sites stay gated |

## 6. What the paper can claim honestly today

> Predicting a task's capability set from the user's words alone, and holding
> each real action to that prediction plus origins the user named, contains the
> attacks that need an **unexpected capability or an unexpected origin**. It
> cannot contain attacks that reuse the task's own capability at the task's own
> origin (the residual). On AgentDojo, whose suites each occupy a single origin,
> that residual is most of the benchmark. Recipient-level checks, which are not
> yet part of the architecture, account for the measured containment there.

Every stronger claim depends on one of the items above being fixed first.
