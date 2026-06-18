#!/usr/bin/env bash
# M6 verification:
#   1) Build server + agent.
#   2) Start server, login, enroll + run a real agent.
#   3) Seed a dummy-webhook DDNS config via direct sqlite.
#   4) Start a webhook listener (python http.server) to capture the
#      IP update.
#   5) Let the agent's GeoIpReport/IP loop trigger DDNS automatically.
#   6) Assert the listener received a POST and ddns_history has a
#      success row.
#
# The agent config is adjusted to report IP every second so the test
# does not need to wait for the 60s background DDNS tick.
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18099}"
GRPC_PORT="${GRPC_PORT:-15069}"
LISTEN_PORT="${LISTEN_PORT:-19999}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m6}"
DB="$LOG_DIR/x.db"
HOOK_BODY="$LOG_DIR/hook.body"
HOOK_LOG="$LOG_DIR/hook.log"

mkdir -p "$LOG_DIR"
rm -f "$DB" "$HOOK_LOG" "$HOOK_BODY" "$LOG_DIR/agent.yaml"

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

echo ">>> start webhook listener"
python3 > "$HOOK_LOG" 2>&1 <<PYHOOK &
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('Content-Length', '0'))
        body = self.rfile.read(n) if n else b''
        with open("$HOOK_BODY", "wb") as f:
            f.write(body)
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")
    def log_message(self, *a, **k): pass
HTTPServer(("127.0.0.1", $LISTEN_PORT), H).serve_forever()
PYHOOK
disown $! 2>/dev/null || true
sleep 2

echo ">>> start server"
nohup env CONFIG_FILE="$LOG_DIR/cfg.toml" \
  XLSTATUS_ALLOW_PRIVATE_OUTBOUND=1 \
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

echo ">>> enroll + run agent (so we have a valid agent_id)"
"$ROOT/target/debug/xlstatus-agent" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$ETOK" \
  --name "verify-m6-ddns-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")
python3 - "$LOG_DIR/agent.yaml" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    cfg = json.load(f)
cfg["ip_report_interval_seconds"] = 1
with open(path, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2)
PY

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!
sleep 5

echo ">>> POST /api/v1/ddns/configs (webhook provider)"
CFG_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/ddns/configs" \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"name\": \"m6-webhook\",
    \"provider\": \"webhook\",
    \"domain\": \"m6.example.test\",
    \"webhook_url\": \"http://127.0.0.1:$LISTEN_PORT/\",
    \"enabled\": true
  }")
echo "  $CFG_RESP"
CFG_ID=$(echo "$CFG_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('config',{}).get('id',''))")
[ -n "$CFG_ID" ] || { echo "FAIL: ddns config create did not return id"; exit 1; }

echo ">>> GET /api/v1/ddns/configs"
LIST_RESP=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/ddns/configs")
echo "  $LIST_RESP"
N=$(echo "$LIST_RESP" | python3 -c "import sys,json;print(len(json.load(sys.stdin).get('data',{}).get('configs',[])))")
if [[ "$N" -lt 1 ]]; then
  echo "FAIL: list returned 0 configs"; exit 1
fi

# Trigger the hot-reload endpoint so the running DdnsManager
# picks up the new config (no server restart needed).
echo ">>> POST /api/v1/ddns/reload"
RELOAD_RESP=$(curl -s -X POST -b "$JAR" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/ddns/reload")
echo "  $RELOAD_RESP"

echo ">>> wait for agent IP report to trigger DDNS webhook"
for i in $(seq 1 20); do
  if [[ -e "$HOOK_BODY" ]]; then
    echo "OK: webhook request received"
    break
  fi
  sleep 1
done
if [[ ! -e "$HOOK_BODY" ]]; then
  echo "FAIL: webhook listener never received traffic"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "  hook body: $(cat $HOOK_BODY | head -c 200)"
echo ">>> assert ddns_history has a success row"
H=$(sqlite3 "$DB" "SELECT COUNT(*) FROM ddns_history WHERE config_id = '$CFG_ID' AND success = 1;")
if [[ "$H" -lt 1 ]]; then
  echo "FAIL: ddns_history has no success rows for $CFG_ID"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: $H success row(s) in ddns_history"

echo ">>> GET /api/v1/ddns/configs/$CFG_ID/history"
HIST_RESP=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/ddns/configs/$CFG_ID/history")
echo "  $HIST_RESP"

echo ""
echo "M6 PASS (DDNS: agent IP report triggers webhook provider, history persisted)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent; pkill -9 -f 'http.server'"
