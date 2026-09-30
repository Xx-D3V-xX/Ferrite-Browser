#!/usr/bin/env bash
# scripts/setup-local.sh — set Ferrite up on THIS machine, repeatably.
# macOS is the primary target; Linux is best-effort. Safe to re-run every time:
# every step checks first and skips what is already done, and nothing is
# deleted or overwritten (an existing env.local, venv or checkpoint is kept).
#
#   scripts/setup-local.sh --help
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/common.sh
. "$HERE/lib/common.sh"

usage() {
  cat <<EOF
Usage: scripts/setup-local.sh [options]

Checks your toolchain, builds the browser, and (optionally) sets up a local
Laya decision server. Prints what it does; changes nothing outside the places
listed below.

Options:
  --with-servo            build with the REAL Servo engine (slow, big; see below)
  --no-servo              build without Servo (the default; fast, NO web rendering)
  --no-build              skip the cargo build entirely
  --laya / --no-laya      set up the local Laya server (default: on) / skip it
  --laya-checkpoint NAME  v10s (default; 322M, faster) or v10 (421M)
  --yes, -y               non-interactive: answer yes to every install prompt
  --dry-run               print every action that WOULD be taken, run none
  -h, --help              this text

Build choices:
  default        cargo build -p ferrite-shell
                 ~2 min cold (docs/BUILD_BUDGET.md). The Servo-free build has NO
                 real web rendering: use it to try the UI, the agent loop and
                 the defense, not to browse real sites.
  --with-servo   cargo build -p ferrite-shell --features ferrite-servo/servo
                 First build: plan on 20-60 minutes and 10+ GB of disk in the
                 target dir (docs/BUILD_BUDGET.md measured 15m31s and 6.4 GB for
                 a DEBUG Servo build added to an existing build). No extra brew
                 package is needed (audio/video playback is not built in).
                 FERRITE_PROFILE=release builds the release
                 profile instead (slower to build).

What it may write, and where:
  \$FERRITE_HOME     (default <repo>/.ferrite, gitignored): the Laya venv, the
                    checkpoint, env.local, pid/log files, build.mode
  \$CARGO_TARGET_DIR (default <repo>/target): build output
  this repo's own files (none are modified by this script)
  Only after an explicit prompt or --yes: rustup (~/.cargo, ~/.rustup, with
  --no-modify-path so your shell profile is untouched), brew packages (never
  sudo). Package-manager caches (pip, Hugging Face) are written by those tools.

Environment: FERRITE_HOME, CARGO_TARGET_DIR, FERRITE_PYTHON (interpreter for
the venv), FERRITE_LAYA_PIP_SPEC (default: $FERRITE_LAYA_PIP_SPEC_DEFAULT),
HF_TOKEN (if a Hugging Face download needs authentication).
EOF
}

DRY_RUN=0
ASSUME_YES=0
WITH_SERVO=ask
NO_BUILD=0
DO_LAYA=1
LAYA_CKPT=$FERRITE_LAYA_DEFAULT_CHECKPOINT
FAILED=''      # newline-separated list of steps that did not complete
BREW_MISSING=''

while [ $# -gt 0 ]; do
  case $1 in
    --with-servo) WITH_SERVO=1 ;;
    --no-servo) WITH_SERVO=0 ;;
    --no-build) NO_BUILD=1 ;;
    --laya) DO_LAYA=1 ;;
    --no-laya) DO_LAYA=0 ;;
    --laya-checkpoint)
      [ $# -ge 2 ] || die "--laya-checkpoint needs a value (v10s or v10)"
      LAYA_CKPT=$2; shift ;;
    --laya-checkpoint=*) LAYA_CKPT=${1#*=} ;;
    --yes|-y) ASSUME_YES=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown option: $1" ;;
  esac
  shift
done

case $LAYA_CKPT in
  ''|*[!A-Za-z0-9._-]*|.|..) die "--laya-checkpoint must be a plain name like v10s or v10" ;;
esac

# ── helpers ───────────────────────────────────────────────────────────────
fail_step() { FAILED="$FAILED${FAILED:+$'\n'}$1"; bad "$1"; }

# confirm PROMPT — yes if --yes; in --dry-run assume yes (so the plan shows the
# action); otherwise ask on the terminal; non-interactive without --yes = no.
confirm() {
  if [ "$ASSUME_YES" = 1 ] || [ "$DRY_RUN" = 1 ]; then return 0; fi
  if [ ! -t 0 ]; then
    info "(non-interactive: skipping \"$1\"; re-run with --yes to allow it)"
    return 1
  fi
  local reply
  printf '  %s [y/N] ' "$1"
  read -r reply || return 1
  case $reply in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# act DESCRIPTION CMD... — run CMD, or in --dry-run print what would run.
act() {
  local desc=$1; shift
  if [ "$DRY_RUN" = 1 ]; then
    printf '  %swould:%s %s\n' "$C_YEL" "$C_OFF" "$desc"
    note "\$ $(quote_cmd "$@")"
    return 0
  fi
  note "\$ $(quote_cmd "$@")"
  "$@"
}

OS=$(uname -s)
ARCH=$(uname -m)

# ── steps ─────────────────────────────────────────────────────────────────
step_preflight() {
  step "Ferrite local setup"
  [ "$DRY_RUN" = 1 ] && warn "DRY RUN: nothing below is executed; each action is only printed."
  info "repo:          $FERRITE_REPO_ROOT"
  info "FERRITE_HOME:  $FERRITE_HOME"
  info "target dir:    $CARGO_TARGET_DIR"
  info "platform:      $OS $ARCH"
  case $OS in
    Darwin) ;;
    Linux) warn "Linux is best-effort: system packages are not installed for you; see hints below." ;;
    *) warn "$OS is not a supported platform for this script." ;;
  esac
  local free need=3
  [ "$WITH_SERVO" = 1 ] && need=15
  free=$(free_gb "$CARGO_TARGET_DIR" || true)
  if [ -n "$free" ]; then
    if [ "$free" -lt "$need" ]; then
      warn "only ${free} GB free where the build goes (rule of thumb: keep >= ${need} GB free for this build)"
    else
      ok "${free} GB free for the build"
    fi
  fi
}

step_git() {
  step "git"
  if have git; then
    ok "git $(git --version | awk '{print $3}')"
  else
    fail_step "git not found. macOS: run 'xcode-select --install'. Linux: install git with your package manager."
  fi
  if [ "$OS" = Darwin ]; then
    if xcode-select -p >/dev/null 2>&1; then
      ok "Xcode command line tools present"
    else
      fail_step "Xcode command line tools missing (needed to compile). Run: xcode-select --install"
    fi
  fi
}

brew_has() {
  local p=$1
  [ -n "$(brew list --versions "$p" 2>/dev/null)" ] && return 0
  [ "$p" = openssl ] && [ -n "$(brew list --versions openssl@3 2>/dev/null)" ] && return 0
  return 1
}

step_system_packages() {
  step "system packages"
  local pkgs="cmake pkg-config openssl sqlite"
  if [ "$OS" = Darwin ]; then
    if ! have brew; then
      fail_step "Homebrew not found. Install it from https://brew.sh (its installer needs sudo, so this script will not run it), then re-run."
      return 0
    fi
    ok "Homebrew $(brew --version 2>/dev/null | head -1 | awk '{print $2}')"
    local p
    for p in $pkgs; do
      if brew_has "$p"; then ok "brew: $p"; else BREW_MISSING="$BREW_MISSING $p"; fi
    done
    BREW_MISSING=${BREW_MISSING# }
    if [ -n "$BREW_MISSING" ]; then
      warn "missing brew packages: $BREW_MISSING (the same set CI installs)"
      if confirm "Run 'brew install $BREW_MISSING'?"; then
        # shellcheck disable=SC2086  # intentional split into package names
        act "install missing brew packages" brew install $BREW_MISSING \
          || fail_step "brew install failed"
      else
        fail_step "missing brew packages: $BREW_MISSING (run: brew install $BREW_MISSING)"
      fi
    fi
  else
    local missing=''
    for p in cc cmake pkg-config; do have "$p" || missing="$missing $p"; done
    if [ -n "$missing" ]; then
      warn "missing:$missing. Debian/Ubuntu: sudo apt install build-essential cmake pkg-config libssl-dev libsqlite3-dev"
      [ "$WITH_SERVO" = 1 ] && warn "Servo on Linux also needs its own system libraries; see https://book.servo.org/hacking/setting-up-your-environment.html"
    else
      ok "cc, cmake, pkg-config present"
    fi
  fi
}

step_rust() {
  step "Rust toolchain"
  if ! ferrite_find_cargo; then
    if have rustup; then
      warn "rustup is present but no default toolchain is installed yet; step below installs it."
    else
      warn "rustup/cargo not found."
      if confirm "Install rustup with the official installer from https://sh.rustup.rs (adds ~/.cargo and ~/.rustup; your shell profile is NOT modified)?"; then
        if [ "$DRY_RUN" = 1 ]; then
          printf '  %swould:%s download the rustup installer, then run it\n' "$C_YEL" "$C_OFF"
          note "\$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o <tmpfile>"
          note "\$ sh <tmpfile> -y --no-modify-path --default-toolchain none"
        else
          have curl || { fail_step "curl is needed to download rustup"; return 0; }
          local tmp
          tmp=$(mktemp "${TMPDIR:-/tmp}/rustup-init.XXXXXX")
          # Downloaded to a file first (not piped into sh) so it exists to inspect.
          if curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$tmp" \
             && sh "$tmp" -y --no-modify-path --default-toolchain none; then
            ferrite_find_cargo || true
          else
            fail_step "rustup installation failed"
          fi
          rm -f "$tmp"
        fi
      else
        fail_step "cargo not found. Install Rust from https://rustup.rs, then re-run."
        return 0
      fi
    fi
  fi
  if have rustup; then
    # rust-toolchain.toml pins stable + rustfmt/clippy/llvm-tools. Newer rustup
    # no longer installs it implicitly, so do it explicitly (a no-op if present).
    if ( cd "$FERRITE_REPO_ROOT" && act "install the toolchain pinned by rust-toolchain.toml" rustup toolchain install ); then
      :
    else
      fail_step "rustup toolchain install failed"
    fi
    if [ "$DRY_RUN" != 1 ] && have cargo; then
      ok "$(cd "$FERRITE_REPO_ROOT" && rustc --version 2>/dev/null || echo 'rustc: unknown')"
    fi
  elif have cargo; then
    ok "$(cargo --version) (no rustup: rust-toolchain.toml cannot be applied; a recent stable is expected)"
  fi
  if [ "$DRY_RUN" != 1 ] && ! have cargo; then
    fail_step "cargo still not on PATH. Add \$HOME/.cargo/bin to PATH (or: . \"\$HOME/.cargo/env\") and re-run."
  fi
}

step_choose_servo() {
  if [ "$WITH_SERVO" = ask ]; then
    WITH_SERVO=0
    if [ "$NO_BUILD" = 0 ] && [ "$ASSUME_YES" = 0 ] && [ "$DRY_RUN" = 0 ] && [ -t 0 ]; then
      printf '\n  Build with the real Servo engine? Slow (20-60 min) and large (10+ GB); needed for real web\n'
      printf '  rendering. The default build is fast but has NO real web rendering.\n'
      if confirm "Build with Servo?"; then WITH_SERVO=1; fi
    fi
  fi
}

step_build() {
  step "Build ferrite-shell"
  if [ "$NO_BUILD" = 1 ]; then info "skipped (--no-build)"; return 0; fi
  if [ "$DRY_RUN" != 1 ] && ! have cargo; then
    fail_step "no cargo; build skipped"; return 0
  fi
  local -a cmd
  if [ "$WITH_SERVO" = 1 ]; then
    cmd=(cargo build ${FERRITE_PROFILE_FLAGS[@]+"${FERRITE_PROFILE_FLAGS[@]}"} -p ferrite-shell --features ferrite-servo/servo)
    warn "REAL SERVO build: expect 20-60 minutes and 10+ GB the first time (later builds are incremental)."
    info "Ctrl-C is safe: re-running continues where cargo left off."
  else
    cmd=(cargo build ${FERRITE_PROFILE_FLAGS[@]+"${FERRITE_PROFILE_FLAGS[@]}"} -p ferrite-shell)
    warn "Servo-FREE build: fast (~2 min cold), but it has NO real web rendering."
    info "For real pages re-run with --with-servo (or: just setup-servo)."
  fi
  if ( cd "$FERRITE_REPO_ROOT" && act "build the browser" "${cmd[@]}" ); then
    if [ "$DRY_RUN" != 1 ]; then
      mkdir -p "$FERRITE_HOME"
      if [ "$WITH_SERVO" = 1 ]; then echo servo > "$FERRITE_BUILD_MODE_FILE"; else echo plain > "$FERRITE_BUILD_MODE_FILE"; fi
      ok "built: $CARGO_TARGET_DIR/$FERRITE_PROFILE_DIR/ferrite-shell ($(cat "$FERRITE_BUILD_MODE_FILE"))"
    else
      printf '  %swould:%s record the build mode (%s) in %s\n' "$C_YEL" "$C_OFF" \
        "$([ "$WITH_SERVO" = 1 ] && echo servo || echo plain)" "$FERRITE_BUILD_MODE_FILE"
    fi
  else
    fail_step "cargo build failed (output above)"
  fi
}

PYTHON=''
find_python() {
  local c
  # 3.12 first: the widest torch wheel coverage. 3.10 is Laya's floor.
  for c in ${FERRITE_PYTHON:-} python3.12 python3.11 python3.13 python3.10 python3 python; do
    [ -n "$c" ] || continue
    have "$c" || continue
    if "$c" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then
      PYTHON=$(command -v "$c"); return 0
    fi
  done
  return 1
}

venv_python_ok() {
  [ -x "$FERRITE_LAYA_VENV/bin/python" ] \
    && "$FERRITE_LAYA_VENV/bin/python" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null
}

step_laya() {
  step "Laya decision server (optional)"
  note "Laya is a small local model that proposes 'which operation, which element' for each"
  note "browser step. The app only uses it when FERRITE_LAYA_URL is set; without it the app is LLM-only."
  local ckpt_dir="$FERRITE_LAYA_CKPT_ROOT/$LAYA_CKPT"

  # -- python --
  if ! find_python; then
    if [ "$OS" = Darwin ] && have brew; then
      warn "no Python >= 3.10 found."
      if confirm "Run 'brew install python@3.12'?"; then
        act "install Python 3.12" brew install python@3.12 || { fail_step "brew install python@3.12 failed"; return 0; }
        find_python || true
      fi
    fi
  fi
  if [ -z "$PYTHON" ]; then
    if [ "$DRY_RUN" = 1 ]; then
      warn "no Python >= 3.10 on PATH; a real run would stop here for the Laya part."
    else
      fail_step "Laya needs Python >= 3.10 (found none). macOS: brew install python@3.12. Or skip Laya: --no-laya"
    fi
    return 0
  fi
  ok "python: $PYTHON ($("$PYTHON" -c 'import platform; print(platform.python_version(), platform.machine())'))"
  if [ "$OS" = Darwin ]; then
    local hw_arm pyarch
    hw_arm=$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)
    pyarch=$("$PYTHON" -c 'import platform; print(platform.machine())')
    if [ "$hw_arm" = 1 ] && [ "$pyarch" != arm64 ]; then
      warn "this Mac is Apple silicon but that Python is $pyarch (Rosetta). PyTorch would then run on the CPU only, with no Metal (mps) acceleration."
      warn "Use a native arm64 Python (Homebrew's lives under /opt/homebrew) and set FERRITE_PYTHON=/opt/homebrew/bin/python3.12"
    elif [ "$ARCH" = x86_64 ] && [ "$hw_arm" != 1 ]; then
      warn "Intel Mac: PyTorch has stopped publishing recent macOS x86_64 wheels, so the Laya install may fail to resolve. If it does, use --no-laya."
    fi
  fi

  # -- venv --
  local py="$FERRITE_LAYA_VENV/bin/python"
  if venv_python_ok; then
    ok "venv: $FERRITE_LAYA_VENV"
  elif [ -e "$FERRITE_LAYA_VENV" ]; then
    fail_step "$FERRITE_LAYA_VENV exists but its python is missing or too old. Delete that directory yourself and re-run (this script never deletes it)."
    return 0
  else
    act "create the Laya virtual environment" "$PYTHON" -m venv "$FERRITE_LAYA_VENV" \
      || { fail_step "venv creation failed (Debian/Ubuntu: sudo apt install python3-venv)"; return 0; }
  fi

  # -- packages --
  local spec=${FERRITE_LAYA_PIP_SPEC:-$FERRITE_LAYA_PIP_SPEC_DEFAULT} need_pip=1
  if [ -z "${FERRITE_LAYA_PIP_SPEC:-}" ] && [ "$DRY_RUN" != 1 ] && [ -x "$py" ] \
     && "$py" - <<'PY' 2>/dev/null
import importlib.metadata as m, sys
v = tuple(int(x) for x in m.version("laya").split(".")[:3] if x.isdigit())
import laya.serve, fastapi, uvicorn, huggingface_hub, torch  # noqa: F401
sys.exit(0 if (0, 3, 21) <= v < (0, 4, 0) else 1)
PY
  then
    need_pip=0
    ok "laya + fastapi + uvicorn + huggingface_hub + torch already installed in the venv"
  fi
  if [ "$need_pip" = 1 ]; then
    act "upgrade pip inside the venv" "$py" -m pip install --disable-pip-version-check --upgrade pip \
      || { fail_step "pip upgrade failed"; return 0; }
    if [ "$OS" = Linux ] && ! have nvidia-smi; then
      note "No NVIDIA GPU detected: installing the CPU-only PyTorch build first (the default Linux wheel drags in CUDA libraries)."
      act "install CPU-only torch" "$py" -m pip install --disable-pip-version-check torch --index-url https://download.pytorch.org/whl/cpu \
        || { fail_step "CPU torch install failed"; return 0; }
    elif [ "$OS" = Darwin ]; then
      note "macOS: the default PyPI torch wheel is used (it includes Apple Metal / mps support). Nothing CUDA is installed."
    fi
    act "install Laya and its server dependencies" "$py" -m pip install --disable-pip-version-check "$spec" huggingface_hub \
      || { fail_step "pip install of $spec failed"; return 0; }
  fi

  # -- checkpoint --
  if laya_checkpoint_complete "$ckpt_dir"; then
    ok "checkpoint already downloaded: $ckpt_dir"
  else
    if [ -e "$ckpt_dir" ]; then warn "$ckpt_dir exists but is incomplete; the download resumes it."; fi
    info "Downloading cklxx/laya-browser/$LAYA_CKPT from the Hugging Face Hub (needs internet; size not measured here)."
    if [ "$DRY_RUN" = 1 ]; then
      printf '  %swould:%s download the checkpoint with huggingface_hub.snapshot_download\n' "$C_YEL" "$C_OFF"
      note "\$ $py scripts/laya/fetch_checkpoint.py cklxx/laya-browser $ckpt_dir $LAYA_CKPT   # lists the repo, then snapshot_download"
    else
      mkdir -p "$FERRITE_LAYA_CKPT_ROOT"
      if ! "$py" "$FERRITE_REPO_ROOT/scripts/laya/fetch_checkpoint.py" cklxx/laya-browser "$ckpt_dir" "$LAYA_CKPT" "$FERRITE_LAYA_VENV/bin/hf"
      then
        fail_step "checkpoint download failed (Laya will be unavailable; the browser still works LLM-only)"
        return 0
      fi
    fi
  fi
  if [ "$DRY_RUN" != 1 ]; then
    if laya_checkpoint_complete "$ckpt_dir"; then
      ok "verified: $ckpt_dir has rl_agent_config.json and model.safetensors"
    else
      # shellcheck disable=SC2012  # a human-readable hint, not parsed
      fail_step "download finished but $ckpt_dir lacks rl_agent_config.json/model.safetensors. Found: $(ls "$FERRITE_LAYA_CKPT_ROOT" 2>/dev/null | tr '\n' ' ')"
      return 0
    fi
    "$py" - "$ckpt_dir/rl_agent_config.json" <<'PY' || true
import json, sys
c = json.load(open(sys.argv[1]))
print("    checkpoint config: head_max_len=%r head_max_len_train=%r max_len=%r" % (
    c.get("head_max_len"), c.get("head_max_len_train"), c.get("max_len")))
if c.get("head_max_len_train") is None:
    print("    note: no head_max_len_train in this checkpoint's config; set FERRITE_LAYA_HEAD_MAX_LEN=768 to follow the model card.")
PY
    if [ ! -d "$ckpt_dir/encoder" ]; then
      warn "no encoder/ folder in the checkpoint: the FIRST server start will fetch the base encoder named in its config from the Hub (internet needed once; cached afterwards)."
    fi
    mkdir -p "$FERRITE_LAYA_DIR"
    echo "$LAYA_CKPT" > "$FERRITE_LAYA_CKPT_NAME_FILE"
    if FERRITE_LAYA_CHECKPOINT_DIR="$ckpt_dir" "$py" "$FERRITE_SCRIPTS_DIR/laya/serve.py" --check >/dev/null; then
      ok "server configuration check passed (no model was loaded; that happens on first start)"
    else
      fail_step "scripts/laya/serve.py --check failed (message above)"
    fi
  else
    printf '  %swould:%s record the chosen checkpoint (%s) in %s\n' "$C_YEL" "$C_OFF" "$LAYA_CKPT" "$FERRITE_LAYA_CKPT_NAME_FILE"
    printf '  %swould:%s run scripts/laya/serve.py --check (configuration check, loads no model)\n' "$C_YEL" "$C_OFF"
  fi
}

step_env_file() {
  step "Local environment file"
  if [ -e "$FERRITE_ENV_FILE" ]; then
    ok "$FERRITE_ENV_FILE exists; left untouched"
  else
    if [ "$DRY_RUN" = 1 ]; then
      printf '  %swould:%s create %s from scripts/local.env.example (mode 600)\n' "$C_YEL" "$C_OFF" "$FERRITE_ENV_FILE"
    else
      mkdir -p "$FERRITE_HOME"
      ( umask 077; cp "$FERRITE_SCRIPTS_DIR/local.env.example" "$FERRITE_ENV_FILE" )
      chmod 600 "$FERRITE_ENV_FILE"
      ok "created $FERRITE_ENV_FILE (every setting is commented out)"
    fi
  fi
  print_keyring_help
}

step_summary() {
  step "Done"
  if [ -n "$FAILED" ]; then
    warn "some steps did not complete:"
    printf '%s\n' "$FAILED" | while IFS= read -r line; do printf '      - %s\n' "$line" >&2; done
  fi
  echo
  info "Next:"
  info "  1. Edit $FERRITE_ENV_FILE and set FERRITE_MODEL_SMALL and FERRITE_MODEL_MAIN"
  info "     (no model name is ever a default; 'just models-local' lists what your endpoint serves)."
  info "  2. Provide OLLAMA_API_KEY via your environment or the keychain (commands above)."
  info "  3. just run-local        # starts Laya (if set up) and the browser"
  info "     just doctor           # checklist of what is and is not ready"
  if [ "$DRY_RUN" = 1 ]; then echo; warn "That was a dry run: nothing was changed."; fi
  [ -z "$FAILED" ]
}

step_choose_servo
step_preflight
step_git
step_system_packages
step_rust
step_build
if [ "$DO_LAYA" = 1 ]; then step_laya; else step "Laya"; info "skipped (--no-laya)"; fi
step_env_file
step_summary
