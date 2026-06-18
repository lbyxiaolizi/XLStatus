#!/usr/bin/env bash
# M5 scheduler verification:
#   1) Start server + real agent.
#   2) Seed a scheduled shell task for the enrolled agent.
#   3) Wait for the server-side TaskScheduler tick to dispatch it.
#   4) Assert task_runs has a success row with stdout captured.
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18108}"
GRPC_PORT="${GRPC_PORT:-15078}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m5-scheduler}"
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

echo ">>> login admin"
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
  --name "verify-m5-scheduler-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")
echo "  agent_id=$AGENT_ID"

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!

for i in $(seq 1 20); do
  SEEN=$(sqlite3 "$DB" "SELECT last_seen_at IS NOT NULL FROM agents WHERE id = '$AGENT_ID';" 2>/dev/null || echo 0)
  [[ "$SEEN" == "1" ]] && break
  sleep 1
done
[[ "$SEEN" == "1" ]] || { echo "FAIL: agent never became online"; tail -20 "$LOG_DIR/agent.log"; exit 1; }

USER_ID=$(sqlite3 "$DB" "SELECT id FROM users WHERE username = 'admin';")
TASK_ID="019ed2fa-58e1-78a2-80aa-0000000000b5"
NOW=$(date -u +%Y-%m-%dT%H:%M:%S)
SELECTOR=$(printf '{"server_ids":["%s"],"group_ids":[],"tags":{}}' "$AGENT_ID")

echo ">>> seed scheduled shell task"
sqlite3 "$DB" <<SQL
INSERT INTO tasks (
  id, owner_user_id, name, task_type, schedule, command,
  payload_json, cover_mode, server_selector_json,
  push_successful, notification_group_id, last_executed_at,
  last_result, enabled, created_at, updated_at
) VALUES (
  '$TASK_ID', '$USER_ID', 'm5-scheduled-echo', '"shell"', '* * * * *', 'echo ok-from-m5-scheduler',
  NULL, '"specific"', '$SELECTOR',
  0, NULL, NULL, NULL, 1, '$NOW', '$NOW'
);
SQL

echo ">>> wait for scheduler dispatch"
for i in $(seq 1 45); do
  N_SUCCESS=$(sqlite3 "$DB" "SELECT COUNT(*) FROM task_runs WHERE task_id = '$TASK_ID' AND status = 'success' AND output LIKE '%ok-from-m5-scheduler%';" 2>/dev/null || echo 0)
  if [[ "$N_SUCCESS" -ge 1 ]]; then
    echo "OK: scheduler produced $N_SUCCESS success row(s)"
    break
  fi
  sleep 1
done
if [[ "$N_SUCCESS" -lt 1 ]]; then
  echo "FAIL: scheduler did not produce success row"
  tail -60 "$LOG_DIR/server.log"
  exit 1
fi

LAST_RESULT=$(sqlite3 "$DB" "SELECT last_result FROM tasks WHERE id = '$TASK_ID';")
case "$LAST_RESULT" in
  *success=1*) echo "OK: task last_result updated: $LAST_RESULT" ;;
  *) echo "FAIL: unexpected last_result '$LAST_RESULT'"; exit 1 ;;
esac

echo ""
echo "M5 SCHEDULER PASS (scheduled task dispatched via gRPC and result persisted)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
