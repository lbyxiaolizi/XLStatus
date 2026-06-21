#!/usr/bin/env bash
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18112}"
GRPC_PORT="${GRPC_PORT:-15082}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m5-files}"
DB="$LOG_DIR/x.db"

mkdir -p "$LOG_DIR"
rm -f "$DB" "$LOG_DIR/agent.yaml"

echo ">>> build"
(cd "$ROOT" && cargo build -p xlstatus-server -p xlstatus-agent 2>&1 | tail -3)

cat > "$LOG_DIR/cfg.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"

[database]
url = "sqlite://$DB?mode=rwc"

[security]
session_secret = "test-secret-key-for-development-only-2026"
session_ttl_hours = 24
TOML

pkill -9 -f xlstatus-server 2>/dev/null || true
pkill -9 -f xlstatus-agent 2>/dev/null || true
sleep 1

echo ">>> start server"
nohup env CONFIG_FILE="$LOG_DIR/cfg.toml" \
  XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
  "$ROOT/target/debug/xlstatus-server" \
  > "$LOG_DIR/server.log" 2>&1 < /dev/null &
disown $!
sleep 5

JAR="$LOG_DIR/jar"
rm -f "$JAR"
curl -s -c "$JAR" -X POST -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"admin-pw"}' > /dev/null
CSRF=$(awk '$6=="xlstatus_csrf" {print $7}' "$JAR")
TOKR=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/enrollment-tokens" \
  -d '{"expires_in_hours":24}')
ETOK=$(echo "$TOKR" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('token',''))")
[ -n "$ETOK" ] || { echo "FAIL: empty enrollment token"; exit 1; }

echo ">>> enroll + run agent"
"$ROOT/target/debug/xlstatus-agent" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$ETOK" \
  --name "verify-m5-files-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")
nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!

for i in $(seq 1 20); do
  SEEN=$(sqlite3 "$DB" "SELECT last_seen_at IS NOT NULL FROM agents WHERE id = '$AGENT_ID';" 2>/dev/null || echo 0)
  [[ "$SEEN" == "1" ]] && break
  sleep 1
done
[[ "$SEEN" == "1" ]] || { echo "FAIL: agent not online"; exit 1; }

TMP_DIR="$(mktemp -d "$LOG_DIR/files.XXXXXX")"
FILE="$TMP_DIR/hello.txt"
echo "hello-from-m5-files" > "$FILE"

LIST=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/files" \
  -d '{"path":"/tmp"}')

echo "$LIST" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('success'); assert isinstance(d.get('data',{}).get('entries',[]), list)"
LIST_COUNT=$(echo "$LIST" | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d.get('data',{}).get('entries',[])))")
echo "  listed $LIST_COUNT entries in /tmp"

WRITE=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/files/write" \
  -d '{"path":"/tmp/xlstatus-m5-files.txt","content":"hello-from-m5-files","encoding":"utf8","create_dirs":true}')
echo "  $WRITE"
echo "$WRITE" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('success')"

READ=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/files/read" \
  -d '{"path":"/tmp/xlstatus-m5-files.txt","encoding":"utf8"}')
echo "  $READ"
echo "$READ" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('success'); assert 'hello-from-m5-files' in d.get('data',{}).get('content','')"

DELETE=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/files/delete" \
  -d '{"path":"/tmp/xlstatus-m5-files.txt","recursive":false}')
echo "  $DELETE"
echo "$DELETE" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('success')"

echo ">>> apply disable_command_execute policy"
CFG=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/config" \
  -d '{"config":{"disable_command_execute":true}}')
echo "  $CFG"
echo "$CFG" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('success')"
sleep 2

echo ">>> assert file write is rejected by disabled policy"
DENY_WRITE=$(curl -s -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/files/write" \
  -d '{"path":"/tmp/xlstatus-m5-denied.txt","content":"should-not-write","encoding":"utf8","create_dirs":true}')
echo "  $DENY_WRITE"
echo "$DENY_WRITE" | python3 -c "import sys,json; d=json.load(sys.stdin); assert not d.get('success'); assert 'disabled' in (d.get('error') or '')"

echo ">>> assert shell task is rejected by disabled policy"
USER_ID=$(sqlite3 "$DB" "SELECT id FROM users WHERE username = 'admin';")
TASK_ID="019ed2fa-58e1-78a2-80aa-0000000005f5"
NOW=$(date -u +%Y-%m-%dT%H:%M:%S)
SELECTOR=$(printf '{"server_ids":["%s"],"group_ids":[],"tags":{}}' "$AGENT_ID")
sqlite3 "$DB" <<SQL
INSERT INTO tasks (
  id, owner_user_id, name, task_type, schedule, command,
  payload_json, cover_mode, server_selector_json,
  push_successful, notification_group_id, last_executed_at,
  last_result, enabled, created_at, updated_at
) VALUES (
  '$TASK_ID', '$USER_ID', 'm5-disabled-echo', '"shell"', NULL, 'echo should-not-run',
  NULL, '"specific"', '$SELECTOR',
  0, NULL, NULL, NULL, 1, '$NOW', '$NOW'
);
SQL
RUN_DENIED=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/tasks/$TASK_ID/run")
echo "  $RUN_DENIED"
FAILURES=$(echo "$RUN_DENIED" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('data',{}).get('summary',{}).get('failure',0))")
[[ "$FAILURES" -ge 1 ]] || { echo "FAIL: disabled shell task did not report failure"; exit 1; }
RUN_ERROR=$(echo "$RUN_DENIED" | python3 -c "import sys,json; d=json.load(sys.stdin); runs=d.get('data',{}).get('runs',[]); print(next((r.get('error') or '' for r in runs if r.get('status')=='failure'), ''))")
case "$RUN_ERROR" in
  *disabled*) echo "OK: disabled task error persisted" ;;
  *) echo "FAIL: disabled task error missing (got: $RUN_ERROR)"; exit 1 ;;
esac

echo ">>> assert terminal disabled branch exists"
grep -q "command_execution_disabled" "$ROOT/crates/agent/src/main.rs"
grep -q "terminal access is disabled by agent policy" "$ROOT/crates/agent/src/main.rs"

echo "M5 FILES PASS (file API + disabled command/file/terminal policy)"
