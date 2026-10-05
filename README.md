# Ferrite

Ferrite is a browser written in Rust. An AI agent can use it to browse the web.
It is built on the [Servo](https://servo.org/) engine. Servo is the web engine
that draws the pages.

Ferrite protects the agent from **indirect prompt injection (IPI)**. An
injection is hidden text that tries to give the AI new orders. "Indirect" means
the text sits in a web page or a tool result, not in the user's own prompt.
Ferrite does not try to spot bad text. It limits what the agent may do. That is
what "architecturally" means here: the defense comes from how the system is
built.

New to a term in this file? [`docs/GLOSSARY.md`](docs/GLOSSARY.md) explains
every term in plain words.

## The idea

An AI agent that browses the web reads content it does not control. Some of that
content can hold orders meant for the agent, not for the user. The orders can hide
in many places. One is a `<div>` the user never sees. Others are an HTML comment,
an image `alt` attribute, or a field of a tool's JSON reply. Ferrite does not try to spot
every wording of such an order. Instead it watches *what the agent does*.

1. **Predict** the task's expected fingerprint. The fingerprint is the list of
   tools and websites the task legitimately needs. Ferrite builds it from the
   user's prompt. It does this before it reads any untrusted content.
2. **Dry-run** the agent's real plan. A dry run is a practice run that does
   nothing real. It runs in a sandbox on fake but believable data. No real
   network can be reached.
3. **Compare** what the agent did against what was predicted.
4. **Consent-gate** anything beyond the prediction. A consent gate is a question
   asked to the user before the action runs. The question appears in a trusted
   part of the window. Page content cannot change it. The real run waits for the
   answer.

A successful injection can still make the agent *try* something unexpected. But
it then tries it in a contained dry run on fake data. And a consent prompt that
the user controls guards it. That is very different from trying it for real. The
real run is held to the same prediction. If an action falls outside it, the run
pauses and asks. The check that stops such an action is called the **guard**. A
hash-chained audit log records the events that matter for security. Each entry
is tied to the one before it by a SHA-256 hash. So anyone can check afterwards
that the defense held. It is not just a claim.

## Documentation

| Read | For |
|---|---|
| [`docs/COMMANDS.md`](docs/COMMANDS.md) | **how to set up, run, use, evaluate, diagnose and release Ferrite**: every `just` recipe, script, flag, shortcut and environment variable |
| [`docs/GLOSSARY.md`](docs/GLOSSARY.md) | every term a new reader meets, in plain words |
| [`docs/TO-DO.md`](docs/TO-DO.md) | the live task ledger: what is open, what is done and with which commit |
| [`docs/PROGRESS.md`](docs/PROGRESS.md) | the dated record of what landed, with the tests and commits, and what was not verified |
| [`docs/EVALUATION.md`](docs/EVALUATION.md) | the evaluation method, numbers and limits (§8 red-team corpus and runtime guard, §9 running with a real model, §10 AgentDojo, §11 the real run with `gemma4:31b`) |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | numbered design decisions (ADR-000 to ADR-022). An ADR is one design decision with its reasons |
| [`docs/BUILD_BUDGET.md`](docs/BUILD_BUDGET.md) | measured build time and disk |
| [`docs/HANDOFF-2026-10-03.md`](docs/HANDOFF-2026-10-03.md) | how to resume the current branch |
| [`CLAUDE.md`](CLAUDE.md) | build commands and the rules that must never be broken |
| `site/` | the project site and its documentation pages (`python3 site/build.py`, see `site/README.md`; deployed by the manual Site workflow) |

## Status

As of 2026-10-04. The A0–A13 rebuild plan (`docs/REBUILD_DIRECTIVE.md`) is
**complete**. Every charter (one job per agent in that plan) ran. Every defect in
the D1–D14 register was fixed, or was explicitly accepted as intended behavior.
`docs/EVALUATION.md` reports real numbers from a real corpus. No data is made up
anywhere.

**"Complete" does not mean "feature-complete". It does not mean "ready to trust
in production".** Read `docs/EVALUATION.md` §5, §6 and §8.6 before you draw any
conclusion from the evaluation numbers. In particular:

- **The app runs on the new stack.** `ferrite-ui` and `ferrite-shell` build a
  real `ferrite-model` provider. They drive `ferrite_agent::browser_loop` against
  the real Servo engine. You connect a provider in the app's Settings drawer
  (provider, API key in the OS keyring, models). Or you set it in the environment
  (`docs/DECISIONS.md` ADR-017). Only Ollama (cloud or local) and Gemini exist.
  There is no OpenAI-compatible backend and no Anthropic backend
  (`docs/TO-DO.md` T-277).
- **The real run is guarded too.** The predicted fingerprint is binding on the
  real run. An action outside it pauses and asks you (ADR-014, ADR-020).
- **The offline corpus is 938 cases** (`docs/EVALUATION.md` §8). The script
  `scripts/gen_redteam_corpus.py` generates 909 of them. A corpus is a set of test
  cases. The agent in this corpus is scripted. The cases are self-authored and
  made from templates. No second author exists. So the confidence ranges (the
  intervals) are narrower than the real uncertainty (`docs/TO-DO.md` T-227). A
  further **1,046 AgentDojo cases** are imported (`docs/EVALUATION.md` §10).
  AgentDojo is a public set of agent attack tests.
- **A real model has now been run through the evaluation.** The owner ran all
  1,984 cases with `ollama` and the model `gemma4:31b`, with and without the guard.
  That is 3,968 runs. The full report is
  `docs/results/live-eval-ollama-gemma4-31b.md`. `docs/EVALUATION.md` §11 explains
  it. The headline:
  - With no guard, 31/1542 = 2.0% of the measurable attack runs tried the attack
    and ran it.
  - With the guard, 6/1542 = 0.4% ran the attack. The guard stopped 25 of those 31.
  - With the guard, 86/226 = 38.1% of the normal (benign) tasks had an action
    refused. That is a false positive: the guard stopped something the user
    wanted.
  - The limits: one model, one attack template, a practice engine that does
    nothing real, and a simulated user who refuses everything. A real user who
    approves prompts would move these numbers toward the no-guard result. Do not
    treat them as a general result.

  The live runner is `live_eval` (`docs/EVALUATION.md` §9). Mock-provider tests
  and a loopback fake server also test it. `docs/TO-DO.md` T-275 and T-278 track
  it.
- **Rendering.** Real web pages need the Servo build (`just setup-servo`). The
  default build has no web rendering. The engine is Servo 0.6.0 from crates.io
  plus one vendored patch of one function (`vendor/servo-script`, ADR-022).
  Pages use features that Servo lacks (`:has()`, `@container`, `aspect-ratio` on
  blocks, ...). So some sites look wrong (`docs/TO-DO.md` T-264). The address bar
  searches Google. The agent's own search prompt still uses DuckDuckGo Lite.
  Since 2026-10-03 the engine is told the display scale. Pages are drawn on the
  CPU only. A GPU renderer was tried and removed on 2026-10-05, because on an
  Apple M1 it left Google loading forever and unusable (ADR-021, T-281, T-305).
  Thread pools follow the core count (T-280, T-282).
- **Not verified on the owner's Mac** (as of the dated entries in
  `docs/PROGRESS.md`; the CPU renderer and the Google and GitHub pages have now run
  there). These were checked on Linux only (software rendering, Xvfb):
  - the display-scale fix;
  - the log file's location;
  - quitting;
  - DevTools;
  - the page-control overlays;
  - the resizable panels.

  The rolling
  `latest` release is built from `main`. `main` does not yet contain the
  2026-10-03 and 2026-10-04 work.
- **Known open problems:**
  - GitHub does not open for the owner. The cause is unknown (T-290).
  - Google sign-in (T-267) and the Google results margin (T-291) are unconfirmed.
  - Speed on real hardware is unmeasured (T-296).
  - The Network tab has no status, size or timing (T-292).
  - Windows writes no log file (T-298).

  `docs/TO-DO.md` lists every open item with its commit and what was not
  verified.

`docs/PROGRESS.md` is the dated status record with citations. `docs/TO-DO.md` is
the live task ledger. Once time has passed since this section was written, read
those two files for the current state, not this section.

There is no `LICENSE` file yet. The crate manifests say `MIT OR Apache-2.0`. The
choice is waiting on the owner (`docs/TO-DO.md` T-209).

## Quick start

`just` is the command surface. `just --list` prints every recipe. `docs/COMMANDS.md`
explains each one. Plain `cargo` works too.

```
just setup-all                # real Servo build (20-60 min, 10+ GB first time) + local Laya, all inside this folder
$EDITOR .ferrite/env.local    # set FERRITE_MODEL_SMALL and FERRITE_MODEL_MAIN (or connect a model in the app's Settings instead)
export OLLAMA_API_KEY=...     # or store it in the keychain, or paste it into Settings
just run-all                  # real-Servo browser (dev profile) + the local Laya server
just run-fast                 # the same with the release profile: the one to use for daily use
```

| You want | Run |
|---|---|
| a checklist of what is ready on this machine | `just doctor` |
| the UI only, no web rendering, nothing to set up | `just run` |
| format, lint and unused-dependency gates | `just check` |
| the full test suite (no network, no API key) | `just test` |
| the offline evaluation | `just eval`, `just guard-eval` |
| a real model in the evaluation loop | `just live-eval-plan ...` then `just live-eval ...` |
| a crash or freeze report to send | `just collect-logs` |
| does this page work in Ferrite? | `just page-shot <url> 8000 /tmp/shot.png` |

You need `rustup`. Use the stable toolchain with `rustfmt`, `clippy` and
`llvm-tools`. The file `rust-toolchain.toml` pins these. You may also want
`just` and `cargo-deny`. Install them with `brew install just cargo-deny`, or with
`cargo install just cargo-deny`. You may also want `cargo-machete`. Install it
with `cargo install cargo-machete --locked`. The command `just install-hooks`
sets up the commit-msg and pre-commit git hooks.

## Workspace

```
crates/
├── ferrite-core           types, IDs, capability/primitive taxonomy, OriginScope, Clock
├── ferrite-model          ModelProvider trait: Mock/Replay/Ollama/Gemini + cache/throttle/budget decorators, settings, keyring
├── ferrite-audit-log      SHA-256 hash-chained, SQLite-backed audit log
├── ferrite-ipi            the IPI defense: fingerprint, sanitizer, dry-run, twin, comparator
├── ferrite-engine         BrowserEngine trait + MockEngine (always built, no feature flag)
├── ferrite-engine-servo   ServoEngine and BorrowedServoEngine, BrowserEngine over real Servo (feature `engine-servo`)
├── ferrite-agent          the agent loop (browser_loop), chats, page context, the optional Laya decider
├── ferrite-servo          the live app's Servo integration (HeadlessServoSession, diagnostics), feature `servo`
├── ferrite-ui             Iced UI: tabs, address bar, DevTools, agent panel, consent cards, settings
├── ferrite-shell          top-level binary, CLI dispatch, log file
└── ferrite-eval           evaluation harness (Servo-free): corpus, harness, adjudication, metrics, live model runner
vendor/
└── servo-script           servo-script 0.6.0 with one patch (not a workspace member; see FERRITE-PATCHES.md)
```

Two crates look like duplicates, but they are not.

- `ferrite-servo` is the integration that the live app runs. It has
  `HeadlessServoSession`, one session per tab.
- `ferrite-engine-servo` adapts that integration to the engine-agnostic
  `BrowserEngine` trait. The app uses `BorrowedServoEngine`, which borrows the
  tab's own session.
- The other adapter, `ServoEngine`, owns its sessions. It does not yet complete a
  real page load in the headless harness (`docs/TO-DO.md` T-220).

The target architecture in `docs/REBUILD_DIRECTIVE.md` §4 also names a
`ferrite-cli` crate. It was never split out of `ferrite-shell`. `ferrite-shell`
remains the one binary.

Servo is not built by default. It sits behind the `servo` feature of
`ferrite-servo`, or the `engine-servo` feature of `ferrite-engine-servo`. Most
development never touches it. All of the IPI defense logic never touches it. See
`CLAUDE.md` for the hard rules. It also lists what was deliberately deleted and
must not come back.

## Running locally

macOS comes first. Linux is best-effort. Windows has not been tried with the
scripts. Every step is idempotent: running it twice is safe. Everything lives
inside this checkout. Build output goes to `target/`. All local state goes to the
gitignored `.ferrite/` folder. Local state means the Laya venv and checkpoint,
`env.local`, the profile and the settings. You can move them with
`$CARGO_TARGET_DIR` and `$FERRITE_HOME`. `rm -rf .ferrite` removes all local
state. Without `FERRITE_HOME`, the app itself keeps its data in
`~/.local/share/ferrite`.

```
just setup            # Servo-free build, optional Laya; add --dry-run --yes to see the plan
just setup-servo      # the real Servo engine: needed for real pages
just setup-all        # both, plus Laya
just doctor           # what is ready, what is not (--fix-hints says how to fix it)
just run-local        # reads .ferrite/env.local, starts Laya if set up, runs whichever build setup made last
just run-all          # force the Servo build
just run-fast         # Servo, release profile
```

`just setup` checks these tools: git, the Xcode command line tools, Homebrew and
rustup. It also checks the brew packages that CI installs (`cmake pkg-config
openssl sqlite`). It offers to install what is missing. It does so only after you
say yes (or pass `--yes`). It never uses `sudo`. These scripts never write API
keys. Keys come from your environment, the OS keychain, or the Settings drawer. No
model name is a default. So you must set `FERRITE_MODEL_SMALL` and
`FERRITE_MODEL_MAIN` in `env.local` for `run-local`. The command
`just models-local` lists the tags your endpoint serves. Or pass
`--no-model-check` and connect a model in Settings. A key stored with `security`
may make macOS ask once if `ferrite-shell` may read it. A rebuilt binary can
trigger that question again.

**Servo or not.** The default build is Servo-free. It is quick: about two
minutes cold, per `docs/BUILD_BUDGET.md`. But it has **no real web rendering**.
For real pages, run `just setup-servo`. It runs
`cargo build -p ferrite-shell --features ferrite-servo/servo`. Audio and video
playback are not built in. So no extra brew package is needed. Plan on 20 to 60
minutes and 10+ GB of disk the first time. `just run-local` uses whichever build
`setup` made last. You can force one with `--servo` or `--no-servo`.

**Laya (optional).** [Laya](https://github.com/NandhaKishorM/laya) is a small
local decision model. It gets the page state and the candidate elements. It
scores "which operation, which element" for the next browser step. Ferrite talks
to Laya over HTTP only when `FERRITE_LAYA_URL` is set. With it unset, the app runs
exactly as before, with the LLM only.

- `just setup` makes a Python venv under `$FERRITE_HOME/laya`.
- It installs `laya[serve]`.
- It downloads the `cklxx/laya-browser` checkpoint from Hugging Face. The default
  is `v10s`, the faster 322M one. Use `--laya-checkpoint v10` for the 421M one.
  Skip all of it with `--no-laya`.
- `just run-local` then starts a local server on `127.0.0.1:8765` for the
  session. It waits for `/health`, sets `FERRITE_LAYA_URL`, and stops the server
  when the app exits.
- Apple silicon uses the Metal (`mps`) backend automatically. On the owner's Mac
  that was slower than the LLM it was meant to beat (`docs/TO-DO.md` T-250). The
  app stops asking Laya by itself when it does not pay off (ADR-015).
- `just laya-verify` measures yours.
- The server binds to loopback only. It refuses any other address unless an API
  key is set.

Settings for the scripts live in `$FERRITE_HOME/env.local`. It is plain
`KEY=VALUE`. The scripts read it as data and never run it as a shell script. Only
`FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and `RUST_LOG` are used. Anything already
exported in your shell wins. `scripts/local.env.example` documents every knob.
**A packaged app does not read `env.local`.** Connect a model from its Settings
drawer.

## Using the browser

The window has these parts:

- tabs;
- an address bar (a URL, a bare host, or a search: anything else goes to Google);
- back, forward and reload;
- an **Agent** toggle;
- an **Audit** shield;
- one overflow menu (zoom, find, bookmarks, history, downloads, developer tools,
  theme, settings).

A few shortcuts follow. Use Cmd on macOS and Ctrl elsewhere. The full table is in
`docs/COMMANDS.md`.

- `T` opens a new tab.
- `W` closes it.
- `L` goes to the address bar.
- `F` finds text.
- `D` bookmarks the page.
- `Shift+A` shows the agent panel.
- `,` opens settings.
- `J` opens developer tools. So do `Cmd+Opt+I` and `Ctrl+Shift+I`.
- `F12` opens the Audit panel.
- `Esc` dismisses, declines or cancels. It never approves.

- **Developer tools** work per tab. They have three views and two buttons.
  - Console: levels, a filter and a JavaScript prompt.
  - Network: requests as they start. It shows no status or timing yet.
  - Engine: panics and dead script threads.
  - The buttons are Copy all and Save log.
- **Panels** are the agent, library, settings, developer tools and audit panels.
  You resize one by dragging the splitter. A double-click resets it. Sizes are
  remembered.
- **Pages' own controls** are drawn by Ferrite. These are dropdowns,
  `alert`/`confirm`/`prompt`, the colour picker and the context menu. The file
  picker is a card where you type the path. A page whose script thread dies shows
  a banner with Reload.
- **Consent** has three forms.
  - A pre-run panel shows what the dry run saw beyond the prediction. You choose
    Approve or Reject for each item.
  - A question appears when the real run steps outside the prediction. You can
    choose *Allow once*, *Allow for task* or *Don't allow*.
  - A hand-off appears when a page wants a password. You sign in. The agent never
    types it.

## Watching what the agent, the LLM and Laya do

Open **Audit**. Use the shield in the toolbar, or press `F12`. The **Model calls**
view is a timeline of every call.

- It shows what the agent ran.
- It shows every LLM request with its full prompt and answer.
- It shows every Laya request. It also says whether Laya's answer was used or sent
  back to the LLM.
- Each row shows how long it took.

Click a row to see what was sent and what came back. A one-line summary at the
top answers "is Laya making it faster?" It uses the timings seen so far. If Laya
is slower than the LLM here, it pauses itself for a few steps and says so. At
start-up the server logs the device it runs on (a CPU is not a fast lane). The
same events are appended to `<data dir>/logs/model-activity.jsonl`. That file
holds prompts and page text. Treat it like the pages themselves. It never leaves
your machine. The **Security log** view is the hash-chained record of network
requests.

## Evaluating the defense

```
just eval                 # the 938-case corpus through every defense mode -> target/eval-report/EVAL_REPORT.md
just guard-eval           # the runtime guard when the dry run could not have seen the attack -> GUARD_REPORT.md
just redteam-corpus-check # the 909 generated cases match the generator
```

`just eval` needs no network and no API key. The exception: you export
`FERRITE_MODEL_SMALL` or `FERRITE_MODEL_MAIN` and a key. Then the fingerprint
prediction makes live calls. Unset them to reproduce the reported numbers. The
scripted agent in `just eval` obeys every injection. That is the worst case.
`just guard-eval` measures what the real run does when the dry run could not have
seen the attack. That is the live app's situation. The red-team suites for single
parts are these two commands:

- `cargo test -p ferrite-ipi --test red_team_sanitizer`
- `cargo test -p ferrite-core --test red_team_scope`

`docs/EVALUATION.md` §8 has the numbers and, more usefully, the limits.

**With a real model** (`docs/EVALUATION.md` §9; every flag is in
`docs/COMMANDS.md` §6):

- `just live-eval-plan --provider gemini --model <tag> --corpus agentdojo` prices
  a selection. It calls nothing.
- `just live-eval ... --batch-size 10 --max-calls 100 --pause-ms 4000` runs one
  batch. You can resume it. Results are stored in `target/live-eval/results/`.
- `just live-eval --report` writes `REPORT.md` and `report.csv`.
- The AgentDojo corpus comes from a pinned checkout of AgentDojo.
  `just agentdojo-check <checkout>` checks that the files in the repo match a
  fresh import. `python3 scripts/import_agentdojo.py --src <checkout>` writes
  them again.

**The owner's real run.** The owner ran all 1,984 cases with
`ollama` and `gemma4:31b`. The headline numbers are in the Status section above.
The full report is `docs/results/live-eval-ollama-gemma4-31b.md`.
`docs/EVALUATION.md` §11 explains it. A command like it is in
`docs/COMMANDS.md` §6.

## Logins and cookies

The browser profile lives in `<data dir>/profile`. It holds cookies, HSTS, saved
HTTP credentials and web storage. It survives restarts. It is written when the
window is closed normally. A crash, `kill` or macOS Cmd+Q may lose the new
cookies of that run. Whether a given site lets you sign in is a separate
question. Google in particular may refuse an embedded engine (`docs/TO-DO.md`
T-267). `FERRITE_USER_AGENT` overrides the user-agent string. Settings also has a
browser identity choice. The default is Firefox-compatible (ADR-018).

## When something goes wrong

Sometimes you start the app without a terminal (from Finder or a launcher). Then
the app writes its standard error output to a log file.

- On macOS the file is `~/Library/Logs/Ferrite/ferrite.log`. The run before is
  `ferrite.previous.log`.
- On Linux the file is `$XDG_STATE_HOME/ferrite/ferrite.log`. Without that
  variable it is `~/.local/state/ferrite/ferrite.log`.
- If you run from a terminal, the app prints there instead.
- Windows writes no log file yet (`docs/TO-DO.md` T-298).

Read the `[ferrite-render]` and `[ferrite-webgl]` lines first. The first says
`CPU rendering`: Ferrite has no GPU renderer. The second says whether pages get
WebGL. `FERRITE_WEBGL=on|off|auto` sets it, and the default is on.

- `just collect-logs` zips the log, the crash reports and the system information
  onto your Desktop. On macOS it also adds a stack sample if the app is frozen.
  Send the zip with a bug report.
- `just crash-report` prints where the newest macOS crash happened.
- Quitting ends the process within 3 seconds of the window closing.
- `docs/COMMANDS.md` §8 has the rest.
