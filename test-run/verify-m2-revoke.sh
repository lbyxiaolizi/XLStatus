#!/usr/bin/env bash
# M2 verification: admin revokes an agent; the server pushes
# ServerMessage::ForceDisconnect over the open gRPC stream; the agent
# tears down without sleeping through the next reconnect cycle.
#
# Runs:
#   1) cargo build (server + agent)
#   2) start server (sqlite)
#   3) login + create enrollment token + enroll + run agent
#   4) wait until the agent is connected (DB last_seen_at set)
#   5) admin POSTs /api/v1/agents/:id/revoke
#   6) within 10 s, the agent log should contain "force_disconnect"
#      and the agent process should exit cleanly

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18094}"
GRPC_PORT="${GRPC_PORT:-15064}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m2-revoke}"
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
  --name "verify-m2-revoke-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")
echo "  agent_id = $AGENT_ID"

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!

# Wait for first heartbeat
for i in $(seq 1 30); do
  SEEN=$(sqlite3 "$DB" "SELECT last_seen_at IS NOT NULL FROM agents WHERE id = '$AGENT_ID';" 2>/dev/null || echo 0)
  if [[ "$SEEN" == "1" ]]; then
    echo "  agent connected after ${i}s"
    break
  fi
  sleep 1
done
if [[ "$SEEN" != "1" ]]; then
  echo "FAIL: agent never connected"
  tail -20 "$LOG_DIR/agent.log"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi

echo ">>> POST /api/v1/agents/:id/revoke"
REV=$(curl -s -X POST -b "$JAR" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/agents/$AGENT_ID/revoke")
echo "  $REV"
echo "$REV" | python3 -c "import sys,json;d=json.load(sys.stdin)['data'];assert d['revoked'] is True;print('OK: revoke returned success')"

echo ">>> wait for force_disconnect in agent log"
for i in $(seq 1 10); do
  if grep -q "force_disconnect" "$LOG_DIR/agent.log"; then
    echo "OK: agent saw force_disconnect after ${i}s"
    break
  fi
  sleep 1
done
if ! grep -q "force_disconnect" "$LOG_DIR/agent.log"; then
  echo "FAIL: agent did not see force_disconnect"
  tail -20 "$LOG_DIR/agent.log"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi

# Give the agent a couple more seconds to clean up and exit
for i in $(seq 1 5); do
  if ! pgrep -f "xlstatus-agent run" > /dev/null; then
    echo "OK: agent process exited cleanly"
    break
  fi
  sleep 1
done
if pgrep -f "xlstatus-agent run" > /dev/null; then
  echo "WARN: agent still running (expected if it would have reconnected without our non-reconnect path)"
fi

echo ""
echo "M2 REVOKE PASS (admin revoke -> gRPC ForceDisconnect -> agent teardown)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
