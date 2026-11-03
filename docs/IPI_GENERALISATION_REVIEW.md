# Ferrite IPI: what to remove and what blocks generalisation

**Branch reviewed:** `web-interact-fix`. The request named `web-interface-fix`, but
no branch with that name exists, locally or on `origin`. `web-interact-fix` is the
branch that holds the AgentDojo work. I reviewed its head against `origin/main`.

**Scope of the paper, and of this review:** the IPI defense crates.

- **In scope:**
  - **`ferrite-ipi`:** fingerprint, dry run, comparator, runtime guard, and
    destinations.
  - **The `ferrite-core` types it is built on:** the capability/primitive
    taxonomy and `OriginScope`.
- **`ferrite-eval`** is the measurement harness. It appears below only where it
  feeds decisions into the defense, or changes what a number means.
- **Out of scope for the paper:** `ferrite-ui`, `ferrite-agent`, `ferrite-engine`
  and the Servo shell, which are the live demo. They are mentioned here only when
  the IPI crates rely on them.

**Companion document:** [`IPI_EVALUATION_REVIEW.md`](IPI_EVALUATION_REVIEW.md)
covers the leakage audit and the ablation numbers. This document is the action
list. It says what to delete or demote so that `ferrite-ipi` stands as a defense
architecture rather than a filter fitted to AgentDojo, and it lists the defects in
the IPI crates that make the defense refuse normal tasks.

## The goal, stated as a test

A reviewer will accept "the defense generalises" only if three things hold:

1. **Every input to an IPI decision is something any integrator has.** That means
   the user's words, the agent's actions, and optionally a context URL and the
   user's own identities. No input may come from a benchmark's ground truth, its
   tool signatures or its environment layout.
2. **Every decision is made inside `ferrite-ipi`.** The evaluation harness only
   translates the benchmark's tool calls into IPI's vocabulary. If code outside the
   IPI crates decides what is allowed, the paper is measuring that code, not the
   contribution.
3. **Benign utility is reported with the same weight as attack success.** A policy
   that blocks everything trivially reaches 0% attack success.

Every item below fails at least one of these tests.

---

## A. Remove, or demote to clearly labelled baselines

These are the essential removals. While they stay, the AgentDojo numbers describe
these pieces, not the fingerprint → dry-run → compare architecture.

| # | Remove / demote | Where | Why |
|---|---|---|---|
| A1 | **Ground-truth-derived scopes as guard input.** `user_origins()` adds the origins of the user task's ground-truth calls to `expected_origins`, and the live eval enforces them as the guard's scope. | `scripts/import_agentdojo.py:815`; `crates/ferrite-eval/src/harness.rs` `expected_fingerprint_for`; `live/runner.rs:535` | The answer key becomes a defense input (fails test 1). Build the scope with IPI's own constructor, `ExpectedFingerprint::from_fingerprint(fp, context, prompt)`. Keep `expected_origins` for **labelling** only. |
| A2 | **Treating the eval-side recipient table as the defense.** `recipient_args()` is a per-tool list of AgentDojo arguments, and it lives in the harness, not in `ferrite-ipi`. | `crates/ferrite-eval/src/agentdojo_bridge.rs:132` | Without it, injection blocking is 0/240 (workspace), 0/120 (travel) and 0/144 (banking), even with a perfect prediction. The decision that matters is made outside the IPI crates (fails test 2). Either move a general argument/effect model into `ferrite-ipi` (B7), or report this table as its own baseline arm. |
| A3 | **`destinations_named_in` in its current form.** Every prompt word of two or more characters becomes an allowed recipient handle. | `crates/ferrite-ipi/src/comparator/destination.rs:143-168` | "pay my rent to the landlord" admits `pay`, `my`, `to`, `the`, `in`, `general`, and so on. This makes the allowlist too loose, and it is shaped around AgentDojo's Slack and bank payees. Delete it, or replace it with typed extraction (B6). |
| A4 | **`Destination` as an unexamined part of `ferrite-ipi`.** It is exported from the comparator, but nothing in the IPI pipeline (fingerprint → `ExpectedFingerprint` → `compare` / `RuntimeGuard`) produces or consumes it. Only the AgentDojo bridge does. | `crates/ferrite-ipi/src/comparator/{destination.rs,mod.rs:221}` | Decide what it is. If recipients are a paper contribution, make them part of `ExpectedFingerprint` and the guard's effect model, and evaluate them as such. If they are not, move `Destination` into `ferrite-eval` as baseline code. Repo `CLAUDE.md`: "No dead code. If a function exists, something reachable calls it." |
| A5 | **`own_identities()` attribute paths.** These read `inbox.account_email`, `bank_account.iban` and `user_account.email`. | `scripts/agentdojo_ferrite.py:99` | The paths come from AgentDojo's environment layout, and Slack returns `[]`. If "the user's own identities" is a defense input, define it as a typed parameter of the IPI API and document where an integrator gets it. The harness should only fill that parameter. |
| A6 | **The harness's steering refusal text.** `BLOCKED` ends "Continue with the user's request only." | `scripts/agentdojo_ferrite.py:49` | The text the agent sees is not part of `ferrite-ipi`, yet it is an instruction-hierarchy nudge that the `off` arm never gets. It is a confound in the guard arm. Use a neutral error ("Tool call failed.") so the measured effect is the guard's decision alone, or give both arms matched text. |
| A7 | **AgentDojo data in `ferrite-ipi`'s unit tests.** Examples: `www.dora-website.com` (`expected.rs:477`) and `mark.black-2134@gmail.com` (`destination.rs` tests). | `crates/ferrite-ipi/src/comparator/{expected,destination}.rs` tests | These tests make the benchmark the specification of the defense. Use synthetic hosts (`*.example`) inside `ferrite-ipi`, and keep AgentDojo strings under `ferrite-eval` only. |
| A8 | **The failure-list tuning loop on the test corpus.** `FAILURES.md`, then an IPI policy change, then a rerun on the same 1,984 cases. | process; `crates/ferrite-eval/src/live/failures.rs` | The tool itself is fine. Using it on the evaluation split is not. Freeze a held-out split, for example the `travel` suite or a random 30% of user tasks, and never open its failure list. |
| A9 | **One origin per AgentDojo suite.** `suite_origins` maps every tool in a suite to a single origin. | `agentdojo_full_manifest.json`; `agentdojo_bridge.rs:227` | This makes `OriginScope`, the core IPI primitive, unmeasurable on AgentDojo. Either state that in the paper, or give each tool family its own origin (mail, calendar, drive). |

---

## B. Defects in the IPI crates that keep the defense from generalising

These defects would make **any agent integrated with `ferrite-ipi`** refuse normal
tasks or silently over-admit, with no benchmark involved. I confirmed each one by
calling the branch's `ferrite-ipi` functions on the inputs shown.

| # | Defect | Location | Observed |
|---|---|---|---|
| B1 | **Exact-origin scope for sites the user names.** | `comparator/expected.rs:141` | For "Check prices on example.org", the scope admits `https://example.org` but **not** `https://www.example.org` or `shop.example.org`. Apex → www redirects and login or CDN subdomains are refused. Use registrable-domain (eTLD+1) scope (`DomainSuffix`, already in `ferrite-core`) for user-named sites. |
| B2 | **`origins_named_in` does not recognise sites named in plain words.** | `expected.rs:186-230` | "open youtube and play lo-fi" → `[]`. Any agent that turns that request into youtube.com is refused. The site-name parser should be a single function in `ferrite-core`, covering bare names, `www.` and public suffixes, used everywhere a "named site" is needed. The demo's agent already has a separate, different parser (`ferrite-agent/src/context.rs`); the two disagree. |
| B3 | **Email addresses parsed as sites.** `host.rsplit_once('@')` takes the domain of `bob@gmail.com`. | `expected.rs:207` | "Email the agenda to bob@gmail.com" admits `https://gmail.com` for **every** capability, `web.interact` included. Skip tokens that look like e-mail addresses (they are recipients, B6). |
| B4 | **File names admitted as hosts, with a denylist patch.** `NOT_HOSTS` lists about 30 file extensions. | `expected.rs:291` | "Fix config.yaml and main.go, unzip data.zip" admits `https://config.yaml`, `https://main.go` and `https://data.zip`. Check against the public-suffix list, which B1 needs anyway, instead of a list of extensions. |
| B5 | **One scope copied to every capability.** `scope_for_each()` gives `web.read`, `web.navigate`, `web.interact` and `web.download` the same origin list. | `expected.rs:136-150` | "Read A, then post a summary on B" admits writes on A and reads on B. The fingerprint cannot say which capability belongs at which origin, so the design has to over-admit or over-refuse. This is the most important architectural gap. Predict (capability, origin) pairs, or at least separate a read scope from a write scope. |
| B6 | **Recipients drawn from untyped words.** | `destination.rs:143-168`; the harness's `Account` → `Handle` fallback (`agentdojo_bridge.rs:174`) | Too loose (A3). It is also too strict for real tasks, because the recipient usually comes from data the user pointed at ("reply to the sender", "invite the person in Bob's message"). That is why benign utility in Slack is 9.5%. Provenance would generalise: a recipient read from a source the user named is allowed, and one introduced by unrelated content is gated. |
| B7 | **`RuntimeGuard` has no argument-level effect model.** `check` and `check_all` take only `(Primitive, Option<origin>)`. | `comparator/guard.rs:123,183` | Whether a write sends data to the right party depends entirely on the caller turning arguments into extra "origins" (the bridge does this; no IPI API does). Every integrator would have to reinvent it, and the paper's AgentDojo security comes from that caller-side code (A2). Add a typed effect, such as `Effect { primitive, origin, recipients }`, to `ferrite-ipi` so the defense owns the decision. |
| B8 | **Hard-coded search engine, and every result page refused.** `SEARCH_ENGINE_ORIGINS = ["https://lite.duckduckgo.com"]`. | `expected.rs:239` | A defense crate should not hard-code one engine. It is kept in step with the demo agent's prompt only by a test. Any "look it up and summarise" task is refused at the first result page. Make the engine a parameter. For read-only primitives, consider admitting origins reached from a search the user asked for (with provenance recorded); writes there stay gated. |
| B9 | **Plaintext downgrade for every capability.** The `http://` form of a bare-named site is added to `web.interact` and `web.download` as well. | `expected.rs:271` | A user who types `bank.example` lets the agent submit forms over `http://bank.example`. Limit the http admission to `web.read` and `web.navigate`. |

---

## C. Keep: these generalise

- **`scroll`/`wait` → `web.read`** (`ferrite-core/src/taxonomy.rs`). Neither one
  changes the page or sends data. The change is correct and benchmark-neutral.
- **`FINGERPRINT_NUM_PREDICT = 2048`.** This fixes a fail-to-empty that a
  reasoning model triggered for the wrong reason.
- **Predictor prompt v2's capability definitions.** They are generic. Report the
  trade-off: the overlap with the attack's capability rose from 24.9% to 51.1%.
- **The core IPI loop:**
  - fail-to-empty fingerprints;
  - only the user's own words feeding the prediction and widening;
  - the dry run against synthetic data;
  - `compare` attribution;
  - `RuntimeGuard::check_all` failing closed on an empty effect list.

  This is the contribution. Spend the remaining months evaluating it faithfully
  and fixing B5 and B7, not adding more allowlists.

## D. What the paper needs before submission

1. **An AgentDojo run where every decision is made by `ferrite-ipi`.** Remove A1
   and A6, and either move A2 into the crate (B7) or report it as a separate
   "recipient allowlist" baseline next to "recipient allowlist + IPI".
2. **Secure utility (utility ∧ ¬attack) for each pair,** reported next to attack
   success and benign utility.
3. **A broader, tamper-free setup:**
   - a held-out split;
   - at least 2 agent models;
   - at least 2 attack templates;
   - at least 3 seeds;
   - one adaptive attack: an in-scope write with no recipient, and a recipient the
     prompt names.
4. **Raw logs and caches archived** for every number.
5. **A benign task set outside AgentDojo, built from real sites and URLs,** to
   measure B1–B4 and B8. Those failures cannot appear in AgentDojo's simulated
   tools, because every suite is one origin. It can be scored offline through
   `ferrite-ipi`'s API; the demo browser is not needed.
