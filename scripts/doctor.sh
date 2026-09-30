#!/usr/bin/env bash
# scripts/doctor.sh — a checklist of what is and is not ready on this machine.
# Read-only: it changes nothing, and never prints an API key.
# Exit status: 0 if nothing FAILed (warnings are allowed), 1 otherwise.
#
#   scripts/doctor.sh [--fix-hints]
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/common.sh
. "$HERE/lib/common.sh"

FIX_HINTS=0
NO_PROBE=0
while [ $# -gt 0 ]; do
  case $1 in
    --fix-hints) FIX_HINTS=1 ;;
    --no-probe) NO_PROBE=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: scripts/doctor.sh [--fix-hints] [--no-probe]

Checks tools and versions, the build, env.local, model tags, API keys (env or
keychain; the value is never printed), the Laya venv / checkpoint / server (with
a small latency probe if a server is answering), and free disk.

  --fix-hints   print how to fix each problem
  --no-probe    do not send the Laya latency-probe requests

Exit status: 0 if nothing FAILed (warnings are allowed), 1 otherwise.
EOF
      exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

NFAIL=0
NWARN=0
p_ok()   { ok "$*"; }
p_warn() { NWARN=$((NWARN + 1)); warn "$*"; }
p_fail() { NFAIL=$((NFAIL + 1)); bad "$*"; }
hint()   { if [ "$FIX_HINTS" = 1 ]; then printf '         %sfix:%s %s\n' "$C_DIM" "$C_OFF" "$*"; fi; }

export FERRITE_HOME
ferrite_load_env_file "$FERRITE_ENV_FILE" 2>/dev/null || true
ferrite_find_cargo >/dev/null 2>&1 || true

printf '%sFerrite doctor%s   home=%s  target=%s\n' "$C_BLD" "$C_OFF" "$FERRITE_HOME" "$CARGO_TARGET_DIR"

# ── tools ─────────────────────────────────────────────────────────────────
step "Tools"
if have git; then p_ok "git $(git --version | awk '{print $3}')"; else p_fail "git not found"; hint "macOS: xcode-select --install"; fi
if have cargo; then
  p_ok "$(cargo --version)"
  # rustc in the repo dir, so rust-toolchain.toml's pin is the one reported.
  p_ok "$(cd "$FERRITE_REPO_ROOT" && rustc --version 2>/dev/null || echo 'rustc: not available')"
else
  p_fail "cargo not found"; hint "scripts/setup-local.sh (offers to install rustup), or https://rustup.rs then PATH+=\$HOME/.cargo/bin"
fi
if have just; then p_ok "just $(just --version | awk '{print $2}')"; else p_warn "just not found (optional: scripts can be run directly)"; hint "brew install just"; fi
if [ "$(uname -s)" = Darwin ]; then
  if have brew; then
    p_ok "Homebrew present"
    for pkg in cmake pkg-config openssl sqlite; do
      if [ -n "$(brew list --versions "$pkg" 2>/dev/null)" ] || { [ "$pkg" = openssl ] && [ -n "$(brew list --versions openssl@3 2>/dev/null)" ]; }; then
        :
      else
        p_warn "brew package missing: $pkg (CI installs it)"; hint "brew install $pkg"
      fi
    done
  else
    p_warn "Homebrew not found"; hint "https://brew.sh"
  fi
fi
PYBIN=''
for c in ${FERRITE_PYTHON:-} python3.12 python3.11 python3.13 python3.10 python3; do
  if [ -z "$c" ] || ! have "$c"; then continue; fi
  if "$c" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then PYBIN=$(command -v "$c"); break; fi
done
if [ -n "$PYBIN" ]; then
  p_ok "python $("$PYBIN" -c 'import platform; print(platform.python_version(), platform.machine())') ($PYBIN)"
else
  p_warn "no Python >= 3.10 (needed only for Laya)"; hint "brew install python@3.12"
fi

# ── build ─────────────────────────────────────────────────────────────────
step "Build"
BIN=$CARGO_TARGET_DIR/release/ferrite-shell
MODE=unknown
[ -f "$FERRITE_BUILD_MODE_FILE" ] && MODE=$(cat "$FERRITE_BUILD_MODE_FILE")
if [ -x "$BIN" ]; then
  p_ok "release binary present: $BIN"
  case $MODE in
    servo) p_ok "last recorded build: WITH Servo (real web rendering)" ;;
    plain) p_warn "last recorded build: Servo-free (NO real web rendering)"; hint "just setup-servo" ;;
    *) p_warn "no build mode recorded (built outside setup-local.sh?): cannot tell whether Servo is in it"; hint "just setup-servo   (or just setup for the Servo-free build)" ;;
  esac
else
  p_warn "no release binary at $BIN (run-local builds it on first use, or run setup)"; hint "just setup"
fi

# ── configuration ─────────────────────────────────────────────────────────
step "Configuration"
if [ -f "$FERRITE_ENV_FILE" ]; then
  p_ok "env file: $FERRITE_ENV_FILE"
  if [ -n "$(find "$FERRITE_ENV_FILE" -maxdepth 0 -perm -0077 2>/dev/null)" ]; then
    p_warn "env file is readable by other users"; hint "chmod 600 $FERRITE_ENV_FILE"
  fi
  SECRETISH=''
  while IFS= read -r entry; do
    k=${entry%%=*}; v=${entry#*=}
    case $k in *KEY*|*TOKEN*|*SECRET*) [ -n "$v" ] && SECRETISH="$SECRETISH $k" ;; esac
  done < <(ferrite_env_entries "$FERRITE_ENV_FILE" 2>/dev/null)
  if [ -n "$SECRETISH" ]; then
    p_warn "env file holds secret values for:$SECRETISH (the app's rule is environment or keychain only)"
    hint "move them to the keychain: security add-generic-password -U -s ferrite -a <NAME> -w   then delete the line"
  fi
else
  p_warn "no env file at $FERRITE_ENV_FILE"; hint "just setup (creates it from scripts/local.env.example)"
fi
for v in FERRITE_MODEL_SMALL FERRITE_MODEL_MAIN; do
  if [ -n "${!v:-}" ]; then p_ok "$v=${!v}"; else p_fail "$v is not set (no model name is ever a default)"; hint "set it in $FERRITE_ENV_FILE; 'scripts/run-local.sh --list-models' shows the tags your endpoint serves"; fi
done
case ${FERRITE_OLLAMA_BASE_URL:-} in
  http://localhost*|http://127.0.0.1*|http://\[::1\]*) p_ok "Ollama endpoint is local (${FERRITE_OLLAMA_BASE_URL}); no API key needed" ;;
  *)
    if [ -n "${OLLAMA_API_KEY:-}" ]; then
      p_ok "OLLAMA_API_KEY present in the environment (value not shown)"
    elif [ "$(uname -s)" = Darwin ] && keyring_has OLLAMA_API_KEY; then
      p_ok "OLLAMA_API_KEY present in the macOS Keychain (value not shown)"
    elif [ -n "${FERRITE_GEMINI_API_KEY:-}" ] || { [ "$(uname -s)" = Darwin ] && keyring_has FERRITE_GEMINI_API_KEY; }; then
      p_ok "Gemini key present (Ollama key absent; the app falls back to Gemini)"
    elif [ "$(uname -s)" = Darwin ]; then
      p_fail "no OLLAMA_API_KEY in the environment or Keychain"
      hint "security add-generic-password -U -s ferrite -a OLLAMA_API_KEY -w   (prompts for the value)"
    else
      p_warn "no OLLAMA_API_KEY in the environment (the OS keyring is not checked on this platform)"
      hint "export OLLAMA_API_KEY=..."
    fi ;;
esac
if [ -n "${FERRITE_TWIN_KEY:-}" ] || { [ "$(uname -s)" = Darwin ] && keyring_has FERRITE_TWIN_KEY; }; then
  p_ok "twin encryption key present (value not shown)"
else
  p_warn "no FERRITE_TWIN_KEY in the environment or Keychain (the dry-run twin needs one when it is used)"
  hint "export FERRITE_TWIN_KEY=\$(openssl rand -hex 32)   or store it: security add-generic-password -U -s ferrite -a FERRITE_TWIN_KEY -w"
fi

# ── Laya ──────────────────────────────────────────────────────────────────
step "Laya (optional)"
PY=$FERRITE_LAYA_VENV/bin/python
CKPT=$(laya_checkpoint_dir)
LAYA_READY=1
if [ -x "$PY" ]; then
  p_ok "venv: $FERRITE_LAYA_VENV"
  if V=$("$PY" -c 'import importlib.metadata as m, fastapi, uvicorn, torch; print("laya", m.version("laya"), "torch", torch.__version__)' 2>/dev/null); then
    p_ok "$V, fastapi, uvicorn importable"
    DEV=$("$PY" -c 'import torch; print("mps" if getattr(torch.backends,"mps",None) and torch.backends.mps.is_available() else "cuda" if torch.cuda.is_available() else "cpu")' 2>/dev/null || echo unknown)
    p_ok "torch would use: ${FERRITE_LAYA_DEVICE:-auto -> $DEV}"
  else
    LAYA_READY=0; p_warn "the venv is missing laya/torch/fastapi/uvicorn"; hint "just setup"
  fi
else
  LAYA_READY=0; p_warn "no Laya venv at $FERRITE_LAYA_VENV (Laya not set up; the app runs LLM-only)"; hint "just setup   (Laya is on by default)"
fi
if laya_checkpoint_complete "$CKPT"; then
  p_ok "checkpoint: $CKPT"
  [ -d "$CKPT/encoder" ] || p_warn "checkpoint has no encoder/ folder: the first server start will fetch the base encoder from the Hugging Face Hub"
else
  LAYA_READY=0; p_warn "no complete checkpoint at $CKPT"; hint "just setup   (downloads cklxx/laya-browser; needs internet)"
fi

laya_resolve_endpoint
URL=${FERRITE_LAYA_URL:-$LAYA_LOCAL_URL}
if http_ok "$URL/health"; then
  p_ok "server answering: $URL/health"
  if [ "$NO_PROBE" = 1 ]; then
    note "latency probe skipped (--no-probe)"
  elif [ -z "$PYBIN" ]; then
    p_warn "no python3 to run the latency probe"
  else
    if OUT=$("$PYBIN" "$FERRITE_SCRIPTS_DIR/laya/verify.py" --url "$URL" --repeat 3 --timeout 30 2>&1); then
      printf '%s\n' "$OUT" | grep -E '^request ' | sed 's/^/         /'
      p_ok "probe: a well-formed answer came back"
      if printf '%s\n' "$OUT" | grep -q '^WARNING'; then
        p_warn "the model's choice on the recorded step was not the obvious one (wrong checkpoint or head_max_len?)"
        hint "run: python3 scripts/laya/verify.py   and read the WARNING text"
      fi
    else
      p_fail "probe failed: $(printf '%s\n' "$OUT" | grep -E '^verify:' | head -1)"
      hint "see $FERRITE_LAYA_LOG"
    fi
  fi
else
  if [ "$LAYA_READY" = 1 ]; then
    p_warn "no server answering at $URL (start it: just laya-serve, or just run-local)"
  else
    note "no server at $URL (expected: Laya is not fully set up)"
  fi
fi

# ── disk ──────────────────────────────────────────────────────────────────
step "Disk"
for d in "$FERRITE_HOME" "$CARGO_TARGET_DIR"; do
  g=$(free_gb "$d" || true)
  if [ -z "$g" ]; then p_warn "cannot read free space for $d"
  elif [ "$g" -lt 5 ]; then p_warn "only ${g} GB free where $d lives"; hint "free space; a Servo build alone needs 10+ GB"
  else p_ok "${g} GB free where $d lives"
  fi
done
[ -d "$CARGO_TARGET_DIR" ] && note "target dir size: $(du -sh "$CARGO_TARGET_DIR" 2>/dev/null | awk '{print $1}')"
[ -d "$FERRITE_HOME" ] && note "FERRITE_HOME size: $(du -sh "$FERRITE_HOME" 2>/dev/null | awk '{print $1}')"

# ── summary ───────────────────────────────────────────────────────────────
echo
if [ "$NFAIL" -gt 0 ]; then
  printf '%s%d problem(s), %d warning(s).%s' "$C_RED" "$NFAIL" "$NWARN" "$C_OFF"
else
  printf '%sno problems, %d warning(s).%s' "$C_GRN" "$NWARN" "$C_OFF"
fi
if [ "$FIX_HINTS" = 0 ] && [ $((NFAIL + NWARN)) -gt 0 ]; then printf '  Re-run with --fix-hints for how to fix them.'; fi
echo
[ "$NFAIL" -eq 0 ]
