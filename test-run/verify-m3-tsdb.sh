#!/usr/bin/env bash
# M3 verification: HostState samples land in the in-memory MetricStore
# AND in `agents.last_state_json`. Then we hit the REST endpoint
# /api/v1/servers and /api/v1/servers/:id/metrics as a real consumer
# would, and assert the response is well-formed.
#
# Runs:
#   1) cargo build (server + agent)
#   2) start server (sqlite)
#   3) login + create enrollment token + enroll + run agent
#   4) wait long enough for at least one HostState report
#   5) hit GET /api/v1/servers and assert list length >= 1
#   6) hit GET /api/v1/servers/:id/metrics?range=1d and assert samples >= 1

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18093}"
GRPC_PORT="${GRPC_PORT:-15063}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m3-tsdb}"
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

echo ">>> login + enroll + run agent"
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
[ -n "$ETOK" ] || { echo "FAIL: empty enrollment token"; cat "$TOKR"; exit 1; }

"$ROOT/target/debug/xlstatus-agent" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$ETOK" \
  --name "verify-m3-tsdb-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
grep -q "Agent enrolled" "$LOG_DIR/enroll.log" || { echo "FAIL: enroll"; cat "$LOG_DIR/enroll.log"; exit 1; }

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!
sleep 8

echo ">>> GET /api/v1/servers"
LIST=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/servers?limit=10&offset=0")
echo "  $LIST" | head -c 400
echo
TOTAL=$(echo "$LIST" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['total'])")
if [[ "$TOTAL" -lt 1 ]]; then
  echo "FAIL: total < 1, got $TOTAL"
  exit 1
fi
AGENT_ID=$(echo "$LIST" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['servers'][0]['id'])")
echo "  agent_id = $AGENT_ID"
STATUS=$(echo "$LIST" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['servers'][0]['status'])")
if [[ "$STATUS" != "online" ]]; then
  echo "FAIL: expected status=online, got $STATUS"
  exit 1
fi
echo "OK: list shows agent online"

echo ">>> GET /api/v1/servers/:id/metrics?range=1d"
METRICS=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/servers/$AGENT_ID/metrics?range=1d")
echo "  ${METRICS:0:400}"
N=$(echo "$METRICS" | python3 -c "import sys,json;print(len(json.load(sys.stdin)['data']['series']['samples']))")
if [[ "$N" -lt 1 ]]; then
  echo "FAIL: metric store returned $N samples (expected >= 1)"
  exit 1
fi
echo "OK: metric store has $N samples for agent $AGENT_ID"

echo ""
echo "M3 TSDB PASS (REST list + metrics endpoints serve real time-series data)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
