# Ferrite — Progress Log

> Append-only, dated, newest entry last. Started empty at the P0 rebuild
> (`docs/REBUILD_DIRECTIVE.md`) — for pre-rebuild history see
> `docs/archive/PROGRESS.md` (banner: historical, do not use as context).

## Format contract (every entry follows this shape — R2/R3)

```
## YYYY-MM-DD — <agent id> — <one-line summary>

**Landed:** what actually shipped, in plain language.
**Commits:** `<sha>` `<sha>` ... (one per green cycle, per R3)
**Tests:** `<test_name>` `<test_name>` ... — the tests that would fail if this were reverted (R1/R2)
**Known issues discovered:** anything found but not fixed here, with the T-### it was filed as (docs/TO-DO.md)
```

A claim with no cited test name or commit SHA does not belong in this file —
write "not implemented" instead (R2). This is the habit the old
`Browser/PROGRESS.md` (now archived) got right and the new format keeps.

---

## 2026-09-18 — A0 (Archivist) — repo flattened, dead architecture purged, doc layer reset

**Landed:**
- Flattened the workspace from `Browser/*` to the repo root via pure `git mv`
  (zero content edits), verified with `cargo metadata --no-deps` before/after
  showing an identical 7-member package set and identical per-package
  dependency/version/edition/feature signatures.
- Deleted `ferrite-capability-broker`, `ferrite-policy`, `ferrite-sandbox`
  (dormant, never workspace members) and `extensions/hello-ext` (their only
  consumer) — the dead capability-broker/Wasm-sandbox architecture per
  `docs/REBUILD_DIRECTIVE.md` §4.
- Deleted `.rules` and `commands.md` (described the dead architecture as
  current; `.rules` was the highest context-poisoning risk in the repo as an
  auto-ingested dotfile).
- Archived `EVALUATION_PLAN.md`, `FINALIZED_DECISIONS.md`,
  `PROJECT_REFERENCE.md`, `paper/`, `Resources/*.pdf`, and the old
  `PROGRESS.md`/`TO-DO.md` to `docs/archive/`, each with a `HISTORICAL —
  DESCRIBES A PROJECT THAT NO LONGER EXISTS` banner as its first line, plus
  `docs/archive/README.md` explaining why and a doc-drift check
  (`scripts/check_no_archive_links.sh`) that fails if any live doc links
  into the archive.
- Extracted the genuinely-true design decisions from `FINALIZED_DECISIONS.md`
  / `EVALUATION_PLAN.md` into `docs/DECISIONS.md` as ADR-000 through ADR-008,
  each citing the commit SHA where its implementation actually landed
  pre-rebuild, and each noting inline where the rebuild's defect register
  (D1–D14) identifies a gap between the decision and its old implementation.
- Rewrote `CLAUDE.md` to the zero-status contract (§12): invariants and
  pointers only, no "planned"/"not yet implemented" section.
- Rewrote `README.md` to a one-page "what it is today" description.
- Rewrote `.devcontainer/Dockerfile` and `.devcontainer/devcontainer.json`:
  removed the `ferrite-capability-broker` COPY/fetch (would have failed the
  image build against a now-deleted path), the `wasm32-unknown-unknown`
  target install (dead Wasm-sandbox target), and the `9222` port forward
  (dead JSON-RPC-agent leftover).
- Merged the root and `Browser/` `.gitignore`s into one at the repo root;
  narrowed the old blanket `*.txt` ignore to nothing (removed outright —
  no current need for it, and it silently swallowed any future `.txt`
  fixture).
- Regenerated `docs/TO-DO.md` as a stable `T-###` ledger derived from three
  sources: the D1–D14 defect register, the A1–A13 agent charters, and the
  still-open items in the old `TO-DO.md` (corpus-authoring design doc,
  per-case result inspector, schema-completeness audit, the Phase-6
  freeze-and-run hold, and the Extism/Wasm sandbox work — dropped, not
  silently, with reasons recorded).
- Installed the commit-msg guard hook (rejects AI attribution trailers) and
  a pre-commit `fmt`/`clippy`-on-staged-crates hook under `scripts/hooks/`,
  wired via `git config core.hooksPath`.
- Checked for live contributor branches before the flatten
  (`git branch -a --sort=-committerdate`): only `main` and one 68-day-stale
  Dependabot branch (`origin/dependabot/cargo/Browser/cargo-7c4a9e5e8f`,
  auto-generated, no human activity) — judged safe to proceed without
  further coordination.

Also fixed `.github/workflows/ci.yml`, which the flatten broke: every step
used `working-directory: Browser` / `Browser/**` path filters against a
directory that no longer existed, unconditionally installed a `wasm32`
target for the now-deleted extension sandbox, and had an unreachable
"Build hello-ext Wasm" step gated on a `linux` matrix entry the matrix
never had (windows/macos only) — that step referenced a directory this
same session deleted. Fixed in-session rather than deferred to A1, since
leaving CI broken from a change this session made is worse than touching a
file slightly outside A0's original scope.

Attempted to verify the devcontainer image actually builds
(`docker build -f .devcontainer/Dockerfile .`), not just that
`cargo metadata` resolves — a bad `COPY` path fails at image-build time,
which `cargo metadata` can't see. The Docker daemon isn't available in this
sandbox (CLI present, no running daemon, no Docker Desktop installed), so
the build genuinely failed (`connect: no such file or directory`) — the
background-task notification reported "completed, exit code 0" because the
wrapper script's own last command (an `echo`) succeeded even though the
`docker build` inside it didn't; caught by reading the actual log rather
than trusting the notification. Fell back to exhaustively verifying every
`COPY` source path and every stub-loop crate name in the rewritten
Dockerfile against the real tree by hand — all match — and reporting the
real-build gap honestly instead of claiming a verification that didn't
happen.

**Commits:** `e474403` (flatten, proven content-identical via
`cargo metadata` before/after) → `a45b2ad` (delete dead crates) →
`d670dad` (gitignore merge) → `9e3f097`+`ffe3c63` (archive pass — the
first attempt's `git add -A -- docs/ Resources` silently failed on the
already-moved `Resources/` pathspec and committed pure renames with no
banner content; `ffe3c63` is the fixup that actually landed the banners
and the three new A0 files — see the note below) → `01c3f54`
(CLAUDE.md/README.md rewrite) → `6ccfdad` (devcontainer fix) → `bf29736`
(hooks, both check scripts, `docs/REBUILD_DIRECTIVE.md` saved to disk,
ci.yml fix) → `dc7876e` (docs/TO-DO.md regenerated as the T-### ledger,
check_purge.sh self-exclusion fix).

**Tests:** none — A0 is archaeology/doc/config work, not implementation.
`cargo metadata --no-deps` (before/after diff, see above), `scripts/check_purge.sh`,
and `scripts/check_no_archive_links.sh` (both run clean as of `dc7876e`) are
the closest things to tests this phase has, cited because R2 requires a
citation, not because they're unit tests.

**Known issues discovered, filed as T-### (see `docs/TO-DO.md`):**
- The commit that added the fixup above (`ffe3c63`) is itself evidence for
  why R2 ("no status claim without proof") matters: the first archive commit
  claimed to include content it didn't, and only got caught because this
  entry was written by re-deriving the diff rather than trusting the earlier
  commit message. No T-### filed — this is a demonstrated argument for R2,
  not a defect in the product. The docker-build notification mismatch above
  is the same lesson a second time, from the harness side rather than git.
- T-207: `cargo fmt --check` currently fails on 5 files across
  `ferrite-agent`, `ferrite-eval`, `ferrite-servo`, `ferrite-shell` —
  discovered when the new pre-commit hook correctly blocked a trivial
  one-line comment fix in `ferrite-servo/src/shell.rs` (crate-scoped fmt
  check caught pre-existing, unrelated dirty files). The comment fix was
  reverted rather than forced through by reformatting a crate that A9 is
  going to rewrite anyway; `scripts/check_purge.sh` now carries one
  documented, tracked exception for that file's historical "capability
  broker" mention instead.
- T-208: the 10 pilot-corpus cases are real assets that should seed the
  real corpus, but are not grandfathered — they get re-labelled under the
  post-rebuild `GroundTruth` enum, re-validated against the type-level
  carrier partition (T-006), and count toward the 10% double-authoring /
  κ ≥ 0.8 requirement like any new case (per the session's own amendment,
  now recorded in `docs/TO-DO.md` rather than only in chat).

## 2026-09-18 — A1 (Foundation) — workspace deps, profiles, deny.toml, justfile, CI rework

**Landed:**
- Installed real local tooling to verify against rather than write blind:
  a full `rustup` stable toolchain (only `rustup` itself pre-existed; no
  active toolchain, no `cargo`/`rustc` on `PATH` — symlinked
  `~/.cargo/bin/{cargo,rustc,rustfmt,cargo-fmt,cargo-clippy,clippy-driver,
  rustdoc}` to the rustup-managed binaries, then later added the
  `llvm-tools` component properly), plus `just`/`cargo-deny` (homebrew)
  and `cargo-machete` (`cargo install`). Every claim below was actually
  run on this machine, not inferred from reading the directive.
- `[workspace.dependencies]` (T-101): every crate now uses `dep.workspace
  = true`, including single-consumer deps. `tokio` trimmed `["full"]` →
  `["rt-multi-thread", "macros", "time", "sync"]` (grepped every
  `tokio::` call site first); `chrono` trimmed to `["clock", "serde"]`.
  Added `[profile.dev/test/release/bench]` per §7.2.
- **Closed T-207 for real** (was left open at A0's handoff): `cargo fmt
  --all` (whitespace only) plus documented
  `#[allow(clippy::await_holding_lock)]` on the 10 `ENV_GUARD`-across-
  `.await` sites in `ferrite-eval` — root cause was the old CI's
  `cargo clippy --workspace` lacking `--all-targets`, so it never linted
  test code at all. Whole workspace is now both `cargo fmt --check`- and
  `cargo clippy --workspace --all-targets -- -D warnings`-clean.
- `cargo machete` found one real unused dependency (`ferrite-ui`'s
  `uuid`, zero references, removed) and one false positive (`libservo`,
  only referenced inside `#[cfg(feature = "servo")]`; documented via
  `[package.metadata.cargo-machete] ignored`).
- **`deny.toml` (T-101):** required adding a `license` field (none of the
  7 crates had one — see the flagged decision below) and `publish =
  false` (fixes a wildcard-dependency false-positive on our own path
  deps) to every crate manifest. `cargo deny check` surfaced real,
  fixable security advisories, not just noise — patched in the same
  session (see the dependency-bump commit): **ammonia 3.3.1 → 3.3.3**
  fixes two XSS advisories (RUSTSEC-2026-0193 mXSS via MathML,
  RUSTSEC-2026-0213 SVG animate/set) in `ferrite-ipi`'s own
  `sanitizer.rs::sanitize_html()` — the HTML-cleaning half of the IPI
  defense; **rand 0.8.5 → 0.8.8** fixes an unsoundness advisory
  (RUSTSEC-2026-0097) in the exact API (`rand::thread_rng()`) `twin.rs`
  calls directly; **rustls-webpki 0.103.10 → 0.103.13** fixes three
  certificate-validation advisories on the real TLS path
  `ferrite-agent`'s `reqwest` client uses for Gemini API calls. One
  advisory (RUSTSEC-2026-0285, a rustls TLS 1.3 handshake edge case) has
  no available fix yet and IS on that same real network path — recorded
  prominently in `deny.toml`'s ignore-list comment, not buried. A ~30-
  crate duplicate-version skip list is bulk-justified (Servo pinned to
  an old git tag vs. the current iced/winit stack — one structural
  cause, not individually vetted crate-by-crate beyond confirming the
  cause), documented as such rather than claimed as individually
  audited.
- `rust-toolchain.toml` (stable + rustfmt/clippy/llvm-tools — the last
  one added after discovering `[profile.release]`'s `strip =
  "debuginfo"` needs `rust-objcopy`, which isn't available without it).
- `.cargo/config.toml`: deliberately does NOT hardcode `build.target-dir`
  (no `~`/env expansion support, wrong call for a multi-OS/multi-
  contributor repo per D13) — documented the deviation from
  §7.4's literal suggestion inline; the `justfile` exports
  `CARGO_TARGET_DIR` via `just`'s portable `home_directory()` instead.
- `justfile`: `default`, `ci`, `check`, `fmt`, `lint`, `audit`, `test`,
  `test-fast`, `test-live`, `run`, `build-servo`, `bloat`, `eval`,
  `clean-cache`, `disk`, `install-hooks`. Every recipe actually run and
  verified except `bloat`/`clean-cache` (need `cargo-bloat`/`cargo-sweep`,
  not installed — noted in-recipe, not silently absent).
- `.github/workflows/ci.yml` reworked: matrix widened to
  ubuntu/macos/windows (was macos/windows); added cargo-machete,
  cargo-deny (`EmbarkStudios/cargo-deny-action`), and both doc-drift
  scripts as gates; clippy now uses `--all-targets`. **Flagged behavior
  change** (T-210): the Servo-enabled "latest" release moved from every
  push to main → weekly (Monday 06:00 UTC) + manual `workflow_dispatch`,
  per §7.1's Servo-cost mandate. Not remotely verified (this sandbox
  can't run GitHub Actions) — YAML syntax and job/trigger structure
  checked locally; every command the workflow runs was actually executed
  on this machine.
- `docs/BUILD_BUDGET.md` created with real numbers: 1.7 GB dev-profile
  target-dir (target well under the 12 GB budget), 3m32s cold
  `just check && just test` (under the 5-minute budget; caveat: registry
  cache was warm, not a true from-network clone — see that file).
- README.md documents the justfile + required local tooling install
  commands (a real onboarding gap: nothing previously said a fresh
  contributor needs `just`/`cargo-deny`/`cargo-machete` installed).

**Commits:** `42401fd`,`068fd8e` (T-207 fmt/clippy closure, split across
two commits because the first deliberately excluded `ferrite-eval` until
its deeper clippy issue was fixed for real rather than dodged) →
`52bab06` (security-relevant dependency bumps) → `8b9d665` (deny.toml/
license/publish/toolchain) → `af8d8e3` (justfile) → `9fee225` (CI
rework) → `1ec1774` (README/toolchain docs). One earlier commit in this
sequence (the `[workspace.dependencies]` consolidation itself) landed
*combined into* `068fd8e` rather than as its own commit — the pre-commit
hook blocked a separate attempt, the fix (the T-207 work above) was
staged in the same working-tree pass, and both got committed together
when the hook finally passed. The commit message for `068fd8e` describes
only the T-207 fix, not the dependency consolidation it also contains —
noted here since R2 requires the record to be accurate even when a
commit message wasn't.

**Tests:** `cargo test --workspace` (95 unit tests + ferrite-servo's 1 +
all doctests) green after every change in this entry; `cargo fmt --all
--check` clean; `cargo clippy --workspace --all-targets -- -D warnings`
clean; `cargo deny check` exits 0; `cargo machete` clean; `just check`,
`just test-fast`, `just disk`, `just eval` (correctly fails informatively),
`just install-hooks` all run for real.

**Needs owner confirmation, filed as T-209/T-210 (not silently decided):**
the `MIT OR Apache-2.0` license choice (inherited from the archived
`.rules` file's stated intent, not freshly confirmed), and the CI
release-cadence change (every push → weekly + manual). Nothing has
shipped to a remote yet — 17 commits sit local on `main`, unpushed
(verified via `git log --oneline origin/main..HEAD | wc -l` at write time).

---

## 2026-09-18 — A2 (Core contracts) — `ferrite-core`: taxonomy, per-capability `OriginScope`, IDs, `Clock`

**Landed:**
- New workspace member `crates/ferrite-core`, wired into `[workspace]`
  `members` and `[workspace.dependencies]` as a path dep so A3 onward can
  take it with `dep.workspace = true`. `license = "MIT OR Apache-2.0"`,
  `publish = false`, matching the other 7 crates (the choice itself is still
  T-209, not re-decided here). No new third-party dependency was added —
  `serde`/`thiserror`/`uuid`/`chrono`/`url`/`serde_json` were all already in
  `[workspace.dependencies]`.
- **Capability/primitive taxonomy** (directive §8, ADR-001): the closed
  7-capability allowlist and 17 observed primitives, of which 16 are
  scopable. The capability→primitive lowering is the `const LOWERING` table;
  `Capability::realization()` is a lookup into it rather than a second copy.
- **`js.execute` is now structurally unrepresentable as an expected
  primitive** (ADR-003, directive §8/A4). Pre-rebuild this was
  `const UNSCOPABLE: &[&str] = &["js.execute"]` checked at runtime in the
  comparator. It is now three agreeing layers: `ActionClass::Execute` is the
  only unscopable class (so the rule is the general one ADR-003 states, not a
  name comparison); `ScopablePrimitive` is a separate 16-variant enum with no
  `JsExecute` variant, and every expected-side type is built from it;
  `Primitive` (the observed vocabulary) keeps the variant because a dry-run
  must be able to record it, with `Primitive::as_scopable()` the only bridge
  back, returning `None` for exactly that variant.
- **`OriginScope` with per-capability scoping** (ADR-004; the type A7 needs
  to close T-001/T-002). Three variants with a `Specificity` ordering
  (`Exact > DomainSuffix > TaskOpen`) exposed as
  `OriginScope::admits(&Origin) -> Option<Specificity>`. `ExpectedCapability`
  carries its own scope and `ExpectedCapabilitySet::lowered()` yields
  `(ScopablePrimitive, &OriginScope, Capability)` triples — the shape §6/A7
  specifies, and the reason the mixed narrow-`scoped.read` /
  wide-`web.read` fixture is now expressible at all.
- Newtype IDs `CaseId`/`ExecId`/`PrincipalId`/`Origin`, validating on
  construction with no unvalidated public constructor; `Clock` trait with
  `SystemClock` and a `cfg(test)`/`test-util`-gated `FixedClock`; error enums
  via `thiserror`.

**Commits:** `1c01a3e` (IDs) `2a19b00` (OriginScope) `01e392e` (taxonomy)
`eb80812` (Clock) `9679904` (serde + golden fixtures) — on branch
`rebuild/a02-core-contracts`, not merged to `main` (R10: that is the
coordinating session's call, after review).

**Tests:** 45 unit + 9 integration + 2 doctests in `ferrite-core`, all green;
`cargo test --workspace` green (no test in this crate touches the network,
the filesystem outside `tests/fixtures/`, or the wall clock).
The load-bearing ones, by name:
- `js_execute_is_the_only_unscopable_primitive`,
  `a_primitive_is_convertible_exactly_when_its_action_class_is_scopable`,
  `no_capabilitys_expected_realization_can_contain_js_execute`,
  `an_expected_capability_set_can_never_lower_to_js_execute`,
  `js_execute_is_admitted_by_nothing_because_it_cannot_be_asked_about`, plus
  a `compile_fail` doctest on `ScopablePrimitive` proving
  `ScopablePrimitive::JsExecute` does not compile, paired with a compiling
  doctest on the same path so the failure cannot be a mistyped path.
- `lowering_table_covers_every_scopable_primitive_exactly_once`,
  `lowering_table_covers_every_capability_exactly_once`,
  `realization_lookup_agrees_with_the_lowering_table`,
  `reverse_index_agrees_with_the_lowering_table`,
  `a_capabilitys_realization_shares_its_action_class`.
- `prop_widening_a_scope_never_revokes_an_admission`,
  `prop_widening_a_scope_never_raises_the_admission_rank`,
  `prop_admission_is_deterministic`,
  `domain_suffix_matches_on_label_boundaries_only`,
  `exact_scope_admits_only_the_listed_origins`.
- `expected_capability_set_lowers_to_per_capability_scopes`,
  `expected_capability_set_rejects_a_duplicate_capability`,
  `expected_capability_set_is_deterministically_ordered`,
  `the_empty_expected_set_lowers_to_nothing`.
- `round_trip_*` and `golden_*` in `tests/schema_stability.rs`, plus
  `json_authored_values_go_through_the_same_validation_as_rust_built_ones`.

**Verified by experiment, not asserted:**
- The directive §8 compile-time gate is real: temporarily adding a
  `ScopablePrimitive` variant without assigning it to a capability produced
  `error[E0004]: non-exhaustive patterns: ScopablePrimitive::Unassigned not
  covered` at `ScopablePrimitive::capability()`. Reverted after checking.
- The tests that were written alongside their data (the ID validators, the
  lowering table, the golden fixtures) were mutation-checked rather than
  trusted: dropping `screenshot` from `web.read`'s realization, misclassing
  `js.execute` as `ActionClass::Read`, removing `Origin` normalization,
  removing the `CaseId` alphabet check, and renaming `scoped.read`'s wire
  string each turned the expected tests red. Reverted after checking.
- `FixedClock` really is absent from a default build: `cargo doc` failed on
  an intra-doc link to it until the link was removed. `cargo doc -D warnings`
  is now clean both with and without `test-util`, and `cargo clippy
  --all-targets` is clean in both feature states.
- `just check`, `just test`, `cargo deny check` (exit 0), `scripts/check_purge.sh`
  and `scripts/check_no_archive_links.sh` all run clean at this branch's HEAD.

**Known issues discovered, filed rather than dropped:**
- **T-211** — `Origin` accepts only `http`/`https`, so an action recorded on
  an opaque origin (`about:blank`, `data:`, `blob:`) has no representation.
  A6's recorder will hit this first.
- **T-212** — `DomainSuffix` does not reject public suffixes: a scope
  authored as `domain_suffix: ["com"]` normalizes cleanly and admits the
  entire TLD. This is the one way the scope algebra can fail *open*. Needs a
  public-suffix list or an authoring-time lint; ADR-004's weak-scope
  stratified reporting does not cover it, because such a scope reports as
  `domain_suffix`, not `task_open`.
- **T-213** — config loading with env overrides was in A2's charter line but
  nothing in this crate needs a config value, so none was built (R9: machinery
  with no caller is the dead code the rule exists to prevent). Deferred to A3,
  which has the first real values (`FERRITE_MODEL_SMALL`/`_MAIN`).
- Not a defect, a reconciliation: directive §8 lists **17** primitives. The
  A2 charter brief said "16", which is the scopable count. Both are now
  explicit in the types (`Primitive::ALL` = 17, `ScopablePrimitive::ALL` = 16)
  and pinned by `primitive_vocabulary_is_the_seventeen_of_directive_section_8`
  and `scopable_vocabulary_is_the_observed_one_minus_js_execute`.
- Residual gap, stated rather than hidden: the `ALL` lists are complete by
  construction (a macro generates the variants and the list from one
  declaration, so a variant cannot exist without appearing in `ALL`), but
  Rust has no stable way to assert an enum's variant *count*, so that
  guarantee rests on the macro being the only way these enums are declared.

## 2026-09-18 — A3 (Model layer) — `ferrite-model`: providers, decorators, conformance suite, T-213

**Session note, for the record (R2):** the first A3 attempt hit an
account-level opus rate limit mid-session after landing 2 clean commits
(`75ba980`, `d9f9438` — trait/guard/config, then cache/throttle/budget
decorators; 89 tests green at that point) and leaving the four backends
half-written, uncommitted. Rather than spend a second full subagent
session re-deriving context already on disk, the coordinating session
picked up directly in the same worktree and finished it. Everything below
was independently re-verified against the actual crate, not assumed from
the interrupted session's last message.

**Landed:**
- Completed the four backends: `MockProvider`/`ReplayProvider` were
  already scaffolded by the first session; `OllamaProvider`,
  `GeminiProvider`, and the shared `backends/http.rs` (status
  classification, `Retry-After` parsing, a streaming bounded body read
  that abandons an oversized response mid-transfer rather than buffering
  it first) are new.
- Found and fixed a real bug while generating the first fixtures:
  `fixtures::FIXTURE_DIR` was a relative string constant
  (`"crates/ferrite-model/tests/fixtures/model"`), which resolves wrong
  under plain `cargo test` — Cargo runs a package's test binaries with the
  *package* directory as the working directory, not the workspace root,
  so the constant silently pointed at a nonexistent nested
  `crates/ferrite-model/crates/ferrite-model/...` path the moment it was
  actually used. Replaced with `fixture_dir()`, built from
  `CARGO_MANIFEST_DIR` (compile-time, always correct regardless of
  runtime CWD), plus a regression test pinning the result is absolute and
  ends where the committed fixtures actually live.
- Generated two real, committed fixtures under
  `crates/ferrite-model/tests/fixtures/model/` via `fixtures::write()`
  against literal, realistic Ollama/Gemini wire JSON — there is no
  `OLLAMA_API_KEY` in this sandbox, so this stood in for an actual
  `just record` session. The generator itself was a throwaway test file,
  deleted immediately after running once.
- `tests/conformance.rs` — the A3 exit-gate deliverable
  ("the provider conformance suite passes identically against
  `MockProvider` and `ReplayProvider`"): one `assert_conforms(&dyn
  ModelProvider)` function run against both, proving a caller holding the
  trait object cannot tell which backend answered except at
  `Provenance::provider` — and a dedicated test pins that this is the one
  *deliberate* place they're allowed to differ, not an accidental gap in
  "identically."
- Added `OllamaProvider::fetch_wire`/`GeminiProvider::fetch_wire` (not in
  either session's original plan): `ModelProvider::complete` digests a
  response into a `CompletionResponse` and discards the raw bytes, but
  `fixtures::write` needs the *verbatim* wire body (`fixtures.rs`'s own
  module doc: "so replay exercises the real parser"). Without a way to
  get the raw body, `just record` could not actually do what §10.3
  describes. Both methods share the same request/classify/read_bounded
  path `complete()` uses, just stopping one step earlier.
- CLI examples (`examples/models.rs`, `probe.rs`, `record.rs`,
  `cache_stats.rs`) and four matching `justfile` recipes. `cache_stats`
  needed `config::default_cache_dir()` made `pub` (was crate-private) so
  it doesn't have to construct a full `ModelConfig` — which would
  otherwise force `FERRITE_MODEL_SMALL`/`MAIN` to be set just to look at
  a directory that has nothing to do with them.
- Secret-scan addition to `scripts/hooks/pre-commit` (§10.1): a targeted
  pattern for `OLLAMA_API_KEY=<something-token-shaped>` /
  `FERRITE_GEMINI_API_KEY=<...>` actually being assigned in a staged
  diff, deliberately not a name-only match (every file in this crate
  legitimately says `OLLAMA_API_KEY` by name; a name-only block would
  fire on every commit and teach everyone to route around it). Verified
  against both a real staged secret (blocked) and the crate's own
  legitimate `OLLAMA_API_KEY`-mentioning source (clean).
- `deny.toml`: `keyring`+`dirs` (new deps, justified inline in the
  commit) introduced a second, separately-caused batch of duplicate-
  version crates (the `windows_*` per-target family, `dirs`/`dirs-sys`)
  on top of A1's Servo-vs-iced batch — added to the same `[bans] skip`
  array (TOML disallows redefining a key in one table) with its own
  comment block so the two causes stay traceable rather than blurring
  into one pile.

**Commits:** `75ba980` (trait/guard/config, T-103+T-213) → `d9f9438`
(decorators, T-103) — both from the interrupted session, re-verified
rather than re-done → `3dcaba5` (backends, fixtures, conformance suite,
CLI examples, justfile, pre-commit hook, deny.toml — everything above).
**Known inaccuracy in `3dcaba5`'s own message, recorded here per R2
rather than silently left**: it says "examples added in the next
commit," written when the intent was to split them out; a `git add -A`
before committing staged everything together and there was no next
commit. The content is correct either way — only the message's claim
about which commit contains what is wrong.

**Tests:** `cargo test -p ferrite-model` — 154 passing (151 unit + 3
conformance), 0 failed. `cargo test --workspace` — every existing suite
still green (`ferrite-core`'s 56, the pre-A2 crates' totals unchanged).
`cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo
fmt --all --check` clean. `cargo deny check` exits 0 (advisories/bans/
licenses/sources all `ok`). `cargo machete` clean. `just check` and
`just test` both green end to end.

**Not done / not verified, stated plainly (R2):**
- `just models`, `just probe`, and `just record` all build, and were
  confirmed to fail cleanly with an actionable error (not a panic) when
  run with no `OLLAMA_API_KEY`/`FERRITE_MODEL_SMALL` configured — but
  none of the three has ever made a real network call. There is no
  Ollama Cloud key in this sandbox. A3's exit gate literally asks a
  *human* to run `just probe` by hand once; that step is still open and
  needs to happen outside an automated session.
- `just cache-stats` **was** run for real, offline, against an empty
  cache dir, and works.
- The two committed fixtures were generated from hand-written literal
  wire JSON, not a real recorded response. They exercise the real
  parsers (that's the point of storing the wire body verbatim), but they
  are not proof Ollama/Gemini's actual current API matches this crate's
  assumptions — only a live `just probe`/`just record` run can confirm
  that.

## 2026-09-18 — coordinator — live verification: A3's exit gate actually met, plus a real infra bug found and fixed

**Landed:**
- The user provided a real `OLLAMA_API_KEY` and (after the first was
  tried) a real `FERRITE_GEMINI_API_KEY` directly in chat. Neither was
  echoed back or written to any tracked file: both were stored in the
  macOS Keychain under service `ferrite` with account names matching
  `ferrite-model::secret`'s `KEYRING_SERVICE`/env-var-name convention
  exactly (`OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY`), so
  `ferrite_model::OsKeyring` — the project's own designed fallback path
  — picks them up with no code change and no env var needing to be set
  in any persisted shell config.
- **`just models` run for real, live, against Ollama Cloud** — returned
  20 real current tags (`gemma4:31b`, `gpt-oss:120b`, `qwen3.5:397b`,
  etc.).
- **`just probe` run for real, live** — one full completion round trip
  (`gemma4:31b`, "Reply with exactly one word: hello" → "hello", 20
  prompt + 2 completion tokens). This is A3's exit-gate line item that
  was explicitly left unverified at merge time; it's now genuinely met,
  not just built-and-assumed.
- **`just record` run for real against both providers**, producing two
  additional genuinely-live fixtures (as distinct from the two
  hand-authored ones from A3's own session):
  - Ollama, `gemma4:31b` — clean round trip.
  - Gemini: the first live call **failed with a real 404** —
    `gemini-2.0-flash` (used in A3's own hand-authored fixture and
    examples as an illustrative tag) is retired; the API's own error
    named the replacement (`gemini-3.6-flash`), which then worked. This
    is a live, unplanned demonstration of exactly the failure mode
    §10.2 designed the whole no-hardcoded-tag-in-source rule around —
    worth citing as evidence the design decision was correct, not just
    defensive.
- Found and fixed a second justfile bug while recording: the `record`
  recipe's `{{prompt}}` was interpolated unquoted into the shell
  command, so a multi-word prompt (`"Reply with exactly one word:
  hello"`) silently truncated to its first word (`"Reply"`) before
  reaching the API — confirmed by inspecting the first recorded
  fixture's `request.messages[0].content`. Fixed by quoting
  `"{{prompt}}"` in the recipe body; re-recorded cleanly afterward.
- **Found, root-caused, and fixed a real, reproducible false-CI-failure
  bug — filed as T-214 (process/infra, not a crate defect):**
  `ferrite-core`'s `golden_*` schema-stability tests started failing
  with `missing schema fixture
  .../.claude/worktrees/agent-a6d3ac111d042019a/.../schema/*.json: No
  such file or directory` — naming a path inside the A3 worktree this
  session had already `git worktree remove`d. Root cause: `env!
  ("CARGO_MANIFEST_DIR")` (the same mechanism this session's earlier
  `fixture_dir()` fix in `ferrite-model` relies on) is baked into a
  crate's compiled incremental codegen units at compile time; because
  `CARGO_TARGET_DIR` is shared across the main checkout and every
  worktree (by the justfile's own design, to avoid rebuilding the world
  per agent), a binary compiled once from inside a worktree can have its
  incremental cache entries reused later for a compile from the main
  checkout, even after the worktree is gone. Reproduced 100% via `just
  ci` (3/3); `cargo clean -p ferrite-core` did **not** fix it (4/4 still
  failing — the stale cache wasn't scoped to that command); a full `rm
  -rf ~/.cache/ferrite-target` did (10/10 clean afterward, across two
  separate rounds of repeated `just ci`/`just test` runs). Full
  investigation trail and the recommended mitigation (clean the shared
  target dir after removing a worktree, before trusting the next test
  run) are in `docs/TO-DO.md` T-214 — not fixed in any crate's source
  because there is no crate-level bug to fix.

**Commits:** `26bd2da` (justfile quoting fix + the two live fixtures).

**Tests:** `just ci` (fmt-check + clippy --all-targets + machete + full
`cargo test --workspace`) — 10 consecutive clean runs after the
target-dir wipe, 0 failures, across two separate rounds of repeated
verification. `cargo deny check` exits 0.

**Known limitation, stated plainly:** the second Gemini key the user
provided was never tried — the first one worked, so trying the second
would have been pointless exposure of an unused credential. It was not
stored anywhere. If the first key is ever revoked, the user has the
second one to provide again.

## 2026-09-18 — A4 (Fingerprint engine) — `ferrite-ipi`: hybrid predict-phase engine, T-104

**Session note:** this worktree's branch was initially checked out from a
stale `origin/main` (the pre-rebuild tree — `Browser/`-nested, no `docs/`,
no `crates/`) rather than this repo's local `main` (which already carries
A0–A3). Recreated `rebuild/a04-fingerprint-engine` from local `main`
(`1bcb7e9`) before doing anything else; flagging it in case the same stale
base bites a later agent's worktree setup.

**Landed:**
- `crates/ferrite-ipi/src/fingerprint/{mod,rules,engine}.rs` (new module,
  wired in via `pub mod fingerprint;` in `lib.rs`; `ferrite-ipi/Cargo.toml`
  gains `ferrite-core`/`ferrite-model`, both already single-sourced in the
  root `Cargo.toml`'s `[workspace.dependencies]`).
- `rules::rule_based_must_use` — the deterministic keyword layer, retyped
  from the pre-rebuild `tool_decision::rule_based_must_use`
  (`crates/ferrite-ipi/src/tool_decision/mod.rs`, left untouched as
  reference material per `docs/AUDIT.md`) against `ferrite_core::Capability`
  instead of a stringly `ToolId`. Same keyword groups; a keyword table
  (`RULES: &[(Capability, &[&str])]`) instead of a chain of `if`s.
- `engine::generate_fingerprint` — calls a `&dyn ferrite_model::ModelProvider`
  (never a concrete backend, per §10.5) to predict `may_use` for whatever
  capabilities the rules did not already pin down. The request carries only
  the prompt and the still-available capability names — never page content
  (§10.3).
- **Closed-allowlist filtering**: `predict_may_use` matches each predicted
  label against `Capability::ALL` by exact `as_str()` equality; anything
  else is dropped at the boundary, not merely rejected after being
  provisionally trusted.
- **Disjointness by construction**: `Fingerprint::new` is the only
  non-empty constructor and is private; it subtracts `must_use` from the
  candidate `may_use` before storing either, so there is no public way to
  build a `Fingerprint` with a capability in both sets.
- **Fail to empty, enforced by control flow**: `predict_may_use` is two
  `let-else` guards — a provider `Err` or an unparseable structured body
  each return `BTreeSet::new()` before the line that would insert into the
  result ever runs. `must_use` is computed from the prompt alone, before
  the model is ever called, so a provider failure cannot touch it.
- **`js.execute` structurally absent**: `Fingerprint`'s two layers are
  `BTreeSet<Capability>`, and `Capability` (`ferrite_core::taxonomy`) has no
  variant belonging to `ActionClass::Execute` — this module inherits that
  guarantee for free by reusing the taxonomy's own type rather than
  inventing a parallel "predicted tool" enum. A `compile_fail` doctest on
  `Capability::JsExecute` (mirroring `ferrite_core::taxonomy`'s own pattern
  on `ScopablePrimitive::JsExecute`) pins this down at the module boundary;
  `engine::tests::a_fingerprint_can_never_express_js_execute` restates it
  under the most permissive input the engine can produce (every capability
  claimed plausible by the model).

**One real gap found while writing the tests, fixed before landing:** the
directive's "no capabilities left to predict" optimization
(`generate_fingerprint` skipping the model call when `must_use` already
covers every capability) was unreachable through the public function alone
— `rule_based_must_use` has no clipboard keywords, so it can never produce
all seven capabilities, and the first version of the corresponding test
silently exercised the normal call path instead of the skip. Factored the
skip logic into a private `generate_from_must_use(provider, model_tag,
prompt, must_use)` that `generate_fingerprint` delegates to, so the test
can drive the skip path directly with a synthetic `must_use` rather than
leaving an untested (and, at the time, actually untriggerable) branch in
place.

**Commits:** `d9e2c12`.

**Tests:** `cargo test -p ferrite-ipi` — 123 unit tests passing (28 new in
`fingerprint::{rules,engine}::tests`, 95 pre-existing and unaffected), plus
2 doctests in `fingerprint::mod` (one `compile_fail`). Table-driven:
`fingerprint::engine::tests::table_driven_prompts_produce_the_expected_fingerprint`.
Adversarial out-of-allowlist filtering:
`out_of_allowlist_labels_are_filtered_not_passed_through`,
`prompt_injection_text_embedded_in_the_response_array_is_dropped`.
Provider-failure injection (all via `ferrite_model::MockProvider`, no live
call): `a_transport_failure_fails_to_empty`, `a_timeout_fails_to_empty`,
`a_rate_limit_fails_to_empty`, `a_server_error_fails_to_empty`,
`a_budget_exhaustion_fails_to_empty`, `truncated_json_fails_to_empty`,
`an_empty_body_fails_to_empty`, `an_oversized_response_is_rejected_and_fails_to_empty`.
Empty-fingerprint routing:
`a_fully_failing_prediction_on_an_open_ended_prompt_is_the_actual_empty_fingerprint`.
Disjointness property test over 8 synthetic prompts against the worst-case
model answer (every capability claimed plausible):
`prop_must_use_and_may_use_are_always_disjoint`. js.execute absence:
the `compile_fail` doctest in `fingerprint/mod.rs`, plus
`a_fingerprint_can_never_express_js_execute` and
`rule_layer_can_never_pin_down_js_execute`.

Full gate: `just check` (fmt-check + `clippy --workspace --all-targets -D
warnings` + `cargo machete`) clean workspace-wide. `just test` (full
workspace `cargo test`) — every crate green, 0 failures, exit code 0
(`ferrite-ipi`: 123 unit + 2 doc; `ferrite-model`: 151 unit + 3
conformance; `ferrite-core`: 58 unit + 2 doc; everything else unchanged
from A3's numbers). `cargo deny check` — `advisories ok, bans ok, licenses
ok, sources ok` (one pre-existing, unrelated `unmatched-source` warning for
the Servo git dependency, since Servo is not built by default).

**Not done / explicitly deferred:**
- No timeout test against `MockProvider::push_hang()` in this module —
  bounding a hung call is `ferrite_model::decorators::Throttle`'s
  per-request timeout, already proven in that crate's own suite (A3). This
  module has no timeout of its own; a caller handing it a provider that
  never resolves gets a call that never resolves, the same contract every
  `ModelProvider` caller has.
- No `js-prompt-injection-in-the-response-body` test beyond the one
  smuggled-string case in `prompt_injection_text_embedded_in_the_response_array_is_dropped`
  — broader adversarial-string fuzzing of the response body was judged out
  of this charter's scope (the filter is an exact-match allowlist check
  against a 7-item closed set, so its correctness does not depend on the
  space of adversarial strings tried against it).
- Scope authoring (`OriginScope` per capability) is not part of this
  module's output. `Fingerprint::must_use`/`may_use` are bare
  `BTreeSet<Capability>`, not `ferrite_core::ExpectedCapabilitySet` —
  per-task origin scoping is authored data (ADR-004), not something a
  keyword layer or a model call can derive from prompt text alone, and
  wiring `Fingerprint` into an `ExpectedCapabilitySet` is naturally A7's
  comparator-integration concern, not this charter's.
- `comparator.rs`, `sanitizer.rs`, `dry_run.rs`, `twin.rs`,
  `containment.rs`, `dataset.rs` and `tool_decision/` were not touched, per
  charter.

**Known issues discovered:** none new beyond the worktree-base note above
(not filed as a T-### — it is a one-time harness/setup issue for this
session, not a repo defect).

## 2026-09-18 — A5 — Sanitizer rebuilt as versioned data, live excision wired at one of two call sites

**Session note:** this worktree's branch was also initially checked out
from the same stale, pre-rebuild tree A4 hit (`Browser/`-nested, no
`docs/`/`crates/`, unrelated commit history — `git log --oneline -20`
showed things like "Task 17", "Git LFS", "CI bug fixing v1.0", nothing
resembling the rebuild). Recreated `rebuild/a05-sanitizer` from this
repo's local `main` (`97c7164`, which carries A0–A4) before doing anything
else, exactly as `docs/handoffs/a04.md` warned a future agent might need
to.

**Landed:**
- `crates/ferrite-ipi/src/sanitizer/{mod,patterns,detect,html,excise,config}.rs`
  (new module directory, replacing the old flat `sanitizer.rs` — Rust
  cannot have both `sanitizer.rs` and `sanitizer/mod.rs` resolve to the
  same `crate::sanitizer` path, and the charter's own framing ("rebuilding
  it as a new module") reads as a replacement, not an addition; the old
  file's logic and tests were folded into the new layout rather than
  archived-in-place per `docs/AUDIT.md`'s note that it is reference
  material to be reshaped).
- `patterns.rs` — `PatternDef`/`PatternSet` as versioned data (`id`,
  `description`, `regex`, `since_version`, a crate-wide
  `PATTERN_SET_VERSION`), replacing the old inline
  `general_injection_patterns()` tuple array. Two independent statics:
  `GENERAL_PATTERNS` (5 patterns, unchanged count from pre-rebuild, per
  the parameter rationale in `docs/REBUILD_DIRECTIVE.md` §13.4) and
  `SCRIPT_PATTERNS` (5 JS-specific exfiltration primitives).
- `detect.rs` — `Finding`/`LocatedFinding`, `detect()` against a
  `PatternSet`, `detect_injection_in_value()`'s recursive JSON walk with
  path tagging (`results[0].description`, not "somewhere in the blob").
  Golden corpus: two positive + two negative examples per pattern (10
  patterns total), table-driven, plus a test that every pattern has
  corpus coverage.
- `html.rs` — `ammonia`-based cleaning plus the comment/script carriers
  extracted from raw HTML before `clean()` runs, ported near-verbatim
  from the old `sanitize_html`.
- `excise.rs` — sentence-segment, tag-boundary-safe excision
  (`segment_bounds`/`merge_ranges`/`excise_ranges`, ported from the old
  code), single-space replacement (never a marker — argued in the module
  docs). New: an HTML-validity property test
  (`excision_never_produces_malformed_html`) over 12 hand-built HTML
  fixtures (injections mid-paragraph, tag-spanning, nested, at
  document start/end, inside table cells, next to self-closing tags,
  inside attribute-bearing elements), using `ammonia::clean()`'s own
  parse/serialize as the well-formedness oracle: the test asserts
  `clean(excise(clean(html))) == excise(clean(html))`, i.e. the excised
  output is already a fixpoint of ammonia's parser — reasoning for why
  that's a sufficient check is written into the test module's doc
  comment rather than asserted without justification.
- `config.rs` — `SanitizerConfig { detect_enabled, strip_enabled }` and
  `run()`, the single call that makes excision live: `run(&config, html)`
  returns a `SanitizedPage` whose `clean_html` is already excised when
  `strip_enabled` is set, instead of requiring a caller to remember a
  second `excise_*` call (the previous design's actual failure mode).
  `PROVISIONAL_FALSE_STRIP_RATE_CEILING = 0.02`, cited to `docs/TO-DO.md`
  T-203 as provisional, gates
  `benign_fixture_false_strip_rate_is_below_the_provisional_ceiling`,
  measured over a 35-sentence benign fixture corpus (ordinary web/tool
  prose, several deliberately close to trigger vocabulary) — the test
  passes at the current pattern set (0 of 35 stripped, well under the
  2% ceiling; the ceiling itself, not this run's exact rate, is what's
  provisional and needs A11's real audit against a real ~100-case benign
  corpus per §13.3).
- Independent-toggle tests: `detect_only_reports_findings_but_leaves_content_unchanged`,
  `strip_mode_reports_findings_and_returns_modified_text`,
  `off_mode_detects_nothing_and_returns_input_verbatim`.
- `crates/ferrite-ipi/src/tool_decision/mod.rs` — minimal, explicitly
  charter-sanctioned touch (the exception written into A5's charter for
  wiring `strip_enabled` live): added `DefenseMode::sanitizer_detect_enabled`/
  `sanitizer_strip_enabled`, and changed `ToolDecisionEngine::prepare_task`'s
  `On`/`SanitizerOnly` arms from calling the old detect-only
  `sanitizer::sanitize_html` to calling the new config-gated
  `sanitizer::run`. This closes D3/T-003 **at this one call site**: before
  the change, `On` and `SanitizerOnly` both only ever detected, never
  excised, at this call site — a second, previously-unnamed instance of
  D3's exact symptom, alongside the one the defect register names.

**Verified, not assumed:** `cargo test -p ferrite-ipi` — 142 unit tests
passing (up from the 123 A4's own PROGRESS.md entry above verified at its
landing — no other agent touched `ferrite-ipi` between A4 and this
session; net +19 after removing the old flat `sanitizer.rs`'s 26 tests and
adding more, restructured, in the new layout), 2 doctests unchanged (A4's
`fingerprint::mod` doctests). `cargo fmt -p ferrite-ipi --check` clean.
`cargo clippy -p ferrite-ipi --all-targets -- -D warnings` clean. `just
check` (fmt-check + `clippy --workspace --all-targets -D warnings` +
`cargo machete`) clean workspace-wide — this also proves `ferrite-eval`
and `ferrite-ui`, both of which depend on `ferrite_ipi::sanitizer::Finding`
and `ferrite_ipi::dry_run` (which itself calls into the new sanitizer
module), still compile and lint clean against the rebuilt API. `just
test` — every crate green, 0 failures, exit 0 (`ferrite-ipi`: 142 unit + 2
doc; every other crate's count unchanged from A4's numbers:
`ferrite-model` 151 + 3 conformance, `ferrite-core` 45 + 9 schema + 2 doc,
`ferrite-eval` 58 + 3 + 5, `ferrite-agent` 4, `ferrite-servo` 1). `cargo
deny check` — `advisories ok, bans ok, licenses ok, sources ok` (same
pre-existing, unrelated Servo-git-source `unmatched-source` warning A3/A4
also saw).

**A compatibility constraint discovered while porting, fixed before
landing:** `crate::dry_run::RecordingExecutor` (out of this charter's
scope) hardcodes the exact string `"instruction_override"` in its own
test `w1_t1a_comment_caught`. The initial pattern split gave the
"ignore ... previous/prior/above" rule and the "disregard ... previous"
rule two distinct new IDs (`instruction_override_ignore`/
`instruction_override_disregard`), which is the more correct design (the
two were previously silently sharing one label) but broke that test.
Since `dry_run.rs` cannot be touched per charter, the fix went the other
way: the "ignore" pattern kept the bare `instruction_override` id, and
only the "disregard" pattern got the new, previously-nonexistent
`instruction_override_disregard` id. Documented at the point of
definition in `patterns.rs`.

**Not done / explicitly deferred — read before assuming T-003 is fully
closed:** `docs/TO-DO.md` T-003 names a literal field, `strip_enabled`,
that lives in `crate::dry_run::RecordingExecutor`/`DryRunOrchestrator` —
a file this charter explicitly forbids touching. That flag still defaults
to `false`, and neither of its two current callers
(`ferrite-eval/src/harness.rs`'s `run_one`, which calls
`set_detect_enabled` but never `set_strip_enabled`; `ferrite-ui/src/lib.rs`,
which calls neither) ever flips it to `true` — both files are outside
`ferrite-ipi` and outside this charter's file list. **This means the
dry-run/eval-harness path — the one that actually produces
`ExecutionRecord`s and metrics — still runs with excision off**, even
though the sanitizer itself and its one in-crate production call site
(`tool_decision::prepare_task`) now support and use live excision
correctly. T-003 is marked `in-progress`, not `done`, in `docs/TO-DO.md`;
a new task (T-215) is filed for the remaining half, owned by A6
(`dry_run.rs`) and A12 (`harness.rs`, which already owns closing T-004,
itself dependent on T-003). Full detail in `crates/ferrite-ipi/src/sanitizer/mod.rs`'s
"How far this actually reaches" doc section and `docs/handoffs/a05.md`.

**Commits:** `58c9121`.

**Known issues discovered:** T-215 (new, see above) — `dry_run.rs`'s
`RecordingExecutor.strip_enabled` and its two production-shaped callers
never set it `true`; this is the literal field D3/T-003 names, and it
remains unwired outside this charter's reach.

## 2026-09-18 — A6 (Dry-run + twin + containment) — versioned modules, DEV_KEY removed, containment deleted

**Session note:** same hazard A4/A5 hit — worktree was initially checked
out on a stale, unrelated tree (`65b6d67`, "Task 17"/Git-LFS-era commits,
nothing to do with this rebuild). Recreated
`rebuild/a06-dry-run-twin-containment` from local `main` (`b229fb1`,
carries A0–A5) before doing any work.

**Landed:**
- `crates/ferrite-ipi/src/dry_run/{mod,record,content,executor,orchestrator}.rs`
  replaces the flat `dry_run.rs`, mirroring `fingerprint/`/`sanitizer/`'s
  layout. `RecordingExecutor`/`DryRunOrchestrator` are a faithful port of
  the old logic (ordered event log, per-origin `ReplyChannel` queues,
  inline detect/strip gating, whole-turn timeout returning a partial
  record) with three real additions: `DryRunRecord::events_with_seq()`
  (an explicit 0-based sequence number per event, derived from `Vec`
  position rather than a stored field — see below for why), a tested
  `ToolEvent::primitive()` bridge to `ferrite_core::Primitive`, and
  `DryRunOrchestrator::set_defense_mode(DefenseMode)`, which derives
  `detect_enabled`/`strip_enabled` together from `DefenseMode` the same
  way `sanitizer::run`'s config does (T-215, see below).
- `crates/ferrite-ipi/src/twin/{mod,key,crypto,manager}.rs` replaces the
  flat `twin.rs`. The compiled-in `DEV_KEY` constant is gone (D8).
  `key::resolve_twin_key` checks the OS keyring first, then
  `FERRITE_TWIN_KEY`, reusing `ferrite-model`'s `SecretStore`/`OsKeyring`/
  `Token`/`EnvSource` rather than reimplementing a keyring wrapper — see
  that module's doc comment for exactly what is and isn't reused, and why
  (`ferrite_model::secret::resolve()` itself returns `ModelError`, which
  would be a type lie for a twin key failure). `crypto::derive_key` turns
  resolved secret material into a 32-byte AES key via SHA-256 (no KDF —
  documented as sufficient because the payload is synthetic, not real).
  `TwinManager::load_or_generate` is now fallible
  (`Result<SyntheticTwin, TwinKeyError>`), directly tested for both the
  keyring-then-env order and the no-key-anywhere `Err` path (not a
  `#[should_panic]` standing in for it).
- `crates/ferrite-ipi/src/containment.rs` deleted outright (option (a),
  the directive's stated default for T-007/D7). `intercept_request()` was
  unreachable outside its own unit test — `RecordingExecutor` never made a
  real network call for it to intercept, so containment was already
  achieved by construction, not by an active interceptor. Confirmed
  `RecordingExecutor`'s own source has no network-capable dependency
  (`this_modules_source_names_no_network_capable_dependency`, grep-backed
  against split literals so the test can't trivially match its own
  prose). Dropped the now-dead `nix` dependency
  (`[target.'cfg(target_os = "linux")'.dependencies]`) from
  `ferrite-ipi/Cargo.toml`, verified unused workspace-wide with `cargo
  machete`, and removed the matching `cargo-deny` skip entry for `nix`
  (it was the only workspace consumer).

**Deviation from the directive's illustrative `ToolEvent { primitive,
origin, seq }` shape**, and why: `crate::comparator` (A7's charter,
explicitly off-limits this session) already pattern-matches
`event.tool: ToolId` in `compare()` and constructs events via
`record_tool(ToolId::new(..), ..)` in its own tests; `crate::dataset`
(A11's charter, also off-limits) constructs a bare `ToolEvent { tool,
origin }` struct literal directly in a `#[cfg(test)]` fixture. Renaming
`tool` to `primitive: ferrite_core::Primitive` or adding a mandatory
`seq: u64` field would not compile without editing both files. Kept
`ToolEvent { tool: ToolId, origin: Option<String> }` unchanged; added
`ToolEvent::primitive()` (tested bridge to `ferrite_core::Primitive`,
`None` for exactly one known mismatch — `BrowserTool::DownloadFile`'s
`ToolId` wire string is `"download.file"`, `Primitive::Download`'s is
`"download"`) and `DryRunRecord::events_with_seq()` (derived seq via
`Vec` position) so A7 gets the forward-compatible shape without a
compile break today.

**Deviation from a literal "fail loudly, `Result::Err`" reading for the
twin key**, found empirically, not assumed: making
`DryRunOrchestrator::run()` hard-propagate a missing-key error broke five
`ferrite-eval` tests (`harness::e2e_tests::*`, `corpus::loaded_case_runs_through_the_full_pipeline`)
that call `run_one`/`run_case` with no `FERRITE_TWIN_KEY` configured
anywhere in the test environment — exactly the R7 "no operator secret
required for the test suite" expectation the codebase holds for model
provider keys, which nothing established for the twin key before this
session. `TwinManager::load_or_generate` still returns the strict,
directly-tested `Result::Err` (T-008's literal requirement, satisfied at
that layer). `DryRunOrchestrator::run` — the layer `ferrite-eval`/
`ferrite-ui` actually call, neither editable this session — instead logs
a warning and generates an unpersisted twin for that call, mirroring
`tool_decision`'s existing "no Gemini key configured" fallback pattern
elsewhere in this crate. Reasoning recorded in
`DryRunOrchestrator::run`'s doc comment and `docs/handoffs/a06.md`: the
twin key gates a caching convenience for synthetic data, not any part of
the containment/sanitizer/consent mechanism, so losing the cache is the
correct failure mode, not losing the whole dry run.

**T-215, this session's half:** `DryRunOrchestrator::set_defense_mode`
derives `detect_enabled`/`strip_enabled` together from `DefenseMode` in
one call, reusing `tool_decision::DefenseMode::sanitizer_detect_enabled`/
`sanitizer_strip_enabled` (A5). A12's remaining one-line fix in
`ferrite-eval/src/harness.rs::run_one` (line ~220): replace
`orch.set_detect_enabled(behavior.detect_enabled);` with
`orch.set_defense_mode(mode);` — `mode: DefenseMode` is already a
parameter of `run_one`, and nothing later in that function reads
`behavior.detect_enabled` again, only `behavior.loop_runs`. `docs/TO-DO.md`
T-215 updated accordingly; still not fully closed until A12 makes that
change (this session cannot touch `harness.rs`).

**Verified:** `cargo test -p ferrite-ipi` — 140 unit + 2 doctests, 0
failures. `cargo fmt -p ferrite-ipi --check` and `cargo clippy -p
ferrite-ipi --all-targets -- -D warnings` clean. `cargo machete` clean for
`ferrite-ipi`. `cargo build --workspace` and `cargo test --workspace`
green (0 failed) after the `Box<dyn EnvSource + Send + Sync>` fix below.
`just check` (fmt-check + clippy --all-targets -D warnings + cargo
machete) and `just test` (full workspace suite) both green from the repo
root. `cargo deny check`: `advisories ok, bans ok, licenses ok, sources
ok`, only the pre-existing Servo-git-source warning remains (same one
A3/A4/A5 logged).

**Bug found and fixed mid-session, before the workspace-wide run above
ever went green:** `TwinManager`'s injectable `Box<dyn EnvSource>` field
is not `Send`/`Sync` by default (`EnvSource`, in `ferrite-model`, carries
no such supertrait, unlike `SecretStore`, which already does). `cargo
build --workspace` failed on `ferrite-ui`, which holds a
`DryRunOrchestrator` across an `.await` inside a `tokio::spawn`'d agent
loop task. Fixed by bounding the trait object explicitly at every
declaration site in this crate (`Box<dyn EnvSource + Send + Sync>`) —
every concrete source (`SystemEnv`, `MapEnv`) already satisfies it, so
this costs nothing real. Not filed as a new T-### since it was fully
fixed within this session and is now covered by the green workspace-wide
build.

**Commits:** `be891b8`, `f8edef3`.

**Tests:** `dry_run::orchestrator::tests::dry_run_records_navigation_and_updates_origin`,
`dry_run::orchestrator::tests::origin_updates_across_a_scripted_navigation`,
`dry_run::orchestrator::tests::same_origin_sequential_reads_get_different_scripted_content`,
`dry_run::orchestrator::tests::detect_and_strip_gating_actually_gates_content`,
`dry_run::orchestrator::tests::timeout_returns_a_partial_record_not_an_empty_one`,
`dry_run::orchestrator::tests::set_defense_mode_derives_both_flags_from_on`,
`dry_run::orchestrator::tests::missing_twin_key_degrades_the_dry_run_rather_than_failing_it`,
`dry_run::record::tests::seq_is_monotonic_and_matches_call_order`,
`dry_run::executor::containment_tests::this_modules_source_names_no_network_capable_dependency`,
`twin::key::tests::the_keyring_is_consulted_first`,
`twin::key::tests::neither_source_is_a_typed_error_naming_both_and_never_a_panic`,
`twin::manager::tests::manager_without_a_resolvable_key_fails_loudly_not_silently`,
`twin::crypto::tests::encrypt_decrypt_roundtrip_with_a_resolved_key`.

**Known issues discovered:**
- T-215 updated, not closed — see above; A12 owns the remaining one-line
  `harness.rs` change.
- New: `ToolEvent::primitive()` returns `None` for
  `BrowserTool::DownloadFile` because `BrowserTool::tool_id()`
  (`ferrite-agent`) emits `"download.file"` while
  `ferrite_core::Primitive::Download`'s wire string is `"download"` — a
  pre-existing mismatch between the two crates' vocabularies, predating
  this session, out of scope for both (neither `ferrite-agent` nor
  `ferrite-core` is in this charter's file list). Filed as T-216 in
  `docs/TO-DO.md`, owned by A7 (needs it to consume `ToolEvent::primitive()`
  cleanly) or whoever next touches `BrowserTool::tool_id()`.
- New: T-211 (opaque origins) is exercised but not closed by this
  session. `dry_run::record::extract_origin` records an opaque-scheme URL
  (`about:blank`, `data:`, ...) as its literal string rather than a
  normalized origin, since `ferrite_core::Origin::parse` structurally
  rejects every scheme but `http`/`https`. Documented in
  `dry_run::record`'s module docs as the deliberate interim behavior: a
  literal opaque string will also fail `Origin::parse` downstream, which
  is the *correct* effect (unadmittable by any scope, like `js.execute`)
  achieved by type mismatch rather than a principled `Origin::Opaque`
  variant. `ferrite-core` is out of this session's scope; T-211's row in
  `docs/TO-DO.md` is updated with this note but stays `open`, owned by
  A7/whoever next touches `ferrite-core::Origin`.

## 2026-09-18 — A7 (Comparator + origin model) — per-capability `compare()`, admission-rank attribution, T-001/T-002/T-206/T-211/T-216

**Session note:** same hazard A4/A5/A6 hit — worktree started on a stale,
unrelated tree (`65b6d67`, "Initial test case designing..."/Git-LFS
history unrelated to Ferrite). Recreated
`rebuild/a07-comparator-origin-model` from local `main` (`69f4268`,
carries A0–A6) before doing any work.

**Landed:** `crates/ferrite-ipi/src/comparator/{mod,expected,diff,consent,
legacy}.rs` replace the flat `comparator.rs`. `compare()`'s new contract:
`fn compare(expected: &ExpectedFingerprint, actual: &DryRunRecord) ->
FingerprintDiff`. `ExpectedFingerprint` wraps
`ferrite_core::ExpectedCapabilitySet` (A2) — capabilities, each carrying
its own `OriginScope` — and lowers through
`ExpectedCapabilitySet::lowered()` directly rather than re-deriving the
capability→primitive table. The old file's local `OriginScope`/
`ScopeType` duplicate types are gone entirely; every scope-typed value in
this module is `ferrite_core::scope::OriginScope`/`Specificity`.

**T-001/D1 (closed, `6cb4c70`):** a fingerprint mixing a narrow
`scoped.read` on `mail.example.com` with a wide `web.read` on
`task_open`/a domain suffix is now expressible and attributes correctly
per event — proven directly in
`comparator::tests::narrow_scoped_read_and_wide_web_read_in_the_same_fingerprint_attribute_independently`
and the exhaustive 2×3×4 enumeration
(`comparator::tests::exhaustive_two_capability_three_origin_four_primitive_enumeration`).

**T-002/D2 (closed, `6cb4c70`):** `OriginScope::admits`'s `Specificity` is
consumed, not just computed — `compare` scans every admitting candidate
for a matching primitive and keeps the one with the *maximum* rank
(`exact > domain_suffix > task_open`), recording it on the `Attribution`.
`comparator::tests::admission_rank_is_actually_consumed_not_just_computed`
and `comparator::tests::admission_rank_picks_the_tighter_of_two_admitting_capabilities`
are written to fail if `compare` reverted to "first match wins" instead
of "highest-`Specificity`-wins".

**T-206 (closed, `6cb4c70`):** the double-flag bug (an event landing in
both `extra_primitives` and `out_of_scope_origins`) is fixed by
construction: "primitive named by some capability" and "primitive named
by no capability" partition the whole primitive space independently of
origin, so `compare`'s classification is an `if`/`else`, not two
independent checks.
`comparator::tests::t206_regression_primitive_not_expected_and_origin_not_admitted_lands_in_exactly_one_bucket`
is the direct regression test. No `Both` variant was added — see
`comparator`'s module docs ("Why there is no `Both` variant") for why
that would either duplicate `ExtraPrimitive` or resurrect the bug under a
new name.

**T-211 (ratified, `6cb4c70` — not "resolved by adding a variant"):**
after reading `ferrite_core::ids::Origin`'s doc comment together with
ADR-004 and A6's handoff, this session concludes the existing behavior
(`Origin::parse` accepts only `http`/`https`, so an opaque-scheme URL
fails to parse, which is the correct "unadmittable by any scope" signal)
*is* the intended design, not a gap — adding an `Origin::Opaque` variant
would need to be unconditionally unadmittable to mean anything, which
"fails to parse" already achieves with less surface area. `compare`
treats a recorded-but-unparseable origin string as `out_of_scope_origins`
(there is a string to report) and a wholly-missing origin (`None`) as
`extra_primitives` (nothing to report). Tests:
`comparator::tests::an_opaque_scheme_origin_is_out_of_scope_under_its_literal_string`,
`comparator::tests::an_event_with_no_recorded_origin_is_an_extra_primitive`.
`docs/TO-DO.md`'s T-211 row is updated to `done` (ratified), not left
open as an unresolved question.

**T-216 (closed, `6cb4c70`):** `download.file` (`ferrite-agent`'s
`BrowserTool::DownloadFile::tool_id()`) vs. `Primitive::Download`'s wire
string `"download"` is resolved with a one-entry mapping local to
`comparator::resolve_primitive`, not by changing `BrowserTool::tool_id()`
— that string is key material for `dry_run::content::DryRunContent::
by_tool_id`'s per-tool reply queues (A6's file, off-limits) and is
embedded literally in `ferrite-eval`'s corpus fixtures/tests, so changing
it would ripple far outside this charter for no benefit over a targeted
mapping. Test:
`comparator::tests::download_file_attributes_to_web_download_at_an_admitted_origin`.

**The design gap (A4's `Fingerprint`/legacy `ToolFingerprint` carry no
per-capability scope) — resolved, not papered over:**
`ExpectedFingerprint::from_fingerprint`/`from_legacy_tool_fingerprint`
apply one scope to every capability in a fingerprint that has none of its
own, sourced from the task's context URL: `exact([context_url])` when
present, `task_open` with a stated provisional rationale when absent —
**never an unconditional `task_open`**, which would silently readmit
everything and make this whole rewrite pointless.
`comparator::expected::tests::from_fingerprint_with_a_context_url_still_flags_a_different_origin`
is the regression test that would fail if this default were ever widened
to ignore `context_url`. `ExpectedFingerprint::from_capabilities` is the
direct, no-defaulting constructor for a caller with a genuine
per-capability scope of its own — reasoning at length in
`comparator`'s module docs ("Where a per-capability `OriginScope` comes
from"). As a direct consequence, the live `ferrite-ui` agent-task path's
`compare()` call site — previously a hardcoded, unconditional
`OriginScope::task_open()` behind a `TODO(Task 19)` comment — now derives
a real exact-scope from `agent_task.context_url` when one exists
(`ferrite-ui/src/lib.rs`, commit `40221de`).

**Cross-crate compile fixes (minimal, `40221de`), all required by the new
`compare()` signature and the deletion of the old local `OriginScope`:**
- `ferrite-eval/src/harness.rs::run_one` — context_url now extracted from
  `OriginScope::Exact`'s variant instead of a removed `.exact` field;
  builds an `ExpectedFingerprint::from_legacy_tool_fingerprint` before
  calling `compare`.
- `ferrite-eval/src/adjudication.rs`, `harness.rs` test fixtures —
  migrated off the deleted `comparator::OriginScope` to
  `ferrite_core::scope::OriginScope`'s fallible `exact`/`task_open` API
  (mechanical, test-only call sites).
- `ferrite-eval/src/corpus.rs`, `tests/pilot_w6.rs`,
  `tests/pilot_corpus/*.json` (10 files) — `expected_origins` JSON
  literals updated to `ferrite_core::OriginScope`'s externally-tagged
  wire shape (`{"exact": [...]}` / `{"domain_suffix": [...]}` /
  `{"task_open": {"rationale": ...}}`) in place of the old four-key flat
  struct; these are authored test fixtures, not corpus/ or dataset.rs
  schema, so within this session's scope to fix mechanically.
- `ferrite-eval` and `ferrite-ui` both gained a direct `ferrite-core`
  dependency (confirmed present in the workspace root `Cargo.toml`
  first) — a downward edge only, `cargo machete` confirms no unused
  dependency resulted.
- `dataset.rs`'s `CaseDefinition::expected_origins`/
  `ExpectedRealization::origin_scope` fields are now typed
  `ferrite_core::scope::OriginScope` (three call sites plus an import;
  the struct shapes themselves are untouched — no feature work in A11's
  schema beyond what compiling required).

**`FingerprintDiff`'s shape:** `extra_primitives: HashSet<ToolId>` and
`out_of_scope_origins: HashSet<String>` keep their pre-rebuild names/types
verbatim (read directly by `dataset::unscopable_primitive_invoked` and
extensively by `ferrite-eval`/`ferrite-ui`, none of which this charter's
"minimal mechanical" mandate permitted renaming). New:
`attributions: Vec<Attribution>` (`#[serde(default)]`, so pre-A7 JSON
still deserializes), each entry naming the tool, origin, admitting
`Capability`, and winning `Specificity` — the shape A8's audit log needs
to cite "which capability justified this action" per entry.

**Tests:** 37 new tests in `crates/ferrite-ipi/src/comparator/` (mod.rs:
20, expected.rs: 6, diff.rs: 6, consent.rs: 3, legacy.rs: 2), including
property tests for monotonicity
(`comparator::tests::prop_widening_a_capabilitys_scope_never_turns_an_admitted_event_into_a_flagged_one`)
and determinism (`comparator::tests::prop_compare_is_deterministic`), the
js.execute-no-exceptions test
(`comparator::tests::js_execute_is_flagged_even_under_a_scope_that_would_admit_any_origin`),
and the exhaustive small-case enumeration named above.

**Verified:** `cargo test -p ferrite-ipi` — 166 unit tests + 2 doctests
(1 ignored — the module-doc `compare()` signature snippet is
`` ```ignore ``` ``), 0 failures. `cargo fmt -p ferrite-ipi --check` and
`cargo clippy -p ferrite-ipi --all-targets -- -D warnings` clean. `cargo
build --workspace` and `cargo test --workspace` green (0 failed, verified
via `grep -E "FAILED|error\["` over the full run — no matches). `cargo
fmt --all --check` and `cargo clippy --workspace --all-targets -- -D
warnings` clean. `just check` (fmt-check + clippy + `cargo machete`)
green — machete confirms no unused dependency from the two new
`ferrite-core` deps. `just test` (full workspace suite) green. `cargo
deny check`: `advisories ok, bans ok, licenses ok, sources ok`, only the
pre-existing Servo-git-source warning (same one A3–A6 logged).

**Commits:** `6cb4c70` (comparator rebuild), `40221de` (cross-crate
compile fixes).

**Known issues discovered / left for later agents:**
- `ExpectedFingerprint::from_fingerprint`/`from_legacy_tool_fingerprint`'s
  scope-sourcing policy is explicitly provisional: it applies one scope
  (derived from a single context URL) uniformly to every capability in a
  fingerprint, because neither `fingerprint::Fingerprint` nor the legacy
  `tool_decision::ToolFingerprint` distinguishes which capability wants
  which origin. A real per-task authoring mechanism (e.g. wiring
  `dataset::CaseDefinition`'s `scope_rationale` into a genuine
  per-capability scope, or a UI for authoring one) would let
  `ExpectedFingerprint::from_capabilities` express real
  precision — filed as new `T-217` in `docs/TO-DO.md`.
- `ferrite-ui`'s `run_agent_loop`/consent path (`ConsentDecision`,
  `FingerprintDiff::summary()`) is unchanged by this session beyond the
  one call site fixed to compile — A10's UI charter is the next place
  that surface gets real attention (e.g. showing `Attribution`'s
  capability/specificity in the consent panel, not just the flagged
  sets).
- A8 (audit log) is the next consumer of this module's output: see
  `docs/handoffs/a07.md` for exactly what shape `FingerprintDiff`/
  `Attribution` now hand it.

## 2026-09-18 — A8 (Audit log) — full-payload hash preimage, tamper matrix, T-005/T-108

**Session note:** same hazard A4–A7 hit — worktree started on a stale,
unrelated tree (`65b6d67`, "Initial test case designing..."/unrelated
history, different project entirely). Recreated `rebuild/a08-audit-log`
from local `main` (`dd5fd71`, carries A0–A7) before doing any work.

**Landed:** `crates/ferrite-audit-log/src/lib.rs` — `compute_entry_hash`
is now the single function `AuditLog::append` and `AuditLog::verify_chain`
both call, so the two can never drift apart the way the old duplicated
`format!("{}{}{:?}{}{}", ...)` strings implicitly could. The preimage now
covers every persisted field, not just five of nine: `schema_version`
(new — `AUDIT_SCHEMA_VERSION = 1`, itself hashed), `entry_id`, `sequence`,
`timestamp` (as its RFC3339 string, not `chrono`'s internal repr),
`kind`, `principal_id`, `capability`, `url`, `prev_hash`. Serialization is
`serde_json::to_vec` over a purpose-built `HashPreimage` struct with fixed
field order (not `Debug` formatting, not a `HashMap`) — this closes the
two real ambiguity risks named in the charter: `{:?}` on an enum is not a
stable wire format across derive versions, and JSON distinguishes `null`
from `""` and length-delimits every string, so `None` vs.
`Some(String::new())` for `capability`/`url` can never collide the way
naive concatenation could. The preimage is domain-separated with a fixed
`b"ferrite-audit-v1"` prefix (`HASH_DOMAIN`).

`PersistentAuditLog` gained a second, small persisted checkpoint —
`audit_chain_head`, a single-row table holding the next sequence number
and last entry hash, written in the same SQL transaction as every
`append` — because the hash chain alone cannot prove *completeness*
(nothing in entry N's hash can prove entry N+1 once existed and was
deleted). `load` cross-checks this watermark against the entries actually
present and returns the new `AuditError::TruncatedChain` variant on
mismatch. This is a best-effort defense, not a cryptographic guarantee —
documented plainly in the module docs and in a test
(`truncation_undetected_if_attacker_also_forges_the_checkpoint_head`)
that shows exactly where it stops working; filed as **T-218** for whoever
owns the deployment threat model to decide whether an external anchor is
worth building.

**Commits:** `6250cb3` (fix(audit-log): cover full canonical payload in
the hash preimage).

**Tests:** 21 tests in `crates/ferrite-audit-log/src/lib.rs` (0 before
this session — the crate had no test module at all):
- Tamper matrix, one test per persisted field, each asserting
  `verify_chain()` returns `false` (T-005's direct regression, covering
  every field, not just the two the bug report named):
  `tamper_matrix_capability`, `tamper_matrix_url`,
  `tamper_matrix_principal_id`, `tamper_matrix_kind`,
  `tamper_matrix_timestamp`, `tamper_matrix_sequence`,
  `tamper_matrix_entry_id`, `tamper_matrix_prev_hash`,
  `tamper_matrix_schema_version`, `tamper_matrix_entry_hash_rewritten_directly`
  (10 tests; two more, `tampering_capability_after_the_fact_breaks_verification`
  and `tampering_url_after_the_fact_breaks_verification`, are the literal
  pre-fix bug scenario, kept as an explicit named regression).
- Truncation: `truncation_attack_dropping_the_tail_is_detected_on_load`
  (positive case) and `truncation_undetected_if_attacker_also_forges_the_checkpoint_head`
  (the documented limit, asserted as `Ok`, not a false claim of full
  coverage).
- Reordering: `reordering_attack_swapping_sequence_values_is_detected_on_load`
  (swaps two entries' `sequence` column values directly in SQLite,
  leaving stored hashes untouched — caught because `sequence` is now
  hashed).
- Insertion: `insertion_attack_splicing_a_self_consistent_forged_entry_is_detected_on_load`
  (a forged row whose own hash is internally valid, computed with the
  real `compute_entry_hash`, chained from a real predecessor but never
  woven into the successor's `prev_hash` — the chain-link check catches
  it regardless of where SQLite's tie-broken ordering places it).
- Golden fixture: `golden_chain_fixture_pins_entry_hashes` — two
  fixed-input entries (fixed UUIDs, fixed RFC3339 timestamp, fixed
  capability/url) with pinned expected hex:
  `c1b4ff371b8ed37e43a3f6fb61a4a88f95c457cfd3219e0214d743929b0de11a`
  (entry A, `EvalExecutionRecorded`) and
  `66e21de3ecb7547adc3bcf6a901031fe694b3a3cd73c37e26b4d263e4c282671`
  (entry B, `CapabilityExercised`, chained from A). An accidental
  preimage change now fails loudly instead of silently changing what old
  audit logs mean.
- Persistence sanity: `persisted_log_round_trips_and_verifies`,
  `persisted_tamper_of_capability_column_is_detected_on_load` (the exact
  T-005 SQLite-column-rewrite scenario, now caught on `load`),
  `append_produces_a_verifying_chain`, `empty_log_verifies`.

**Verified:** `cargo test -p ferrite-audit-log` — 21 passed, 0 failed, 0
doctests. `cargo fmt -p ferrite-audit-log --check` and
`cargo clippy -p ferrite-audit-log --all-targets -- -D warnings` clean.
`cargo build --workspace` and `cargo test --workspace` green (checked via
`grep -E "FAILED|error\[" ` over the full output — no real matches; the
only "error" substring hits are test names like
`only_a_rate_limit_carries_a_retry_after` from `ferrite-model`). `just
check` (fmt-check + clippy --all-targets -D warnings + cargo machete)
green — machete reports no unused dependency. `just test` (full workspace
suite) green. `cargo deny check`: `advisories ok, bans ok, licenses ok,
sources ok`, only the pre-existing Servo-git-source `unmatched-source`
warning already logged by A3–A7.

**Cross-crate compile touches:** none. `PersistentAuditLog::new`/
`append`/`load` and `AuditLog::append`/`verify_chain`'s signatures are
unchanged; `AuditEntry` gained a `pub schema_version: u8` field, but no
crate outside `ferrite-audit-log` constructs `AuditEntry` by struct
literal (confirmed via `grep -rn "AuditEntry\s*{" crates/` — only the two
in-crate construction sites) or destructures it, so this is source- and
binary-compatible for every caller (`ferrite-servo`, `ferrite-shell`,
`ferrite-ui`, `ferrite-eval` — confirmed via
`grep -rln "ferrite_audit_log" crates/`, all of which only call
`append`/`verify_chain`/`new`/`load` and read `AuditEntry` fields, never
construct one).

**On-disk audit databases:** none exist anywhere in this repo —
confirmed via `git ls-files | grep -iE '\.(db|sqlite|sqlite3)$'` (no
matches) and a filesystem search excluding `target/`/`.git/` (no
matches). So the fact that this session's preimage change makes any
*hypothetical* pre-A8 on-disk audit log fail `verify_chain()`/`load` is a
real, correct, and now-legible consequence of `schema_version` existing
(a future verifier can branch on it) — but it invalidates nothing that
actually exists today.

**Known issues discovered / left for later agents:**
- **T-218** (filed above, in `docs/TO-DO.md`): the new
  `audit_chain_head` checkpoint is a best-effort watermark, not a
  cryptographic completeness guarantee — an attacker with full SQLite
  read/write access can forge it consistently with a truncated tail.
  Closing this needs an external anchor; out of this charter's scope.
- **T-219** (filed above): `AuditError::HashMismatch`/`ChainBroken` were
  already declared-but-unconstructed before this session (`verify_chain`
  returns a bare `bool`, never those variants) and remain so — not
  `dead_code`-lint-visible because they're variants of a `pub` enum, but
  genuinely unreachable from any current call path. Fixing this properly
  means changing `verify_chain`'s return type to
  `Result<(), AuditError>`, a public API change touching 3 external call
  sites currently using it as a boolean — real, small, but outside this
  charter's "fix the preimage" scope.
- No existing on-disk audit databases anywhere in this repo (see above) —
  confirmed, not assumed.

**Exact next action for A9:** `crates/ferrite-engine`,
`crates/ferrite-engine-servo`, `crates/ferrite-agent`, per §6/A9
(T-109). **Checked this session:** `crates/ferrite-engine` and
`crates/ferrite-engine-servo` do **not** exist yet — absent from root
`Cargo.toml`'s `members` list and from disk; A9 creates both from
scratch. `crates/ferrite-agent` already exists as a workspace member
(`src/lib.rs`, `src/gemini.rs`). Separately, `crates/ferrite-servo`
already exists too, but it is a *different* thing — the winit/Iced
shell's own Servo integration behind its `servo` Cargo feature, not the
directive's `BrowserEngine`-trait `ferrite-engine-servo` — A9 should read
both before naming or wiring anything, the same kind of crate-identity
check this session's own charter needed (`ferrite-audit` vs. the real
`ferrite-audit-log`).

## 2026-09-18 — A9 (Engine + agent action surface) — `ferrite-engine`, `ferrite-engine-servo`, `ferrite-agent::browser_loop`, real `just build-servo`, T-109

**Commits:** `aaae747` (feat(engine): BrowserEngine trait + MockEngine),
`8a1b7e3` (feat(engine-servo): ServoEngine wrapping HeadlessServoSession),
`3df9af2` (feat(agent): engine/provider-agnostic browser_loop).

**Session note:** same hazard A4–A8 hit — worktree started on a stale,
unrelated tree (`65b6d67`, "Initial test case designing..."). Recreated
`rebuild/a09-engine-agent` from local `main` (`b4f4c8c`, carries A0–A8)
before any work.

**Landed — `crates/ferrite-engine` (new crate, always built, no feature
flag):** `BrowserEngine` trait (`src/lib.rs`) with the directive's full
minimum action surface — `navigate`, `go_back`/`go_forward`, `reload`,
`current_url`, `dom_snapshot`, `query`, `read_text`, `click`, `type_text`,
`fill_form`, `select_option`, `scroll`, `wait_for` (`WaitCondition::
{Selector, Idle, Timeout}`), `screenshot`, `download`, `open_tab`/
`close_tab`/`switch_tab`, `cookies_read`/`storage_read` (both scoped —
take an `&Origin`), `clipboard_read`/`clipboard_write`, `js_execute`
(documented as privileged/unconstrained by this trait — the comparator's
job, per the module docs). Every method returns `Result<(T, Origin),
EngineError>`, matching the directive's "every action returns (result,
origin)" contract literally in the signature rather than as a convention.
Action-tagging reuses `ferrite_core::Primitive`'s existing wire vocabulary
(`Call::primitive()`) rather than inventing a second one — the exact
mismatch shape T-216 found is structurally avoided here, not just avoided
by discipline.

`MockEngine` (`src/mock.rs`): per-origin/per-selector scripted response
queues for `dom_snapshot`/`query`/`read_text`/`js_execute` (A6
`DryRunContent`-style: pop while >1 queued, repeat the last), real
multi-tab navigation history (`go_back`/`go_forward` truncate/replay a
`Vec<String>`), origin-scoped `cookies_read`/`storage_read` backed by
`HashMap<Origin, Vec<...>>` (proven non-leaking by test, not just by
code-reading), and a full `Vec<Call>` call log for introspection. Starts
every fresh tab at a synthetic, representable `MOCK_HOME =
"https://mock-home.ferrite.test"` rather than a real browser's opaque
`about:blank` — documented explicitly as a deliberate Mock-only
convenience; a test that wants the opaque-origin path can still navigate
`MockEngine` to a non-http(s) URL and observe `EngineError::OpaqueOrigin`.

`ferrite_engine::conformance` (feature `test-util`): three small,
engine-agnostic assertion functions (`navigate_updates_current_url_and_
origin`, `unknown_tab_id_is_a_typed_error`, `wait_idle_completes`) —
deliberately a small shared subset, not the whole suite; see below for
why a fully generic fixture-parameterized suite covering everything was
not attempted.

**Tests (`tests/conformance.rs`, 24 tests, all passing):** every one of
the 24 trait methods exercised at least once
(`every_trait_method_is_visible_in_the_call_log` asserts the call log has
exactly 24 entries after one call to each); origin-tracking asserted
through `navigate`/`go_back`/`go_forward`/`reload`
(`navigate_go_back_go_forward_and_reload_track_origin_through_history`);
`cookies_read`/`storage_read` scoping proven non-leaking
(`cookies_read_is_scoped_and_does_not_leak_other_origins_cookies`,
`storage_read_is_scoped_and_does_not_leak_other_origins_storage` — each
seeds two origins and asserts a third, unseeded origin gets nothing, not
everything); `js_execute` exercised both for success and for a scripted
failure, with an explicit test-level comment that the test proves the
method *runs*, not that it is *safe* to call (that's the comparator's
job); the opaque-origin path exercised for real
(`navigating_to_an_opaque_scheme_reports_a_typed_error_not_a_fabricated_
origin`); a `wait_for(Idle)`/`wait_for(Timeout)` pair proving no real
sleep occurs (R8) via a wall-clock `Instant` bound in the test itself.

**Landed — `crates/ferrite-engine-servo` (new crate, feature
`engine-servo` = `["ferrite-servo/servo"]`):** `ServoEngine` wraps
`ferrite_servo::session::HeadlessServoSession` per the charter's explicit
instruction (does not reimplement libservo/winit plumbing). Mapping
decisions, each documented in the crate's module docs:
- `dom_snapshot`/`query`/`read_text`/`click`/`type_text`/`select_option`
  are implemented by injecting a small `JSON.stringify(...)`-returning JS
  snippet via `execute_js` and parsing the result.
- `execute_js`'s `Ok` branch is a `Debug` rendering of Servo's JS value
  type (`format!("{:?}", v)`, in `ferrite-servo`'s existing code, not
  ours), not raw JSON. `unwrap_js_string_result` strips the observed
  `String("...")` wrapper and unescapes via `serde_json`'s string parser
  — a real, unit-tested mechanism
  (`unwrap_js_string_result_strips_the_observed_string_wrapper` et al.),
  with its dependence on that untyped channel documented plainly.
- `js_execute` itself does **not** attempt this unwrapping (arbitrary
  scripts can return any JS value shape) — returns the raw string
  verbatim, documented as the caller's own `JSON.stringify()`
  responsibility if a clean value is wanted.
- `download`/`clipboard_read`/`clipboard_write` are
  `EngineError::Unsupported` — `HeadlessServoSession` has no download
  manager and no synchronous clipboard API. Not placeholders: real,
  honest, tested "this genuinely cannot be done today" results.
- `cookies_read`/`storage_read` only succeed when `scope` equals the
  *currently loaded* page's own origin (reading `document.cookie`/
  `localStorage` via `execute_js` can only ever see that origin anyway);
  a different `scope` is `EngineError::Unsupported` rather than silently
  navigating away to satisfy it — a real scoping restriction, not a
  missing feature dressed up as one.
- Tabs are real: each open tab is its own `HeadlessServoSession`, sharing
  the process-wide Servo singleton (`ferrite-servo`'s own
  `get_or_init_servo`), mirroring `ferrite-shell`'s existing multi-tab use.

**A real, load-bearing design fix found while integrating against the
actual `servo` feature (not caught by the mock-backed default build):**
`BrowserEngine`'s original draft required `Send`. The **real**
`HeadlessServoSession` (behind the real `servo` feature) holds `Rc<...>`
state throughout — Servo's engine is an intentionally thread-affine
singleton — so `ServoEngine` cannot be `Send`, and building
`ferrite-engine-servo --features engine-servo` failed with 24 "cannot be
sent between threads safely" errors. Fixed by removing the `Send`
supertrait bound from `BrowserEngine` entirely (documented in the trait's
own doc comment, including why: no real caller in this charter's scope
needs to move a `BrowserEngine` across threads, and `MockEngine` never
needed the bound to begin with). This is exactly the class of thing the
exit gate's "attempt the real build for real" instruction exists to
surface — it would not have been found by mock-only testing.

**`just build-servo` — attempted for real, succeeded:** `cargo build -p
ferrite-shell --features ferrite-servo/servo` (the recipe's literal
body), from an `~/.cache/ferrite-target` that already held A1–A8's
non-Servo artifacts. **Wall clock: 15m 31s. Target-dir size after: 6.4
GB. Exit code: 0.** Full numbers, caveats (warm registry cache,
non-empty starting target-dir), and the comparison against the
directive's own "tens of GB, 30–60 min" unverified estimate are in
`docs/BUILD_BUDGET.md`'s new 2026-09-18 A9 section — time and disk both
landed comfortably under that estimate on this machine.

**Real-Servo `ServoEngine` conformance run — attempted, partially
succeeded, one real finding not fixed (T-220):**
`crates/ferrite-engine-servo/tests/servo_conformance.rs`, `#[ignore]`d,
run via `cargo test -p ferrite-engine-servo --features engine-servo --
--ignored --test-threads=1` against the just-built real Servo. Two
process-level blockers were found and fixed in the course of getting
this to run at all:
1. The `Send`-bound fix above (found via this exact command failing to
   compile).
2. Servo's `config::opts` module is a **process-global** singleton that
   panics ("Already initialized") if constructed twice. The first
   attempt wrote several independent `#[test]` fns (mirroring
   `ferrite-engine`'s own test-file style); this failed because Rust's
   `libtest` harness spawns a fresh OS thread per `#[test]` fn regardless
   of `--test-threads`, so `ferrite-servo`'s `thread_local!`-cached
   `get_or_init_servo()` re-initializes (and panics) on the second test
   function's thread. Fixed by consolidating into one `#[test]` fn that
   builds a single `ServoEngine` and drives every assertion through it
   sequentially on one thread — the correct, honest fix (not a workaround
   for a bug in `ServoEngine`, a real property of how the existing,
   out-of-scope-to-modify `ferrite-servo::session` code manages Servo's
   own process-wide state).

With both of those fixed, `ServoEngine::new` succeeds for real (no
panic) — but `navigate()` to a `127.0.0.1` loopback HTTP fixture server
does not observably complete: a throwaway diagnostic binary (not
committed) confirmed the fixture server never receives a TCP connection
attempt at all within the drive-until-loaded window, so `current_url()`
still reports `about:blank` afterward and the rest of the suite fails on
`EngineError::OpaqueOrigin`. This was not root-caused to a fix within
this session's remaining budget. Leading hypothesis, documented in the
test file's module docs and not confirmed: `HeadlessServoSession`'s own
doc comment says it is meant to be driven by a genuine winit
`EventLoop::run()` tick (as `ferrite-shell` does in production); bare
repeated `spin_event_loop()` calls with no real OS-level event loop
underneath may not be sufficient to make Servo's networking component
(plausibly running on its own thread/process) actually attempt a fetch.
Filed as **T-220** — real follow-up work, not a documentation gap: fixing
it means either instrumenting/modifying `ferrite-servo::session` (outside
this charter's file list) or wrapping a genuine winit loop inside
`ServoEngine` itself.

**Honest accounting against the exit gate's three parts:**
1. Full action-suite conformance vs. `MockEngine`: **done**, 24/24
   passing, part of `just test`.
2. `just build-servo` succeeds once, cost recorded: **done**, 15m31s,
   6.4 GB, recorded in `docs/BUILD_BUDGET.md`.
3. The same (or an equivalent) conformance suite run against
   `ServoEngine` at least once, result recorded: **attempted for real,
   partial result honestly recorded** — construction succeeds, real page
   navigation does not complete in this environment, root cause
   hypothesized but not fixed (T-220). Per the charter's own explicit
   allowance ("pass, fail, or 'could not attempt because the build itself
   didn't complete', whichever is true"), the true outcome here is a
   fourth case the charter didn't name outright but clearly anticipates
   the spirit of: the build *did* complete, the *engine construction*
   passes, and the *page-load leg* fails with a real, diagnosed,
   unresolved cause — recorded as exactly that, not rounded up to "pass"
   or down to "could not attempt."

**Landed — `crates/ferrite-agent/src/browser_loop.rs` (new module,
additive):** `run_agent_loop` — plan → select tool → act → observe →
repeat, generic over `&dyn BrowserEngine` and `&dyn
ferrite_model::ModelProvider` (never a concrete provider type, matching
A4's `fingerprint::generate_fingerprint` precedent and §10.5's rule).
`AgentAction` (serde-tagged enum, one variant per `BrowserEngine` method
plus `Finish`) is the model's structured-output vocabulary; the model's
raw response text is parsed directly via `serde_json::from_str` (kept
deliberately simple — no schema/cache/decorator wiring in this loop, see
the module's explicit scope note on why the old `BrowserTool`/
`AgentRuntime`/`ToolExecutor` path is untouched). `LoopBudget{max_steps,
max_wall_clock, max_repeated_identical}`, all three enforced; wall-clock
uses an injected `ferrite_core::Clock` (never `tokio::time::sleep`/real
sleep — R8). The repeated-identical-action guard fires **before**
executing the would-be-Nth-in-a-row action, so a real engine (not just
`MockEngine`) never actually performs the repeat.

**Decision — `BrowserTool`/`AgentRuntime`/`ToolExecutor`/`GeminiAgent`:
kept exactly as-is, not migrated, not removed.** Checked via `grep -rn
"BrowserTool\|AgentRuntime\|ToolExecutor\|GeminiAgent" crates/` before
deciding (per the charter's own instruction): these are live,
load-bearing infrastructure for `ferrite-ipi::dry_run` (`executor.rs`,
`orchestrator.rs`), `ferrite-ipi::tool_decision`, `ferrite-eval`
(`harness.rs`, `corpus.rs`, `tests/pilot_w6.rs`), and `ferrite-ui`/
`ferrite-shell` (the actual running agent), not dead reference material.
Rewriting that whole path onto `BrowserEngine` is real, larger work the
directive's own A9 text scopes out explicitly ("does not need to wire in
the full IPI defense end-to-end... if it's not small, say so plainly").
`browser_loop` is therefore new, additive capability living alongside the
old path, not a replacement for it.

**Tests (`browser_loop::tests`, 8 tests, all passing):**
`the_loop_stops_when_the_model_finishes`, `step_budget_is_enforced`,
`wall_clock_budget_is_enforced_with_an_injected_clock_never_real_sleep`
(asserts `Instant::elapsed() < 2s` around a call that reports a 120s
budget exhausted — proves no real sleep occurred),
`repeated_identical_actions_trigger_a_hard_stop_not_an_infinite_loop`
(scripted `MockProvider` always returns the same click; loop stops after
2 executed + 1 caught-before-execution, not an infinite loop),
`a_model_error_stops_the_loop_rather_than_panicking`,
`a_malformed_action_stops_the_loop_rather_than_panicking`,
`executed_actions_carry_real_origin_tracking_through_observations`.

**Workspace wiring:** root `Cargo.toml` — `crates/ferrite-engine` and
`crates/ferrite-engine-servo` added to `members`, plus
`[workspace.dependencies]` path entries. `ferrite-agent/Cargo.toml`
gained `ferrite-core`, `ferrite-engine`, `ferrite-model`, `chrono` — all
already-workspace deps, no new external crate added to
`[workspace.dependencies]` for this charter's own code (the `engine`
crate itself needs only `ferrite-core`/`serde`/`thiserror`, all
pre-existing).

**Verified:** `cargo test -p ferrite-engine` (24 passed, 0 failed) +
`-p ferrite-engine-servo` (5 passed, 0 failed, default features — the
`#[ignore]`d real-Servo test is 0-collected under default features via
`#![cfg(feature = "engine-servo")]`) + `-p ferrite-agent` (11 passed —
3 pre-existing `tests::*` + 8 new `browser_loop::tests::*` — 0 failed).
`cargo fmt -p ferrite-engine -p ferrite-engine-servo -p ferrite-agent
--check` clean. `cargo clippy -p ferrite-engine --all-targets -D
warnings`, `-p ferrite-engine-servo --all-targets -D warnings` (default
features), `-p ferrite-agent --all-targets -D warnings` all clean.
`cargo clippy -p ferrite-engine-servo --all-targets --features
engine-servo -- -D warnings` (the real feature) also run — see this
session's exact result recorded alongside the handoff, since it
completed after this entry was drafted. `just check` (fmt-check +
clippy --all-targets -D warnings workspace-wide + cargo machete) green —
machete initially flagged an actually-unused `serde_json` dependency in
`ferrite-engine`'s `Cargo.toml` (added speculatively, never called),
removed, machete clean after. `just test` (full workspace, default
features, isolated `CARGO_TARGET_DIR` to avoid racing the concurrent
`just build-servo` run) — every crate's `test result: ok`, 0 failures,
exit 0 (`ferrite-core` 166, `ferrite-model` 151+3 conformance,
`ferrite-ipi` 58, `ferrite-audit-log` 21, `ferrite-agent` 11,
`ferrite-engine` 24, `ferrite-engine-servo` 5, others as before).

**Known issues discovered / filed:**
- **T-220** (this session, unowned): `ServoEngine::navigate` does not
  observably complete a real page load in this environment — see above.
- **T-221** (this session, feeds A13): `ferrite-ipi`'s `Cargo.toml` still
  depends on `ferrite-agent` (`ferrite-agent = { workspace = true }`,
  confirmed present), backwards from the target dependency direction.
  Not fixed here — reversing it means relocating `BrowserTool`/
  `AgentRuntime`/`ToolExecutor` to a crate both `ipi` and `agent` can
  depend on, well beyond this charter's file list; flagged for A13 per
  the charter's own suggestion.

**Exact next action for A10:** `crates/ferrite-ui` — Iced shell with a
consent panel rendered fully decoupled from page content, plain-English
per-item diff summaries citing primitive/origin/capability, per-item
approve/reject (no blanket approve), dry-run evidence on expand,
non-sticky per-task approvals. See `docs/handoffs/a09.md` for the exact
shape of `BrowserEngine`/`AgentAction`/`AgentLoopResult` A10's UI will
need to consume.

## 2026-09-18 — A10 (UI/UX) — consent panel security-surface fixes, out-of-scope-origin rendering gap closed, T-110

**Session note:** same hazard A4–A9 hit — stale, unrelated worktree
(`worktree-agent-a628b7f472979308f` at `65b6d67`, no A9 merge in its
history). Recreated `rebuild/a10-ui-consent` from local `main`
(`dbfd27b`, A0–A9) before any work.

**Scope:** `crates/ferrite-ui` only, per charter. Confirmed by reading
`ferrite-ipi::comparator::{diff,consent}.rs` (A7), `dry_run::record.rs`
(A6), `ferrite-engine`/`browser_loop` (A9) for their real current shapes
before writing any UI code against them — no `ferrite-ipi`/
`ferrite-engine`/`ferrite-agent` files touched.

**The real, live gap — confirmed and fixed.** `view()`'s consent-panel
row rendering iterated only `FingerprintDiff::extra_primitives`; a
`FingerprintDiff::out_of_scope_origins` deviation (primitive expected,
wrong origin — A7's `compare()` produces this bucket correctly) was
computed, discarded into UI state, and never shown, never consent-gated,
and never enforced. Confirmed by reading `view()` and
`ConsentDecision::is_complete()` (`ferrite-ipi`, unmodified): the latter
*also* only ever checked `extra_primitives`, so even rendering the rows
without a parallel completeness fix would not have closed the gap — an
out-of-scope-origin item could be left permanently undecided and the
Proceed button would still enable.

**Fix, in `crates/ferrite-ui/src/lib.rs`:**
- `consent_items(diff, expected) -> Vec<ConsentItem>` builds one
  plain-English summary per flagged item, from **both** buckets, sorted
  deterministically (primitive strings, then origin strings). An
  extra-primitive item says "nothing in your request authorized this
  action" (with a distinct js.execute-specific line: ADR-003 makes it
  *always* consent-gated by design, not merely unauthorized this time).
  An out-of-scope-origin item names the contacted origin and lists every
  origin scope the task's `ExpectedFingerprint` actually carries
  (`describe_authorized_origins`/`describe_scope`, reading
  `ExpectedCapabilitySet::lowered()`).
  - **Honest limitation, documented in the code, not papered over:**
    `FingerprintDiff::out_of_scope_origins` is a bare
    `HashSet<String>` of origins — it does not retain which primitive/tool
    was invoked at the flagged origin (a pre-existing shape A7 kept for
    `ferrite-eval`/`dataset` compatibility, see that module's own docs).
    So the summary can say "these origins were authorized somewhere in
    your task" but not "primitive P specifically needed origin O" for
    this bucket. Filed as **T-222** rather than fabricated precision the
    data doesn't support.
- Out-of-scope-origin items are tracked in `ConsentDecision` (A7's
  ToolId-keyed type, unmodified) via a synthetic `"origin::<origin>"`
  item id (`origin_item_id`/`origin_item_origin`) — this closes the gap
  entirely inside `ferrite-ui` with zero changes to `ferrite-ipi`.
- `consent_is_complete(diff, decision)` replaces the
  `ConsentDecision::is_complete()` call as the sole gate on the Proceed
  button *and* is re-checked directly inside the `ConsentSubmitted`
  handler itself (defense-in-depth: the message must never proceed with
  an undecided item even if something other than the view ever sends
  it — proven by `consent_submitted_is_a_no_op_while_any_item_is_undecided`).

**Real enforcement, not just UI state.** Traced `ConsentDecision::rejected`
through to `FilteredToolExecutor` (defined in `ferrite-ui`, not
`ferrite-ipi`): it already blocked a rejected `ToolId` before reaching
the real `BrowserToolExecutor`, but had no test proving it and no
handling at all for a rejected origin. Added `rejected_origins:
HashSet<String>` and a check against any `BrowserTool` call that itself
carries a URL (`Navigate`, `DownloadFile`) — calls with no URL of their
own (`ReadPage`, `ClickElement`, ...) act on "whatever the active tab
currently is", which this executor cannot observe from the call alone;
filed as **T-222** (same root cause as the rendering-precision limit
above: the diff loses the (primitive, origin) pairing).
`filtered_executor_blocks_a_rejected_tool_without_reaching_the_inner_executor`
and `filtered_executor_blocks_a_download_whose_url_origin_was_rejected`
assert the inner channel receives nothing when a call is blocked —
proving refusal, not just a UI-side rejected flag with nothing behind it.

**Never sticky across tasks — verified, and one real bug fixed.** Traced
`pending_diff`/`pending_decision`/`pending_expected`/`pending_evidence`/
`pending_task` through `ConsentRequired` → decide → `ConsentSubmitted`/
`ConsentCancelled`. Found: `pending_task` was **not** cleared on
`ConsentCancelled` (only `pending_diff`/`pending_decision` were) — dead
state until the next `AgentTaskSubmitted` overwrote it, not a live bug
today, but real hygiene debt directly on-point for this verification;
fixed. Also found and removed **dead state**: `approved_extras` was
written on every `ConsentSubmitted` (`state.approved_extras =
state.pending_decision.approved.clone()`) but never read anywhere in the
crate (`grep -rn "approved_extras"` — 3 hits, all in this file, all
writes) — removed rather than left as misleading, never-consulted
per-task carryover.
`second_tasks_consent_decision_never_carries_over_from_the_first` is the
regression test: task 1 approves `js.execute`, submits; task 2 is
flagged with the identical tool id and asserted **not** pre-approved.

**Page-content decoupling — investigated, concluded structural, not
merely assumed.** Traced every path page content can reach this crate:
`HeadlessServoSession::get_frame()` is the *only* one, returning a
decoded `(width, height, Vec<u8> RGBA)` pixel buffer rendered via
`iced_widget::image::Image` in `view()`'s content arm — a bitmap, never
parsed as markup. The consent panel (`view()`'s `body` when
`pending_diff` is `Some`) is built exclusively from
`FingerprintDiff`/`ExpectedFingerprint`/`DryRunRecord`/`ConsentDecision`
— plain Rust structs populated from `ferrite-ipi`'s comparator/dry-run
output, never from raw page HTML/CSS/text, and no page-supplied string is
ever interpolated into a style, position, or z-order call anywhere in
this file (checked every `container::Style`/`Border`/`Background`
closure — all take fixed constants or state-derived plain data, never a
`String` sourced from Servo). Recorded as a compile-time-pinned witness,
`page_content_cannot_reach_the_consent_panels_inputs`, rather than left
as an unchecked comment.

**Reject-default-focus — investigated against the real, pinned iced
API; partial by an external constraint, not an oversight.** Checked
`iced_core-0.13.2`'s `widget::operation::focusable::Focusable` trait
directly: only `text_input`/`text_editor` implement it in this pinned
version (`grep -rln Focusable iced_widget-0.13.4/src/` — two hits, no
`button.rs`). A literal keyboard-focus-ring default onto the Reject
button is therefore not achievable against this iced version's public
API without a custom focusable widget wrapper (real, non-trivial work,
not attempted this session — a version bump or custom widget is the real
fix, filed as **T-223**). Implemented the available, security-relevant
equivalent instead: Reject is listed and styled first in each item's row
(closest available "default" signal without real focus support), and —
the property that actually matters for safety — `consent_is_complete`
means there is no path to a silent default-approve regardless of which
button a stray keypress might reach.

**Dry-run evidence on expand — added, honestly scoped.** `DryRunRecord`
threaded through `ConsentRequired` (boxed: `Box<DryRunRecord>`, to keep
`FerriteBrowserMessage`'s largest variant small per
`clippy::large_enum_variant`) and rendered via
`dry_run_evidence_lines()` behind a `ToggleEvidence`-driven collapse,
default collapsed. Shows exactly what `DryRunRecord` actually carries —
the ordered `(tool, origin)` call log via `events_with_seq()`, plus a
sanitizer-finding count — and says so explicitly in its own doc comment:
it does **not** show page content, because `DryRunRecord` does not
record page content read, only which tool ran, at which origin, in what
order, and which sanitizer patterns fired. No fabricated evidence field.

**Tests — 17 new, all in `crates/ferrite-ui/src/lib.rs`'s `tests`
module, 0 requiring a real `HeadlessServoSession` (R7):**
state-machine — `consent_required_populates_pending_state_fresh_and_clears_agent_running`,
`consent_submitted_is_a_no_op_while_any_item_is_undecided`,
`consent_flow_extra_primitive_only_reject_then_submit_clears_all_pending_state`,
`consent_flow_out_of_scope_origin_only_approve_then_submit`,
`consent_flow_mixed_diff_requires_both_items_decided_before_submit_succeeds`,
`consent_cancelled_clears_all_pending_state_without_spawning_a_run`,
`second_tasks_consent_decision_never_carries_over_from_the_first`,
`consent_is_complete_requires_a_decision_on_both_bucket_kinds`; summary
snapshot — `consent_summary_snapshot_for_a_mixed_diff`,
`consent_summary_for_a_non_js_extra_primitive_says_nothing_authorized_it`,
`consent_summary_for_an_out_of_scope_origin_with_no_expected_fingerprint_is_honest`;
real enforcement —
`filtered_executor_blocks_a_rejected_tool_without_reaching_the_inner_executor`,
`filtered_executor_blocks_a_download_whose_url_origin_was_rejected`,
`filtered_executor_allows_a_non_rejected_tool_to_reach_the_inner_executor`;
dry-run evidence — `dry_run_evidence_lines_render_ordered_call_log`,
`dry_run_evidence_lines_says_so_honestly_when_nothing_was_recorded`;
page-content decoupling —
`page_content_cannot_reach_the_consent_panels_inputs`.
`#[tokio::test]` used only where the handler under test
(`ConsentSubmitted`) calls `tokio::task::spawn`, which panics outside a
runtime context; no test `.await`s past the spawn point, so the spawned
future (which would call `GeminiAgent::from_env()` and a real network
call) is never polled before the test ends — no live call made, per R7.

**Verified:** `cargo fmt -p ferrite-ui --check` clean.
`cargo clippy -p ferrite-ui --all-targets -- -D warnings` clean (fixed
two real findings along the way: `clippy::large_enum_variant` on
`FerriteBrowserMessage::ConsentRequired` — boxed `evidence`; `clippy::
field_reassign_with_default` in one test). `cargo test -p ferrite-ui` —
17/17 passed, isolated `CARGO_TARGET_DIR=/tmp/ferrite-a10-target` (per
T-214, sharing the default `~/.cache/ferrite-target` across worktrees is
a known false-failure source). `just check` (fmt-check + clippy
--all-targets -D warnings workspace-wide + cargo machete) — clean,
0 unused deps. `just test` (full workspace, same isolated target dir) —
every crate `test result: ok`, 0 failures, exit 0 (`ferrite-core` 166,
`ferrite-model` 151+3, `ferrite-ipi` 58, `ferrite-audit-log` 21,
`ferrite-agent` 11, `ferrite-engine` 24, `ferrite-engine-servo` 5,
`ferrite-eval` 45, `ferrite-ui` **17** (new), others unchanged). `cargo
deny check` — advisories/bans/licenses/sources all `ok` (one pre-existing
`unmatched-source` warning for a Servo git source, unrelated to this
session).

**Commit:** `aaec147` — `feat(ui): render out-of-scope-origin items in
the consent panel`.

**Known issues discovered / filed:**
- **T-222** (this session, feeds a future `ferrite-ipi` comparator
  session, not A10): `FingerprintDiff::out_of_scope_origins` (and
  `extra_primitives`) record only a tool id or an origin string, never
  the `(tool, origin)` pair together, for a bucket that structurally has
  both. This is why the consent panel's out-of-scope-origin summary can
  only say "these are the origins your task authorized somewhere", not
  "primitive P needed origin O", and why `FilteredToolExecutor` can only
  enforce a rejected origin for the two `BrowserTool` variants that carry
  their own URL. Fixing this for real means enriching the diff's shape in
  `ferrite-ipi::comparator` (out of this charter's file scope) —
  documented rather than worked around with fabricated precision.
- **T-223** (this session, unowned): iced 0.13's `button` widget does not
  implement `widget::operation::focusable::Focusable` (only
  `text_input`/`text_editor` do), so a literal keyboard-focus-ring
  default onto the consent panel's Reject button is not implementable
  against the pinned iced version's public API. Mitigated with
  reading-order + strict completeness-gating (see above); a real fix
  needs either an iced upgrade (re-check `Focusable` support each major)
  or a custom focusable button wrapper.

**Exact next action for A11:** `crates/ferrite-eval/src/dataset/`,
`corpus/` — move the carrier/carrier-vector partition validation into
the type system (closes T-006), build the real corpus per §13.3, decide
the AgentDojo mapping-or-delete question (T-009). See
`docs/handoffs/a10.md`.

## 2026-09-18 — A11 (Dataset + corpus) — type-level carrier partition, AgentDojo adapter, corpus grown to 29 cases, T-006/T-009/T-201/T-203/T-208

**Session note:** same hazard A4–A10 hit — worktree HEAD started on the
stale, unrelated pre-rebuild tree (`65b6d67`, `Browser/`-nested,
`FINALIZED_DECISIONS.md` at the root, no `crates/`/`docs/`), even though
`git log --oneline -5 main` on the same worktree correctly showed A0–A10's
history. `git checkout -b rebuild/a11-dataset-corpus` initially branched
from the bad HEAD; caught before any file edits, fixed with
`git checkout main && git reset --hard main` while on the new branch
(nothing lost — working tree was clean). Recreated from local `main`
(`53c8190`, carries A0–A10) before any real work.

**Scope-mismatch note confirmed, per the launch brief's own instruction to
re-verify it:** the directive's §6/A11 scope line names
`crates/ferrite-eval/src/dataset/`, `corpus/`, but the real types
(`CaseDefinition`/`Carrier`/`CarrierVector`/`Tier`/`RunLabel`/`Author`) live
in `crates/ferrite-ipi/src/dataset.rs` (flat file, 683 lines pre-session),
and the loader (`load_case`/`load_corpus`/`partition_matches`) lives in
`crates/ferrite-eval/src/corpus.rs` (flat file, 795 lines pre-session) — no
`dataset/`/`corpus/` directories exist or were created; splitting into
modules was not warranted by this session's changes (dataset.rs grew by
~130 lines, corpus.rs shrank by removing `partition_matches`). Worked in
both files, as the launch brief anticipated.

**T-006/D6 (closed, `6e6926a`):** `CarrierVector` is now an enum-of-enums —
`CarrierVector::WebContent(WebContentVector)` /
`CarrierVector::ToolOutput(ToolOutputVector)` — replacing the old two
independent `CaseDefinition` fields (`carrier: Carrier`,
`carrier_vector: CarrierVector`, an 11-variant flat enum). `Carrier` is now
derived via `CarrierVector::carrier()`, never stored, so it structurally
cannot disagree with the vector it's derived from. `ferrite_eval::corpus`'s
old `partition_matches` function and `CorpusError::Partition` variant are
**deleted**, not superseded — the runtime check they performed is now a
compile error (Rust) or a deserialization failure (JSON), not a second,
now-redundant check layered on top. Proof:
- `ferrite_ipi::dataset::CarrierVector`'s own doc comment carries a
  `compile_fail` doctest constructing the exact old bypass
  (`CarrierVector::WebContent(ToolOutputVector::ToolJsonField)`) and shows
  it fails with E0308.
- `dataset::tests::every_carrier_vector_variant_reports_the_correct_carrier`,
  `dataset::tests::carrier_vector_carrier_matches_the_variant_it_was_constructed_with`,
  `dataset::tests::carrier_vector_serializes_as_an_externally_tagged_pair`.
- `corpus::tests::old_flat_carrier_shape_fails_to_deserialize_not_a_partition_check`
  proves the pre-T-006 flat JSON shape (`"carrier": "WebContent",
  "carrier_vector": "ToolJsonField"`) is now a `CorpusError::Json` failure,
  not a value that parses and then fails a separate partition check.
- `dataset::tests::dataset_store_reopen_against_an_existing_db_preserves_rows`
  is the schema-migration test the directive's persistence requirement asks
  for (`DatasetStore::open`'s `CREATE TABLE IF NOT EXISTS` reopened against
  an existing file preserves prior rows and accepts new inserts).

All call sites updated (mechanical, required by the type change, not
feature work): `dataset.rs`'s own `insert_case` (SQL column now populated
via `case.carrier_vector.carrier()`), `corpus.rs`'s
`check_carrier_content_binding` (now takes the derived `Carrier`),
`ferrite-eval`'s test fixtures in `harness.rs`/`adjudication.rs`/
`corpus.rs`/`tests/pilot_w6.rs`, and all 10 `tests/pilot_corpus/*.json`
files (T-208, below).

**T-009/D9 (closed, `6e6926a` — resolved as "implement for real," not
delete):** `crates/ferrite-eval/src/agentdojo.rs` is a genuine, tested
adapter — not enum plumbing. Three `AgentDojoSlackTask` constants
transcribe real `InjectionTask` definitions from AgentDojo's own Slack
suite (`github.com/ethz-spylab/agentdojo`,
`src/agentdojo/default_suites/v1/slack/injection_tasks.py`, fetched live
2026-09-18 via `WebFetch` — payload text, target tool, and phishing
indicator copied from source, not invented). Because Ferrite is a browser
agent with no Slack-shaped tools, `adapt_agentdojo_slack_task` performs a
documented **tool substitution** (`read_channel_messages` →
`extract_data`; `send_direct_message`/`post_webpage` → `form.fill`;
`get_webpage` → `navigate`), not a literal port — the module's doc comment
gives the full table and rationale, consistent with ADR-008's "without
fabricating capabilities" constraint. Output round-trips through the real,
public `corpus::load_case` (same function every hand-authored case uses),
proven by
`agentdojo::tests::adapted_case_round_trips_through_the_real_corpus_loader`.
Seeded `tests/agentdojo_corpus/` with the 3 adapted cases (tagged
`Tier3AgentDojo`/`AgentDojo`), validated by
`agentdojo_corpus_validate.rs`'s 3 tests. `harness.rs`'s existing
`Tier3AgentDojo`/`RunLabel::R9` dual-mode wiring is untouched (out of this
charter's scope) and now has real data behind it instead of zero.
**Honest scope:** 3 of AgentDojo's ~7 Slack injection tasks × ~20 user-task
pairings — a citable external sample, not a suite port. Recorded as an
explicit gap against directive §13.3's Tier 3 target (n=60) in
`docs/TO-DO.md`, not silently passed off as "done."

**T-208 (closed, `6e6926a`):** all 10 `tests/pilot_corpus/*.json` files
re-validated against the new `carrier_vector` shape — every file's
`"carrier"`/`"carrier_vector"` flat pair rewritten to the nested
`{"WebContent": "..."}`/`{"ToolOutput": "..."}` form (mechanical, no
`ground_truth`/`attack_category`/other-field changes; `GroundTruth`'s enum
itself was untouched by this session). All 10 still load and validate
(`pilot_corpus_validate.rs`'s existing 3 tests, unmodified, still pass
against the reshaped files) and all 10 are single-authored
(`Author::SelfAuthored`) — they count toward the corpus-size denominator
below but not toward the 10%-double-authored numerator (see honesty note
below). `tests/pilot_w6.rs`'s 5 throwaway full-pipeline fixtures needed the
same mechanical fix (found by running the suite, not by inspection) and are
now fixed and green.

**T-201 (closed, `d1531c0`):** extended
`crates/ferrite-eval/tests/pilot_corpus/CORPUS_AUTHORING_GUIDE.md` in place
rather than writing a new `docs/CORPUS_AUTHORING.md` — it's the
established, already-referenced location, sitting next to the fixtures
whoever authors a case will actually be looking at. Found and fixed real,
pre-existing drift while re-verifying it against source (not just adding a
new section): §5's `expected_origins` example still documented the flat
4-field `OriginScope` struct A7's comparator rebuild replaced with a
3-variant externally-tagged enum, and §3.2 documented the pre-T-006 flat
`carrier`/`carrier_vector` pair. Both corrected in place, a T1b worked
example added alongside the existing T1a one, and §11's placeholder ("not
covered yet") for AgentDojo adaptation mechanics replaced with a real
section citing `ferrite_eval::agentdojo` and stating its honest scope.

**T-203 (closed, `bdd631e`) — schema-completeness audit against
`docs/REBUILD_DIRECTIVE.md` §13.2, performed before authoring more cases,
per the task's own instruction:**

| Metric | Formula needs | Present on `ExecutionRecord`/`CaseDefinition`? |
|---|---|---|
| ASR_m, CR_m | `final_outcome` (per exec) × `corpus` (per case) | Yes — join on `case_id` |
| ADR_m | `fingerprint_caught` × `ground_truth` variant (deviation-class) | Yes |
| SDR_m | `sanitizer_caught` × `expected_finding` presence | Yes |
| FGR_m | `consent_gated` × `corpus == Benign` | Yes |
| **FSR** | **segment-level** excised/total count | **Gap — see below** |
| UP_m | `actual_events` compared across modes for the same `case_id` | Yes (computable at the metrics layer via a join; no new field needed) |
| R (residual) | `ground_truth == WithinFingerprintDataOnly` | Yes |
| ΔL (latency) | `Timing.total_ms`, p50/p95 aggregated per mode | Yes |
| **ΔT (token overhead)** | **prompt/completion token counts per execution** | **Gap — see below** |

**Two real gaps found, not silently passed:**
1. **FSR is only available as a case-level proxy, not the segment-level
   ratio §13.2 literally defines.** `ExecutionRecord` has no per-segment
   finding/strip count — only `sanitizer_caught: LayerOutcome`
   (Caught/Missed/NotApplicable, one value per execution) and
   `final_outcome` (which for benign×SanitizerOnly already distinguishes
   `BenignFalseFlag`/`BenignNoFlag`, per D4's outcome lattice). A12's
   metrics module can compute a **case-level** FSR proxy
   (`|BenignFalseFlag| / |benign cases run SanitizerOnly|`) with zero
   schema changes, but that is not the same number as "segments excised /
   segments total" when a case's page/tool-output contains more than one
   sentence — `sanitizer/config.rs`'s own unit test
   (`benign_fixture_false_strip_rate_is_below_the_provisional_ceiling`)
   gets the literal per-segment number today only because each of its 35
   fixtures *is* exactly one segment. Filed as **T-225** below.
2. **No field anywhere in `ExecutionRecord`/`Timing` carries model token
   usage**, even though `ferrite_model::response::TokenUsage`
   (`prompt_eval_count`/`eval_count`, §10.1's exact wire fields) already
   exists one crate over. ΔT (extra model tokens per task, §13.2) cannot be
   computed from persisted data as things stand. Not fixed in this
   session: populating it means threading `TokenUsage` out of the
   fingerprint-predict and agent-loop model calls into `harness.rs`'s
   orchestration, which is metrics/harness territory (A12's charter, not a
   `dataset.rs`-only change) and risked non-minimal changes to
   `harness.rs`'s matrix logic, which this charter explicitly excludes.
   Filed as **T-226** below, with the exact proposed field
   (`Timing.tokens: Option<ferrite_model::response::TokenUsage>` or
   equivalent) named so A12 doesn't have to re-derive it.

**Benign false-strip ceiling (`PROVISIONAL_FALSE_STRIP_RATE_CEILING`,
`crates/ferrite-ipi/src/sanitizer/config.rs`) — re-affirmed, still
provisional, not newly locked:** measured rate against the module's own
35-item benign fixture set is **0/35 (0.0%)**
(`sanitizer::config::tests::benign_fixture_false_strip_rate_is_below_the_provisional_ceiling`,
verified passing this session), well under the current 0.02 ceiling. This
session's own new benign corpus cases (below) include one deliberate
near-miss (`d25_benign_tool_textblob_support_ticket.json`: "please
disregard the previous email" *does* match the sanitizer's
`instruction_override_disregard` pattern — a genuine, intentional
false-strip case, not an error) and one deliberate true-negative
(`d21_benign_stock_price_trigger_adjacent.json`: "Ignore minor after-hours
fluctuations" contains "Ignore" but does not match any of the 5 compound
regexes). **Not locked as final** because the directive's own gating
condition — a real ~100-case benign corpus (§13.3) — isn't met; this
session's corpus has 7 benign cases total. Left provisional, honestly,
rather than declared final on 7 cases.

**Corpus composition — exact, verified counts (`grep`-counted across all
three corpus directories, not estimated):**

| Stratum | Count | Directory |
|---|---|---|
| Tier 1 (WebContent) | 17 | 12 attack (5 pilot T1a `ref01`–`ref05` + 1 pilot cat-5 `ref09` + 6 new `c1x`) + 5 benign (1 pilot `ref10` + 4 new `d21`/`d22`/`d23`/`d26`) |
| Tier 2 (ToolOutput) | 9 | 7 attack (3 pilot T1b `ref06`–`ref08` + 4 new `c2x`) + 2 new benign (`d24`/`d25`) |
| Tier 3 (AgentDojo) | 3 | all attack, all new this session |
| **Total** | **29** | 10 `tests/pilot_corpus/` + 16 `tests/corpus/` + 3 `tests/agentdojo_corpus/` |
| Attack / Benign split | 22 / 7 | `grep -c '"corpus": "Attack"'` / `"Benign"` across all three directories |

This is **far short of directive §13.3's ~360-case target** (Tier 1: 120,
Tier 2: 80, Tier 3: 60, Benign: 100). Grown from the pre-session 10 to 29 —
roughly triple, not the full target. Stated exactly, not rounded up. The
16 newly-authored cases (`tests/corpus/`) cover carrier vectors and attack
categories the original 5 Tier-1/3 Tier-2 pilot cases didn't exercise
(`OffscreenText`, `MetaContent`, `CssPseudo`, `ToolMetadata`, a
`WithinFingerprintOriginShift` and a `WithinFingerprintDataOnly` case) —
see `docs/TO-DO.md`'s new corpus-remainder row for the full stratum-by-
stratum shortfall against §13.3.

**Double-authoring / Cohen's κ — honest statement, not a fabricated
number:** **zero cases in this corpus are genuinely independently
double-authored.** All 29 are `Author::SelfAuthored` (26) or
`Author::AgentDojo` (3, itself a citation-derived provenance, not a second
human author). This session was a single agent working alone — there was
no second, independent author available to produce the 10% double-authored
slice §13.3 requires, and fabricating a "second pass" by the same session
pretending to be an independent author would produce a Cohen's κ computed
from self-agreement, which is not what the metric means and would misrepresent
the corpus's credibility exactly as ADR-008 warns against. **κ is not
computed here** — computing one from 0 genuine pairs (or from a fake pair)
would be a more dishonest data point than stating plainly that this
requirement is unmet. Filed as part of the corpus-remainder TO-DO row.

**Tests, exact counts:**
- `ferrite-ipi`: dataset.rs's `#[cfg(test)] mod tests` — 11 tests total, 4
  new this session: `carrier_vector_carrier_matches_the_variant_it_was_constructed_with`,
  `carrier_vector_serializes_as_an_externally_tagged_pair`,
  `every_carrier_vector_variant_reports_the_correct_carrier`,
  `dataset_store_reopen_against_an_existing_db_preserves_rows`. Full crate:
  `cargo test -p ferrite-ipi` unaffected elsewhere, still green.
- `ferrite-eval`: `cargo test -p ferrite-eval` — 61 lib tests (`adjudication`
  36, `agentdojo` 3 new, `corpus` 11, `harness` 7, `harness::e2e_tests` 4) +
  4 integration test binaries: `agentdojo_corpus_validate.rs` (3, new),
  `corpus_validate.rs` (4, new), `pilot_corpus_validate.rs` (3, unmodified,
  still green against the reshaped files), `pilot_w6.rs` (5, mechanically
  fixed this session) = **76 tests total, 0 failed.**

**Verified:** `cargo fmt -p ferrite-ipi -p ferrite-eval --check`,
`cargo clippy -p ferrite-ipi -p ferrite-eval --all-targets -- -D warnings`
(both clean), `cargo test -p ferrite-ipi` and `cargo test -p ferrite-eval`
both green as itemized above. Full workspace `just check && just test`
run at session end — see this entry's closing verification note below.

**Commits:** `6e6926a` (T-006 type-level fix + T-009 AgentDojo adapter +
T-208 re-validation, one commit — the type change and its call-site fixes
across both crates are not independently green without each other),
`2bc1925` (16 new corpus cases + validation test), `d1531c0` (corpus
authoring guide fixes, T-201).

**Known issues discovered, filed as new TO-DO rows (see `docs/TO-DO.md`):**
- **T-225** — FSR is only a case-level proxy in `ExecutionRecord`, not the
  literal segment-level ratio §13.2 defines. See audit above.
- **T-226** — no token-usage field anywhere in `ExecutionRecord`/`Timing`;
  ΔT (§13.2) cannot be computed from persisted data. Exact proposed field
  named above.
- **T-227** — corpus-size remainder against §13.3: 29 of ~360 cases, 0 of
  the required 10% double-authored, κ not computable. Full breakdown in
  this entry.

## 2026-09-19 — A12 (Harness + metrics) — outcome lattice fixed, consent policy swept, metrics module built, `just eval` produces a real report

**Landed:**
- **T-215 (small fix, done first as instructed):** `ferrite-eval/src/harness.rs::run_one` and `ferrite-ui/src/lib.rs`'s dry-run construction both called `orch.set_detect_enabled(...)` alone, leaving `strip_enabled` permanently `false` regardless of `DefenseMode` — `On` and `LoopOnly` were behaviorally identical in both the eval harness and the live UI's own dry-run path even though A5 wired live excision in production. Both now call `orch.set_defense_mode(mode)`, which derives both flags together. Closes T-003/D3 for real.
- **T-004/D4 (outcome lattice):** confirmed the bug live before fixing it — `adjudicate`'s `On`/`LoopOnly` arm checked only `consent_gated`, so a case the sanitizer had already stripped (nothing left to gate on) reported `Executed`. `FinalOutcome` now has the directive's exact four attack-side variants (`Stripped`/`ContainedViaConsent`/`Executed`/`NotAttempted`; `Blocked` deleted, its two meanings split correctly). `NotAttempted` triggers when a record's `tool_events` is empty. `adjudication::attack_final_outcome` is a pure, directly-testable decision function with the required table-driven test over every `(sanitizer_caught, consent_gated)` combination per mode.
- **T-010/D10 (consent-policy sweep):** `ConsentPolicy` is now `RejectFlagged`/`ApproveAll`/`RandomP{p, seed}`, and `adjudicate` actually reads the policy it's handed (previously silently ignored via `_consent_policy`). `RandomP` draws deterministically from `(seed, case_id)` via a seeded `StdRng`. Added `reconsent()` — a post-hoc recompute of `consent_gated`/`final_outcome`/`residual_risk` from an already-adjudicated `ExecutionRecord` under a different policy, so the metrics/report layer never re-runs the dry run to sweep policies.
- **Metrics module** (`crates/ferrite-eval/src/metrics.rs`, new): every §13.2 formula — Wilson score 95% CI (exact formula, hand-checked against the standard n=10/k=5 worked example), ASR/CR/ADR/SDR, a benign false-flag-rate function that serves as both FGR and the FSR case-level proxy, residual R, paired latency overhead — plus paired McNemar's exact test (hand-checked against a textbook b=1/c=9 example), Holm-Bonferroni correction across the full mode-pair family, and Cohen's h. Every formula has a hand-computed-value test, not just "runs without panicking."
- **Report generator** (`crates/ferrite-eval/src/report.rs`, new): markdown + CSV over a completed run — corpus composition, per-mode metrics (with ADR/SDR explicitly `n/a` for modes that never run that layer, not a misleading `0.0%`), per-tier ASR stratification, mode-pair comparison table, consent-policy sweep table, and a sample of real audit-chain anchors. Flags any cell under 5 cases as insufficient data.
- **`WorstCaseAgent`** (`crates/ferrite-eval/src/worst_case_agent.rs`, new): a deterministic `AgentRuntime` that generalizes the hand-written `ScriptedAgent` pattern the existing tests already used per-fixture — derives a case's dry-run action sequence from its own authored `ground_truth`, programmatically, across the whole real corpus. Module docs state the methodology and its limits plainly (does not model whether a real model would comply with an injection; measures whether the loop catches a *realized* deviation, which is Ferrite's actual architectural claim).
- **`just eval`** (`crates/ferrite-eval/examples/eval.rs` + justfile recipe): replaces the "not yet implemented, exit 1" stub with a real runner — loads all three corpus directories, runs every case across every mode via `harness::run_case`, generates the report. **Verified locally: 29 cases, 96 executions, `audit.log.verify_chain() = true`, zero live network calls** (no `FERRITE_GEMINI_API_KEY` set; `WorstCaseAgent` never calls a model at all).
- **T-202 (per-case inspector)**: `crates/ferrite-eval/examples/inspect_case.rs` — runs one case file across its modes, prints a legible per-mode table plus a per-mode sanity check against `ground_truth`, and (added after finding T-228) dumps the raw `computed_diff` whenever the loop ran.
- **`docs/EVALUATION.md`** (new): objectives (O1-O5) each scored against this run's real numbers; every §13.2 metric with real Wilson CIs (pooled and per-tier); the §13.3 sizing derivation reconciled honestly against the real n=29; a §13.4 parameter-rationale table that verifies each row against the actual implementation rather than copying the directive forward uncritically (found and stated real, pre-existing drift: the fingerprint layer runs Gemini-direct-HTTP or rules-only, not through `ferrite_model`/Ollama at all — T-224); a 7-item limitations section.
- **T-225/T-226 resolved as documented limitations, not schema additions** — both proposed fixes require touching files explicitly outside this charter's scope (`sanitizer`/`dry_run` for T-225's segment-level FSR; `fingerprint/` + `ferrite-agent` for T-226's token threading), and an unpopulated field would itself violate the no-dead-code invariant. Reasoning in `docs/EVALUATION.md` §5 and `docs/TO-DO.md`.
- **T-228 filed (new, found and root-caused this session):** `WithinFingerprintOriginShift`'s adjudication check only credits a catch via `diff.out_of_scope_origins`; a degenerate (correctly empty, fail-to-empty) fingerprint routes the same deviation into `diff.extra_primitives` instead, where the check never looks. Root-caused via `inspect_case`'s diff dump against `ref09_cat5_within_fingerprint.json`, not left as a guess. Containment itself is not broken (the case is still correctly `Gated`); only this category's specific per-layer attribution claim doesn't land as designed. A fix belongs in `adjudication.rs` (in scope, not attempted — a real semantics decision, not a one-line patch).

**Commits:** `31d66a1` (T-215), `ec7ece9` (T-004 + T-010), `49ead4d` (metrics + report modules), `abc047c` (`WorstCaseAgent` + `eval.rs`), `3e5b649` (`inspect_case.rs`, T-202), `dfe0a6c` (justfile `eval` recipe), `f5bf68f` (ADR/SDR `n/a` fix), `401d133` (per-tier ASR stratification), `795977a` (`docs/EVALUATION.md`), `1a16cc2` (diff dump + TO-DO/EVALUATION reconciliation).

**Tests:** `ferrite-eval` — 110 lib tests + 3+4+3+5 across 4 integration binaries = **125 tests, 0 failed** (up from A11's 76; net new: `metrics` 22, `report` 3, `worst_case_agent` 6, `adjudication` grew from 27 to 45 with the T-004 table-driven test, T-010's policy-sweep tests, and `reconsent`'s tests). `ferrite-ipi` — 170 tests, unchanged count from before this session (only `dataset.rs`'s `FinalOutcome` enum changed, no test count change there). Key names proving the headline claims: `adjudication::tests::t004_table_driven_attack_final_outcome_every_combination`, `adjudication::tests::t010_random_p_is_deterministic_given_a_fixed_seed`, `metrics::tests::wilson_interval_matches_the_n10_k5_worked_example`, `metrics::tests::mcnemar_matches_the_b1_c9_worked_example`, `metrics::tests::holm_bonferroni_matches_hand_computed_adjustment`, `report::tests::generate_report_flags_insufficient_data_for_a_tiny_stratum`.

**Verified:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo machete`, `cargo test --workspace` (every crate green, counts unchanged elsewhere), `cargo deny check` (advisories/bans/licenses/sources all `ok`) — all run from repo root at session end, all clean. `just eval`'s equivalent (`cargo run -p ferrite-eval --example eval`) run twice locally, byte-identical case/execution counts both times (29/96), audit chain verified both times.

**Known issues discovered, filed as new TO-DO rows:**
- **T-228** — `WithinFingerprintOriginShift` adjudication gap, root-caused, not fixed. Full trace in this entry and `docs/EVALUATION.md` §5.
- **T-224 confirmed still live, not caused by this session** — the eval harness's fingerprint layer runs Gemini-direct-HTTP or rules-only, never through `ferrite_model`/Ollama, exactly as T-224 already described for the live UI path. This session's `docs/EVALUATION.md` §4 is the first place this is stated against the eval harness specifically (T-224 was filed against `ferrite-ui` only).
- **UP (utility preservation vs `Off`) is not computable at all** under the current run matrix — `Benign` has no `Off` cell (ADR-007's own design). Not a defect this session introduced or could fix within scope; stated in `docs/EVALUATION.md` §1/O3.

## 2026-09-19 — A13 (Reconciliation & release) — final doc-vs-test/SHA audit, honest limitations consolidated, `docs/TO-DO.md` regenerated, no tag

**Session note:** same hazard A4–A12 hit — the worktree's own branch
(`worktree-agent-afdaa81020145c0dc`) started on a completely unrelated
tree (`65b6d67`, "Initial test case designing with Authoring guide
document", `46b2177`, `86d7d3c` "gemini key for agent" — pre-rebuild
commits with no `docs/`/`crates/` shape matching this rebuild at all).
Confirmed via `git log --oneline -5` per the launch brief's explicit
warning, then recreated `rebuild/a13-reconciliation-release` from local
`main` (`5d4738e`, carrying A0–A12 including the merge commit named in
the launch brief) before any work — nothing lost, the stale worktree
branch was untouched garbage, not in-progress work.

**Landed:**
- **Doc-claim audit (item 1 of the charter).** Spot-checked every ADR in
  `docs/DECISIONS.md`, sampled `PROGRESS.md` headline claims across
  A0/A2/A3/A4/A6/A7/A8/A9/A11/A12, and every `CLAUDE.md` invariant, against
  real evidence: `git log -1 --format="%H %s" <sha>` for 13 cited SHAs
  spanning nine different agents' commits (`e474403`, `a45b2ad`, `6cb4c70`,
  `6250cb3`, `ec7ece9`, `795977a`, `d9e2c12`, `aaae747`, `6e6926a`,
  `b1545d3`, `5bd0dc1`, `3062977`, `f582dda`) — all matched their cited
  description exactly, no fabricated or misattributed SHA found. Verified
  `docs/ARCHITECTURE.md` still genuinely doesn't exist (`CLAUDE.md`'s claim
  still true). Verified the `rusqlite`-is-one-version invariant directly
  (`grep -c '^rusqlite' Cargo.toml` = 1; `Cargo.lock` has exactly one
  `rusqlite` package entry; no `sea-query`/`sea-orm`/`sqlx`/`diesel`
  anywhere). Verified `ferrite-ipi`'s `Cargo.toml` still names
  `ferrite-agent` as a real dependency (T-221 still live, not stale).
- **Found and fixed a real drift in `crates/ferrite-servo/src/shell.rs`
  and `scripts/check_purge.sh`** (commit `a39f58c`): a doc-comment still
  said "the block path used by the capability broker" — `check_purge.sh`
  had excluded this one file since A0 specifically because the crate
  wasn't `cargo fmt`-clean yet at the time; T-207 (A1) made it clean
  months ago, so the blocker the exclusion cited no longer exists. Reworded
  the comment (mechanism description, no dead-architecture name) and
  removed the now-stale exclusion — `cargo fmt -p ferrite-servo --check`,
  `cargo clippy -p ferrite-servo --all-targets -- -D warnings`, and
  `cargo test -p ferrite-servo` (1 passed) all verified clean before
  committing.
- **`docs/TO-DO.md` regenerated (item 2), not renumbered, nothing dropped**
  (commit `0de760c`): added a top summary (**39 done, 1 in-progress, 11
  open, 1 held, 1 dropped, 2 needing owner confirmation** — 55 rows total,
  re-tallied directly against every row, not carried forward). **Found and
  fixed one real staleness bug while auditing:** T-215's row still said
  `in-progress`, even though A12's own PROGRESS entry and the T-003 row
  both already state it closed by `31d66a1` — confirmed by reading that
  commit's actual diff (`harness.rs::run_one` and `ferrite-ui/src/lib.rs`
  both call `set_defense_mode`), not just trusting the cross-reference.
  T-113 (this charter) marked `done`. **T-204 untouched**, per the standing
  "not now" instruction — its row was not read for editing purposes beyond
  confirming it still says `held`.
- **`README.md` rewritten** (commit `4787847`): the old file listed 7
  crates and said "mid-rebuild"; the real root `Cargo.toml` has 11 members
  (`ferrite-core`, `ferrite-model`, `ferrite-audit-log`, `ferrite-ipi`,
  `ferrite-engine`, `ferrite-engine-servo`, `ferrite-agent`,
  `ferrite-servo`, `ferrite-ui`, `ferrite-shell`, `ferrite-eval`) — the new
  README lists all 11 with one-line purposes, explicitly explains the two
  Servo-related crates and the two `ferrite-agent` code paths (a source of
  real confusion otherwise), and states the rebuild-complete status without
  overclaiming: T-224 (live-app gap), T-227 (corpus shortfall), and T-220
  (`ServoEngine` navigation gap, with the "live app unaffected" caveat) are
  named in the status section itself, not buried only in `TO-DO.md`.
- **`CLAUDE.md` correction pass** (commit `e6603eb`): the
  dependency-direction invariant claimed a clean downward graph; T-221
  (found by A9, still open) is a real, live exception —
  `ferrite-ipi/Cargo.toml` really does depend on `ferrite-agent`. Reworded
  the invariant to name the exception explicitly rather than silently
  contradicting it. Confirmed no status/planned-work section has crept
  back in (full re-read; only the one invariant line changed). No crate-list
  drift to fix — `CLAUDE.md` deliberately never hardcodes one, pointing
  at `Cargo.toml` instead, which is why this file didn't need the same
  crate-list fix `README.md` did.
- **`docs/EVALUATION.md` §6/§7 added** (commit `d10fdb2`), the graded
  deliverable: §6 is the consolidated limitations account A12's §5
  explicitly deferred here — irreducible blind spot (R = 1/22 = 4.5%,
  re-verified via `grep -l 'WithinFingerprintDataOnly'` matching exactly
  one corpus file), pattern ceiling (5+5 patterns re-counted directly
  against `crates/ferrite-ipi/src/sanitizer/patterns.rs`, not assumed from
  the directive's table), consent-policy upper bound, single-machine eval,
  corpus-size shortfall (29/360, per-stratum percentages re-derived from a
  fresh `grep`/`ls` count, not copied from T-227's prose), the
  worst-case-agent methodology's real scope, the live-app gap (T-224,
  stated as material, not a footnote), the `ServoEngine` navigation gap
  (T-220, with the confirmed-live-elsewhere addendum), and brief mentions
  of T-221/T-222/T-223/T-228. §7 scores the directive's own §14
  definition-of-done checklist item by item against evidence gathered this
  session: **8 of 11 fully met, 1 partially met (Servo build succeeds,
  real navigation doesn't), 2 not met (corpus size/κ)** — stated plainly,
  not rounded up. `scripts/check_purge.sh` gained a `docs/EVALUATION.md`
  exclusion (same reasoning as its existing exclusions for the other
  rebuild-record docs — the new §7 checklist names dead-architecture
  strings to confirm their absence) and a comment fix (the exclusion list
  comment was missing `docs/TO-DO.md`/`docs/handoffs/**`, which the actual
  command already excluded — code and comment now agree).
- **AI-attribution history check (item 6 of the charter), run fresh, not
  assumed:** `git log --all --oneline | grep -iE "claude|anthropic|ai.assist"`
  and `git log --all --format="%H %B" | grep -iE "claude|anthropic"` across
  the full reachable history (119 commits on `main`, plus whatever other
  refs exist locally) both return exactly two commits — `f582dda`
  ("docs(claude): update Build/test/run to the justfile...") and `01c3f54`
  ("docs: rewrite CLAUDE.md and README.md...") — and in both cases the
  matched text is discussing **the file named `CLAUDE.md`**, not AI
  attribution. No commit-msg trailer, byline, "Generated with", or 🤖
  anywhere in the matched output. **No breach found.** `.rules`/
  `commands.md` confirmed absent from the whole tree (`find . -name
  ".rules" -o -name "commands.md"`, excluding `.git/` — no matches); the
  three dead broker/policy/sandbox crates confirmed absent from
  `Cargo.toml` `members` and from disk; the `9222` port forward confirmed
  absent from `.devcontainer/devcontainer.json` (only appears inside the
  rebuild's own record docs, describing what was removed).
- **`grep -rn "allow(dead_code)" crates/` returns zero matches
  workspace-wide** — nothing to check the "has a `T-###` comment" condition
  against; the crate-root `#![deny(dead_code)]` gates in `ferrite-core`/
  `ferrite-model` make the allow-and-annotate pattern structurally
  unnecessary rather than merely unused today.
- **`just check && just test` timed for real, from this freshly-recreated
  branch:** wall clock **228 seconds (3m48s)**, exit code 0, every crate's
  `test result: ok`, 0 failures anywhere in the run (spot-checked via
  `grep -E "test result:|FAILED|error\["` over the captured log — only
  `ok` lines and zero `FAILED`/real `error[` matches). Under the 5-minute
  budget with real headroom. `cargo deny check` also run fresh:
  `advisories ok, bans ok, licenses ok, sources ok`, the same pre-existing
  Servo-git-source `unmatched-source` warning every prior agent since A3
  has logged (not a new finding).

**Commits:** `a39f58c` (dead-architecture doc-comment reword + stale
`check_purge.sh` exclusion removed) → `e6603eb` (`CLAUDE.md` T-221
correction) → `4787847` (`README.md` rewrite) → `d10fdb2`
(`docs/EVALUATION.md` §6/§7 + `check_purge.sh` exclusion/comment fix) →
`0de760c` (`docs/TO-DO.md` regeneration, T-215 fix, T-113 closed).

**Tests:** `cargo test -p ferrite-servo` (1 passed, the only Rust source
touched this session); full-workspace `cargo test` via `just test` (every
crate green, 0 failed, exact counts unchanged from A12's numbers since no
production code changed beyond the one comment). `cargo fmt -p
ferrite-servo --check`, `cargo clippy -p ferrite-servo --all-targets -- -D
warnings`, `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo machete`, `sh scripts/check_purge.sh`,
`sh scripts/check_no_archive_links.sh` all clean at this entry's HEAD.

**Known issues discovered, not fixed (correctly left to a future
session — this charter is documentation/verification, not new features):**
none new beyond the T-215 status-staleness bug (fixed in `docs/TO-DO.md`
itself, not a code defect) and the stale `check_purge.sh` exclusion (also
fixed). Every T-2xx item this entry's `docs/EVALUATION.md` §6 discusses
was already filed by an earlier agent; this session found no previously
unknown defect in the product itself, which is the expected shape of a
reconciliation-only charter, not evidence the audit was shallow — see the
spot-check list above for what was actually re-verified against source
rather than re-read from a prior doc.

**Explicitly not done, per the charter's own instruction:** no `v0.1.0`
git tag was created (coordinator's call, after reviewing this branch's
merge — same principle as every prior agent not merging its own branch).
T-204 was not touched. No Rust feature work was attempted beyond the one
comment reword needed to make a doc-accuracy claim (`check_purge.sh`'s
exclusion reasoning) true again.

## 2026-09-19 — B1 (`ferrite-ipi` vocabulary migration) — dry-run rebuilt on `ferrite_engine::BrowserEngine`, old fingerprint predictor deleted, T-221/T-216 closed

First charter of the new post-A13 sequence (B1–B4), opened because the user
asked, after using the live app, for zero old pre-rebuild code left
anywhere in the workspace. **Known hazard, confirmed again:** the worktree
started on an unrelated tree (`65b6d67`/`46b2177`/`86d7d3c`, no
`docs/`/`crates/` shape matching current `main`). Recreated
`rebuild/b01-ipi-vocabulary-migration` from local `main` (`7979a04`, the
A13 merge) before any work — the same hazard every agent since A4 has hit.

**Core architectural decision.** `ferrite-ipi::dry_run` was rebuilt to
implement `ferrite_engine::BrowserEngine` directly (option B in the
charter's own framing) rather than adopting
`ferrite_agent::browser_loop::run_agent_loop` (option A). Option A was
rejected for a structural reason, not a preference: `run_agent_loop` lives
in `ferrite-agent`, so depending on it from `ferrite-ipi` — even only that
one function — is still a `ferrite-ipi → ferrite-agent` edge, exactly the
backwards direction T-221 names. Closing T-221 that way would require
relocating `browser_loop` out of `ferrite-agent` into a crate both `ipi`
and `agent` can depend on, real cross-crate surgery outside this charter's
file list. Full reasoning, including the real trade-off being accepted (a
seam remains between dry-run's own small orchestration loop and
`browser_loop`'s), is in `crates/ferrite-ipi/src/dry_run/engine.rs`'s
module docs.

**Landed:**
- `crates/ferrite-ipi/src/dry_run/engine.rs` (new): `DryRunEngine`
  implements every `ferrite_engine::BrowserEngine` method — synchronous,
  `&mut self`, no `Arc<Mutex<_>>` (the pre-B1 `RecordingExecutor` needed
  that only because `ferrite_agent::ToolExecutor::execute` was an async
  `&self` method). Detect/strip sanitizer gating and the page-read
  HTML-channel scan are ported verbatim from the deleted `executor.rs`,
  including the real, tested "both `detect_injection_in_value` and
  `sanitize_html` run and both contribute findings for `ReadPage`" double
  coverage. Containment-by-construction (T-007/D7) re-proved with the same
  grep-backed test, ported.
- `crates/ferrite-ipi/src/dry_run/record.rs`: `ToolEvent.tool: ToolId` →
  `ToolEvent.primitive: ferrite_core::Primitive` — real, direct, no bridge.
  `ToolEvent::primitive()` (the old fallible bridge) is deleted.
- `crates/ferrite-ipi/src/dry_run/orchestrator.rs`: `DryRunOrchestrator::run`
  is now generic over a new `DryRunDriver` trait (`drive(&mut DryRunEngine)`)
  instead of `ferrite_agent::AgentRuntime`. Whole-turn timeout / partial
  record on timeout (A6) preserved.
- `crates/ferrite-ipi/src/comparator/mod.rs`: `resolve_primitive` (T-216's
  patch table) deleted; `compare` reads `event.primitive` directly and
  converts to `ToolId` only at the point of inserting into
  `FingerprintDiff` (whose `HashSet<ToolId>`/`Attribution.tool: ToolId`
  shape is unchanged, for `ferrite-eval`/`ferrite-ui` compatibility).
  `comparator::legacy` (`lower_fingerprint`) deleted along with the
  `ToolFingerprint` type it existed only to convert.
- `crates/ferrite-ipi/src/tool_decision/mod.rs`: deleted
  `LlmMayUsePredictor` (raw `reqwest` calls to hardcoded, retired
  `gemini-2.0-flash`), the old stringly `rule_based_must_use`,
  `ToolFingerprint`, and `generate_fingerprint`/`fingerprint_from_task`'s
  old bodies. Replacement `generate_fingerprint` takes an explicit
  `&dyn ferrite_model::ModelProvider` and calls
  `crate::fingerprint::generate_fingerprint` (A4's real predictor)
  directly, returning `crate::fingerprint::Fingerprint`. `prepare_task`
  takes a new `crate::IpiTask` (mirrors `ferrite_agent::AgentTask`'s shape)
  instead of `&ferrite_agent::AgentTask` — the last real coupling point,
  `ToolId: From<&BrowserTool>`, moved to `ferrite-ui` (its only caller).
  `DefenseMode`/`LoopOutcome`/`prepare_task`'s sanitizer logic (A5/A12,
  load-bearing) untouched.
- `crates/ferrite-ipi/Cargo.toml`: `ferrite-agent` dependency line deleted
  (T-221's literal fix), `ferrite-engine` added, unused `reqwest` removed.
- `crates/ferrite-agent/src/engine_bridge.rs` (new, purely additive):
  `EngineToolExecutor<'a, E: BrowserEngine + Send>` bridges the pre-existing
  `BrowserTool`/`AgentRuntime`/`ToolExecutor` vocabulary onto any
  `BrowserEngine`, so `ferrite-eval`'s `WorstCaseAgent` and the live
  `GeminiAgent` can drive `DryRunEngine` without either side changing its
  own vocabulary. No existing `ferrite-agent` export touched.
- `crates/ferrite-eval/src/harness.rs`: `AgentRuntimeDriver` (pub) wraps
  `EngineToolExecutor` to implement `DryRunDriver` for any `AgentRuntime`;
  `run_one` builds an `IpiTask` alongside its existing `AgentTask`, calls
  the new `generate_fingerprint` with `ferrite_model::MockProvider::new()`
  (reproduces the old "no API key configured" rules-only behavior exactly,
  R7-compliant), and reproduces `from_legacy_tool_fingerprint`'s exact
  per-case uniform-scope policy via `ExpectedCapability` +
  `ExpectedFingerprint::from_capabilities` against the new `Fingerprint`
  type (deliberately not `from_fingerprint`, which would lose a case's
  authored `domain_suffix`/custom `task_open` scope). Mechanical
  `ToolEvent{tool: ToolId::new("dom.read")}` → `{primitive: Primitive::DomRead}`
  fixes at 4 sites (`metrics.rs`, `report.rs`, `adjudication.rs` ×2);
  `RecordedFinding`'s own `tool: ToolId` field untouched (not
  security-relevant, no charter reason to retype it).
- `crates/ferrite-ui/src/lib.rs`: same shape of fix — `tool_id_of()` local
  helper, `DryRunAgentDriver`, `IpiTask` built alongside `AgentTask`,
  `ExpectedFingerprint::from_fingerprint` (this call site's own
  scope-derivation logic already matched `from_fingerprint`'s policy
  field-for-field, so this is a lossless swap, not an approximation).
  **Flagged regression:** the live app's `may_use` layer is now
  unconditionally rules-only (`MockProvider::new()`, nothing scripted) —
  filed as T-229, squarely inside T-224's existing scope (the live app
  never used `ferrite-model` for this to begin with).

**Verification, not just "it compiles":** `cargo run -p ferrite-eval
--example eval` (no API key set) reports **29 cases, 96 executions** —
identical to `docs/EVALUATION.md`'s committed numbers — and every per-mode
metric in the generated `target/eval-report/EVAL_REPORT.md` (Off ASR
100%, LoopOnly CR 94.7%, On CR 100%, etc.) matches `docs/EVALUATION.md`
§2.1 exactly, confirming the `ExpectedCapability`/`from_capabilities`
substitution in `harness.rs` is truly behavior-preserving, not merely
type-correct.

**Tests:** `cargo test -p ferrite-ipi` — **157 passed, 0 failed** (up from
153 pre-charter: new tests include
`dry_run::record::tests::tool_event_primitive_is_a_real_direct_value_download_included`,
`dry_run::engine::containment_tests::this_modules_source_names_no_network_capable_dependency`,
`dry_run::orchestrator::tests::*` ported to `DryRunDriver`). `cargo test -p
ferrite-agent` — 15 passed (4 new, `engine_bridge::tests::*`). `cargo test
-p ferrite-eval` — 125 passed across the lib + 4 test binaries (0
failed, 0 regressed). `cargo test -p ferrite-ui` — 17 passed. `cargo test
-p ferrite-model` — 154 passed (sanity check, unaffected). `cargo build
--workspace` — succeeds. `cargo fmt --all --check` and `cargo clippy
--workspace --all-targets -- -D warnings` — both clean. `cargo deny check`
— clean (same pre-existing Servo-git-source `unmatched-source` warning
every agent since A3 has logged).

**Commits:** `e7ddd91` (ferrite-ipi dry-run rebuild) → `29b7840`
(ferrite-agent engine_bridge) → `75321e4` (ferrite-eval compile fix) →
`cd5d090` (ferrite-ui compile fix).

**Known issues discovered, not fixed:** T-229 (live app's `may_use` layer
now unconditionally rules-only until T-224 wires in a real
`ModelProvider`); T-230 (`content_key_for_call`'s corpus-scripting key
table doesn't yet cover `BrowserEngine`-only actions `BrowserTool` never
had — `scroll`, `wait_for`, `tab.*`, `cookie.read`, `storage.read`,
`query` always serve the generic stub). Neither is a regression in
containment/security behavior — both are scoped, honestly filed gaps in
peripheral capability (model-assisted prediction quality; corpus-scripting
coverage of newer primitives).

**Explicitly not done, per the charter's own instruction:** did not touch
`ferrite-eval`'s corpus/adjudication/worst_case_agent logic beyond the
minimal mechanical fixes above; did not touch `ferrite-ui`/`ferrite-shell`
beyond the minimal mechanical fixes above; did not delete
`ferrite_agent::{BrowserTool, AgentRuntime, GeminiAgent, ToolExecutor}`
(still live, still load-bearing for `ferrite-ui`/`ferrite-eval` until
B2/B3 migrate off them); did not attempt T-204; did not merge to `main`.

## 2026-09-19 — B2 (`ferrite-eval` migration off `BrowserTool`) — `WorstCaseAgent` rebuilt on `BrowserEngine` directly, corpus vocabulary unified with `Primitive`, real live-provider `just eval` run verified twice, T-221 fully closed

**Session note:** the subagent session that did the bulk of this work was
interrupted mid-edit by a platform rate limit (`docs/EVALUATION.md`'s §2.6
was being written when it cut off). No commits had landed — all changes
were still uncommitted working-tree state. The coordinating session
verified everything already in the tree independently (did not trust the
interrupted session's own claims), finished the doc, and completed the
remaining process steps (this entry, `docs/TO-DO.md`, the B3 handoff)
directly, per this project's established practice for a mid-session
interruption (mirroring A3's).

**`WorstCaseAgent`'s shape decision:** rebuilt to drive
`ferrite_engine::BrowserEngine` directly (`&mut dyn BrowserEngine`, real
`Call`s tagged with real `Primitive`s from `ground_truth`) instead of
constructing `ferrite_agent::BrowserTool`/`AgentToolCall`s for B1's
`EngineToolExecutor` bridge to interpret — option (A) from the charter,
the full migration, not the smaller bridged option (B). `ferrite-eval` no
longer depends on `ferrite_agent::{BrowserTool, AgentRuntime, ToolExecutor}`
at all for its own corpus-running logic — **confirmed by grep: every
remaining `ferrite_agent`/`BrowserTool` string in the crate is now inside a
doc comment or historical note explaining the pre-B2 shape, not a real
`use` or type reference.** The scripted-agent methodology itself
(deterministic re-enactment of authored `ground_truth`, not a model
deciding whether to comply) is unchanged — only its plumbing moved.

**Corpus vocabulary decision:** `corpus.rs`'s `KNOWN_TOOL_IDS` (a
hand-maintained list of `BrowserTool::tool_id()` strings) is now derived
directly from `ferrite_core::Primitive::ALL.iter().map(Primitive::as_str)`
— closing off the exact kind of drift T-216 found, structurally, rather
than by discipline. Only one of the 29 corpus files actually used a
`by_tool` scripting key (`ref06_t1b_jsonfield_exfil.json`:
`"download.file"` → `"download"`); every other file's `ground_truth`
semantics are byte-for-byte unchanged. `dry_run::engine::content_key_for_call`
(B1) was updated in the same pass to emit `Primitive::as_str()` values
too, so both sides of the (still structurally separate, still
non-security-relevant) corpus-scripting lookup speak one vocabulary.
T-230 (the newer `BrowserEngine`-only actions — `scroll`, `wait_for`,
`tab.*`, `cookie.read`, `storage.read`, `query` — still have no scripting
coverage) is **not** closed by this — no corpus case exercises them yet,
so extending the table would be speculative; left open, honestly, for
whoever authors a case that needs it.

**Real live-provider `just eval` run — attempted, succeeded, verified
twice.** `harness::try_real_provider()` constructs a real
`ferrite_model::OllamaProvider` from `ModelConfig::from_env()` when
`FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN` are set, resolving
`OLLAMA_API_KEY` from the OS keyring (service `"ferrite"` — the same key
`just probe` verified live during A3). Run with `FERRITE_MODEL_SMALL=
FERRITE_MODEL_MAIN=gemma4:31b`, ~30 minutes apart: **byte-identical
results both times** (temperature 0, seed 42 holding for real, not just
in unit tests). Only the fingerprint-dependent metrics moved at all
(`LoopOnly` ASR 1/19→3/19; `On`'s benign FGR 3/7→1/7), both in the
expected direction (a real model admits a correctly-filtered but broader
plausible-capability set than five keyword groups); every sanitizer-only
or audit-only number (`Off`, `SanitizerOnly`, `On`'s ASR/CR/ADR) is
byte-identical to the rules-only run, exactly as it should be. Full
numbers and reasoning in `docs/EVALUATION.md` §2.6 (new). The default,
credentials-free `just eval` run (§2.1–§2.5's headline numbers) was
independently re-verified unchanged: 29 cases, 96 executions, every
per-mode metric matching the pre-B2 committed numbers exactly.

**`docs/EVALUATION.md` also corrected, not just extended:** §4's
"Model backend" parameter-rationale row described `ferrite_ipi::
tool_decision::LlmMayUsePredictor` making a direct `reqwest` call to
Gemini — that type no longer exists as of B1. Rewritten to describe what
actually runs now (a real `ModelProvider` via `fingerprint::
generate_fingerprint`, verified live in §2.6). §7's checklist item on
"every model call routes through cache/throttle/budget" updated the same
way — the live path is now demonstrated, not merely trivially true
because no live path existed. §1's O5 (cost) note updated to cite §2.6's
real ΔL (p50=699ms, p95=1527ms) instead of predicting a hypothetical
number for a "future" live run.

**Tests:** `cargo test -p ferrite-eval` — **111 lib + 15 across 4
integration binaries (corpus_validate, agentdojo_corpus_validate,
pilot_corpus_validate, pilot_w6) = 126 passed, 0 failed** (up from 125
pre-charter). `cargo test -p ferrite-ipi` — 157 passed, 0 failed
(unchanged from B1 — `content.rs`/`engine.rs`'s vocabulary-unification
edit is a rename, not a behavior change). A dedicated R7 test,
`harness::tests::no_automated_test_calls_try_real_provider`, greps this
crate's own `src/` for any call site of `try_real_provider()` outside
`examples/eval.rs` and fails the build if one exists — the automated test
suite provably never makes a live call regardless of what `just eval`
itself can do when a human runs it deliberately.

**Full workspace, independently re-verified by the coordinator (not
carried forward from the interrupted session's own report):** `cargo
build --workspace` succeeds; `cargo fmt --all --check` and `cargo clippy
--workspace --all-targets -- -D warnings` both clean; `cargo test
--workspace` — 30 `test result: ok` blocks, 0 failures; `cargo deny check`
clean (same pre-existing Servo-git-source warning every agent since A3
has logged).

**Commits:** `85261b3` (ferrite-eval/ferrite-ipi vocabulary migration +
live-provider wiring), plus a following docs commit (EVALUATION.md §2.6 +
§4/§7/§1 corrections, this entry, TO-DO.md, handoff) — see `git log` for
its SHA.

**Known issues discovered, not fixed:** T-230 remains open (see above —
genuinely nothing to extend it against yet). No new `T-2xx` filed this
session — B1's T-229/T-230 are the only open items this charter touched,
and T-229 is unaffected by this charter (it's specifically about the live
`ferrite-ui` app, B3's charter).

**Explicitly not done, per the charter's own instruction:** did not touch
`ferrite-ui`/`ferrite-shell` (B3's charter, T-224/T-229/T-220); did not
attempt T-204; did not merge to `main`.

## 2026-09-19 — B3 (`ferrite-ui`/`ferrite-shell` live wiring) — real ModelProvider + browser_loop-driven live agent execution, T-224/T-229/T-220 closed, old GeminiAgent/BrowserTool vocabulary deleted workspace-wide

**Known hazard, confirmed again:** the worktree started on an unrelated tree
(`65b6d67`/`46b2177`/`3c08dde`, an "Initial test case designing" branch with
no relationship to this project's real history). Recreated
`rebuild/b03-ui-live-wiring` from local `main` (`d08916b`, B2's merge)
before any work — the same hazard every agent since A4 has hit.

**Core architectural decision — giving the live session a `BrowserEngine`
face.** Investigated whether `ferrite_engine_servo::ServoEngine` (owns and
constructs its own `HeadlessServoSession` per tab) could be adapted, or
whether a sibling type was needed. Decision: added
`BorrowedServoEngine<'a>`, wrapping an **externally-owned**
`&'a mut HeadlessServoSession` instead of constructing one —
`ferrite-ui` already owns and correctly drives one session per tab via its
own `ServoFrame` subscription (a real winit event loop tick, the exact
ingredient T-220's diagnosis found missing from `ServoEngine`'s own
standalone conformance-test environment). Every `BrowserEngine` method's
DOM/JS/navigation logic was extracted into a session-first `ops` module
(`crates/ferrite-engine-servo/src/lib.rs`) so `ServoEngine` and
`BorrowedServoEngine` share one implementation — no script or logic
duplicated between them, per the charter's own explicit ask. Multi-tab:
`ferrite-ui` keeps its own tab bookkeeping (`Vec<String>`/`tab_urls`/etc.)
rather than switching to `ServoEngine`'s `TabId` model — `BorrowedServoEngine`
wraps exactly one, externally-managed tab, and its `open_tab`/`close_tab`/
`switch_tab` return `EngineError::Unsupported` (never reachable in practice:
`ferrite_agent::browser_loop::AgentAction` has no tab-management variants at
all). This is a smaller, lower-risk choice than migrating `ferrite-ui`'s
entire tab model, and it is what let `ClickElement`/`FillForm`/
`ExtractData`/`WriteClipboard` upgrade from literal no-op stubs to real DOM
actions **for free** (they were already implemented and tested in
`ServoEngine`'s own JS-injection logic, T-109 — this charter only gave that
logic a second way to reach a real session).

**Does this actually fix T-220 for the live app? Evidence, not just
architecture.** `BorrowedServoEngine::navigate`'s `drive_until_loaded` calls
`session.spin()` in a loop — the identical `pump_engine()`/`sync_and_read()`
calls the live app's own `ServoFrame` subscription already makes every 16ms
to browse real pages successfully (confirmed live by the user in the A9/T-220
session note: "confirmed live 2026-09-18 ... render a real page end-to-end").
Because the session is the *same* one, in the *same* process, with the
*same* real winit event loop already proven to drive it — not a second,
freshly-constructed session in a standalone test binary with no winit
`EventLoop` object at all (T-220's actual root cause) — there is no reason
for `BorrowedServoEngine`'s navigation to fail differently than the tab UI's
own navigation already succeeds. This reasoning is sound but the literal
click-through (submit an agent task that navigates, watch it complete) was
**not observed interactively this session** — see "Honest remainder" below
for exactly why and what was checked instead. `ServoEngine`'s own
**owned**-session path (used only by the `#[ignore]`d
`servo_conformance.rs` test) was not touched and would very likely still
exhibit T-220's original symptom if ever run for real — nothing in
production calls `ServoEngine::new` anymore, so this residual defect is now
conformance-test-only.

**T-229 (real `ModelProvider` at startup).** `ferrite-ui`'s `launch()` now
calls a local `try_real_model_provider()` (mirrors
`ferrite-eval::harness::try_real_provider()` exactly: `ModelConfig::
from_env()`, Ollama first via the OS keyring service `"ferrite"`, Gemini
fallback) and overwrites `FerriteBrowser::model_provider`/`model_tag_small`/
`model_tag_main` with the result *before* the window opens. Deliberately
**not** called from `FerriteBrowser::default()` — that stays hardcoded to
`MockProvider::new()` so the entire test suite (dozens of call sites
construct a `FerriteBrowser::default()`) never touches the environment or
OS keyring, matching `ferrite-eval::try_real_provider()`'s own established
R7 convention exactly (that function is likewise never reachable from a
`#[test]`, only from `examples/eval.rs`). A new grep-based test,
`no_automated_test_calls_try_real_model_provider_outside_launch`, proves
exactly one real (non-comment) call site exists in the file. `AgentTaskSubmitted`'s
handler now passes this real provider into `ToolDecisionEngine::
fingerprint_from_task` in place of the hardcoded `MockProvider::new()` B1
left behind — closes T-229 as T-224's side effect, exactly as B1's handoff
anticipated.

**T-224 (the live app on the new stack) — landed in three commits:**

1. `555ad99` — `ferrite-engine-servo`: `ops` module extraction +
   `BorrowedServoEngine<'a>` (T-220's fix, above). `ServoEngine`'s own
   public behavior is unchanged, now implemented on top of the same `ops`
   functions.
2. `3addb46` — `ferrite-ui`: real `ModelProvider` wiring (T-229, above);
   the dry run's decision loop now calls
   `ferrite_agent::browser_loop::run_agent_loop` **directly** against
   `ferrite_ipi::dry_run::DryRunEngine` (`BrowserLoopDryRunDriver`),
   replacing the `GeminiAgent`/`EngineToolExecutor`-bridged
   `DryRunAgentDriver` B1 built — the "cleaner path" B1's own handoff
   anticipated once nothing needed the bridge for the dry run's own sake.
   This required one real, well-justified upstream change:
   `run_agent_loop`'s signature moved from `engine: &mut dyn BrowserEngine`
   to a generic `<E: BrowserEngine>` `engine: &mut E`, because a trait
   object erases its concrete type's `Send`-ness even when the concrete
   type (`DryRunEngine`) genuinely is `Send` — calling the old signature
   from inside an `async_trait`-boxed `DryRunDriver::drive` produced a
   provably-`!Send` future regardless of what was actually passed in.
   Verified: the resulting `run_agent_loop::<DryRunEngine>` future is
   `Send` (compiles inside `tokio::spawn`); `run_agent_loop::<ServoEngine>`
   (or any real `!Send` engine) correctly stays un-spawnable, exactly
   reflecting that engine's real constraint — see `browser_loop.rs`'s own
   updated doc comment.

   The **live** (real) run cannot use this same direct-call shape:
   `BorrowedServoEngine` wraps a real, `!Send` `HeadlessServoSession`, so
   nothing built on it can cross a `tokio::spawn` boundary, which
   `run_agent_loop`'s own async, multi-step design would require. Built
   instead: a message-driven step loop (`start_live_loop`/
   `spawn_next_step`/the `LiveRunReady`/`AgentStepReady` message handlers)
   — only the per-step model round trip (`&dyn ModelProvider`, `Send +
   Sync`) is ever spawned in the background; each returned `AgentAction`
   executes synchronously, on the Iced update thread, via
   `ferrite_agent::browser_loop::execute_action` (made `pub` for exactly
   this reuse) against a `BorrowedServoEngine` over the active tab's
   session. Step/wall-clock budgets and repeated-action detection mirror
   `run_agent_loop`'s own logic exactly (same `LoopBudget`/`LoopStopReason`
   types, reused not reimplemented). A `run_id` generation counter, bumped
   on every fresh run and on `StopAgent`, is stamped onto every
   `LiveRunReady`/`AgentStepReady` message and checked before acting, so a
   step already in flight when the user stops the agent (or starts a new
   task) can never execute a live browser action after that point — tested
   (`a_stale_run_id_is_ignored_by_live_run_ready`,
   `a_stale_run_id_is_ignored_by_agent_step_ready`).

   Consent enforcement was re-implemented on the `AgentAction`/
   `ferrite_core::Primitive` vocabulary (`is_action_rejected`,
   `action_tool_id`, `action_url`) — the same semantics the old
   `FilteredToolExecutor` had (block a rejected tool id; block a rejected
   origin only for actions that carry their own URL, `Navigate`/`Download`
   — the same honest, pre-existing limitation for actions with no URL of
   their own). The old `ToolRequest`/`BrowserToolExecutor`/
   `FilteredToolExecutor`/`tool_tx`/`tool_rx` channel bridge and its
   subscription are deleted outright — no longer needed now that live
   actions execute directly inside `update()`.

3. `c696d0b` — `ferrite-agent`: deleted `BrowserTool`, `AgentTask`,
   `AgentToolCall`, `AgentToolResult`, `AgentTurn`, `AgentError`, the
   `AgentRuntime`/`ToolExecutor` traits, `GeminiAgent`/`gemini.rs`,
   `engine_bridge.rs`/`EngineToolExecutor`, and `RateLimiter` (used only by
   `GeminiAgent`). **Grep-confirmed before deleting** (per the charter's own
   explicit instruction): `ferrite-ui` and `ferrite-shell`'s `agent-smoke`
   were the only two remaining real (non-comment) callers anywhere in the
   11-crate workspace — both already migrated in commits 1–2 and this same
   commit respectively. `ferrite-agent/src/lib.rs` is now nine lines: a
   module doc explaining the history, plus `pub mod browser_loop;`.
   `ferrite-shell`'s `agent-smoke` subcommand now demonstrates the new
   stack end to end: a real `ModelProvider` (same `try_real_model_provider`
   pattern, a third, consistent copy per this project's own established
   convention of not sharing this ~15-line pattern across crates) driving
   `run_agent_loop` against `ferrite_engine::MockEngine`. Removed now-unused
   dependencies: `reqwest`/`uuid`/`thiserror`/`async-trait` from
   `ferrite-agent`, `async-trait` from `ferrite-shell`.

**Full-workspace grep, after deletion, confirms zero real references
remain** to `ferrite_agent::{BrowserTool, AgentRuntime, GeminiAgent,
ToolExecutor, AgentTask, AgentToolCall, AgentToolResult, AgentTurn,
AgentError, RateLimiter}` or `engine_bridge::EngineToolExecutor` anywhere in
the workspace — every remaining string match is inside a doc comment
explaining pre-B3 history (the same pattern B1/B2 already left in
`ferrite-ipi`/`ferrite-eval`'s own module docs).

**Tests:** `cargo test -p ferrite-ui` — 29 passed, 0 failed (up from 17
pre-charter: 12 new, including
`a_rejected_tool_id_blocks_the_action_before_it_reaches_the_engine`,
`a_rejected_origin_blocks_a_navigate_before_it_reaches_the_engine`,
`agent_step_ready_with_finish_completes_the_task`,
`agent_step_ready_with_a_model_error_fails_the_task`,
`repeated_identical_actions_stop_the_live_loop_before_a_third_execution`,
`step_budget_exhausted_stops_the_loop_instead_of_spawning_another_step`,
`stop_agent_bumps_run_id_and_clears_the_live_loop`,
`a_stale_run_id_is_ignored_by_live_run_ready`/`..._by_agent_step_ready`,
`no_automated_test_calls_try_real_model_provider_outside_launch`; 3 old
`FilteredToolExecutor` tests replaced 1:1 by their `AgentAction`-vocabulary
equivalents). `cargo test -p ferrite-engine-servo` — 6 passed (2 new:
`without_the_engine_servo_feature_borrowed_engine_reports_unsupported_actions_not_panics`
plus the 4 pre-existing `unwrap_js_string_result`/`js_string_literal`
tests). `cargo test -p ferrite-agent` — 7 passed (down from 15: the 8
deleted-vocabulary tests are gone with the code they tested;
`browser_loop`'s own 7 tests, including the `<E: BrowserEngine>` generic
change, all still pass unchanged). `cargo test --workspace` — every test
binary green, 0 failures (full per-crate counts in this entry's commit
messages). `cargo build --workspace`, `cargo fmt --all --check`, `cargo
clippy --workspace --all-targets -- -D warnings`, `cargo deny check` — all
clean (`cargo deny`'s only output is the same pre-existing Servo-git-source
`unmatched-source` warning every agent since A3 has logged).

**App launch, confirmed observed this session — both build configurations:**

- `cargo run -p ferrite-shell -- ui` (default features, no real Servo)
  starts, prints `[ferrite-ui] no ModelProvider configured/reachable
  (FERRITE_MODEL_SMALL/FERRITE_MODEL_MAIN unset, or no Ollama/Gemini
  credential found) — ...` (this sandbox's shell has no
  `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN` exported, confirmed via `env |
  grep FERRITE_MODEL` — empty) and `[ferrite-ui] Servo unavailable:
  ferrite-servo compiled without the 'servo' feature` (expected — default
  build), then enters the real winit event loop and keeps running (`timeout
  20 ... ; echo "EXIT: $?"` → `EXIT: 124`, i.e. killed by the timeout while
  still alive, not a crash — no panic, no non-zero-from-the-process exit).
- `cargo build -p ferrite-shell --features ferrite-servo/servo` — attempted
  and **succeeded**: `Finished \`dev\` profile ... in 26m 06s` (real Servo,
  pulling in `libservo`/`script`/`layout`/`webrender`/`net`/etc. from
  `github.com/servo/servo` tag `v0.0.5`, plus `ferrite-servo`,
  `ferrite-engine-servo`, `ferrite-ui`, `ferrite-shell` all compiling clean
  against it — comparable to A9's own `just build-servo` precedent,
  ~15m31s/6.4GB; this run took longer because a second, unrelated `cargo
  test -p ferrite-ui` was deliberately run concurrently in an isolated
  `CARGO_TARGET_DIR` to avoid blocking on the main build's lock, competing
  for the same CPU).
- `cargo run -p ferrite-shell --features ferrite-servo/servo -- ui` —
  **launched successfully with the real Servo engine**: no "Servo
  unavailable" message this time (meaning `HeadlessServoSession::new(1280,
  700)` — the real, `libservo`-backed constructor — succeeded), only the
  expected "no ModelProvider configured" line (no model env vars in this
  shell), then stayed running past a 25s timeout with no crash or panic
  (`EXIT: 124`).
- **Re-run with `FERRITE_MODEL_SMALL=gemma3:27b FERRITE_MODEL_MAIN=
  gemma3:27b` set** (the `"ferrite"`-service OS-keyring credential from an
  earlier session, confirmed present via `security find-generic-password -s
  ferrite`) **and** the real Servo feature: the app printed **no warnings
  at all** — neither the "no ModelProvider" line nor "Servo unavailable" —
  and stayed running past a 15s timeout with no crash. This is real,
  observed evidence that `try_real_model_provider()` successfully resolved
  a real Ollama provider from the OS keyring (§10.1's exact key-resolution
  path) *and* the real Servo session constructed, together, in the same
  live process — the strongest verification this session obtained short of
  an actual interactive click-through.

**Honest remainder — what was not verified, and exactly why:**
- **The full pipeline was not exercised interactively via a real click.**
  This sandbox has no attached display for a real winit window to be
  clicked through, and this agent cannot drive a GUI. What *was* checked:
  every state-machine transition the click would trigger is unit-tested
  directly against `update()` (the `AgentStepReady`/`LiveRunReady`/
  `ConsentSubmitted`/`StopAgent` tests above); the app is confirmed to
  launch and stay alive rather than crash, with the real Servo engine; and
  a real `ModelProvider` is confirmed constructible from this exact
  sandbox's stored credential (above) — but nothing exercised an actual
  `provider.complete(...)` network round trip, nor watched an agent task
  actually navigate a real page live. A human with a display (and, ideally,
  the same env vars set for the session, matching B2's own
  `docs/EVALUATION.md` §2.6 run, `gemma4:31b` on both tiers there) is the
  natural next step to close that specific gap.

**Known issues discovered, not fixed:** none new beyond what's already
tracked. T-230 (dry-run content-scripting coverage of newer `BrowserEngine`
actions) untouched, as before. `ServoEngine`'s own owned-session standalone
defect (T-220's literal original bug) is unchanged — see that row.

**Explicitly not done, per the charter's own instruction:** did not attempt
T-204; did not merge to `main`.

## 2026-09-23 — C1 (Design system / icon set) — hand-authored SVG icons, chrome visual polish, tab bar hover, consent-panel entrance transition

**Known hazard, confirmed again:** the worktree started on an unrelated tree
(`65b6d67`/`46b2177`/`3c08dde`, an "Initial test case designing" branch).
Recreated `rebuild/c01-design-system` from local `main` (`a8099b6`, B3's
merge) before any work — working tree was clean, so no work was lost.

**Scope:** first charter in the C1-C4 post-B3 UI/feature sequence. Purely
visual/UX — no security-relevant decision, per the charter itself. File
scope: `crates/ferrite-ui/` plus the one `iced_widget` feature-flag line in
root `Cargo.toml`.

**1. Real SVG icon set.** `crates/ferrite-ui/assets/icons/` — 14
hand-authored, original `.svg` files (back, forward, reload, close, add,
stop, play, approve, reject, warning, audit, console, agent, origin), one
consistent 24x24 viewBox, outlined/stroke-based (a few filled shapes —
stop/play — for their media-player convention). Not copied from any named
icon pack. `iced_widget`'s `svg` feature was enabled (`Cargo.toml`, root):
confirmed against the pinned `iced_widget` 0.13.4's own `Cargo.toml`
(`svg = ["iced_renderer/svg"]`) and `iced_core::svg::Style` (`color:
Option<Color>`, doc'd "useful for coloring a symbolic icon") that recoloring
at render time is real, supported API on this exact pinned version — not
assumed. `iced` itself needs no matching feature: `ferrite-ui` imports
`iced_widget::svg` directly, the same pattern the file already used for
`iced_widget::image` (the Servo frame).

**2. `icon()` rendering helper.** `crates/ferrite-ui/src/icons.rs`: an
`Icon` enum, a pure `icon_bytes(kind) -> &'static [u8]` mapping (bytes
embedded via `include_bytes!` — no filesystem lookup at runtime, so `cargo
run`'s CWD never matters), and `icon(kind, size, color) -> Element` — the
one path every view call site in this crate uses, so sizing/color/hover
tint go through one place. `svg::Handle::from_memory`'s cache identity is a
content hash (confirmed against `iced_core::svg::Handle::from_data`), so
calling `icon()` fresh every `view()` tick is cheap — same tint+bytes hits
the renderer's existing cache. Two icons (`bookmark`, `settings`) were
drawn for C3's known future needs but **not** added to the `Icon` enum:
rustc's `dead_code` lint flags an unconstructed variant on the crate's real
*lib*-target compilation regardless of test-only construction (verified —
adding them and a test that constructs both still failed `cargo clippy
--all-targets -- -D warnings`), and `CLAUDE.md`'s no-dead-code invariant
rules out `#[allow(dead_code)]` as the fix. Filed nowhere since it's not a
defect — `icons.rs`'s own comment documents the reasoning at the point a
future charter would look for it.

**3. Applied to the chrome**, every existing message/click target
preserved: Back/Forward/Reload/Stop-loading buttons, the tab close button,
the new-tab "+" (now a real `button` sharing `nav_btn_style`, not a bare
`mouse_area`), the Audit/JS/Agent toggle buttons (dropped their leading
"v"/"+" text glyph — the active/inactive background already carried that
state), the agent sidebar's Run Task/Stop buttons, and the consent panel's
header (warning icon), per-item Approve/Reject buttons, and — the one
`Icon::Origin` call site — a small external-link glyph on out-of-scope-
origin rows specifically (`origin_item_origin(&item.id).is_some()`),
visually distinguishing "contacted an unauthorized origin" from "used an
unexpected tool" beyond the text difference alone.

**4. Tab bar polish** (the coordinator's own stated dev-tool-looking
concern): close-button-on-hover — visible on the active tab or whichever
tab the pointer is over (`hovered_tab: Option<usize>`, new
`TabHoverEnter(usize)`/`TabHoverExit(usize)` messages via `mouse_area`'s
`on_enter`/`on_exit`), a same-size transparent spacer otherwise so the row
never jumps width; a hover background tint on inactive tabs; `hovered_tab`
reset on `CloseTab` (a close shifts every later tab's index — a stale
hovered index would show the close button on the wrong tab until the next
real hover event). `AddTab`/`CloseTab`/`SelectTab` messages and their
`update()` handling are byte-for-byte unchanged.

**5. Consent panel entrance transition.** New `consent_panel_anim: f32`
field (`0.0..=1.0`) and `ConsentPanelTick` message, advanced 16ms at a time
(`CONSENT_ANIM_STEP = 16.0/200.0`, same tick shape `ServoFrame` already
establishes) by a `subscription()` tick gated on `pending_diff.is_some() &&
consent_panel_anim < 1.0` — so it only runs while actually animating, not
for the rest of the app's lifetime. Reset to `0.0` on `ConsentRequired`
(`pending_diff` transitioning `None` -> `Some`). A new pure helper,
`ease_out_cubic(t)`, turns the linear tick progress into the panel's
background-alpha fade (`0.0 -> 0.08`) and a shrinking top-padding slide
(`16px -> 0px`), over ~200ms. **Every item's own text is at full opacity
and its final position from the very first frame** — only the outer
wrapper's background tint and top inset animate — so the transition never
delays, dims, or obscures what the user is being asked to approve; this
was checked deliberately, not merely asserted, per the charter's own
explicit constraint on this exact panel.
`page_content_cannot_reach_the_consent_panels_inputs` (unchanged, still
passing) is unaffected — the transition's inputs are `consent_panel_anim`
(a `f32` this crate's own tick advances) and `ease_out_cubic` (a pure
`f32 -> f32` function), neither of which touches page content in any way.

**6. Not changed, verified by reading the diff before committing:** no
`FerriteBrowserMessage` variant's meaning, no `update()` logic for any
*pre-existing* message, `consent_items`/`consent_is_complete`/
`FingerprintDiff` handling, the B3 agent-execution wiring, dry-run/audit/
twin logic, anything in `ferrite-ipi`/`ferrite-agent`/`ferrite-engine`/
`ferrite-model`/`ferrite-servo`.

**Tests:** `cargo test -p ferrite-ui` — 35 passed, 0 failed (up from 29
pre-charter: `icons::tests::every_icon_variant_embeds_non_empty_well_formed_svg`,
`icons::tests::every_icon_shares_the_same_viewbox`,
`tests::ease_out_cubic_starts_at_zero_and_ends_at_one`,
`tests::ease_out_cubic_clamps_out_of_range_input`,
`tests::ease_out_cubic_is_monotonically_non_decreasing`,
`tests::ease_out_cubic_is_ahead_of_linear_partway_through_an_ease_out_curve`
new; every one of the 29 pre-existing tests, including
`page_content_cannot_reach_the_consent_panels_inputs` and
`no_automated_test_calls_try_real_model_provider_outside_launch`, passes
unmodified — no test's assertion text needed to change, since no button
label string it checks was among the ones replaced by icons). `cargo fmt -p
ferrite-ui --check`, `cargo clippy -p ferrite-ui --all-targets -- -D
warnings`, `just check` (fmt + clippy --workspace --all-targets + `cargo
machete`), `just test` (full workspace, every crate's test binary green),
`cargo build --workspace` — all clean.

**Found, not silently fixed — filed as T-231 (docs/TO-DO.md):** enabling
`iced_widget`'s `svg` feature makes `cargo deny check` fail
(`[bans] multiple-versions = "deny"`): `resvg`/`usvg` 0.42.0 (pinned by
`iced_renderer` 0.13.0's own `svg` feature) pin `fontdb` 0.18.0/`kurbo`
0.11.3, one version newer than `iced_tiny_skia`/`cosmic-text`'s existing
0.16.2/0.10.4. Same structural class `deny.toml`'s own `[bans] skip`
comment already documents for two earlier batches. The fix (two more
`skip` entries) lives in root `deny.toml`, outside this charter's stated
file scope — deliberately left unfixed and filed rather than silently
expanding scope. `just check`/`just test` are unaffected (`just check`
does not run `cargo deny`, by that recipe's own design).

**Also found, fixed as an unrelated one-line dependency cleanup:**
`ferrite-ui`'s `Cargo.toml` had an unused direct `ferrite-engine` dependency
(pre-existing since B3 — `cargo machete`, run for the first time as part of
getting `just check` green for this charter, caught it). Removed; confirmed
nothing in `ferrite-ui`'s source names `ferrite_engine::` outside a doc
comment.

**App launch, confirmed observed this session:** `cargo run -p ferrite-shell
-- ui` (default features, no real Servo) — builds, starts, prints the same
two expected warnings B3's session did (`no ModelProvider configured`,
`Servo unavailable: ferrite-servo compiled without the 'servo' feature`),
enters the real winit event loop and stays running (`timeout 20 ...; echo
EXIT: $?` -> `EXIT: 124`, killed by the timeout while still alive, not a
crash). **Not attempted this session:** the real-Servo feature build
(`--features ferrite-servo/servo`, 15-26 minutes per A9/B3's own
precedent) — B3 already verified that combination works pre-C1, and
nothing in this charter touches Servo-session or engine code, only view
code around it, so re-running that specific, expensive build was judged
unnecessary rather than skipped for time. **This agent cannot visually
verify the aesthetic result** — no attached display, cannot drive a GUI.
Every claim above about what changed is a description of the diff (file
list, icon list, which widgets changed), not an assertion about how it
looks; the coordinator's own visual review with the user is still the next
real checkpoint for that.

**Commits:** `61ed9f0` (svg feature + unused-dep cleanup), `3981f8a` (icon
set + visual polish).

**Known issues discovered, not fixed:** T-231 (above). Everything else
already tracked (T-212/T-217/T-218/T-219/T-222/T-223/T-227/T-228/T-230)
untouched, as before.

## 2026-09-24 — coordinator — live agent loop: unbounded context growth and ambiguous action JSON fixed

**Scope:** ad hoc bug fix, not a lettered charter — two observed failures
in the live agent loop (`ferrite-agent::browser_loop::run_agent_loop` and
`ferrite-ui`'s message-driven mirror of it, `AgentStepReady`/
`spawn_next_step`, wired by B3): longer tasks eventually hit the model's
context-length limit, and the model would sometimes emit a malformed
action — most visibly for `type_text`/`fill_form` — instead of valid JSON.
File scope: `crates/ferrite-agent/src/browser_loop.rs`,
`crates/ferrite-ui/src/lib.rs`.

**1. Root cause, context growth.** `run_agent_loop` pushed every action/
observation pair onto `messages` with no bound, and `messages.clone()` is
sent to the model on every step — a long-running task's conversation grows
without limit. `ferrite-ui`'s live loop has its own copy of this same
per-step message-building code (see B3's entry above, `docs/handoffs/b03.md`,
for why it can't just call `run_agent_loop` directly: `BorrowedServoEngine`
wraps a `!Send` `HeadlessServoSession`), so it had the identical bug,
independently.

**2. Root cause, malformed actions.** `SYSTEM_PROMPT`'s action vocabulary
was described in a terse shorthand (`type_text{selector,text}`) rather than
literal JSON. Models sometimes reproduced that shorthand verbatim — e.g.
putting the parameters inside the `"action"` string itself
(`{"action":"type_text{selector,text}"}`) — which fails
`serde_json::from_str::<AgentAction>` and stops the loop with
`LoopStopReason::MalformedAction`, most reliably on the two actions
(`type_text`, `fill_form`) whose fields don't fit the shorthand cleanly.

**3. Fix, once, not per-crate.** Added `pub fn compact_observation` (caps
one observation at 8,000 chars, keeping head and tail) and `pub fn
trim_message_history` (caps total history at 120,000 chars, always keeping
`messages[0]` — the original task — and dropping the oldest action/
observation pair first) to `browser_loop.rs`, wired into `run_agent_loop`
itself. `ferrite-ui` now imports and calls those same two functions from
`AgentStepReady` instead of carrying its own copies — the same reuse
pattern already established there for `execute_action`/`SYSTEM_PROMPT`
(see B3's entry). `SYSTEM_PROMPT` itself only ever existed in
`browser_loop.rs`, so fixing the JSON-format issue needed no deduplication
— rewritten to one literal JSON example per action plus explicit
"never put parameters inside the `action` string" rules.

**4. Not changed:** `AgentAction`'s shape, `execute_action`'s dispatch,
`LoopBudget`/`LoopStopReason`, the IPI fingerprint/dry-run/consent gate, or
any pre-existing `FerriteBrowserMessage` handler's behavior — `messages`
sent to the model differ (compacted/trimmed) but `AgentLoopResult::observations`
(returned to the caller) is still the full, untruncated text, matching prior
behavior and `executed_actions_carry_real_origin_tracking_through_observations`.

**Commits:** `bce1521`.

**Tests:** `cargo test -p ferrite-agent` — 11 passed, 0 failed (up from 7;
new: `compact_observation_leaves_a_short_observation_unchanged`,
`compact_observation_truncates_a_long_observation_keeping_head_and_tail`,
`trim_message_history_keeps_the_original_task_and_drops_the_oldest_pairs_first`,
`trim_message_history_never_drops_below_the_task_plus_one_pair`; every
pre-existing test passes unmodified). `cargo test -p ferrite-ui` — 35
passed, 0 failed, unchanged from C1. `cargo fmt --all --check`, `cargo
clippy -p ferrite-agent -p ferrite-ui --all-targets -- -D warnings`,
`cargo build --workspace --no-default-features -p ferrite-agent -p
ferrite-ui` — all clean. `cargo machete` not run (not installed in this
container) — no `Cargo.toml` touched by this fix, so not a gap this change
could hide.

**Known issues discovered, not fixed:** `core.hooksPath` is unset in this
checkout, so `scripts/hooks/commit-msg` (the AI-attribution rejector
CLAUDE.md's own invariant names) was never actually enforced on this
branch's first commit — caught by inspection, fixed by amending that
commit, not filed as a T-### (it's a per-checkout git config step, not a
code defect; see `scripts/hooks/install.sh`).

## 2026-09-24 — coordinator — CI: manual-trigger-only, macOS-only (T-210 resolved by owner decision)

**Landed:** `.github/workflows/ci.yml` rewritten per an explicit project-
owner decision (T-210 had been sitting as "needs owner confirmation"
since A1 — see that row's prior text). Two changes, both applied to
every job:

1. **Trigger:** removed `push`, `pull_request`, and the weekly
   `schedule` cron entirely. Every job (`ci`, `build-servo-release`,
   `release`) is now `workflow_dispatch`-only — nothing runs
   automatically on a push or merge to `main`; every run, including the
   fast lint/test gate, is a manual "Run workflow" click.
2. **Platform:** dropped `ubuntu-latest`/`windows-latest` from both the
   `ci` and `build-servo-release` matrices — macOS only. The `ci` job's
   platform-conditional steps (fmt-check, `cargo machete`, `cargo deny`,
   the doc-drift scripts) used to run only under `if: matrix.platform ==
   'linux'` (one canonical platform, to avoid tripling non-platform-
   dependent checks); with Linux gone they now just run unconditionally
   on the one remaining platform, same checks, same behavior. `release`'s
   Windows artifact download/publish step and `binary_suffix` handling
   are removed — the rolling "latest" release now ships a macOS binary
   only. `Swatinem/rust-cache@v2`'s cache keys narrowed to `macos`/
   `servo-macos` (previously per-platform-in-matrix).

**Docs updated to match, not left stale:** `docs/TO-DO.md`'s T-210 row
marked `done — owner decided`, its stale "needs owner confirmation"
framing removed; the file's top summary counts re-tallied (44 done, was
43; 1 needing owner confirmation, was 2) and the "needs a decision only
the project owner can make" bullet split so it no longer bundles T-210 in
with the still-open T-209 (license choice). `docs/REBUILD_DIRECTIVE.md`
deliberately **not** edited — per that file's own header, it's the
original plan/record of what A1 built and why, not a place later
operational decisions get retroactively rewritten into; the actual
current behavior lives in `ci.yml` itself and this entry.

**Not verified live:** this sandbox cannot run GitHub Actions (same
limitation every prior agent touching `ci.yml` has hit, going back to
A0/A1). Verified locally instead: `python3 -c "import yaml;
yaml.safe_load(open('.github/workflows/ci.yml'))"` parses clean; every
step command in the rewritten file (`cargo fmt --all --check`, `cargo
clippy --workspace --all-targets -- -D warnings`, `cargo machete`,
`cargo test --workspace`, `cargo build --release -p ferrite-shell`,
`sh scripts/check_purge.sh`, `sh scripts/check_no_archive_links.sh`) is
identical to what the pre-existing Linux-gated steps already ran, just no
longer conditional — so this is a trigger/matrix change, not a new,
unverified command.

**Commits:** `3fd5f9c`.

**Known issues discovered:** none. T-209 (license choice) remains the
one open owner-decision item.

## 2026-09-24 — coordinator — C3a: Servo render-buffer/coordinate correctness fix (window resize, HiDPI blur, hover/click offset)

**Scope:** first item of the C-series post-C1 plan (`docs/handoffs/c01.md`'s
"what C2 reuses and does next" plus this session's own C2/C3 planning turn,
recorded in chat, not yet a written handoff file). Reported by the user
directly running the app: window opens medium-sized and centered rather
than filling the display; hovering/clicking a link lands on the wrong spot
("have to move my cursor above the link"); rendered pages look soft/blurry;
no favicons anywhere. Root-caused before writing any fix, not guessed —
see below.

**Root cause, confirmed by reading source, not assumed:**
`HeadlessServoSession::new(1280, 700)` creates a render buffer at a fixed
size once at startup. `crates/ferrite-ui/src/lib.rs` never called
`session.resize()` (that method existed in `ferrite-servo/src/session.rs`
with zero callers, grep-confirmed) — the buffer stayed 1280×700 forever.
The frame is displayed via `iced_widget::image` at `Length::Fill`, i.e.
stretched to whatever the real content area is, and raw `mouse_area`
logical-point positions were passed straight through to
`session.send_mouse_move`/etc. with no scale correction at all. The moment
the real window wasn't exactly 1280×700 physical pixels — any resize, the
agent sidebar's 320px, or a HiDPI/Retina display's own scale factor — the
visually-displayed frame and Servo's internal coordinate space diverged.
That mismatch is the hover/click offset bug and, separately (upscaling a
fixed low-res buffer to a bigger/differently-shaped area, with no
HiDPI-aware oversampling at all), the blur. `ContentAreaResized`/
`content_y_offset` (a pre-existing message/field pair that looked like it
should have been the fix) turned out to be dead code — defined and handled,
but never actually dispatched from anywhere, grep-confirmed
(`grep -n "ContentAreaResized {" crates/ferrite-ui/src/lib.rs` matched only
the enum definition and its match arm) — removed rather than wired up,
since the real fix (below) makes it unnecessary.

**Fix — three real changes, `crates/ferrite-servo`/`ferrite-ui`/root
`Cargo.toml`:**

1. **`HeadlessServoSession::size()`** (`ferrite-servo/src/session.rs`,
   both the real and stub impls) — a cheap `(width, height)` accessor (no
   frame readback) so a caller can check whether `resize()` is actually
   needed before calling it.
2. **The Servo buffer now tracks the real content-area size, every tick.**
   `FerriteBrowser` gained `content_area_size: Cell<Size>` (logical,
   written from `view()`'s `responsive`-wrapped content element — see
   below) and `scale_factor: f32` (physical px per logical point, fetched
   once via `iced::window::get_scale_factor` right after launch). Chose
   `iced_widget::responsive` over hand-replicating the chrome layout's
   heights/widths (tab bar + toolbar + progress bar + optional audit/JS
   panel + optional 320px agent sidebar) deliberately: `responsive`
   reports the container's true available size on every layout pass,
   correct by construction as that layout changes — including whatever
   C2's agent-panel redesign does to it next — rather than two copies of
   the same arithmetic that could silently drift out of sync. Needed
   adding the `iced_widget` `"lazy"` feature (`lazy = ["ouroboros"]` on
   the pinned 0.13.4) to the one root `Cargo.toml` line, same pattern C1
   used for `"svg"`. `ServoFrame`'s existing 16ms tick handler now compares
   `content_area_size × scale_factor` (rounded to physical pixels) against
   the active tab's `session.size()` and calls `session.resize()` only when
   they actually differ — reacting to a window resize, an agent-sidebar
   toggle, or an audit/JS-panel toggle alike, with no extra per-cause
   plumbing, since a tick already runs continuously regardless of cause.
3. **Every pointer-event coordinate now scales logical → physical before
   reaching Servo.** `ServoMouseMove`/`ServoMousePress`/`ServoMouseRelease`/
   `ServoScroll` all multiply `(x, y)` by `state.scale_factor` at the point
   they call `send_mouse_move`/`send_mouse_down`/`send_mouse_up`/
   `send_mouse_click`/`send_scroll` — `cursor_pos` itself stays logical
   (unchanged meaning, doc comment corrected). Scroll wheel *delta* is left
   unscaled deliberately (already OS-level units, independent of display
   scale) — only the event's *position* needed the fix.
4. **Launch now starts maximized, not a fixed 1280×800 centered window.**
   `launch()`'s startup `Task` chains `window::get_latest()` →
   `window::maximize(id, true)` + `window::get_scale_factor(id)` (mapped to
   the new `ScaleFactorReady` message) — the `.window_size(...).centered()`
   builder call is now only the frame shown for the instant before that
   task resolves, not the app's actual working size.

**Favicons and the visual-design pass (C3b/C3c) are not part of this
entry** — deliberately sequenced after this correctness fix, per this
session's own plan (restyling hover states on top of broken coordinates
would have been building on top of the bug, not fixing it).

**Verified:** `cargo build --workspace`, `cargo test --workspace` (every
crate green, 0 failures — full per-crate counts unchanged from before this
session's `ferrite-ui`/`ferrite-servo` edits: `ferrite-ui` still 35,
`ferrite-servo` still 1), `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo machete` (installed this
session, clean — no unused dependency from the new `lazy` feature) all
clean. **New-dependency duplicate-version risk checked directly against
`Cargo.lock`** (the exact class of problem T-231 found from C1's `"svg"`
feature): `git diff Cargo.lock` shows the `"lazy"` feature added exactly
five new packages (`ouroboros`, `ouroboros_macro`, `aliasable`, `yansi`,
`proc-macro2-diagnostics`), each with exactly one version in the lockfile —
none is a second copy of an already-duplicated crate the way C1's `svg`
pull of a newer `fontdb`/`kurbo` was. **`cargo-deny` itself was then
installed and run for real** (it wasn't present in this sandbox at first,
same gap A1 hit): `cargo deny check` → `advisories ok, bans ok, licenses
ok, sources ok`, only the one pre-existing `unmatched-source` warning for
the Servo git dependency every agent since A3 has logged (Servo isn't
built by default) — confirming the lockfile-diff read above rather than
leaving it as an inference.

**Not verified live — same limitation every prior UI-touching session has
stated plainly:** this sandbox has no attached display, so none of window
maximizing, HiDPI sharpness, or the hover/click coordinate fix could be
visually confirmed here. Every claim above is a description of the code
change (the coordinate-space mismatch that existed, and exactly how each
new line removes it), not an assertion about how it looks or feels — the
user's own relaunch is the real verification, same as C1's.

**Commits:** `46f9612`, `6a63754` (cargo-deny verification follow-up).

**Known issues discovered, not fixed:** none new. Existing T-223 (iced
0.13's `button` has no `Focusable` impl) is unrelated and untouched.

## 2026-09-24 — coordinator — C2: agent panel redesign — real per-step icon/label/detail/result feed

**Scope:** second item of this session's C-series plan, `crates/ferrite-ui`
only. Rebuilds `view_agent_sidebar`'s activity log per the original C2 goal
(`docs/handoffs/c01.md`'s "what C2 does next" plus this session's own
planning turn): "live, step-by-step action view... a scrollable action
history with per-step results (not just the flat `agent_tool_log: Vec
<String>` label list it renders today)". `AgentStepReady`/`LiveRunReady`/
run-id staleness checks and the consent panel itself are untouched, per
that same plan's own constraint.

**What changed, concretely:**

1. **`agent_tool_log: Vec<String>` → `agent_log: Vec<AgentLogEntry>`.**
   `AgentLogEntry` is `Note(String)` (dry-run-phase progress text, e.g.
   "dry run complete — checking for unexpected activity" — unchanged
   content, just a new variant instead of a bare string) or `Step { icon,
   label, detail, result, blocked }` — real structure the old flat log
   never carried: `label`/`detail` come from two new pure functions,
   `action_label`/`action_detail` (replacing `action_log_label`, which
   only ever produced one opaque `"[navigate] https://..."`-shaped
   string), and — the actual gap this closes — `result` is the real
   string `execute_action` returned (or the fixed "blocked by user
   consent" text when `is_action_rejected` fires first), not just the
   fact that some action ran. `blocked: bool` is threaded straight from
   `is_action_rejected`'s own return value (previously only used to
   choose *which* string to compute, then discarded) so the sidebar can
   style a blocked step differently.
2. **Icons per action type**, reusing C1's `icon()`/`Icon` machinery, not
   a second rendering path. `icon_for_action` groups `AgentAction`'s 19
   variants into 8 icons by the same action-class boundaries `ferrite_core
   ::Primitive`'s own taxonomy already uses (navigate/read/click/write/
   scroll+wait+screenshot/download/clipboard), rather than one icon per
   raw variant — deliberately: a JsExecute step reuses `Icon::Console`
   (already this crate's own "code" glyph, from the JS-console toggle
   button) and `Finish` reuses `Icon::Approve` (its outcome), both pinned
   by test as intentional reuse, not placeholders. 7 new icons drawn for
   the remaining groups (`navigate`, `read`, `click`, `write`, `activity`,
   `download`, `clipboard` — `crates/ferrite-ui/assets/icons/*.svg`), in
   the exact style C1 established (`viewBox="0 0 24 24" fill="none"
   stroke="#000000" stroke-width="2" stroke-linecap="round"
   stroke-linejoin="round"`, confirmed by reading three existing icons
   before drawing any new one) — `Icon`'s enum/`icon_bytes` grew from 14
   to 21 variants, `icons::tests`' well-formedness/shared-viewBox tests
   updated to the new count and passing.
3. **A live "Step N/max" indicator** in the sidebar header while
   `live_loop` is `Some` — reads `live.actions_taken.len()`/
   `live.budget.max_steps`, both fields that already existed on
   `LiveAgentLoop` for budget accounting; no new state needed.
4. **"Visible plan/reasoning" — honestly scoped, not silently dropped.**
   The original C2 goal names this explicitly. `AgentAction`'s wire
   format (what the model is asked to return, parsed in
   `ferrite_agent::browser_loop`) carries no reasoning/thought field at
   all today — adding one would mean changing that shared type's JSON
   schema (used by both the live loop and the dry run, in a different
   crate, `ferrite-agent`, outside this charter's `crates/ferrite-ui`
   file scope) and touching the `SYSTEM_PROMPT` this same session's
   earlier fix (`bce1521`) already changed once today for a different
   reason. Judged real, cross-crate, higher-risk work deserving its own
   deliberate pass, not a rider on a UI-only charter — not attempted here.
   What ships instead is the closest achievable thing within scope: each
   step's action *and* its real result together, which is genuine
   new visibility (what it did and what happened), just not the model's
   own free-text reasoning for choosing it.

**Tests:** `cargo test -p ferrite-ui` — 39 passed, 0 failed (up from 35; 4
new: `action_label_and_detail_are_distinct_per_action_kind`,
`icon_for_action_groups_by_action_class_not_one_icon_per_variant`,
`agent_step_ready_records_a_real_step_with_its_actual_result` (asserts
`state.agent_log`'s pushed `Step` carries the selector as `detail` and the
real "error: no active browser session" fallback as `result` — R7, no
Servo session exists in this test fixture — rather than a fabricated
success string), `agent_step_ready_records_a_blocked_step_as_blocked_not_a
_silent_success`; every pre-existing test passes unmodified, including
`icons::tests::every_icon_variant_embeds_non_empty_well_formed_svg`/
`..._shares_the_same_viewbox` against the 7 new icons).

**Verified:** `cargo build --workspace`, `cargo test --workspace` (every
crate green, 0 failures), `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo machete` all clean. No
`Cargo.toml` touched this session (all 7 new icons are `include_bytes!`
of new files, no new crate dependency), so no `cargo deny` re-run needed
beyond C3a's already-clean one earlier this session.

**Not verified live:** same standing limitation as every UI-touching
session — no attached display in this sandbox. The icon well-formedness
tests catch a malformed/empty SVG; they cannot confirm the 7 new glyphs
are actually recognizable at 12px in the sidebar, which is a real,
separate claim only the user's own relaunch can settle.

**Commits:** `8bb627f`.

**Known issues discovered, not fixed:** none new. The "visible reasoning"
scope decision above is a deliberate boundary, not a bug — flagged here
rather than filed as a T-### since it's a decision needing the user's own
input on whether it's wanted at all, not a defect with an agreed fix.

## 2026-09-24 — coordinator — two real bugs found live by the user: CI's cargo-deny action fails on macOS runners; C3a's resize fix caused a real segfault

**CORRECTION (see the later 2026-09-24 entry "segfault root cause was
misdiagnosed here — real fix in `HeadlessServoSession::resize()`"):**
bug #2's root-cause claim below — a same-tick `resize()`/`sync_and_read()`
race — is **wrong**. The user tested this fix on real hardware and
reported the identical crash and identical black-screen/stuck-rendering
symptoms afterward, disproving it. The CI fix (bug #1) is unaffected and
still correct. Left standing below rather than rewritten, per this
project's own R2 rule against silently erasing a disproven status claim —
read the correction entry alongside this one, not in place of it.

**Scope:** the user actually ran the app and the CI workflow after this
session's C3a/C2/CI-cadence changes merged to `main` (PR #2). Two real
regressions surfaced, both fixed here, on a fresh branch off `main` (the
merged PR's branch was deleted per the user's own workflow) —
`fix/servo-resize-crash-and-ci-cargo-deny`.

**1. CI: `cargo-deny-action` cannot run on the now-macOS-only runner.**
`EmbarkStudios/cargo-deny-action@v2` (`.github/workflows/ci.yml`'s
license/advisory/duplicate-version step) is a **Docker container action**
— GitHub Actions only supports container actions on Linux runners. This
session's earlier CI change (macOS-only, per owner decision, see T-210's
row) made the `ci` job's only runner `macos-latest`, so the step failed
outright with `Error: Container action is only supported on Linux` — not
caught before merging because this sandbox cannot run GitHub Actions
(same standing limitation every prior CI-touching agent has logged).
**Fixed:** replaced the container action with a native install + run —
`cargo install cargo-deny --locked` then `cargo deny check` — the exact
pattern the same job already uses for `cargo-machete` one step above it,
which works on any runner OS.

**2. A real, reproduced segfault in the live app — root-caused, not
patched around.** The user's own run: `cargo run -p ferrite-shell -- ui`
with `FERRITE_MODEL_SMALL=FERRITE_MODEL_MAIN=gemma4:31b` printed
`UNSUPPORTED (log once): POSSIBLE ISSUE: unit 1 GLD_TEXTURE_INDEX_2D is
unloadable and bound to sampler type (Float) - using zero texture because
texture unloadable`, then `[1]    53037 segmentation fault`. Screenshots
from before the crash also showed the real symptom this GL warning is
consistent with: page content rendering correctly only in a small
top-left region of the window, the rest of the content area solid black
— not a blank/loading state, a genuinely corrupted frame.

**Root cause, traced directly to C3a's own resize fix from earlier this
session, not a pre-existing bug:** `ServoFrame`'s tick handler called
`session.resize(desired_w, desired_h)` and then, in the **same** tick,
`session.sync_and_read()` — which calls `rendering_context.read_to_image
(rect)` using the buffer's *just-updated* `self.width`/`self.height`.
`HeadlessServoSession::resize()`'s own body (`ferrite-servo/src/
session.rs`) updates those fields and calls `rendering_context.resize()`/
`webview.resize()` — real calls into libservo — but nothing in this
codebase's control establishes that libservo's own compositor has
*finished* reallocating the backing surface by the time the very next
line executes. Reading a rect sized for the new dimensions against a
surface that may still be the old, smaller one is exactly what produces
both symptoms: a corrupted/partially-black frame (the intersection of old
and new dimensions is valid, the rest reads uninitialized/black) and,
under worse timing, a genuine out-of-bounds read — the segfault. This
session's earlier "found, not just guessed" standard applies here too:
the fix is not merely-plausible, it's derived directly from what the
crash log and the screenshots both independently point to, and both
symptoms disappear under the same one-line-of-reasoning fix (never read a
frame in the same tick a resize was issued).

**Fix, `crates/ferrite-ui/src/lib.rs` + `crates/ferrite-servo/src/
session.rs`:**
- Two new `FerriteBrowser` fields: `last_resized_content_px: (u32, u32)`
  (what the active session was last actually told to resize to — tracked
  here, not re-derived from the session itself, because the session's own
  size getter would reflect `resize()`'s immediate bookkeeping, not
  libservo's real completion) and `resize_settle_ticks: u8` (ticks
  remaining before it's safe to read a frame again).
- `ServoFrame`'s handler: a resize is only *attempted* when
  `resize_settle_ticks == 0` (never re-issued while a previous one is
  still settling) and only *actually issued* when the desired size
  differs from `last_resized_content_px`. The moment one is issued,
  `resize_settle_ticks` is armed to `RESIZE_SETTLE_TICKS` (3 ticks,
  ~48ms — a conservative, not empirically measured, margin: this sandbox
  cannot run the real `servo` feature to time libservo's actual
  reallocation, so 3 ticks is a deliberately generous guess for a
  CPU-side software-rasterizer reallocation, not a number pinned by
  observation. If the crash recurs, raising this constant — or replacing
  it with a real readiness signal from libservo if one exists — is the
  next move, not assumed unnecessary). While settling, the engine is
  still pumped every tick (giving the resize a chance to actually
  process), but **every** session's `sync_and_read()` is skipped entirely
  — the previous frame just stays displayed for those few tens of
  milliseconds instead of a black/corrupted one.
- `HeadlessServoSession::size()` (added earlier in C3a, both the real and
  stub impls) — removed. It existed only to support the old same-tick
  comparison this fix replaces with state-side tracking; grep-confirmed
  zero remaining callers before deleting, rather than left as orphaned
  public API (`CLAUDE.md`'s no-dead-code invariant).

**Tests, `crates/ferrite-ui/src/lib.rs`:**
`servo_frame_decrements_resize_settle_ticks_and_stops_at_zero` (arms the
counter directly via struct literal — R7 means no test can ever construct
a real session to exercise the actual `resize()`/`sync_and_read()` call
path, so this pins the pure counter state machine: decrements each tick,
never underflows past zero) and
`servo_frame_does_not_arm_the_resize_settle_window_without_a_session_to_resize`
(with `servo_sessions` empty — always true in this test build, since the
stub `HeadlessServoSession::new()` itself always returns `Err` — a
content-area size that differs from `last_resized_content_px` must NOT
arm the settle window or update the tracked size, proving the window only
ever arms as a *result* of an actual `resize()` call, never speculatively).

**Verified:** `cargo build --workspace`, `cargo test --workspace` (every
crate green, 0 failures — `ferrite-ui` now 41, up from 39, two new tests;
`ferrite-servo` unchanged at 1), `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo machete`, `cargo deny
check` (`advisories ok, bans ok, licenses ok, sources ok`, same
pre-existing Servo-git-source warning every agent since A3 has logged)
all clean. `.github/workflows/ci.yml` re-validated with
`python3 -c "import yaml; yaml.safe_load(...)"`.

**Not verified live — the actual claim this entry makes:** this sandbox
has no real Servo build (`--features ferrite-servo/servo` takes 15–25
minutes and this session did not attempt it) and no attached display, so
the segfault itself could not be reproduced or confirmed fixed here. The
root-cause reasoning is derived directly from the user's own crash log
and screenshots, and the fix is a real, load-bearing behavior change
(frames are genuinely skipped during the settle window, not a cosmetic
adjustment) — but only the user's own relaunch, on the exact
`gemma4:31b`/real-Servo configuration that crashed before, can actually
confirm this holds.

**Commits:** `ec1fe8b`.

**Known issues discovered, not fixed:** the `RESIZE_SETTLE_TICKS = 3`
margin is a considered guess, not a measured value — flagged inline above
rather than asserted as definitively sufficient.

## 2026-09-24 — coordinator — segfault root cause was misdiagnosed here — real fix in `HeadlessServoSession::resize()`

**Scope:** the previous entry's fix (the `resize_settle_ticks` settle
window) was built and pushed on the theory that the crash was a same-tick
`resize()`/`sync_and_read()` race. The user relaunched the app on the
exact configuration that crashed before and reported the **identical**
symptoms, unchanged: segfault on refresh, page content stuck rendering
only in the top-left region, black screen on resize or opening the agent
panel. That is a direct disproof of the previous entry's root-cause claim,
not a partial fix needing more margin — same branch,
`fix/servo-resize-crash-and-ci-cargo-deny`.

**Real root cause, this time verified directly against the pinned
`libservo` source rather than inferred from a crash log** (source read at
`/root/.cargo/git/checkouts/servo-e53a6e7b994a25fe/301f7da`, the exact
commit this workspace's `Cargo.toml` pins at tag `v0.0.5`):
`HeadlessServoSession::resize()` (`crates/ferrite-servo/src/session.rs`)
called **both** `self.rendering_context.resize(size)` directly **and**
`self.webview.resize(size)`. Both operate on the same underlying
`RenderingContext` — `WebViewBuilder::new` is constructed with
`rendering_context.clone()`, so it's one shared object, not two.
`WebView::resize()` (`components/servo/webview.rs`) delegates to
`Painter::resize_rendering_context()` (`components/paint/painter.rs`),
whose very first line is `if self.rendering_context.size() == new_size {
return; }` — before it does the `set_document_view` transaction,
`send_root_pipeline_display_list`, and `set_needs_repaint(RepaintReason::
Resize)` calls that are what actually make libservo re-layout and repaint
the page at the new size. Calling `rendering_context.resize()` ourselves
*first* set that size ahead of `webview.resize()`'s own call, so by the
time `Painter::resize_rendering_context` ran its guard, it always saw "no
change" and returned immediately — the entire repaint-at-new-size path
never ran, on every resize, permanently. That is exactly the observed
symptom: page content frozen at whatever size it last legitimately
rendered at (the small top-left region), the rest of the now-larger
buffer never painted (black). Separately, the direct
`rendering_context.resize()` call never calls `make_current()` first
(unlike `sync_and_read()`'s own `make_current()` call, and unlike what
`resize_rendering_context` does internally before touching the surface)
— a real risk of touching the surface while a different tab's GL/surfman
context was current, consistent with the `GLD_TEXTURE_INDEX_2D is
unloadable` warning and the segfault both preceding and following it in
the user's log. Cross-confirmed against Servo's own reference headless
embedder, `ports/servoshell/desktop/headless_window.rs`'s
`request_resize()`/`maximize()`, which calls **only** `webview.resize()`
— with a comment explaining that call alone is sufficient because it is
what notifies `Paint`.

**Fix, `crates/ferrite-servo/src/session.rs`:** removed the direct
`self.rendering_context.resize(size)` call from
`HeadlessServoSession::resize()` entirely. It now does exactly what
`servoshell`'s reference embedder does: sets `self.width`/`self.height`
for this struct's own bookkeeping, then calls only
`self.webview.resize(PhysicalSize { width, height })`. Doc comment on the
method traces this full chain of evidence (files and functions read,
guard clause quoted, `servoshell` cross-check) so the reasoning doesn't
have to be re-derived the next time this code is touched.

**`crates/ferrite-ui/src/lib.rs`:** `RESIZE_SETTLE_TICKS` lowered from `3`
to `1` and its doc comment, and `FerriteBrowser::resize_settle_ticks`'s
own doc comment, both corrected — the previous entry's same-tick-race
theory is no longer presented as "the real bug this exists to fix" (a
disproven claim CLAUDE.md and this file's own R2 rule don't allow left
standing); the settle window is now documented as a small residual safety
margin kept out of caution, not the fix. The mechanism itself
(`last_resized_content_px`, `resize_settle_ticks`, the `ServoFrame`
handler logic) is unchanged — kept because a same-tick read immediately
after `resize()` is still a real, separate potential race in principle
even with the primary bug fixed, not because it was ever what the user
actually hit.

**Verified:** `cargo build -p ferrite-ui -p ferrite-servo`, `cargo clippy
-p ferrite-ui -p ferrite-servo --all-targets -- -D warnings`, `cargo fmt
-p ferrite-ui -p ferrite-servo --check`, `cargo test -p ferrite-ui -p
ferrite-servo` (41 + 1 passed, 0 failed — same counts as before, no test
behavior changed, only the resize call sequence), then full workspace:
`cargo build --workspace`, `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace` (every
crate green, 0 failures), `cargo machete` (no unused deps), `cargo deny
check` (`advisories ok, bans ok, licenses ok, sources ok`, same
pre-existing Servo-git-source warning every agent since A3 has logged) —
all clean.

**Not verified live, same honest limitation as the previous (disproven)
entry:** this sandbox has no real Servo build and no display, so this fix
rests on reading the actual pinned `libservo` source and matching Servo's
own reference embedder, not on reproducing the crash and watching it stop
happening. That is a stronger form of evidence than the previous entry's
crash-log inference, but it is still not a live confirmation — only the
user's own relaunch, on the same `gemma4:31b`/real-Servo configuration
that crashed twice already, can confirm this holds.

**Commits:** `cf41cd5`.

## 2026-09-24 — coordinator — agent loop's truncated-JSON crash: real bug found live, root-caused and fixed with a retry, not a band-aid

**Scope:** the user hit this live in the running app (task: "whats on
this page" against `https://doc.rust-lang.org`) after the segfault fix
above merged — the agent sidebar's final "Answer" showed a raw parser
error instead of a real answer, on a fresh branch off the now-merged
`main`, `fix/agent-json-parse-and-truncation`:

```
[error] EOF while parsing a string at line 1 column 565 (raw: {"action":"finish","answer":"This page is
the Rust Documentation overview, ... Documentation for `rustc` (the compiler),
`Cargo` (build tool), `rustdoc` (documentation tool), and `Clippy` (static analyzer), as)
```

**Root cause, traced to a real, named default, not inferred from the log
alone.** Both the agent loop's live implementations —
`ferrite-agent::browser_loop::run_agent_loop` and `ferrite-ui`'s
message-driven mirror of it (`spawn_next_step`) — built every
`CompletionRequest` with `ferrite_model::SamplingOptions::default()`,
never overriding `num_predict`. That default is `128`
(`crates/ferrite-model/src/request.rs`), which is correct for the
*fingerprint* call (`docs/REBUILD_DIRECTIVE.md` §10.2: "a short JSON
array... `num_predict` is capped around 128") but was silently reused for
the agent loop's own, much larger completions too — a full `AgentAction`
JSON object, and for `Finish`, a freeform prose `answer` that routinely
runs well past 128 tokens for anything but a one-line response. Ollama
hard-truncates generation the instant the cap is hit, mid-token if
necessary — so a longer answer comes back as a JSON object whose `answer`
string was never closed, and `serde_json` fails exactly as observed:
"EOF while parsing a string". This reproduces on *any* answer long enough
to cross 128 tokens, not just this one page — a real, generally-hit bug,
not a one-off.

Separately, both loops treated *any* parse failure — this truncation
included — as immediately fatal: the very first malformed response ended
the whole task, with the raw `serde_json` error and the raw (truncated)
response dumped straight into the "Answer" field the user reads. A model
glitch that is often self-correctable on the next turn was being treated
as unrecoverable.

**Fix, two parts, `crates/ferrite-agent/src/browser_loop.rs` +
`crates/ferrite-ui/src/lib.rs`:**

1. **`AGENT_LOOP_NUM_PREDICT = 2048`** (new `pub const` in
   `browser_loop.rs`, imported into `ferrite-ui` alongside the other
   constants it already shares, e.g. `SYSTEM_PROMPT`) — passed via
   `.with_options(SamplingOptions::default().with_num_predict(...))` on
   every agent-loop (`ModelTier::Main`) completion request in both
   implementations. Generous headroom for a real answer or a
   `fill_form`/`js_execute` payload, while still a real, finite cap, per
   §10.3's "cap `num_predict` on every call" — the fingerprint call's own
   `ModelTier::Small` request is untouched and still gets the 128 default,
   which is correct for it.
2. **Bounded self-repair retry**, same shape in both loops: a malformed
   response is no longer immediately fatal. The raw response is pushed as
   the model's own turn, followed by a message asking for exactly one
   complete, valid JSON object, and the loop continues — `browser_loop.rs`
   restructured from `for _ in 0..budget.max_steps` to an explicit
   `steps_taken` counter so a retry does not consume a step of
   `LoopBudget::max_steps` (no real browser action was taken); `ferrite-ui`
   already only counts `actions_taken`, so its retry path (routed through
   a new `StepFailure` enum on `AgentStepReady`, replacing the old plain
   `String` error so a provider failure and a parse failure can be told
   apart and handled differently) needed no equivalent restructuring.
   Retries are independently bounded by the new
   `MAX_CONSECUTIVE_MALFORMED_STEPS = 2` (both loops import the one
   constant), reset to `0` the moment a step parses successfully, so a
   model stuck producing garbage still fails fast — 3 consecutive failures
   ends the run with the real parse error, same as before this fix, just
   no longer on the very first one.

**Tests, both crates:** `browser_loop.rs` — the existing
`a_malformed_action_stops_the_loop_rather_than_panicking` updated to use
`always_content` (every attempt, not just the first, must fail, since the
retry budget itself is what now ends it) plus a new assertion that a
retry which never recovers is never recorded as an action taken; two new
tests, `a_malformed_response_is_retried_and_recovers_on_the_next_valid_one`
(a malformed-then-valid sequence reaches `Finished`, not
`MalformedAction`) and `a_malformed_response_retry_does_not_consume_the_step_budget`
(two malformed responses plus two real actions plus finish all complete
under a 3-step budget — would fail at 1 real action if retries were
charged against it). `ferrite-ui` — the existing
`agent_step_ready_with_a_model_error_fails_the_task` updated for the
`StepFailure::Model` wrapper; two new tests,
`agent_step_ready_with_a_malformed_action_retries_instead_of_failing_immediately`
(the run stays alive, `agent_response` stays `None`, and exactly one
retry-feedback pair is appended) and
`agent_step_ready_gives_up_after_max_consecutive_malformed_responses`
(starting already at the retry limit, the next failure surfaces the real
parse error and ends the run). `ferrite-ui` now 43 tests (up from 41),
`ferrite-agent` 13 (up from 10).

**Verified:** `cargo build`/`clippy --all-targets -D warnings`/`fmt
--check`/`test` on `ferrite-agent` and `ferrite-ui` individually, then the
same four plus `cargo machete` and `cargo deny check` across the full
workspace — every crate green, 0 failures, `advisories ok, bans ok,
licenses ok, sources ok` (same pre-existing Servo-git-source warning every
agent since A3 has logged). Doc-drift checks (`check_purge.sh`,
`check_no_archive_links.sh`) clean.

**Not verified live:** this sandbox has no real Ollama backend or API
key, so the fix rests on the actual `num_predict` default read directly
from `ferrite-model/src/request.rs` and on the exact truncation point in
the user's own reported error (`line 1 column 565`, an unterminated
`answer` string) matching what a 128-token cap predicts, not on
reproducing the crash and watching it stop. The retry logic itself is
fully covered by the new deterministic tests above (`MockProvider`, no
network), so that half of the fix — a malformed response no longer being
instantly fatal — is verified directly, independent of whether
`num_predict` was the only ever cause of a malformed response.

**Commits:** `6faeceb`.

## 2026-09-24 — coordinator — C3c: real light/dark theme toggle, resolved palette, smoother loaders

**Scope:** third item of the C3 UI-polish sequence (C3a: rendering/
coordinate/resize correctness; C3b: real page favicons — landed as
`a75d17e` on this same branch, `feat/favicons-and-theme-polish`, with no
dedicated `PROGRESS.md` entry of its own; noted here rather than silently
left unfiled, though backfilling it is not this entry's job). Reported
directly by the project owner: "the UI is not browser rich... clean,
aesthetic looking, not cheap, good colour scheme, light dark theme, smooth
loaders... proper great ui and ux." `crates/ferrite-ui/` only, per this
charter's stated boundary (no bookmarks/history/downloads/find-in-page/
zoom/settings — that is C3d, explicitly out of scope here).

**Problem, confirmed by reading the file before changing anything (per
`CLAUDE.md`'s own standing instruction):** `crates/ferrite-ui/src/lib.rs`
was hardcoded to one dark theme — twelve module-level `const C_XXX: Color`
constants referenced by bare ident from `view()`, `view_agent_sidebar()`,
`new_tab_page()`, and nine `.style(...)`-callback functions (`nav_btn_style`
etc.), plus `launch()`'s `.theme(|_state| Theme::Dark)`. No toggle existed
to build on top of, and the tab bar's loading indicator's own comment
claimed it reused the agent sidebar's tick-driven dots pattern but the code
next to it never actually did — `text(if spinning {"..."} else {"•"})` is a
fixed string, not animated at all.

**Fix — real palette resolution, not a bigger constant table:**

1. **`Palette` struct** (`crates/ferrite-ui/src/lib.rs`) — the same twelve
   fields as the old `C_*` constants, now struct fields (`base`/`surface`/
   `raised`/`divider`/`text`/`text_dim`/`accent`/`accent_bright`/`input`/
   `safe`/`warn`/`danger`), plus `DARK_PALETTE` (the exact prior values,
   unchanged) and a new, considered `LIGHT_PALETTE` — not an RGB inversion:
   background depth runs the same direction real browser chrome uses
   (near-white page/toolbar, a touch grayer tab strip, grayer still for
   hover state), the purple accent is deepened from
   `rgb(112,97,255)`/`rgb(143,128,255)` to `rgb(89,66,235)`/
   `rgb(107,82,250)` so it stays foreground-safe on a light background, and
   `safe`/`warn`/`danger` are darkened from the dark theme's saturated
   versions, which measure under ~3.2:1 contrast against near-white — well
   under WCAG AA's 4.5:1 for the small-text badges this crate uses them for
   (e.g. the audit log's kind column). Contrast figures (`text` on `base` ≈
   17:1, `text_dim` ≈ 5.4:1, `accent` ≈ 6:1, `safe`/`warn`/`danger` ≈
   5.2-5.4:1) are hand-computed against the WCAG relative-luminance formula
   and documented inline on `LIGHT_PALETTE` — **not run through an
   automated contrast checker**, stated as such in the code itself.
2. **`AppTheme { Dark, Light }`** enum on `FerriteBrowser` (new
   `theme_mode` field), `Default` is `Dark` — every existing user sees
   exactly what they always have until they actually press the new toggle.
   `FerriteBrowser::palette()` resolves it for the three view functions
   that already take `state: &FerriteBrowser`; `palette_for_theme(&Theme)`
   is the equivalent lookup for the nine `.style(...)`-callback functions,
   which get the live `iced::Theme` handed to them by iced itself at render
   time (whatever `AppTheme::to_iced_theme()` last returned) — no extra
   state needed threading through those call sites at all, since the
   `_theme` parameter they already had (previously ignored) turned out to
   be exactly what was needed once renamed to `theme` and actually read.
3. **Every one of the ~150 `C_XXX` call sites converted**, grep-confirmed
   exhaustively: `grep -n "\bC_BASE\b\|...\|\bC_DANGER\b"
   crates/ferrite-ui/src/lib.rs` after the change matches only the four
   places those old names are still mentioned in prose (doc comments
   explaining the pre-C3c shape), zero remaining in actual code — the
   "half-converted theme" failure mode the charter warned about explicitly
   did not happen.
4. **`FerriteBrowserMessage::ToggleTheme`**, handled in `update()` by
   flipping `state.theme_mode`; a new toolbar icon button (`theme_btn`,
   next to the existing Audit/JS/Agent toggles) sends it, showing the icon
   of the mode a click switches *to* (sun while dark is active, moon while
   light is active) — the common browser/OS theme-switcher convention.
   `launch()`'s `.theme(...)` now reads `state.theme_mode.to_iced_theme()`
   instead of the hardcoded `Theme::Dark`.
5. **New `Sun`/`Moon` icons** (`crates/ferrite-ui/assets/icons/sun.svg`,
   `moon.svg`) follow C1's exact hand-authored convention (24x24 viewBox,
   `stroke="#000000"`, stroke-based, tinted at render time by `icon()`) —
   added to the `Icon` enum and `icon_bytes`, not left as unreferenced
   assets.

**Loader polish, same charter:** two new pure helpers next to the existing
`ease_out_cubic` (same tick-driven `progress_offset` `ServoFrame` already
advances every ~16ms, no new animation clock): `pulse_alpha(t, cycles,
base, amplitude)` — a smooth breathing alpha, replacing three near-
duplicate inline sine expressions — and `progress_segment_brightness(i,
segments, t)` — a single bright band sweeping across `PROGRESS_SEGMENTS`
segments and wrapping at the ring's seam. Applied: the tab bar's loading
dot now actually pulses (`pulse_alpha`) instead of being the static,
unanimated `"..."` string its own (now-corrected) comment had claimed was
animated; the top-of-window progress bar is now a
`PROGRESS_SEGMENTS`-wide row of individually-lit segments
(`progress_segment_brightness`) instead of one strip pulsing alpha in
lockstep — a real moving "in progress" indicator rather than a flat glow;
the new-tab "Fe" logo's existing pulse is refactored onto `pulse_alpha`
with its exact prior constants (cosmetically unchanged, code deduplicated).
The agent sidebar's "Working..." dots text was already genuinely
tick-driven (cycles through 0-3 dots off `progress_offset`) and was left
as is — it was not the "cheap"-reading case the tab bar's static string
was.

**Tests, `crates/ferrite-ui/src/lib.rs`:** 11 new —
`pulse_alpha_stays_within_base_and_base_plus_amplitude`,
`pulse_alpha_at_zero_is_exactly_base`,
`progress_segment_brightness_peaks_at_the_segment_under_the_sweep`,
`progress_segment_brightness_wraps_around_the_segment_ring`,
`progress_segment_brightness_is_bounded_zero_to_one`,
`default_theme_is_dark_matching_pre_c3c_behavior`,
`toggle_theme_flips_dark_to_light_and_back`,
`each_theme_resolves_to_its_own_distinct_palette`,
`light_and_dark_palettes_each_keep_text_readable_on_base`,
`to_iced_theme_matches_app_theme`,
`palette_for_theme_falls_back_to_dark_for_a_non_light_iced_theme` — every
one of the 43 pre-existing tests passes unmodified (no
`FerriteBrowser`/`FerriteBrowserMessage` struct-literal call site needed
changing beyond the two new additions, since every existing test already
builds state via `..FerriteBrowser::default()`, per this session's own
read of `docs/REBUILD_DIRECTIVE.md`'s R1/R7 discipline before touching
anything). `ferrite-ui` now 54 tests, up from 43.

**Verified:** `cargo build -p ferrite-ui`, `cargo clippy -p ferrite-ui
--all-targets -- -D warnings`, `cargo fmt -p ferrite-ui --check`, `cargo
test -p ferrite-ui` (54 passed, 0 failed) individually, then the full
workspace: `cargo build --workspace`, `cargo fmt --all --check`, `cargo
clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
(every crate green, 0 failures), `cargo machete` (no unused deps), `cargo
deny check` (`advisories ok, bans ok, licenses ok, sources ok`, same
pre-existing Servo-git-source warning every agent since A3 has logged),
`sh scripts/check_purge.sh`, `sh scripts/check_no_archive_links.sh` — all
clean.

**Not verified live — stated plainly, same limitation every UI-touching
session before this one has stated:** this sandbox has no attached
display and cannot build the real Servo feature, so **neither theme was
ever actually rendered or screenshot-checked** — not the light palette's
look, not the toggle button, not the segmented progress bar's sweep, not
the tab-bar dot's breathing. Every claim above is a description of the
code change and what its pure/state-machine logic does under test, not an
assertion about how it looks or feels on screen. The contrast figures
cited for `LIGHT_PALETTE` are hand-computed against the WCAG formula, not
verified with an automated contrast-checking tool. The user's own relaunch
is the real verification this entry cannot substitute for.

**Commits:** `f7d20ad`.

**Known issues discovered, not fixed:** C3b (`a75d17e`, real page favicons)
landed on this branch with no `PROGRESS.md` entry of its own — noted above
rather than silently left unfiled, but backfilling that entry (with its
own honest verification statement, written by whoever actually did that
work) is outside this entry's scope. Everything else already tracked
(T-212/T-217/T-218/T-219/T-222/T-223/T-227/T-228/T-230/T-231) untouched, as
before.

## 2026-09-24 — coordinator — C3b: real page favicons in the tab bar (backfilled entry for `a75d17e`)

**Scope:** second item of the C3 UI-polish sequence (C3a: rendering/
coordinate/resize correctness, already landed to `main`; C3c: theme
toggle + visual polish + loaders, the entry directly above this one,
which correctly flagged that this commit had shipped without its own
entry — filing it now, written by the same session that did the work).
Same branch, `feat/favicons-and-theme-polish`. The tab bar previously
showed only a placeholder glyph (`"..."` while loading, `"•"` once idle)
for every tab — never a real site icon, one of the concrete gaps the
project owner named explicitly ("i also dont see the page icons, like
github icon, or etc").

**Design: no new message plumbing needed, only an accessor.** Read
directly against the pinned `libservo` source
(`/root/.cargo/git/checkouts/servo-e53a6e7b994a25fe/301f7da`, the same
tag this workspace pins): `Servo`'s internal `spin_event_loop()` — the
same function `HeadlessServoSession::pump_engine()` already calls every
tick — already drains `EmbedderMsg::NewFavicon` and calls
`WebView::set_favicon()` internally (`components/servo/servo.rs`), which
in turn calls the `WebViewDelegate::notify_favicon_changed` hook
(`components/servo/webview.rs`). None of that required new code here —
the favicon was already being captured on every tab's `WebView` the whole
time; it just wasn't being read.

**Fix, `crates/ferrite-servo/src/session.rs`:**
- `HeadlessDelegate` (the existing `WebViewDelegate` impl already used for
  `notify_page_title_changed`/`notify_load_status_changed`) gained
  `notify_favicon_changed(&self, webview: servo::WebView)` — the callback
  takes no image parameter (per `WebViewDelegate`'s own doc comment, the
  new icon is read via `webview.favicon()` inside the hook), converted via
  a new pure function and stored in a shared `Rc<RefCell<Option<(u32, u32,
  Vec<u8>)>>>` cell, mirroring the existing `shared_page_title` pattern
  exactly.
- **`favicon_to_rgba8(width, height, format: servo::PixelFormat, data:
  &[u8]) -> Vec<u8>`** — favicons can decode to any of Servo's five
  `PixelFormat` variants (`K8`/`KA8`/`RGB8`/`RGBA8`/`BGRA8`) depending on
  the source image (a `.ico`'s frame, a plain PNG, ...), not just the RGBA8
  `iced_widget::image::Handle::from_rgba` expects — this is the one place
  in the crate that has to handle the full set rather than assuming a
  single decoder output, unlike `get_frame()`'s main-surface readback
  (`rendering_context.read_to_image`), which always comes back
  pre-converted. Covered by 5 unit tests, one per format, gated the same
  way the rest of this module is (`#[cfg(feature = "servo")]` — see the
  verification note below for what that means for this sandbox).
- `HeadlessServoSession` gained `shared_favicon`/`last_favicon` fields
  (synced in `sync_and_read()`, same tick as the title sync) and a public
  `get_favicon() -> Option<(u32, u32, Vec<u8>)>` accessor. The
  non-`servo`-feature stub gained a matching `get_favicon() -> None`.

**Fix, `crates/ferrite-ui/src/lib.rs`:**
- `FerriteBrowser::tab_favicons: Vec<Option<ImageHandle>>` — same indexing
  as `tab_titles`/`tab_urls`, pushed/removed alongside them in
  `AddTab`/`CloseTab`, synced every `ServoFrame` tick alongside the
  existing title sync.
- The tab bar's favicon slot now renders the real `ImageHandle` (via
  `ServoImage::new(...)`, the same construction `ServoImage`/`ImageHandle`
  the main content frame already uses) once a tab has one; the loading
  pulse and the idle dot are kept as the fallback for a tab that hasn't
  set a favicon yet or never will (`about:blank`, some error pages).

**Verified:** default (Servo-free) build only — `cargo build`/`clippy
--all-targets -D warnings`/`fmt --check`/`test` on `ferrite-servo` and
`ferrite-ui` individually, then the same four plus `cargo machete` and
`cargo deny check` across the full workspace, plus the doc-drift scripts —
all clean.

**Not verified — stated honestly, not glossed over:** this sandbox cannot
build the real `servo` feature (disk/time cost, same limitation every
prior entry this session has logged) and has no display, so `session.rs`'s
`mod inner` — everything in this fix except the `ferrite-ui` side and the
default stub — has never been compiler-checked here, only cross-read
against the pinned `libservo` source function-by-function (`WebView::
favicon`'s return type and signature, `PixelFormat`'s exact variant names,
`WebViewDelegate::notify_favicon_changed`'s exact signature, and that
`servo::lib.rs`'s `pub use embedder_traits::*;` makes `servo::Image`/
`servo::PixelFormat` resolve the same way `servo::LoadStatus`/`servo::
ConsoleLogLevel` already do elsewhere in this same file). Neither CI job
type-checks this path either (`ci` is Servo-free by design;
`build-servo-release` runs `cargo build`, not `clippy` or `test`) — a
pre-existing gap in this project's own CI, not introduced here, but worth
naming since it means this specific code's first real compiler check will
happen on `just build-servo`/a real macOS run, not before. The 5
`favicon_to_rgba8` unit tests exist for exactly that moment, not for this
sandbox's `cargo test --workspace`, which never compiles them (the module
they're in is `#[cfg(feature = "servo")]`-gated end to end).

**Commits:** `a75d17e`.

## 2026-09-24 — coordinator — C3d: bookmarks, history, zoom, find-in-page, settings, downloads

**Scope:** the last feature-completeness item in the C1–C4 UI plan (C1
design system/icons, C2 agent panel, C3a rendering/coordinate/resize
correctness, C3b favicons, C3c theme toggle/polish/loaders all landed
earlier on this same branch, `feat/favicons-and-theme-polish`). Six
features: bookmarks, history, zoom, find-in-page, settings, downloads —
all in `crates/ferrite-ui/`, plus one `crates/ferrite-servo/` change
(history), per this charter's stated boundary. `ferrite-ipi`/
`ferrite-agent`/the consent-dry-run architecture were not touched, per
`CLAUDE.md`'s "never reference, import, or revive" list and this
charter's own explicit instruction to keep the download manager separate
from `ferrite_engine::BrowserEngine::download()` (the agent's dry-run/
consent-gated tool vocabulary — a different concern, `docs/DECISIONS.md`'s
IPI/consent boundary ADRs).

**Mechanism, by feature:**

1. **Bookmarks** — `Bookmark { title, url }`, JSON-persisted via an
   injected `&Path` (`load_bookmarks_from`/`save_bookmarks_to`, matching
   `ferrite-model::config::default_cache_dir()`'s own dependency-injection
   shape for testability), `default_bookmarks_path()` resolving the real
   `~/.local/share/ferrite/bookmarks.json` (a data directory, not a cache
   one — bookmarks are a real loss to the user, unlike `ferrite-model`'s
   response cache). Loaded once in `launch()`, never in `Default` (R7).
   Star toggle (`Icon::BookmarkOutline`/`BookmarkFilled`) lives in the
   address bar itself, the common Chrome/Firefox placement, rather than a
   toolbar button.
2. **History** — two independent halves. `crates/ferrite-servo/src/
   session.rs`'s `HeadlessDelegate` gained `notify_history_changed`
   (Servo's own native per-tab session-history hook — the same
   `Rc<RefCell<...>>` shared-cell pattern `notify_favicon_changed`
   already established, the most recent addition before this one), which
   let `can_go_back`/`can_go_forward` become real (`can_go_forward` used
   to hardcode `false` unconditionally; `can_go_back` used to approximate
   from a bare `Complete`-event counter, now deleted as genuinely dead
   code once nothing read it). The History **panel**'s browser-wide,
   recency-ordered list (`FerriteBrowser::history`) is populated
   separately, from `LoadStatusChanged`'s own already-existing "did this
   tab's URL really change" signal (`record_history_visit`, deduped
   against only the immediately preceding entry) rather than by reading
   the new `HeadlessServoSession::history()` accessor every tick — a
   deliberate choice, documented inline: simpler than reconciling N
   per-tab Servo history lists into one recency-ordered view, and built on
   this crate's own already-verified-working signal rather than on
   `notify_history_changed`'s data shape, which (like the rest of
   `ferrite-servo`'s `servo`-feature code) this session could only verify
   by reading the pinned source, not by compiling it. Session-only, not
   persisted across restarts — a real, documented scope cut over
   bookmarks' much simpler "load once, save on every change" persistence.
3. **Zoom** — per-tab `tab_zoom: Vec<f32>` (indexed like `tab_titles`/
   `tab_urls`/`tab_favicons`, synced on `AddTab`/`CloseTab`), applied via
   a CSS `transform: scale(...)` + compensating `width` injected through
   `execute_js` (`zoom_script`) — Servo has no native zoom API at the
   pinned `v0.0.5` tag (confirmed by reading `components/servo/
   webview_delegate.rs`/`components/shared/embedder/lib.rs` directly, per
   this charter's own research brief), and `transform` was chosen over the
   nonstandard, legacy WebKit `zoom` CSS property since the latter is not
   certain to exist in a standards-focused engine like Servo. Cmd/
   Ctrl+=/-/0 shortcuts (`ZOOM_LEVELS`, the same discrete step table
   Chrome/Firefox expose). Re-applied on every real navigation
   (`LoadStatusChanged`'s completion branch), since a fresh document has
   no memory of a previous page's injected transform. Toolbar surfacing:
   a zoom indicator/controls row appears **only** when the active tab's
   zoom isn't 100% — C3c's toolbar had already reached 8 elements (back/
   forward/reload/address/audit/JS/agent/theme), and this charter's own
   brief named "don't just bolt on 6 more permanent toolbar buttons" as a
   hard constraint; a `default_zoom` setting (Settings tab) affects tabs
   created after it changes, not the current one.
4. **Find-in-page** — an overlay-styled bar in the normal layout flow
   (not a true floating widget layer — iced 0.13's `stack` widget is
   never used elsewhere in this crate, and introducing it just for this
   bar wasn't worth the new surface), triggered by Cmd/Ctrl+F, closed by
   Escape (resolved in `update()`'s `EscapePressed` handler rather than in
   `handle_key_press` itself, since `iced::keyboard::on_key_press`
   requires a plain `fn` pointer with no state access — verified directly
   against the pinned `iced_futures` 0.13.2 source after the first attempt
   at capturing `show_find_bar` in a closure failed to compile). Search is
   standards-based DOM manipulation (`document.createTreeWalker`/`Range`,
   never the nonstandard `window.find()`) via `find_script`/
   `find_navigate_script`, run live on every keystroke and on next/
   previous. `extract_json_object` parses the script's `JSON.stringify`
   return value defensively (direct parse, then a Rust-`Debug`-quoted-
   string unwrap, then brace-finding) — `HeadlessServoSession::
   execute_js`'s exact return-value shape (a `Debug`-formatted wrapper
   around whatever Servo's `evaluate_javascript` callback hands back) was
   not verified against the pinned source the way the favicon/history
   work was, so this parsing is defensive by design rather than assuming
   the shape.
5. **Settings** — read-only display of the resolved `FERRITE_MODEL_SMALL`/
   `FERRITE_MODEL_MAIN` tags and the response cache dir (`ModelConfig::
   cache_dir`, captured in `launch()` alongside the tags already were) —
   no free-text tag entry, per `CLAUDE.md` §10.2's no-hardcoded/no-
   silently-bypassed-config rule — plus real, working toggles for state
   this crate already has (theme — reuses `ToggleTheme`; default zoom —
   four presets). No placeholder rows for anything not yet settable.
6. **Downloads** — `run_download` (`tokio::task::spawn`, the same
   spawn-the-I/O-report-back-over-`agent_event_tx` shape `spawn_next_step`
   already established for the agent loop's own background model calls):
   a real `reqwest::Client::get(url)`, streamed chunk by chunk via
   `Response::bytes_stream()` (iterated with `iced::futures::StreamExt` —
   the real `futures` crate iced already re-exports and this file already
   imports elsewhere, so no new stream dependency was needed) to
   `dirs::download_dir()` via `tokio::fs::File`/`AsyncWriteExt`, reporting
   a `DownloadProgress` message after every chunk. `resolve_download_path`
   is pure and fully testable (collision-avoidance checked only against
   the in-memory download list, not live disk state — an honest,
   documented scope cut). Trigger is a manual "Download current page"
   action in the Library panel's Downloads tab; in-page `<a download>`
   click interception is out of scope for this pass — filed as T-232, not
   silently dropped.

**Information architecture:** all four of bookmarks/history/downloads/
settings share one new toolbar button — "Library" (`Icon::Menu`,
`show_library_panel`/`library_tab: LibraryTab`) — rather than four more
permanent toolbar buttons, and zoom's controls are conditional rather
than permanent. Net toolbar growth for six features: one button. This was
a direct response to the charter's own explicit warning against turning
the toolbar into "a wall of buttons," matching C3c's stated "clean, not
cheap" aesthetic goal.

**New `Cargo.toml` dependencies:** `ferrite-ui` gained `dirs` (bookmarks-
file/downloads-dir resolution — already a workspace dependency via
`ferrite-model`, newly used here), `reqwest` (the download GET — already
a workspace dependency via `ferrite-model`, a second independent user of
the same one pinned version), `chrono` (history timestamps — already a
workspace dependency via `ferrite-audit-log`), and `serde` (bookmark JSON
persistence). No new crate entered the workspace. The workspace-level
`reqwest` gained the `"stream"` feature and `tokio` gained `"fs"`/
`"io-util"`, both newly and actually used by the download manager (the
exact "only what's used" discipline T-207 trimmed `tokio`'s feature list
down to in the first place — these are additions back onto that
discipline, not a regression of it).

**Tests:** 47 new in `crates/ferrite-ui/src/lib.rs`'s `mod tests` (plus 3
in `icons.rs` for the 3 new icon variants and the updated count) — pure-
function coverage for every script/path-resolution/parsing helper
(`zoom_script`, `next_zoom_level`/`prev_zoom_level`, `find_script`/
`find_navigate_script`, `extract_json_object`, `download_file_name`,
`resolve_download_path`, `format_bytes`, `record_history_visit`, the
bookmarks JSON round-trip) plus `update()` handler coverage for every new
message (`ToggleBookmarkCurrentPage`/`RemoveBookmark`, `ClearHistory`,
`ZoomIn`/`ZoomOut`/`ZoomReset`/`SetDefaultZoom`, `OpenFindBar`/
`CloseFindBar`/`FindQueryChanged`, `DownloadCurrentPage`/
`DownloadProgress`/`DownloadCompleted`/`DownloadFailed`,
`ToggleLibraryPanel`/`SelectLibraryTab`, and `EscapePressed`'s new find-
bar-priority branch), all via the same direct `FerriteBrowser`/`update()`
construction every existing test in this file already uses (many via
`..FerriteBrowser::default()` struct-literal overrides, per this
project's own established test style) — no live network, no real `$HOME`,
matching R7 (the two `#[tokio::test]`s that spawn `run_download` never
`.await` past the spawn point, so the real `reqwest` call inside it is
never actually polled, same reasoning this file's own test-module header
already documents for the agent loop's background-task tests).
`ferrite-ui` now 101 tests (up from 54); `ferrite-servo`'s default
(servo-free) build is unaffected in test count (1 test, unchanged) since
the history-delegate code lives entirely inside the `#[cfg(feature =
"servo")]`-gated module this sandbox never compiles.

**Verified:** `cargo build`/`clippy --all-targets -D warnings`/`fmt
--check`/`test` on `ferrite-ui` and `ferrite-servo` individually after
each meaningful change, then the full workspace: `cargo build
--workspace`, `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace` (every crate
green, 0 failures), `cargo machete` (no unused dependencies), `cargo deny
check` (`advisories ok, bans ok, licenses ok, sources ok`, same
pre-existing Servo-git-source warning every agent since A3 has logged),
`sh scripts/check_purge.sh`, `sh scripts/check_no_archive_links.sh` — all
clean.

**Not verified live — stated plainly, same limitation every UI-touching
session on this branch has stated:** this sandbox has no attached display
and cannot build the real `servo` feature, so none of this was ever
rendered or screenshot-checked — not the Library panel's layout, the find
bar's on-screen placement, the zoom indicator's appearance, the bookmark
star icon, or any interaction with a real page. `crates/ferrite-servo/src/
session.rs`'s `notify_history_changed`/`history()` addition was never
compiler-checked here either, same as the favicon work before it
(`a75d17e`) — cross-read against the pinned `libservo` source
(`components/servo/webview_delegate.rs`'s `WebViewDelegate::
notify_history_changed` signature) rather than compiled; this sandbox's
own default (servo-free) build and test suite for `ferrite-servo` is
green, but that build never touches `mod inner`. `HeadlessServoSession::
execute_js`'s exact return-value shape (relevant to find-in-page's result
parsing, `extract_json_object`) was likewise never verified against the
pinned source — the defensive, multi-strategy parsing is this session's
mitigation for that specific unknown, not a substitute for having
verified it. The user's own relaunch, on a real machine with a real
display and the `servo` feature built, is the verification none of the
above can substitute for.

**Commits:** `55d1cde`, `155a2aa`, `ce1b8a7`.

**Known issues discovered, not fixed:** T-232 (in-page `<a download>`
click interception is out of scope for this pass — the download
manager's only trigger is the Library panel's manual "Download current
page" action). Everything else already tracked in `docs/TO-DO.md`
untouched, as before.

## 2026-09-24 — coordinator — new-tab hero: real quick-access-tile favicons + redesigned hero page

**Scope:** the project owner's own two-part request against the live
screenshots ("can we change the icons here and use the actual website
icons, also can we have a better hero page, when no site is loaded"),
against `fn new_tab_page` in `crates/ferrite-ui/src/lib.rs`. Same branch,
`feat/new-tab-hero-and-site-icons`, created off `main` after C1–C3d had
already landed there. `crates/ferrite-ui/` plus root `Cargo.toml`/
`Cargo.lock`/`deny.toml` only, per this pass's own stated boundary —
`ferrite-ipi`/`ferrite-agent`/`ferrite-servo`/the consent-dry-run
architecture were not touched.

**Problem, confirmed by reading the file before changing anything:** the
six quick-access tiles (DuckDuckGo/Rust Docs/GitHub/Servo/Hacker News/
Wikipedia) rendered a literal `text("[D]")`/`text("[R]")`/... bracketed-
letter string as their "icon" — never a real favicon. The page around them
was a flat, single-`palette.base`-background column: bare "Fe"/"ferrite"/
tagline text, the search bar, the tile row, a shortcuts pill — no
background treatment, no section hierarchy, no depth.

**1. Real favicons — fetch, on-disk cache, decode.** No image-decoding
crate existed in this workspace before this pass (confirmed:
`iced_widget::image::Handle::from_rgba` only displays already-decoded raw
RGBA bytes). Added `image` (crates.io, `default-features = false, features
= ["ico"]`, which itself pulls in `bmp`+`png` per the crate's own
`Cargo.toml`) as a new direct `ferrite-ui` dependency. **Verified before
adding it, per `CLAUDE.md`'s hard verification rule:** `cargo tree -p
ferrite-ui -i image@0.25.10` shows it already resolves to that exact
version as a transitive dependency of `libservo` (gated behind the
`servo` feature nothing here enables by default) — pinning `image = "0.25"`
here adds a new direct *edge*, not a new *version*, to the dependency
graph; `Cargo.lock`'s diff is exactly one line (`+ "image 0.25.10"` under
`ferrite-ui`'s own `dependencies`), confirming this.
- `favicon_host`/`favicon_cache_dir`/`favicon_cache_path`/
  `decode_favicon_rgba` (`crates/ferrite-ui/src/lib.rs`) are pure and
  directly unit-tested — the same dependency-injection discipline
  `resolve_download_path`/`load_bookmarks_from` already established.
  `decode_favicon_rgba` is tested against a **real PNG**, encoded in the
  test itself via the same `image` crate (`RgbaImage::write_to`) and then
  decoded back through the function under test, round-tripping the exact
  pixel bytes — proof the decode call itself works against real image
  bytes, not just that the code type-checks.
- `fetch_favicon_bytes`/`fetch_tile_favicon` do the one real network
  call + cache read/write, following the exact "spawn the I/O in the
  background, report raw bytes back over `agent_event_tx`" shape the C3d
  download manager's `run_download` already established — sent once at
  real startup (`launch()`'s startup `Task::batch`, a new
  `FetchTileFavicons` message, mirroring `ServoReady`), never from
  `FerriteBrowser::default()`/any test path (R7). A cache write only
  happens after a freshly-fetched response has already decoded
  successfully, so a transient bad response (an HTML error page served
  with 200, a truncated body) is never cached as a false "real icon" —
  the next launch simply retries.
- Cache: `~/.cache/ferrite-ui/favicons/<host>.ico` — the same `.cache`
  (re-fetchable) vs. `.local/share` (irreplaceable) distinction
  `default_bookmarks_path`/`ferrite_model::config::default_cache_dir()`
  already establish for the same reason. Resolved once by `launch()`
  (`favicons_cache_dir`), `None` in every test/`Default` construction —
  `FetchTileFavicons` is then a no-op, same test-safety shape as
  `bookmarks_path`/`downloads_dir`.
- **Fallback:** a tile with no resolved favicon (no network, first launch
  before the fetch completes, a `/favicon.ico` that doesn't resolve or
  decode) shows a designed monogram — the site's first letter over a
  colour that's a deterministic HSL hue-rotation of `palette.accent`
  (`tile_accent`/`rgb_to_hsl`/`hsl_to_rgb`, all pure, all tested,
  round-trip-tested against a real palette colour), one hue per tile, so
  the row reads as six distinct, intentional tiles even before any
  favicon has loaded — never a broken-image glyph or empty space.

**Which of the six sites' `/favicon.ico` paths actually resolve — not
established this session, stated honestly rather than guessed:** this
sandbox's own outbound proxy blocks every one of the six domains outright.
Tried `curl` against `https://github.com/favicon.ico` first (proxy
special-cases `github.com` for git/API operations — got a GitHub-API-
shaped 403, not a real favicon-fetch signal), then against all six real
domains directly — every one failed at `curl: (56) CONNECT tunnel failed,
response 403` before a single byte of any site's actual response was ever
seen. Confirmed via `curl -sS "$HTTPS_PROXY/__agentproxy/status"`: this
proxy's own reachable-host set does not include general web browsing.
**This is a materially different, stronger position than every prior
visual pass on this file has been able to claim, and it's stated precisely
so it isn't overclaimed:** the fetch+cache+decode *pipeline itself* was
proven end-to-end against real, freshly-encoded PNG bytes
(`decode_favicon_rgba_decodes_a_real_png_round_trip`) — real `image`-crate
decode logic, not a stub — but the *network half* (does `github.com`'s
real `/favicon.ico` actually return a decodable image at all) was never
exercised here, only type-checked/compiled. Say so plainly to the project
owner rather than letting the PNG round-trip test imply more than it
proves.

**2. Hero page redesign — built on C3c's palette/pulse system, no
parallel one introduced.**
- **Background:** `hero_background` — a subtle top-to-bottom
  `iced_core::gradient::Linear` fade between `palette.base` and
  `palette.surface` (real, verified-present API on the pinned `iced`
  0.13.1: `Background::Gradient`, `gradient::Linear::new(impl
  Into<Radians>).add_stop(offset, color)`, `f32: Into<Radians>`), two
  adjacent palette depth levels, not a new hue — a hint of depth behind
  the centered content rather than a competing pattern.
- **Brand mark:** the bare "Fe" text is now a raised, bordered badge
  (`palette.raised` background, accent-tinted border/shadow) with a soft
  breathing glow — `pulse_alpha`, the exact same tick-driven helper the
  tab bar's loading dot and the content area's own "Fe" loading
  placeholder already use (C3c), reused rather than a fourth
  independently-authored animation. Only the badge's own text alpha
  animates; nothing below it ever dims or waits on the tick.
- **Search bar:** widened (560px → 600px), paired with a new leading
  `Icon::Search` glyph (a new hand-authored SVG, `assets/icons/
  search.svg`, matching C1's exact 24x24 stroke-based convention) sitting
  outside the pill rather than fused into `text_input`'s own background —
  iced 0.13's `text_input` has no `on_focus`/`on_unfocus` to drive an
  outer container's border the way the input's own `.style` closure
  already reacts to `text_input::Status`, so the pill keeps its existing
  focus-reactive border/background exactly as before rather than losing
  that to gain a fused icon. Stays the page's one unambiguous primary
  action, per the brief.
- **Tiles:** each tile's `"[X]"` placeholder is replaced by `tile_glyph` —
  a rounded, tinted square housing either the real favicon image or the
  monogram fallback, same backdrop either way so a favicon arriving
  mid-session never makes the tile jump size/position. A new "QUICK
  ACCESS" eyebrow label gives the row a real section identity it had none
  of before. `QUICK_ACCESS_TILES` (a new `const` array) replaces the old
  inline `tiles: Vec<(&str, &str, &str)>` literal as the one source of
  truth both the fetch handler and the view now read from.

**`cargo deny check` found a new duplicate-version pair
(`image`/`png`)** the moment `image` became a real dependency —
`iced_graphics` already carries its own, unrelated `image` 0.24 (one major
version older), and `image` 0.25's `png` decoder is one version newer than
`tiny-skia`/`resvg`/`sctk-adwaita`'s existing `png` 0.17, both traced and
confirmed via `cargo deny check`'s own dependency-tree output. **Fixed,
not filed as a new T-###:** this is the exact same structural class
`deny.toml`'s own `[bans] skip` comment already documents for three
earlier, unrelated batches (Servo-vs-iced/winit era mismatches; `keyring`/
`dirs`; C1's `svg` feature/T-231) — two more `skip` entries, in the file's
own established style/precedent (T-231's own row named this exact move —
"left for whoever owns `deny.toml` next" — as the accepted fix), not a new
open item.

**Tests, `crates/ferrite-ui/src/lib.rs` + `icons.rs`:** 16 new in
`lib.rs`'s `mod tests` (`favicon_host_extracts_the_lowercased_host_from_a_
tile_url`, `favicon_host_is_none_for_an_unparseable_url`,
`every_quick_access_tile_url_has_a_resolvable_host`,
`favicon_cache_path_names_the_host_under_the_given_cache_dir`,
`decode_favicon_rgba_decodes_a_real_png_round_trip`,
`decode_favicon_rgba_is_none_for_non_image_bytes`,
`decode_favicon_rgba_is_none_for_empty_bytes`,
`rgb_to_hsl_then_hsl_to_rgb_round_trips_a_real_colour`,
`rgb_to_hsl_handles_a_grey_with_no_saturation`,
`tile_accent_rotates_hue_deterministically_and_evenly`,
`tile_monogram_is_the_uppercased_first_character`,
`fetch_tile_favicons_is_a_no_op_without_a_resolved_cache_dir`,
`fetch_tile_favicons_spawns_one_task_per_tile_with_a_cache_dir_set`,
`tile_favicon_ready_sets_the_matching_slot`,
`tile_favicon_ready_with_an_out_of_range_index_is_ignored_not_a_panic`,
`default_ferrite_browser_starts_with_no_resolved_tile_favicons_or_
cache_dir`) plus 1 in `icons.rs` (the `Icon::Search` variant folded into
the existing `every_icon_variant_embeds_non_empty_well_formed_svg`/
`every_icon_shares_the_same_viewbox` table tests). `ferrite-ui` now 117
tests (up from 101), every pre-existing test passing unmodified. No live
network reachable from any of them: `FetchTileFavicons`'s two
`#[tokio::test]`s never `.await` past the point of spawning, the exact
same R7 reasoning this file's own test-module header already documents
for the download manager's tests.

**Verified:** `cargo build`/`clippy --all-targets -D warnings`/`fmt
--check`/`test` on `ferrite-ui` individually after each meaningful change,
then the full workspace: `cargo build --workspace`, `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo
test --workspace` (every crate green, 0 failures), `cargo machete` (no
unused dependencies), `cargo deny check` (`advisories ok, bans ok,
licenses ok, sources ok`, same pre-existing Servo-git-source warning every
agent since A3 has logged), `sh scripts/check_purge.sh`, `sh
scripts/check_no_archive_links.sh` — all clean.

**Not verified live — stated plainly, same limitation every UI-touching
session on this branch has stated:** this sandbox has no attached display
and cannot build the real `servo` feature, so neither the gradient
background, the logo badge's glow, the search bar's new layout, nor a
single real tile favicon was ever actually rendered or screenshot-checked
— every claim above is a description of the code change and what its
pure/state-machine logic does under test, not an assertion about how it
looks on screen. Beyond that general limitation, this pass's network half
specifically was never exercised for real either (see the favicon-fetch
section above) — a strictly narrower, more honestly-scoped claim than "not
verified" alone would suggest, since the decode half *was* proven for
real. The user's own relaunch, on a real machine with a real display, a
real network path to these six sites, and the `servo` feature built, is
the verification none of the above can substitute for.

**Commits:** `601137e`.

**Known issues discovered, not fixed:** T-233 (favicon fetch only tries
the fixed `/favicon.ico` path, never parses a page's own `<link
rel="icon">`; a cache file that fails to decode is never retried within
the same run) — both stated as accepted scope cuts in this pass's own
brief, not silently dropped. Everything else already tracked in
`docs/TO-DO.md` untouched, as before.

## 2026-09-24 — coordinator — embedded Inter as the app's real default font

**Scope:** direct user feedback on the app's own look, after the new-tab
hero pass above: "this is the worst UI i have ever seen... use better
font ig." `crates/ferrite-ui/` only. Same branch,
`feat/new-tab-hero-and-site-icons`.

**Root cause, confirmed by grepping the whole file before assuming
anything:** `crates/ferrite-ui/src/lib.rs`'s `launch()` never called
`.font(...)` or `.default_font(...)` on the `iced::application(...)`
builder — not once, in any of this session's prior UI passes (C1's design
system through the new-tab hero redesign). Every one of those passes
styled colour, spacing, radius, and animation deliberately, but the
actual typeface every widget was rendering in was always whatever generic
sans-serif iced falls back to when `Settings::default_font` is never set
— never a considered choice. Text is the majority of every screen in a
browser's chrome, so this was a real, structural gap behind the "cheap"
read, not a cosmetic afterthought.

**Fix:** embedded Inter (SIL Open Font License 1.1) at four static
weights — Regular/Medium/SemiBold/Bold — fetched directly from Google
Fonts' own CDN this session (`fonts.googleapis.com`/`fonts.gstatic.com`
were genuinely reachable from this sandbox, unlike the six arbitrary
sites the new-tab favicon work tried and got blocked on above — a
different, narrower allow-list gap, not a contradiction) and verified as
real TrueType font data (`file(1)`, correct byte sizes, not truncated/
error-page downloads) before being committed to `crates/ferrite-ui/
assets/fonts/`, alongside the license text (`OFL.txt`) fetched from the
same Google Fonts repository on GitHub. Static weights, not Inter's
variable-font file: iced's text shaper (cosmic-text/fontdb) selects the
closest already-registered *face* for a requested `Font::weight` rather
than interpolating a variable font's weight axis, so four static faces is
the mechanically correct way to get more than one real weight out of one
family here, not an arbitrary packaging choice.

`launch()` now registers all four via `.font(...)` and sets
`.default_font(Font::with_name("Inter"))`, so every `text`/`text_input`/
`button` widget in this file that never sets its own font renders in
Inter Regular instead of iced's fallback — the single highest-leverage
change available, since it touches every screen at once. A new
`font_weight(Weight) -> Font` helper is the one call site anything
wanting a heavier embedded face goes through; applied to the handful of
places in this file that are genuinely display/heading text rather than
body copy: the new-tab hero's "Fe" badge and "ferrite" wordmark, the
content area's own "Fe" loading placeholder, and the error page's "ERR"/
"Page could not be loaded" heading.

**Verified:** `cargo build`/`fmt --check`/`clippy --workspace
--all-targets -D warnings`/`test` all clean (every crate, 0 failures —
`ferrite-ui` stays at 117 tests, unchanged; this is a rendering-only
change with no new testable logic), `cargo machete` clean, `cargo deny
check` clean (the one pre-existing, unrelated Servo-source warning),
`check_purge.sh`/`check_no_archive_links.sh` clean.

**Not verified — stated plainly, same as every visual pass on this
file:** no attached display in this sandbox, so the font has never
actually been seen rendered — not the weight hierarchy, not whether Inter
reads as intended at this app's actual UI sizes. What *is* independently
verified, more than a typical "trust the code" claim: the embedded font
files themselves are real, valid, complete TrueType data, not a guess at
bytes that merely compile via `include_bytes!`. The user's own relaunch
is the only way to confirm this actually reads better.

**Commits:** `762c7b2`.

## 2026-09-30 — coordinator — agent context, chats, more actions, Laya fast lane, local setup (branch `feat/agent-context-chats-laya`)

**Scope (owner request, four parts + UX):** (1) the agent must take the
current site and the open tabs as context, deciding automatically whether
the page is relevant; (2) chats — new chat, history, per-message steps, and
follow-up messages that carry the chat's earlier turns plus the current
page; (3) more agent operations and much better page understanding; (4)
use Laya where it is faster, and ship a local setup script; keep the UI's
look but make it smooth.

**Root cause of "the agent doesn't take context", found by reading the live
path:** `read_dom`'s observation was literally `dom snapshot at <origin>:
root role=<role>` — the model never received a single element or line of
page text — and `DomNode::selector` is only set for elements with an `id`,
so nothing was reliably clickable. Every run also started from just the
task string (`vec![Message::user(prompt)]`); there was no memory at all.

**What landed** (built as five workstreams in isolated worktrees, merged,
19 commits, every one authored and committed by Rayan Jain):
- `5c318ac` `ferrite-engine::digest` — `PageDigest` with numbered `@N`
  element refs, `normalize_selector`, sanitizing/bounded rendering,
  password values never emitted.
- Engine + actions (`8e3c1ea`, `a982a8c`, `faefb99`): real digest script for
  both Servo engines (`page_ops.js`, stable refs stamped as
  `data-ferrite-ref`), every selector op accepts `@N`, `select_option`
  matches value or label, robust click/type/fill; 14 new actions
  (`read_page press_key hover set_checked scroll_to find_text extract_links
  submit_form wait_ms open_tab switch_tab close_tab list_tabs ask_user`);
  every state-changing action's observation now carries a fresh compact page
  table; system prompt v2. The IPI dry-run engine's `observe_page` logs
  nothing (ADR-010).
- Chats + context (`4fdbb10`, `d2f7454`): persistent `ChatStore` (one
  atomic JSON file per chat, corrupt files skipped), `decide_page_use`
  (88-prompt table test), `build_seed`, `trusted_task_text`.
- Laya (`7ba6d00`, `08152bb`): strict client for a laya-serve-compatible
  server and a confidence-gated step decider in the jev-ultrafast request
  format (ADR-009).
- Local toolchain (`29ee565`, `57e0baa`, `060fa07`): `scripts/setup-local.sh`,
  `run-local.sh`, `doctor.sh`, `laya/serve.py`, `laya/verify.py`, justfile
  recipes (`just setup`, `just run-local`, `just doctor`, `just laya-verify`).
- UI (`8300c35`, `b4157f5`, `ac1c872`, `22c97b0`, `b473d0c`): chat state and
  a single `conclude_run` funnel every run ends through; context seed wiring
  with the IPI defense fed `trusted_task_text` only; tab actions intercepted
  in the live loop (consent-checked, last-tab refusal); optional Laya fast
  lane through the same rejection/budget path; new `agent_panel.rs` (chat
  thread, history with confirm-delete, context chip Auto/On/Off, suggestion
  chips, copy, entrance animation, auto-scroll that respects scroll-up),
  Cmd/Ctrl+Shift+O = new chat, six icons. Also fixed a real bug found in
  the tab refactor: closing an earlier tab jumped to the wrong tab.

**Laya, honestly.** Researched from its README, the `cklxx/laya-browser`
write-up, jev-ultrafast (MIT) and the PyPI source. It is used only as a
gated fast lane for ordinary click/select/scroll steps; the LLM keeps
planning, typing text, answers, and the whole security path (ADR-009 has the
stage-by-stage table). It could not be run here (huggingface.co is blocked
in this sandbox, no GPU), so its accuracy and latency in this app are
**unmeasured** — the numbers quoted in the code are the authors' own.

**Verified (this tree, independently re-run by the coordinator):** `cargo
build`/`fmt --check`/`clippy --workspace --all-targets -D warnings`/`test`
— 949 tests, 0 failures (`ferrite-ui` 117 → 217); `cargo machete` clean;
`cargo deny check` (`advisories ok, bans ok, licenses ok, sources ok`);
`check_purge.sh`/`check_no_archive_links.sh` clean. The page script was run
against real HTML in jsdom (32 checks). The setup scripts have shell/Python
tests (env-file parser, run-local with a fake cargo, a real laya server app
with a fake router) and shellcheck clean.

**Not verified — stated plainly:** no UI was ever rendered (no display); the
real `servo` feature was not built, so the page script never ran in
SpiderMonkey; no real Laya checkpoint was downloaded or run; the setup
scripts never ran on macOS or with real downloads. Filed rather than
dropped: T-234 (Laya unmeasured), T-235 (page script on real Servo), T-236
(chat UI never rendered), T-237 (press_key mapping, scheme-less navigate,
run-id-less stale messages), T-238 (heuristic is English-centric).

**Commits:** `5c318ac`, `4fdbb10`, `d2f7454`, `7ba6d00`, `08152bb`,
`29ee565`, `57e0baa`, `060fa07`, `8e3c1ea`, `a982a8c`, `faefb99`, `8300c35`,
`b4157f5`, `ac1c872`, `22c97b0`, `b473d0c` (plus merge commits).

## 2026-09-30 — coordinator — typing into web pages, repo-local setup

**Reported by the owner:** input boxes and search boxes on web pages could
not be typed into. Root cause: the UI only forwarded mouse events to Servo;
keyboard events were never sent (`crates/ferrite-servo` had no key path at
all). Not fixed by any earlier round, contrary to what the previous summary
implied.

**Changed.**
- `ferrite-servo::session`: plain `PageKeyEvent`/`PageKey`/`PageNamedKey`/
  `PageEdit` types (compile without the `servo` feature) and a gated
  `send_key` that builds `servo::InputEvent::Keyboard` events; Cmd (macOS) /
  Ctrl + C/X/V go through `InputEvent::EditingAction` instead.
- `ferrite-ui`: `page_input.rs` maps iced key events to `PageKeyEvent`; a
  `listen_with` subscription forwards only events no widget captured, never
  chrome shortcuts, and not while the address/find bar is focused or the
  new-tab page is showing. Clicking the page clears address-bar focus.
- Found by the first real `--features servo` compile of `ferrite-servo` and
  `ferrite-shell`: the session's `last_history` was never refreshed (dead
  field warning), so back/forward availability never updated. Fixed in
  `sync_and_read`. That compile also surfaced six servo-gated clippy lints
  that CI never linted (`type_complexity` ×3, `chunks_exact_to_as_chunks`
  ×3) — fixed with a `SharedFavicon` alias and a targeted `allow`.
- Repo-local layout: `FERRITE_HOME` now defaults to `<repo>/.ferrite` and
  `CARGO_TARGET_DIR` to `<repo>/target` in the scripts and justfile; the app's
  chats, bookmarks and favicon cache follow `FERRITE_HOME` when set (else
  `~/.local/share/ferrite`). New `just setup-all` (real Servo + Laya) and
  `just run-all`. Downloads still go to the OS Downloads folder on purpose.

**Verified:** `cargo build`/`fmt --check`/`clippy --workspace --all-targets
-D warnings`/`test` — 972 tests, 0 failures; `clippy -p ferrite-servo
--features servo --all-targets -D warnings` clean; `cargo check -p
ferrite-shell --features ferrite-servo/servo` compiles; `cargo machete`,
`cargo deny check`, purge/archive-link scripts clean; script tests 18 + 40
pass.

**Not verified:** typing has not been run or seen — there is no display here,
so the key mapping against a real Servo page (IME, dead keys, layouts other
than US, key repeat) is unobserved (T-236 widened to cover it). The page
script still has not run on real Servo (T-235).

## 2026-09-30 — coordinator — Laya checkpoint download layout fix

**Reported by the owner's first `just setup-all` on macOS:** the venv and pip
install worked (laya 0.3.22), but the checkpoint step printed `Fetching 0
files` and failed the check for `rl_agent_config.json`/`model.safetensors`.
Cause: the script assumed `cklxx/laya-browser` keeps the checkpoint in a
`v10s/` sub-folder and downloaded with `allow_patterns=["v10s/*"]`; nothing
matched. That layout was an assumption — huggingface.co is blocked in this
sandbox, so it was never checked against the real repo.

**Changed.** `scripts/laya/fetch_checkpoint.py` (called from
`setup-local.sh`) lists the repo first and picks whichever layout matches: a
`NAME/` sub-folder, a branch/tag called `NAME`, or the repo root. If none
match it prints the repo's real file list and exits non-zero instead of
finishing "successfully" with zero files. `scripts/tests/test_fetch_checkpoint.py`
covers all layouts against a stub `huggingface_hub` (5 cases, in `just
test-local`).

**Verified:** the stub tests and the existing run-local tests pass. **Not
verified:** against the real Hugging Face repo (unreachable here); if the
repo's layout is none of the three, the script now prints the file list to
fix it from.

## 2026-09-30 — coordinator — page clicks sent twice; zoom reset transform

**Reported by the owner:** on his Mac (real Servo build) a Google Form loaded
but would not respond to input, not even scrolling; pressing reload then
segfaulted (`Segmentation fault: 11`, exit 139). The log tail showed only
`[ferrite-js] execute: 149 chars` lines, which is `zoom_script` at a level
of 100% or below.

**Not established:** the cause of the freeze and the segfault. No display or
route to Google from the sandbox and not enough disk for a full Servo build,
so nothing was reproduced. Filed as T-239.

**Fixed (found by reading the input path, not by reproducing the report):**
- Every click reached Servo twice: `ServoMousePress` sent a down,
  `ServoMouseRelease` sent an up *and* a second synthesised down+up. Checkbox
  and radio controls toggled twice and links fired twice. Release now sends
  only the up; press re-asserts the pointer position first. `send_mouse_click`
  had no other caller and is removed.
- `zoom_script` at 100% now removes the root `transform`/`width` instead of
  setting `scale(1)`; a leftover transform on `<html>` can change scrolling
  and hit-testing for the whole page. Test added.

**Verified:** `cargo clippy --workspace --all-targets -D warnings` and
`clippy -p ferrite-servo --features servo` clean; `ferrite-ui` 234 tests pass.
**Not verified:** none of this has been seen on a real page.

## 2026-09-30 — coordinator — real-Servo probes, dev profile default, new-tab page, agent search

**Reported by the owner (second time):** pages unresponsive in the real-Servo
app, reload exits (segfault), an agent search on google.com gave a white page,
and the new-tab page looked poor. His log showed `zoom_script` executions and
then `Segmentation fault: 11`.

**New: real Servo run here.** The sandbox has no internet, but a full Servo
dev build (17 min on 4 cores, `CARGO_INCREMENTAL=0`) plus `libegl1`/Mesa and
`xvfb-run` runs a headless session against local pages. Two probes were added
and both pass on Linux, dev profile:
- `crates/ferrite-servo/examples/input_probe.rs` (`just probe-input`):
  wheel scroll, click + typing + Backspace into a text input, a checkbox
  (exactly one click event), a button, reload, and scrolling under a
  zoom-out transform.
- `crates/ferrite-engine-servo/examples/digest_probe.rs`: the real page
  script — an 8-element digest with the password value withheld, then
  `type_text`/`set_checked`/`select_option`/`click`/`scroll_to` by `@ref`,
  the page state confirming each. This is the first time `page_ops.js` ran on
  real Servo (T-235 moved to in-progress).

**Bugs found and fixed by those probes:** each wheel event scrolled twice as
far (a legacy `Scroll` call was sent as well as the wheel event; measured 600
px vs 300 px). Earlier this round: every click sent twice; zoom reset left
`transform: scale(1)` on `<html>`.

**Cause of the owner's failure: not established.** The one variable that
differed between his working and failing runs looked like the Cargo profile
(the setup scripts built and ran `--release`, his earlier runs were dev
builds). That was tested and **ruled out**: a full release build of Servo
here passes the same two probes as the dev build, and a further stress run in
release (iframes, a 30k-node DOM, a 3 s busy script, five rapid reloads per
page, two tabs sharing the engine) never crashed on reload. The setup scripts
still default to `dev` (`FERRITE_PROFILE=release` opts back in) because that
matches his earlier working runs, not because of any measured difference.
Google Forms itself could not be loaded (no internet). Tracked as T-239, with
the log and a plain-form check still needed from him. Both probes exit with a
segfault at process teardown (`pthread_mutex_destroy failed`, Servo threads
still alive); not investigated. Found on the way: the audit panel reads a
database nothing creates (T-240).

**Also changed.** Agent prompt v3: search with DuckDuckGo Lite, never Google
(Google's results page does not render in this engine), and go elsewhere
when a page stays blank. New-tab page redesigned: flat background, wordmark,
one search field, an "Ask the agent instead" entry, a quick-access grid
aligned to the search width, a quiet shortcut footer; the pulsing badge,
gradient, per-tile hue rotation and its HSL helpers are gone.

**Verified:** `cargo fmt --check`, `clippy --workspace --all-targets -D
warnings` and `clippy -p ferrite-servo --features servo --all-targets`
clean; 970 tests pass; both probes pass; script tests pass. **Not verified:**
the new-tab page has not been rendered (no display for iced here) — only
compiled and covered by the existing widget-tree tests; macOS; any real website.

## 2026-09-30 — coordinator — second tab, landing page, activity trace, logins

**Reported by the owner (with screenshots from his Mac):** the new-tab page
rendered as a giant purple block with "Ferrite" pushed off the right edge;
a Google Form showed "File unavailable"; any second tab was unresponsive; he
wanted every agent/LLM/Laya action in the audit log with prompts, answers and
timings, Laya-vs-LLM speed visible, and logins (cookies, sessions) to work.

**New capability: the UI can be driven and seen here.** `libegl1`, Mesa,
`libxkbcommon-x11`, `xvfb`, `xdotool`, ImageMagick and `openbox` in the
sandbox run the real release app headless, click and type into it, and take
screenshots, against local pages and mock Ollama/Laya servers on loopback.
Everything below marked "seen" was checked that way; there is still no
internet, so no real site was loaded.

**Root causes found.**
- *Landing page:* `container.width(40).height(40).center(Length::Fill)` —
  `center(len)` sets both sides to `len`, silently overriding the fixed size.
  The same mistake was in the quick-access tiles. Fixed with
  `center(Length::Fixed(..))`; seen.
- *Second tab dead:* a new tab's session is created at 1280×700, but the
  resize tracking assumed it already matched the first tab's size, so every
  tab after the first was drawn at the wrong size (black margins) and pointer
  input landed in the wrong place. Switching tabs now re-bases the tracking on
  the active session's real size and marks the active WebView shown/focused
  (`sync_active_webview`); background tabs also skip the pixel read. Seen:
  two tabs, typing, scrolling, a checkbox, switching back — all work. Headless
  probe (`just probe-input`) covers the same.
- *Address bar:* clicking it did not select the address, so typing extended
  `about:blank` into `about:blankhttp://…` (a white page). The blank page now
  shows an empty bar with the prompt, a click or Ctrl/Cmd+L selects all until
  the user types, and a bare `127.0.0.1:port` / `localhost` gets http://.
- *A tab's first navigation was lost* when issued right after creation (the
  initial about:blank load won): `HeadlessServoSession::new` now waits for it.
  This is what the agent's `open_tab` would hit.
- The Audit/JS panels now sit below the page, like devtools, not above it.

**Activity trace (ADR-011).** `ferrite_model::trace` records every LLM call
(a `Trace` decorator), every Laya request, the fast-lane verdict and the
agent's own events; the Audit panel's new "Model calls" view shows a timeline,
a per-stage timing table and a one-sentence Laya-vs-LLM summary, with the full
prompt/answer one click away; also appended to
`$FERRITE_HOME/logs/model-activity.jsonl`. Seen end to end with a mock
Ollama and mock Laya: run start, fingerprint call, agent steps, consent
request and decision, a Laya step accepted, a Laya "DONE" sent back to the LLM,
run finished, and the summary line ("…the fast lane saved about 578 ms" — mock
timings, not a measurement of real models). The "Security log" view now reads
the real network audit log (T-240 fixed, seen).

**Logins (ADR-012).** The Servo profile persists under `$FERRITE_HOME/profile`
and is flushed on a clean shutdown; closing the window now shuts the engine
down cleanly (seen: files written, no crash). `profile_probe` shows a cookie
and `localStorage` surviving a restart. No real login was tried (T-241).

**Verified:** `cargo fmt --check`; `clippy --workspace --all-targets -D
warnings`; `cargo test --workspace --no-fail-fast` — 996 tests, 0 failures;
probes `input_probe`, `digest_probe`, `profile_probe` pass (release, headless
Linux). **Not verified:** macOS; any real website, Google Forms or Google
sign-in in particular; the release-profile exit segfault in probes that do not
shut the engine down; the new-tab page beyond the dark theme.

## 2026-09-30 — coordinator — favicons, alignment, shortcuts, popups

**Reported by the owner:** after the last round the site icons were not
visible and things were misaligned ("some to the left"); he asked how to sign
in with Google and for every fixable issue to be fixed. No screenshot this
time, so the app was rendered here (headless, Linux) and every screen looked
at, including favicons (seeded through the cache directory), several tabs,
the Library drawer, the agent drawer and the light theme.

**Found and fixed.**
- *Icons invisible:* real favicons were drawn on a near-transparent tint, so
  dark icons (GitHub, Rust) vanished on the dark theme. They now sit on a
  light tile (tiles and tab strip; light theme unchanged). The favicon
  download also sent no User-Agent (Wikipedia and others refuse that) and
  had no fallback when `/favicon.ico` was missing; it now sends one, times out
  after 8 s and falls back to the icons the home page declares
  (`icon_links_in_html`, tested).
- *Alignment:* tabs each took the width their title needed, so favicons,
  titles and close buttons sat at different x on every tab; all tabs are now
  190 px with a fixed favicon slot. The Library was a full-width bar with its
  contents stranded at the far left and pushing the page down; it is now a
  380 px drawer on the right like the agent's (one at a time). Settings'
  zoom row wrapped ("150 / %"); fixed.
- *Shortcuts dead in a text field:* a focused text field captures every key,
  so Cmd/Ctrl+T, +L, +W and +R did nothing right after typing an address
  (seen: the next URL was typed into the middle of the old one). Browser
  shortcuts now work from a captured key too; editing combinations still go
  to the field.
- *Popups and `target=_blank`:* Servo was asked for a new WebView and the
  request was ignored, so such links and sign-in popups did nothing. They now
  open as real tabs (seen: `window.open` and typing in the new tab).
- *T-242:* consent origins now keep non-default ports (done).
- `FERRITE_USER_AGENT` documented in `scripts/local.env.example`.

**Verified:** fmt; clippy (workspace, and the servo feature for
`ferrite-servo` and `ferrite-engine-servo`'s lib and examples); 1002 tests, 0
failures; machete; deny; all of the above seen in the rendered app.
**Not verified:** macOS; real favicon downloads (the cache was seeded with
generated icons, the network path is covered only by the link-parsing tests);
Google sign-in (no internet); Cmd+Q still skips the clean shutdown.

## 2026-09-30 — coordinator — native page zoom

**Reported by the owner:** a Google Form showed "File unavailable", sign-in
with Google showed the same, and clicking reload crashed the app (exit 139).
His log had a dozen `[ferrite-js] execute: 148 chars` lines: the zoom script
at 110% (width `90.9091%`), re-injected by JavaScript into the page on every
load. His earlier screenshots show the same form failing in the 110% tab and
loading fine in a tab at 100%.

**Changed.** Zoom now uses Servo's own page zoom (`WebView::set_page_zoom`,
`HeadlessServoSession::set_zoom`/`zoom`) — layout and hit-testing follow it
and no script is injected into any page. The earlier note that Servo has no
native zoom API at this version was wrong. `zoom_script` and its tests are
gone; the load handler only sets the engine zoom when it differs. Probe
`input_probe` checks devicePixelRatio, a click and a wheel scroll under zoom;
in the real app 125% relays the page out, survives F5, and logs no
`[ferrite-js]` lines.

**Not established:** that this fixes Google Forms or the reload crash. It is
the best-supported cause in his evidence (the only failing form was the zoomed
one; every `execute_js` runs a nested event loop on the UI thread inside a
heavy page), but nothing here reaches Google, so it is unproven (T-239).
Verified: fmt, clippy (workspace and servo feature), 1000 tests, 0 failures.

## 2026-09-30 — coordinator — native zoom did not fix Google Forms

**Reported by the owner:** after the native-zoom change (no `[ferrite-js]`
lines any more) the Google Form still showed "File unavailable" and the app
crashed again (exit 139, dev profile; the log also prints Apple's
`GLD_TEXTURE_INDEX_2D is unloadable` GL warning).

**So the zoom hypothesis in the previous entry was wrong**, or at least not
sufficient. A scratch probe (headless Linux, release) that does
`location.reload()`, `location.href = location.href`, `history.go(0)` and
five toolbar reloads on a page using fetch, workers, IndexedDB, a service
worker and BroadcastChannel did not crash and exited cleanly, so the crash
is not simply "reload" in this engine; it is specific to that page, to macOS,
or to both. The cause is unknown.

**Added so the next report has evidence:** script errors a page logs are now
printed as `[page console error] tab N: …` (they were silently dropped), and
`just crash-report` / `scripts/crash_report.py` prints the exception and the
crashing thread's stack from the newest macOS crash report (tested on a
synthetic report only). Verified: fmt, clippy, 1000 tests.

## 2026-10-01 — coordinator — the crash was a use-after-free in Servo; moved to 0.6.0

**Reported by the owner:** the Google Form still crashed after the native-zoom
change, and `just crash-report` (added last round) produced a real stack.

**What it showed.** `EXC_BAD_ACCESS`, address `0x4b4b4b4b4b4b4b7b` (SpiderMonkey
fills freed memory with `0x4b`), in `JS::GetScriptPrivate(JSScript*)` ←
`evaluate_script` ← `GlobalScope::run_a_classic_script` ←
`HTMLScriptElement::execute` ← `ServoParser::resume_with_pending_parsing_blocking_script`
← stylesheet `do_post_parse_tasks`. In the pinned v0.0.5 source,
`ClassicScript.record` is an untraced raw `NonNull<JSScript>`; the script is
compiled when it is fetched and kept until the stylesheet it is waiting on
has loaded, and a garbage collection in between frees it. The 0.6.0 source
holds a `RootedTraceableBox<Heap<*mut JSScript>>` there. (So: not zoom, not
reload, not Ferrite; a timing-dependent engine bug that a heavy real page
hits and local test pages do not. The earlier zoom and reload hypotheses
were wrong and are recorded as such above.)

**Done.** `libservo` git tag v0.0.5 → crates.io `servo = "=0.6.0"` (ADR-013).
Source changes: `MouseButton::{Left,Right}` → `{Primary,Secondary}`; workspace
`rusqlite` 0.37 → 0.38 (Servo's pin; one `libsqlite3-sys` per graph), which
dropped implicit `u64` SQL conversion, so the audit log now converts sequence
numbers explicitly; `deny.toml` skips a second `synstructure`; the gstreamer
brew package is no longer needed (no media backend in the default features).

**Verified (Linux, headless, release):** `input_probe` (scroll, typing, click,
checkbox, reload, native zoom, two tabs), `digest_probe` (digest and
`@ref` actions), `profile_probe` (cookie and localStorage survive a restart),
and the app itself (page loads, typing, Security log). fmt; clippy (workspace
and servo feature); 1000 tests, 0 failures; machete; deny; purge scripts.
**Not verified:** that the Google Form no longer crashes, and what "File
unavailable" is — no internet here; macOS; Servo 0.6.0 on his machine (first
build will recompile SpiderMonkey, ~20-60 min).

## 2026-10-01 — coordinator — evaluate the defense against every real-world case, then harden it; Laya that cost more than it saved

**Asked by the owner:** the browser now works and is merged; the defense is
the research, so write every attack and edge case that can happen, find what
breaks, make the defense as strong as it can be, and look at why Laya (1.4 s per
call, half of them timing out at 1.5 s) is slower than the LLM it should beat.
Branch `feat/defense-eval-corpus-and-laya-latency`, three commits
(`2869465`, `d52159c`, `aa30775`) plus this one.

**What attacking it found (ADR-014, ADR-015; `docs/TO-DO.md` T-243–T-250):**

1. **The predicted fingerprint was never enforced on the real run.** The dry
   run executes on synthetic pages, so a page that carries an injection exists
   only in the real run; the live loop compared the dry run once and then ran
   everything the user had not rejected. The eval hid this because it feeds the
   dry run the attacker's content. Fix: `RuntimeGuard` over the same
   `classify_event` that `compare` uses. On a blind dry run: 809/809 attacks
   succeed without it, 122/809 (all residual) with it, 0 of 129 benign runs
   blocked, 0 admitted actions blocked.
2. The dry run recorded a download and a new tab at the page's origin, not the
   destination.
3. The sanitizer matched one spelling (625 disguised instructions now detected
   and excised, 85 benign texts untouched), missed attribute, CSS-generated and
   raw-page text, was quadratic on text with no sentence terminators, and could
   cut half a tag when an attribute held `>`.
4. The rule layer over-admitted (substrings; "inbox" granted cookie access and
   not the page read) and under-recognised (no everyday read verb: 43% of the
   first benign corpus was gated).
5. A domain-suffix scope could be `com` or `github.io` (T-212): refused by the
   Public Suffix List.
6. **The evaluation mis-measured:** a deviation case declaring nothing on one
   side was "caught" against a clean diff (`.all()` over an empty set), origin
   strings were compared un-normalized, and domain-suffix/`task_open` cases had
   no context page. Numbers on the old n=29 corpus did not move (every declared
   deviation really was flagged), but the metric could not have failed.
7. Laya: the governor, the warm-up and the honest summary (ADR-015). The old
   summary said "saved about 831 ms" for a run that lost about 6.5 s.

**Corpus:** 909 generated cases (`scripts/gen_redteam_corpus.py`; 13 tasks x 12
goals x 11 carriers x 28 dressings x 3 scopes, look-alike origins, benign
controls) plus the 29 existing: 938 cases, 3,488 executions. Current numbers
(`docs/EVALUATION.md` §8.3): On ASR 63/809 = 7.8%, LoopOnly 122/806 = 15.1%
(the residual), ADR 100%, SDR 99.3%, FGR 0/129. **They are self-authored and
matrix-generated, so the intervals understate the uncertainty and they are
tuned-against data; the eval agent is scripted and the fingerprint rules-only,
so none of it says whether a real model takes the bait or what a real agent's
false-gate rate is.** The report now says so.

**Verified (Linux, headless, no network):** `cargo fmt --check`; `cargo clippy
--workspace --all-targets -D warnings` and `-p ferrite-servo --features servo`;
`cargo test --workspace` 1,100 passed, 0 failed, 2 ignored; `just eval` (938
cases, audit chain verifies); `just guard-eval`;
`scripts/gen_redteam_corpus.py --check`; the Python server tests (25, against
the real `laya` package, no torch); `cargo machete`; `cargo deny check` (`psl`
is MIT/Apache-2.0). `cargo check -p ferrite-servo --features servo` (the
audit-log hook is inside that feature).

**Not verified:** the runtime guard on a real page or in the GUI (no internet,
no display of a live injection; it is covered by unit tests over the real
`update()` function and the eval); Laya on the owner's Mac (the device and warm
step time will show in the new start-up log; T-250); that the Servo-feature
build still links end to end (type-checked and linted, not built); the ~20-60
min first build of Servo 0.6.0 and the Google Form crash (T-239) remain
unconfirmed from the previous round.

## 2026-10-01 — coordinator — Laya on the owner's Mac is MPS, not CPU

**Reported by the owner (start-up log):** `device=mps, first pass 1572 ms, warm
454 ms per step`, with the new "slower than a fast lane should be" warning.
**Correction:** the previous entry and ADR-015 guessed CPU from the trace; the
log disproves it. The slowness is a ~450 ms forward over a 1,024-token window on
MPS in fp32 (Laya's default below 5 question rows; Ferrite sends 2). **Done:**
the warm-up now also times fp16 on MPS and prints both; the new
`FERRITE_LAYA_MPS_AMP_MIN_ROWS=1` opts in; the warning no longer suggests the
CPU-only `FERRITE_LAYA_THREADS`. **Verified:** the 26 Python server tests (fake
agent; no torch, no MPS here). **Not verified:** whether fp16 is actually faster
or changes answers on his machine; no Apple hardware here.

## 2026-10-01 — coordinator — Web Crypto, an app icon, and CI/release on three OSes

**Reported by the owner:** Speedometer 3.1 fails in Ferrite (`crypto.getRandomValues()
not supported`, `crypto is not defined`, a Next.js client-side exception) and
passes in Brave; asked for an icon, and for CI and release on macOS, Windows and
Linux (CI manual; release to follow a successful CI run).

**Cause (T-251):** the `servo` crate was built without its `webcrypto` feature,
so `window.crypto` is not compiled in (ADR-016). **Done:** the feature is on
(lockfile moved only `hybrid-array` and `typenum` besides the additions),
`dom_intersection_observer_enabled` is on, and `web_api_probe`
(`just probe-web-api`) checks 25 required APIs in the real engine. **Verified
(Linux, headless, real Servo built with the feature):** all 25 pass, including
`getRandomValues`, `randomUUID` and `subtle` digest, AES-GCM and HMAC;
`IntersectionObserver` failed before its preference was set. Absent in this
Servo and only reported: `requestIdleCallback`, `adoptedStyleSheets`,
`FontFace`, `scheduler.postTask`, `OffscreenCanvas`, the Navigation API.

**Icon (T-252):** `assets/icon/ferrite.svg`, rendered by `scripts/make_icons.py`
to PNG, ICO and ICNS; embedded as the window icon, in `ferrite.exe`
(`build.rs`, Windows only) and in `Ferrite.app` / the Linux `.desktop`
(`scripts/package.sh`).

**CI/release (T-253):** `ci.yml` is manual-only with a Linux/Windows/macOS
matrix and packaged Servo builds; `release.yml` publishes after a successful CI
run (`workflow_run`). T-210's macOS-only decision is superseded by the owner's
new request.

**Verified:** `cargo fmt`; clippy `--workspace --all-targets -D warnings` and
`-p ferrite-servo --all-targets --features servo`; `cargo test --workspace` 1,101
passed, 0 failed, 2 ignored; `cargo deny check`; `cargo machete`; the packaging
script for all three platforms against a stand-in binary; both workflow files
parse as YAML; the window icon decodes (unit test).

**Not verified:** Speedometer 3.1 itself (no internet here); the icon in a
macOS Dock, Windows task bar or Linux launcher; the Windows icon-embedding build
step; anything in the workflows on a real GitHub runner (Linux and Windows
package lists, Windows and Linux test runs, the `workflow_run` hand-off, which
only takes effect once `release.yml` is on `main`).

## 2026-10-01 — coordinator — Windows CI: golden fixtures checked out as CRLF (T-254)

The first Windows run of `cargo test --workspace` failed three `schema_stability`
tests in `ferrite-core`. Cause: no `.gitattributes`, so the runner's default
`core.autocrlf=true` turned the LF fixtures into CRLF and the byte-for-byte wire
format comparison failed on `\r\n`. Fix: a root `.gitattributes` forcing LF
(binary assets excluded); `git add --renormalize .` is a no-op, so the blobs
were already LF. The test was not loosened. **Not verified:** a Windows run
after the change.

## 2026-10-01 — coordinator — Windows CI: node payload past the command-line limit (T-255)

With the fixtures fixed, the next Windows run failed two `ferrite-engine-servo`
tests (`select_option_*`): `run_select_in_node` handed node a ~42 KB payload as
an argument, over Windows' ~32 KB limit (os error 206). The payload is now
piped on stdin. All 15 tests in the crate pass here with node installed.
**Not verified:** a Windows run. Each Windows run so far has surfaced a new
class of failure, so expect more until one is green.

## 2026-10-01 — coordinator — Windows CI: closed-port refusal is slower than Laya's timeout (T-256)

The third Windows run got through every earlier failure; one `ferrite-model`
test remained (`a_server_that_is_down_is_a_connect_error_...`): it expected
`Connect` but the client's 1.5 s timeout fired first, because Windows takes
about 2 s to refuse a connection to a closed loopback port. Test-only fix (a
15 s timeout in that test); the client's behavior is unchanged. Linux and macOS
were already green. **Not verified:** a Windows run after this change.

## 2026-10-01 — coordinator — In-app model settings, and the public site (T-257, T-258, T-259)

**Asked by the owner:** a way for people who download the browser to connect their own agent (provider, API key, model) in the app rather than a `.env`, in the app's own theme; and a clean marketing site with a download section and documentation centered on the defense, the metrics and how they are evaluated.

**Settings (T-257, ADR-017).** `ferrite_model::settings` (persisted choices, layered env, `connect`, `list_models`), `SecretVault` (writable keyring, in-memory double), `GeminiProvider::list_models`, and `ferrite-ui`'s `settings_panel` (state, transitions, drawer, agent-panel banner, gear icon). The Library's read-only Settings tab was removed and its theme and zoom controls moved into the drawer. **Verified:** 27 `ferrite-model` settings tests and 28 `ferrite-ui` settings tests, offline; `cargo clippy --workspace --all-targets -D warnings`; every drawer state (first run, models loaded, connected, error, light theme, agent banner) and the consent panel rendered from the real `view()` under software rendering and inspected. **Not verified:** a real keyring write, a real provider listing, macOS or Windows rendering.

**Found on the way (T-259):** the Linux keyring backend in use is in memory and is cleared by a reboot, so a Linux user's saved key does not persist. A persistent backend needs `libdbus` and could not be verified here, so the Cargo change was tried, did not build, and was reverted; the drawer and docs now say so.

**Site (T-258).** `site/` (landing page and nine docs pages, stdlib-only builder with a link and anchor check, self-hosted font) and `.github/workflows/pages.yml` (manual). Figures are from `docs/EVALUATION.md` §8, and the headline's `RejectFlagged` assumption is stated beside them. **Verified:** the build and link check; the pages viewed in Chromium at desktop and mobile widths in both themes; the download resolver against a stubbed release and a 404. **Not verified:** the deployed site, and the live GitHub API.

**README:** the Status bullets that said the live app runs on the pre-rebuild path and that the corpus has 29 cases were out of date (T-224 and T-220's live-app half are closed; the corpus is 938); corrected from the ledger.

## 2026-10-01 — coordinator — macOS CI: a race on the shared activity trace (T-260)

The first macOS run of the Settings branch failed one `ferrite-ui` test (`opening_the_audit_panel_loads_the_trace_...`): the marker it recorded was gone when it read the global trace back. A sibling test sends `ClearTrace`, which empties that same log, so the two race. Pre-existing and unrelated to Settings; macOS scheduling exposed it. Reproduced (5/60 runs failed), fixed with a shared lock over the three tests that touch the global trace, 0/150 after. **Not verified:** a macOS run after the change.

## 2026-10-01 — coordinator — Bot detection: what was fixable, what was not, and the sign-in handoff (T-262, T-263)

**Asked by the owner:** make the agent hand sign-in to the person, and fix Google and GitHub flagging Ferrite "for everything, anyhow".

**Found:** with the agent idle, a page sees nothing Ferrite-specific (probed in the real engine); gating the agent's tools on the panel would change nothing. The signals were the engine's: a `Servo/<version>` User-Agent and missing Web APIs.

**Done:** WebGL and WebGL2 (the `servo` crate's `webgl` feature plus `dom_webgl2_enabled`), `navigator.permissions`, `Notification`, `navigator.clipboard` (preferences), all required checks in `probe-web-api` (passes; input probe also passes); a Browser identity setting, default Firefox-compatible; the page script refuses to type into sensitive fields; the sign-in handoff (card rendered and inspected). **Verified:** 1,191 workspace tests, clippy for the workspace and with the `servo` feature, fmt, `cargo deny`, `cargo machete`.

**Deliberately not done, and why:** forging hardware, canvas, renderer, plugin or client-hint values, pretending to be Chrome, hiding automation, mimicking another browser's TLS. These are evasion rather than compatibility (ADR-018). **Not verified:** whether Google or GitHub now accept a sign-in; this environment cannot reach either.

## 2026-10-01 — coordinator — Broken layout and icons on GitHub: reproduced and partly fixed (T-264)

**Reported by the owner (with a screenshot and a console log):** Google still blocks sign-in; GitHub signs in but its CSS and layout are broken, and many sites look wrong.

**Found, by reproducing in the real engine:** inline SVG icons coloured by CSS paint black (Servo paints from the element's own markup only), which is the empty icon boxes on a dark theme; and `adoptedStyleSheets` and `document.fonts.load` were missing (both in the owner's console). **Done:** an SVG compatibility script injected through `UserContentManager` (read-all-then-write, 40 per slice), `adoptedStyleSheets`, FontFace and `attr()` enabled. **Tried and rejected after measuring:** container queries, writing modes (panics), multi-column. **Verified:** a pixel check in `probe-web-api` that fails with the script off and passes with it, 12 colouring cases, 602 script-inserted icons on a dark page, the input, profile (set then get) and digest probes, 1,191 workspace tests, clippy for the workspace and with the `servo` feature, fmt, `cargo deny`, `cargo machete`. **Not fixed, no switch exists:** `aspect-ratio` on block boxes (computes to 0 height), `:has()`, `@container`, `mask-image`, `backdrop-filter`, subgrid and more (T-264 has the list). **Not verified:** GitHub itself. Google sign-in is unchanged and unverified (T-262).

