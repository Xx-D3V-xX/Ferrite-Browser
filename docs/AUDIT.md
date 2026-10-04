# Ferrite Rebuild — Agent A0 Demolition Plan

> **Status note (2026-10-04): this plan was carried out in A0. It is kept as the
> record of that work. It does not describe the current state.** These things
> happened:
>
> - The workspace was flattened to the repo root.
> - The dead crates, `.rules` and `commands.md` were deleted.
> - The old docs were archived under `docs/archive/`.
> - `CLAUDE.md`, `README.md`, the devcontainer files and the CI workflow were
>   rewritten.
>
> See `docs/TO-DO.md` T-011, T-012, T-013 and T-014. See also the 2026-09-18 A0 entry
> of `docs/PROGRESS.md`. The sentence quoted below, "Nothing below has been acted on
> yet", was true at the moment it was written. The crate and file names in the tables
> are the names from before the rebuild. Terms are explained in
> [`GLOSSARY.md`](GLOSSARY.md).

> Agent A0 (the Archivist) made this file, as `docs/REBUILD_DIRECTIVE.md` §6/A0 asks.
> It has one line per file or file group. These are the verdicts:
>
> - `keep`: not changed. It feeds a later agent as reference, or it is still needed.
> - `rewrite`: the content stays, but the form changes.
> - `archive`: moved to `docs/archive/`, not deleted.
> - `delete`: removed completely. Git history is the archive.
>
> **By §15, this plan is shown before any deletion runs. Nothing below has been
> acted on yet.**

---

## 0. Structural decision this audit surfaces but does NOT resolve

The whole Rust workspace now lives under `Browser/`. That means `Browser/Cargo.toml`
and `Browser/crates/...`. But the directive's §4 target tree puts `crates/`,
`Cargo.toml`, `justfile` and `docs/` at the **repo root**, with no `Browser/` nesting.

Removing the nesting is a real move. It touches CI path filters, devcontainer paths,
and every relative path in every crate's tests. It changes build foundations. It is
not archaeology. So it belongs to **A1 (Foundation)**, not A0.

This audit assumes `Browser/` stays where it is for now. Before A1 writes the
workspace `Cargo.toml`, A1's charter should decide on purpose: remove the nesting, or
keep it. This is flagged here so that nobody decides it silently either way.

---

## 1. Root level

| Path | Verdict | Reason |
|---|---|---|
| `.DS_Store` | delete | It is a macOS artifact. It is not gitignored at the repo root. `Browser/.DS_Store` and `paper/.DS_Store` exist too. The verdict is the same for all three. |
| `.gitignore` | rewrite | Keep the good parts: the Rust, LaTeX, Python and OS entries, and the `gemini_key.txt` rule. Add `Browser/target/` variants for wherever the workspace ends up after A1. Add corpus DB files. (`~/.cache` is outside the repo, so it does not apply.) Add entries that sit next to secret-scanning, per the R11 hook. Remove the `Browser/staged/` and `Executables/` lines if those paths no longer exist after the rebuild. |
| `EVALUATION_PLAN.md` | archive → extract | It has real design content. Its §2 threat model, §3 corpora, §5 metrics and §7 schema are the direct ancestor of §13 of `docs/EVALUATION.md` in the directive. Pull every decision that is still true into `docs/DECISIONS.md` as dated ADRs. (The A0 charter says to do this from `FINALIZED_DECISIONS.md`. This file feeds the same pass.) Then move the original to `docs/archive/`. |
| `FINALIZED_DECISIONS.md` | archive → extract | Decisions 1–6 are the real ancestor of two things. One is the capability and primitive taxonomy in directive §8. The other is the comparator rewrite in A7. Extract them as numbered ADRs into `docs/DECISIONS.md`. Archive the original. This is the single most valuable extraction in the whole audit. Do not skip it, or A7 loses its trail of reasons. |
| `PROJECT_REFERENCE.md` | archive | `docs/ARCHITECTURE.md` replaces it. (That file is new. A0 or A1 writes it.) It holds the stale claims "six crates" and "dataset pipeline is a stub". Both were confirmed false against the source: `dataset.rs` is 673 lines and fully implemented. It has historical value only. |
| `paper/` (the whole directory) | archive, untouched | Directive §0 says so directly: "Out of scope for this rebuild: the paper... Archive that tracker." Move it to `docs/archive/paper/` as one unit. Make no edits. No read-through is needed beyond confirming that it is LaTeX plus a status tracker. That was confirmed: `main.tex`, 8 `sections/*.tex`, `bibliography.bib`, plot scripts, csv data and `PAPER_STATUS.md`. |
| `Resources/*.pdf` (9 files) | archive, low priority | These are planning PDFs from before 2026-04. They describe the dead architecture. Examples are `03 capability broker.pdf` and `04 07 policy sandbox audit governance.pdf`. No tool reads them on its own (unlike `.rules`). So they carry no risk of misleading AI tools, and there is no urgency. Move them to `docs/archive/planning-pdfs/` when it is convenient. This does not block P0. |
| `.devcontainer/devcontainer.json` | rewrite | Remove the `9222` port forward. It is an item on the D-list. It is a leftover of the dead JSON-RPC agent. Point `workspaceFolder` and the mounts at the new place if A1 removes the `Browser/` nesting. |
| `.devcontainer/Dockerfile` | rewrite | **It is not only the port forward.** Line 50 installs `wasm32-unknown-unknown`. That is the target of the dead Wasm/Extism sandbox. Lines 58 and 67 literally `COPY` and `cargo fetch` `ferrite-capability-broker` by path. Suppose `ferrite-capability-broker` is deleted, as §1 below says, and this file is not fixed. Then the devcontainer build breaks on a missing path. Edit this file in the same pass as the crate deletion. Do not leave it for later. |
| `.devcontainer/.dockerignore` | keep | No reference to the dead architecture was found. Look at it again only if paths change when A1 removes the nesting. |
| `.github/workflows/ci.yml` | rewrite (A1 scope) | The `paths:` filter is `Browser/**` only. It breaks without any warning if A1 removes the nesting. It needs these changes: the `justfile` as the command surface, the `cargo-deny` and `cargo-machete` gates, and the "Servo weekly, not every push" split from directive §6/A1. A0 did not touch it. It is flagged here so A1 does not have to find the same things again. |

---

## 2. `Browser/` level

| Path | Verdict | Reason |
|---|---|---|
| `Browser/.DS_Store` | delete | Same as at the root. |
| `Browser/.gitignore` | rewrite | `*.txt` is a broad blanket ignore inside a Rust workspace. It would silently swallow any future `.txt` fixture. Make it narrower. Otherwise it is fine: `target`, `.claude` and the binary name `ferrite` are all correctly ignored. |
| `Browser/.rules` | **delete** | Directive §4 says so directly. The whole file describes the dead capability-broker architecture. It names Ed25519 tokens, the Regorus policy hierarchy and a Merkle-tree audit. It names the crates `ferrite-types`, `ferrite-broker`, `ferrite-network`, `ferrite-a11y` and `ferrite-cef`, which do not exist. It names a JSON-RPC agent on `ws://localhost:9222`. It is a dotfile, and AI tools that read rules files load it on their own. So it is the riskiest file in the repo for misleading them. Nothing in it matches reality today. This is D12. |
| `Browser/Cargo.lock` | keep | It is committed on purpose. It settles a conflict between `sea-query-rusqlite` and `libsqlite3-sys`, per the history in `PROGRESS.md`. A binary workspace correctly commits its lockfile. It carries forward, whatever A1 does to the dependency table. |
| `Browser/Cargo.toml` | rewrite (A1 scope) | It needs its dependencies merged into `[workspace.dependencies]` (directive §7.3). The list of members shrinks by 3 once the dead crates below are deleted. A0 did not touch it, except to remove the entries of the deleted crates if there are any. A0 checked: the dead crates were already outside the workspace `members`. So no edit was needed here for the deletion. This was confirmed against the original `members = [...]` list. It never included `ferrite-capability-broker`, `ferrite-policy` or `ferrite-sandbox`. |
| `Browser/CLAUDE.md` | rewrite | **D11.** It holds a section called "Planned Near-Term Work (not yet implemented)". The section lists 5 items. All 5 were in fact implemented in commit `46b2177`. That commit also touched this file. Rewrite the file to the contract in directive §12. The contract is: at most 120 lines, no status claims at all, pointers only to `docs/PROGRESS.md`, `docs/TO-DO.md` and `docs/ARCHITECTURE.md`, and the literal line that forbids a "not yet implemented" section from ever coming back. |
| `Browser/commands.md` | **delete** | Directive §4 says so directly. The `justfile` (A1) replaces it. It also tells the reader to build `ferrite-capability-broker` and `extensions/hello-ext` as if both were live. Both are being deleted. |
| `Browser/PROGRESS.md` | archive old, replace with empty | The A0 charter says the new `docs/PROGRESS.md` starts empty, with a header that sets the format. The *habit* in the old file is good. It is dated entries that cite exactly what changed, including known issues written by the team. The header should describe it as the template. But the 1851 lines of history are archived, not carried over line by line. Most entries describe the implementation from before the rebuild, which this rebuild replaces. |
| `Browser/README.md` | rewrite | **D12.** It describes the dead broker and Wasm-sandbox architecture as if it were current ("Capability Broker is the single privileged boundary", the Extism sandbox, Rego policies). Rewrite it to directive §4: "what it is TODAY, 1 page." |
| `Browser/TO-DO.md` | archive old, regenerate | The A0 charter says to rebuild it from scratch as `docs/TO-DO.md`. It gets stable `T-###` IDs. They come from the §9 defect register (D1–D14) and the §6 agent charters (A1–A13). The old file (5909 lines) is truly the most current and most accurate doc in the repo today. Archive it whole. It is reference material for whoever writes the new task list. Do not throw away the knowledge that is in it. |

---

## 3. Crates — dead architecture (delete outright)

| Path | Verdict | Reason |
|---|---|---|
| `crates/ferrite-capability-broker/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 says so directly. It is not a workspace member. It is already dormant. Its only user is `ferrite-sandbox`, which is also being deleted. The other user is the Dockerfile, which is being rewritten in the same pass (see §1 above). |
| `crates/ferrite-policy/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 says so directly. It is the Regorus/Rego policy engine. It has no live user. |
| `crates/ferrite-sandbox/` (Cargo.toml + src/lib.rs) | **delete** | Directive §4 says so directly. It is the Wasm/Extism extension sandbox. It depends on `ferrite-capability-broker`, which is also deleted. Both must go in the same commit. Otherwise the tree will not pass `cargo metadata` cleanly in the middle of the deletion. |
| `extensions/hello-ext/` (Cargo.toml, Cargo.lock, src/lib.rs) | **delete** | The directive's delete list does not name it directly. But it belongs to the same dead architecture. It is a Wasm guest module. It calls `host_dom_read`, `host_network_fetch`, `host_storage_read` and `host_js_execute`. That is the demo extension of the Extism sandbox itself. Only `ferrite-sandbox` (deleted above) runs it. Only these files mention it: `commands.md`, `README.md` and `TO-DO.md` (all being rewritten or archived), and `ferrite-sandbox/src/lib.rs` (deleted). No live crate refers to it. It already opts out of the Cargo workspace, with its own `[workspace]` stanza. So deleting it does not touch the `Cargo.toml` members. |

---

## 4. Crates — live, kept as reference material (not modified in A0)

The mission statement is clear. Rebuild from these as reference. Do not use them as a
base for patches. Nothing below is touched in P0. Each row says which later agent
charter uses it, and which defects it carries forward.

| Path | Verdict | Feeds | Carries forward |
|---|---|---|---|
| `crates/ferrite-shell/` (Cargo.toml, src/main.rs) | keep, reference | A9 (`ferrite-cli`) | The CLI dispatch mixes three things in one `main()` match: starting the UI, the smoke tests, and a `jstest` Servo probe. The new `ferrite-cli` separates them. |
| `crates/ferrite-servo/` (Cargo.toml, lib.rs, session.rs, shell.rs) | keep, reference | A9 (`ferrite-engine-servo`) | `HeadlessServoSession` (software rendering, no GPU or window handle) has the right shape for an engine backend that sits behind a feature gate. It carries forward mostly as it is, behind the new `BrowserEngine` trait. |
| `crates/ferrite-ui/` (Cargo.toml, src/lib.rs, 2389 lines) | keep, reference | A10 | The consent flow (`ConsentRequired`, `ConsentDecision`, approve or reject per tool) already matches the security needs of directive A10 in its structure. It is the largest carry-forward of any crate. It needs the UI state-machine test suite that the current code lacks. |
| `crates/ferrite-agent/` (Cargo.toml, src/lib.rs, src/gemini.rs) | keep, reference | A3 (`ferrite-model`, Gemini backend) + A9 (agent loop) | The trait shapes `BrowserTool`, `AgentRuntime` and `ToolExecutor` are sound. The Gemini function-calling loop becomes one `ModelProvider` implementation. It stops being the whole agent. The key loading (`read_api_key()`, environment first, then file) is truly good design. It carries forward as the pattern for handling `OLLAMA_API_KEY` in A3, per directive §10.1. |
| `crates/ferrite-audit-log/` (Cargo.toml, src/lib.rs) | keep, reference | A8 (`ferrite-audit`) | **D5** lives here. The hash preimage leaves out `capability` and `url`. So the `exec_id` and `case_id` payload on eval entries is not covered by `verify_chain()`. The rest of the chain structure is sound and carries forward. It is SHA-256, linked by sequence number, and saved in SQLite. A8 fixes the preimage, not the structure. |
| `crates/ferrite-ipi/` (comparator.rs, containment.rs, dataset.rs, dry_run.rs, lib.rs, sanitizer.rs, tool_decision/mod.rs, twin.rs) | keep, reference | A4–A8 | It carries the most defects of any crate. That is by design, because it is the security-critical core. **D1/D2** are in `comparator.rs`: there is a single global `OriginScope`, and `admission_rank()` is computed and never used. **D3** is in `sanitizer.rs` and `dry_run.rs`: `strip_enabled` is wired up but never turned on. **D7** is in `containment.rs`: `intercept_request()` cannot be reached from anywhere except its own test. So containment works by mock, not by interception. **D8** is in `twin.rs`: a hardcoded AES key, `ferrite-ipi-twin-dev-key-32byte!`. The keyword-based `rule_based_must_use()` and the regex `general_injection_patterns()` are legitimate reference material for A4 and A5. Both get reshaped anyway: the rules become a typed capability taxonomy, and the regexes become a versioned `PatternSet`. |
| `crates/ferrite-eval/` (Cargo.toml, src/{lib,harness,adjudication,corpus}.rs, tests/pilot_w6.rs, tests/pilot_corpus/*.json + `CORPUS_AUTHORING_GUIDE.md` + `PILOT_CORPUS_REFERENCE.md`) | keep, reference | A11 (dataset) + A12 (harness/adjudication) | **D4** lives in `adjudication.rs`: in On-mode, `final_outcome` ignores `sanitizer_caught`. **D6** lives in `corpus.rs` and `dataset.rs`: the `CarrierVector` partition is checked only when the JSON loads. The type system does not check it. **D9**: `Tier3AgentDojo` and `RunLabel::R9` are only enum plumbing, with zero adapter code behind them. A11 must write the mapping or delete the variants. No placeholder may be left standing. The 10 pilot corpus JSON cases and the authoring guide are real assets that can be reused. This is real authored content, not scaffolding. It should seed the real corpus in A11. It should not be regenerated from zero. It was confirmed Servo-free. Its `Cargo.toml` dependencies are only `ferrite-ipi`, `ferrite-audit-log` and `ferrite-agent`. That already matches the engine-optional principle of the directive. |

---

## 5. Summary counts

- **Delete:** 4 crate directories (`ferrite-capability-broker`, `ferrite-policy`,
  `ferrite-sandbox`, `extensions/hello-ext`), plus 2 files (`Browser/.rules`,
  `Browser/commands.md`), plus 3 stray `.DS_Store`.
- **Rewrite:** `Browser/CLAUDE.md`, `Browser/README.md`,
  `.devcontainer/devcontainer.json`, `.devcontainer/Dockerfile`, `.gitignore` (both),
  `.github/workflows/ci.yml` (put off to A1).
- **Archive:** `EVALUATION_PLAN.md`, `FINALIZED_DECISIONS.md`, `PROJECT_REFERENCE.md`,
  `Browser/PROGRESS.md`, `Browser/TO-DO.md`, `paper/` (the whole directory),
  `Resources/*.pdf`.
- **Keep as reference (untouched in P0):** the source of all 7 live crates. Every
  defect in them is the job of a *later* agent (A4–A12), not of A0.
- **Open structural question, not resolved here:** `Browser/` nesting or a workspace
  at the repo root. Hand it to A1.

Nothing above has been executed. This plan waits for a go-ahead before the deletions,
archiving and rewrites go ahead.
