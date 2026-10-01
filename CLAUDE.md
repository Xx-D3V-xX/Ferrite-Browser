# Ferrite — Claude Code Context

Ferrite is a Rust agentic browser built on Servo whose research contribution
is an *architectural* defense against indirect prompt injection: predict the
agent's expected tool/origin fingerprint, dry-run its plan against synthetic
data, compare actual behavior to the prediction, and consent-gate any
deviation before anything real executes. A hash-chained audit log makes the
containment decision verifiable after the fact. The A0–A13 rebuild plan in
`docs/REBUILD_DIRECTIVE.md` is complete — read `docs/TO-DO.md` for the live
task ledger (what's still open) and `README.md`'s Status section for what
"complete" does and doesn't mean before assuming anything works end to end.

**This file must never contain a "planned" or "not yet implemented" list.
Status lives only in `docs/PROGRESS.md` and `docs/TO-DO.md`.** The failure
this rule exists to prevent already happened once: a single commit
implemented five features and edited this file in the same diff while
leaving a "not yet implemented" section standing describing exactly what it
had just built. That was a structural bug in where status lived, not a
one-off mistake — fixed by removing status from this file entirely rather
than by trying to remember to update it.

## Workspace

`crates/` at the repo root (flattened from the prior `Browser/` nesting in
the P0 rebuild — see `docs/DECISIONS.md` ADR history if you find a stale
`Browser/`-prefixed path anywhere and don't know why). `Cargo.toml` at the
repo root is the source of truth for active workspace members — if a crate
isn't listed there, it isn't active, regardless of what any doc says.

**Never reference, import, or revive:** `ferrite-capability-broker`,
`ferrite-policy`, `ferrite-sandbox`, or an Extism/Wasm extension sandbox.
This architecture was deleted, not archived-in-place — it exists only in
git history (pre-`chore: delete dead broker-era crates and extension`) and
in `docs/archive/` for provenance. If a task seems to need one of these,
stop and flag it; do not resurrect the crate.

## Build / test / run

`justfile` at the repo root is the command surface (`just --list` for every
recipe):

```
just check          # fmt-check + clippy --all-targets -D warnings + unused deps
just test           # full workspace test suite (no network, no API key required — R7)
just audit          # cargo-deny: licenses, advisories, duplicate versions
just build-servo    # pulls in the real Servo engine; slow, optional, not part of check/test
```

Plain `cargo` works too if `just` isn't installed: `cargo build --workspace`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all --check`. Note `--all-targets` on clippy — its absence
previously hid a real bug for months (`docs/TO-DO.md` T-207).

## Invariants that must never be violated

- **`js.execute` is unconditionally unscopable.** It can synthesize any
  other primitive past the `ToolExecutor` boundary, so no capability's
  expected realization may ever include it, at any scope. It is always a
  deviation, always consent-gated. See `docs/DECISIONS.md` ADR-003.
- **Fail to empty, never to a bypass.** Any model-provider error, timeout,
  malformed response, or missing API key degrades the fingerprint's may-use
  set to empty — never skips fingerprinting, never widens what's admitted.
  An empty fingerprint routes everything through consent.
- **No live network calls in tests.** Every test must pass with the network
  cable pulled and no provider API key set. Live-provider tests are
  `#[ignore]`d.
- **No AI attribution in commit messages.** Enforced by the commit-msg hook
  in `scripts/hooks/` (installed via `git config core.hooksPath
  scripts/hooks`) — see that file if a commit is being rejected.
- **Dependency direction is strictly downward** per the rebuild directive's
  target architecture (`core ← {model, audit, engine} ← ipi ← agent ← {ui,
  eval, cli}`). No cycles, no new upward imports. T-221 (`ferrite-ipi`
  depending on `ferrite-agent`, backwards) is fixed — closed by B1/B2/B3;
  see `docs/TO-DO.md`'s T-221 row for exactly what changed. This invariant
  now holds everywhere in the workspace; keep it that way.
- **No dead code.** If a function exists, something reachable calls it.
  `OriginScope::admission_rank()` being computed and never consumed by
  `compare()` was exactly this failure mode — see `docs/TO-DO.md` T-001/T-002.
- **rusqlite is one version, workspace-wide, with `features = ["bundled"]`.**
  Do not add `sea-query`, `sea-orm`, `sqlx`, or `diesel` — this exact
  combination previously caused a recurring CI link failure.

## Where things live

- **Design rationale:** `docs/DECISIONS.md` — numbered ADRs, each citing the
  commit where its implementation actually landed.
- **Current status:** `docs/PROGRESS.md` (append-only, dated, cites tests/SHAs).
- **Open tasks:** `docs/TO-DO.md` (stable `T-###` IDs; nothing gets silently dropped).
- **Target architecture:** `docs/ARCHITECTURE.md` — **not yet written** (A1/A2
  deliverable). Until it exists, `docs/DECISIONS.md` plus the crate-level
  doc comments are the closest thing to a design reference.
- **The rebuild plan itself:** `docs/REBUILD_DIRECTIVE.md`.
- **Historical/superseded docs:** `docs/archive/` — banner-marked
  `HISTORICAL — DESCRIBES A PROJECT THAT NO LONGER EXISTS`. Do not use as
  context for current state; see `docs/archive/README.md` for why this
  directory exists and what rule keeps it from becoming the next `.rules`.

## Hard verification rule

Never propose a `Cargo.toml` edit, dependency change, or architecture claim
without first confirming the relevant crate/version/file actually exists in
the workspace as described. If you cannot verify, say so and ask.
