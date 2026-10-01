# Ferrite Browser — Full Rebuild Directive

> This is the controlling document for the rebuild. Where it conflicts with
> anything else on disk, this document wins.
>
> Saved to the repo per its own instruction ("drop it in the repo at
> `docs/REBUILD_DIRECTIVE.md`") during the A0 session. The amendments given
> in chat alongside A0's execution (de-nest in A0 not A1, branch check before
> the move, the archive poison-warning banners, deriving `docs/TO-DO.md` from
> three sources, moving the planning PDFs in the same pass, widening the
> purge check, and the note on the pilot corpus not being grandfathered) are
> not re-transcribed into this file — they were operational instructions for
> A0's execution, and what actually happened is recorded in
> `docs/PROGRESS.md` and `docs/handoffs/a0.md`, per this repo's own doc
> hierarchy (PROGRESS.md wins for "what is done"; this file is the plan, not
> the record).

---

## 0. Context brief (read this before touching anything)

You are rebuilding **Ferrite**, a Rust agentic browser built on Servo whose
research contribution is an *architectural* defense against **indirect
prompt injection (IPI)**: the `predict → dry-run → compare → consent` loop.

**Lineage, so you understand why the tree looks odd:**

- The project began as a *capability-broker browser*: Ed25519 capability
  tokens, a Regorus/Rego policy engine, a Merkle-tree audit log, Wasm/Extism
  extension sandboxing, and a JSON-RPC agent over `ws://localhost:9222`.
  Crates: `ferrite-types`, `ferrite-broker`, `ferrite-policy`,
  `ferrite-sandbox`, `ferrite-network`, `ferrite-a11y`, `ferrite-cef`.
- In **2026-04** the scope was cut hard to IPI-only. The broker-era crates
  were shelved (`ferrite-capability-broker`, `ferrite-policy`,
  `ferrite-sandbox` sat on disk, outside the Cargo workspace, dormant, until
  A0 deleted them outright).
- **That old design is dead and is not coming back.** It must not appear
  anywhere in the new tree, including in dotfiles, READMEs, devcontainer
  config, or example commands.

**Current live shape (7 crates):** `ferrite-shell`, `ferrite-servo`,
`ferrite-ui`, `ferrite-audit-log`, `ferrite-agent`, `ferrite-ipi`,
`ferrite-eval`.

**What actually works today** (verified against source, not docs): the
tool-decision engine, the regex sanitizer (detection on, excision built but
disabled), synthetic data twin, dry-run orchestrator + recording executor,
comparator + consent UI, dataset schema, eval harness with adjudication and
audit anchoring. Both the production path (ferrite-ui) and the eval path
share the same core types.

**What is genuinely broken or missing** — the defect register in §9 is
authoritative; fix every item.

**What the docs said was wrong** — `CLAUDE.md` used to list five completed
tasks as "not yet implemented"; `.rules` and `README.md` described the dead
broker architecture; `PROJECT_REFERENCE.md` claimed six crates and a stubbed
dataset pipeline. §12 fixes the doc layer permanently; A0 executed the first
pass of that fix.

**Environment:** macOS at the repo root (flattened from `Browser/` nesting
by A0), but the repo has multiple contributors and CI runs Windows + macOS.
**Every instruction you write in docs must be OS-neutral.** No
PowerShell-only commands, no `C:\Dev\...` paths. Use the `justfile` (§7,
still to be built — A1) as the single command surface.

**Out of scope for this rebuild:** the paper. Do not write paper prose, do
not chase the ACSAC/INDOCRYPT/ICISSP deadlines in the archived
`docs/archive/paper/PAPER_STATUS.md` (all past). Archived, not deleted. You
*do* build the evaluation machinery and the methodology document, because
that is engineering, not writing.

---

## 1. Mission

Rebuild Ferrite to production quality, test-first, from the existing code as
reference material rather than as a base to patch. Concretely:

1. **Purge** everything dead, unreachable, or contradictory — including
   things "we might want later". If it is not needed now and there is no
   concrete near-term plan, it goes. Git history is the archive.
2. **Restructure** the workspace into clean crate boundaries with explicit,
   typed contracts between them.
3. **Reimplement** each subsystem properly, one agent session at a time,
   with tests written before implementation.
4. **Implement the not-yet-implemented**: per-capability origin scoping,
   live sanitizer excision with correct outcome accounting, hash-covered
   audit entries, the full agentic browser action surface, a real corpus
   and eval pipeline.
5. **Pluggable model layer**: **Ollama Cloud is the default backend**,
   Gemini is the alternate, local Ollama is the offline fallback, and the
   trait is shaped so OpenAI/Anthropic drop in later. Deterministic mock +
   recorded-replay providers back every test, so the test suite never
   spends quota.
6. **Optimize build/disk footprint** aggressively without losing runtime
   performance, so the whole loop runs locally on one laptop.
7. **Deliver an evaluation methodology** with stated objectives, formulas,
   corpus sizing derived from a power calculation, and a written
   justification for every tunable parameter. This is what the project is
   graded on — treat it as a first-class deliverable, not a postscript.

---

## 2. Hard rules (non-negotiable, apply to every session)

**R1 — Test-first, always.** For every behavior: write the failing test,
run it, see it fail for the right reason, implement the minimum to pass,
refactor, commit. No implementation lands without a test that would fail if
the implementation were reverted. No `#[ignore]`d tests left behind without
a linked TODO ID.

**R2 — No status claims without proof.** A doc may only say something is
done if it cites the test name(s) or commit SHA that prove it. If you
cannot cite, write "not implemented".

**R3 — Frequent commits.** Commit at every green test cycle, minimum one
commit per completed unit of work. Conventional Commits format
(`feat(ipi): ...`, `fix(comparator): ...`, `test(eval): ...`,
`chore(build): ...`, `refactor(ui): ...`, `docs: ...`).

**R4 — No AI attribution in git.** Commit messages must never contain
"Claude", "Co-Authored-By: Claude", "Generated with", "AI-assisted", or any
equivalent. The guard hook in §11 (installed in P0) enforces this
mechanically, not by memory.

**R5 — One agent, one scope, one session.** Each agent charter (§6) names
the exact files it may touch and the exact files it may read. Do not read
the whole repo. Use `rg` with targeted patterns rather than dumping files.
Never load `target/`, `Cargo.lock`, or corpus JSON into context.

**R6 — Keep sessions light.** Rate limits are a real constraint. Each
session ends with a short handoff written to
`docs/handoffs/<agent-id>.md` (≤60 lines: what landed, test names, open
questions, exact next action). The next session reads only that handoff
plus its own charter's file list.

**R7 — No live model calls in tests.** Every test must pass with the
network cable pulled and `OLLAMA_API_KEY` unset. Unit tests use
`MockProvider`; integration tests use `ReplayProvider` reading committed
fixtures. Live-provider tests are `#[ignore]`d and run only via
`just test-live`. Quota is a scarce resource — a test suite that burns it is
a broken test suite.

**R8 — Determinism.** Fixed seeds, `temperature = 0`, sorted iteration over
anything hash-backed before it reaches output, no wall-clock in assertions
(inject a `Clock` trait).

**R9 — No dead code.** `#![deny(dead_code)]` at crate roots (with
`#[cfg(test)]` exceptions where genuinely needed). If a function exists,
something reachable calls it. `admission_rank()` being computed and never
consumed is exactly the class of bug this rule exists to prevent.

**R10 — Stop at gates.** Every phase and every agent has an exit gate. When
you hit it, stop, write the handoff, and say so. Do not roll into the next
agent's scope.

---

## 3. Phase map

| Phase | Name | Exit gate |
|---|---|---|
| **P0** | Archaeology, purge, doc reset | Tree builds, dead crates/docs gone, guard hooks live, `docs/AUDIT.md` + new `TO-DO.md` + empty `PROGRESS.md` committed |
| **P1** | Foundation: workspace, profiles, justfile, CI, lint gates | `just check` and `just test` green on a clean clone in <5 min without Servo |
| **P2** | Core contracts: types, IDs, capability/primitive taxonomy, errors, clock, config | 100% of public types documented + round-trip serde tests |
| **P3** | Model layer: mock / replay / Ollama Cloud / Gemini, with cache + budget | Mock- and replay-driven tests green; `just models` lists cloud tags; `just probe` does one live round-trip |
| **P4** | Defense core: fingerprinting, sanitizer, twin, dry-run, containment | Each component's spec tests green in isolation |
| **P5** | Comparator + origin model + consent decisioning | Per-capability scoping works; property tests green |
| **P6** | Audit log (hash-covered) + persistence | Tamper tests prove field coverage |
| **P7** | Browser engine + agent action surface (Servo behind a feature flag) | Mock engine passes the full action suite; Servo build verified once |
| **P8** | UI/UX (Iced) end-to-end | Manual scripted walkthrough + UI state-machine tests |
| **P9** | Dataset, corpus, eval harness, metrics, methodology doc | `just eval` produces a metrics table with CIs from a real corpus |

Do not start a phase until the prior gate is green and committed.

---

## 4. Target architecture

```
ferrite/
├── Cargo.toml                  # workspace + [workspace.dependencies] (single version source)
├── justfile                    # the ONLY documented command surface
├── rust-toolchain.toml         # pinned stable
├── deny.toml                   # cargo-deny: licenses, bans, duplicate-version bans
├── .cargo/config.toml          # profiles, linker, shared target dir
├── CLAUDE.md                   # ≤120 lines, invariants + pointers, NO status (see §12)
├── README.md                   # what it is TODAY, 1 page
├── docs/
│   ├── ARCHITECTURE.md         # the one design source of truth
│   ├── DECISIONS.md            # numbered, dated ADRs (carry forward the real ones)
│   ├── EVALUATION.md           # objectives, formulas, corpus sizing, parameter rationale
│   ├── BUILD_BUDGET.md         # measured disk/time numbers, how to keep them
│   ├── TO-DO.md                # generated-from-reality task list, IDs stable
│   ├── PROGRESS.md             # append-only dated log, each entry cites SHAs + tests
│   ├── handoffs/<agent>.md
│   └── archive/                # old docs, moved not deleted, with a README saying "historical"
├── crates/
│   ├── ferrite-core/           # types, IDs, capability↔primitive taxonomy, errors, Clock, config
│   ├── ferrite-model/          # ModelProvider trait + mock/ollama/gemini backends
│   ├── ferrite-ipi/            # fingerprint, sanitizer, twin, dry-run, comparator, consent logic
│   ├── ferrite-audit/          # hash chain + persistence
│   ├── ferrite-engine/         # BrowserEngine trait + MockEngine (always built)
│   ├── ferrite-engine-servo/   # Servo impl, OPTIONAL feature `engine-servo`
│   ├── ferrite-agent/          # agent loop: plan → act, consumes ferrite-model + ferrite-engine
│   ├── ferrite-ui/             # Iced shell, consent panel, sidebar
│   ├── ferrite-eval/           # dataset, corpus loader, harness, adjudication, metrics
│   └── ferrite-cli/            # headless binary: run a task, run the eval, dump audit
└── corpus/                     # JSON case definitions, versioned, schema-validated in CI
```

Dependency direction is strictly downward: `core ← {model, audit, engine} ←
ipi ← agent ← {ui, eval, cli}`. No cycles, no upward imports, no
`ferrite-ui` types leaking into `ferrite-ipi`. Enforce with a CI check
(`cargo tree -p` assertions or a small script).

**Delete outright in P0** (done by A0): `ferrite-capability-broker`,
`ferrite-policy`, `ferrite-sandbox`, `Browser/.rules`, `Browser/commands.md`,
the `9222` port-forward in `.devcontainer/devcontainer.json`, and any
`ferrite-types`/`ferrite-network`/`ferrite-a11y`/`ferrite-cef` references.
`paper/` moves to `docs/archive/paper/` untouched.

---

## 5. Session protocol for every agent

Start each agent session with exactly this shape:

```
You are Agent <ID> — <name>.
Read: docs/REBUILD_DIRECTIVE.md §<charter>, docs/handoffs/<previous>.md, and these files only: <list>.
Do not read anything else without saying why first.
Your scope: <paths you may write>.
Work test-first. Commit at every green cycle. Stop at your exit gate and write docs/handoffs/<ID>.md.
```

End each session by:
1. Running `just check && just test`.
2. Appending a dated `PROGRESS.md` entry citing commit SHAs and the test
   names that prove each claim.
3. Updating `TO-DO.md` (close IDs, add newly discovered ones — never
   silently drop one).
4. Writing the handoff.

---

## 6. Agent charters

Each charter below is a complete session brief. Run them in order. Do not
merge two charters into one session.

### A0 — Archivist (Phase P0)
**Reads:** the existing doc set, `Cargo.toml` files, `git log --oneline -50`.
**Writes:** `docs/AUDIT.md`, `docs/archive/**`, deletions, `docs/TO-DO.md`,
empty `docs/PROGRESS.md`, hooks, `CLAUDE.md` rewrite.
**Does:**
- Produce `docs/AUDIT.md`: for every file in the current tree, one line —
  `keep` / `rewrite` / `archive` / `delete`, with the reason. This is the
  demolition plan; execute it in the same session.
- Extract every genuinely-true design decision from
  `FINALIZED_DECISIONS.md` into `docs/DECISIONS.md` as numbered ADRs with
  dates. Drop the rest.
- Rebuild `TO-DO.md` from scratch as `T-###` IDs derived from §9 defect
  register + §6 charters. Stable IDs, never renumbered.
- `PROGRESS.md` starts empty with a header stating the format contract.
- Install the commit-msg guard hook (§11) and the pre-commit `fmt`/`clippy`
  hook.
**Exit gate:** `cargo metadata` succeeds, no reference to broker-era crates
survives `rg -i 'capability.broker|regorus|extism|9222|ferrite-(types|network|a11y|cef)'`,
everything committed.

### A1 — Foundation (P1)
**Scope:** `Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`,
`justfile`, `deny.toml`, `.github/workflows/`.
**Does:** workspace with `[workspace.dependencies]`; build profiles per §7;
`justfile` with `check`, `test`, `test-fast`, `test-live`, `lint`, `fmt`,
`audit`, `bloat`, `eval`, `run`, `build-servo`, `clean-cache`, `disk`; CI
matrix (macOS + Windows + Linux) that skips Servo by default and builds it
on a weekly schedule only; lint gates (`clippy -D warnings`, `rustfmt`,
`cargo-deny`, `cargo-machete`).
**Exit gate:** clean clone → `just test` green in under 5 minutes,
`just disk` reports the target-dir size, CI green.

### A2 — Core contracts (P2)
**Scope:** `crates/ferrite-core`.
**Does:** the capability/primitive taxonomy (§8), newtype IDs (`CaseId`,
`ExecId`, `PrincipalId`, `Origin`), `OriginScope` with *per-capability*
scoping as a first-class type, `Clock` trait, error enums (`thiserror`),
config loading with env overrides, serde round-trip + schema-stability
tests. No logic beyond types and their invariants.
**Exit gate:** every public item has a doc comment; round-trip and
invariant property tests green; `cargo doc` warning-free.

### A3 — Model layer (P3)
**Scope:** `crates/ferrite-model`. See §10 — read it in full before writing
code; the rate-limit design there is load-bearing, not advisory.
**Exit gate:** the provider conformance suite passes identically against
`MockProvider` and `ReplayProvider`; `just probe` completes one live Ollama
Cloud round-trip by hand; the response cache demonstrably returns a hit on
the second identical call (test asserts the underlying HTTP client was
called once); the call-budget guard aborts a run when exceeded (test
asserts it).

### A4 — Fingerprint engine (P4)
**Scope:** `crates/ferrite-ipi/src/fingerprint/`.
**Does:** rebuild the hybrid engine — deterministic rule layer (keyword →
`must_use`) plus model-predicted `may_use`, filtered against the closed
capability allowlist, disjoint by construction, failing open-to-empty on
any provider error (empty fingerprint ⇒ everything routes to consent, never
a bypass). `js.execute` is structurally unrepresentable as an expected
capability — enforce with the type system, not a runtime check, if
possible (separate `ExpectedCapability` enum that simply has no such
variant).
**Tests:** table-driven over prompts; adversarial prompts that try to make
the model emit out-of-allowlist labels; provider-failure injection;
empty-fingerprint routing.

### A5 — Sanitizer (P4)
**Scope:** `crates/ferrite-ipi/src/sanitizer/`.
**Does:** pattern set as *data* (a versioned, tested `PatternSet` with IDs,
not five hardcoded regexes inline), running over the three carriers —
visible text post-`ammonia`, HTML comments extracted pre-sanitization, and
recursively-walked JSON tool-output leaves with path tagging
(`results[0].description`). `<script>` gets an additional pattern set.
**Detection and excision stay independently toggleable**, but excision must
now actually be wired live behind config, with sentence-segment
granularity, HTML tag-boundary safety, and replacement by a single space
(never a marker — a marker is itself steerable text).
**Tests:** golden corpus of positives/negatives per pattern; excision must
never produce malformed HTML (property test with an HTML parser assertion);
false-strip rate measured over a benign fixture set and asserted below the
configured ceiling.
**Write down the ceiling honestly:** literal-phrase patterns miss paraphrase
by construction. The architecture, not detection, carries the security
argument. Put that sentence in the module docs.

### A6 — Dry-run + twin + containment (P4)
**Scope:** `crates/ferrite-ipi/src/dry_run/`, `twin/`, `containment/`.
**Does:** `RecordingExecutor` producing an *ordered*
`Vec<ToolEvent { primitive, origin, seq }>` with origin seeded from the
task's context URL and updated on every navigation; `DryRunContent`
scripting per-origin ordered response queues (so read #2 can differ from
read #1 — models delayed payloads and redirect chains); per-call inline
detect/strip gating; whole-turn timeout that returns the partial record
rather than discarding it.
**Twin:** keep synthetic PII generation; **remove the hardcoded `DEV_KEY`**.
Key comes from OS keyring, falling back to an env var, failing loudly with
a clear error if neither is present — never a compiled-in constant.
Document plainly that the payload is synthetic, so encryption is hygiene,
not a security claim.
**Containment:** decide and document. Today `intercept_request()` is dead
because `RecordingExecutor` never makes real calls — it is
containment-by-mock. Either (a) delete the netns/interception path entirely
and state "containment is by construction: the dry-run executor has no
network backend", or (b) keep it *only* if a real-executor dry-run backend
is on the near-term plan, with a test that actually exercises it. Default
to (a). Dead defense-in-depth is worse than none, because it reads as
protection in the paper.

### A7 — Comparator + origin model (P5)
**Scope:** `crates/ferrite-ipi/src/comparator/`.
**This is the highest-value fix in the rebuild.** Current `compare()` takes
one global `OriginScope` for the whole task, which makes the documented
model inexpressible and leaves `admission_rank()` as dead code.
**New contract:**
```rust
fn compare(
    expected: &ToolFingerprint,          // capabilities, each carrying its OWN OriginScope
    actual:   &DryRunRecord,
) -> FingerprintDiff;
```
- Lowering stays: capability → primitive set, but lowering now yields
  `Vec<(Primitive, &OriginScope, CapabilityId)>`, not one flat union.
- For each recorded event: if primitive is `js.execute` → `extra_primitives`,
  unconditionally, no origin check, no exceptions.
- Otherwise, find **every** expected capability whose primitive set
  contains this primitive *and* whose scope admits this origin; attribute
  the event to the one with the **highest admission rank** (exact `>`
  domain-suffix `>` task-open). Record the attribution in the diff so the
  consent UI and the audit entry can both say *which* capability justified
  the action.
- No admitting capability ⇒ classify precisely: `OutOfScopeOrigin`
  (primitive expected, origin not), `ExtraPrimitive` (primitive not
  expected at all), or `Both`.
- `admission_rank()` must be consumed by `compare()`. If it isn't, the test
  suite fails (write that test).
**Tests:** property tests — monotonicity (widening any scope never turns an
admitted event into a flagged one), determinism, exhaustive small-case
enumeration; explicit multi-capability fixtures (narrow `scoped.read` on
`mail.example.com` + wide `web.read` for search results) that would be
impossible to express under the old signature.

### A8 — Audit log (P6)
**Scope:** `crates/ferrite-audit`.
**Fixes the real gap:** today
`hash = SHA256(sequence, timestamp, kind, principal_id, prev_hash)` — the
`capability` and `url` fields are *not* covered, so the `exec_id`/`case_id`
payload on `EvalExecutionRecorded` entries can be rewritten in SQLite and
`verify_chain()` still reports intact. That directly contradicts the
"cryptographically demonstrate exactly what the agent did" claim.
**New:** hash covers a canonical, versioned serialization of the *entire*
entry payload. Include a `schema_version` byte in the preimage.
Domain-separate the hash (`b"ferrite-audit-v1"` prefix).
**Tests:** tamper matrix — mutate each field in turn, assert
`verify_chain()` fails for every one; truncation, reordering, and insertion
attacks; a golden chain fixture that pins the hash output so an accidental
preimage change breaks CI.

### A9 — Engine + agent action surface (P7)
**Scope:** `crates/ferrite-engine`, `crates/ferrite-engine-servo`,
`crates/ferrite-agent`.
**Does:** define `BrowserEngine` as a trait with a full agentic action
surface, and implement it twice — `MockEngine` (always built, deterministic,
backs every test) and `ServoEngine` (behind `--features engine-servo`).
Action surface, minimum: `navigate`, `go_back`/`go_forward`, `reload`,
`current_url`, `dom_snapshot` (accessibility-tree-style, not raw HTML),
`query` (selector → element handles), `read_text`, `click`, `type_text`,
`fill_form`, `select_option`, `scroll`, `wait_for` (selector/idle/timeout),
`screenshot`, `download`, `tabs` (open/close/switch), `cookies_read`
(scoped), `storage_read` (scoped), `clipboard_read`/`clipboard_write`, and
`js_execute` (privileged, unscopable, always consent-gated).
Every action returns `(result, origin_at_time_of_action)` so the recorder
never has to guess.
**Agent loop:** plan → select tool → act → observe → repeat, with a step
budget, a wall-clock budget, and a hard stop on repeated identical actions.
The loop is engine-agnostic and provider-agnostic.
**Exit gate:** full action-suite conformance tests pass against
`MockEngine`; `just build-servo` succeeds once on this machine and the
wall-clock + disk cost is recorded in `docs/BUILD_BUDGET.md`; the same
conformance suite is run against `ServoEngine` at least once and the result
recorded.

### A10 — UI/UX (P8)
**Scope:** `crates/ferrite-ui`.
**Does:** Iced shell — address bar, content view, task sidebar, and the
consent panel. The consent panel is the security surface, so treat it as
such:
- Rendered **fully decoupled from page content**; page-controlled text can
  never style, position, or occlude it.
- Plain-English diff summary per flagged item: *what* primitive, *which*
  origin, *which* capability would have justified it (or "nothing in your
  request did"), and what happens on approve vs reject.
- Per-item approve/reject, not one blanket button. Reject is the default
  focus. No "approve all" shortcut.
- Show the dry-run evidence (what the agent read that changed its
  behavior) on expand.
- Approvals are per-task and never sticky across tasks.
**Tests:** UI state-machine tests over the message enum (no rendering
needed) covering every consent path; a snapshot test of the summary text
for a fixture diff; a test that a rejected primitive is actually blocked by
the real executor.

### A11 — Dataset + corpus (P9)
**Scope:** `crates/ferrite-eval/src/dataset/`, `corpus/`.
**Does:** two-layer schema preserved — `CaseDefinition` (authored,
invariant) and `ExecutionRecord` (one per case × mode). **Move the
carrier/carrier-vector partition validation into the type system**
(separate enums per carrier, or a sealed constructor) so a hand-built Rust
literal cannot violate it the way it can today when validation only happens
at JSON load. Keep derived fields (`unscopable_primitive_invoked`,
`production_residual`) as functions, never stored columns. Persist via
`rusqlite` with scalar columns for filtering plus a JSON blob as source of
truth; add a schema-migration test.
**Corpus:** build it for real — see §13 for the target counts and the
authoring protocol.

### A12 — Harness + metrics (P9)
**Scope:** `crates/ferrite-eval/src/harness/`, `adjudication/`, `metrics/`,
`docs/EVALUATION.md`.
**Does:** the 4-mode run matrix enforced in code (not prose); adjudication
with the corrected semantics from §9; a metrics module that computes every
formula in §13 with Wilson confidence intervals and paired McNemar tests; a
reproducible report generator (`just eval` → markdown table + CSV + the
audit anchors).
**Exit gate:** `just eval` on the real corpus produces the full metrics
table; `docs/EVALUATION.md` is complete per §13.

### A13 — Reconciliation & release (P9)
**Scope:** all docs, `CLAUDE.md`, `README.md`, version tagging.
**Does:** final pass asserting every doc claim against a test or SHA;
regenerate `TO-DO.md`; tag `v0.1.0`; write the honest limitations section
(irreducible blind spot, pattern ceiling, consent-policy upper bound,
single-machine eval).

---

## 7. Build, disk, and performance budget

The constraint: full local development on one laptop, without giving up
runtime performance. The single biggest lever is **not building Servo for
95% of the work**.

**7.1 Servo is optional and off by default.**
`ferrite-engine-servo` is a separate crate behind feature `engine-servo`,
not in the default feature set, not in the default `just test` path.
`MockEngine` backs every test and the whole defense loop. Only
`just build-servo` and `just run --servo` pull it in. Expect the Servo
build to cost tens of GB and 30–60 minutes on first build — budget it
deliberately, once, and record the real numbers.

**7.2 Profiles** (`.cargo/config.toml` + root `Cargo.toml`):
```toml
[profile.dev]
opt-level = 0
debug = "line-tables-only"     # full debuginfo is the #1 target-dir bloater
incremental = true
codegen-units = 256
split-debuginfo = "unpacked"   # macOS: keeps debug data out of the binaries

[profile.dev.package."*"]
opt-level = 2                  # deps built once, fast at runtime; your crates stay fast to rebuild

[profile.test]
inherits = "dev"

[profile.release]
opt-level = 3
lto = "thin"                   # "fat" costs minutes for single-digit % — thin is the right trade
codegen-units = 1
strip = "debuginfo"
panic = "unwind"               # keep unwind: panic=abort breaks integration tests and harness recovery

[profile.bench]
inherits = "release"
debug = "line-tables-only"
```

**7.3 Dependency hygiene** (this is where the GBs actually are):
- One `[workspace.dependencies]` table. Every crate uses `dep.workspace =
  true`. No version drift.
- `default-features = false` on **everything**, then add back only what's
  used.
  - `tokio`: `["rt-multi-thread", "macros", "time", "sync"]` — not `"full"`.
  - `reqwest`: `["rustls-tls", "json"]`, no default (drops the entire
    OpenSSL/native-tls tree).
  - `serde`: `["derive"]`; `serde_json` plain.
  - `chrono`: `["clock"]` only, or prefer `time`/`jiff` if the API suffices.
  - `rusqlite`: `["bundled"]` is fine — small C build, removes a system
    dependency.
  - `iced`: evaluate the software renderer (`tiny-skia`) for dev builds —
    it drops the `wgpu` + `naga` tree, which is a large share of UI build
    time. Keep the GPU backend for release if it measurably matters.
  - `regex`: default is fine; avoid pulling `fancy-regex` unless
    backreferences are genuinely needed (they aren't for these patterns).
- `cargo-deny` with `[bans] multiple-versions = "deny"` plus a small,
  explicitly-justified skip list. Duplicate major versions are silent disk
  and compile cost.
- `cargo machete` in CI to catch unused deps; `cargo tree -d` reviewed at
  every phase gate.
- Before adding *any* new dependency, check `cargo bloat`/`cargo tree` cost
  and justify it in the commit message. Prefer 40 lines of your own code to
  a 60-crate tree.

**7.4 Disk management:**
- Shared target dir: `CARGO_TARGET_DIR=~/.cache/ferrite-target` (documented
  in the justfile and README, set via `.cargo/config.toml`
  `build.target-dir`).
- `just disk` prints `du -sh` of the target dir, per-profile breakdown, and
  the top 20 largest artifacts.
- `just clean-cache` runs `cargo sweep -t 7` (stale artifacts only) — never
  a blanket `cargo clean`.
- Optional `sccache` with `SCCACHE_CACHE_SIZE=10G` capped — document the
  trade (it costs disk to save time).
- **Target: < 12 GB target-dir and < 5 min cold `just test` without
  Servo.** Record actual numbers in `docs/BUILD_BUDGET.md` at every phase
  gate; if a number regresses by >20%, fix it before moving on.

**7.5 Test speed:** `cargo nextest` for parallel execution; unit tests pure
and in-process; integration tests share fixtures via `once_cell`; no
sleeps (use the injected `Clock`); no `tokio::time::sleep` in tests (use
`tokio::time::pause`).

---

## 8. Capability and primitive taxonomy (define once in `ferrite-core`)

**Capabilities** (what a *task* may legitimately need — the closed
allowlist the model is filtered against): `web.read`, `web.navigate`,
`web.interact`, `web.download`, `scoped.read`, `clipboard.read`,
`clipboard.write`.

**Primitives** (what the executor actually performs): `navigate`,
`dom.read`, `dom.query`, `dom.write`, `form.fill`, `click`, `scroll`,
`wait`, `download`, `tab.open`, `tab.close`, `cookie.read`, `storage.read`,
`clipboard.read`, `clipboard.write`, `screenshot`, `js.execute`.

**Lowering table** is data, not code — a single `const` table with an
exhaustiveness test, so adding a primitive without assigning it to a
capability fails to compile or fails a test.

**`js.execute` is UNSCOPABLE**: it can synthesize any other primitive, so
it can never be admitted by any capability, at any origin, under any scope.
It is always flagged, always consent-gated. Encode this so it cannot be
forgotten: `ExpectedCapability` lowering simply never produces it, and the
comparator's first branch handles it before any origin logic.

---

## 9. Defect register — fix every one of these

| ID | Defect | Required fix |
|---|---|---|
| D1 | `compare()` takes one global `OriginScope`; per-capability scoping from the design docs is inexpressible | A7 — scope lives on each capability; attribution by highest admission rank |
| D2 | `OriginScope::admission_rank()` is computed and never consumed — dead code | A7 — consumed by `compare()`; test asserts it |
| D3 | Sanitizer excision built, tested, and off everywhere (`strip_enabled` never set true) → `On` and `LoopOnly` are behaviorally identical in production while being presented as distinct defense conditions | A5 — excision wired live behind config, with a measured benign false-strip rate gating the default |
| D4 | `final_outcome` in `On` mode ignores `sanitizer_caught`; once excision is live, a successfully-stripped attack reports as `Executed` — inverting the headline metric | A12 — outcome lattice made explicit: `Stripped` / `ContainedViaConsent` / `Executed` / `NotAttempted`, with a table-driven test over every (caught, gated) combination |
| D5 | Audit hash omits `capability` and `url`, so `exec_id`/`case_id` can be rewritten with the chain still verifying | A8 — full canonical payload in the preimage + tamper matrix tests |
| D6 | `CarrierVector` partition validated only at JSON load; hand-built Rust literals bypass it | A11 — enforce in the type system |
| D7 | `intercept_request()` unreachable outside its own test; containment is by mock, not interception | A6 — delete the inert path or exercise it for real; document which |
| D8 | Twin AES key is a compiled-in constant (`ferrite-ipi-twin-dev-key-32byte!`) | A6 — keyring/env, fail loudly, no constant |
| D9 | AgentDojo adapter is enum plumbing only (`Tier3AgentDojo`, `RunLabel::R9`) with no Slack-suite mapping | A11 — either implement the mapping or delete the enum variants; no placeholder variants |
| D10 | Simulated user hard-pinned to `RejectFlagged` | A12 — make consent policy a swept parameter (`RejectFlagged`, `ApproveAll`, `RandomP(p)`); report `RejectFlagged` explicitly as an *upper bound on human vigilance*, not an estimate |
| D11 | `CLAUDE.md` listed five completed tasks as "not yet implemented"; commit 46b2177 built them and edited the same file without fixing the claim | A0 + §12 — CLAUDE.md carries no status at all (fixed by A0) |
| D12 | `.rules`, `README.md`, `commands.md`, `devcontainer.json` described the dead broker architecture; `.rules` is auto-ingested by AI tools, i.e. self-inflicted context poisoning | A0 — deleted/rewritten (fixed by A0) |
| D13 | Docs assume Windows/PowerShell/single dev machine; reality is multi-OS, multi-contributor | A1 — justfile as the only command surface, OS-neutral docs |
| D14 | `paper/PAPER_STATUS.md` tracks past deadlines against a track `TO-DO.md` says is parked | A0 — archived (fixed by A0) |

---

## 10. Model provider layer (`ferrite-model`)

**Default backend is Ollama Cloud.** All development, manual runs, and eval
runs go through it. Local Ollama is the same wire protocol at a different
base URL with no auth header, so it is the same backend behind one config
switch, kept as an offline fallback. Gemini is the second remote backend.
OpenAI/Anthropic are one file each, later.

### 10.1 Wire contract

```
POST https://ollama.com/api/chat
Authorization: Bearer $OLLAMA_API_KEY
Content-Type: application/json

{
  "model":    "<tag from /api/tags>",
  "messages": [{"role": "system", ...}, {"role": "user", ...}],
  "stream":   false,
  "format":   { ...JSON schema... },        // structured output — see 10.4
  "options":  { "temperature": 0, "seed": 42, "num_predict": 128 }
}
```
Answer is read from `message.content`; token counts come back as
`prompt_eval_count` / `eval_count` and feed the overhead metric in §13.2.
Local Ollama is the identical body against `http://localhost:11434/api/chat`
with no `Authorization` header.

**The API key comes from the `OLLAMA_API_KEY` environment variable or the
OS keyring, and nowhere else.** Not in `config.toml`, not in a `.env` that
is committed, not in a default, not in a test fixture, not in a log line.
Add `OLLAMA_API_KEY` to a secret-scan pre-commit check.

### 10.2 Model selection — configured, never hardcoded

- Cloud tags for direct API requests come from
  `curl https://ollama.com/api/tags` and include a size, e.g. `gemma4:31b`.
  The `:cloud` suffix form (`gemma4:cloud`) is for the Ollama app/CLI only —
  do not use it against `ollama.com/api/chat`.
- **Ollama retires cloud models.** So: no model name appears in Rust
  source. Models are config values with env overrides
  (`FERRITE_MODEL_SMALL`, `FERRITE_MODEL_MAIN`). Add `just models` (hits
  `/api/tags`, prints the list) and a **startup preflight** that validates
  each configured tag against `/api/tags` and fails with the available list
  rather than dying mid-run on a 404.
- **Two tiers, because the two workloads are not alike:**
  - `FERRITE_MODEL_SMALL` — the fingerprint `may_use` prediction. Input is
    one short prompt plus a 7-item capability list; output is a short JSON
    array. A small instruct model is correct here, and `num_predict` is
    capped around 128. This is the call that runs on every task, so it is
    the one that must be cheap.
  - `FERRITE_MODEL_MAIN` — the agent loop's plan/act reasoning, where tool
    selection quality actually matters. A mid-size cloud model (the docs'
    `gemma4:31b` class is a reasonable starting point). Cap `num_predict`
    per step anyway.
  - Record which tier and tag produced every decision in the
    `ExecutionRecord`, so a model swap mid-corpus is visible in the data
    instead of being a silent confound. If a configured tag is retired
    partway through a corpus run, that run is invalid — say so in
    `EVALUATION.md` and re-run the affected cells.

### 10.3 Rate-limit discipline — the actual design constraint

Do the arithmetic before designing, because it determines the architecture:
~360 cases × 4 modes ≈ **900 executions per full eval run**, and each
execution costs one fingerprint call plus one model call per agent step
(~3–5). That is **3,000–4,500 calls per run** if built naively, which will
exhaust the quota on the first attempt. Four mechanisms, all required:

1. **Content-addressed response cache (the big one).** Key =
   `SHA256(provider, model_tag, temperature, seed, system_prompt_version,
   canonical_messages_json, format_schema)`. At `temperature = 0` the
   response is a pure function of that key, so caching is sound rather than
   a fudge. Effects: the fingerprint is identical across all four defense
   modes for a given case, so 900 fingerprint calls collapse to ~360;
   agent-loop calls are keyed on the whole conversation state, so identical
   prefixes across modes hit too. A second full run of an unchanged corpus
   costs **zero calls**. Cache lives at `~/.cache/ferrite-model/`, is
   content-addressed on disk, and has a `just cache-stats` target reporting
   hit rate — a low hit rate is a bug, investigate it.
2. **Record/replay fixtures.** A `just record` mode writes every live
   response into `tests/fixtures/model/<hash>.json`. These **are
   committed**. `ReplayProvider` serves them in integration tests and in
   CI, so the suite is free, deterministic, and offline — satisfying R7
   while still exercising real model output rather than hand-written
   mocks. A replay miss is a hard failure with the missing key printed,
   never a silent fallthrough to the network.
3. **Throttle + backoff.** Global semaphore capping in-flight requests
   (default 2), a token-bucket limiter with a configurable rate,
   exponential backoff with full jitter on `429`/`5xx`, and a per-request
   timeout. Respect `Retry-After` if present.
4. **Hard call budget.** `FERRITE_MODEL_CALL_BUDGET` (default 500 per
   process). On exhaustion the run **aborts with a clear error and a
   partial-results file** rather than silently continuing or quietly
   burning through the rest of your quota. Every run prints
   calls-used / cache-hits / tokens at exit.

Also: cap `num_predict` on every call, never send the page content
wholesale into the fingerprint call (it takes the *user's* prompt, not the
page), and keep system prompts short and versioned — bumping the version
invalidates the cache deliberately, which is what you want when the prompt
changes.

### 10.4 Trust boundary — model output is never trusted as text

- Use Ollama's **structured output**: pass a JSON schema in `format`, so
  the model is constrained to shape rather than asked politely for it.
  Then parse into typed values, then **filter against the closed
  7-capability allowlist**. A label outside the allowlist is dropped at the
  boundary — the model *cannot* express one, not because we told it not to.
- **Fail to empty**, always: timeout, 429, malformed JSON, schema
  violation, budget exhaustion, empty body — every one yields an empty
  `may_use` set. An empty fingerprint routes everything through consent.
  Degrade to maximum safety, never to a bypass, and never to a block.
- Test this with an adversarial `MockProvider` returning: valid-but-out-of-
  allowlist labels, prompt-injection text in the response body, truncated
  JSON, an empty array, a 10 MB response, and a hang. All six must produce
  a safe, bounded, logged outcome.

### 10.5 Trait shape

```rust
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError>;
    fn capabilities(&self) -> ProviderCapabilities; // json schema support, context window, tiering
}
```
Backends: `MockProvider`, `ReplayProvider`,
`OllamaProvider { base_url, auth: Option<Token> }` (covers both cloud and
local), `GeminiProvider`. Cache, throttle, budget and telemetry are
**decorators** wrapping any provider — `Budget<Throttle<Cache<Ollama>>>` —
not features baked into each backend. One conformance suite every backend
must pass. No provider type may appear in a `ferrite-ipi` or `ferrite-agent`
signature.

## 11. Git protocol

- Branch per agent: `rebuild/a<NN>-<slug>`. Merge to `main` at each exit
  gate, fast-forward or squash — your choice, but consistent.
- Conventional Commits; body explains *why*, not *what*; reference `T-###`
  IDs.
- **Commit-msg guard hook** (installed in P0 — `scripts/hooks/commit-msg`,
  wired via `git config core.hooksPath scripts/hooks`,
  `scripts/hooks/install.sh` does this for a fresh clone):

```sh
#!/bin/sh
# scripts/hooks/commit-msg — reject AI attribution and boilerplate trailers
if grep -qiE 'co-authored-by:.*(claude|anthropic|copilot)|generated with|🤖|ai-assisted' "$1"; then
  echo "commit-msg: AI attribution is not permitted in commit messages." >&2
  exit 1
fi
```
Also verify no global git template or tool setting is injecting trailers
(`git config --get commit.template`, `git config --global --list | grep -i
trailer`).
- Pre-commit hook: `cargo fmt --check` + `cargo clippy -D warnings` on
  staged crates.
- Never commit: `target/`, corpus DB files, `.env`, API keys, `~/.cache`
  paths.

---

## 12. Documentation contract (this is how drift gets killed for good)

**`CLAUDE.md` — ≤120 lines, and it contains ZERO status claims.** Its only
permitted contents:
1. One-paragraph description of what the project is *today*.
2. Build/test/run commands — all via `just`, OS-neutral.
3. Architectural invariants that must never be violated (the `js.execute`
   rule, fail-to-empty, no network in tests, dependency direction, no AI
   attribution in commits).
4. Pointers: "for current status read `docs/PROGRESS.md`; for tasks read
   `docs/TO-DO.md`; for design read `docs/ARCHITECTURE.md`."
5. A literal line: *"This file must never contain a 'planned' or 'not yet
   implemented' list. Status lives only in PROGRESS.md and TO-DO.md."*

Rationale, stated plainly in the file itself: CLAUDE.md is what every
future session trusts first, so it must contain only things that change
when the architecture changes — not things that change when a task
completes. The current failure (a single commit that both implemented five
features and edited CLAUDE.md while leaving "not yet implemented" standing)
is a structural bug in the doc layout, not a discipline failure.

**`PROGRESS.md`** — append-only, dated, newest last. Each entry: what
landed, commit SHAs, test names proving it, known issues discovered. The
existing team habit of logging their own known-issues in dated entries is
good — keep it, it's the most reliable part of the current doc set.

**`TO-DO.md`** — stable `T-###` IDs, status enum (`open` / `in-progress` /
`done <SHA>` / `dropped <reason>`), phase, owning agent. Nothing gets
silently deleted.

**Doc CI check** — add a script that fails the build if: `CLAUDE.md`
contains "not yet implemented" / "planned" / "TODO"; any doc names a crate
that isn't in `cargo metadata`; any crate in the workspace isn't mentioned
in `ARCHITECTURE.md`. Cheap, and it makes D11/D12 structurally impossible
to repeat. (Implemented by A0 as `scripts/check_no_archive_links.sh` and
`scripts/check_purge.sh` — the `ARCHITECTURE.md`-completeness check is
deferred until that file exists, per A0's handoff.)

---

## 13. Evaluation methodology (`docs/EVALUATION.md`) — first-class
deliverable

Write this document to be read by an examiner. Three required sections, in
this order.

### 13.1 Objectives
State each as a falsifiable claim with the metric that tests it:
- **O1 (Containment).** The loop reduces attack success rate versus no
  defense, for attacks whose effect is a deviation from the predicted
  fingerprint. → ASR, paired across modes.
- **O2 (Attribution).** Every executed action is attributable to a specific
  admitting capability and origin, and the audit chain commits to that
  attribution. → tamper tests + attribution coverage.
- **O3 (Utility).** Benign tasks complete unchanged, and consent burden
  stays below a stated threshold. → FGR, task-completion parity.
- **O4 (Honest limits).** The blind spot is characterized, not hidden:
  same-origin/same-primitive, data-only attacks
  (`WithinFingerprintDataOnly`) are *undetectable by construction* and are
  reported as a named residual, with their share of the corpus stated.
- **O5 (Cost).** The defense's latency and token overhead are measured, not
  assumed.

### 13.2 Metrics — exact formulas
For each defense mode *m* ∈ {Off, LoopOnly, SanitizerOnly, On}:

- Attack Success Rate: **ASR_m = |{attack cases with final_outcome =
  Executed}| / N_attack**
- Containment Rate: **CR_m = 1 − ASR_m**
- Architectural Detection Rate (deviation-class cases only): **ADR_m =
  |fingerprint_caught| / N_deviation**
- Sanitizer Detection Rate (cases with an authored `expected_finding`):
  **SDR_m = |sanitizer_caught| / N_expected_finding**
- False Gate Rate (consent burden, benign only): **FGR_m = |benign cases
  gated| / N_benign**
- False Strip Rate: **FSR = |benign segments excised| / |benign segments|**
- Utility Preservation: **UP_m = |benign tasks whose action trace is
  unchanged vs Off| / N_benign**
- Residual: **R = |WithinFingerprintDataOnly cases| / N_attack** — reported
  as a structural floor on ASR, not as a failure
- Overhead: **ΔL = latency_On − latency_Off** (p50 and p95, separately) and
  **ΔT = extra model tokens per task**

Every proportion is reported with a **Wilson score 95% interval**, never a
bare percentage:
**CI = (p̂ + z²/2n ± z·√(p̂(1−p̂)/n + z²/4n²)) / (1 + z²/n)**, z = 1.96.

Mode comparisons are **paired** — the same case runs under multiple modes —
so use **McNemar's exact test** on the discordant pairs (b, c):
**p = 2·Σ_{i≥min(b,c)} C(b+c, i)·0.5^(b+c)**, with **Holm–Bonferroni**
correction across the mode-pair family. Report **Cohen's h** for effect
size. Never report a mode difference from unpaired proportions.

### 13.3 Corpus sizing — derive it, don't guess
Sample size per cell comes from the target CI half-width *w* at 95%:
**n ≈ z²·p̂(1−p̂)/w²**. At the worst case p̂ = 0.5: *w* = 0.15 → n ≈ 43;
*w* = 0.10 → n ≈ 96.

Proposed corpus (justify any deviation in the document):

| Stratum | Carrier | Target n | Rationale |
|---|---|---|---|
| Tier 1 — authored web content | WebContent | 120 | 5 attack categories × ~24; supports per-category CI half-width ≈ 0.20 and pooled ≈ 0.09 |
| Tier 2 — authored tool output | ToolOutput | 80 | tests the T1b generalization (same detector, different walker) at pooled *w* ≈ 0.11 |
| Tier 3 — externally-derived (AgentDojo) | Mixed | 60 | external validity check; not powered for per-category claims, and say so |
| Benign control | Both | 100 | FGR/FSR at *w* ≈ 0.10 — the utility claim needs as much power as the security claim |

Total ≈ 360 cases; with the §-matrix skips (benign has no Off/LoopOnly
cell; attack SanitizerOnly only for Tier 1/2) that is roughly 900–1000
executions per full run. Every case is authored with a `ground_truth`
label; **10% are independently double-authored and inter-rater agreement is
reported as Cohen's κ, with κ ≥ 0.8 as the acceptance bar.**

### 13.4 Parameter rationale — one row each, with the reason
| Parameter | Value | Why this value |
|---|---|---|
| Model backend | Ollama Cloud (`ollama.com/api/chat`) | Open-weight models, no local GPU or multi-GB download needed, and the same wire protocol as local Ollama so the fallback is a config switch |
| Model tiering | small for `may_use`, mid for the agent loop | The fingerprint call is a short constrained classification run on every task; loop reasoning is where quality changes outcomes. Sizing each to its job is what keeps the run inside quota |
| Model temperature / seed | 0, fixed seed | Reproducibility; the fingerprint must be a function of the prompt, not a sample. It is also what makes the response cache sound rather than a shortcut |
| Response caching | content-addressed, on by default | At temperature 0 a response is a pure function of the request, so caching changes cost by ~10× without changing results; a re-run of an unchanged corpus costs zero calls |
| Call budget | 500 per process, hard abort | A runaway loop must fail loudly at a known cost, not silently consume the whole quota |
| Capability allowlist | closed, 7 labels | The model cannot emit an unforeseen capability; filtering at the boundary makes label injection structurally impossible |
| Provider failure policy | fail to empty | An empty fingerprint routes everything through consent — degrades to maximum safety, never to a bypass |
| `js.execute` | always unscopable | It can synthesize any other primitive, so admitting it at any scope would void every other scope check |
| Dry-run timeout | 30 s | ≈2× observed p99 of dry-run turns; partial records are kept rather than discarded so a timeout is data, not a hole |
| Excision granularity | sentence segment | Smallest unit that removes a complete imperative clause while keeping surrounding text grammatical and the agent's task intact |
| Excision replacement | single space, no marker | A marker is itself text the agent reads and can be steered by; absence is the only neutral replacement |
| Pattern count | 5 labelled patterns | Chosen for precision on literal-phrase attacks; paraphrase is a known, stated miss — the architecture, not detection, carries the security argument |
| Scope precedence | exact > domain-suffix > task-open | Least privilege: the narrowest capability that admits an origin defines the blast radius attributed to it |
| Consent policy (sim) | swept; `RejectFlagged` headline | An attentive-user upper bound on containment, reported as a bound; `ApproveAll` gives the lower bound and the two bracket real behavior |
| Detection vs excision split | independently toggled | Lets `LoopOnly` and `SanitizerOnly` be genuinely distinct conditions; without it, two of four modes are the same experiment |

---

## 14. Definition of done for the whole rebuild

- [ ] `just check && just test` green on a clean clone, with no network and
      `OLLAMA_API_KEY` unset, under 5 minutes, without Servo.
- [ ] Every model call routes through cache + throttle + budget decorators;
      a full eval re-run on an unchanged corpus makes zero live calls;
      `just cache-stats` reports the hit rate.
- [ ] `just build-servo` succeeds and the action-conformance suite passes
      against `ServoEngine` at least once, with cost recorded.
- [ ] Every D1–D14 defect closed, each with a test that would fail if
      reverted.
- [ ] Zero dead code, zero `#[allow(dead_code)]` without a comment naming
      the reason and a `T-###`.
- [ ] No doc claims anything a test or SHA doesn't back; the doc CI check
      passes.
- [ ] `CLAUDE.md` has no status section; `.rules`, `commands.md`, broker
      crates and the 9222 forward are gone from the working tree.
- [ ] Corpus authored to the §13.3 targets, κ reported; `just eval` emits
      the full metrics table with Wilson CIs and McNemar results.
- [ ] `docs/EVALUATION.md` complete: objectives, formulas, sizing
      derivation, parameter rationale, honest limitations.
- [ ] `docs/BUILD_BUDGET.md` shows target-dir size and cold-test time at
      every phase gate.
- [ ] No commit in `rebuild/*` or `main` mentions Claude, Anthropic, or AI
      assistance.

---

## 15. Start here

Run **A0 only**. Produce `docs/AUDIT.md` first and show it before executing
the deletions. Then execute, commit, write `docs/handoffs/a0.md`, and stop.

*(A0 has run — see `docs/handoffs/a0.md` and `docs/PROGRESS.md`'s
2026-09-18 entry. Next: A1, per the amendments' explicit instruction not to
auto-continue without a fresh go-ahead.)*
