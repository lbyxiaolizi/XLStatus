#!/usr/bin/env bash
# M5 verification:
#   1) Build server + agent.
#   2) Start server, login, enroll + run a real agent.
#   3) Seed a task via direct sqlite (CoverMode::Specific against
#      the enrolled agent's UUID) with a Shell command that prints
#      "ok-from-m5".
#   4) POST /api/v1/tasks/:id/run. The handler dispatches a
#      gRPC ServerMessage::Task to the live agent, the agent
#      executes the command and replies with TaskResult.
#   5) Assert /api/v1/tasks/:id/runs returns >= 1 row with status
#      "success" and the captured stdout.
#
# Exit 0 + "M5 PASS" on success.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18098}"
GRPC_PORT="${GRPC_PORT:-15068}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m5}"
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

# Issue a PAT for the test runner (PATs bypass CSRF).
TOKR=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/enrollment-tokens" \
  -d '{"expires_in_hours":24}')
ETOK=$(echo "$TOKR" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('token',''))")
[ -n "$ETOK" ] || { echo "FAIL: empty enrollment token"; cat "$TOKR"; exit 1; }

echo ">>> enroll + run agent"
"$ROOT/target/debug/xlstatus-agent" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$ETOK" \
  --name "verify-m5-task-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")
echo "  agent_id=$AGENT_ID"

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!

# Wait for the agent to be online (last_seen_at updated).
for i in $(seq 1 20); do
  SEEN=$(sqlite3 "$DB" "SELECT last_seen_at IS NOT NULL FROM agents WHERE id = '$AGENT_ID';" 2>/dev/null || echo 0)
  if [[ "$SEEN" == "1" ]]; then
    break
  fi
  sleep 1
done
if [[ "$SEEN" != "1" ]]; then
  echo "FAIL: agent never showed up as online"
  tail -20 "$LOG_DIR/agent.log"
  exit 1
fi

USER_ID=$(sqlite3 "$DB" "SELECT id FROM users WHERE username = 'admin';")
TASK_ID="019ed2fa-58e1-78a2-80aa-0000000000a5"
NOW=$(date -u +%Y-%m-%dT%H:%M:%S)
SELECTOR=$(printf '{"server_ids":["%s"],"group_ids":[],"tags":{}}' "$AGENT_ID")

echo ">>> seed task with shell command 'echo ok-from-m5'"
sqlite3 "$DB" <<SQL
INSERT INTO tasks (
  id, owner_user_id, name, task_type, schedule, command,
  payload_json, cover_mode, server_selector_json,
  push_successful, notification_group_id, last_executed_at,
  last_result, enabled, created_at, updated_at
) VALUES (
  '$TASK_ID', '$USER_ID', 'm5-echo', '"shell"', NULL, 'echo ok-from-m5',
  NULL, '"specific"', '$SELECTOR',
  0, NULL, NULL, NULL, 1, '$NOW', '$NOW'
);
SQL

echo ">>> POST /api/v1/tasks/$TASK_ID/run"
RUN_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/tasks/$TASK_ID/run")
echo "  $RUN_RESP"
SUCC=$(echo "$RUN_RESP" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('data',{}).get('summary',{}).get('success',-1))")
if [[ "$SUCC" -lt 1 ]]; then
  echo "FAIL: run_task did not report any successes"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: run_task reported $SUCC success(es)"

echo ">>> assert /api/v1/tasks/$TASK_ID/runs has a success row"
RUNS_RESP=$(curl -s -b "$JAR" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/tasks/$TASK_ID/runs?limit=10")
echo "  $RUNS_RESP"
N_SUCCESS=$(echo "$RUNS_RESP" | python3 -c "import sys,json;d=json.load(sys.stdin).get('data',{}).get('runs',[]);print(sum(1 for r in d if r.get('status')=='success'))")
if [[ "$N_SUCCESS" -lt 1 ]]; then
  echo "FAIL: task_runs has no success rows"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: task_runs contains $N_SUCCESS success row(s)"

# Confirm the captured output is persisted.
OUTPUT=$(echo "$RUNS_RESP" | python3 -c "import sys,json;d=json.load(sys.stdin).get('data',{}).get('runs',[]);print(next((r.get('output','') for r in d if r.get('status')=='success'),''))")
# The HTTP summary does not echo the stdout; the row itself carries it.
ROW_OUTPUT=$(sqlite3 "$DB" "SELECT output FROM task_runs WHERE task_id = '$TASK_ID' AND status = 'success' LIMIT 1;")
echo "  captured stdout: $ROW_OUTPUT"
case "$ROW_OUTPUT" in
  *ok-from-m5*) echo "OK: stdout captured" ;;
  *) echo "FAIL: stdout not captured (got: $ROW_OUTPUT)"; exit 1 ;;
esac

echo ""
echo "M5 PASS (task dispatch via gRPC, agent execution, result persisted)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
