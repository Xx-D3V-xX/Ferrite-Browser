# Ferrite — the ONLY documented command surface (docs/REBUILD_DIRECTIVE.md §7/T-101/T-013).
# docs/COMMANDS.md explains every recipe in detail (what it needs, what it prints).
# The build, lint, test, eval and probe recipes are plain `cargo`/`python3`
# commands. The local toolchain recipes (setup*, run-local/-fast/-all, models-local,
# laya-*, doctor, collect-logs, crash-report, test-local, disk, probe-profile) call
# bash scripts or POSIX tools: macOS and Linux; not run on Windows.
# Run `just` alone (or `just --list`) to see this list.

# Shared, portable target dir (see .cargo/config.toml's comment for why this
# lives here and not in a hardcoded `build.target-dir`). Override per-
# contributor with a real CARGO_TARGET_DIR env var; this is only the default.
export CARGO_TARGET_DIR := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")

# List all recipes (default).
default:
    @just --list

# Full local CI-equivalent gate: format, lint, unused-deps, license/ban audit, tests.
# This is what `just check && just test` (the A1 exit-gate phrase) expands to
# when you want both halves in one shot.
# Format, lint, unused-dependency and test gates in one command (check, then test).
ci: check test

# Structural/style gate: fmt-check + clippy (deny warnings) + unused deps.
# Does NOT build Servo (default feature set) and does NOT run cargo-deny
# (that's `just audit` — separated because it hits the network for the
# advisory DB and is slower).
# Structural gate: format check, clippy (deny warnings, all targets) and unused dependencies.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo machete crates

# Format the whole workspace in place.
fmt:
    cargo fmt --all

# Clippy alone, deny warnings, all targets (lib + bins + tests + examples —
# the old CI's `cargo clippy --workspace` without --all-targets never linted
# test code at all; see docs/TO-DO.md T-207 for what that hid).
# Clippy on every target, deny warnings.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# License/advisory/duplicate-version/source audit. Networked (fetches the
# RustSec advisory DB) — kept separate from `check` for that reason.
# License, advisory, duplicate-version and source audit with cargo-deny (needs the network).
audit:
    cargo deny check

# Full workspace test suite (unit + integration + doctests). Must pass with
# no network and no provider API key set (R7) — nothing in this workspace's
# tests currently needs either, and CI should stay that way.
# The full workspace test suite; needs no network and no API key.
test:
    cargo test --workspace

# Unit tests only (crate libs, no integration tests/doctests) — quick
# iteration loop.
# Library unit tests only, for a quick loop.
test-fast:
    cargo test --workspace --lib

# Runs #[ignore]'d tests, so `just test` never spends quota or needs Servo (R7).
# The one #[ignore]d test in the workspace (the ServoEngine conformance suite) is
# compiled only with `--features engine-servo`, so this recipe as written does not
# reach it; run it with: cargo test -p ferrite-engine-servo --features engine-servo
# -- --ignored --test-threads=1 (needs the real Servo build).
# Run #[ignore]d tests (the Servo conformance test needs its own command, above).
test-live:
    cargo test --workspace -- --ignored

# Lists every tag the configured Ollama endpoint actually serves
# (docs/REBUILD_DIRECTIVE.md §10.2's startup preflight target). Networked;
# needs OLLAMA_API_KEY (or the OS keyring) unless FERRITE_OLLAMA_BASE_URL
# points at a local endpoint, and FERRITE_MODEL_SMALL/FERRITE_MODEL_MAIN
# set (T-213: no model name is ever a default). Not part of check/test/CI.
# List every tag the configured Ollama endpoint serves (needs a key, or a local Ollama).
models:
    cargo run -p ferrite-model --example models

# One live Ollama Cloud round-trip, by hand — A3's exit gate. Same
# credentials/config as `models`. Not part of check/test/CI; this is the
# one target the directive explicitly asks a human to run once, not a
# thing `just test` should ever do (R7).
# One live Ollama round trip, by hand (needs a key).
probe:
    cargo run -p ferrite-model --example probe

# Reports the on-disk response cache's size and the hit rate flushed by the
# last run (§10.3: "a low hit rate is a bug, investigate it"). Offline —
# only reads ~/.cache/ferrite-model/ (or $FERRITE_MODEL_CACHE_DIR), no
# network, no model config required. Safe to run anywhere, including CI.
# Report the model response cache's size and last hit rate (offline).
cache-stats:
    cargo run -p ferrite-model --example cache_stats

# Records one live response into the committed fixture directory
# (crates/ferrite-model/tests/fixtures/model/) for ReplayProvider to serve
# in offline tests — §10.3: "these are committed." Usage:
#   just record ollama "your prompt"
#   just record gemini "your prompt"
# Networked; same config/credentials as `models`/`probe`. Not part of
# check/test/CI — a fixture this writes is reviewed and committed by hand,
# like any other change to the test tree.
# Record one live response as a test fixture: just record ollama|gemini "prompt".
record provider prompt="":
    cargo run -p ferrite-model --example record -- {{provider}} "{{prompt}}"

# Subcommands (ferrite-shell/src/main.rs): ui (default), window, jstest,
# agent-smoke, smoke. This recipe never enables Servo, so pages do not render;
# for real pages use run-local, run-fast or run-all.
# Run the shell binary without the Servo engine (the UI, the agent loop, the defense).
run *ARGS:
    cargo run -p ferrite-shell -- {{ARGS}}

# Build with the real Servo engine — feature-gated, slow, NOT part of
# `just check`/`just test`/CI's default path (docs/REBUILD_DIRECTIVE.md
# §7.1: Servo is optional and off by default; in CI it is built by the manual run's second job). Cost
# should be recorded in docs/BUILD_BUDGET.md whenever this is run.
# Build ferrite-shell with the real Servo engine (slow: 20-60 minutes the first time).
build-servo:
    cargo build -p ferrite-shell --features ferrite-servo/servo

# Audio, video and WebRTC need GStreamer (its development files to build, its runtime and
# plugins to run); docs/COMMANDS.md lists the packages. Not part of any release build.
# Run the real-Servo browser with audio, video and WebRTC (needs GStreamer).
run-media *ARGS:
    cargo run -p ferrite-shell --features ferrite-servo/servo,ferrite-servo/media -- ui {{ARGS}}

# Checks the whole of "a page asks for the camera, microphone or screen": the prompt, the
# answers, remembered decisions (and that the agent's presence stops them being used),
# `stop()`, the browser's "stop sharing", and that a page cannot hide a live capture.
# Uses the engine's test sources for the camera and microphone. Needs GStreamer.
# Check camera, microphone and screen sharing in the real engine (needs GStreamer).
probe-capture:
    cargo run -p ferrite-servo --features servo,media --example capture_probe

# Registers a service worker from loopback and checks its whole life: install, activate,
# `ready`, `controller`, `postMessage`, `clients`, `fetch` events (answered, passed through,
# a POST body, the Cache API inside the worker), a remembered registration on the next page,
# `unregister` and the refusals. Needs the real Servo build.
# Check service workers in the real engine.
probe-sw:
    cargo run -p ferrite-servo --features servo --example sw_probe

# Plays a WebM video and an Ogg file, draws a decoded frame, opens a WebRTC data channel
# between two peers and checks that camera, microphone and screen capture are refused.
# Needs the real Servo build with GStreamer.
# Check audio, video and WebRTC in the real engine (needs GStreamer).
probe-media:
    cargo run -p ferrite-servo --features servo,media --example media_probe

# Media Source Extensions in a real headless Servo session: a page builds a stream out of
# SourceBuffers and it plays, seeks, stalls and ends (56 checks). Needs GStreamer.
probe-mse:
    cargo run -p ferrite-servo --features servo,media --example mse_probe

# Drives a real headless Servo session against a built-in page and checks
# scrolling, clicking, typing and reload (no network or window needed).
# Pass a URL to probe another page. Needs the real Servo build.
# Check scrolling, clicking, typing and reload in a real headless Servo session.
probe-input *ARGS:
    cargo run -p ferrite-servo --features servo --example input_probe -- {{ARGS}}

# Checks that the Web APIs benchmark and framework bundles assume exist in the
# real engine: window.crypto (getRandomValues, randomUUID, subtle), observers,
# fetch, custom elements and more, served from loopback. Needs the real Servo build.
# Check that the Web APIs benchmarks and frameworks assume exist in the real engine.
probe-web-api:
    cargo run -p ferrite-servo --features servo --example web_api_probe

# Checks page storage in the real engine: IndexedDB indexes and cursors, the Cache
# API (`caches`), localStorage and sessionStorage, served from loopback. Needs the
# real Servo build.
# Check IndexedDB, the Cache API and web storage in the real engine.
probe-storage:
    cargo run -p ferrite-servo --features servo --example storage_probe

# Runs the real page script in a headless Servo session and drives a form by
# `@ref`: digest, type, tick, select, click, scroll. Needs the real Servo build.
# Check that the page script reads a form and drives it by @ref in the real engine.
probe-engine:
    cargo run -p ferrite-engine-servo --features engine-servo --example digest_probe

# A <select>, confirm(), prompt() and a colour input, served from loopback. Needs the real Servo build.
# Check that the engine's page controls reach the embedder and answers take effect.
probe-controls:
    cargo run -p ferrite-servo --features servo --example controls_probe

# Prints the title, load time, console messages and request summary, and writes a PNG.
# Args: <url> [wait_ms] [out.png] [width] [height] [scale]
# (defaults: https://example.com 8000 page_shot.png 1280 800 1; scale 2 renders as Retina).
# Does this page work in Ferrite, without the app? Needs the real Servo build.
page-shot *ARGS:
    cargo run -p ferrite-servo --features servo --example page_shot -- {{ARGS}}

# Checks that a cookie and localStorage survive a restart: one run sets them,
# a fresh process reads them back, in a throwaway profile directory.
# Check that a cookie and localStorage survive a restart.
probe-profile:
    rm -rf target/profile-probe
    FERRITE_HOME={{justfile_directory()}}/target/profile-probe cargo run -p ferrite-servo --features servo --example profile_probe -- set
    FERRITE_HOME={{justfile_directory()}}/target/profile-probe cargo run -p ferrite-servo --features servo --example profile_probe -- get

# Dependency-bloat report. Requires `cargo install cargo-bloat` (not
# bundled — it's a diagnostic tool you reach for before adding a
# dependency, per §7.3, not a gate every run needs).
# Report what takes space in a release build (needs cargo-bloat).
bloat *ARGS:
    cargo bloat --release {{ARGS}}

# Runs the real corpus (tests/corpus + tests/pilot_corpus +
# tests/agentdojo_corpus + tests/corpus_redteam, 938 cases) through the real pipeline across every defined
# mode and writes the metrics report (docs/REBUILD_DIRECTIVE.md §13.2:
# markdown table + CSV + audit-chain anchors) to target/eval-report/ (or
# FERRITE_EVAL_OUT_DIR if set). The agent in it is ground-truth-scripted, never
# model-backed. With no FERRITE_MODEL_SMALL/FERRITE_MODEL_MAIN set the fingerprint
# layer runs rules-only and the whole run makes zero network calls; with them set
# and a key available it makes live fingerprint calls (unset them to reproduce the
# reported numbers). NOT part of `just check`/`just test`/CI (T-112, A12). See
# docs/EVALUATION.md.
# Run the 938-case corpus through every defense mode and write the metrics report (offline by default).
eval:
    cargo run -p ferrite-eval --example eval

# The runtime-guard experiment (ADR-014, docs/EVALUATION.md §8.4): what the real
# run does when the dry run could not have seen the attack. Prints a table and
# writes target/eval-report/GUARD_REPORT.md. No network, no API key.
# Run the runtime-guard experiment: prints a table, writes GUARD_REPORT.md (offline).
guard-eval:
    cargo run -p ferrite-eval --example guard_eval

# Example: just inspect-case crates/ferrite-eval/tests/corpus/c11_offscreen_scope_escalation.json
# Prints each layer's verdict and whether it matches the case's ground truth (T-202). Offline.
# Run ONE authored case file through every mode its run label defines.
inspect-case path:
    cargo run -p ferrite-eval --example inspect_case -- "{{path}}"

# Regenerate the 909-case red-team corpus (deterministic; ids are uuid5 of the
# case name). `redteam-corpus-check` fails if the files on disk differ.
# Regenerate the 909 red-team cases deterministically.
redteam-corpus:
    python3 scripts/gen_redteam_corpus.py

# Fail if the red-team corpus on disk differs from a fresh generation.
redteam-corpus-check:
    python3 scripts/gen_redteam_corpus.py --check

# Needs a checkout of github.com/ethz-spylab/agentdojo at commit 089ed468cf3ed0322acc66b0211f26d9d90dbf60
# (the importer prints the clone and checkout commands if it is missing or elsewhere).
# Check that the committed AgentDojo import (1,046 cases) is byte-identical to a fresh import from <src>.
agentdojo-check src:
    python3 scripts/import_agentdojo.py --src "{{src}}" --check

# A real model in the evaluation loop, in resumable batches (docs/EVALUATION.md section 9,
# docs/COMMANDS.md section 6 lists every flag). --provider gemini|ollama|mock and a model
# tag are required; keys come from the environment or the OS keyring only. Results go to
# target/live-eval. Run live-eval-plan first. One full run exists, with ollama gemma4:31b (docs/results/).
# Example: just live-eval --provider mock --model x --batch-size 12
# Run the live evaluation: a real model in the loop, in resumable batches (run live-eval-plan first).
live-eval *ARGS:
    cargo run --release -p ferrite-eval --example live_eval -- {{ARGS}}

# Prices a selection (calls, tokens, invocations at the --max-calls cap) and calls nothing; no key needed.
# Example: just live-eval-plan --provider gemini --model <tag>
# Preview what a live evaluation would cost, calling nothing.
live-eval-plan *ARGS:
    cargo run --release -p ferrite-eval --example live_eval -- --plan {{ARGS}}

# Prune stale (7+ day old) build artifacts from the target dir. Requires
# `cargo install cargo-sweep` (not bundled, same reasoning as `bloat`).
# Deliberately NEVER a blanket `cargo clean` — REBUILD_DIRECTIVE.md §7.4.
# Prune build artifacts older than 7 days (needs cargo-sweep).
clean-cache:
    cargo sweep -t 7 "$CARGO_TARGET_DIR"

# Report target-dir size, per-profile breakdown, and the 20 largest
# artifacts. No extra tool required — pure du/find/sort. See
# docs/BUILD_BUDGET.md for the target (<12GB, <5min cold `just test`
# without Servo) and the recorded numbers at each phase gate.
# Report the target dir's size, per-profile sizes and the 20 largest artifacts.
disk:
    @echo "=== target dir: $CARGO_TARGET_DIR ==="
    @du -sh "$CARGO_TARGET_DIR" 2>/dev/null || echo "(not built yet)"
    @echo ""
    @echo "=== per-profile ==="
    @du -sh "$CARGO_TARGET_DIR"/*/ 2>/dev/null || true
    @echo ""
    @echo "=== 20 largest artifacts ==="
    @find "$CARGO_TARGET_DIR" -type f -exec du -h {} + 2>/dev/null | sort -rh | head -20 || true

# Install the git hooks for this repo (commit-msg guard + pre-commit
# fmt/clippy gate). Idempotent — safe to re-run.
# Install the git hooks (commit-msg guard and pre-commit gate).
install-hooks:
    ./scripts/hooks/install.sh

# ── Local setup + run toolchain ──────────────────────────────────────────
# Scripts under scripts/ (bash 3.2-compatible, macOS-first). State lives
# in $FERRITE_HOME (default <repo>/.ferrite, gitignored); every
# recipe is safe to re-run. `just setup --dry-run --yes` prints the plan.
# (`just --list` shows only the last comment line of each recipe, so that
# line is the one-sentence summary.)

# Flags pass through: --no-laya, --yes, --dry-run, --laya-checkpoint v10, ...
# Local setup, every time: toolchain checks, Servo-free build, optional Laya.
setup *ARGS:
    ./scripts/setup-local.sh {{ARGS}}

# First build: 20-60 min and 10+ GB.
# Local setup with the REAL Servo engine (real web rendering).
setup-servo *ARGS:
    ./scripts/setup-local.sh --with-servo {{ARGS}}

# The whole project in one go: the real Servo build (20-60 min, 10+ GB the
# first time) plus the Laya venv and browser checkpoint, all inside this folder.
# Everything: real Servo + local Laya, set up in one command.
setup-all *ARGS:
    ./scripts/setup-local.sh --with-servo --laya {{ARGS}}

# Flags: --no-laya, --servo/--no-servo, --wait N, --dry-run; arguments after
# `--` go to ferrite-shell instead of `ui`.
# Run the browser with env.local, starting the local Laya server if set up.
run-local *ARGS:
    ./scripts/run-local.sh {{ARGS}}

# The same, built with the release profile. The dev profile is for working on
# Ferrite; this one is what to use to *use* it, or to judge its speed: the
# engine runs several times faster optimised (T-269).
# Run the real-Servo browser built with the release profile (what to use to use it).
run-fast *ARGS:
    FERRITE_PROFILE=release ./scripts/run-local.sh --servo {{ARGS}}

# Starts the Laya server (if set up), then the real-Servo browser with it.
# Run everything: real Servo browser + local Laya server.
run-all *ARGS:
    ./scripts/run-local.sh --servo {{ARGS}}

# `just models` needs both tags set before it can list anything; this loads
# env.local and supplies placeholders. Needs OLLAMA_API_KEY or a local Ollama.
# List the model tags your Ollama endpoint serves.
models-local:
    ./scripts/run-local.sh --list-models

# Run ONLY the Laya server, in the foreground (Ctrl-C stops it).
laya-serve:
    ./scripts/run-local.sh --laya-only

# Stdlib-only; exits non-zero with a suggested fix if the server is unreachable.
# Send one recorded browser step to Laya; prints its decision and latency.
laya-verify:
    ./scripts/run-local.sh --verify

# Needs python3 only. Prints the exception and the crashing thread's stack.
# After a crash (exit 139): where did the newest ferrite-shell crash happen?
crash-report *ARGS:
    python3 scripts/crash_report.py {{ARGS}}

# Writes ~/Desktop/ferrite-logs-<time>.zip (home folder if there is no Desktop): ferrite.log
# and the previous run's, macOS crash reports, system info, and, on macOS if Ferrite is running
# right now, a 5 second stack sample (run it while the app is frozen). Needs bash and zip. Nothing is uploaded.
# Gather the log, crash reports and system info into one zip to send with a bug report.
collect-logs:
    bash scripts/collect-logs.sh

# Exit 1 only if something FAILs. `just doctor --fix-hints` says how to fix.
# Checklist of what is ready: tools, build, env, keys, Laya, disk.
doctor *ARGS:
    ./scripts/doctor.sh {{ARGS}}

# No network, no cargo build; the Python server tests skip themselves unless
# fastapi, uvicorn and laya are importable.
# Tests for the local toolchain scripts themselves.
test-local:
    bash scripts/tests/test_env_parser.sh
    bash scripts/tests/test_run_local.sh
    python3 scripts/tests/test_laya_serve.py
    python3 scripts/tests/test_fetch_checkpoint.py
