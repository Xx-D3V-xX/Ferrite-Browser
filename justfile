# Ferrite — the ONLY documented command surface (docs/REBUILD_DIRECTIVE.md §7/T-101/T-013).
# OS-neutral by construction: every recipe below is a `cargo`/`just` command,
# no PowerShell-only or bash-only syntax, no `C:\...` paths. Run `just` alone
# (or `just --list`) to see this list from any shell on any platform.

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
ci: check test

# Structural/style gate: fmt-check + clippy (deny warnings) + unused deps.
# Does NOT build Servo (default feature set) and does NOT run cargo-deny
# (that's `just audit` — separated because it hits the network for the
# advisory DB and is slower).
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo machete

# Format the whole workspace in place.
fmt:
    cargo fmt --all

# Clippy alone, deny warnings, all targets (lib + bins + tests + examples —
# the old CI's `cargo clippy --workspace` without --all-targets never linted
# test code at all; see docs/TO-DO.md T-207 for what that hid).
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# License/advisory/duplicate-version/source audit. Networked (fetches the
# RustSec advisory DB) — kept separate from `check` for that reason.
audit:
    cargo deny check

# Full workspace test suite (unit + integration + doctests). Must pass with
# no network and no provider API key set (R7) — nothing in this workspace's
# tests currently needs either, and CI should stay that way.
test:
    cargo test --workspace

# Unit tests only (crate libs, no integration tests/doctests) — quick
# iteration loop.
test-fast:
    cargo test --workspace --lib

# Live-provider tests, #[ignore]'d by convention so `just test` never spends
# quota (R7). Nothing is marked #[ignore] yet in this workspace — this
# recipe is here for when A3's ModelProvider conformance suite adds them,
# not decorative.
test-live:
    cargo test --workspace -- --ignored

# Lists every tag the configured Ollama endpoint actually serves
# (docs/REBUILD_DIRECTIVE.md §10.2's startup preflight target). Networked;
# needs OLLAMA_API_KEY (or the OS keyring) unless FERRITE_OLLAMA_BASE_URL
# points at a local endpoint, and FERRITE_MODEL_SMALL/FERRITE_MODEL_MAIN
# set (T-213: no model name is ever a default). Not part of check/test/CI.
models:
    cargo run -p ferrite-model --example models

# One live Ollama Cloud round-trip, by hand — A3's exit gate. Same
# credentials/config as `models`. Not part of check/test/CI; this is the
# one target the directive explicitly asks a human to run once, not a
# thing `just test` should ever do (R7).
probe:
    cargo run -p ferrite-model --example probe

# Reports the on-disk response cache's size and the hit rate flushed by the
# last run (§10.3: "a low hit rate is a bug, investigate it"). Offline —
# only reads ~/.cache/ferrite-model/ (or $FERRITE_MODEL_CACHE_DIR), no
# network, no model config required. Safe to run anywhere, including CI.
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
record provider prompt="":
    cargo run -p ferrite-model --example record -- {{provider}} "{{prompt}}"

# Run the shell binary (launches the Iced UI by default — see
# ferrite-shell/src/main.rs's CLI dispatch for the other subcommands:
# window, jstest, agent-smoke, smoke).
run *ARGS:
    cargo run -p ferrite-shell -- {{ARGS}}

# Build with the real Servo engine — feature-gated, slow, NOT part of
# `just check`/`just test`/CI's default path (docs/REBUILD_DIRECTIVE.md
# §7.1: Servo is optional and off by default; weekly-only in CI). Cost
# should be recorded in docs/BUILD_BUDGET.md whenever this is run.
build-servo:
    cargo build -p ferrite-shell --features ferrite-servo/servo

# Drives a real headless Servo session against a built-in page and checks
# scrolling, clicking, typing and reload (no network or window needed).
# Pass a URL to probe another page. Needs the real Servo build.
probe-input *ARGS:
    cargo run -p ferrite-servo --features servo --example input_probe -- {{ARGS}}

# Checks that the Web APIs benchmark and framework bundles assume exist in the
# real engine: window.crypto (getRandomValues, randomUUID, subtle), observers,
# fetch, custom elements and more, served from loopback. Needs the real Servo build.
probe-web-api:
    cargo run -p ferrite-servo --features servo --example web_api_probe

# Runs the real page script in a headless Servo session and drives a form by
# `@ref`: digest, type, tick, select, click, scroll. Needs the real Servo build.
probe-engine:
    cargo run -p ferrite-engine-servo --features engine-servo --example digest_probe

# Checks that a cookie and localStorage survive a restart: one run sets them,
# a fresh process reads them back, in a throwaway profile directory.
probe-profile:
    rm -rf target/profile-probe
    FERRITE_HOME={{justfile_directory()}}/target/profile-probe cargo run -p ferrite-servo --features servo --example profile_probe -- set
    FERRITE_HOME={{justfile_directory()}}/target/profile-probe cargo run -p ferrite-servo --features servo --example profile_probe -- get

# Dependency-bloat report. Requires `cargo install cargo-bloat` (not
# bundled — it's a diagnostic tool you reach for before adding a
# dependency, per §7.3, not a gate every run needs).
bloat *ARGS:
    cargo bloat --release {{ARGS}}

# Runs the real corpus (tests/corpus + tests/pilot_corpus +
# tests/agentdojo_corpus) through the real pipeline across every defined
# mode and writes the metrics report (docs/REBUILD_DIRECTIVE.md §13.2:
# markdown table + CSV + audit-chain anchors) to target/eval-report/ (or
# FERRITE_EVAL_OUT_DIR if set). Makes zero live model calls by construction
# (the corpus-runner agent is ground-truth-scripted, never model-backed;
# the fingerprint layer runs rules-only without FERRITE_GEMINI_API_KEY set)
# — safe to run anywhere, though NOT part of `just check`/`just test`/CI
# (T-112, A12: Harness + metrics). See docs/EVALUATION.md.
eval:
    cargo run -p ferrite-eval --example eval

# The runtime-guard experiment (ADR-014, docs/EVALUATION.md §8.4): what the real
# run does when the dry run could not have seen the attack. Prints a table and
# writes target/eval-report/GUARD_REPORT.md. No network, no API key.
guard-eval:
    cargo run -p ferrite-eval --example guard_eval

# Regenerate the 909-case red-team corpus (deterministic; ids are uuid5 of the
# case name). `redteam-corpus-check` fails if the files on disk differ.
redteam-corpus:
    python3 scripts/gen_redteam_corpus.py

redteam-corpus-check:
    python3 scripts/gen_redteam_corpus.py --check

# Prune stale (7+ day old) build artifacts from the target dir. Requires
# `cargo install cargo-sweep` (not bundled, same reasoning as `bloat`).
# Deliberately NEVER a blanket `cargo clean` — REBUILD_DIRECTIVE.md §7.4.
clean-cache:
    cargo sweep -t 7 "$CARGO_TARGET_DIR"

# Report target-dir size, per-profile breakdown, and the 20 largest
# artifacts. No extra tool required — pure du/find/sort. See
# docs/BUILD_BUDGET.md for the target (<12GB, <5min cold `just test`
# without Servo) and the recorded numbers at each phase gate.
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
