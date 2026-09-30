#!/usr/bin/env bash
# scripts/run-local.sh — run Ferrite on this machine, with the local Laya
# decision server if it has been set up. Safe to run every time.
#
#   scripts/run-local.sh --help
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/common.sh
. "$HERE/lib/common.sh"

usage() {
  cat <<'EOF'
Usage: scripts/run-local.sh [options] [-- ferrite-shell arguments]

  1. loads $FERRITE_HOME/env.local (plain KEY=VALUE, parsed as data; only
     FERRITE_*, OLLAMA_*, GEMINI_*, RUST_LOG are read; variables already
     exported in your shell win over the file)
  2. checks the model tags are configured (says exactly what is missing)
  3. starts the local Laya server in the background if it is set up and not
     already running (pid + log under $FERRITE_HOME/laya/), waits for /health,
     and exports FERRITE_LAYA_URL for the app
  4. runs the browser:  cargo run --release -p ferrite-shell [--features
     ferrite-servo/servo] -- ui
  5. on exit / Ctrl-C / TERM, stops the Laya server IT started (never one that
     was already running)

Options:
  --no-laya           do not start or use a Laya server (LLM-only)
  --servo / --no-servo  force the Servo (real rendering) / Servo-free build;
                      default: whatever `scripts/setup-local.sh` built last
  --wait SECONDS      how long to wait for Laya's /health (default 120; the
                      first start loads the model)
  --no-model-check    launch even if FERRITE_MODEL_SMALL/MAIN are unset
  --list-models       load env.local, then list the model tags your endpoint
                      serves (what `just models` does, without needing the
                      tags to be set first) and exit
  --laya-only         run ONLY the Laya server, in the foreground (Ctrl-C to
                      stop); refuses if one is already running
  --verify            send one recorded browser step to the Laya server and
                      print its decision + latency (scripts/laya/verify.py)
  --dry-run           print what would happen; start nothing
  -h, --help          this text
  -- ARGS...          pass ARGS to ferrite-shell instead of `ui`
                      (subcommands: ui, window, jstest, agent-smoke, smoke)

Laya settings (env or env.local): FERRITE_LAYA_URL (use an external server; we
will not start one), FERRITE_LAYA_HOST, FERRITE_LAYA_PORT (default
127.0.0.1:8765), FERRITE_LAYA_CHECKPOINT, FERRITE_LAYA_API_KEY. See
scripts/local.env.example.
EOF
}

DRY_RUN=0
USE_LAYA=1
SERVO=auto
WAIT=120
CHECK_MODELS=1
LIST_MODELS=0
LAYA_ONLY=0
DO_VERIFY=0
APP_ARGS=()

while [ $# -gt 0 ]; do
  case $1 in
    --no-laya) USE_LAYA=0 ;;
    --laya) USE_LAYA=1 ;;
    --servo) SERVO=1 ;;
    --no-servo) SERVO=0 ;;
    --wait) [ $# -ge 2 ] || die "--wait needs a number of seconds"; WAIT=$2; shift ;;
    --wait=*) WAIT=${1#*=} ;;
    --no-model-check) CHECK_MODELS=0 ;;
    --list-models) LIST_MODELS=1 ;;
    --laya-only) LAYA_ONLY=1 ;;
    --verify) DO_VERIFY=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage; exit 0 ;;
    --) shift; APP_ARGS=("$@"); break ;;
    *) usage >&2; die "unknown option: $1 (arguments for the app go after --)" ;;
  esac
  shift
done
case $WAIT in ''|*[!0-9]*) die "--wait must be a whole number of seconds" ;; esac
[ ${#APP_ARGS[@]} -gt 0 ] || APP_ARGS=(ui)

export FERRITE_HOME

# ── 1. environment ────────────────────────────────────────────────────────
if [ -f "$FERRITE_ENV_FILE" ]; then
  ferrite_load_env_file "$FERRITE_ENV_FILE"
  info "environment: $FERRITE_ENV_FILE"
else
  warn "no $FERRITE_ENV_FILE yet (run 'just setup'); using only your shell environment"
fi

NEED_CARGO="cargo not found. Run 'just setup' (or install Rust from https://rustup.rs and add \$HOME/.cargo/bin to PATH)."

if [ "$LIST_MODELS" = 1 ]; then
  ferrite_find_cargo || die "$NEED_CARGO"
  # The tag loader insists both tags are non-empty even just to list them; any
  # placeholder does (crates/ferrite-model/src/config.rs require_tag).
  export FERRITE_MODEL_SMALL=${FERRITE_MODEL_SMALL:-placeholder}
  export FERRITE_MODEL_MAIN=${FERRITE_MODEL_MAIN:-placeholder}
  cd "$FERRITE_REPO_ROOT"
  if [ "$DRY_RUN" = 1 ]; then note "would run: cargo run -p ferrite-model --example models"; exit 0; fi
  exec cargo run -p ferrite-model --example models
fi

# Is the pid in the pidfile a live Laya server started by these scripts?
pidfile_live_pid() {
  local pid=''
  [ -f "$FERRITE_LAYA_PIDFILE" ] || return 1
  IFS= read -r pid < "$FERRITE_LAYA_PIDFILE" || true
  case $pid in ''|*[!0-9]*) return 1 ;; esac
  pid_alive "$pid" || return 1
  # Guard against a recycled pid: the command line must be our server.
  ps -p "$pid" -o command= 2>/dev/null | grep -q 'laya/serve.py' || return 1
  printf '%s' "$pid"
}

# ── Laya-only modes (no model tags needed) ────────────────────────────────
if [ "$DO_VERIFY" = 1 ]; then
  have python3 || die "python3 is needed to run verify.py"
  laya_resolve_endpoint
  target=${FERRITE_LAYA_URL:-$LAYA_LOCAL_URL}
  if [ "$DRY_RUN" = 1 ]; then note "would run: python3 scripts/laya/verify.py --url $target"; exit 0; fi
  exec python3 "$FERRITE_SCRIPTS_DIR/laya/verify.py" --url "$target"
fi

if [ "$LAYA_ONLY" = 1 ]; then
  laya_resolve_endpoint
  py=$FERRITE_LAYA_VENV/bin/python
  ckpt=$(laya_checkpoint_dir)
  [ -x "$py" ] || die "no Laya venv at $FERRITE_LAYA_VENV. Run 'just setup' first."
  laya_checkpoint_complete "$ckpt" || die "no complete checkpoint at $ckpt. Run 'just setup' first."
  if [ "$DRY_RUN" = 1 ]; then
    note "would run in the foreground: $py -u $FERRITE_SCRIPTS_DIR/laya/serve.py (bind $LAYA_BIND_HOST:$LAYA_BIND_PORT)"
    exit 0
  fi
  if live=$(pidfile_live_pid); then die "a Laya server is already running (pid $live). Stop it, or use 'just run-local'."; fi
  if http_ok "$LAYA_LOCAL_URL/health"; then die "something already answers at $LAYA_LOCAL_URL. Stop it or change FERRITE_LAYA_PORT."; fi
  export FERRITE_LAYA_HOST=$LAYA_BIND_HOST FERRITE_LAYA_PORT=$LAYA_BIND_PORT
  info "Laya server on $LAYA_BIND_HOST:$LAYA_BIND_PORT (Ctrl-C to stop). First start loads the model."
  exec "$py" -u "$FERRITE_SCRIPTS_DIR/laya/serve.py"
fi

ferrite_find_cargo || die "$NEED_CARGO"

# ── 2. model configuration ────────────────────────────────────────────────
if [ "$CHECK_MODELS" = 1 ]; then
  missing=''
  [ -n "${FERRITE_MODEL_SMALL:-}" ] || missing="FERRITE_MODEL_SMALL"
  [ -n "${FERRITE_MODEL_MAIN:-}" ] || missing="$missing${missing:+ }FERRITE_MODEL_MAIN"
  if [ -n "$missing" ]; then
    {
      printf '%serror:%s not configured: %s\n\n' "$C_RED" "$C_OFF" "$missing"
      cat <<EOF
  Model names are never defaults (Ollama retires cloud models), so the agent
  cannot run until you choose them. Set them in $FERRITE_ENV_FILE:

      FERRITE_MODEL_SMALL=<a model tag>
      FERRITE_MODEL_MAIN=<a model tag>      (may be the same tag)

  or export them in your shell. To see which tags your endpoint serves:

      scripts/run-local.sh --list-models     (needs OLLAMA_API_KEY, or a local
                                              Ollama via FERRITE_OLLAMA_BASE_URL)

  To open the UI anyway without the agent: --no-model-check
EOF
    } >&2
    exit 2
  fi
  ok "model tags: small=$FERRITE_MODEL_SMALL main=$FERRITE_MODEL_MAIN"
fi

# API key advisory (never printed). Local Ollama needs none.
case ${FERRITE_OLLAMA_BASE_URL:-} in
  http://localhost*|http://127.0.0.1*|http://\[::1\]*) ;;
  *)
    if [ -n "${OLLAMA_API_KEY:-}" ]; then
      ok "OLLAMA_API_KEY is set in the environment"
    elif [ "$(uname -s)" = Darwin ]; then
      if keyring_has OLLAMA_API_KEY; then
        ok "OLLAMA_API_KEY found in the macOS Keychain (service \"ferrite\")"
      elif [ -n "${FERRITE_GEMINI_API_KEY:-}" ] || keyring_has FERRITE_GEMINI_API_KEY; then
        ok "no OLLAMA_API_KEY, but a Gemini key is configured (the app falls back to Gemini)"
      else
        warn "no OLLAMA_API_KEY in the environment or Keychain; the agent will have no model. See scripts/local.env.example."
      fi
    else
      note "OLLAMA_API_KEY is not in the environment; the OS keyring is not checked on this platform."
    fi
    ;;
esac

# ── 3. Laya ───────────────────────────────────────────────────────────────
LAYA_STARTED_PID=''
APP_PID=''

# shellcheck disable=SC2329  # invoked via `trap cleanup EXIT`
cleanup() {
  local rc=$?
  trap - EXIT INT TERM
  if [ -n "$APP_PID" ] && pid_alive "$APP_PID"; then
    # Reached on INT/TERM while the app is still up: stop it first, so nothing
    # is left talking to a Laya server that is about to go away.
    kill "$APP_PID" 2>/dev/null || true
    local j=0
    while pid_alive "$APP_PID" && [ "$j" -lt 5 ]; do sleep 1; j=$((j + 1)); done
    pid_alive "$APP_PID" && kill -9 "$APP_PID" 2>/dev/null || true
  fi
  if [ -n "$LAYA_STARTED_PID" ]; then
    if pid_alive "$LAYA_STARTED_PID"; then
      info "stopping the Laya server (pid $LAYA_STARTED_PID)"
      kill "$LAYA_STARTED_PID" 2>/dev/null || true
      local i=0
      while pid_alive "$LAYA_STARTED_PID" && [ "$i" -lt 8 ]; do sleep 1; i=$((i + 1)); done
      pid_alive "$LAYA_STARTED_PID" && kill -9 "$LAYA_STARTED_PID" 2>/dev/null || true
    fi
    # Remove the pidfile only if it is still ours.
    local recorded=''
    [ -f "$FERRITE_LAYA_PIDFILE" ] && IFS= read -r recorded < "$FERRITE_LAYA_PIDFILE" || true
    [ "$recorded" = "$LAYA_STARTED_PID" ] && rm -f "$FERRITE_LAYA_PIDFILE"
  fi
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_health() { # wait_for_health URL PID — 0 healthy, 1 died, 2 timed out
  local url=$1 pid=$2 i=0
  while [ $i -lt "$WAIT" ]; do
    http_ok "$url/health" && return 0
    pid_alive "$pid" || return 1
    if [ $((i % 10)) -eq 9 ]; then info "still waiting for Laya (${i}s; the first start loads the model)..."; fi
    sleep 1
    i=$((i + 1))
  done
  http_ok "$url/health" && return 0
  return 2
}

setup_laya() {
  local host port url py ckpt pid rc
  if [ -n "${FERRITE_LAYA_URL:-}" ] && ! is_loopback_host "$(url_host "$FERRITE_LAYA_URL")"; then
    if [ "$DRY_RUN" = 1 ]; then info "would use the external Laya server at $FERRITE_LAYA_URL (not starting one)"; return 0; fi
    if http_ok "${FERRITE_LAYA_URL%/}/health"; then
      ok "using the external Laya server at $FERRITE_LAYA_URL"
    else
      warn "external Laya server $FERRITE_LAYA_URL is not answering /health right now; the app will run without it if it stays down"
    fi
    return 0
  fi

  laya_resolve_endpoint
  host=$LAYA_BIND_HOST
  port=$LAYA_BIND_PORT
  url=$LAYA_LOCAL_URL
  py=$FERRITE_LAYA_VENV/bin/python
  ckpt=$(laya_checkpoint_dir)

  if pid=$(pidfile_live_pid); then
    if [ "$DRY_RUN" = 1 ]; then info "would reuse the running Laya server (pid $pid) at $url"; return 0; fi
    info "Laya server already running (pid $pid); not starting a second one"
    if wait_for_health "$url" "$pid"; then
      ok "Laya healthy at $url"
      export FERRITE_LAYA_URL=$url
    else
      warn "the running Laya server (pid $pid) is not answering /health at $url; continuing without Laya"
      unset FERRITE_LAYA_URL
    fi
    return 0
  fi
  [ -f "$FERRITE_LAYA_PIDFILE" ] && { note "removing stale pidfile"; rm -f "$FERRITE_LAYA_PIDFILE"; }

  if [ "$DRY_RUN" != 1 ] && http_ok "$url/health"; then
    ok "a Laya server is already answering at $url (not started by these scripts); using it"
    export FERRITE_LAYA_URL=$url
    return 0
  fi

  # A loopback FERRITE_LAYA_URL from env.local only ever names OUR server. From
  # here on nothing is answering it, so unless a server comes up it must not
  # reach the app.
  unset FERRITE_LAYA_URL
  if [ ! -x "$py" ] || ! laya_checkpoint_complete "$ckpt"; then
    note "Laya is not set up (venv or checkpoint missing: $ckpt); running LLM-only. 'just setup' installs it."
    return 0
  fi

  if [ "$DRY_RUN" = 1 ]; then
    info "would start: $py $FERRITE_SCRIPTS_DIR/laya/serve.py   (bind $host:$port, log $FERRITE_LAYA_LOG)"
    info "would wait up to ${WAIT}s for $url/health, then export FERRITE_LAYA_URL=$url"
    return 0
  fi

  mkdir -p "$FERRITE_LAYA_DIR"
  info "starting the Laya server on $host:$port (log: $FERRITE_LAYA_LOG)"
  FERRITE_LAYA_HOST=$host FERRITE_LAYA_PORT=$port \
    nohup "$py" -u "$FERRITE_SCRIPTS_DIR/laya/serve.py" >>"$FERRITE_LAYA_LOG" 2>&1 </dev/null &
  LAYA_STARTED_PID=$!
  echo "$LAYA_STARTED_PID" > "$FERRITE_LAYA_PIDFILE"

  rc=0
  wait_for_health "$url" "$LAYA_STARTED_PID" || rc=$?
  case $rc in
    0)
      ok "Laya healthy at $url"
      export FERRITE_LAYA_URL=$url
      ;;
    1)
      warn "the Laya server exited during start-up; continuing WITHOUT it. Last log lines:"
      tail -n 15 "$FERRITE_LAYA_LOG" >&2 || true
      LAYA_STARTED_PID=''
      rm -f "$FERRITE_LAYA_PIDFILE"
      ;;
    *)
      warn "Laya did not become healthy within ${WAIT}s; continuing WITHOUT it (raise --wait; log: $FERRITE_LAYA_LOG)"
      ;;
  esac
}

if [ "$USE_LAYA" = 1 ]; then
  step "Laya"
  setup_laya
else
  # An inherited URL from env.local must not leak through --no-laya.
  unset FERRITE_LAYA_URL
  step "Laya"; info "disabled (--no-laya): the app runs LLM-only"
fi

# ── 4. the browser ────────────────────────────────────────────────────────
if [ "$SERVO" = auto ]; then
  SERVO=0
  if [ -f "$FERRITE_BUILD_MODE_FILE" ] && [ "$(cat "$FERRITE_BUILD_MODE_FILE" 2>/dev/null)" = servo ]; then SERVO=1; fi
fi
CARGO_CMD=(cargo run --release -p ferrite-shell)
if [ "$SERVO" = 1 ]; then
  CARGO_CMD+=(--features ferrite-servo/servo)
else
  warn "Servo-free build: no real web rendering. For real pages: just setup-servo, then re-run."
fi
CARGO_CMD+=(-- "${APP_ARGS[@]}")

step "Ferrite"
info "FERRITE_LAYA_URL=${FERRITE_LAYA_URL:-<unset: LLM-only>}"
info "$(quote_cmd "${CARGO_CMD[@]}")"
if [ "$DRY_RUN" = 1 ]; then
  note "dry run: not starting anything"
  exit 0
fi

cd "$FERRITE_REPO_ROOT"
# The app runs in the background and we `wait` on it, so a TERM/INT sent to this
# script runs the trap at once (a foreground child would defer it until the app
# exits). fd 3 keeps the terminal as the app's stdin: an async command otherwise
# gets /dev/null.
exec 3<&0
"${CARGO_CMD[@]}" <&3 &
APP_PID=$!
rc=0
wait "$APP_PID" || rc=$?
APP_PID=''
exit "$rc"
