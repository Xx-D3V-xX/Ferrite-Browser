# Ferrite

Ferrite is a Rust agentic browser, built on the [Servo](https://servo.org/)
engine, that defends AI browser agents against **indirect prompt injection
(IPI)** architecturally rather than by trying to detect malicious text.

## The idea

An AI agent browsing the web reads content it doesn't control. Some of that
content can carry instructions aimed at the agent, not the user — a hidden
`<div>`, an HTML comment, an image `alt` attribute, a field in a tool's JSON
response. Ferrite doesn't try to win the arms race of spotting every phrasing
of an injected instruction. Instead it watches *what the agent does*:

1. **Predict** the task's expected fingerprint — which tools and origins the
   task legitimately needs — from the user's prompt, before any untrusted
   content is read.
2. **Dry-run** the agent's actual plan in a sandbox against synthetic,
   fake-but-plausible data, with no real network reachable.
3. **Compare** what the agent actually did against what was predicted.
4. **Consent-gate** anything beyond the prediction in a trusted UI surface,
   decoupled from page content, before the real run proceeds.

A successful injection can still make the agent *try* something unexpected —
but trying it during a contained dry-run against fake data, gated behind a
consent prompt the user controls, is a very different outcome than trying it
for real. A SHA-256 hash-chained audit log records the security-relevant
events so containment is verifiable after the fact, not just asserted.

## Status

The A0–A13 rebuild plan (`docs/REBUILD_DIRECTIVE.md`) is **complete** as of
A13 (reconciliation & release). That means every charter ran, every defect
in the D1–D14 register was fixed or explicitly ratified as intended
behavior, and `docs/EVALUATION.md` reports real numbers from a real (if
small) corpus with no fabricated data anywhere.

**It does not mean "feature-complete" or "ready to trust in production."**
Read `docs/EVALUATION.md` §5 and §6 before drawing any conclusion from the
eval numbers — in particular:

- The live `ferrite-ui`/`ferrite-shell` app constructs a real `ferrite-model`
  provider at startup and drives the new `ferrite-engine`/`browser_loop`
  stack (`docs/TO-DO.md` T-224, closed). A provider is connected from the
  app's Settings drawer (provider, API key in the OS keyring, models), or from
  the environment as before (`docs/DECISIONS.md` ADR-017).
- The eval corpus is **938 cases** (`docs/EVALUATION.md` §8), up from the 29
  the original run used. The agent in it is scripted, the cases are
  self-authored and template-generated, and no second author exists, so the
  intervals understate the uncertainty (`docs/TO-DO.md` T-227).
- `ServoEngine` (the new, engine-agnostic path's Servo backend) does not yet
  complete a real page load in this environment (`docs/TO-DO.md` T-220),
  though the live app's separate, older Servo integration
  (`ferrite-servo`/`ferrite-shell`) does — confirmed by direct observation.

`docs/PROGRESS.md` is the dated, cited status record; `docs/TO-DO.md` is the
live task ledger (open items, held items, and everything closed with the
commit that closed it); `docs/EVALUATION.md` is the full accounting,
including a `docs/REBUILD_DIRECTIVE.md` §14 definition-of-done checklist
scored item by item. Read those three for current state — not this section,
once time has passed since it was written.

## Site

`site/` holds the marketing site and documentation (`python3 site/build.py`,
see `site/README.md`).

## Workspace

```
crates/
├── ferrite-core           types, IDs, capability/primitive taxonomy, OriginScope, Clock
├── ferrite-model           ModelProvider trait: Mock/Replay/Ollama/Gemini + cache/throttle/budget decorators
├── ferrite-audit-log       SHA-256 hash-chained, SQLite-backed audit log
├── ferrite-ipi             the IPI defense: fingerprint, sanitizer, dry-run, twin, comparator
├── ferrite-engine          BrowserEngine trait + MockEngine (always built, no feature flag)
├── ferrite-engine-servo    ServoEngine, the BrowserEngine impl over real Servo (feature `engine-servo`)
├── ferrite-agent           BrowserTool/AgentRuntime/ToolExecutor (pre-rebuild, still load-bearing) + browser_loop (new, additive)
├── ferrite-servo           the live app's own Servo integration (HeadlessServoSession), feature `servo`
├── ferrite-ui              Iced UI: tabs, address bar, agent sidebar, consent panel
├── ferrite-shell           top-level binary, CLI dispatch
└── ferrite-eval            evaluation harness (Servo-free): dataset schema, corpus, harness, adjudication, metrics
```

Two things worth naming explicitly because they look like duplication and
aren't: `ferrite-engine-servo` and `ferrite-servo` are different, deliberate
things — the former is the new `BrowserEngine`-trait Servo backend (T-220:
real navigation doesn't complete yet), the latter is the live app's own,
working Servo integration that `ferrite-shell`/`ferrite-ui` actually run.
`ferrite-agent` similarly carries both the pre-rebuild `BrowserTool`/
`AgentRuntime` path (still what the live app runs, per T-224) and the new,
additive `browser_loop` module (engine/provider-agnostic, tested, not yet
wired into the live app). The target architecture in
`docs/REBUILD_DIRECTIVE.md` §4 also names a `ferrite-cli` crate; it was
never split out of `ferrite-shell` — `ferrite-shell` remains the one binary.

Servo (`ferrite-servo`'s `servo` feature, or `ferrite-engine-servo`'s
`engine-servo` feature) is not built by default — most development and all
of the IPI defense logic never touches it. See `CLAUDE.md` for build
commands and hard invariants (in particular: no crate outside this list may
be reintroduced without explicit sign-off — see the "Never reference"
section there for what was deliberately deleted and why).

## Building

The `justfile` is the single command surface (`just --list` to see every
recipe). Common ones:

```
just check   # fmt-check + clippy (--all-targets, deny warnings) + unused deps
just test    # full workspace test suite — no network, no API key required (R7)
just eval    # runs the real corpus through the harness, writes the metrics report
just audit   # cargo-deny: licenses, security advisories, duplicate versions
```

Plain `cargo` works too if you don't have `just`:

```
cargo build --workspace
cargo test --workspace
```

Building the real Servo engine is optional and slow (`just build-servo`, or
`cargo build -p ferrite-shell --features ferrite-servo/servo`).

### Local tooling

`rustup` (stable, with the `rustfmt`/`clippy`/`llvm-tools` components —
`rust-toolchain.toml` pins these) plus:

```
brew install just cargo-deny        # or: cargo install just cargo-deny
cargo install cargo-machete --locked
```

`just install-hooks` wires the commit-msg/pre-commit git hooks after that.

## Running locally

macOS first; Linux is best-effort. Every step is idempotent, so run the same
commands again whenever you like. Everything lives inside this checkout: build
output in `target/` and all local state (Laya venv and checkpoint, `env.local`,
logs) in the gitignored `.ferrite/` (override with `$CARGO_TARGET_DIR` /
`$FERRITE_HOME`); `rm -rf .ferrite` removes all local state.

The whole project (real Servo + Laya) in one go, then every day:

```
just setup-all        # real Servo build + Laya venv/checkpoint, all inside this folder
$EDITOR .ferrite/env.local     # set FERRITE_MODEL_SMALL and FERRITE_MODEL_MAIN
export OLLAMA_API_KEY=...      # or store it in the keychain, see below
just run-all          # local Laya server + the real-Servo browser
```

Step by step (`just setup` is the quick Servo-free variant):

```
just setup            # or: ./scripts/setup-local.sh   (add --dry-run --yes to see the plan)
$EDITOR .ferrite/env.local     # set FERRITE_MODEL_SMALL and FERRITE_MODEL_MAIN
security add-generic-password -U -s ferrite -a OLLAMA_API_KEY -w     # prompts; or: export OLLAMA_API_KEY=...
just run-local        # or: ./scripts/run-local.sh
```

`just setup` checks git, Xcode command line tools, Homebrew and the brew
packages CI installs (`cmake pkg-config openssl sqlite`), and rustup. It offers
to install what is missing but only after you say yes (or pass `--yes`), and it
never uses `sudo`. It then builds `ferrite-shell` and sets up Laya (below). API
keys are never written by these scripts: they come from your environment or the
OS keychain, as `crates/ferrite-model/src/secret.rs` requires. No model name is
a default, so `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` must be set in
`env.local` (`just models-local` lists the tags your endpoint serves). A key
stored with `security` may make macOS ask once whether `ferrite-shell` may read
it; a rebuilt binary can trigger that prompt again.

**Servo or not.** The default build is Servo-free: quick (about two minutes
cold, per `docs/BUILD_BUDGET.md`), but it has **no real web rendering**. For
real pages run `just setup-servo` (`cargo build -p ferrite-shell
--features ferrite-servo/servo`; audio and video playback are not built in, so no
extra brew package is needed): plan on 20 to
60 minutes and 10+ GB of disk the first time. `just run-local` uses whichever
build setup made last; force one with `--servo` / `--no-servo`.

**Laya (optional).** [Laya](https://github.com/NandhaKishorM/laya) is a small
local decision model. Given the page state and the candidate elements it scores
"which operation, which element" for the next browser step (its model card
reports tens of milliseconds on a GPU; `just laya-verify` measures yours).
Ferrite talks to it over HTTP only when `FERRITE_LAYA_URL` is
set; with it unset the app runs exactly as before, LLM only. `just setup`
creates a Python venv under `$FERRITE_HOME/laya`, installs `laya[serve]`, and
downloads the `cklxx/laya-browser` checkpoint from Hugging Face (`v10s`, the
faster 322M one, by default; `--laya-checkpoint v10` for the 421M one; skip all
of it with `--no-laya`). `just run-local` then starts a local server on
`127.0.0.1:8765` for the session, waits for `/health`, sets `FERRITE_LAYA_URL`,
and stops the server when the app exits. Apple silicon uses the Metal (`mps`)
backend automatically. Two honest notes: the browser checkpoint is mounted in
Laya's `typed-decisions` router slot because Laya has no other slot for a local
model (`scripts/laya/serve.py` explains), and the server serves the
checkpoint's training `head_max_len` (768) as the default, as its model card
requires.

| Command | What it does |
|---|---|
| `just doctor` | checklist: tools, build, env file, model tags, keys (never printed), Laya venv/checkpoint/server with a latency probe, disk. `--fix-hints` says how to fix each item |
| `just laya-serve` | only the Laya server, in the foreground |
| `just laya-verify` | sends one recorded browser step to the running server, prints its decision and latency |
| `just probe-input` / `just probe-engine` / `just probe-profile` / `just probe-web-api` | drive a real headless Servo session against a built-in page and report what works: scrolling, clicks, typing, reload, two tabs; the page digest and `@ref` actions; cookies and storage surviving a restart; the Web APIs benchmarks and frameworks assume, `window.crypto` first among them (real Servo build only; on Linux run under `xvfb-run`) |
| `just crash-report` | after a crash (exit 139): prints the exception and the crashing thread's stack from the newest macOS crash report, for bug reports |
| `just test-local` | tests for these scripts (no network; the Python ones skip unless `fastapi`, `uvicorn` and `laya` import) |

Settings live in `$FERRITE_HOME/env.local` (plain `KEY=VALUE`, read as data,
never sourced; only `FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and `RUST_LOG` are
honoured; anything already exported in your shell wins). `scripts/local.env.example`
documents every knob, including `FERRITE_LAYA_URL` (use a Laya server you run
yourself), `FERRITE_LAYA_HOST`/`PORT`, `FERRITE_LAYA_CHECKPOINT`,
`FERRITE_LAYA_DEVICE` and `FERRITE_LAYA_API_KEY`. The Laya server binds
loopback only and refuses any other address unless an API key is set.

## Watching what the agent, the LLM and Laya do

Open **Audit** (toolbar, or `F12`). The **Model calls** view is a timeline of
every call: what the agent ran, every LLM request with its full prompt and
answer, every Laya request, and whether Laya's answer was used or sent back to
the LLM, each with how long it took. Click a row to see what was sent and what
came back. A one-line summary at the top answers "is Laya making it faster?"
from the timings it has seen: what asking Laya cost in total (failed and
declined calls included) against the LLM time its used answers skipped. If Laya
is slower than the LLM here it pauses itself for a few steps and says so, and
the server logs the device it runs on at start-up (a CPU is not a fast lane).
The same events are appended to
`$FERRITE_HOME/logs/model-activity.jsonl` (prompts and page text included, so
treat it like the pages themselves; it never leaves your machine). The
**Security log** view is the hash-chained record of network requests.

## Evaluating the defense

`just eval` runs the 938-case corpus through four defense modes and writes
`target/eval-report/EVAL_REPORT.md` (no network, no API key). `just guard-eval`
measures what the real run does when the dry run could not have seen the attack
(the live app's situation), where the runtime guard blocks any action outside
the predicted fingerprint that you did not approve. 909 of the cases are
generated by `scripts/gen_redteam_corpus.py` (`just redteam-corpus`) from a
matrix of tasks, attacker goals, carriers, payload disguises and scopes; they
are self-authored, so read the intervals as lower bounds on the uncertainty.
The component red-team suites are `cargo test -p ferrite-ipi --test
red_team_sanitizer` and `cargo test -p ferrite-core --test red_team_scope`.
`docs/EVALUATION.md` §8 has the numbers and, more usefully, the limits.

## Logins and cookies

The browser profile (cookies, HSTS, saved HTTP credentials, web storage) lives
in `$FERRITE_HOME/profile` and survives restarts. It is written when the window
is closed normally; a crash, `kill` or macOS Cmd+Q loses that run's new
cookies. Whether a particular site lets you sign in is a separate question:
Google in particular may refuse an embedded engine. `FERRITE_USER_AGENT`
overrides the user-agent string.
