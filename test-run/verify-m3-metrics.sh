#!/usr/bin/env bash
# M3 verification: agent connects via gRPC, sends HostInfo + HostState,
# server persists them to the agents.last_state_json / last_info_json
# columns. (Full TSDB writes are deferred to M8 per plan/08-roadmap.md.)
#
# Runs:
#   1) cargo build (server + agent)
#   2) start server (sqlite)
#   3) login admin -> create enrollment token -> enroll agent -> run agent
#   4) wait long enough for at least one HostState report (3 s)
#   5) SELECT agents.last_state_json / last_info_json and assert non-empty

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18091}"
GRPC_PORT="${GRPC_PORT:-15061}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m3}"
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
  --name "verify-m3-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
grep -q "Agent enrolled" "$LOG_DIR/enroll.log" || { echo "FAIL: enroll"; cat "$LOG_DIR/enroll.log"; exit 1; }

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!
sleep 8

echo ">>> check last_state_json / last_info_json in sqlite"
SEEN=$(sqlite3 "$DB" \
  "SELECT (last_state_json IS NOT NULL), (last_info_json IS NOT NULL), (last_seen_at IS NOT NULL) FROM agents LIMIT 1;")
echo "  $SEEN"
if [[ "$SEEN" != "1|1|1" ]]; then
  echo "FAIL: expected last_state=1 last_info=1 last_seen=1, got $SEEN"
  tail -20 "$LOG_DIR/server.log"
  tail -20 "$LOG_DIR/agent.log"
  exit 1
fi

LEN=$(sqlite3 "$DB" "SELECT length(last_state_json) FROM agents LIMIT 1;")
if [[ "$LEN" -lt 100 ]]; then
  echo "FAIL: last_state_json too short ($LEN bytes)"
  exit 1
fi
echo "OK: last_state_json length = $LEN bytes"

INFO_LEN=$(sqlite3 "$DB" "SELECT length(last_info_json) FROM agents LIMIT 1;")
if [[ "$INFO_LEN" -lt 50 ]]; then
  echo "FAIL: last_info_json too short ($INFO_LEN bytes)"
  exit 1
fi
echo "OK: last_info_json length = $INFO_LEN bytes"

echo ""
echo "M3 PASS (HostInfo + HostState persistence on agent connect)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
