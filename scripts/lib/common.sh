# shellcheck shell=bash
# shellcheck disable=SC2034  # the variables below are consumed by the sourcing scripts
# scripts/lib/common.sh — shared helpers for the local setup/run toolchain.
# Sourced by setup-local.sh, run-local.sh and doctor.sh; never executed.
#
# Written for bash 3.2 (the /bin/bash macOS ships): no associative arrays, no
# mapfile, no ${var,,}, and empty arrays are expanded with the
# ${arr[@]+"${arr[@]}"} idiom because `set -u` treats a bare "${arr[@]}" of an
# empty array as an error there.
#
# Everything this toolchain writes lives under $FERRITE_HOME (default: the
# repo's own gitignored `.ferrite/` directory), so `rm -rf "$FERRITE_HOME"`
# removes all of the local state and nothing else on the machine is touched.
# The whole project — sources, build output, Python venv, Laya checkpoints,
# settings — therefore lives in one folder.

FERRITE_SCRIPTS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FERRITE_REPO_ROOT=$(cd "$FERRITE_SCRIPTS_DIR/.." && pwd)

FERRITE_HOME=${FERRITE_HOME:-$FERRITE_REPO_ROOT/.ferrite}
FERRITE_ENV_FILE=$FERRITE_HOME/env.local
FERRITE_LAYA_DIR=$FERRITE_HOME/laya
FERRITE_LAYA_VENV=$FERRITE_LAYA_DIR/venv
FERRITE_LAYA_CKPT_ROOT=$FERRITE_LAYA_DIR/checkpoints/laya-browser
FERRITE_LAYA_PIDFILE=$FERRITE_LAYA_DIR/serve.pid
FERRITE_LAYA_LOG=$FERRITE_LAYA_DIR/serve.log
FERRITE_BUILD_MODE_FILE=$FERRITE_HOME/build.mode
FERRITE_LAYA_CKPT_NAME_FILE=$FERRITE_LAYA_DIR/checkpoint.name

FERRITE_LAYA_DEFAULT_HOST=127.0.0.1
FERRITE_LAYA_DEFAULT_PORT=8765
FERRITE_LAYA_DEFAULT_CHECKPOINT=v10s
# Laya is pinned to the 0.3 line: scripts/laya/serve.py subclasses
# laya.router.Router and reads Agent.cfg, which are library internals. It was
# read against 0.3.21. Override with FERRITE_LAYA_PIP_SPEC to try another.
FERRITE_LAYA_PIP_SPEC_DEFAULT='laya[serve]>=0.3.21,<0.4'

# The Cargo target dir. The justfile exports the same default for `just`
# recipes; repeating it here keeps a direct `scripts/run-local.sh` and
# `just run-local` building into ONE directory instead of two.
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$FERRITE_REPO_ROOT/target}

# ── output ────────────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_BLU=$'\033[34m'
  C_DIM=$'\033[2m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
  C_RED=''; C_GRN=''; C_YEL=''; C_BLU=''; C_DIM=''; C_BLD=''; C_OFF=''
fi

step() { printf '\n%s==> %s%s\n' "$C_BLD$C_BLU" "$*" "$C_OFF"; }
info() { printf '    %s\n' "$*"; }
ok()   { printf '  %s[ ok ]%s %s\n' "$C_GRN" "$C_OFF" "$*"; }
warn() { printf '  %s[warn]%s %s\n' "$C_YEL" "$C_OFF" "$*" >&2; }
bad()  { printf '  %s[FAIL]%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; }
note() { printf '  %s%s%s\n' "$C_DIM" "$*" "$C_OFF"; }
die()  { printf '%serror:%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# Print a command the way a shell would need it typed.
quote_cmd() {
  local out='' a
  for a in "$@"; do out="$out$(printf '%q ' "$a")"; done
  printf '%s' "${out% }"
}

# Dotted-version comparison: version_ge 3.10.4 3.10 -> success.
version_ge() {
  local a=$1 b=$2 i x y
  local IFS=.
  # shellcheck disable=SC2206  # intentional word-splitting on '.'
  local av=($a) bv=($b)
  for i in 0 1 2; do
    x=${av[$i]:-0}; y=${bv[$i]:-0}
    x=${x//[!0-9]/}; y=${y//[!0-9]/}
    [ "${x:-0}" -gt "${y:-0}" ] && return 0
    [ "${x:-0}" -lt "${y:-0}" ] && return 1
  done
  return 0
}

# ── env-file parsing ──────────────────────────────────────────────────────
# $FERRITE_HOME/env.local is plain KEY=VALUE. It is NEVER `source`d: a sourced
# file runs arbitrary shell (`$(...)`, backticks, `;`). This parser reads it as
# data, expands nothing, and only lets through a whitelist of key families:
#   FERRITE_*  OLLAMA_*  GEMINI_*  RUST_LOG
# (Anything else — PATH, LD_PRELOAD, DYLD_*, CARGO_*, HOME — is refused.)
#
# Accepted line shapes:
#   KEY=value        KEY="quoted value"     KEY='quoted value'
#   export KEY=value                         # comment lines / blank lines
#   KEY=value   # trailing comment (only when '#' follows whitespace)
# Quoted values end at the next matching quote; there are no escapes and no
# $VAR / $(cmd) expansion of any kind.
ferrite_env_key_allowed() {
  case $1 in
    FERRITE_*|OLLAMA_*|GEMINI_*|RUST_LOG) return 0 ;;
    *) return 1 ;;
  esac
}

# ferrite_env_entries FILE — print each accepted entry as KEY=VALUE, one per
# line. Rejected lines are reported on stderr by line number and key NAME only
# (never the value, which may be a secret).
ferrite_env_entries() {
  local file=$1 line key val rest lineno=0
  [ -r "$file" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    lineno=$((lineno + 1))
    line=${line%$'\r'}
    line=${line#"${line%%[![:space:]]*}"}                 # trim leading space
    case $line in ''|'#'*) continue ;; esac
    case $line in
      export[[:space:]]*) line=${line#export}; line=${line#"${line%%[![:space:]]*}"} ;;
    esac
    case $line in
      *=*) ;;
      *) printf '%s:%d: not KEY=VALUE, ignored\n' "$file" "$lineno" >&2; continue ;;
    esac
    key=${line%%=*}
    val=${line#*=}
    key=${key%"${key##*[![:space:]]}"}                    # trim trailing space
    case $key in
      ''|[!A-Za-z_]*|*[!A-Za-z0-9_]*)
        printf '%s:%d: invalid variable name, ignored\n' "$file" "$lineno" >&2; continue ;;
    esac
    if ! ferrite_env_key_allowed "$key"; then
      printf '%s:%d: %s is not an allowed key (FERRITE_*, OLLAMA_*, GEMINI_*, RUST_LOG), ignored\n' \
        "$file" "$lineno" "$key" >&2
      continue
    fi
    val=${val#"${val%%[![:space:]]*}"}                    # trim leading space
    case $val in
      \"*)
        rest=${val#\"}
        case $rest in
          *\"*) val=${rest%%\"*} ;;
          *) printf '%s:%d: unterminated quote for %s, ignored\n' "$file" "$lineno" "$key" >&2; continue ;;
        esac ;;
      \'*)
        rest=${val#\'}
        case $rest in
          *\'*) val=${rest%%\'*} ;;
          *) printf '%s:%d: unterminated quote for %s, ignored\n' "$file" "$lineno" "$key" >&2; continue ;;
        esac ;;
      *)
        val=${val%%[[:space:]]#*}                         # strip ' # comment'
        val=${val%"${val##*[![:space:]]}"}                # trim trailing space
        ;;
    esac
    printf '%s=%s\n' "$key" "$val"
  done < "$file"
}

# ferrite_load_env_file FILE — export accepted entries. A variable that is
# already set (non-empty) in the calling environment WINS over the file, so a
# one-off `FERRITE_DEFENSE=off just run-local` behaves as typed.
ferrite_load_env_file() {
  local file=$1 entry key val
  [ -f "$file" ] || return 0
  while IFS= read -r entry; do
    key=${entry%%=*}
    val=${entry#*=}
    if [ -n "${!key:-}" ]; then continue; fi
    export "$key=$val"
  done < <(ferrite_env_entries "$file")
}

# ── Laya paths / state ────────────────────────────────────────────────────
# Which checkpoint: env FERRITE_LAYA_CHECKPOINT, else what setup recorded,
# else the default. Constrained to a plain directory name (env.local is data
# we do not fully trust; this keeps it from pointing outside the checkpoint
# root).
laya_checkpoint_name() {
  local n=${FERRITE_LAYA_CHECKPOINT:-}
  if [ -z "$n" ] && [ -f "$FERRITE_LAYA_CKPT_NAME_FILE" ]; then
    IFS= read -r n < "$FERRITE_LAYA_CKPT_NAME_FILE" || true
  fi
  n=${n:-$FERRITE_LAYA_DEFAULT_CHECKPOINT}
  case $n in
    ''|*[!A-Za-z0-9._-]*|.|..) n=$FERRITE_LAYA_DEFAULT_CHECKPOINT ;;
  esac
  printf '%s' "$n"
}

laya_checkpoint_dir() {
  if [ -n "${FERRITE_LAYA_CHECKPOINT_DIR:-}" ]; then
    printf '%s' "$FERRITE_LAYA_CHECKPOINT_DIR"
  else
    printf '%s/%s' "$FERRITE_LAYA_CKPT_ROOT" "$(laya_checkpoint_name)"
  fi
}

# A Laya checkpoint directory always carries these two files
# (laya/agent.py refuses a directory without them).
laya_checkpoint_complete() {
  [ -f "$1/rl_agent_config.json" ] && [ -f "$1/model.safetensors" ]
}

is_loopback_host() {
  case $1 in
    127.*|localhost|::1|'[::1]') return 0 ;;
    *) return 1 ;;
  esac
}

# Host part / port part of an http(s)://host:port[/...] URL.
url_host() {
  local r=${1#*://}
  r=${r%%/*}; r=${r##*@}
  case $r in
    \[*) r=${r%%]*}; printf '%s' "${r#[}" ;;
    *) printf '%s' "${r%%:*}" ;;
  esac
}
url_port() {
  local r=${1#*://}
  r=${r%%/*}; r=${r##*@}
  case $r in
    \[*\]:*) printf '%s' "${r##*]:}" ;;
    \[*) printf '' ;;
    *:*) printf '%s' "${r##*:}" ;;
    *) printf '' ;;
  esac
}

# laya_resolve_endpoint — from the environment, set:
#   LAYA_BIND_HOST / LAYA_BIND_PORT   what the managed local server binds
#   LAYA_LOCAL_URL                    the URL a client uses to reach it
# A loopback FERRITE_LAYA_URL only contributes its port (it names OUR server).
laya_resolve_endpoint() {
  local connect
  LAYA_BIND_HOST=${FERRITE_LAYA_HOST:-$FERRITE_LAYA_DEFAULT_HOST}
  LAYA_BIND_PORT=${FERRITE_LAYA_PORT:-}
  if [ -z "$LAYA_BIND_PORT" ] && [ -n "${FERRITE_LAYA_URL:-}" ]; then
    LAYA_BIND_PORT=$(url_port "$FERRITE_LAYA_URL")
  fi
  LAYA_BIND_PORT=${LAYA_BIND_PORT:-$FERRITE_LAYA_DEFAULT_PORT}
  case $LAYA_BIND_HOST in
    0.0.0.0) connect=127.0.0.1 ;;
    ::) connect='[::1]' ;;
    *:*) connect="[$LAYA_BIND_HOST]" ;;
    *) connect=$LAYA_BIND_HOST ;;
  esac
  LAYA_LOCAL_URL=http://$connect:$LAYA_BIND_PORT
}

# ── misc ──────────────────────────────────────────────────────────────────
# http_ok URL — success iff GET returns 2xx within a few seconds.
http_ok() {
  if have curl; then
    curl -fsS --max-time 3 -o /dev/null "$1" >/dev/null 2>&1
  elif have python3; then
    python3 - "$1" <<'PY' >/dev/null 2>&1
import sys, urllib.request
urllib.request.urlopen(sys.argv[1], timeout=3).read()
PY
  else
    return 1
  fi
}

pid_alive() { [ -n "${1:-}" ] && kill -0 "$1" 2>/dev/null; }

# Free disk in whole GB on the filesystem holding $1 (walks up to an existing
# parent). Prints nothing if df is unusable.
free_gb() {
  local p=$1 kb
  while [ ! -e "$p" ] && [ "$p" != / ]; do p=$(dirname "$p"); done
  kb=$(df -k "$p" 2>/dev/null | awk 'NR==2 {print $4}')
  case $kb in ''|*[!0-9]*) return 0 ;; esac
  printf '%s' $((kb / 1024 / 1024))
}

# Source rustup's env file when cargo is installed but not on PATH yet
# (a fresh rustup install, or a shell that never sourced ~/.cargo/env).
ferrite_find_cargo() {
  have cargo && return 0
  # shellcheck disable=SC1091
  if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
  have cargo
}

# The keyring commands, printed for the user to run. The app reads
# service "ferrite", account = the variable name (crates/ferrite-model/src/
# secret.rs). `-w` as the LAST option makes macOS `security` prompt for the
# value, so the secret never appears in argv or shell history.
print_keyring_help() {
  note "Secrets are never written by these scripts. Give the app a key ONE of two ways:"
  info "1) environment (this shell / your shell profile):   export OLLAMA_API_KEY=..."
  info "2) macOS Keychain (persistent; the app looks up service \"ferrite\", account = variable name):"
  info "     security add-generic-password -U -s ferrite -a OLLAMA_API_KEY -w"
  info "   (it prompts for the value). Gemini fallback account: FERRITE_GEMINI_API_KEY."
  info "   Twin encryption key account: FERRITE_TWIN_KEY (or export FERRITE_TWIN_KEY=...)."
}

keyring_has() { # keyring_has ACCOUNT — macOS only; never prints the secret
  have security || return 2
  security find-generic-password -s ferrite -a "$1" >/dev/null 2>&1
}
