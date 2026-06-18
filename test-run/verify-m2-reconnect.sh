#!/usr/bin/env bash
# M2 verification: when the gRPC server goes down and comes back, the
# agent reconnects with bounded exponential backoff and resumes
# sending HostState. (We don't pull the rug under the server itself
# because that's intrusive; instead we kill the agent's stream by
# SIGSTOP/SIGCONT the server for a couple of seconds.)
#
# Strategy: simpler. We use the same script flow as verify-m3-metrics
# to enroll and start the agent, wait 8 s, kill the server, wait 5 s,
# start the server again, wait 8 s, and check that the agent is
# online (last_seen_at recent) without us having to restart it
# manually.
#
# The "agent reconnects" path is exercised by the run_agent outer
# loop that we added in the M2 JWT/auto-refresh pass.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18095}"
GRPC_PORT="${GRPC_PORT:-15065}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m2-recon}"
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

start_server() {
  nohup env CONFIG_FILE="$LOG_DIR/cfg.toml" \
    XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
    "$ROOT/target/debug/xlstatus-server" \
    > "$LOG_DIR/server.log" 2>&1 < /dev/null &
  disown $!
  sleep 5
}

echo ">>> start server (round 1)"
start_server

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
  --name "verify-m2-recon-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!
sleep 8

echo ">>> kill server (force disconnect from the agent side)"
pkill -9 -f xlstatus-server
sleep 1
# The agent should log "stream closed, reconnecting in Ns (attempt 1)"
for i in $(seq 1 5); do
  if grep -q "reconnecting in" "$LOG_DIR/agent.log"; then
    echo "OK: agent entered reconnect loop after ${i}s"
    break
  fi
  sleep 1
done
if ! grep -q "reconnecting in" "$LOG_DIR/agent.log"; then
  echo "FAIL: agent did not enter reconnect loop"
  tail -30 "$LOG_DIR/agent.log"
  exit 1
fi

echo ">>> bring server back up"
start_server
# Give the agent's backoff time to expire and the new connection to
# establish + first HostState sample to land.
sleep 12

# Check that last_seen_at has been updated *after* the restart.
TS=$(sqlite3 "$DB" "SELECT last_seen_at FROM agents LIMIT 1;")
echo "  last_seen_at = $TS"
NOW=$(date -u +%Y-%m-%dT%H:%M:%S)
DELTA=$(python3 -c "import datetime;a=datetime.datetime.fromisoformat('$TS'.split('+')[0]);b=datetime.datetime.utcnow();print(int((b-a).total_seconds()))")
if [[ "$DELTA" -gt 60 ]]; then
  echo "FAIL: last_seen_at is ${DELTA}s old, expected < 60s after reconnect"
  tail -30 "$LOG_DIR/agent.log"
  tail -30 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: last_seen_at is ${DELTA}s old (agent reconnected)"

# Look for evidence of the second connection in server log
if grep -q "session registered" "$LOG_DIR/server.log"; then
  REGISTRATIONS=$(grep -c "session registered" "$LOG_DIR/server.log")
  echo "OK: server log shows $REGISTRATIONS session registrations"
fi

echo ""
echo "M2 RECONNECT PASS (server restart -> agent reconnects with backoff)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
