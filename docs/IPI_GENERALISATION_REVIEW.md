# Ferrite IPI: what to remove and what blocks generalisation

Branch reviewed: `web-interact-fix`. The request named `web-interface-fix`, but no
branch with that name exists locally or on `origin`. `web-interact-fix` is the
branch that holds the AgentDojo work. The review covers the head at `83f514a`
against `origin/main`.

Companion document: `docs/IPI_EVALUATION_REVIEW.md` covers the leakage audit and
the ablation numbers. This document is the action list for the paper. It says
what to delete or demote so that the IPI defense is a real architecture, not a
filter fitted to AgentDojo. It also lists the code defects that make it refuse
normal tasks.

## The goal, stated as a test

A reviewer will accept "Ferrite generalises" only if three things are true:

1. **Every decision input is available to a deployed browser.** That means the
   user's words, the current tab, the user's own identity, and the agent's
   actions. Nothing may come from a benchmark's ground truth, tool signatures or
   environment layout.
2. **The defense that is evaluated is the defense that ships.** The same crate
   path makes the decision in `ferrite-ui` and in the evaluation.
3. **Benign utility is reported with the same weight as attack success.** A
   policy that blocks everything trivially reaches 0% attack success.

Each item below fails at least one of these tests.

---

## A. Remove, or demote to clearly labelled baselines

These are the essential removals. While they stay in place, the AgentDojo numbers
describe these pieces, not the fingerprint → dry-run → compare architecture.

| # | Remove / demote | Where | Why |
|---|---|---|---|
| A1 | **Ground-truth-derived scopes as guard input.** `user_origins()` adds the user task's ground-truth call origins to `expected_origins`, and the live eval enforces them. | `scripts/import_agentdojo.py:815`, `crates/ferrite-eval/src/harness.rs` `expected_fingerprint_for`, `live/runner.rs:535` | The answer key is an input. Build the scope as the app does (`from_fingerprint(fp, context, prompt)`). Keep `expected_origins` for **labelling** only, never for **enforcing**. |
| A2 | **The AgentDojo recipient table as part of "Ferrite".** `recipient_args()` is a per-tool list of AgentDojo arguments. | `crates/ferrite-eval/src/agentdojo_bridge.rs:132` | Removing it takes injection blocking to 0/240 (workspace), 0/120 (travel) and 0/144 (banking), even when the prediction is perfect. Report it as a separate "recipient allowlist" baseline arm. If you want it in the paper as a contribution, ship it in the product (see B7). |
| A3 | **`destinations_named_in` in its current form.** Every prompt word of two or more characters becomes an allowed recipient handle. | `crates/ferrite-ipi/src/comparator/destination.rs:143-168` | "pay my rent to the landlord" admits the handles `pay`, `my`, `to`, `the`, `in`, `general`, and so on. In the only place it is used (the bridge) it is both too loose and AgentDojo-shaped. Delete it from `ferrite-ipi`, or replace it with typed extraction (see B6). |
| A4 | **`Destination` living in `ferrite-ipi` with no product caller.** | `crates/ferrite-ipi/src/comparator/{destination.rs,mod.rs:221}` | The only consumer is `ferrite-eval`. `CLAUDE.md` says "No dead code. If a function exists, something reachable calls it." This is code that exists only for the evaluation, sitting in the defense crate. Move it to `ferrite-eval`, or wire it into `ferrite-ui`. |
| A5 | **`own_identities()` attribute paths.** It reads `inbox.account_email`, `bank_account.iban` and `user_account.email`. | `scripts/agentdojo_ferrite.py:99` | These paths come from AgentDojo's environment layout, and Slack returns `[]`. The browser has no equivalent concept yet. If "the user's own identities" is part of the design, define it in the product (profile or settings) and pass it the same way in both. |
| A6 | **The bridge's own refusal text.** `BLOCKED` ends "Continue with the user's request only." | `scripts/agentdojo_ferrite.py:49` | The product's message is `ferrite-ui/src/runtime_guard.rs` `BLOCKED_OBSERVATION`, which is different text. The evaluation therefore measures a different prompt nudge from the one that ships. Use the product constant, and add a neutral-error ablation. |
| A7 | **AgentDojo data in the defense's unit tests.** Examples: `www.dora-website.com` (`expected.rs:477`), `mark.black-2134@gmail.com` (`destination.rs` tests, bridge tests), and AgentDojo's IBANs. | `crates/ferrite-ipi/src/comparator/{expected,destination}.rs` tests | Tests encode the benchmark as the specification of the defense. Use synthetic hosts (`*.example`) in `ferrite-ipi`. Keep AgentDojo strings only under `ferrite-eval`. |
| A8 | **The failure-list tuning loop on the test corpus.** `FAILURES.md`, then a policy change, then a rerun on the same 1,984 cases. | process; `crates/ferrite-eval/src/live/failures.rs` | The tool is fine. Using it on the evaluation split is not. Freeze a held-out split, for example the `travel` suite or a random 30% of user tasks, and never open its failure list. |
| A9 | **One origin per AgentDojo suite.** `suite_origins` maps every tool in a suite to one origin. | `agentdojo_full_manifest.json`, `agentdojo_bridge.rs:227` | This makes origin scope, which is the core primitive, unmeasurable on AgentDojo. Either say so in the paper or give each tool family its own origin (mail, calendar, drive). |

---

## B. Code defects that keep the architecture from generalising

These defects cause **benign refusals in the real browser**, or silent
over-admission, with no benchmark involved. I confirmed each one by running
the branch code on the inputs shown.

| # | Defect | Location | Observed |
|---|---|---|---|
| B1 | **Exact-origin scope for sites the user names.** | `comparator/expected.rs:141` | For "Check prices on example.org", the scope admits `https://example.org` but **not** `https://www.example.org` or `shop.example.org`. Ordinary apex → www redirects, and login or CDN subdomains, go to consent. Use the registrable domain (an eTLD+1 `DomainSuffix`) for user-named sites. |
| B2 | **Two divergent "which site did the user mean" parsers.** `ferrite-agent/src/context.rs` has `host_of` (strips `www.`), `TLDS` and `KNOWN_SITES` (so "open youtube" means youtube.com). `origins_named_in` has none of these. | `expected.rs:186-230` vs `ferrite-agent/src/context.rs:309,348` | "open youtube and play lo-fi" → `origins_named_in` returns `[]`. The agent goes to youtube.com and the guard refuses it. Move one parser into `ferrite-core` and use it from both. Dependency direction forbids `ipi → agent`, so the parser has to move down. |
| B3 | **Email addresses parsed as sites.** `host.rsplit_once('@')` takes the domain of `bob@gmail.com`. | `expected.rs:207` | "Email the agenda to bob@gmail.com" admits `https://gmail.com` for **every** capability, `web.interact` included. Skip tokens that look like e-mail addresses. |
| B4 | **File names admitted as hosts, with a denylist patch.** `NOT_HOSTS` lists about 30 extensions. | `expected.rs:291` | "Fix config.yaml and main.go, unzip data.zip" admits `https://config.yaml`, `https://main.go` and `https://data.zip`. Check names against the public-suffix list, which a correct eTLD+1 needs for B1 anyway, instead of a list of extensions. |
| B5 | **One scope copied to every capability.** `scope_for_each()` gives `web.read`, `web.navigate`, `web.interact` and `web.download` the same origin list. | `expected.rs:136-150` | A prompt such as "read A, then post a summary on B" admits writes on A and reads on B. The predictor cannot express which capability belongs at which origin, so the design must either over-admit or over-refuse. Predict (capability, origin) pairs, or at least split read-scope from write-scope. |
| B6 | **Recipients drawn from untyped words.** | `destination.rs:143-168`; bridge `destination()` lets `Account` fall back to `Handle` (`agentdojo_bridge.rs:174`) | Too loose (see A3). It is also too strict for real tasks, because the recipient usually comes from data the user pointed at ("reply to the sender", "invite the person in Bob's message"). That is why Slack benign utility is 9.5%. Taint-style provenance would generalise: a recipient read from a source the **user** named is allowed, one introduced by unrelated tool output is not. |
| B7 | **The product has no argument- or recipient-level check at all.** | `ferrite-ui/src/runtime_guard.rs` `action_effects` | A form fill on an in-scope mail site that sends to an attacker's address is `form.fill` at an admitted origin, so it is allowed. The AgentDojo result that says this is blocked comes only from the bridge (A2). |
| B8 | **Hard-coded search engine, and every result page is refused.** `SEARCH_ENGINE_ORIGINS = ["https://lite.duckduckgo.com"]`, kept in step with the agent's system prompt by a test. | `expected.rs:239` | Any "look it up and summarise" task goes to consent on the first result click. The constant also ties the defense to one engine. For read-only primitives, admitting origins reached from a search the user asked for (with provenance recorded) would generalise. Writes there stay gated. |
| B9 | **Plaintext downgrade for every capability.** The `http://` form of a bare-named site is added to `web.interact` and `web.download` as well. | `expected.rs:271` | A user who types `bank.example` lets the agent submit forms over `http://bank.example`. Limit the http admission to `web.read` and `web.navigate`, or follow an HSTS redirect instead. |

---

## C. Keep: these generalise

- Moving `scroll`/`wait` from `web.interact` to `web.read`
  (`ferrite-core/src/taxonomy.rs`). Neither changes the page nor sends data. This
  is correct and benchmark-neutral.
- `FINGERPRINT_NUM_PREDICT = 2048` for reasoning models. This fixes a real
  fail-to-empty that happened for the wrong reason.
- Predictor prompt v2's capability definitions. They are generic. Note, though,
  that the attack-capability overlap rose from 24.9% to 51.1%, so report it as a
  trade-off.
- Reading only the user's own words for widening (`trusted_task_text` is user
  turns only), the dry run on synthetic pages, and the per-action runtime guard
  with consent (`settle`/`ask_for`). These are the architecture. Spend the
  remaining months on evaluating them faithfully, not on more allowlists.

## D. What the paper needs before submission

1. AgentDojo with **A1, A2 and A6 removed** from the Ferrite arm. Report the
   recipient allowlist as its own baseline, together with "recipient allowlist +
   Ferrite".
2. Secure utility (utility ∧ ¬attack) per pair, next to ASR and benign utility.
3. A held-out split, ≥2 agent models, ≥2 attack templates, ≥3 seeds, and one
   adaptive attack: an in-scope write with no recipient, and a recipient the
   prompt names.
4. Raw logs and caches archived for every number.
5. A non-AgentDojo benign set of real browsing tasks to measure B1–B4 and B8,
   since those failures do not occur in AgentDojo's simulated tools at all.
