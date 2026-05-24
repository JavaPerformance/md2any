#!/usr/bin/env bash
# Stress test for the remote-image cache + retry path in src/image.rs.
#
# Spins up tests/integration/cache_server.py on a local port, then runs
# md2any against synthetic markdown that references its endpoints and
# asserts:
#   - cold fetch reaches the server
#   - warm fetch hits the cache (no server call)
#   - URL normalisation collapses equivalent forms to one cache slot
#   - --no-remote-image-cache always refetches
#   - retry loop survives flaky 503s and caches the eventual success
#   - 30 MB payload is rejected by the cap and not cached
#   - 200-OK-but-garbage body is rejected and not cached
#   - 200-OK-empty body is rejected and not cached
#   - 5 concurrent renders of the same URL produce 1 cache file, 0 .tmp leaks
#
# Usage:
#   tests/integration/cache_stress.sh
#
# Honours MD2ANY_BIN to point at a non-default binary, and PYTHON3 to pick
# a specific Python interpreter. Exits non-zero on any assertion failure
# so it can drive `cargo test -- --ignored`.

set -u

BIN="${MD2ANY_BIN:-./target/release/md2any}"
PY="${PYTHON3:-python3}"
PORT="${MD2ANY_TEST_PORT:-18421}"
BASE="http://127.0.0.1:${PORT}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_SCRIPT="${SCRIPT_DIR}/cache_server.py"

WORKDIR="$(mktemp -d -t md2any-cache-XXXXXX)"
CACHE="${WORKDIR}/cache"
PASS=0
FAIL=0
SERVER_PID=""

# --- harness -----------------------------------------------------------------
cleanup() {
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

note() { printf "\n=== %s ===\n" "$*"; }
pass() { printf "  PASS  %s\n" "$*"; PASS=$((PASS + 1)); }
fail() { printf "  FAIL  %s\n" "$*"; FAIL=$((FAIL + 1)); }

# --- prerequisites -----------------------------------------------------------
if ! command -v "$PY" >/dev/null 2>&1; then
  printf "skip: %s not on PATH\n" "$PY"
  exit 77   # autoconf convention for skipped tests
fi
if [ ! -x "$BIN" ]; then
  printf "skip: %s missing (build first: cargo build --release)\n" "$BIN"
  exit 77
fi
if ! command -v curl >/dev/null 2>&1; then
  printf "skip: curl not on PATH\n"
  exit 77
fi

# --- server lifecycle --------------------------------------------------------
"$PY" "$SERVER_SCRIPT" "$PORT" >"${WORKDIR}/server.log" 2>&1 &
SERVER_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -sf "${BASE}/count" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done
if ! curl -sf "${BASE}/count" >/dev/null 2>&1; then
  printf "skip: test server did not start on %s\n" "$BASE"
  cat "${WORKDIR}/server.log" || true
  exit 77
fi

cleanup_cache() {
  rm -rf "$CACHE"
  mkdir -p "$CACHE"
}

mk_md() {
  # mk_md OUTFILE URL [URL ...]
  local out="$1"; shift
  {
    printf -- '---\ntitle: cache test\n---\n# Cache stress\n\n'
    for u in "$@"; do printf '![img](%s)\n\n' "$u"; done
  } >"$out"
}

curl -sf "${BASE}/reset" >/dev/null

# --- scenarios ---------------------------------------------------------------
note "1. cold fetch + cache write"
cleanup_cache
RC_BEFORE=$(curl -sf "${BASE}/count")
mk_md "${WORKDIR}/t1.md" "${BASE}/good.png"
"$BIN" "${WORKDIR}/t1.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t1.pdf" >/dev/null 2>&1 \
  && pass "first render succeeded" \
  || fail "first render failed"
RC_AFTER=$(curl -sf "${BASE}/count")
DELTA=$((RC_AFTER - RC_BEFORE - 1))
[ "$DELTA" -ge 1 ] && pass "server saw a fetch (delta=$DELTA)" || fail "no server fetch"
ls "$CACHE"/*.img >/dev/null 2>&1 && pass "cache file created" || fail "no cache file"

note "2. warm cache: no second fetch"
RC_BEFORE=$(curl -sf "${BASE}/count")
"$BIN" "${WORKDIR}/t1.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t1b.pdf" >/dev/null 2>&1 \
  && pass "second render succeeded" \
  || fail "second render failed"
RC_AFTER=$(curl -sf "${BASE}/count")
DELTA=$((RC_AFTER - RC_BEFORE - 1))
[ "$DELTA" -eq 0 ] \
  && pass "no extra server fetch (delta=$DELTA)" \
  || fail "server hit on warm cache (delta=$DELTA)"

note "3. URL normalisation hits same cache slot"
RC_BEFORE=$(curl -sf "${BASE}/count")
mk_md "${WORKDIR}/t3.md" "${BASE}/good.png/" "${BASE}/good.png#anchor" "${BASE}/good.png  "
"$BIN" "${WORKDIR}/t3.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t3.pdf" >/dev/null 2>&1 \
  && pass "normalised variants render" \
  || fail "normalised variants failed"
RC_AFTER=$(curl -sf "${BASE}/count")
DELTA=$((RC_AFTER - RC_BEFORE - 1))
[ "$DELTA" -eq 0 ] \
  && pass "all 3 URL variants hit cache (delta=$DELTA)" \
  || fail "URL variants caused extra fetches (delta=$DELTA)"
COUNT=$(ls "$CACHE"/*.img | wc -l)
[ "$COUNT" -eq 1 ] && pass "still only one cache file" || fail "cache exploded to $COUNT files"

note "4. --no-remote-image-cache always refetches"
RC_BEFORE=$(curl -sf "${BASE}/count")
"$BIN" "${WORKDIR}/t1.md" --no-remote-image-cache -o "${WORKDIR}/t4.pdf" >/dev/null 2>&1 \
  && pass "render with cache disabled" \
  || fail "render with --no-remote-image-cache failed"
RC_AFTER=$(curl -sf "${BASE}/count")
DELTA=$((RC_AFTER - RC_BEFORE - 1))
[ "$DELTA" -ge 1 ] \
  && pass "fetched fresh (delta=$DELTA)" \
  || fail "cache disable not honoured (delta=$DELTA)"

note "5. retry on 503 then 200 (flaky URL)"
cleanup_cache
curl -sf "${BASE}/reset" >/dev/null
mk_md "${WORKDIR}/t5.md" "${BASE}/flaky.png"
"$BIN" "${WORKDIR}/t5.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t5.pdf" >/dev/null 2>&1 \
  && pass "render succeeded after retries" \
  || fail "render failed despite retries"
ls "$CACHE"/*.img >/dev/null 2>&1 && pass "flaky URL cached after success" || fail "no cache after retry success"

note "6. oversize body: render uses placeholder, cap is reported"
cleanup_cache
mk_md "${WORKDIR}/t6.md" "${BASE}/huge.png"
OUT=$("$BIN" "${WORKDIR}/t6.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t6.pdf" 2>&1)
RC=$?
[ "$RC" -eq 0 ] && pass "render succeeded with placeholder" || fail "render bailed (exit=$RC)"
[ -s "${WORKDIR}/t6.pdf" ] && pass "output produced" || fail "no output produced"
if echo "$OUT" | grep -qi "20 MB cap\|exceeds.*cap"; then
  pass "stderr warning mentions cap"
else
  fail "no cap mention in stderr"
  printf '%s\n' "$OUT" | sed 's/^/    /'
fi
ls "$CACHE"/*.img 2>/dev/null \
  && fail "huge body was cached!" \
  || pass "huge body not cached"

note "7. garbage body (200 OK + HTML): placeholder substituted, not cached"
cleanup_cache
mk_md "${WORKDIR}/t7.md" "${BASE}/garbage.png"
OUT=$("$BIN" "${WORKDIR}/t7.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t7.pdf" 2>&1)
[ $? -eq 0 ] && pass "render succeeded with placeholder" || fail "render bailed on garbage"
echo "$OUT" | grep -qi "warning: image failed" && pass "stderr warning printed" || fail "no warning"
ls "$CACHE"/*.img 2>/dev/null \
  && fail "garbage was cached" \
  || pass "garbage not cached (sniff-before-cache works)"

note "8. empty 200 OK: placeholder substituted, not cached"
cleanup_cache
mk_md "${WORKDIR}/t8.md" "${BASE}/empty.png"
OUT=$("$BIN" "${WORKDIR}/t8.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t8.pdf" 2>&1)
[ $? -eq 0 ] && pass "render succeeded with placeholder" || fail "render bailed on empty"
echo "$OUT" | grep -qi "warning: image failed" && pass "stderr warning printed" || fail "no warning"
ls "$CACHE"/*.img 2>/dev/null \
  && fail "empty body was cached" \
  || pass "empty body not cached"

note "9. concurrent renders of same URL — no tmp clobber"
cleanup_cache
mk_md "${WORKDIR}/t9.md" "${BASE}/good.png"
for i in 1 2 3 4 5; do
  "$BIN" "${WORKDIR}/t9.md" --remote-image-cache "$CACHE" -o "${WORKDIR}/t9.${i}.pdf" >/dev/null 2>&1 &
done
wait
N_IMG=$(ls "$CACHE"/*.img 2>/dev/null | wc -l)
N_TMP=$(ls "$CACHE"/*.tmp.* 2>/dev/null | wc -l)
[ "$N_IMG" -eq 1 ] && pass "exactly one cache file ($N_IMG)" || fail "expected 1 cache file, got $N_IMG"
[ "$N_TMP" -eq 0 ] && pass "no leaked .tmp files" || fail "leaked $N_TMP .tmp files"

# ----------------------------------------------------------------------------
echo
echo "================================="
echo "passed: $PASS    failed: $FAIL"
echo "================================="
exit "$FAIL"
