#!/usr/bin/env bash
# scripts/tests/test_run_local.sh — exercises scripts/run-local.sh against a
# fake `cargo` (PATH shim) and a fake Laya venv python that serves /health.
# Proves the env-file whitelist/precedence, the model-tag message, pidfile
# handling, "already running", stale-pidfile recovery, start-up failure
# fallback, --no-laya, servo auto-detection, and trap cleanup on TERM.
# No network, no torch, no real cargo build; touches only a temp dir.
#   bash scripts/tests/test_run_local.sh
# shellcheck disable=SC2015  # `A && ok_ || fail_`: ok_ cannot fail, so this is a safe if/else here
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUN=$HERE/../run-local.sh
command -v python3 >/dev/null || { echo "SKIP: python3 needed for the fake server"; exit 0; }

TMP=$(mktemp -d)
BGPIDS=''
# shellcheck disable=SC2329  # invoked via trap
cleanup_all() {
  local p
  for p in $BGPIDS; do kill "$p" 2>/dev/null || true; done
  # any fake server a failed assertion left behind
  if [ -f "$TMP/home/laya/serve.pid" ]; then kill "$(cat "$TMP/home/laya/serve.pid")" 2>/dev/null || true; fi
  rm -rf "$TMP"
}
trap cleanup_all EXIT

PASSES=0
FAILS=0
ok_() { PASSES=$((PASSES + 1)); }
fail_() { FAILS=$((FAILS + 1)); printf 'FAIL: %s\n' "$*"; }
assert() { # assert DESC CMD...
  local d=$1; shift
  if "$@" >/dev/null 2>&1; then ok_; else fail_ "$d"; fi
}
contains() { grep -qF -- "$2" "$1"; }

mkdir -p "$TMP/bin" "$TMP/home/laya/venv/bin" "$TMP/home/laya/checkpoints/laya-browser/v10s"
touch "$TMP/home/laya/checkpoints/laya-browser/v10s/rl_agent_config.json" \
      "$TMP/home/laya/checkpoints/laya-browser/v10s/model.safetensors"

PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')

# fake cargo: records argv and selected env, optionally sleeps, exits FAKE_CARGO_RC
cat > "$TMP/bin/cargo" <<EOF
#!/usr/bin/env bash
{
  echo "ARGS: \$*"
  echo "LAYA_URL=\${FERRITE_LAYA_URL-<unset>}"
  echo "DEFENSE=\${FERRITE_DEFENSE-<unset>}"
  echo "EVIL=\${EVIL-<unset>}"
  echo "PATH_OK=\$(case \$PATH in /nonexistent*) echo no ;; *) echo yes ;; esac)"
  echo "HEALTH=\$(curl -fsS --max-time 2 "\${FERRITE_LAYA_URL:-http://127.0.0.1:1}/health" 2>/dev/null || echo none)"
} >> "$TMP/cargo.log"
[ -z "\${FAKE_CARGO_SLEEP:-}" ] || sleep "\$FAKE_CARGO_SLEEP"
exit "\${FAKE_CARGO_RC:-0}"
EOF
chmod +x "$TMP/bin/cargo"

# fake "venv python": ignores the script it is given and serves /health
cat > "$TMP/home/laya/venv/bin/python" <<'EOF'
#!/usr/bin/env python3
import os, sys, http.server
if os.environ.get("FAKE_LAYA_MODE") == "die":
    print("fake laya: simulated start-up failure", file=sys.stderr)
    sys.exit(1)
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.path == "/health"
        body = b'{"status":"ok"}' if ok else b"{}"
        self.send_response(200 if ok else 404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass
http.server.HTTPServer((os.environ.get("FERRITE_LAYA_HOST", "127.0.0.1"), int(os.environ["FERRITE_LAYA_PORT"])), H).serve_forever()
EOF
chmod +x "$TMP/home/laya/venv/bin/python"

cat > "$TMP/home/env.local" <<EOF
FERRITE_MODEL_SMALL=small-tag
FERRITE_MODEL_MAIN=main-tag
FERRITE_DEFENSE=on
FERRITE_LAYA_PORT=$PORT
FERRITE_OLLAMA_BASE_URL=http://localhost:11434
EVIL=1
PATH=/nonexistent
EOF

export FERRITE_HOME=$TMP/home
export CARGO_TARGET_DIR=$TMP/tgt
export PATH=$TMP/bin:$PATH
export NO_COLOR=1
unset FERRITE_LAYA_URL FERRITE_DEFENSE FERRITE_MODEL_SMALL FERRITE_MODEL_MAIN FERRITE_LAYA_PORT EVIL FAKE_LAYA_MODE FAKE_CARGO_SLEEP FAKE_CARGO_RC 2>/dev/null || true

URL=http://127.0.0.1:$PORT
healthy() { curl -fsS --max-time 2 "$URL/health" >/dev/null 2>&1; }
last_args() { grep '^ARGS:' "$TMP/cargo.log" | tail -1; }
reset_log() { : > "$TMP/cargo.log"; }
wait_until() { # wait_until SECONDS CMD...
  local n=$1 i=0; shift
  while [ "$i" -lt "$n" ]; do "$@" >/dev/null 2>&1 && return 0; sleep 1; i=$((i + 1)); done
  return 1
}

# ── 1. missing model tags: names exactly what is missing, exit 2, no cargo ──
reset_log
mv "$TMP/home/env.local" "$TMP/home/env.local.bak"
printf 'FERRITE_MODEL_MAIN=only-main\n' > "$TMP/home/env.local"
rc=0; "$RUN" --no-laya > "$TMP/o1" 2>&1 || rc=$?
[ "$rc" -eq 2 ] && ok_ || fail_ "missing tag: exit 2 (got $rc)"
contains "$TMP/o1" "not configured: FERRITE_MODEL_SMALL" && ok_ || fail_ "missing tag: names FERRITE_MODEL_SMALL"
! contains "$TMP/o1" "FERRITE_MODEL_MAIN=<" || true
[ ! -s "$TMP/cargo.log" ] && ok_ || fail_ "missing tag: cargo must not run"
mv "$TMP/home/env.local.bak" "$TMP/home/env.local"

# ── 2. happy path: whitelist, URL export, args, cleanup on exit ─────────────
reset_log
rc=0; "$RUN" --wait 20 > "$TMP/o2" 2>&1 || rc=$?
[ "$rc" -eq 0 ] && ok_ || fail_ "happy path: exit 0 (got $rc)"; [ "$rc" -eq 0 ] || sed 's/^/    /' "$TMP/o2"
[ "$(last_args)" = "ARGS: run --release -p ferrite-shell -- ui" ] && ok_ || fail_ "happy path: cargo args: $(last_args)"
contains "$TMP/cargo.log" "LAYA_URL=$URL" && ok_ || fail_ "happy path: FERRITE_LAYA_URL exported"
contains "$TMP/cargo.log" 'HEALTH={"status":"ok"}' && ok_ || fail_ "happy path: server healthy while the app ran"
contains "$TMP/cargo.log" "DEFENSE=on" && ok_ || fail_ "happy path: file value loaded"
contains "$TMP/cargo.log" "EVIL=<unset>" && ok_ || fail_ "whitelist: EVIL must not be exported"
contains "$TMP/cargo.log" "PATH_OK=yes" && ok_ || fail_ "whitelist: PATH must not be overridden"
contains "$TMP/o2" "PATH is not an allowed key" && ok_ || fail_ "whitelist: rejection reported"
! healthy && ok_ || fail_ "cleanup: server must be stopped after the app exits"
[ ! -f "$TMP/home/laya/serve.pid" ] && ok_ || fail_ "cleanup: pidfile must be removed"

# ── 3. real environment beats the file ─────────────────────────────────────
reset_log
FERRITE_DEFENSE=off "$RUN" --no-laya > "$TMP/o3" 2>&1 || true
contains "$TMP/cargo.log" "DEFENSE=off" && ok_ || fail_ "precedence: env must win over env.local"

# ── 4. --no-laya: nothing started, URL from the file is not leaked ──────────
reset_log
printf 'FERRITE_LAYA_URL=http://127.0.0.1:%s\n' "$PORT" >> "$TMP/home/env.local"
"$RUN" --no-laya > "$TMP/o4" 2>&1 || true
contains "$TMP/cargo.log" "LAYA_URL=<unset>" && ok_ || fail_ "--no-laya: FERRITE_LAYA_URL must be unset"
! healthy && ok_ || fail_ "--no-laya: no server may be started"
# (the loopback URL in env.local now just names the port for the managed server)

# ── 5. app exit status is propagated ───────────────────────────────────────
reset_log
rc=0; FAKE_CARGO_RC=7 "$RUN" --no-laya > "$TMP/o5" 2>&1 || rc=$?
[ "$rc" -eq 7 ] && ok_ || fail_ "exit status: expected 7, got $rc"

# ── 6. already running: second run reuses, does not restart, does not kill ──
reset_log
FAKE_CARGO_SLEEP=25 "$RUN" --wait 20 > "$TMP/o6a" 2>&1 &
FIRST=$!; BGPIDS="$BGPIDS $FIRST"
wait_until 20 healthy && ok_ || fail_ "already-running: first run never became healthy"
PID1=$(cat "$TMP/home/laya/serve.pid" 2>/dev/null || echo none)
rc=0; "$RUN" --wait 20 > "$TMP/o6b" 2>&1 || rc=$?
[ "$rc" -eq 0 ] && ok_ || fail_ "already-running: second run exit 0 (got $rc)"
contains "$TMP/o6b" "already running (pid $PID1)" && ok_ || fail_ "already-running: message names pid $PID1"
[ "$(cat "$TMP/home/laya/serve.pid" 2>/dev/null || echo none)" = "$PID1" ] && ok_ || fail_ "already-running: pidfile unchanged"
healthy && ok_ || fail_ "already-running: second run must not stop the first run's server"

# ── 7. TERM to the owning run-local: trap stops server, removes pidfile ─────
kill -TERM "$FIRST" 2>/dev/null || true
wait_until 20 bash -c '! kill -0 '"$PID1"' 2>/dev/null' && ok_ || fail_ "TERM: server pid $PID1 still alive"
! healthy && ok_ || fail_ "TERM: server still answering"
rc=0; wait "$FIRST" 2>/dev/null || rc=$?
[ "$rc" -eq 143 ] && ok_ || fail_ "TERM: run-local should exit 143 (got $rc)"
[ ! -f "$TMP/home/laya/serve.pid" ] && ok_ || fail_ "TERM: pidfile not removed"

# ── 8. stale pidfile is replaced ────────────────────────────────────────────
reset_log
echo 999999 > "$TMP/home/laya/serve.pid"
"$RUN" --wait 20 > "$TMP/o8" 2>&1 || true
contains "$TMP/o8" "removing stale pidfile" && ok_ || fail_ "stale pidfile: not reported"
contains "$TMP/cargo.log" "LAYA_URL=$URL" && ok_ || fail_ "stale pidfile: server did not start afterwards"

# ── 9. start-up failure: continue without Laya ─────────────────────────────
reset_log
rc=0; FAKE_LAYA_MODE=die "$RUN" --wait 10 > "$TMP/o9" 2>&1 || rc=$?
[ "$rc" -eq 0 ] && ok_ || fail_ "start-up failure: app should still run (exit $rc)"
contains "$TMP/o9" "exited during start-up" && ok_ || fail_ "start-up failure: not reported"
contains "$TMP/cargo.log" "LAYA_URL=<unset>" && ok_ || fail_ "start-up failure: URL must not be exported"
[ ! -f "$TMP/home/laya/serve.pid" ] && ok_ || fail_ "start-up failure: pidfile must be removed"

# ── 10. servo detection + override + passthrough args ──────────────────────
reset_log
echo servo > "$TMP/home/build.mode"
"$RUN" --no-laya > /dev/null 2>&1 || true
[ "$(last_args)" = "ARGS: run --release -p ferrite-shell --features ferrite-servo/servo -- ui" ] && ok_ || fail_ "servo auto-detect: $(last_args)"
"$RUN" --no-laya --no-servo -- smoke > /dev/null 2>&1 || true
[ "$(last_args)" = "ARGS: run --release -p ferrite-shell -- smoke" ] && ok_ || fail_ "--no-servo + passthrough: $(last_args)"
echo plain > "$TMP/home/build.mode"
"$RUN" --no-laya --servo > /dev/null 2>&1 || true
[ "$(last_args)" = "ARGS: run --release -p ferrite-shell --features ferrite-servo/servo -- ui" ] && ok_ || fail_ "--servo: $(last_args)"

# ── 11. --dry-run starts nothing ───────────────────────────────────────────
reset_log
"$RUN" --dry-run > "$TMP/o11" 2>&1 || true
[ ! -s "$TMP/cargo.log" ] && ok_ || fail_ "--dry-run: cargo must not run"
! healthy && ok_ || fail_ "--dry-run: no server may start"
contains "$TMP/o11" "would start:" && ok_ || fail_ "--dry-run: should describe the server start"

# ── 12. --verify / --laya-only resolve the endpoint (dry run) ──────────────
"$RUN" --verify --dry-run > "$TMP/o12" 2>&1 || true
contains "$TMP/o12" "verify.py --url $URL" && ok_ || fail_ "--verify --dry-run: $(cat "$TMP/o12")"
"$RUN" --laya-only --dry-run > "$TMP/o13" 2>&1 || true
contains "$TMP/o13" "bind 127.0.0.1:$PORT" || contains "$TMP/o13" "(bind 127.0.0.1:$PORT)" || contains "$TMP/o13" "127.0.0.1:$PORT" \
  && ok_ || fail_ "--laya-only --dry-run: $(cat "$TMP/o13")"

printf 'run-local tests: %d passed, %d failed\n' "$PASSES" "$FAILS"
[ "$FAILS" -eq 0 ]
