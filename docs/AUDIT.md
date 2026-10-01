# Ferrite Rebuild — Agent A0 Demolition Plan

> Produced by Agent A0 (Archivist) per `docs/REBUILD_DIRECTIVE.md` §6/A0.
> One line per file or file-group. Verdicts: `keep` (unmodified, feeds a later
> agent as reference or is still load-bearing) / `rewrite` (content survives,
> form doesn't) / `archive` (moved to `docs/archive/`, not deleted) / `delete`
> (removed outright, git history is the archive).
>
> **Per §15: this is shown before any deletion executes. Nothing below has
> been acted on yet.**

---

## 0. Structural decision this audit surfaces but does NOT resolve

The whole Rust workspace currently lives under `Browser/` (`Browser/Cargo.toml`,
`Browser/crates/...`), but the directive's §4 target tree puts `crates/`,
`Cargo.toml`, `justfile`, `docs/` at the **repo root** with no `Browser/`
nesting. De-nesting is a real move (CI path filters, devcontainer paths, every
relative path in every crate's tests) and touches build foundations, not
archaeology. It belongs to **A1 (Foundation)**, not A0. This audit assumes
`Browser/` stays put for now; A1's charter should explicitly decide de-nest
vs. keep-nested before writing the workspace `Cargo.toml`. Flagging so it
isn't silently decided either way.

---

## 1. Root level

| Path | Verdict | Reason |
|---|---|---|
| `.DS_Store` | delete | macOS artifact, not gitignored at repo root (Browser/.DS_Store and paper/.DS_Store exist too — same verdict, all three) |
| `.gitignore` | rewrite | Keep the good parts (Rust/LaTeX/Python/OS entries, the `gemini_key.txt` rule). Add `Browser/target/` variants for wherever the workspace ends up post-A1, add corpus DB files, add `~/.cache` is external so N/A, add secret-scan-adjacent entries per R11 hook. Remove the `Browser/staged/`/`Executables/` lines if those paths no longer exist post-rebuild. |
| `EVALUATION_PLAN.md` | archive → extract | Real design content (§2 threat model, §3 corpora, §5 metrics, §7 schema) is the direct ancestor of `docs/EVALUATION.md` §13 in the directive. Pull every still-true decision into `docs/DECISIONS.md` as dated ADRs (A0 charter says do this from `FINALIZED_DECISIONS.md`; this file feeds the same pass), then move the original to `docs/archive/`. |
| `FINALIZED_DECISIONS.md` | archive → extract | Decisions 1–6 are the real ancestor of the capability/primitive taxonomy in directive §8 and the comparator rewrite in A7. Extract as numbered ADRs into `docs/DECISIONS.md`, archive the original. This is the single highest-value extraction in the whole audit — do not skip it or A7 loses its rationale trail. |
| `PROJECT_REFERENCE.md` | archive | Superseded by `docs/ARCHITECTURE.md` (new, written by A0/A1). Contains the stale "six crates" / "dataset pipeline is a stub" claims (confirmed false against source — `dataset.rs` is 673 lines, fully implemented). Historical value only. |
| `paper/` (whole directory) | archive, untouched | Directive §0 explicit: "Out of scope for this rebuild: the paper... Archive that tracker." Move to `docs/archive/paper/` as one unit, no edits, no read-through needed beyond confirming it's LaTeX + a status tracker (confirmed: `main.tex`, 8 `sections/*.tex`, `bibliography.bib`, plot scripts, csv data, `PAPER_STATUS.md`). |
| `Resources/*.pdf` (9 files) | archive, low priority | Pre-2026-04 planning PDFs (`03 capability broker.pdf`, `04 07 policy sandbox audit governance.pdf`, etc.) describing the dead architecture. Not auto-ingested by any tool (unlike `.rules`), so no context-poisoning risk — no urgency. Move to `docs/archive/planning-pdfs/` whenever convenient; not a P0 blocker. |
| `.devcontainer/devcontainer.json` | rewrite | Remove the `9222` port forward (D-list item, dead JSON-RPC-agent leftover). Re-point `workspaceFolder`/mounts if A1 de-nests `Browser/`. |
| `.devcontainer/Dockerfile` | rewrite | **Not just the port forward.** Line 50 installs `wasm32-unknown-unknown` (dead Wasm/Extism sandbox target) and lines 58/67 literally `COPY` and `cargo fetch` `ferrite-capability-broker` by path. If `ferrite-capability-broker` is deleted per §1 below and this file isn't fixed, the devcontainer build breaks on a missing path. Must be edited in the same pass as the crate deletion, not left for later. |
| `.devcontainer/.dockerignore` | keep | No dead-architecture references found; revisit only if paths change under A1 de-nesting. |
| `.github/workflows/ci.yml` | rewrite (A1 scope) | `paths:` filter is `Browser/**` only — breaks silently if A1 de-nests. Needs the `justfile`-as-command-surface rework, `cargo-deny`/`cargo-machete` gates, and the "Servo weekly not every push" split from directive §6/A1. Not touched in A0; flagged so A1 doesn't rediscover it from scratch. |

---

## 2. `Browser/` level

| Path | Verdict | Reason |
|---|---|---|
| `Browser/.DS_Store` | delete | Same as root. |
| `Browser/.gitignore` | rewrite | `*.txt` is a broad blanket ignore inside a Rust workspace — would silently swallow any future `.txt` fixture. Narrow it. Otherwise fine (`target`, `.claude`, binary name `ferrite` all correctly ignored). |
| `Browser/.rules` | **delete** | Directive §4 explicit. Entire file describes the dead capability-broker architecture (Ed25519 tokens, Regorus policy hierarchy, Merkle-tree audit, `ferrite-types`/`ferrite-broker`/`ferrite-network`/`ferrite-a11y`/`ferrite-cef` crates that don't exist, JSON-RPC agent on `ws://localhost:9222`). Dotfile — highest context-poisoning risk in the repo (auto-ingested by rules-reading AI tools), confirmed nothing in it matches current reality. D12. |
| `Browser/Cargo.lock` | keep | Committed deliberately (resolves a `sea-query-rusqlite`/`libsqlite3-sys` conflict per PROGRESS.md history) — a binary workspace correctly commits its lockfile. Carries forward regardless of A1's dependency-table rewrite. |
| `Browser/Cargo.toml` | rewrite (A1 scope) | Needs `[workspace.dependencies]` consolidation (directive §7.3), and membership shrinks by 3 once the dead crates are deleted below. Not touched in A0 beyond removing the deleted crates' entries if any exist (checked: dead crates were already outside workspace `members`, so no edit needed here for deletion — confirmed via original `members = [...]` list, which never included `ferrite-capability-broker`/`ferrite-policy`/`ferrite-sandbox`). |
| `Browser/CLAUDE.md` | rewrite | **D11.** Currently contains a "Planned Near-Term Work (not yet implemented)" section listing 5 items that were actually implemented in commit `46b2177` — the same commit that touched this file. Rewrite to the directive §12 contract: ≤120 lines, zero status claims, pointers only to `docs/PROGRESS.md`/`docs/TO-DO.md`/`docs/ARCHITECTURE.md`, plus the literal line forbidding a "not yet implemented" section from ever reappearing. |
| `Browser/commands.md` | **delete** | Directive §4 explicit. Superseded by the `justfile` (A1). Also currently instructs building `ferrite-capability-broker` and `extensions/hello-ext` as if both are live — both are being deleted. |
| `Browser/PROGRESS.md` | archive old, replace with empty | A0 charter: new `docs/PROGRESS.md` starts empty with a format-contract header. The *habit* in the old file (dated entries citing exactly what changed, including team-authored known-issues) is good and should be the template the new header describes — but the 1851 lines of history are archived, not carried line-by-line, since most entries describe the pre-rebuild implementation this rebuild is replacing. |
| `Browser/README.md` | rewrite | **D12.** Describes the dead broker/Wasm-sandbox architecture as current ("Capability Broker is the single privileged boundary," Extism sandbox, Rego policies). Rewrite to directive §4: "what it is TODAY, 1 page." |
| `Browser/TO-DO.md` | archive old, regenerate | A0 charter: rebuild from scratch as `docs/TO-DO.md` with stable `T-###` IDs derived from the §9 defect register (D1–D14) + the §6 agent charters (A1–A13). The old file (5909 lines) is genuinely the most current/accurate doc in the repo today — archive it intact as reference material for whoever writes the new task list, don't discard the institutional knowledge in it. |

---

## 3. Crates — dead architecture (delete outright)

| Path | Verdict | Reason |
|---|---|---|
| `crates/ferrite-capability-broker/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 explicit. Not a workspace member, already dormant. Only consumer is `ferrite-sandbox` (also being deleted) and the Dockerfile (being rewritten in the same pass, see §1 above). |
| `crates/ferrite-policy/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 explicit. Regorus/Rego policy engine — no live consumer. |
| `crates/ferrite-sandbox/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 explicit. Wasm/Extism extension sandbox; depends on `ferrite-capability-broker` (also deleted) — must go in the same commit or the tree won't `cargo metadata` clean mid-deletion. |
| `extensions/hello-ext/` (Cargo.toml, Cargo.lock, src/lib.rs) | **delete** | Not named explicitly in the directive's delete list, but belongs to the same dead architecture: a Wasm guest module that calls `host_dom_read`/`host_network_fetch`/`host_storage_read`/`host_js_execute` — the Extism sandbox's own demo extension, exercised only through `ferrite-sandbox` (deleted above) and referenced only from `commands.md`/`README.md`/`TO-DO.md` (all being rewritten or archived) and `ferrite-sandbox/src/lib.rs` (deleted). No reference from any live crate. Opts itself out of the Cargo workspace already (its own `[workspace]` stanza), so deleting it doesn't touch `Cargo.toml` membership. |

---

## 4. Crates — live, kept as reference material (not modified in A0)

Mission statement is explicit: rebuild from these as reference, not as a patch base. None of the following are touched in P0; each is annotated with which later agent charter consumes it and which defect(s) it carries forward.

| Path | Verdict | Feeds | Carries forward |
|---|---|---|---|
| `crates/ferrite-shell/` (Cargo.toml, src/main.rs) | keep, reference | A9 (`ferrite-cli`) | CLI dispatch mixes UI launch, smoke tests, and a `jstest` Servo probe in one `main()` match — the new `ferrite-cli` separates these. |
| `crates/ferrite-servo/` (Cargo.toml, lib.rs, session.rs, shell.rs) | keep, reference | A9 (`ferrite-engine-servo`) | `HeadlessServoSession` (software-rendering, no GPU/window handle) is the right shape for a feature-gated engine backend; carries forward largely as-is behind the new `BrowserEngine` trait. |
| `crates/ferrite-ui/` (Cargo.toml, src/lib.rs, 2389 lines) | keep, reference | A10 | Consent flow (`ConsentRequired`/`ConsentDecision`/per-tool approve-reject) already matches the directive's A10 security requirements structurally — largest carry-forward of any crate. Needs the UI-state-machine test suite the current code lacks. |
| `crates/ferrite-agent/` (Cargo.toml, src/lib.rs, src/gemini.rs) | keep, reference | A3 (`ferrite-model`, Gemini backend) + A9 (agent loop) | `BrowserTool`/`AgentRuntime`/`ToolExecutor` trait shapes are sound; Gemini function-calling loop becomes one `ModelProvider` impl instead of the whole agent. Key-loading (`read_api_key()`, env-then-file) is genuinely good design — carries forward as the pattern for `OLLAMA_API_KEY` handling in A3, per directive §10.1. |
| `crates/ferrite-audit-log/` (Cargo.toml, src/lib.rs) | keep, reference | A8 (`ferrite-audit`) | **D5** lives here: hash preimage excludes `capability`/`url`, so the `exec_id`/`case_id` payload on eval entries isn't covered by `verify_chain()`. Chain structure (SHA-256, sequence-linked, SQLite-persisted) is otherwise sound and carries forward; A8 fixes the preimage, not the structure. |
| `crates/ferrite-ipi/` (comparator.rs, containment.rs, dataset.rs, dry_run.rs, lib.rs, sanitizer.rs, tool_decision/mod.rs, twin.rs) | keep, reference | A4–A8 | Carries the most defects of any crate — by design, since this is the security-critical core: **D1/D2** in `comparator.rs` (single global `OriginScope`, `admission_rank()` computed and never consumed), **D3** in `sanitizer.rs`/`dry_run.rs` (`strip_enabled` wired but never activated), **D7** in `containment.rs` (`intercept_request()` unreachable outside its own test — containment is by mock, not interception), **D8** in `twin.rs` (hardcoded AES key `ferrite-ipi-twin-dev-key-32byte!`). The keyword-based `rule_based_must_use()` and the regex `general_injection_patterns()` are legitimate reference material for A4/A5 even though both get reshaped (rules → typed capability taxonomy, regexes → versioned `PatternSet`). |
| `crates/ferrite-eval/` (Cargo.toml, src/{lib,harness,adjudication,corpus}.rs, tests/pilot_w6.rs, tests/pilot_corpus/*.json + `CORPUS_AUTHORING_GUIDE.md` + `PILOT_CORPUS_REFERENCE.md`) | keep, reference | A11 (dataset) + A12 (harness/adjudication) | **D4** lives in `adjudication.rs` (`final_outcome` in On-mode ignores `sanitizer_caught`). **D6** lives in `corpus.rs`/`dataset.rs` (`CarrierVector` partition validated only at JSON-load, not by the type system). **D9**: `Tier3AgentDojo`/`RunLabel::R9` are enum plumbing with zero adapter code behind them — A11 must implement the mapping or delete the variants, no placeholder left standing. The 10 pilot corpus JSON cases + the authoring guide are genuine, reusable assets — this is real authored content, not scaffolding, and should seed the real corpus in A11 rather than being regenerated from zero. Confirmed Servo-free (`Cargo.toml` deps: `ferrite-ipi`, `ferrite-audit-log`, `ferrite-agent` only) — matches the directive's engine-optional principle already. |

---

## 5. Summary counts

- **Delete:** 4 crate-directories (`ferrite-capability-broker`, `ferrite-policy`, `ferrite-sandbox`, `extensions/hello-ext`) + 2 files (`Browser/.rules`, `Browser/commands.md`) + 3 stray `.DS_Store`.
- **Rewrite:** `Browser/CLAUDE.md`, `Browser/README.md`, `.devcontainer/devcontainer.json`, `.devcontainer/Dockerfile`, `.gitignore` (both), `.github/workflows/ci.yml` (deferred to A1).
- **Archive:** `EVALUATION_PLAN.md`, `FINALIZED_DECISIONS.md`, `PROJECT_REFERENCE.md`, `Browser/PROGRESS.md`, `Browser/TO-DO.md`, `paper/` (whole dir), `Resources/*.pdf`.
- **Keep as reference (untouched in P0):** all 7 live crates' source — every defect in them is a *later* agent's job (A4–A12), not A0's.
- **Open structural question, not resolved here:** `Browser/`-nesting vs. repo-root workspace — hand to A1.

Nothing above has been executed. Awaiting go-ahead before deletions/archiving/rewrites proceed.
