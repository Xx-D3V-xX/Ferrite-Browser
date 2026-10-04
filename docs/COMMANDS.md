# Ferrite — Commands and Usage Guide

How to set Ferrite up, run it, use it, evaluate it, diagnose it and release it.
Every command, flag, variable, path and shortcut below was read from the code or
script that implements it (the source is named next to it where that helps).
Status of the work (what is finished, what is open) is not here: it lives in
`docs/PROGRESS.md` and `docs/TO-DO.md`. Design reasons live in `docs/DECISIONS.md`.

**How this was checked.** Written on a Linux machine without a display and
without a Servo build. Script flags were checked by running the scripts' own
`--help` and `--dry-run` modes; the live runner's flags by reading its argument
parser and running the already-built `live_eval` (`--help`, `--plan`, and a real
three-case `--provider mock` batch plus `--report`); shortcuts and menus from
`crates/ferrite-ui/src`. Every recipe was checked with `just --list` and
`just --dry-run <recipe>` (just 1.58.0) so the command each one runs is as written
here; no recipe was executed through `just`. Nothing here was run on macOS or
Windows. Each section ends with what is not verified.

## Contents

1. [Conventions](#1-conventions)
2. [First-time setup](#2-first-time-setup)
3. [Running the browser](#3-running-the-browser)
4. [Using the browser](#4-using-the-browser)
5. [Models, keys and the agent](#5-models-keys-and-the-agent)
6. [Evaluation](#6-evaluation)
7. [Engine probes and `page_shot`](#7-engine-probes-and-page_shot)
8. [Diagnostics](#8-diagnostics)
9. [Quality gates](#9-quality-gates)
10. [Release and CI](#10-release-and-ci)
11. [Environment variable reference](#11-environment-variable-reference)
12. [Where files live](#12-where-files-live)

---

## 1. Conventions

- **`just`** is the command surface (`just --list` prints every recipe; `just`
  alone does the same). If you do not have `just` (`brew install just`, or
  `cargo install just`), each recipe below also shows the plain command it runs.
- **Two kinds of recipe.** The build, lint, test, eval and probe recipes are plain
  `cargo`/`python3` commands. The local toolchain recipes (`setup*`, `run-local`,
  `run-fast`, `run-all`, `models-local`, `laya-*`, `doctor`, `crash-report`,
  `test-local`, `disk`, `probe-profile`) call bash scripts under `scripts/` (bash
  3.2 compatible, macOS first, Linux best-effort) and need a POSIX shell. They have
  not been run on Windows.
- **"Needs"** in each entry is what the command costs: network, an API key, the real
  Servo build, time, disk.
- **Two builds.** The default build has **no Servo**: it compiles in about two
  minutes cold (`docs/BUILD_BUDGET.md`), opens the UI, runs the agent loop and the
  defense, but **cannot render web pages** (the engine stub refuses to start:
  "compiled without the `servo` feature"). The **Servo build** adds
  `--features ferrite-servo/servo` and is what renders real pages; the first build
  takes 20 to 60 minutes and 10+ GB (`scripts/setup-local.sh --help`; the only
  measured figure in the repo is an older 15m31s / 6.4 GB debug build added to an
  existing target dir, `docs/BUILD_BUDGET.md`). Everything that talks to a page,
  including every `probe-*` recipe and `page_shot`, needs the Servo build.
- **Profiles.** `dev` is the default (builds faster). `release` is what to use to
  *use* the browser or judge its speed: the engine runs several times faster
  optimised (`docs/TO-DO.md` T-269). `FERRITE_PROFILE=release` selects it for the
  scripts; `just run-fast` does so for you.
- **Build output** goes to `target/` (override with `CARGO_TARGET_DIR`; the
  justfile exports `<repo>/target` as the default, and the scripts do the same).
  All local state goes to `$FERRITE_HOME`, which the scripts default to the
  gitignored `<repo>/.ferrite/`.

---

## 2. First-time setup

Prerequisites: `git`, `rustup` with the toolchain in `rust-toolchain.toml` (stable,
with `rustfmt`, `clippy`, `llvm-tools`), `python3` (scripts, corpus tools, Laya),
and a C toolchain plus `cmake` and `pkg-config`.

Optional tools: `just`, `cargo-deny` (`just audit`), `cargo-machete` (`just check`),
`cargo-bloat` (`just bloat`), `cargo-sweep` (`just clean-cache`):

```
brew install just cargo-deny              # or: cargo install just cargo-deny
cargo install cargo-machete --locked
just install-hooks                        # git hooks, see section 9
```

### `just setup [flags]`  (`./scripts/setup-local.sh [flags]`)

Checks your toolchain, builds `ferrite-shell` (Servo-free), optionally sets up Laya,
and creates `$FERRITE_HOME/env.local` from `scripts/local.env.example` (mode 600,
never overwritten). Safe to re-run: every step checks first and skips what is done;
nothing is deleted or overwritten. It never uses `sudo`; it offers to install rustup
and brew packages and only does so after you say yes or pass `--yes`.

| Flag | Effect |
|---|---|
| `--with-servo` / `--no-servo` | build with / without the real Servo engine. With neither, an interactive run asks; a non-interactive run (no TTY, `--yes`, `--dry-run`) builds without |
| `--no-build` | skip the cargo build |
| `--laya` / `--no-laya` | set up / skip the local Laya server (default: on) |
| `--laya-checkpoint NAME` | `v10s` (default, 322M, faster) or `v10` (421M) |
| `--yes`, `-y` | answer yes to every install prompt |
| `--dry-run` | print every action that would be taken, run none |
| `-h`, `--help` | the script's own usage text |

Environment it honours: `FERRITE_HOME`, `CARGO_TARGET_DIR`, `FERRITE_PYTHON` (the
interpreter used for the Laya venv), `FERRITE_LAYA_PIP_SPEC` (default
`laya[serve]>=0.3.21,<0.4`), `FERRITE_PROFILE`, `HF_TOKEN` (if a Hugging Face
download needs authentication).

What it may write: `$FERRITE_HOME` (the Laya venv and checkpoint, `env.local`,
pid/log files, `build.mode`), `$CARGO_TARGET_DIR`, and, only after a prompt or
`--yes`, rustup and brew packages. `rm -rf .ferrite` removes all local state.

Expected output (from `just setup --dry-run --yes --no-laya` on Linux): a header
with the repo, `FERRITE_HOME`, target dir and platform; sections `git`, `system
packages`, `Rust toolchain`, `Build ferrite-shell` (it prints `Servo-FREE build:
fast (~2 min cold), but it has NO real web rendering`), `Laya`, `Local environment
file`, then a reminder that keys are never written by these scripts.

Needs: network (rustup, crates.io, brew, pip, Hugging Face for Laya), roughly 2
minutes for the Servo-free build, more for Laya's checkpoint download.

### `just setup-servo [flags]`

`setup-local.sh --with-servo`: the same, with the real Servo engine
(`cargo build -p ferrite-shell --features ferrite-servo/servo`). Needs 20 to 60
minutes and 10+ GB the first time; later builds are incremental and Ctrl-C is safe
(re-running continues where cargo left off). On Linux the script warns that Servo
also needs its own system libraries (it links to Servo's setup page; it does not
install them for you). Use `FERRITE_PROFILE=release just setup-servo` to build the
release profile instead (slower to build, much faster to run).

### `just setup-all [flags]`

`setup-local.sh --with-servo --laya`: Servo and Laya in one command, all inside
this folder.

### `just doctor [--fix-hints] [--no-probe]`  (`./scripts/doctor.sh`)

A read-only checklist; it changes nothing and never prints an API key. Sections:
`Tools` (git, cargo, rustc, just, python), `Build` (whether the binary exists for
the chosen profile and whether setup recorded a Servo or plain build),
`Configuration` (the `env.local` file, `FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN`,
`OLLAMA_API_KEY`, `FERRITE_TWIN_KEY`), `Laya (optional)` (venv, checkpoint, a
latency probe of a running server), `Disk` (free space where `FERRITE_HOME` and the
target dir live, and the target dir's size).

- `--fix-hints` prints how to fix each problem.
- `--no-probe` skips the Laya latency-probe requests.
- Exit 0 if nothing is `FAIL` (warnings are allowed), 1 otherwise.
- Output ends with a line like `2 problem(s), 7 warning(s).`
- A missing model tag is a `FAIL`, even though the app can now be connected from its
  Settings drawer instead (section 5): the doctor checks the environment route only.
- On Linux it says the OS keyring is not checked.

Not verified: `setup` or `setup-servo` end to end on macOS from a clean machine in
this session (only `--dry-run` was read and run, on Linux); the Laya parts.

---

## 3. Running the browser

### Which command

| You want | Run | Notes |
|---|---|---|
| Real web pages, every day, fast | `just run-fast` | release profile + Servo, with Laya if set up |
| Real web pages, working on Ferrite | `just run-all` | dev profile + Servo, with Laya if set up |
| Whatever `setup` built last | `just run-local` | reads `env.local`, starts Laya if set up |
| The UI only, no web rendering | `just run` | Servo-free; the only recipe that takes no setup |
| A packaged app | double-click, or `./ferrite` | see section 10 |

### `just run [args]`  (`cargo run -p ferrite-shell -- [args]`)

Runs the shell binary. This recipe cannot enable Servo (it never passes
`--features`), so pages will not render; use it for the UI, the agent loop and
the defense. Arguments after `run` go to `ferrite-shell`. No arguments, or an
unrecognised first argument, launches the UI.

### `just run-local [flags] [-- shell args]`  (`./scripts/run-local.sh`)

1. loads `$FERRITE_HOME/env.local` (plain `KEY=VALUE`, read as data, never sourced;
   only `FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and `RUST_LOG` are honoured; anything
   already exported in your shell wins);
2. checks `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` are set and **exits 2 with
   instructions if not**; pass `--no-model-check` to open the UI anyway (then connect
   a model from the Settings drawer);
3. prints an advisory about `OLLAMA_API_KEY` (never the value; the macOS keychain is
   checked on macOS only);
4. starts the local Laya server in the background if it is set up and not already
   running, waits for `/health`, exports `FERRITE_LAYA_URL`;
5. runs `cargo run [--release] -p ferrite-shell [--features ferrite-servo/servo] -- ui`;
6. on exit, Ctrl-C or TERM, stops the Laya server it started (never one that was
   already running).

| Flag | Effect |
|---|---|
| `--no-laya` | do not start or use Laya (LLM only); an inherited `FERRITE_LAYA_URL` is dropped |
| `--servo` / `--no-servo` | force the Servo / Servo-free build; default is what `setup` recorded in `$FERRITE_HOME/build.mode` (Servo-free if there is none) |
| `--wait SECONDS` | how long to wait for Laya's `/health` (default 120; the first start loads the model) |
| `--no-model-check` | launch even if the model tags are unset |
| `--list-models` | list the tags your Ollama endpoint serves, then exit (this is `just models-local`) |
| `--laya-only` | run only the Laya server, in the foreground (this is `just laya-serve`) |
| `--verify` | send one recorded step to the Laya server and print its decision and latency (this is `just laya-verify`) |
| `--dry-run` | print what would happen; start nothing |
| `-- ARGS...` | pass `ARGS` to `ferrite-shell` instead of `ui` (`window`, `jstest`, `agent-smoke`, `smoke`) |

Environment: `FERRITE_PROFILE=dev` (default) or `release`. A dry run, for reference:
`just run-local --dry-run --no-model-check --no-laya --servo` prints
`cargo run -p ferrite-shell --features ferrite-servo/servo -- ui`.

### `just run-fast [flags]`

`FERRITE_PROFILE=release ./scripts/run-local.sh --servo [flags]`. The release engine
build is slow the first time. The effect on speed was **not measured** in this repo
(`docs/TO-DO.md` T-269): the dev profile's measured disadvantage was a reason, not a
benchmark of this recipe.

### `just run-all [flags]`

`./scripts/run-local.sh --servo [flags]`: the real-Servo browser in the dev profile
plus the local Laya server.

### The `ferrite-shell` subcommands

The first argument selects one; none takes flags (`crates/ferrite-shell/src/main.rs`).
Run directly with
`cargo run -p ferrite-shell --features ferrite-servo/servo -- <subcommand>`
(or without `--features` where noted), or after the recipe's `--`:
`just run-local -- jstest`.

| Subcommand | What it does | Needs |
|---|---|---|
| `ui` (default) | the browser (Iced window, tabs, agent panel) | Servo build for pages |
| `window` | `ServoShell::new().run()` (`ferrite-servo/src/shell.rs`): a bare winit window hosting the engine, with none of Ferrite's tabs, address bar or agent. Not exercised in this pass | Servo build |
| `jstest` | opens a headless Servo session, loads `https://example.com`, `https://lite.duckduckgo.com` and `https://doc.rust-lang.org`, prints a table (JS executed, title, console errors) and writes `../paper/data/js_compat_baseline.csv` relative to the working directory, warning if it cannot create the folder | Servo build, network |
| `agent-smoke` | runs the agent loop against the in-process `MockEngine` with a real model provider on "What is the title of the page at https://example.com?". Prints `[agent-smoke] actions taken`, then `final_response` or `asked_user`. Exits 0 with a "skipping" message if no model is configured or reachable (`FERRITE_MODEL_SMALL`/`FERRITE_MODEL_MAIN` unset or no Ollama/Gemini credential), exits 1 if the loop does not finish cleanly | model tags, a key or local Ollama, network |
| `smoke` | appends two events to a temporary SQLite audit log (`ferrite_smoke_test.db` in the OS temp dir), verifies the hash chain and prints `Smoke test: ALL CHECKS PASSED` | nothing |

`ui` exits the process explicitly when the window closes (section 8 explains why).
The process exit code is 1 only if the UI returned an error.

### Packaged apps

The release packages (`ferrite-<sha>-macos-arm64.zip`, `...-windows-x64.zip`,
`...-linux-x64.tar.gz`) contain the Servo build in the release profile. They run
`ui` with no arguments. **A packaged app does not read `env.local` (the Linux
package's own `README.txt` says it does; that is wrong, `docs/TO-DO.md` T-299) or
any `FERRITE_*` variable you did not export before launching it**; connect a model from
its Settings drawer (section 5). macOS: the app is ad-hoc signed, not notarized, so
the first launch needs System Settings, Privacy & Security, Open Anyway (or
`xattr -dr com.apple.quarantine /Applications/Ferrite.app`) (`docs/TO-DO.md` T-261).
Windows: SmartScreen may ask once. Linux: needs a graphical session with OpenGL/EGL
and the shared libraries named in the package's `README.txt`; `./ferrite` runs it,
`./install.sh` copies it into `~/.local`.

Not verified: any of this on macOS or Windows; `run-fast`'s speed; the packaged
Linux and Windows builds on a real machine.

---

## 4. Using the browser

### Window and tabs

A 34 px tab strip, a 40 px toolbar and the page. On macOS the title bar is
transparent and the tab strip holds the traffic lights; Linux and Windows keep their
decorations. Tabs share the strip equally down to a minimum then scroll; a title is
cut with an ellipsis; the close button shows on the active or hovered tab;
middle-click closes a tab; the strip's empty part drags the window and a double
click maximizes it. A coloured dot on a tab says the page stopped responding (red)
or is waiting for you (accent). A page that opens a new window (`window.open`,
`target=_blank`) gets a tab.

Toolbar, left to right: back, forward, reload (or stop while loading), the address
bar, the **Agent** toggle (a dot appears when something needs you), the **Audit**
shield (`F12`) and the overflow menu. The address bar:

- accepts a URL with a scheme as is; `about:` pages as is;
- a bare host or path with a dot (and no spaces) gets `https://`; `localhost` and IP
  literals get `http://`;
- **anything else is a Google search** (`https://www.google.com/search?q=...`). The
  agent's own search prompt still points at DuckDuckGo Lite, deliberately
  (`docs/TO-DO.md` T-279);
- clicking it selects everything; Enter navigates and hands the keyboard back to the
  page; the zoom chip and bookmark star sit inside it.

A thin sweeping bar over the toolbar's bottom edge shows loading. A new tab shows a
page with a search box and quick-access tiles.

The **overflow menu** (dots) holds: New tab, a zoom control, Find in page, Bookmark
this page (only on a real page), Bookmarks, History, Downloads, Developer tools,
Switch to dark/light theme and Settings. Bookmarks, History and Downloads share one
drawer (the Library). Downloads has a manual "Download current page" action; clicks
on `<a download>` links are not intercepted (`docs/TO-DO.md` T-232).

### Keyboard shortcuts

`Mod` is **Cmd on macOS and Ctrl everywhere else**. From `handle_key_press` in
`crates/ferrite-ui/src/lib.rs`; they also work while the address bar or the agent
box has focus (editing combinations such as Mod+A/C/V/X/Z still reach the field).

| Keys | Action |
|---|---|
| Mod+T | new tab |
| Mod+W | close the active tab |
| Mod+R, F5 | reload |
| Mod+L | focus and select the address bar |
| Mod+F | find in page |
| Mod+D | bookmark / remove bookmark for this page |
| Mod+= or Mod++, Mod+-, Mod+0 | zoom in, out, reset (steps: 50, 67, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300 %) |
| Mod+[ , Mod+] | back, forward |
| Alt+Left, Alt+Right | back, forward (Alt on every platform) |
| Mod+Shift+[ , Mod+Shift+] | previous, next tab |
| Ctrl+Tab, Ctrl+Shift+Tab | next, previous tab (Ctrl, also on macOS) |
| Mod+1 to Mod+8, Mod+9 | that tab by number; the last tab |
| Mod+Shift+A | show or hide the agent panel |
| Mod+Shift+O | new agent chat |
| Mod+, | settings drawer |
| Mod+J | developer tools (toggle) |
| Cmd+Opt+I (macOS), Ctrl+Shift+I (elsewhere) | developer tools (toggle) |
| F12 | the **Audit** panel (not DevTools; whether F12 should open DevTools is an open owner decision, `docs/TO-DO.md` T-297) |
| Esc | closes, in priority order: a page control the page is waiting on (cancel), the menu, the find bar; answers the agent's question with **no**; cancels the pre-run consent panel; stops loading; unfocuses the address bar |

Typing, pointer and wheel input go to the page unless a field of the browser has
focus or a page control (below) is open. Cmd/Ctrl+C/X/V reach the page's focused
field. Not handled: IME composition and HTTP basic-auth prompts (`docs/TO-DO.md`
T-294); non-US layouts, dead keys and key repeat are unobserved (T-236).

### Developer tools

Open with Cmd+Opt+I / Ctrl+Shift+I, Mod+J, or the menu's Developer tools. A bottom
panel with three tabs; counts show on the tab labels.

- **Console**: every message the active tab's page printed, all levels, per tab
  (up to 5,000 rows per tab, 4,000 characters per message, 8 MiB of text per tab,
  oldest dropped first). A filter box and level chips (All, Errors, Warnings, Info,
  Debug), a "Preserve log" toggle that keeps messages across navigation, long messages
  folded with an expand control, and a **prompt** (`Run JavaScript on this page`):
  type an expression and press Enter or Run to run it in the page; Up and Down
  recall earlier input (up to 100). This is you running script in the page, not the
  agent.
- **Network**: each request the tab made, as it starts (method, URL, kind) with a
  filter box and kind chips (All, Document, Script, Style, Image, Font, Media,
  Other). **No status, size or timing** yet: the engine does not report them
  (`docs/TO-DO.md` T-292). `fetch()` and XHR fall under Other.
- **Engine**: panics on engine threads and pages whose script thread died, this
  session, with the thread, message and location.
- Header buttons: **Copy all** (the active tab as text to the clipboard), **Save
  log...** (writes `ferrite-<console|network|engine>-tab<N>-<YYYYMMDD-HHMMSS>.log`
  into the log folder, section 8) and **Open log folder** (`open` on macOS,
  `explorer` on Windows, `xdg-open` elsewhere). Console lines are also mirrored to
  stderr (so into the log file) as `[console:<level>] tab N: ...`, at most 20 per
  UI tick.

### Panels

The agent, Library and Settings drawers share the right-hand side (one at a time,
beside the page); DevTools and Audit share the bottom. **Drag the splitter** between
a panel and the page to resize it; **double-click** it to reset. Side drawers are
300 to 760 px wide (default 380), bottom panels 140 to 720 px tall (default 260), and
the page always keeps at least 360 px by 160 px. Sizes are clamped to the real window
every time and saved when you release the mouse, in `ui-layout.json` under the data
folder (section 12).

### The agent and consent

Open the agent panel (toolbar **Agent**, Mod+Shift+A), type a task and send. Each
run is a chat; chats are saved and listed. The panel says plainly when no model is
connected and has a button to Settings. The agent's answer is rendered as Markdown
(no HTML interpreted, no images fetched, a link shows the site it really goes to and
opens only on your click).

What you may be asked, in order:

1. **Before the run** (the pre-run consent panel): after the dry run, anything the
   agent attempted beyond the predicted fingerprint is listed as cards, each with
   **Reject** / **Approve**; **Proceed** and **Cancel** stay pinned under the list
   and Proceed needs every item decided. Card edge colours: amber undecided, green
   approved, red rejected; the surface is neutral.
2. **During the real run**: an action outside the prediction pauses the run with a
   card naming only the action and the site (never page text): **Don't allow**
   (Esc), **Allow once**, **Allow for task**. `js.execute` is always asked
   (`docs/DECISIONS.md` ADR-003, ADR-020).
3. **Sign-in**: when the page asks for a password or other secret, or is a known
   sign-in host, the run pauses before any model call: sign in yourself, then press
   **I've done it, continue** (or stop the task). The agent never types into
   password fields (ADR-018).

The **Audit** panel (shield or F12) has two views: **Model calls**, a timeline of
every agent step, LLM request (full prompt and answer) and Laya request with
timings and whether Laya's answer was used (click a row for the payloads; a summary
line says whether Laya is paying for itself), and **Security log**, the hash-chained
record of network requests. The model calls are also appended to
`<data dir>/logs/model-activity.jsonl` (prompts and page text included: treat it
like the pages themselves; it never leaves your machine).

### What a page can ask the browser for

Dropdown lists (`<select>`), `alert()`/`confirm()`/`prompt()`, the colour picker, the
file picker and the page's context menu are drawn by Ferrite as overlays. The page
is blocked until you answer, and pages get no pointer or key input while one is
open. Page text in them is drawn as plain text and a dialog says which site it comes
from. The **file picker is a card where you type or paste the path(s)**; there is no
native file dialog yet. The page's cursor shape (pointer, text, crosshair, ...) is
followed; `cursor: none` shows the ordinary arrow.

A **page crash** (its script thread died) leaves the page frozen on screen with a
banner: **Reload** (a fresh session for the same address), **Details** (reason and
backtrace, with Copy) and **Dismiss**. It is also in DevTools' Engine tab and the log.

### Settings (Mod+,)

- **Model**: choose Ollama Cloud, a local Ollama (loopback address only), or Gemini;
  paste the API key; **Load models** asks the provider what the key can use (this
  also proves the key before a task spends a call); pick the fast and agent models
  (or one for both); **Save and use** applies at once, no restart. The key goes to
  the OS keyring under service `ferrite` (account `OLLAMA_API_KEY` or
  `FERRITE_GEMINI_API_KEY`), never to a file; **Remove key** deletes it and
  disconnects. The environment still wins over the saved choice, and the drawer says
  when it does. On Linux the key store is the kernel keyring, **cleared on reboot**;
  export the variable for a permanent key (`docs/TO-DO.md` T-259).
- **Appearance**: theme and default zoom for new tabs.
- **Browser identity**: Firefox-compatible (default; Servo's own User-Agent with the
  engine token swapped for `Gecko`) or Ferrite (names its engine). Applies at the
  next launch.
- A footer lists where the settings file, the key and the response cache are.

### Logins and cookies

The browser profile (cookies, HSTS, saved HTTP credentials, web storage) is
`<data dir>/profile` and survives restarts. **Servo writes it when the window closes
normally**: a crash, `kill` or macOS Cmd+Q may lose that run's new cookies (Cmd+Q's
path is untested, `docs/TO-DO.md` T-295). Whether a site accepts a sign-in is a
separate question: Google in particular may refuse an embedded engine
(`docs/TO-DO.md` T-267). `FERRITE_USER_AGENT` overrides the User-Agent string
(it wins over the Settings choice).

Not verified: all of section 4 on macOS or Windows; HiDPI; the page-control overlays
against a real page in the app (they were driven under Xvfb at scale 1 and in the
engine probe `controls_probe`, and the file picker's answer path is untested).

---

## 5. Models, keys and the agent

The agent needs a model provider. Three ways to give it one, strongest first:

1. **Environment variables** (also read from `$FERRITE_HOME/env.local` by
   `run-local`): `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` (tags; **no model
   name is ever a default**, they may be the same) plus a key.
2. **Settings drawer** (works in a packaged app; saved to `settings.json`, key to the
   keyring).
3. Nothing: the agent panel says no model is connected; the defense logic still
   fails closed (an empty fingerprint routes everything through consent).

Keys come from the environment or the OS keyring and nowhere else.

| Provider | Key | Where the key can live | Notes |
|---|---|---|---|
| Ollama Cloud (`https://ollama.com`) | `OLLAMA_API_KEY` | environment, or keyring service `ferrite` account `OLLAMA_API_KEY` | default endpoint |
| Ollama local | none | | `FERRITE_OLLAMA_BASE_URL=http://localhost:11434`; the bearer token is never sent to a non-cloud host |
| Gemini | `FERRITE_GEMINI_API_KEY` | environment, or keyring account `FERRITE_GEMINI_API_KEY` | base URL `FERRITE_GEMINI_BASE_URL` (default `https://generativelanguage.googleapis.com/v1beta/models`) |

With no choice saved and both configured, the app tries Ollama first, then Gemini.
Only Ollama and Gemini exist; an OpenAI-compatible or Anthropic backend does not
(`docs/TO-DO.md` T-277).

Store a key in the macOS Keychain (the command prompts for the value; the first use
may make macOS ask whether `ferrite-shell` may read it, and a rebuilt binary can
trigger that again):

```
security add-generic-password -U -s ferrite -a OLLAMA_API_KEY -w
```

Model-layer knobs (all optional; defaults from `crates/ferrite-model/src/config.rs`):
`FERRITE_MODEL_CALL_BUDGET` (500), `FERRITE_MODEL_MAX_IN_FLIGHT` (2),
`FERRITE_MODEL_TIMEOUT_SECS` (60), `FERRITE_MODEL_MAX_RESPONSE_BYTES` (1048576),
`FERRITE_MODEL_CACHE_DIR` (`~/.cache/ferrite-model`).
`FERRITE_DEFENSE` selects the defense mode for the live app: unset or any
unrecognised value is the full defense; `off`, `sanitizer_only`, `loop_only` are for
ablations only. `FERRITE_TWIN_KEY` is the dry-run twin's encryption key (keyring
account `FERRITE_TWIN_KEY`, else the variable); without either, the dry run degrades
caching, not the defense.

### Model recipes

| Recipe | Runs | What it does | Needs |
|---|---|---|---|
| `just models` | `cargo run -p ferrite-model --example models` | prints every tag the configured **Ollama** endpoint serves, one per line (`(no tags served)` if none); Gemini is not listed by it | network, key unless local; **both** `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` set (any placeholder will do, the loader insists) |
| `just models-local` | `./scripts/run-local.sh --list-models` | the same, after loading `env.local` and supplying placeholders for the tags | network, key or local Ollama |
| `just probe` | `cargo run -p ferrite-model --example probe` | one live round trip to Ollama: prints `provider`, `tag`, `tokens` (prompt + completion) and `content` for "Reply with exactly one word: hello"; exits 1 on any error | network, key, `FERRITE_MODEL_SMALL` a served tag; not part of check/test/CI |
| `just record ollama\|gemini "prompt"` | `cargo run -p ferrite-model --example record -- <provider> "<prompt>"` | records one live response as a fixture under `crates/ferrite-model/tests/fixtures/model/` for offline tests (the prompt defaults to the one-word reply; the first argument is required to be `ollama` or `gemini`). You review and commit the fixture by hand | network, key, config |
| `just cache-stats` | `cargo run -p ferrite-model --example cache_stats` | offline: prints `cache dir`, `entries`, `size` in bytes, last run's hits/misses/uncacheable and the hit rate (`n/a` before any cacheable call) from `~/.cache/ferrite-model/` or `FERRITE_MODEL_CACHE_DIR`; "a low hit rate is a bug" | nothing |

### Laya (optional local decision model)

[Laya](https://github.com/NandhaKishorM/laya) scores "which operation, which element"
for the next browser step. Ferrite talks to it only when `FERRITE_LAYA_URL` is set;
unset, the app is LLM-only. `just setup` builds a venv under `$FERRITE_HOME/laya` and
downloads the checkpoint; `just run-local` starts a server on `127.0.0.1:8765`.

| Recipe | What it does |
|---|---|
| `just laya-serve` | only the Laya server, in the foreground (Ctrl-C stops); refuses if one is already running or something answers at the URL |
| `just laya-verify` | `./scripts/run-local.sh --verify`: sends one recorded browser step to the server and prints its decision and latency; exits non-zero with a suggested fix if unreachable |
| `just test-local` | tests for the toolchain scripts themselves (section 9) |

Laya variables (full list with comments in `scripts/local.env.example`):
`FERRITE_LAYA_URL`, `FERRITE_LAYA_HOST` and `FERRITE_LAYA_PORT` (default
127.0.0.1:8765; the server refuses a non-loopback host unless an API key is set),
`FERRITE_LAYA_CHECKPOINT` (`v10s` or `v10`), `FERRITE_LAYA_DEVICE` (empty = auto,
or `mps`/`cuda`/`cpu`), `FERRITE_LAYA_THREADS`, `FERRITE_LAYA_MPS_AMP_MIN_ROWS`,
`FERRITE_LAYA_WARMUP` (0 skips the start-up warm-up), `FERRITE_LAYA_HEAD_MAX_LEN`,
`FERRITE_LAYA_LOG_LEVEL`, `FERRITE_LAYA_OFFLINE`, `FERRITE_LAYA_API_KEY` (export it,
do not write it to a file), and the app-side `FERRITE_LAYA_TIMEOUT_MS` (default
1500), `FERRITE_LAYA_MODEL`, `FERRITE_LAYA_OP_GATE` (0.80), `FERRITE_LAYA_TARGET_GATE`
(0.60). The warm-up logs the device and milliseconds per step; the app stops asking
Laya by itself when it does not pay for itself (ADR-015). Whether Laya makes the app
faster on a given machine is not established: measure it with the Audit panel's
summary line (`docs/TO-DO.md` T-234, T-250).

Not verified: any live provider call (Ollama Cloud or Gemini) in this session; the
keyring write on any platform; Laya.

---

## 6. Evaluation

All of these run from the repo root. Reports go to `target/eval-report/` unless said
otherwise. Read `docs/EVALUATION.md` for what the numbers do and do not show.

### `just eval`  (`cargo run -p ferrite-eval --example eval`)

Runs the **938-case corpus** (`corpus` 16 + `pilot_corpus` 10 + `agentdojo_corpus` 3
+ `corpus_redteam` 909, all under `crates/ferrite-eval/tests/`) through the real
pipeline in every defense mode and writes `EVAL_REPORT.md`, `eval_report.csv`,
`corpus.db` and `audit.db` to `target/eval-report/` (or `$FERRITE_EVAL_OUT_DIR`).
The agent in it is **scripted** (a worst-case agent derived from each case's ground
truth) and never calls a model. With no model configured the fingerprint layer runs
rules-only and the whole run makes zero network calls. **If `FERRITE_MODEL_SMALL`,
`FERRITE_MODEL_MAIN` and a key are set, the fingerprint prediction goes through the
real provider** (live calls); unset them for the reproducible run. Not part of
check/test/CI. Needs: nothing by default.

### `just guard-eval`  (`cargo run -p ferrite-eval --example guard_eval`)

The runtime-guard experiment (ADR-014, `docs/EVALUATION.md` section 8.4): what the
real run does when the dry run could not have seen the attack. Prints a table and
writes `target/eval-report/GUARD_REPORT.md`. Mock provider only: no network, no key.

### Corpus tools

| Command | What it does |
|---|---|
| `just redteam-corpus` (`python3 scripts/gen_redteam_corpus.py`) | regenerates the 909 red-team cases into `crates/ferrite-eval/tests/corpus_redteam/` deterministically (ids are uuid5 of the case name) from a matrix of tasks, attacker goals, carriers, payload disguises and scopes |
| `just redteam-corpus-check` (`... --check`) | fails if the files on disk differ from a fresh generation |
| `cargo run -p ferrite-eval --example inspect_case -- <path-to-case.json>` | runs one authored case through every mode its run label defines and prints each layer's verdict and whether it matches the case's ground truth (also `just inspect-case <path>`) |
| `cargo test -p ferrite-ipi --test red_team_sanitizer` | the sanitizer red-team suite |
| `cargo test -p ferrite-core --test red_team_scope` | the origin-scope red-team suite |

### AgentDojo import: `scripts/import_agentdojo.py`

Parses (never executes) AgentDojo's task files and writes
`crates/ferrite-eval/tests/agentdojo_full/` (**1,046 cases**: 949 attack plus 97
benign twins) and `agentdojo_full_manifest.json`, pinned to AgentDojo commit
`089ed468cf3ed0322acc66b0211f26d9d90dbf60`, benchmark v1.2.2. Python standard
library only. The dataset itself is not in this repository:

```
git clone https://github.com/ethz-spylab/agentdojo /tmp/agentdojo
git -C /tmp/agentdojo checkout 089ed468cf3ed0322acc66b0211f26d9d90dbf60
python3 scripts/import_agentdojo.py --src /tmp/agentdojo --check   # fail if files on disk differ
python3 scripts/import_agentdojo.py --src /tmp/agentdojo           # (re)write the corpus
```

Flags: `--src SRC` (required), `--out OUT`, `--benchmark-version`, `--attack
{direct,ignore_previous,important_instructions,injecagent,system_message}` (default
`important_instructions`), `--check`, `--allow-other-commit` (output must not be
committed). With `--src` missing or at another commit it exits 2 and prints the
clone and checkout commands. Offline check of the committed files:
`cargo test -p ferrite-eval --test agentdojo_full_validate`. `just agentdojo-check
<src>` wraps the `--check` line. Needs: git and network for the clone; no cargo.

### The live runner: a real model in the loop

```
cargo run --release -p ferrite-eval --example live_eval -- [flags]
```

(`just live-eval [flags]`; `just live-eval-plan [flags]` adds `--plan`.) For each
case it runs the app's own agent loop against the dry-run engine, with the injection
planted in what the agent reads. Two roles are each a model or not: the **predictor**
(`--predictor llm|rules`) and the **agent** (`--agent llm|scripted`). **No model has
been run through it yet** (`docs/TO-DO.md` T-278): every behaviour is tested against
the `mock` provider and a loopback fake server. Full method: `docs/EVALUATION.md`
section 9.

Run `--plan` first: it prices a selection and calls nothing (no key needed). A real
call needs a key in `OLLAMA_API_KEY` / `FERRITE_GEMINI_API_KEY` or the keyring (never
a flag, never printed or stored) and model tags (no default).

| Flag | Meaning |
|---|---|
| `--provider gemini\|ollama\|mock` | the backend; required, never defaulted (env `FERRITE_LIVE_PROVIDER`) |
| `--model TAG` | one tag for both roles (env `FERRITE_LIVE_MODEL`) |
| `--small-model TAG` | the predictor (env `FERRITE_LIVE_SMALL_MODEL`, then `FERRITE_MODEL_SMALL`) |
| `--main-model TAG` | the agent (env `FERRITE_LIVE_MAIN_MODEL`, then `FERRITE_MODEL_MAIN`) |
| `--base-url URL` | the Ollama or Gemini endpoint (a local Ollama, `http://localhost:11434`, needs no key) |
| `--predictor llm\|rules` | default `llm`; `rules` = rule layer only |
| `--agent llm\|scripted` | default `llm`; `scripted` = the worst-case script, no calls |
| `--modes off,guard[,full,dryrun]` | defense modes per case; default `off,guard` |
| `--max-steps N` | agent steps per run; default 8 |
| `--mock-behavior compliant\|resistant\|mixed` | the mock agent's behaviour; default `mixed` |
| `--corpus agentdojo[,redteam,core,pilot,agentdojo-hand,all]` | which corpus; default `agentdojo` (the 1,046-case import) |
| `--corpus-root DIR` | corpus directory (default: `crates/ferrite-eval/tests`); not in `--help` |
| `--suite agentdojo/workspace[,..]` | keep suites with these prefixes (`agentdojo/banking`, `/slack`, `/travel`, `/workspace`) |
| `--only attack\|benign\|all` | default `all` |
| `--seed S` | shuffle the id-sorted order reproducibly (so a small batch samples every suite) |
| `--batch-size N` | cases per invocation; with no `--batch-index`, the next N that still have work to do |
| `--batch-index K` | with `--batch-size`: slice K of the whole ordering |
| `--offset N`, `--limit N` | an explicit window |
| `--retry-failed` | redo cases whose stored result is an error |
| `--plan` (alias `--dry-plan`) | print calls and tokens the selection would cost; call nothing |
| `--max-calls N` | hard cap on requests that reach the provider this invocation, retries included; default 100 |
| `--pause-ms MS` | least gap between calls; default 0 |
| `--max-attempts N` | tries per call, honouring `Retry-After`; default 5 |
| `--backoff-base-ms`, `--backoff-max-ms`, `--timeout-secs` | backoff first ceiling (2 s), longest wait (90 s), per-request timeout (90 s) |
| `--max-consecutive-failures N` | stop after N provider failures in a row; default 3 |
| `--no-cache` | do not reuse recorded responses |
| `--out DIR` | results, cache and reports; default `target/live-eval` (relative to the working directory) |
| `--report` | aggregate stored results into `REPORT.md` and `report.csv` |
| `--compare provider:model[,..]` | limit a report to these runs |
| `--help`, `-h` | the runner's own help |

**Resume.** Results are an append-only JSONL file, one line per case and mode,
synced before the next case starts: `<out>/results/<provider>--<small>--<main>.jsonl`.
Re-running the same command skips what is stored (only when the settings that change
behaviour hash the same). Responses are also cached under `<out>/model-cache/`, so a
re-run of an unchanged case is free. A crash or Ctrl-C loses at most the case in
flight. `<out>/budget-partial-results.json` is where the call budget keeps its ledger of
partial results.

**Exit codes** (a loop should branch on them): 0 finished, go on to the next batch;
1 an I/O or internal failure; 2 usage or configuration (flags, missing key or tag);
3 `--max-calls` spent, nothing lost; 4 the provider kept failing, wait and re-run;
5 the batch finished but some cases failed, `--retry-failed` after a pause.

**First commands to run** (verified to parse; `--plan` and the mock run were run
offline):

```
# no key, calls nothing: how big is this selection?
cargo run --release -p ferrite-eval --example live_eval -- --plan \
    --provider mock --model x --corpus agentdojo
#   prints e.g. 1046 cases, 2092 runs, about 7,300 model calls typical, 51,254 at most

# the whole flow with no network or key
cargo run --release -p ferrite-eval --example live_eval -- --provider mock --model x \
    --corpus agentdojo --seed 1 --batch-size 12

# one small real batch (needs a key); then repeat for the next batch
cargo run --release -p ferrite-eval --example live_eval -- --provider gemini --model <tag> \
    --corpus agentdojo --seed 1 --batch-size 10 --max-calls 100 --pause-ms 4000

# read what happened
cargo run --release -p ferrite-eval --example live_eval -- --report
```

Not verified: any real provider run, real token counts and step counts (so the
`--plan` figures are estimates), the OS keyring path; `docs/TO-DO.md` T-275, T-278.

---

## 7. Engine probes and `page_shot`

These drive a **real headless Servo session** (software GL, no window), so they need
the Servo build, take a few minutes the first time, and on Linux run under
`xvfb-run` if there is no display. They are not part of check/test/CI. Each prints
what it checked and exits non-zero when a required check fails.

| Recipe | Runs | What it checks |
|---|---|---|
| `just probe-input [url]` | `cargo run -p ferrite-servo --features servo --example input_probe -- [url]` | scrolling, clicking, typing, reload and two tabs against a built-in page (no network). A page you pass must contain `#q` (text input), `#cb` (checkbox) and `#btn` (button) and be taller than the viewport |
| `just probe-web-api` | `... --example web_api_probe` | the Web APIs benchmarks and frameworks assume exist: `window.crypto` (`getRandomValues`, `randomUUID`, `subtle`), observers, `fetch`, custom elements, WebGL, permissions, notifications and more, served from loopback (a secure context), plus a painted-pixel check for inline SVG colour. Required APIs fail the run; `INFO` ones are listed only |
| `just probe-engine` | `cargo run -p ferrite-engine-servo --features engine-servo --example digest_probe` | the real page script: reads the numbered element table of a loopback form, hides the password value, drives `type`, `tick`, `select`, `click`, `scroll` by `@ref` |
| `just probe-profile` | removes `target/profile-probe`, then `profile_probe -- set` and `-- get` with `FERRITE_HOME=<repo>/target/profile-probe` | a cookie and `localStorage` survive a restart (the first process sets them and shuts down cleanly; a fresh process must find them) |
| `just probe-controls` | `... --example controls_probe` | `<select>`, `confirm()`, `prompt()` and a colour input reach the embedder and answers take effect |

### `page_shot`: does this page work in Ferrite?

```
cargo run -p ferrite-servo --features servo --example page_shot -- \
    <url> [wait_ms] [out.png] [width] [height] [scale]
```

(`just page-shot <args>`.) Positional arguments, all optional: URL (default
`https://example.com`), how long to pump the engine in milliseconds (default 8000),
output PNG (default `page_shot.png`), viewport size in device pixels (1280 x 800) and
the display scale (1.0; `2` renders as a Retina screen would, so a 1280-pixel frame
is a 640 CSS-pixel viewport). Prints `URL`, `TITLE`, `VIEWPORT` (inner size and
device pixel ratio), `LOADED <ms>` or `never within <ms>`, a `REQUESTS` count by
kind, up to 300 characters of each `CONSOLE` message, and `FRAME <w>x<h> -> <file>`.
Needs the Servo build and network for a real URL (a sandbox may block a page's asset
hosts; the page then renders unstyled). It also exits cleanly so the profile is
written.

Not verified: these probes were not re-run for this guide. They were run by the
authors of the commits that added them (`docs/PROGRESS.md`); `probe-controls` has no
recipe comment of its own in the source and was read, not run.

---

## 8. Diagnostics

### The log file

Launched with no terminal (Finder, a Start menu, a launcher), a run writes standard
error to a file. From a terminal it prints to the terminal instead and **no file is
written**.

| OS | Folder | File |
|---|---|---|
| macOS | `~/Library/Logs/Ferrite/` (Console.app shows it under Log Reports) | `ferrite.log`, previous run `ferrite.previous.log` |
| Linux | `$XDG_STATE_HOME/ferrite/`, else `~/.local/state/ferrite/` | same names |
| Windows | `%LOCALAPPDATA%\Ferrite\logs` is where the app looks for it, **but the standard-error redirect is Unix-only, so no `ferrite.log` is written on Windows** (`docs/TO-DO.md` T-298) | |

It starts with a banner (`[ferrite] ferrite-shell <version> on <os> <arch> (pid N),
logging to <path>`). Panics always carry a backtrace (`RUST_BACKTRACE=1` is set when
you have not set it). A UI that stops ticking for 8 seconds writes one line
(`the UI has not responded for Ns (a page or the engine is probably stuck)`) and one
when it recovers. Quitting: within 3 seconds of a close request the process ends
whatever the engine is doing (`[ferrite] shutdown did not finish in 3s; ending the
process`). DevTools' **Save log...** and **Open log folder** use this same folder.

The line to look for first on a new machine is the renderer one, printed once per
run:

```
[ferrite-render] GPU rendering (<renderer name>)
[ferrite-render] the GPU path did not pass its self-test (<error>); using the CPU renderer
[ferrite-render] CPU rendering (FERRITE_RENDERER=gpu to try the GPU)
```

`FERRITE_RENDERER=gpu|cpu|auto` (default `auto`; `hardware` and `software` are
accepted spellings). **`auto` tries the GPU on macOS only**; on Linux and Windows it
uses the CPU renderer unless you set `gpu`. The GPU context proves itself by
clearing to a colour and reading it back, and falls back to the CPU renderer on any
failure (ADR-021). **The GPU path has never run on a real GPU or on macOS**; it is
pixel-identical to the CPU path on Mesa's software GL (`docs/PROGRESS.md`).

Related engine settings: the display scale is read from the window and passed to
every tab (a Retina page is laid out at half the pixel width); style and layout
threads follow the core count clamped to 3-8, WebRender and worker pools clamp to
4-8 (`session.rs`; no speed-up was measured).

### `scripts/collect-logs.sh`  (`just collect-logs`)

```
bash scripts/collect-logs.sh
bash /Applications/Ferrite.app/Contents/Resources/collect-logs.sh   # from the packaged app
```

Gathers one zip to send: `ferrite.log` and `ferrite.previous.log` (or a `NO-LOG.txt`
saying there is none), on macOS the three newest `ferrite*.ips` crash reports, a
5-second `sample` of the running process if Ferrite is open **right now** (run it
while the app is frozen), and `system.txt` (macOS: `sw_vers`, `uname`, CPU and
memory, a display listing; elsewhere `uname` and `lscpu`). Writes
`~/Desktop/ferrite-logs-<YYYYMMDD-HHMMSS>.zip` (home folder if there is no Desktop)
and prints its path. Nothing is uploaded; the log can contain page URLs and console
errors. Needs `bash`, `zip`; the sampling part needs macOS. Windows is not
supported by it.

### `just crash-report [report]`  (`python3 scripts/crash_report.py`)

After a crash (exit 139 on macOS): prints the exception and the crashing thread's
stack from the newest `ferrite-shell*.ips` or `.crash` in
`~/Library/Logs/DiagnosticReports/`, or from the report path you pass. Python
standard library only; macOS reports only. Exits non-zero with a message if there
is no report.

### Page console errors

Page console messages are in DevTools (section 4) and mirrored to the log as
`[console:<level>] tab N: ...`. Older builds printed `[page console error]`.

### Common situations

| Symptom | Check |
|---|---|
| Pages are blank or "no real web rendering" | the Servo-free build is running: `just setup-servo`, then `just run-local --servo` (`just doctor` says which build setup recorded) |
| `run-local` exits 2 "not configured" | set both model tags in `env.local`, or pass `--no-model-check` and connect in Settings |
| Agent panel says no model | Settings (Mod+,) or the environment variables of section 5 |
| A page stops responding | the crash banner and DevTools Engine tab; `ferrite.log`; `collect-logs.sh` while it is frozen |
| A packaged app ignores `env.local` | it never reads it; use the Settings drawer |
| Linux: key forgotten after reboot | T-259: export `OLLAMA_API_KEY` / `FERRITE_GEMINI_API_KEY` instead |
| Cargo link failure with a full disk | build and test crate by crate (`cargo test -p <crate>`); `just disk` shows where the space is; `rm -rf target/debug/examples target/debug/incremental` reclaims some |

---

## 9. Quality gates

| Recipe | Runs | What it does | Needs |
|---|---|---|---|
| `just check` | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo machete crates` | structural gate: format, lint (all targets, deny warnings), unused dependencies. Does not build Servo, does not run cargo-deny | `cargo-machete` |
| `just fmt` | `cargo fmt --all` | formats the workspace in place | |
| `just lint` | `cargo clippy --workspace --all-targets -- -D warnings` | clippy alone. `--all-targets` matters: its absence hid a real bug for months (T-207) | |
| `just test` | `cargo test --workspace` | the full suite (unit, integration, doctests). Needs **no network and no API key** (R7) | enough disk to link; if short, per crate |
| `just test-fast` | `cargo test --workspace --lib` | library unit tests only | |
| `just test-live` | `cargo test --workspace -- --ignored` | runs `#[ignore]`d tests. The one ignored test in the workspace is the `ServoEngine` conformance suite, which is compiled only with `--features engine-servo`, so this recipe as written does not reach it; run it as `cargo test -p ferrite-engine-servo --features engine-servo -- --ignored --test-threads=1` (needs the Servo build; `docs/TO-DO.md` T-220) | |
| `just audit` | `cargo deny check` | licenses, RustSec advisories, duplicate versions, sources (`deny.toml`) | network for the advisory DB; `cargo-deny` |
| `just ci` | `just check` then `just test` | the local equivalent of CI's lint and test job (CI also runs `cargo deny` and the two doc checks below) | |
| `just test-local` | four script tests: `scripts/tests/test_env_parser.sh`, `test_run_local.sh`, `test_laya_serve.py`, `test_fetch_checkpoint.py` | tests for the local toolchain scripts. No network; the Python server tests skip themselves unless `fastapi`, `uvicorn` and `laya` import | bash, python3 |

Docs and drift checks (CI runs both on Linux; run them before committing docs):

```
sh scripts/check_purge.sh              # fails if the deleted broker/policy/sandbox/extension architecture reappears outside the allowed history docs
sh scripts/check_no_archive_links.sh   # fails on a markdown link into docs/archive/, or a status/planned heading in CLAUDE.md
```

Both only look at files tracked by git: `git add` a new doc first.

Housekeeping:

| Recipe | What it does |
|---|---|
| `just install-hooks` | sets `core.hooksPath` to `scripts/hooks`: a **commit-msg** hook that rejects AI attribution (a co-author trailer naming an AI assistant, "generated with", a robot emoji, "ai-assisted"; see `scripts/hooks/commit-msg` for the exact pattern) and a **pre-commit** hook that scans staged lines for a real-looking `OLLAMA_API_KEY=` / `FERRITE_GEMINI_API_KEY=` value, then runs fmt and clippy on the staged crates only |
| `just disk` | prints the target dir's size, a per-profile breakdown and the 20 largest artifacts (pure `du`/`find`) |
| `just clean-cache` | `cargo sweep -t 7` on the target dir: prunes build artifacts older than 7 days (needs `cargo-sweep`). Never a blanket `cargo clean` |
| `just bloat [args]` | `cargo bloat --release [args]`: dependency-size report (needs `cargo-bloat`) |
| `just build-servo` | `cargo build -p ferrite-shell --features ferrite-servo/servo`: the Servo build without the setup script; record its cost in `docs/BUILD_BUDGET.md` |

Rules enforced in review (from `CLAUDE.md`): `js.execute` is never scopable;
failures fail to an empty fingerprint, never a bypass; no live network calls in
tests; dependency direction is strictly downward; no dead code; one `rusqlite`
version with `bundled`; no AI attribution in commits.

---

## 10. Release and CI

### Workflows (`.github/workflows/`)

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` (**CI**) | **manual only** (`workflow_dispatch`; Actions tab, Run workflow). No push, pull request or schedule trigger: an owner decision (`docs/TO-DO.md` T-210) | Job 1, on Linux, Windows and macOS: `cargo fetch`; on Linux `cargo fmt --all --check`; clippy (all targets, deny warnings); on Linux `cargo machete crates` (our crates only; `vendor/` is upstream code), `cargo deny check`, `check_purge.sh` and `check_no_archive_links.sh`; `cargo test --workspace`; `cargo build --release -p ferrite-shell` (Servo-free). Job 2 (after job 1 passes everywhere), each OS: builds `cargo build --release -p ferrite-shell --features ferrite-servo/servo`, packages it with `scripts/package.sh` and uploads it as an artifact kept 90 days |
| `release.yml` (**Release**) | automatically when a CI run completes, and only if that run was a manual dispatch that **succeeded** | downloads the three packages CI built and republishes the rolling prerelease tagged `latest` (the old one is deleted first) with the commit, branch, CI run and download notes. It never checks out or runs the CI run's code. A `workflow_run` workflow only fires from the default branch's copy of the file |
| `pages.yml` (**Site**) | manual only | builds `site/` with `python3 site/build.py` (fails on a broken internal link or anchor) and deploys to GitHub Pages. One-time setup: repository Settings, Pages, Source "GitHub Actions" |

To release: run CI from the Actions tab on `main`; when every job is green Release
publishes. Observed through the Actions API on 2026-10-04: the latest `main` runs
(`f4ed744`, 2026-10-02) both succeeded and the `latest` release holds the three
packages. That release was built from `main`, which contains the work up to commit
`8962b01` (the log file and quit watchdog) but not the display-scale fix, GPU
context, page controls or DevTools from this branch.

### Packages: `scripts/package.sh <macos|windows|linux> <label> <binary> [out-dir]`

Packages one release binary into `out-dir` (default `dist/`); CI uses the short
commit sha as the label.

| Platform | Result | Contents |
|---|---|---|
| macos | `ferrite-<label>-macos-arm64.zip` | `Ferrite.app` with `Info.plist` (version from `crates/ferrite-shell/Cargo.toml`), the icon, `scripts/collect-logs.sh` in `Contents/Resources`, ad-hoc signed when `codesign` exists |
| windows | `ferrite-<label>-windows-x64.zip` | `ferrite.exe` (the icon is embedded by `crates/ferrite-shell/build.rs`) |
| linux | `ferrite-<label>-linux-x64.tar.gz` | `ferrite`, `ferrite.png`, `ferrite.desktop`, `README.txt`, `install.sh` |

Exits 1 if the binary is missing and 2 for an unknown platform. Notarization is not
done (`docs/TO-DO.md` T-261).

Not verified here: any workflow run (only the Actions API summary above), the
package script on macOS or Windows runners from this machine, the packaged apps on
real machines.

---

## 11. Environment variable reference

Only the variables below exist in the code or scripts. "App" means the running
browser; "scripts" means `scripts/*.sh`; "eval" means the evaluation examples.

| Variable | Used by | Meaning |
|---|---|---|
| `FERRITE_HOME` | app, scripts | data folder. Scripts default it to `<repo>/.ferrite`; the app, when unset, uses `~/.local/share/ferrite` on every OS |
| `CARGO_TARGET_DIR` | justfile, scripts | build output; default `<repo>/target` |
| `FERRITE_PROFILE` | scripts | `dev` (default) or `release` |
| `FERRITE_PYTHON` | `setup-local.sh` | interpreter for the Laya venv |
| `FERRITE_RENDERER` | app | `gpu`, `cpu` or `auto` (default); `auto` is GPU on macOS only |
| `FERRITE_USER_AGENT` | app | override the User-Agent string (wins over Settings) |
| `FERRITE_DEFENSE` | app, eval | `on` (default; any unrecognised value too), `off`, `sanitizer_only`, `loop_only` |
| `FERRITE_MODEL_SMALL`, `FERRITE_MODEL_MAIN` | app, eval, model examples | model tags; no default |
| `FERRITE_OLLAMA_BASE_URL` | app, eval, model examples | Ollama endpoint; default `https://ollama.com` |
| `FERRITE_GEMINI_BASE_URL` | same | Gemini endpoint |
| `OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY` | same | provider keys (or the keyring) |
| `FERRITE_MODEL_CALL_BUDGET` / `_MAX_IN_FLIGHT` / `_TIMEOUT_SECS` / `_MAX_RESPONSE_BYTES` / `_CACHE_DIR` | same | defaults 500 / 2 / 60 / 1048576 / `~/.cache/ferrite-model` |
| `FERRITE_TWIN_KEY` | app, eval | dry-run twin encryption key (or keyring) |
| `FERRITE_LAYA_URL` and the other `FERRITE_LAYA_*` | app, scripts | section 5; the script-only ones are `FERRITE_LAYA_CHECKPOINT_DIR`, `_HOST`, `_PORT`, `_DEVICE`, `_THREADS`, `_WARMUP`, `_HEAD_MAX_LEN`, `_LOG_LEVEL`, `_OFFLINE`, `_MPS_AMP_MIN_ROWS`, `_PIP_SPEC` |
| `FERRITE_EVAL_OUT_DIR` | `just eval` | report folder; default `target/eval-report` |
| `FERRITE_LIVE_PROVIDER`, `FERRITE_LIVE_MODEL`, `FERRITE_LIVE_SMALL_MODEL`, `FERRITE_LIVE_MAIN_MODEL` | `live_eval` | defaults for `--provider`, `--model`, `--small-model`, `--main-model` |
| `FERRITE_PAGE_SCRIPT_DUMP` | engine | a directory the assembled page scripts are written to, for debugging |
| `RUST_LOG`, `RUST_BACKTRACE` | scripts, app | logging; the app sets `RUST_BACKTRACE=1` if unset |
| `HF_TOKEN` | `setup-local.sh` | Hugging Face token for the Laya checkpoint if needed |
| `NO_COLOR` | scripts | disable colour |
| `XDG_STATE_HOME`, `LOCALAPPDATA`, `HOME` | app, scripts | where the log folder is (section 8) |

`env.local` accepts only `FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and `RUST_LOG`, is
read by the scripts (never by the app itself), and never holds keys.

---

## 12. Where files live

Let `<data>` be `$FERRITE_HOME` if set, else `~/.local/share/ferrite`.

| Path | What |
|---|---|
| `<data>/profile/` | cookies, HSTS, saved HTTP credentials, web storage (written on a clean close) |
| `<data>/settings.json` | the Settings drawer's provider and model choices, browser identity (never a key) |
| `<data>/bookmarks.json` | bookmarks |
| `<data>/chats/` | one `<id>.json` per agent chat |
| `<data>/ui-layout.json` | saved panel sizes |
| `<data>/audit/network.db` | the hash-chained network audit log (the Audit panel's Security log); the OS temp dir if no data folder resolves |
| `<data>/logs/model-activity.jsonl` | the model-call trace (contains prompts and page text) |
| `<data>/cache/favicons/` | favicon cache when `FERRITE_HOME` is set, else `~/.cache/ferrite-ui/favicons` |
| `<data>/env.local`, `build.mode`, `laya/` | created by the scripts under `$FERRITE_HOME` only |
| OS keyring, service `ferrite` | API keys (`OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY`) and `FERRITE_TWIN_KEY` |
| `~/.cache/ferrite-model/` | the model response cache (`FERRITE_MODEL_CACHE_DIR`) |
| the downloads folder | files saved by the Library's Downloads (falls back to home) |
| the log folder | section 8 |
| `target/eval-report/`, `target/live-eval/` | `just eval` / `guard-eval` and the live runner's output |
| `target/profile-probe/` | `probe-profile`'s throwaway profile |

Browsing history is session-only and is not saved.
