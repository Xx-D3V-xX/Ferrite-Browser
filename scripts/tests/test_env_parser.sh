#!/usr/bin/env bash
# scripts/tests/test_env_parser.sh — unit tests for the env.local parser in
# scripts/lib/common.sh. No dependencies beyond bash; touches only a temp dir.
#   bash scripts/tests/test_env_parser.sh
# shellcheck disable=SC2015,SC2016,SC2153  # A && B || C is a test idiom here; single quotes keep '$' literal on purpose
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=../lib/common.sh
. "$HERE/../lib/common.sh"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAILS=0
PASSES=0

check() { # check NAME EXPECTED ACTUAL
  if [ "$2" = "$3" ]; then
    PASSES=$((PASSES + 1))
  else
    FAILS=$((FAILS + 1))
    printf 'FAIL: %s\n  expected: %s\n  actual:   %s\n' "$1" "$2" "$3"
  fi
}

entries() { ferrite_env_entries "$1" 2>/dev/null; }

# ── shapes ────────────────────────────────────────────────────────────────
f=$TMP/basic.env
cat > "$f" <<'EOF'
# a comment
FERRITE_MODEL_SMALL=small-tag:1b
   FERRITE_MODEL_MAIN = main-tag:31b
export FERRITE_DEFENSE=on
FERRITE_LAYA_URL="http://127.0.0.1:8765"
OLLAMA_API_KEY='sk with spaces # not a comment'
FERRITE_X=value   # trailing comment
FERRITE_HASH=http://host/#frag
GEMINI_THING=

RUST_LOG=info,ferrite=debug
EOF
expected=$(printf '%s\n' \
  'FERRITE_MODEL_SMALL=small-tag:1b' \
  'FERRITE_MODEL_MAIN=main-tag:31b' \
  'FERRITE_DEFENSE=on' \
  'FERRITE_LAYA_URL=http://127.0.0.1:8765' \
  'OLLAMA_API_KEY=sk with spaces # not a comment' \
  'FERRITE_X=value' \
  'FERRITE_HASH=http://host/#frag' \
  'GEMINI_THING=' \
  'RUST_LOG=info,ferrite=debug')
check "accepted shapes" "$expected" "$(entries "$f")"

# ── the whitelist ─────────────────────────────────────────────────────────
f=$TMP/reject.env
cat > "$f" <<'EOF'
PATH=/evil
LD_PRELOAD=/evil.so
DYLD_INSERT_LIBRARIES=/evil.dylib
CARGO_HOME=/evil
HOME=/evil
BASH_ENV=/evil
LAYA_API_KEY=nope
NOTFERRITE=1
FERRITE_OK=1
EOF
check "whitelist rejects everything but the allowed families" "FERRITE_OK=1" "$(entries "$f")"
err=$(ferrite_env_entries "$f" 2>&1 >/dev/null)
case $err in
  *PATH*'not an allowed key'*) PASSES=$((PASSES + 1)) ;;
  *) FAILS=$((FAILS + 1)); echo "FAIL: rejection is reported on stderr: $err" ;;
esac

# ── nothing is ever evaluated ─────────────────────────────────────────────
f=$TMP/inject.env
marker=$TMP/pwned
cat > "$f" <<EOF
FERRITE_A=\$(touch $marker)
FERRITE_B=\`touch $marker\`
FERRITE_C=x; touch $marker
FERRITE_D="\$HOME"
FERRITE_E=\${HOME}
touch $marker
\$(touch $marker)=1
EOF
out=$(entries "$f")
[ ! -e "$marker" ] && PASSES=$((PASSES + 1)) || { FAILS=$((FAILS + 1)); echo "FAIL: env file content was executed"; }
check "expansions stay literal" \
  "$(printf '%s\n' 'FERRITE_A=$(touch '"$marker"')' 'FERRITE_B=`touch '"$marker"'`' \
       'FERRITE_C=x; touch '"$marker" 'FERRITE_D=$HOME' 'FERRITE_E=${HOME}')" \
  "$out"

# ── malformed lines are dropped, not fatal ────────────────────────────────
f=$TMP/bad.env
cat > "$f" <<'EOF'
no equals sign here
=novalue
1BAD=x
FERRITE-DASH=x
FERRITE_Q="unterminated
FERRITE_GOOD=yes
EOF
check "malformed lines dropped, good line kept" "FERRITE_GOOD=yes" "$(entries "$f")"

# ── CRLF and a final line with no newline ─────────────────────────────────
printf 'FERRITE_CR=a\r\nFERRITE_LAST=b' > "$TMP/crlf.env"
check "CRLF + no trailing newline" "$(printf 'FERRITE_CR=a\nFERRITE_LAST=b')" "$(entries "$TMP/crlf.env")"

# ── missing file is not an error ──────────────────────────────────────────
check "missing file" "" "$(entries "$TMP/does-not-exist")"

# ── precedence: real environment beats the file; file fills the gaps ──────
f=$TMP/prec.env
printf 'FERRITE_T_SET=from-file\nFERRITE_T_UNSET=from-file\nFERRITE_T_EMPTY=from-file\n' > "$f"
(
  export FERRITE_T_SET=from-env
  export FERRITE_T_EMPTY=
  unset FERRITE_T_UNSET
  ferrite_load_env_file "$f" 2>/dev/null
  printf '%s|%s|%s' "$FERRITE_T_SET" "$FERRITE_T_UNSET" "$FERRITE_T_EMPTY"
) > "$TMP/prec.out"
check "env wins, file fills unset/empty" "from-env|from-file|from-file" "$(cat "$TMP/prec.out")"

# ── helpers ───────────────────────────────────────────────────────────────
check "url_host"  "127.0.0.1" "$(url_host http://127.0.0.1:8765/x)"
check "url_port"  "8765"      "$(url_port http://127.0.0.1:8765/x)"
check "url_port none" ""      "$(url_port http://example.com/x)"
check "url_host v6" "::1"     "$(url_host 'http://[::1]:9000')"
check "url_port v6" "9000"    "$(url_port 'http://[::1]:9000')"
is_loopback_host 127.0.0.1 && is_loopback_host localhost && ! is_loopback_host 0.0.0.0 \
  && ! is_loopback_host example.com && PASSES=$((PASSES + 1)) \
  || { FAILS=$((FAILS + 1)); echo "FAIL: is_loopback_host"; }
version_ge 3.10.4 3.10 && version_ge 3.10 3.10 && ! version_ge 3.9.18 3.10 && version_ge 3.12 3.10 \
  && PASSES=$((PASSES + 1)) || { FAILS=$((FAILS + 1)); echo "FAIL: version_ge"; }

# checkpoint name is constrained to a plain directory name
(
  export FERRITE_LAYA_CHECKPOINT='../../etc'
  check "checkpoint name traversal falls back" "$FERRITE_LAYA_DEFAULT_CHECKPOINT" "$(laya_checkpoint_name)"
  export FERRITE_LAYA_CHECKPOINT=v10
  check "checkpoint name accepted" "v10" "$(laya_checkpoint_name)"
  printf '%s\n' "$PASSES $FAILS" > "$TMP/sub.count"
)
read -r sp sf < "$TMP/sub.count"
PASSES=$sp; FAILS=$sf

printf 'env parser tests: %d passed, %d failed\n' "$PASSES" "$FAILS"
[ "$FAILS" -eq 0 ]
