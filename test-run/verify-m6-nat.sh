#!/usr/bin/env bash
# M6 NAT verification:
#   1) Build server + agent.
#   2) Start a local HTTP service that stands in for an "agent-local"
#      private service.
#   3) Start server, login, enroll + run a real agent.
#   4) Create a NAT mapping through the real HTTP API.
#   5) Let the running NatTunnelManager reload the enabled mapping.
#   6) Curl the public NAT port and assert the response came from the
#      local private service via the agent IoStream reverse tunnel.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18103}"
GRPC_PORT="${GRPC_PORT:-15073}"
SERVICE_PORT="${SERVICE_PORT:-19081}"
NAT_PORT="${NAT_PORT:-19082}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m6-nat}"
DB="$LOG_DIR/x.db"
SERVICE_LOG="$LOG_DIR/private-http.log"
BODY_FILE="$LOG_DIR/private-body.txt"

mkdir -p "$LOG_DIR"
rm -f "$DB" "$LOG_DIR/agent.yaml" "$SERVICE_LOG" "$BODY_FILE"

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
pkill -9 -f "PRIVATE_NAT_SERVICE_$SERVICE_PORT" 2>/dev/null || true
sleep 1

echo ">>> start private service on agent side"
python3 > "$SERVICE_LOG" 2>&1 <<PY &
from http.server import BaseHTTPRequestHandler, HTTPServer

BODY = b"m6-nat-ok"

class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)
    def log_message(self, *a, **k):
        pass

print("PRIVATE_NAT_SERVICE_$SERVICE_PORT", flush=True)
HTTPServer(("127.0.0.1", $SERVICE_PORT), H).serve_forever()
PY
disown $! 2>/dev/null || true
sleep 2

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
  --name "verify-m6-nat-agent" \
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
[[ "$SEEN" == "1" ]] || { echo "FAIL: agent not online"; tail -30 "$LOG_DIR/agent.log"; exit 1; }

echo ">>> create NAT mapping"
MAP_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/nat/mappings" \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"local_host\": \"127.0.0.1\",
    \"local_port\": $SERVICE_PORT,
    \"public_port\": $NAT_PORT,
    \"protocol\": \"tcp\",
    \"description\": \"m6 nat test\"
  }")
echo "  $MAP_RESP"
MAP_ID=$(echo "$MAP_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('mapping',{}).get('id',''))")
[ -n "$MAP_ID" ] || { echo "FAIL: nat mapping create did not return id"; exit 1; }

sleep 2

echo ">>> curl public NAT port"
for i in $(seq 1 10); do
  if curl -fsS "http://127.0.0.1:$NAT_PORT/" > "$BODY_FILE" 2>/dev/null; then
    break
  fi
  sleep 1
done

if [[ ! -f "$BODY_FILE" ]]; then
  echo "FAIL: NAT public port never returned a response"
  tail -40 "$LOG_DIR/server.log" || true
  tail -40 "$LOG_DIR/agent.log" || true
  exit 1
fi

BODY=$(cat "$BODY_FILE")
echo "  body: $BODY"
[[ "$BODY" == "m6-nat-ok" ]] || {
  echo "FAIL: unexpected NAT response body '$BODY'"
  tail -40 "$LOG_DIR/server.log" || true
  tail -40 "$LOG_DIR/agent.log" || true
  exit 1
}

echo ""
echo "M6 NAT PASS (public port reaches agent-local HTTP service via IoStream reverse tunnel)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent; pkill -9 -f PRIVATE_NAT_SERVICE_$SERVICE_PORT"
