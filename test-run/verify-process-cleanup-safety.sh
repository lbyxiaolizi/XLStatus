#!/usr/bin/env bash
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
source "$ROOT/test-run/lib/test-processes.sh"

if rg -n 'pkill[[:space:]].*-f|pgrep[[:space:]].*-f|kill_port\(' \
  "$ROOT/test-run" "$ROOT/deploy" -g '*.sh' \
  -g '!verify-process-cleanup-safety.sh'; then
  echo "FAIL: broad process-name cleanup remains" >&2
  exit 1
fi

for script in "$ROOT"/test-run/verify-m{0,1-pg,2-reconnect,2-revoke,3-metrics,3-tsdb,3-ws,4-alerts,5-files,5-scheduler,5-task,5-terminal,6-ddns,6-mcp,6-nat}.sh; do
  grep -Fq 'test-run/lib/test-processes.sh' "$script" || {
    echo "FAIL: $script does not use the scoped process helper" >&2
    exit 1
  }
done

grep -Fq 'process_group_id" == "$pid' "$ROOT/test-run/lib/test-processes.sh"
grep -Fq '_XLSTATUS_TEST_PROCESS_SUPERVISOR"*' "$ROOT/test-run/lib/test-processes.sh"
grep -Fq '"$command_line" == *"$token"*' "$ROOT/test-run/lib/test-processes.sh"
grep -Fq -- '-p "127.0.0.1:${PG_PORT}:5432"' "$ROOT/test-run/verify-m1-pg.sh"
grep -Fq -- '--label "com.xlstatus.test-token=$PG_TOKEN"' "$ROOT/test-run/verify-m1-pg.sh"
if grep -Fq 'DROP SCHEMA public CASCADE' "$ROOT/test-run/verify-m1-pg.sh"; then
  echo "FAIL: PostgreSQL verification must not reset a reused schema" >&2
  exit 1
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/xlstatus-process-safety.XXXXXX")"
REGISTRY="$TMP/processes.tsv"
CONTROL_PID=""
cleanup() {
  test_process_cleanup "$REGISTRY"
  if [[ -n "$CONTROL_PID" ]]; then
    kill "$CONTROL_PID" 2>/dev/null || true
    wait "$CONTROL_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
: > "$REGISTRY"

# The control process deliberately has the same executable name as the owned
# descendant. Scoped cleanup must terminate only the registered process group.
sleep 30 &
CONTROL_PID=$!
test_process_start "$REGISTRY" owned "$TMP/owned.log" \
  bash -c 'sleep 30 & echo "$!" > "$1"; wait' _ "$TMP/descendant.pid"
OWNED_PID="$TEST_PROCESS_PID"

for _ in $(seq 1 50); do
  [[ -s "$TMP/descendant.pid" ]] && break
  sleep 0.02
done
[[ -s "$TMP/descendant.pid" ]] || {
  echo "FAIL: owned descendant did not start" >&2
  exit 1
}
DESCENDANT_PID="$(cat "$TMP/descendant.pid")"

test_process_cleanup "$REGISTRY"
for _ in $(seq 1 30); do
  if ! kill -0 "$OWNED_PID" 2>/dev/null && ! kill -0 "$DESCENDANT_PID" 2>/dev/null; then
    break
  fi
  sleep 0.05
done

if kill -0 "$OWNED_PID" 2>/dev/null || kill -0 "$DESCENDANT_PID" 2>/dev/null; then
  echo "FAIL: scoped cleanup left an owned process alive" >&2
  exit 1
fi
if ! kill -0 "$CONTROL_PID" 2>/dev/null; then
  echo "FAIL: scoped cleanup terminated an unrelated process" >&2
  exit 1
fi

# A short-lived direct child must not allow a TERM-ignoring descendant in the
# owned process group to outlive the supervisor.
: > "$REGISTRY"
test_process_start "$REGISTRY" stubborn-descendant "$TMP/stubborn.log" \
  bash -c 'trap "" TERM; sleep 30 & echo "$!" > "$1"' _ "$TMP/stubborn.pid"
for _ in $(seq 1 50); do
  [[ -s "$TMP/stubborn.pid" ]] && break
  sleep 0.02
done
[[ -s "$TMP/stubborn.pid" ]] || {
  echo "FAIL: stubborn descendant did not start" >&2
  exit 1
}
STUBBORN_PID="$(cat "$TMP/stubborn.pid")"
for _ in $(seq 1 80); do
  if ! kill -0 "$STUBBORN_PID" 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if kill -0 "$STUBBORN_PID" 2>/dev/null; then
  echo "FAIL: TERM-ignoring descendant escaped supervisor cleanup" >&2
  exit 1
fi

echo "Process cleanup static and isolation checks passed"
