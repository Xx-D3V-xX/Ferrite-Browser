# Ferrite — Commands and Usage Guide

This guide shows how to set Ferrite up, run it, use it, evaluate it, diagnose it
and release it. Every command, flag, variable, path and shortcut below was read
from the code or script that implements it. The source is named next to it where
that helps. This guide does not give the status of the work (what is finished,
what is open). That lives in `docs/PROGRESS.md` and `docs/TO-DO.md`. The reasons
for the design live in `docs/DECISIONS.md`.

Some words in this guide are technical. [`docs/GLOSSARY.md`](GLOSSARY.md) explains
all of them in plain words. The most common ones are also explained where they
first appear.

**How this was checked.** The guide was written on a Linux machine. That machine
had no display and no Servo build. Here is what was checked and how:

- Script flags were checked by running each script's own `--help` and `--dry-run`
  modes.
- The live runner's flags were checked in two ways. The argument parser was read.
  The already-built `live_eval` was run: `--help`, `--plan`, and a real
  three-case `--provider mock` batch plus `--report`.
- Shortcuts and menus were read from `crates/ferrite-ui/src`.
- Every recipe was checked with `just --list` and `just --dry-run <recipe>`
  (just 1.58.0). So the command each one runs is as written here.
- No recipe was run through `just`.
- Nothing here was run on macOS or Windows.

Each section ends with what is not verified.

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

- **`just`** is the command surface. A command surface is the one tool you use to
  start everything. `just --list` prints every recipe. A recipe is one named
  command. `just` alone does the same. If you do not have `just`, install it with
  `brew install just` or `cargo install just`. Each recipe below also shows the
  plain command it runs.
- **Two kinds of recipe.**
  - The build, lint, test, eval and probe recipes are plain `cargo` or `python3`
    commands.
  - The local toolchain recipes call bash scripts under `scripts/`. These are
    `setup*`, `run-local`, `run-fast`, `run-all`, `models-local`, `laya-*`,
    `doctor`, `crash-report`, `test-local`, `disk` and `probe-profile`. The scripts
    work with bash 3.2. They are written for macOS first and Linux as best-effort.
    They need a POSIX shell. They have not been run on Windows.
- **"Needs"** in each entry is what the command costs. It can be network, an API
  key, the real Servo build, time or disk.
- **`--dry-run` is not the same as the defense's "dry run".** On a script,
  `--dry-run` means "print what you would do and do nothing". The defense's dry
  run is a practice run of the agent's plan on fake data. The context tells you
  which one is meant.
- **Two builds.**
  - The default build has **no Servo**. Servo is the web engine that draws pages.
    The default build compiles in about two minutes cold (`docs/BUILD_BUDGET.md`).
    It opens the UI. It runs the agent loop and the defense. But it **cannot
    render web pages**. Its engine stub refuses to start and says "compiled
    without the `servo` feature".
  - The **Servo build** adds `--features ferrite-servo/servo`. It renders real
    pages. The first build takes 20 to 60 minutes and 10+ GB
    (`scripts/setup-local.sh --help`). The only measured figure in the repo is an
    older one: a debug build that took 15m31s and 6.4 GB, added to an existing
    target dir (`docs/BUILD_BUDGET.md`).
  - Everything that talks to a page needs the Servo build. That includes every
    `probe-*` recipe and `page_shot`.
- **Profiles.** A profile is a set of build settings. `dev` is the default. It
  builds faster. `release` is what you use to *use* the browser or to judge its
  speed. The engine runs several times faster when optimised (`docs/TO-DO.md`
  T-269). `FERRITE_PROFILE=release` selects it for the scripts. `just run-fast`
  does that for you.
- **Build output** goes to `target/`. You can override it with `CARGO_TARGET_DIR`.
  The justfile exports `<repo>/target` as the default. The scripts do the same.
  All local state goes to `$FERRITE_HOME`. The scripts default it to the gitignored
  folder `<repo>/.ferrite/`.

---

## 2. First-time setup

You need these tools first:

- `git`;
- `rustup`, with the toolchain in `rust-toolchain.toml` (stable, with `rustfmt`,
  `clippy`, `llvm-tools`);
- `python3` (for scripts, corpus tools and Laya);
- a C toolchain, plus `cmake` and `pkg-config`.

Some tools are optional:

- `just`;
- `cargo-deny` (for `just audit`);
- `cargo-machete` (for `just check`);
- `cargo-bloat` (for `just bloat`);
- `cargo-sweep` (for `just clean-cache`).

```
brew install just cargo-deny              # or: cargo install just cargo-deny
cargo install cargo-machete --locked
just install-hooks                        # git hooks, see section 9
```

### `just setup [flags]`  (`./scripts/setup-local.sh [flags]`)

In short: this is the first command to run. It checks your tools and builds the
app without Servo.

What it does:

- It checks your toolchain.
- It builds `ferrite-shell` without Servo.
- It can set up Laya. Laya is a small local model that helps pick the next click.
- It creates `$FERRITE_HOME/env.local` from `scripts/local.env.example`. The file
  has mode 600. It is never overwritten.

It is safe to run again. Every step checks first and skips what is done. Nothing is
deleted or overwritten. It never uses `sudo`. It offers to install rustup and brew
packages. It does so only after you say yes or pass `--yes`.

| Flag | Effect |
|---|---|
| `--with-servo` / `--no-servo` | build with / without the real Servo engine. With neither flag, an interactive run asks you. A non-interactive run (no TTY, `--yes`, `--dry-run`) builds without Servo |
| `--no-build` | skip the cargo build |
| `--laya` / `--no-laya` | set up / skip the local Laya server (default: on) |
| `--laya-checkpoint NAME` | `v10s` (default, 322M, faster) or `v10` (421M) |
| `--yes`, `-y` | answer yes to every install question |
| `--dry-run` | print every action it would take, and run none |
| `-h`, `--help` | the script's own usage text |

Environment variables it uses:

- `FERRITE_HOME`
- `CARGO_TARGET_DIR`
- `FERRITE_PYTHON` (the interpreter used for the Laya venv)
- `FERRITE_LAYA_PIP_SPEC` (default `laya[serve]>=0.3.21,<0.4`)
- `FERRITE_PROFILE`
- `HF_TOKEN` (needed only if a Hugging Face download needs a login)

What it may write:

- `$FERRITE_HOME`: the Laya venv and checkpoint, `env.local`, pid and log files,
  and `build.mode`.
- `$CARGO_TARGET_DIR`.
- Only after a question or `--yes`: rustup and brew packages.

`rm -rf .ferrite` removes all local state.

Expected output, from `just setup --dry-run --yes --no-laya` on Linux:

- A header with the repo, `FERRITE_HOME`, the target dir and the platform.
- These sections: `git`, `system packages`, `Rust toolchain`, `Build ferrite-shell`
  (it prints `Servo-FREE build: fast (~2 min cold), but it has NO real web
  rendering`), `Laya` and `Local environment file`.
- A reminder that these scripts never write keys.

Needs: network (rustup, crates.io, brew, pip, and Hugging Face for Laya). It takes
roughly 2 minutes for the build without Servo. The Laya checkpoint download takes
more.

### `just setup-servo [flags]`

In short: the same as `just setup`, but with the real web engine.

This runs `setup-local.sh --with-servo`. That builds with the real Servo engine
(`cargo build -p ferrite-shell --features ferrite-servo/servo`).

- It needs 20 to 60 minutes and 10+ GB the first time.
- Later builds are incremental. Ctrl-C is safe. Run it again and it continues
  where cargo stopped.
- On Linux the script warns that Servo also needs its own system libraries. It
  links to Servo's setup page. It does not install them for you.
- Use `FERRITE_PROFILE=release just setup-servo` to build the release profile
  instead. It is slower to build and much faster to run.

### `just setup-all [flags]`

In short: Servo and Laya in one command.

This runs `setup-local.sh --with-servo --laya`. Everything stays inside this
folder.

### `just doctor [--fix-hints] [--no-probe]`  (`./scripts/doctor.sh`)

In short: a checklist that tells you what is ready on this machine.

It is read-only. It changes nothing. It never prints an API key. It has these
sections:

- `Tools`: git, cargo, rustc, just, python.
- `Build`: whether the binary exists for the chosen profile. Also whether setup
  recorded a Servo build or a plain build.
- `Configuration`: the `env.local` file, `FERRITE_MODEL_SMALL` and
  `FERRITE_MODEL_MAIN`, `OLLAMA_API_KEY`, `FERRITE_TWIN_KEY`.
- `Laya (optional)`: the venv, the checkpoint, and a latency probe of a running
  server.
- `Disk`: free space where `FERRITE_HOME` and the target dir live, and the size of
  the target dir.

Flags and results:

- `--fix-hints` prints how to fix each problem.
- `--no-probe` skips the Laya latency-probe requests.
- It exits with 0 if nothing is `FAIL`. Warnings are allowed. Otherwise it exits
  with 1.
- The output ends with a line like `2 problem(s), 7 warning(s).`
- A missing model tag is a `FAIL`. This is so even though you can now connect the
  app from its Settings drawer instead (section 5). The doctor checks only the
  environment route.
- On Linux it says that the OS keyring is not checked.

Not verified: `setup` or `setup-servo` from start to end on macOS from a clean
machine in this session. Only `--dry-run` was read and run, on Linux. The Laya
parts were also not verified.

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

In short: the quickest way to open the UI. Pages will not draw.

This runs the shell binary. This recipe cannot turn on Servo, because it never
passes `--features`. So pages do not render. Use it for the UI, the agent loop and
the defense. Arguments after `run` go to `ferrite-shell`. With no arguments, or
with a first argument it does not know, it starts the UI.

### `just run-local [flags] [-- shell args]`  (`./scripts/run-local.sh`)

In short: the everyday launcher. It loads your settings and starts Laya for you.

It does these steps in order:

1. It loads `$FERRITE_HOME/env.local`. The file is plain `KEY=VALUE`. It is read as
   data and never run as a script. Only `FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and
   `RUST_LOG` are used. Anything already exported in your shell wins.
2. It checks that `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` are set. If not,
   it **exits with 2 and prints instructions**. Pass `--no-model-check` to open the
   UI anyway. Then connect a model from the Settings drawer.
3. It prints a note about `OLLAMA_API_KEY`. It never prints the value. The macOS
   keychain is checked on macOS only.
4. It starts the local Laya server in the background. It does this only if Laya is
   set up and not already running. It waits for `/health`. It exports
   `FERRITE_LAYA_URL`.
5. It runs `cargo run [--release] -p ferrite-shell [--features ferrite-servo/servo] -- ui`.
6. When the app exits, or on Ctrl-C or TERM, it stops the Laya server that it
   started. It never stops a server that was already running.

| Flag | Effect |
|---|---|
| `--no-laya` | do not start or use Laya (LLM only). An inherited `FERRITE_LAYA_URL` is dropped |
| `--servo` / `--no-servo` | force the Servo / Servo-free build. The default is what `setup` recorded in `$FERRITE_HOME/build.mode` (Servo-free if there is none) |
| `--wait SECONDS` | how long to wait for Laya's `/health` (default 120; the first start loads the model) |
| `--no-model-check` | start even if the model tags are unset |
| `--list-models` | list the tags your Ollama endpoint serves, then exit (this is `just models-local`) |
| `--laya-only` | run only the Laya server, in the foreground (this is `just laya-serve`) |
| `--verify` | send one recorded step to the Laya server and print its decision and latency (this is `just laya-verify`) |
| `--dry-run` | print what would happen; start nothing |
| `-- ARGS...` | pass `ARGS` to `ferrite-shell` instead of `ui` (`window`, `jstest`, `agent-smoke`, `smoke`) |

Environment: `FERRITE_PROFILE=dev` (default) or `release`. Here is a dry run, as an
example. `just run-local --dry-run --no-model-check --no-laya --servo` prints
`cargo run -p ferrite-shell --features ferrite-servo/servo -- ui`.

### `just run-fast [flags]`

In short: the real browser, built for speed.

This runs `FERRITE_PROFILE=release ./scripts/run-local.sh --servo [flags]`. The
release engine build is slow the first time. The effect on speed was **not
measured** in this repo (`docs/TO-DO.md` T-269). The reason for the release
profile was the measured weakness of the dev profile. It was not a benchmark of
this recipe.

### `just run-all [flags]`

In short: the real browser in the dev profile, with Laya.

This runs `./scripts/run-local.sh --servo [flags]`. That is the real-Servo browser
in the dev profile plus the local Laya server.

### The `ferrite-shell` subcommands

The first argument selects one. None takes flags (`crates/ferrite-shell/src/main.rs`).
Run one directly with
`cargo run -p ferrite-shell --features ferrite-servo/servo -- <subcommand>`
(or without `--features` where noted). Or pass it after the recipe's `--`, as in
`just run-local -- jstest`.

| Subcommand | What it does | Needs |
|---|---|---|
| `ui` (default) | the browser (Iced window, tabs, agent panel) | Servo build for pages |
| `window` | runs `ServoShell::new().run()` (`ferrite-servo/src/shell.rs`). That is a bare winit window that hosts the engine. It has none of Ferrite's tabs, address bar or agent. Not exercised in this pass | Servo build |
| `jstest` | opens a headless Servo session. It loads `https://example.com`, `https://lite.duckduckgo.com` and `https://doc.rust-lang.org`. It prints a table (JS executed, title, console errors). It writes `../paper/data/js_compat_baseline.csv` relative to the working directory. It warns if it cannot create the folder | Servo build, network |
| `agent-smoke` | runs the agent loop against the in-process `MockEngine` with a real model provider. The task is "What is the title of the page at https://example.com?". It prints `[agent-smoke] actions taken`, then `final_response` or `asked_user`. It exits 0 with a "skipping" message if no model is configured or reachable (`FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` unset, or no Ollama or Gemini credential). It exits 1 if the loop does not finish cleanly | model tags, a key or local Ollama, network |
| `smoke` | adds two events to a temporary SQLite audit log (`ferrite_smoke_test.db` in the OS temp dir). It checks the hash chain. It prints `Smoke test: ALL CHECKS PASSED` | nothing |

`ui` ends the process on its own when the window closes (section 8 says why). The
process exit code is 1 only if the UI returned an error.

### Packaged apps

The release packages are:

- `ferrite-<sha>-macos-arm64.zip`
- `...-windows-x64.zip`
- `...-linux-x64.tar.gz`

They hold the Servo build in the release profile. They run `ui` with no arguments.

**A packaged app does not read `env.local`.** The Linux package's own `README.txt`
says that it does. That is wrong (`docs/TO-DO.md` T-299). A packaged app also does
not read any `FERRITE_*` variable that you did not export before you started it.
Connect a model from its Settings drawer (section 5).

Notes for each system:

- **macOS:** the app is ad-hoc signed. It is not notarized. So on first launch you
  must open System Settings, then Privacy & Security, then Open Anyway. Or run
  `xattr -dr com.apple.quarantine /Applications/Ferrite.app` (`docs/TO-DO.md`
  T-261).
- **Windows:** SmartScreen may ask once.
- **Linux:** it needs a graphical session with OpenGL/EGL. It also needs the shared
  libraries named in the package's `README.txt`. `./ferrite` runs it.
  `./install.sh` copies it into `~/.local`.

Not verified: any of this on macOS or Windows. Also not verified: the speed of
`run-fast`, and the packaged Linux and Windows builds on a real machine.

---

## 4. Using the browser

### Window and tabs

The window has a 34 px tab strip, a 40 px toolbar and the page.

- On macOS the title bar is transparent. The tab strip holds the traffic lights.
  Linux and Windows keep their own window decorations.
- Tabs share the strip equally down to a minimum width. Then the strip scrolls.
- A long title is cut with an ellipsis.
- The close button shows on the active tab or the hovered tab.
- Middle-click closes a tab.
- The empty part of the strip drags the window. A double-click maximizes it.
- A coloured dot on a tab says the page stopped responding (red) or is waiting for
  you (accent).
- A page that opens a new window (`window.open`, `target=_blank`) gets a tab.

The toolbar, from left to right, has these parts:

- back;
- forward;
- reload (or stop, while loading);
- the address bar;
- the **Agent** toggle (a dot appears when something needs you);
- the **Audit** shield (`F12`);
- the overflow menu.

The address bar works like this:

- A URL with a scheme is used as it is. So are `about:` pages.
- A bare host or path with a dot (and no spaces) gets `https://`. `localhost` and
  IP literals get `http://`.
- **Anything else is a Google search** (`https://www.google.com/search?q=...`). The
  agent's own search prompt still points at DuckDuckGo Lite. This is on purpose
  (`docs/TO-DO.md` T-279).
- A click selects everything. Enter goes to the page and hands the keyboard back to
  the page. The zoom chip and the bookmark star sit inside the bar.

A thin sweeping bar over the toolbar's bottom edge shows loading. A new tab shows a
page with a search box and quick-access tiles.

The **overflow menu** (the dots) holds these items:

- New tab
- a zoom control
- Find in page
- Bookmark this page (only on a real page)
- Bookmarks
- History
- Downloads
- Developer tools
- Switch to dark/light theme
- Settings

Bookmarks, History and Downloads share one drawer, called the Library. Downloads
has a manual "Download current page" action. Clicks on `<a download>` links are not
caught (`docs/TO-DO.md` T-232).

### Keyboard shortcuts

`Mod` means **Cmd on macOS and Ctrl everywhere else**. The list comes from
`handle_key_press` in `crates/ferrite-ui/src/lib.rs`. The shortcuts also work while
the address bar or the agent box has focus. Editing keys such as Mod+A, C, V, X and
Z still reach the field.

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
| Esc | closes things in this order of priority: a page control that the page is waiting on (cancel), the menu, the find bar; answers the agent's question with **no**; cancels the pre-run consent panel; stops loading; unfocuses the address bar |

Typing, pointer and wheel input go to the page. The exceptions are when a field of
the browser has focus, and when a page control (below) is open. Cmd/Ctrl+C, X and V
reach the page's focused field.

What is not handled:

- IME composition and HTTP basic-auth prompts (`docs/TO-DO.md` T-294).
- Non-US layouts, dead keys and key repeat. These are unobserved (T-236).

### Developer tools

Open them with Cmd+Opt+I or Ctrl+Shift+I, with Mod+J, or with the menu item
Developer tools. They are a bottom panel with three tabs. Counts show on the tab
labels.

- **Console** shows every message that the active tab's page printed, at all
  levels, per tab.
  - Limits: up to 5,000 rows per tab, 4,000 characters per message, and 8 MiB of
    text per tab. The oldest are dropped first.
  - It has a filter box and level chips (All, Errors, Warnings, Info, Debug).
  - It has a "Preserve log" toggle. That keeps messages across navigation.
  - Long messages are folded. An expand control opens them.
  - It has a **prompt** (`Run JavaScript on this page`). Type an expression and
    press Enter or Run to run it in the page. Up and Down recall earlier input (up
    to 100). This is you running script in the page. It is not the agent.
- **Network** shows each request that the tab made, as it starts (method, URL,
  kind). It has a filter box and kind chips (All, Document, Script, Style, Image,
  Font, Media, Other). It shows **no status, size or timing** yet, because the
  engine does not report them (`docs/TO-DO.md` T-292). `fetch()` and XHR fall
  under Other.
- **Engine** shows panics on engine threads and pages whose script thread died,
  in this session. It shows the thread, the message and the location.
- The header buttons are:
  - **Copy all**: the active tab as text, to the clipboard.
  - **Save log...**: writes
    `ferrite-<console|network|engine>-tab<N>-<YYYYMMDD-HHMMSS>.log` into the log
    folder (section 8).
  - **Open log folder**: it uses `open` on macOS, `explorer` on Windows and
    `xdg-open` elsewhere.
- Console lines are also copied to standard error. So they go into the log file.
  The format is `[console:<level>] tab N: ...`. At most 20 lines go out per UI
  tick.

### Panels

The agent, Library and Settings drawers share the right-hand side. Only one shows
at a time, beside the page. DevTools and Audit share the bottom.

- **Drag the splitter** between a panel and the page to resize it.
- **Double-click** the splitter to reset it.
- Side drawers are 300 to 760 px wide (default 380).
- Bottom panels are 140 to 720 px tall (default 260).
- The page always keeps at least 360 px by 160 px.
- Sizes are fitted to the real window every time. They are saved when you release
  the mouse, in `ui-layout.json` under the data folder (section 12).

### The agent and consent

Open the agent panel with the toolbar **Agent** button or Mod+Shift+A. Type a task
and send it. Each run is a chat. Chats are saved and listed. The panel says plainly
when no model is connected. It has a button to Settings. The agent's answer is
shown as Markdown. No HTML is run. No images are fetched. A link shows the site it
really goes to. It opens only when you click.

Here is what you may be asked, in order. A "consent" step is a question that the
user must answer before an action runs.

1. **Before the run** (the pre-run consent panel). The dry run is a practice run
   that does nothing real. After it, anything the agent tried beyond the predicted
   fingerprint is listed as cards. The fingerprint is the list of tools and
   websites the task is expected to need. Each card has **Reject** and **Approve**.
   **Proceed** and **Cancel** stay pinned under the list. Proceed needs every item
   decided. Card edge colours: amber means undecided, green means approved, red
   means rejected. The surface is neutral.
2. **During the real run.** An action outside the prediction pauses the run. A card
   names only the action and the site. It never shows page text. The buttons are
   **Don't allow** (Esc), **Allow once** and **Allow for task**. `js.execute` is
   always asked (`docs/DECISIONS.md` ADR-003, ADR-020). An ADR is one design
   decision with its reasons.
3. **Sign-in.** A page may ask for a password or another secret. Or it may be a
   known sign-in host. Then the run pauses before any model call. You sign in
   yourself. Then press **I've done it, continue**, or stop the task. The agent
   never types into password fields (ADR-018).

The **Audit** panel (the shield, or F12) has two views:

- **Model calls** is a timeline of every agent step, every LLM request (full prompt
  and answer) and every Laya request. It shows timings. It shows whether Laya's
  answer was used. Click a row for the payloads. A summary line says whether Laya
  is paying for itself.
- **Security log** is the hash-chained record of network requests. Each entry is
  tied to the one before it by a hash. So a change to an old entry can be seen.

The model calls are also appended to `<data dir>/logs/model-activity.jsonl`. That
file holds prompts and page text. Treat it like the pages themselves. It never
leaves your machine.

### What a page can ask the browser for

Ferrite draws these as overlays: dropdown lists (`<select>`), `alert()`,
`confirm()`, `prompt()`, the colour picker, the file picker and the page's context
menu.

- The page is blocked until you answer. Pages get no pointer or key input while
  one is open.
- Page text in them is drawn as plain text. A dialog says which site it comes from.
- The **file picker is a card where you type or paste the path or paths**. There is
  no native file dialog yet.
- The page's cursor shape (pointer, text, crosshair, ...) is followed. `cursor:
  none` shows the ordinary arrow.

A **page crash** means its script thread died. The page stays frozen on screen with
a banner. The banner has three buttons:

- **Reload** (a fresh session for the same address).
- **Details** (the reason and a backtrace, with Copy).
- **Dismiss**.

The crash is also in the DevTools Engine tab and in the log.

### Settings (Mod+,)

- **Model:**
  - Choose Ollama Cloud, a local Ollama (a loopback address only), or Gemini.
  - Paste the API key.
  - **Load models** asks the provider what the key can use. This also proves the
    key before a task spends a call.
  - Pick the fast model and the agent model (or one model for both).
  - **Save and use** applies at once. No restart is needed.
  - The key goes to the OS keyring under service `ferrite` (account
    `OLLAMA_API_KEY` or `FERRITE_GEMINI_API_KEY`). It never goes to a file.
  - **Remove key** deletes it and disconnects.
  - The environment still wins over the saved choice. The drawer says when it does.
  - On Linux the key store is the kernel keyring. It is **cleared on reboot**.
    Export the variable for a permanent key (`docs/TO-DO.md` T-259).
- **Appearance:** theme, and the default zoom for new tabs.
- **Browser identity:**
  - Firefox-compatible is the default. It uses Servo's own User-Agent, with the
    engine token swapped for `Gecko`.
  - Ferrite names its engine.
  - The choice applies at the next launch.
- A footer lists where the settings file, the key and the response cache are.

### Logins and cookies

The browser profile is `<data dir>/profile`. It holds cookies, HSTS, saved HTTP
credentials and web storage. It survives restarts. **Servo writes it when the
window closes normally.** A crash, `kill` or macOS Cmd+Q may lose the new cookies of
that run. The Cmd+Q path is untested (`docs/TO-DO.md` T-295). Whether a site
accepts a sign-in is a separate question. Google in particular may refuse an
embedded engine (`docs/TO-DO.md` T-267). `FERRITE_USER_AGENT` overrides the
User-Agent string. It wins over the Settings choice.

Not verified: all of section 4 on macOS or Windows; HiDPI (a high-density screen).
The page-control overlays were not checked against a real page in the app. They
were driven under Xvfb at scale 1 and in the engine probe `controls_probe`. The
file picker's answer path is untested.

---

## 5. Models, keys and the agent

The agent needs a model provider. A provider is the service that runs the model.
There are three ways to give the agent one. The strongest comes first.

1. **Environment variables.** `run-local` also reads them from
   `$FERRITE_HOME/env.local`. Set `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN`.
   These are model tags. A model tag is the model's name at the provider. **No
   model name is ever a default.** The two may be the same. You also need a key.
2. **Settings drawer.** This works in a packaged app. It saves to `settings.json`.
   The key goes to the keyring.
3. Nothing. The agent panel says no model is connected. The defense logic still
   fails closed. An empty fingerprint sends everything through consent.

Keys come from the environment or the OS keyring and nowhere else.

| Provider | Key | Where the key can live | Notes |
|---|---|---|---|
| Ollama Cloud (`https://ollama.com`) | `OLLAMA_API_KEY` | environment, or keyring service `ferrite` account `OLLAMA_API_KEY` | default endpoint |
| Ollama local | none | | `FERRITE_OLLAMA_BASE_URL=http://localhost:11434`; the bearer token is never sent to a non-cloud host |
| Gemini | `FERRITE_GEMINI_API_KEY` | environment, or keyring account `FERRITE_GEMINI_API_KEY` | base URL `FERRITE_GEMINI_BASE_URL` (default `https://generativelanguage.googleapis.com/v1beta/models`) |

If you saved no choice and both providers are set up, the app tries Ollama first,
then Gemini. Only Ollama and Gemini exist. An OpenAI-compatible backend does not
exist. An Anthropic backend does not exist (`docs/TO-DO.md` T-277).

To store a key in the macOS Keychain, run the command below. It asks you for the
value. On first use, macOS may ask whether `ferrite-shell` may read the key. A
rebuilt binary can trigger that question again.

```
security add-generic-password -U -s ferrite -a OLLAMA_API_KEY -w
```

These model-layer settings are all optional. The defaults come from
`crates/ferrite-model/src/config.rs`.

- `FERRITE_MODEL_CALL_BUDGET` (500)
- `FERRITE_MODEL_MAX_IN_FLIGHT` (2)
- `FERRITE_MODEL_TIMEOUT_SECS` (60)
- `FERRITE_MODEL_MAX_RESPONSE_BYTES` (1048576)
- `FERRITE_MODEL_CACHE_DIR` (`~/.cache/ferrite-model`)

`FERRITE_DEFENSE` selects the defense mode for the live app. If it is unset, or has
a value the app does not know, you get the full defense. The values `off`,
`sanitizer_only` and `loop_only` are for ablations only. An ablation turns parts of
the defense off to see what each part does. `FERRITE_TWIN_KEY` is the encryption
key of the dry-run twin. The twin is the fake copy of data that the dry run uses.
The key comes from the keyring account `FERRITE_TWIN_KEY`, else from the variable.
Without either, the dry run loses caching. The defense itself is not weakened.

### Model recipes

| Recipe | Runs | What it does | Needs |
|---|---|---|---|
| `just models` | `cargo run -p ferrite-model --example models` | prints every tag that the configured **Ollama** endpoint serves, one per line (`(no tags served)` if none). It does not list Gemini | network, a key unless local; **both** `FERRITE_MODEL_SMALL` and `FERRITE_MODEL_MAIN` set (any placeholder will do, the loader insists) |
| `just models-local` | `./scripts/run-local.sh --list-models` | the same, after it loads `env.local` and supplies placeholders for the tags | network, a key or local Ollama |
| `just probe` | `cargo run -p ferrite-model --example probe` | one live round trip to Ollama. It prints `provider`, `tag`, `tokens` (prompt + completion) and `content` for "Reply with exactly one word: hello". It exits 1 on any error | network, a key, `FERRITE_MODEL_SMALL` set to a served tag; not part of check/test/CI |
| `just record ollama\|gemini "prompt"` | `cargo run -p ferrite-model --example record -- <provider> "<prompt>"` | records one live response as a fixture under `crates/ferrite-model/tests/fixtures/model/` for offline tests. The prompt defaults to the one-word reply. The first argument must be `ollama` or `gemini`. You review and commit the fixture by hand | network, a key, config |
| `just cache-stats` | `cargo run -p ferrite-model --example cache_stats` | works offline. It prints `cache dir`, `entries`, `size` in bytes, the last run's hits, misses and uncacheable calls, and the hit rate (`n/a` before any cacheable call). It reads `~/.cache/ferrite-model/` or `FERRITE_MODEL_CACHE_DIR`. "A low hit rate is a bug" | nothing |

A cache keeps the answers to earlier model calls. So the same call can be answered
again without the model.

### Laya (optional local decision model)

[Laya](https://github.com/NandhaKishorM/laya) scores "which operation, which
element" for the next browser step. Ferrite talks to it only when `FERRITE_LAYA_URL`
is set. With it unset, the app uses the LLM only. `just setup` builds a venv under
`$FERRITE_HOME/laya` and downloads the checkpoint. `just run-local` starts a server
on `127.0.0.1:8765`.

| Recipe | What it does |
|---|---|
| `just laya-serve` | runs only the Laya server, in the foreground (Ctrl-C stops it). It refuses to start if one is already running, or if something answers at the URL |
| `just laya-verify` | runs `./scripts/run-local.sh --verify`. It sends one recorded browser step to the server and prints its decision and latency. It exits non-zero with a suggested fix if the server cannot be reached |
| `just test-local` | tests for the toolchain scripts themselves (section 9) |

Laya variables (the full list with comments is in `scripts/local.env.example`):

- `FERRITE_LAYA_URL`
- `FERRITE_LAYA_HOST` and `FERRITE_LAYA_PORT` (default 127.0.0.1:8765). The server
  refuses a non-loopback host unless an API key is set.
- `FERRITE_LAYA_CHECKPOINT` (`v10s` or `v10`)
- `FERRITE_LAYA_DEVICE` (empty means auto, or `mps`, `cuda`, `cpu`)
- `FERRITE_LAYA_THREADS`
- `FERRITE_LAYA_MPS_AMP_MIN_ROWS`
- `FERRITE_LAYA_WARMUP` (0 skips the start-up warm-up)
- `FERRITE_LAYA_HEAD_MAX_LEN`
- `FERRITE_LAYA_LOG_LEVEL`
- `FERRITE_LAYA_OFFLINE`
- `FERRITE_LAYA_API_KEY` (export it; do not write it to a file)
- The app-side variables: `FERRITE_LAYA_TIMEOUT_MS` (default 1500),
  `FERRITE_LAYA_MODEL`, `FERRITE_LAYA_OP_GATE` (0.80) and
  `FERRITE_LAYA_TARGET_GATE` (0.60).

The warm-up logs the device and the milliseconds per step. The app stops asking
Laya by itself when it does not pay for itself (ADR-015). It is not established
that Laya makes the app faster on a given machine. Measure it with the summary line
of the Audit panel (`docs/TO-DO.md` T-234, T-250).

Not verified when this guide was written: any live provider call (Ollama Cloud or
Gemini) by the author of this guide, the keyring write on any platform, and Laya.
The owner has since run `gemma4:31b` through Ollama in the live evaluation runner
(section 6).

---

## 6. Evaluation

All of these run from the repo root. Reports go to `target/eval-report/` unless
said otherwise. Read `docs/EVALUATION.md` for what the numbers show and do not
show. An evaluation here means: run test cases through the defense and count what
happened.

### What the real run found

The owner ran **all 1,984 cases with a real model**. The model was `gemma4:31b`,
served by `ollama`. It played both roles. The run used the guard off and the guard
on, so it has 3,968 runs. The guard is the check that stops an action outside the
predicted fingerprint. The full report is
`docs/results/live-eval-ollama-gemma4-31b.md`. `docs/EVALUATION.md` §11 explains it.

- **No guard:** 31/1542 = 2.0% of the measurable attack runs tried the attack. They
  also ran it.
- **With the guard:** 6/1542 = 0.4% ran the attack. The guard stopped 25 of those
  31.
- **Normal tasks:** with the guard, 86/226 = 38.1% of the benign (normal) tasks had
  an action refused. That is a false positive. The guard stopped something the user
  wanted.
- **Limits:** one model, one attack template, a practice engine that does nothing
  real, and a simulated user who refuses everything. A real user who approves
  prompts would move these numbers toward the no-guard result. Do not treat the
  numbers as a general result.

### `just eval`  (`cargo run -p ferrite-eval --example eval`)

In short: run the offline test corpus through every defense mode and write a
report.

It runs the **938-case corpus** through the real pipeline in every defense mode. A
corpus is a set of test cases. The 938 cases are `corpus` 16 + `pilot_corpus` 10 +
`agentdojo_corpus` 3 + `corpus_redteam` 909. They are all under
`crates/ferrite-eval/tests/`. It writes `EVAL_REPORT.md`, `eval_report.csv`,
`corpus.db` and `audit.db` to `target/eval-report/` (or `$FERRITE_EVAL_OUT_DIR`).

- The agent in it is **scripted**. It is a worst-case agent that is derived from
  each case's ground truth. It never calls a model. Ground truth is the correct
  answer that the case's author wrote down.
- With no model configured, the fingerprint layer uses rules only. The whole run
  makes zero network calls.
- **If `FERRITE_MODEL_SMALL`, `FERRITE_MODEL_MAIN` and a key are set, the
  fingerprint prediction goes through the real provider.** Those are live calls.
  Unset them for the run that you can reproduce.
- It is not part of check, test or CI.
- Needs: nothing by default.

### `just guard-eval`  (`cargo run -p ferrite-eval --example guard_eval`)

In short: test the guard in the case where the dry run could not have seen the
attack.

This is the runtime-guard experiment (ADR-014, `docs/EVALUATION.md` section 8.4).
It shows what the real run does when the dry run could not have seen the attack.
It prints a table. It writes `target/eval-report/GUARD_REPORT.md`. It uses the mock
provider only. A mock provider is a stand-in that makes no network calls. So it
needs no network and no key.

### Corpus tools

| Command | What it does |
|---|---|
| `just redteam-corpus` (`python3 scripts/gen_redteam_corpus.py`) | writes the 909 red-team cases again into `crates/ferrite-eval/tests/corpus_redteam/`. It is deterministic: the ids are uuid5 of the case name. It builds them from a matrix of tasks, attacker goals, carriers, payload disguises and scopes |
| `just redteam-corpus-check` (`... --check`) | fails if the files on disk differ from a fresh generation |
| `cargo run -p ferrite-eval --example inspect_case -- <path-to-case.json>` | runs one authored case through every mode that its run label defines. It prints each layer's verdict. It prints whether the verdict matches the case's ground truth (also `just inspect-case <path>`) |
| `cargo test -p ferrite-ipi --test red_team_sanitizer` | the sanitizer red-team suite |
| `cargo test -p ferrite-core --test red_team_scope` | the origin-scope red-team suite |

### AgentDojo import: `scripts/import_agentdojo.py`

AgentDojo is a public set of agent attack tests. This script reads AgentDojo's task
files. It parses them. It never runs them. It writes
`crates/ferrite-eval/tests/agentdojo_full/` (**1,046 cases**: 949 attack cases plus
97 benign twins) and `agentdojo_full_manifest.json`. The import is pinned to
AgentDojo commit `089ed468cf3ed0322acc66b0211f26d9d90dbf60`, benchmark v1.2.2. The
script uses the Python standard library only. The dataset itself is not in this
repository.

```
git clone https://github.com/ethz-spylab/agentdojo /tmp/agentdojo
git -C /tmp/agentdojo checkout 089ed468cf3ed0322acc66b0211f26d9d90dbf60
python3 scripts/import_agentdojo.py --src /tmp/agentdojo --check   # fail if files on disk differ
python3 scripts/import_agentdojo.py --src /tmp/agentdojo           # (re)write the corpus
```

Flags:

- `--src SRC` (required)
- `--out OUT`
- `--benchmark-version`
- `--attack {direct,ignore_previous,important_instructions,injecagent,system_message}`
  (default `important_instructions`)
- `--check`
- `--allow-other-commit` (the output must not be committed)

If `--src` is missing, or points at another commit, the script exits with 2. It
prints the clone and checkout commands. You can check the committed files offline
with `cargo test -p ferrite-eval --test agentdojo_full_validate`. `just
agentdojo-check <src>` wraps the `--check` line. Needs: git and network for the
clone. It needs no cargo.

### The live runner: a real model in the loop

In short: this runs each test case with a real model as the agent, and stores the
results so you can stop and resume.

```
cargo run --release -p ferrite-eval --example live_eval -- [flags]
```

You can also use `just live-eval [flags]`. `just live-eval-plan [flags]` adds
`--plan`. For each case, the runner runs the app's own agent loop against the
dry-run engine. The injection is planted in what the agent reads. Two roles can each
be a model or not:

- the **predictor** (`--predictor llm|rules`) builds the fingerprint;
- the **agent** (`--agent llm|scripted`) chooses the actions.

The full method is in `docs/EVALUATION.md` section 9. Tests cover every behaviour
against the `mock` provider and a loopback fake server. The owner has now run it
with a real model (see "What the real run found" above, and `docs/TO-DO.md` T-275,
T-278).

Run `--plan` first. It prices a selection and calls nothing. It needs no key. A real
call needs two things. The first is a key in `OLLAMA_API_KEY` or
`FERRITE_GEMINI_API_KEY`, or in the keyring. A key is never a flag. It is never
printed or stored. The second is the model tags. There is no default.

| Flag | Meaning |
|---|---|
| `--provider gemini\|ollama\|mock` | the backend. It is required and never has a default (env `FERRITE_LIVE_PROVIDER`) |
| `--model TAG` | one tag for both roles (env `FERRITE_LIVE_MODEL`) |
| `--small-model TAG` | the predictor (env `FERRITE_LIVE_SMALL_MODEL`, then `FERRITE_MODEL_SMALL`) |
| `--main-model TAG` | the agent (env `FERRITE_LIVE_MAIN_MODEL`, then `FERRITE_MODEL_MAIN`) |
| `--base-url URL` | the Ollama or Gemini endpoint (a local Ollama, `http://localhost:11434`, needs no key) |
| `--predictor llm\|rules` | default `llm`; `rules` means the rule layer only |
| `--agent llm\|scripted` | default `llm`; `scripted` means the worst-case script, with no calls |
| `--modes off,guard[,full,dryrun]` | the defense modes to run per case; default `off,guard` |
| `--max-steps N` | agent steps per run; default 8 |
| `--mock-behavior compliant\|resistant\|mixed` | how the mock agent behaves; default `mixed` |
| `--corpus agentdojo[,redteam,core,pilot,agentdojo-hand,all]` | which corpus; default `agentdojo` (the 1,046-case import) |
| `--corpus-root DIR` | the corpus directory (default: `crates/ferrite-eval/tests`); not in `--help` |
| `--suite agentdojo/workspace[,..]` | keep only suites with these prefixes (`agentdojo/banking`, `/slack`, `/travel`, `/workspace`) |
| `--only attack\|benign\|all` | default `all` |
| `--seed S` | shuffle the id-sorted order in a way you can repeat (so a small batch samples every suite) |
| `--batch-size N` | the number of cases per run of the command. With no `--batch-index`, it takes the next N that still have work to do |
| `--batch-index K` | with `--batch-size`: slice K of the whole ordering |
| `--offset N`, `--limit N` | an explicit window |
| `--retry-failed` | redo the cases whose stored result is an error |
| `--plan` (alias `--dry-plan`) | print the calls and tokens the selection would cost; call nothing |
| `--max-calls N` | a hard cap on the requests that reach the provider in this run of the command, retries included; default 100 |
| `--pause-ms MS` | the least gap between calls; default 0 |
| `--max-attempts N` | tries per call, honouring `Retry-After`; default 5 |
| `--backoff-base-ms`, `--backoff-max-ms`, `--timeout-secs` | the first backoff ceiling (2 s), the longest wait (90 s) and the per-request timeout (90 s) |
| `--max-consecutive-failures N` | stop after N provider failures in a row; default 3 |
| `--no-cache` | do not reuse recorded responses |
| `--out DIR` | results, cache and reports; default `target/live-eval` (relative to the working directory) |
| `--report` | add up the stored results into `REPORT.md` and `report.csv` |
| `--compare provider:model[,..]` | limit a report to these runs |
| `--help`, `-h` | the runner's own help |

Some words in this table need a plain meaning:

- A **rate limit** is the provider's cap on how many calls you may send in a
  period. A provider that is over the cap answers with an error and
  `Retry-After`. That header says how long to wait.
- A **backoff** is a growing wait before you try a failed call again.
- A **batch** is a group of cases that one run of the command handles.
- A **cache** keeps the answers to earlier calls, so a repeat call is free.

**Tip: `--batch-size` and `--max-calls` count different things.**

- `--batch-size` counts **cases**.
- `--max-calls` counts **model calls**. Its default is 100.
- One case needs more than one model call. A plan for the AgentDojo import
  estimates about 7,300 model calls for 1,046 cases in two modes. That is a rough
  estimate. It is about 7 calls per case.
- So a batch of 100 cases can need a few hundred calls or more. With the default
  cap of 100 calls, such a batch stops early. Then the exit code is 3. Nothing is
  lost.
- Set `--max-calls` high enough for the batch. Or run the same command again to
  carry on.

**A command like the one the owner used** for the real run is this. The owner's first batch used the default `--max-calls` of 100, which stopped after 18 cases. The value 800 below is a suggestion for finishing a batch of 100 cases. It is not a recorded value:

```
just live-eval --provider ollama --model gemma4:31b --corpus all --seed 1 --batch-size 100 --max-calls 800 --pause-ms 2000
```

In plain words, it says this:

- `--provider ollama --model gemma4:31b`: use Ollama, and use this one model for
  both roles.
- `--corpus all`: use every corpus (1,984 cases).
- `--seed 1`: shuffle the order in a fixed way. A small batch then samples every
  suite.
- `--batch-size 100`: take the next 100 cases that still have work to do.
- `--max-calls 800`: stop after 800 requests reach the provider in this run.
- `--pause-ms 2000`: wait at least 2,000 ms (2 seconds) between calls. This is
  gentle on the provider's rate limit.

Run the same command again and again. Each run takes the next 100 cases. The
runner prints `the selection is complete. Next: --report` when no case has work
left. Before that, it prints how many cases still have work to do. Then run
`just live-eval --report` to write `REPORT.md` and `report.csv`. The key must be in
`OLLAMA_API_KEY` or the keyring for Ollama Cloud. A local Ollama (`--base-url
http://localhost:11434`) needs no key.

**Resume.** Results are an append-only JSONL file. It has one line per case and
mode. Each line is synced to disk before the next case starts. The file is
`<out>/results/<provider>--<small>--<main>.jsonl`. Run the same command again, and it
skips what is stored. It does so only when the settings that change behaviour have
the same hash. Responses are also cached under `<out>/model-cache/`. So a re-run of
an unchanged case is free. A crash or Ctrl-C loses at most the case in flight.
`<out>/budget-partial-results.json` is where the call budget keeps its ledger of
partial results.

**Exit codes.** A loop should branch on them.

| Code | In plain words | What to do |
|---|---|---|
| 0 | The batch finished. | Go on to the next batch. |
| 1 | An I/O or internal failure. | Stop and look. |
| 2 | A usage or configuration problem (flags, a missing key or tag). | Fix it. |
| 3 | `--max-calls` is spent. Nothing is lost. | Raise `--max-calls`, or run the same command again. |
| 4 | The provider kept failing. | Wait, then run the same command again. |
| 5 | The batch finished, but some cases failed. | Run with `--retry-failed` after a pause. |

**First commands to run.** They were checked to parse. The `--plan` and the mock
run were run offline.

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

Not verified here: the real provider run happened on the owner's machine. It was
not repeated in the sandbox where this guide was checked. The `--plan` figures are
estimates that were written before the real run. The real run's own cost table
(calls, cache hits, tokens, run time) is in
`docs/results/live-eval-ollama-gemma4-31b.md`. The OS keyring path is also not
verified. See `docs/TO-DO.md` T-275, T-278.

---

## 7. Engine probes and `page_shot`

These recipes drive a **real headless Servo session**. Headless means there is no
window. It uses software GL (drawing on the CPU). So they need the Servo build. The
first run takes a few minutes. On Linux they run under `xvfb-run` if there is no
display. They are not part of check, test or CI. Each one prints what it checked. It
exits non-zero when a required check fails.

| Recipe | Runs | What it checks |
|---|---|---|
| `just probe-input [url]` | `cargo run -p ferrite-servo --features servo --example input_probe -- [url]` | scrolling, clicking, typing, reload and two tabs, against a built-in page (no network). A page you pass must have `#q` (a text input), `#cb` (a checkbox) and `#btn` (a button). It must also be taller than the viewport |
| `just probe-web-api` | `... --example web_api_probe` | that the Web APIs that benchmarks and frameworks expect do exist. These are `window.crypto` (`getRandomValues`, `randomUUID`, `subtle`), observers, `fetch`, custom elements, WebGL, permissions, notifications and more. They are served from loopback (a secure context). It also checks the painted pixels for the colour of an inline SVG. Required APIs fail the run. `INFO` ones are only listed |
| `just probe-engine` | `cargo run -p ferrite-engine-servo --features engine-servo --example digest_probe` | the real page script. It reads the numbered element table of a loopback form. It hides the password value. It drives `type`, `tick`, `select`, `click` and `scroll` by `@ref` |
| `just probe-profile` | removes `target/profile-probe`, then runs `profile_probe -- set` and `-- get` with `FERRITE_HOME=<repo>/target/profile-probe` | that a cookie and `localStorage` survive a restart. The first process sets them and shuts down cleanly. A fresh process must find them |
| `just probe-controls` | `... --example controls_probe` | that `<select>`, `confirm()`, `prompt()` and a colour input reach the embedder, and that the answers take effect |

### `page_shot`: does this page work in Ferrite?

In short: it loads one page in the real engine and saves a picture, plus a short
report.

```
cargo run -p ferrite-servo --features servo --example page_shot -- \
    <url> [wait_ms] [out.png] [width] [height] [scale]
```

You can also use `just page-shot <args>`. All arguments are optional and
positional:

- the URL (default `https://example.com`);
- how long to run the engine, in milliseconds (default 8000);
- the output PNG (default `page_shot.png`);
- the viewport size in device pixels (1280 x 800);
- the display scale (1.0). A scale of `2` renders as a Retina screen would. Then a
  1280-pixel frame is a 640 CSS-pixel viewport.

It prints these lines:

- `URL`
- `TITLE`
- `VIEWPORT` (inner size and device pixel ratio)
- `LOADED <ms>`, or `never within <ms>`
- a `REQUESTS` count by kind
- up to 300 characters of each `CONSOLE` message
- `FRAME <w>x<h> -> <file>`

It needs the Servo build. For a real URL it also needs network. A sandbox may block
the asset hosts of a page. The page then renders without style. It also exits
cleanly, so the profile is written.

Not verified: these probes were not run again for this guide. The authors of the
commits that added them ran them (`docs/PROGRESS.md`). `probe-controls` has no
recipe comment of its own in the source. It was read, not run.

---

## 8. Diagnostics

### The log file

If you start the app with no terminal (Finder, a Start menu, a launcher), it writes
standard error to a file. If you start it from a terminal, it prints to the terminal
and **no file is written**.

| OS | Folder | File |
|---|---|---|
| macOS | `~/Library/Logs/Ferrite/` (Console.app shows it under Log Reports) | `ferrite.log`, previous run `ferrite.previous.log` |
| Linux | `$XDG_STATE_HOME/ferrite/`, else `~/.local/state/ferrite/` | same names |
| Windows | `%LOCALAPPDATA%\Ferrite\logs` is where the app looks for it, **but the standard-error redirect is Unix-only, so no `ferrite.log` is written on Windows** (`docs/TO-DO.md` T-298) | |

The file starts with a banner:
`[ferrite] ferrite-shell <version> on <os> <arch> (pid N), logging to <path>`.

- Panics always carry a backtrace. `RUST_BACKTRACE=1` is set when you have not set
  it.
- A UI that stops ticking for 8 seconds writes one line. The line is `the UI has
  not responded for Ns (a page or the engine is probably stuck)`. It writes one more
  line when the UI recovers.
- Quitting: within 3 seconds of a close request, the process ends, whatever the
  engine is doing. The line is `[ferrite] shutdown did not finish in 3s; ending the
  process`.
- DevTools' **Save log...** and **Open log folder** use this same folder.

The lines to look for first on a new machine are the renderer line and the WebGL
line. Each is printed once per run:

```
[ferrite-render] CPU rendering (software); there is no GPU renderer
[ferrite-webgl] on (on by default)
```

**Ferrite has no GPU renderer.** Pages are always drawn on the CPU. A GPU renderer
was added (ADR-021) and then removed. On an Apple M1 (macOS 26.6.2, a CI build) its
self-test passed, but with it Google never finished loading and could not be scrolled
or clicked, and the engine's WebGL thread panicked. With the CPU renderer, GitHub
worked fully (`docs/TO-DO.md` T-281 and T-305). The cost is that pages may be slower.
Speed is not measured. `FERRITE_RENDERER` is no longer read. If it is set, the app
prints `FERRITE_RENDERER is ignored: there is only the CPU renderer`.

`FERRITE_WEBGL=on|off|auto` sets whether pages get WebGL. The default is `auto`,
which is on. With `off`, `getContext('webgl')` returns `null`, and a page falls back
to its non-3D version. Use `off` if a page's WebGL ever freezes it. The engine has no
runtime switch for WebGL 1, so `off` forces context creation to fail. Tested on Linux
only: with `on` a canvas draws and reads back the right pixel for 120 frames; with
`off` `getContext` returns `null` and the page keeps running. The vendored engine no
longer panics a page's script thread when the WebGL thread is gone
(`vendor/servo-script/FERRITE-PATCHES.md` item 2). That patch was compiled but never
exercised.

Related engine settings:

- The display scale is read from the window. It is passed to every tab. A Retina
  page is laid out at half the pixel width.
- Style and layout threads follow the core count, clamped to 3-8.
- WebRender and worker pools are clamped to 4-8 (`session.rs`). No speed-up was
  measured.

### `scripts/collect-logs.sh`  (`just collect-logs`)

In short: it packs the logs and system facts into one zip that you can send with a
bug report.

```
bash scripts/collect-logs.sh
bash /Applications/Ferrite.app/Contents/Resources/collect-logs.sh   # from the packaged app
```

It gathers one zip to send:

- `ferrite.log` and `ferrite.previous.log`. If there is none, it adds a
  `NO-LOG.txt` that says so.
- On macOS, the three newest `ferrite*.ips` crash reports.
- A 5-second `sample` of the running process, if Ferrite is open **right now**. Run
  it while the app is frozen.
- `system.txt`. On macOS it has `sw_vers`, `uname`, the CPU and memory, and a
  display listing. Elsewhere it has `uname` and `lscpu`.

It writes `~/Desktop/ferrite-logs-<YYYYMMDD-HHMMSS>.zip` (in the home folder if
there is no Desktop). It prints the path. Nothing is uploaded. The log can contain
page URLs and console errors. It needs `bash` and `zip`. The sampling part needs
macOS. It does not support Windows.

### `just crash-report [report]`  (`python3 scripts/crash_report.py`)

In short: after a crash, it shows where the crash happened.

Use it after a crash (exit 139 on macOS). It prints the exception and the stack of
the crashing thread. It reads the newest `ferrite-shell*.ips` or `.crash` in
`~/Library/Logs/DiagnosticReports/`. Or it reads the report path that you pass. It
uses the Python standard library only. It works for macOS reports only. It exits
non-zero with a message if there is no report.

### Page console errors

Page console messages are in DevTools (section 4). They are also copied to the log as
`[console:<level>] tab N: ...`. Older builds printed `[page console error]`.

### Common situations

| Symptom | Check |
|---|---|
| Pages are blank, or "no real web rendering" | The Servo-free build is running. Run `just setup-servo`, then `just run-local --servo`. (`just doctor` says which build setup recorded.) |
| `run-local` exits 2 "not configured" | Set both model tags in `env.local`. Or pass `--no-model-check` and connect in Settings. |
| Agent panel says no model | Settings (Mod+,) or the environment variables of section 5. |
| A page stops responding | The crash banner and the DevTools Engine tab. Also `ferrite.log`. Run `collect-logs.sh` while it is frozen. |
| A packaged app ignores `env.local` | It never reads it. Use the Settings drawer. |
| Linux: key forgotten after reboot | T-259: export `OLLAMA_API_KEY` or `FERRITE_GEMINI_API_KEY` instead. |
| Cargo link failure with a full disk | Build and test crate by crate (`cargo test -p <crate>`). `just disk` shows where the space is. `rm -rf target/debug/examples target/debug/incremental` reclaims some. |

---

## 9. Quality gates

A quality gate is a check that must pass before you accept a change.

| Recipe | Runs | What it does | Needs |
|---|---|---|---|
| `just check` | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo machete crates` | the structural gate: format, lint (all targets, deny warnings) and unused dependencies. It does not build Servo. It does not run cargo-deny | `cargo-machete` |
| `just fmt` | `cargo fmt --all` | formats the workspace in place | |
| `just lint` | `cargo clippy --workspace --all-targets -- -D warnings` | clippy alone. `--all-targets` matters: its absence hid a real bug for months (T-207) | |
| `just test` | `cargo test --workspace` | the full suite (unit, integration, doctests). It needs **no network and no API key** (R7) | enough disk to link; if short, test per crate |
| `just test-fast` | `cargo test --workspace --lib` | library unit tests only | |
| `just test-live` | `cargo test --workspace -- --ignored` | runs the `#[ignore]`d tests. The workspace has one ignored test: the `ServoEngine` conformance suite. It is built only with `--features engine-servo`. So this recipe as written does not reach it. Run it as `cargo test -p ferrite-engine-servo --features engine-servo -- --ignored --test-threads=1` (it needs the Servo build; `docs/TO-DO.md` T-220) | |
| `just audit` | `cargo deny check` | licenses, RustSec advisories, duplicate versions and sources (`deny.toml`) | network for the advisory DB; `cargo-deny` |
| `just ci` | `just check` then `just test` | the local match of the CI lint and test job. CI also runs `cargo deny` and the two doc checks below | |
| `just test-local` | four script tests: `scripts/tests/test_env_parser.sh`, `test_run_local.sh`, `test_laya_serve.py`, `test_fetch_checkpoint.py` | tests for the local toolchain scripts. They need no network. The Python server tests skip themselves unless `fastapi`, `uvicorn` and `laya` import | bash, python3 |

Docs and drift checks. CI runs both on Linux. Run them before you commit docs.

```
sh scripts/check_purge.sh              # fails if the deleted broker/policy/sandbox/extension architecture reappears outside the allowed history docs
sh scripts/check_no_archive_links.sh   # fails on a markdown link into docs/archive/, or a status/planned heading in CLAUDE.md
```

Both look only at files that git tracks. Run `git add` on a new doc first.

Housekeeping:

| Recipe | What it does |
|---|---|
| `just install-hooks` | sets `core.hooksPath` to `scripts/hooks`. That installs two hooks. A **commit-msg** hook rejects AI attribution (a co-author trailer that names an AI assistant, "generated with", a robot emoji, "ai-assisted"; see `scripts/hooks/commit-msg` for the exact pattern). A **pre-commit** hook scans the staged lines for a real-looking `OLLAMA_API_KEY=` or `FERRITE_GEMINI_API_KEY=` value. Then it runs fmt and clippy on the staged crates only |
| `just disk` | prints the size of the target dir, a breakdown per profile and the 20 largest artifacts (pure `du` and `find`) |
| `just clean-cache` | runs `cargo sweep -t 7` on the target dir. It prunes build artifacts older than 7 days (needs `cargo-sweep`). It is never a blanket `cargo clean` |
| `just bloat [args]` | runs `cargo bloat --release [args]`. It is a report on dependency size (needs `cargo-bloat`) |
| `just build-servo` | runs `cargo build -p ferrite-shell --features ferrite-servo/servo`. It is the Servo build without the setup script. Record its cost in `docs/BUILD_BUDGET.md` |

Rules that review enforces (from `CLAUDE.md`):

- `js.execute` is never scopable.
- Failures fail to an empty fingerprint, never to a bypass.
- No live network calls in tests.
- Dependency direction is strictly downward.
- No dead code.
- One `rusqlite` version, with `bundled`.
- No AI attribution in commits.

---

## 10. Release and CI

CI means continuous integration: automatic checks that run on the code.

### Workflows (`.github/workflows/`)

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` (**CI**) | **manual only** (`workflow_dispatch`; Actions tab, Run workflow). It has no push, pull request or schedule trigger. That is an owner decision (`docs/TO-DO.md` T-210) | Job 1 runs on Linux, Windows and macOS: `cargo fetch`; on Linux `cargo fmt --all --check`; clippy (all targets, deny warnings); on Linux `cargo machete crates` (our crates only; `vendor/` is upstream code), `cargo deny check`, `check_purge.sh` and `check_no_archive_links.sh`; `cargo test --workspace`; `cargo build --release -p ferrite-shell` (Servo-free). Job 2 runs after job 1 passes everywhere, on each OS. It builds `cargo build --release -p ferrite-shell --features ferrite-servo/servo`. It packages the result with `scripts/package.sh`. It uploads it as an artifact that is kept for 90 days |
| `release.yml` (**Release**) | runs by itself when a CI run completes. It acts only if that run was a manual dispatch that **succeeded** | downloads the three packages that CI built. It publishes them again as the rolling prerelease tagged `latest`. The old one is deleted first. The release has the commit, branch, CI run and download notes. It never checks out or runs the CI run's code. A `workflow_run` workflow fires only from the default branch's copy of the file |
| `pages.yml` (**Site**) | manual only | builds `site/` with `python3 site/build.py` (it fails on a broken internal link or anchor). It deploys to GitHub Pages. One-time setup: repository Settings, Pages, Source "GitHub Actions" |

To release, run CI from the Actions tab on `main`. When every job is green, Release
publishes. This was seen through the Actions API on 2026-10-04. The latest `main`
runs (`f4ed744`, 2026-10-02) both succeeded. The `latest` release holds the three
packages. That release was built from `main`. It contains the work up to commit
`8962b01` (the log file and quit watchdog). It does not contain the display-scale
fix, the page controls or DevTools from this branch.

### Packages: `scripts/package.sh <macos|windows|linux> <label> <binary> [out-dir]`

This packages one release binary into `out-dir` (default `dist/`). CI uses the short
commit sha as the label.

| Platform | Result | Contents |
|---|---|---|
| macos | `ferrite-<label>-macos-arm64.zip` | `Ferrite.app` with `Info.plist` (the version comes from `crates/ferrite-shell/Cargo.toml`), the icon, and `scripts/collect-logs.sh` in `Contents/Resources`. It is ad-hoc signed when `codesign` exists |
| windows | `ferrite-<label>-windows-x64.zip` | `ferrite.exe` (the icon is embedded by `crates/ferrite-shell/build.rs`) |
| linux | `ferrite-<label>-linux-x64.tar.gz` | `ferrite`, `ferrite.png`, `ferrite.desktop`, `README.txt`, `install.sh` |

It exits with 1 if the binary is missing. It exits with 2 for an unknown platform.
Notarization is not done (`docs/TO-DO.md` T-261).

Not verified here: any workflow run (only the Actions API summary above). Also not
verified: the package script on macOS or Windows runners, from this machine, and the
packaged apps on real machines.

---

## 11. Environment variable reference

Only the variables below exist in the code or scripts. "App" means the running
browser. "Scripts" means `scripts/*.sh`. "Eval" means the evaluation examples.

| Variable | Used by | Meaning |
|---|---|---|
| `FERRITE_HOME` | app, scripts | the data folder. Scripts default it to `<repo>/.ferrite`. The app, when it is unset, uses `~/.local/share/ferrite` on every OS |
| `CARGO_TARGET_DIR` | justfile, scripts | build output; default `<repo>/target` |
| `FERRITE_PROFILE` | scripts | `dev` (default) or `release` |
| `FERRITE_PYTHON` | `setup-local.sh` | the interpreter for the Laya venv |
| `FERRITE_RENDERER` | app | ignored; there is only the CPU renderer (`docs/TO-DO.md` T-281) |
| `FERRITE_WEBGL` | app | `on`, `off` or `auto` (default); `auto` is on (`docs/TO-DO.md` T-305) |
| `FERRITE_USER_AGENT` | app | overrides the User-Agent string (it wins over Settings) |
| `FERRITE_DEFENSE` | app, eval | `on` (default; any value that is not known also means `on`), `off`, `sanitizer_only`, `loop_only` |
| `FERRITE_MODEL_SMALL`, `FERRITE_MODEL_MAIN` | app, eval, model examples | model tags; no default |
| `FERRITE_OLLAMA_BASE_URL` | app, eval, model examples | the Ollama endpoint; default `https://ollama.com` |
| `FERRITE_GEMINI_BASE_URL` | same | the Gemini endpoint |
| `OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY` | same | provider keys (or the keyring) |
| `FERRITE_MODEL_CALL_BUDGET` / `_MAX_IN_FLIGHT` / `_TIMEOUT_SECS` / `_MAX_RESPONSE_BYTES` / `_CACHE_DIR` | same | defaults 500 / 2 / 60 / 1048576 / `~/.cache/ferrite-model` |
| `FERRITE_TWIN_KEY` | app, eval | the encryption key of the dry-run twin (or the keyring) |
| `FERRITE_LAYA_URL` and the other `FERRITE_LAYA_*` | app, scripts | see section 5. The script-only ones are `FERRITE_LAYA_CHECKPOINT_DIR`, `_HOST`, `_PORT`, `_DEVICE`, `_THREADS`, `_WARMUP`, `_HEAD_MAX_LEN`, `_LOG_LEVEL`, `_OFFLINE`, `_MPS_AMP_MIN_ROWS`, `_PIP_SPEC` |
| `FERRITE_EVAL_OUT_DIR` | `just eval` | the report folder; default `target/eval-report` |
| `FERRITE_LIVE_PROVIDER`, `FERRITE_LIVE_MODEL`, `FERRITE_LIVE_SMALL_MODEL`, `FERRITE_LIVE_MAIN_MODEL` | `live_eval` | defaults for `--provider`, `--model`, `--small-model`, `--main-model` |
| `FERRITE_PAGE_SCRIPT_DUMP` | engine | a directory that the assembled page scripts are written to, for debugging |
| `RUST_LOG`, `RUST_BACKTRACE` | scripts, app | logging; the app sets `RUST_BACKTRACE=1` if it is unset |
| `HF_TOKEN` | `setup-local.sh` | the Hugging Face token for the Laya checkpoint, if needed |
| `NO_COLOR` | scripts | turns colour off |
| `XDG_STATE_HOME`, `LOCALAPPDATA`, `HOME` | app, scripts | where the log folder is (section 8) |

`env.local` accepts only `FERRITE_*`, `OLLAMA_*`, `GEMINI_*` and `RUST_LOG`. The
scripts read it. The app itself never reads it. It never holds keys.

---

## 12. Where files live

Let `<data>` be `$FERRITE_HOME` if it is set, else `~/.local/share/ferrite`.

| Path | What |
|---|---|
| `<data>/profile/` | cookies, HSTS, saved HTTP credentials, web storage (written on a clean close) |
| `<data>/settings.json` | the provider and model choices of the Settings drawer, and the browser identity (never a key) |
| `<data>/bookmarks.json` | bookmarks |
| `<data>/chats/` | one `<id>.json` per agent chat |
| `<data>/ui-layout.json` | saved panel sizes |
| `<data>/audit/network.db` | the hash-chained network audit log (the Security log of the Audit panel); the OS temp dir if no data folder resolves |
| `<data>/logs/model-activity.jsonl` | the model-call trace (it contains prompts and page text) |
| `<data>/cache/favicons/` | the favicon cache when `FERRITE_HOME` is set, else `~/.cache/ferrite-ui/favicons` |
| `<data>/env.local`, `build.mode`, `laya/` | made by the scripts under `$FERRITE_HOME` only |
| OS keyring, service `ferrite` | API keys (`OLLAMA_API_KEY`, `FERRITE_GEMINI_API_KEY`) and `FERRITE_TWIN_KEY` |
| `~/.cache/ferrite-model/` | the model response cache (`FERRITE_MODEL_CACHE_DIR`) |
| the downloads folder | files saved by the Downloads of the Library (falls back to home) |
| the log folder | section 8 |
| `target/eval-report/`, `target/live-eval/` | the output of `just eval` and `guard-eval`, and of the live runner |
| `target/profile-probe/` | the throwaway profile of `probe-profile` |

Browsing history is for the session only. It is not saved.
