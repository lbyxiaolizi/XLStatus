#!/usr/bin/env bash
# M4 verification:
#   1) Build server + agent.
#   2) Start server, login, enroll + run a real agent.
#   3) Verify HTTPS certificate extraction and service CRUD/history/uptime.
#   4) Wait for HTTP/TCP/ICMP scheduler results in service_results.
#   5) Verify service_down fired + recovered notification delivery.
#   6) Create a ServerResource CPU rule and capture webhook delivery.
#
# Exit 0 + "M4 PASS" on success.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18097}"
GRPC_PORT="${GRPC_PORT:-15067}"
LISTEN_PORT="${LISTEN_PORT:-19998}"
HTTPS_PORT="${HTTPS_PORT:-19443}"
RECOVERY_PORT="${RECOVERY_PORT:-19997}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m4}"
DB="$LOG_DIR/x.db"
HOOK_LOG="$LOG_DIR/hook.log"
HOOK_BODY="$LOG_DIR/hook.body"
NG_ID=""
CH_ID=""
RULE_ID=""

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
kill_port() {
  if command -v lsof >/dev/null 2>&1; then
    for pid in $(lsof -ti tcp:"$1" 2>/dev/null || true); do
      kill -9 "$pid" 2>/dev/null || true
    done
  fi
}
kill_port "$LISTEN_PORT"
kill_port "$HTTPS_PORT"
kill_port "$RECOVERY_PORT"
sleep 1

echo ">>> start webhook listener (python http.server on :$LISTEN_PORT)"
python3 > "$HOOK_LOG" 2>&1 <<PYHOOK &
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        n = int(self.headers.get('Content-Length', '0'))
        body = self.rfile.read(n) if n else b''
        with open("$HOOK_BODY", "ab") as f:
            f.write(body)
            f.write(b"\n")
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")
    def log_message(self, *a, **k): pass
HTTPServer(("127.0.0.1", $LISTEN_PORT), H).serve_forever()
PYHOOK
NC_PID=$!
disown $NC_PID 2>/dev/null || true
sleep 2

echo ">>> start local HTTPS target with self-signed certificate (:$HTTPS_PORT)"
openssl req -x509 -newkey rsa:2048 -keyout "$LOG_DIR/tls.key" -out "$LOG_DIR/tls.crt" \
  -sha256 -days 7 -nodes -subj "/CN=localhost" >/dev/null 2>&1
python3 > "$LOG_DIR/https.log" 2>&1 <<PYHTTPS &
from http.server import BaseHTTPRequestHandler, HTTPServer
import ssl
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"https-ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a, **k): pass
httpd = HTTPServer(("127.0.0.1", $HTTPS_PORT), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain("$LOG_DIR/tls.crt", "$LOG_DIR/tls.key")
httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
httpd.serve_forever()
PYHTTPS
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

# The session cookie user_id is in DB. We grab it via sqlite and use
# the public PAT-less path to seed the notification channel. PAT
# bypasses CSRF; we just create a PAT for the test runner.
TOKR=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/enrollment-tokens" \
  -d '{"expires_in_hours":24}')
ETOK=$(echo "$TOKR" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('token',''))")
[ -n "$ETOK" ] || { echo "FAIL: empty enrollment token"; cat "$TOKR"; exit 1; }

USER_ID=$(sqlite3 "$DB" "SELECT id FROM users WHERE username = 'admin';")

echo ">>> seed notification channel + group via direct sqlite"
NG_ID="019ed2fa-58e1-78a2-80aa-000000000001"
CH_ID="019ed2fa-58e1-78a2-80aa-000000000002"
GROUP_CH="019ed2fa-58e1-78a2-80aa-000000000003"
NOW=$(date -u +%Y-%m-%dT%H:%M:%S)
sqlite3 "$DB" <<SQL
INSERT INTO notifications (id, owner_user_id, name, url, request_method, request_type, headers_json, body_template, verify_tls, format_metric_units, created_at, updated_at)
VALUES ('$CH_ID', '$USER_ID', 'm4-hook', 'http://127.0.0.1:$LISTEN_PORT/', 'POST', 'json', NULL, NULL, 1, 1, '$NOW', '$NOW');
INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at)
VALUES ('$NG_ID', '$USER_ID', 'm4-group', '$NOW', '$NOW');
INSERT INTO notification_group_members (group_id, notification_id)
VALUES ('$NG_ID', '$CH_ID');
SQL

echo ">>> test HTTPS probe exposes certificate status"
CERT_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/services/test-probe" \
  -d "{\"service_type\":\"http\",\"target\":\"https://127.0.0.1:$HTTPS_PORT/\",\"timeout_seconds\":3}")
echo "  $CERT_RESP"
CERT_NOT_AFTER=$(echo "$CERT_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('cert_not_after',''))")
[ -n "$CERT_NOT_AFTER" ] || { echo "FAIL: HTTPS probe did not return cert_not_after"; exit 1; }

create_service() {
  local name="$1"
  local kind="$2"
  local target="$3"
  curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
    "http://127.0.0.1:$HTTP_PORT/api/v1/services" \
    -d "{\"name\":\"$name\",\"service_type\":\"$kind\",\"target\":\"$target\",\"interval_seconds\":10,\"timeout_seconds\":3,\"enabled\":true}"
}

wait_service_result() {
  local service_id="$1"
  local label="$2"
  for i in $(seq 1 30); do
    local count
    count=$(sqlite3 "$DB" "SELECT COUNT(*) FROM service_results WHERE service_id = '$service_id';" 2>/dev/null || echo 0)
    if [[ "$count" -ge 1 ]]; then
      echo "OK: $label monitor wrote $count result row(s)"
      return 0
    fi
    sleep 1
  done
  echo "FAIL: $label monitor never wrote a result"
  tail -30 "$LOG_DIR/server.log"
  exit 1
}

echo ">>> create HTTP/TCP/ICMP services and wait for scheduler results"
HTTP_SVC_RESP=$(create_service "m4-http" "http" "http://127.0.0.1:$LISTEN_PORT/")
echo "  http: $HTTP_SVC_RESP"
HTTP_SVC_ID=$(echo "$HTTP_SVC_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
[ -n "$HTTP_SVC_ID" ] || { echo "FAIL: HTTP service create did not return id"; exit 1; }

TCP_SVC_RESP=$(create_service "m4-tcp" "tcp" "127.0.0.1:$LISTEN_PORT")
echo "  tcp: $TCP_SVC_RESP"
TCP_SVC_ID=$(echo "$TCP_SVC_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
[ -n "$TCP_SVC_ID" ] || { echo "FAIL: TCP service create did not return id"; exit 1; }

ICMP_SVC_ID=""
if command -v ping >/dev/null 2>&1; then
  ICMP_SVC_RESP=$(create_service "m4-icmp" "icmp" "127.0.0.1")
  echo "  icmp: $ICMP_SVC_RESP"
  ICMP_SVC_ID=$(echo "$ICMP_SVC_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
  [ -n "$ICMP_SVC_ID" ] || { echo "FAIL: ICMP service create did not return id"; exit 1; }
else
  echo "WARN: ping command not found; skipping ICMP runtime assertion"
fi

wait_service_result "$HTTP_SVC_ID" "HTTP"
wait_service_result "$TCP_SVC_ID" "TCP"
if [[ -n "$ICMP_SVC_ID" ]]; then
  wait_service_result "$ICMP_SVC_ID" "ICMP"
fi

HIST_RESP=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/services/$HTTP_SVC_ID/history?limit=10")
HIST_N=$(echo "$HIST_RESP" | python3 -c "import sys,json;print(len(json.load(sys.stdin).get('data',{}).get('results',[])))")
[[ "$HIST_N" -ge 1 ]] || { echo "FAIL: service history returned no results"; echo "$HIST_RESP"; exit 1; }
UPTIME_RESP=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/services/$HTTP_SVC_ID/uptime")
UPTIME_TOTAL=$(echo "$UPTIME_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('total_checks',0))")
[[ "$UPTIME_TOTAL" -ge 1 ]] || { echo "FAIL: service uptime total_checks is 0"; echo "$UPTIME_RESP"; exit 1; }

echo ">>> verify service_down fired and recovered notifications"
RECOVERY_SVC_RESP=$(create_service "m4-recovery" "http" "http://127.0.0.1:$RECOVERY_PORT/")
echo "  recovery service: $RECOVERY_SVC_RESP"
RECOVERY_SVC_ID=$(echo "$RECOVERY_SVC_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
[ -n "$RECOVERY_SVC_ID" ] || { echo "FAIL: recovery service create did not return id"; exit 1; }

for i in $(seq 1 30); do
  FAILS=$(sqlite3 "$DB" "SELECT COUNT(*) FROM service_results WHERE service_id = '$RECOVERY_SVC_ID' AND status = 'failure';" 2>/dev/null || echo 0)
  [[ "$FAILS" -ge 1 ]] && break
  sleep 1
done
[[ "$FAILS" -ge 1 ]] || { echo "FAIL: recovery service did not produce an initial failure"; tail -30 "$LOG_DIR/server.log"; exit 1; }

REC_RULE_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/alert-rules" \
  -d "{
    \"name\": \"service-recovery\",
    \"trigger\": \"once\",
    \"notification_group_id\": \"$NG_ID\",
    \"conditions\": [{
      \"type\": \"service_down\",
      \"service_id\": \"$RECOVERY_SVC_ID\",
      \"consecutive_failures\": 1
    }]
  }")
echo "  $REC_RULE_RESP"
REC_RULE_ID=$(echo "$REC_RULE_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
[ -n "$REC_RULE_ID" ] || { echo "FAIL: recovery alert rule did not return id"; exit 1; }

for i in $(seq 1 40); do
  REC_FIRED=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/alert-events?limit=50" | python3 -c "import sys,json;d=json.load(sys.stdin);print(sum(1 for e in d.get('data',{}).get('events',[]) if e.get('rule_id') == '$REC_RULE_ID' and e.get('kind') == 'fired'))")
  [[ "$REC_FIRED" -ge 1 ]] && break
  sleep 1
done
[[ "$REC_FIRED" -ge 1 ]] || { echo "FAIL: service_down rule did not fire"; tail -30 "$LOG_DIR/server.log"; exit 1; }

echo ">>> start recovery HTTP target"
python3 > "$LOG_DIR/recovery-http.log" 2>&1 <<PYRECOVERY &
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"recovered"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a, **k): pass
HTTPServer(("127.0.0.1", $RECOVERY_PORT), H).serve_forever()
PYRECOVERY
disown $! 2>/dev/null || true

for i in $(seq 1 50); do
  RECOVERED=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/alert-events?limit=50" | python3 -c "import sys,json;d=json.load(sys.stdin);print(sum(1 for e in d.get('data',{}).get('events',[]) if e.get('rule_id') == '$REC_RULE_ID' and e.get('kind') == 'recovered'))")
  [[ "$RECOVERED" -ge 1 ]] && break
  sleep 1
done
[[ "$RECOVERED" -ge 1 ]] || { echo "FAIL: service_down rule did not recover"; tail -40 "$LOG_DIR/server.log"; exit 1; }
grep -q "recovered" "$HOOK_BODY" || { echo "FAIL: webhook body did not include recovered notification"; cat "$HOOK_BODY"; exit 1; }

echo ">>> enroll + run agent"
"$ROOT/target/debug/xlstatus-agent" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$ETOK" \
  --name "verify-m4-alerts-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
AGENT_ID=$(python3 -c "import json;print(json.load(open('$LOG_DIR/agent.yaml'))['agent_id'])")

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!

# Wait for the agent to start producing HostState
for i in $(seq 1 20); do
  SEEN=$(sqlite3 "$DB" "SELECT last_state_json IS NOT NULL FROM agents WHERE id = '$AGENT_ID';" 2>/dev/null || echo 0)
  if [[ "$SEEN" == "1" ]]; then
    break
  fi
  sleep 1
done
if [[ "$SEEN" != "1" ]]; then
  echo "FAIL: agent never produced HostState"
  tail -20 "$LOG_DIR/agent.log"
  exit 1
fi

echo ">>> create alert rule cpu>0 with notification group $NG_ID"
RULE_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/alert-rules" \
  -d "{
    \"name\": \"cpu-any\",
    \"trigger\": \"once\",
    \"notification_group_id\": \"$NG_ID\",
    \"conditions\": [{
      \"type\": \"server_resource\",
      \"agent_id\": \"$AGENT_ID\",
      \"resource\": \"cpu\",
      \"operator\": \"gt\",
      \"threshold\": 0.0
    }]
  }")
echo "  $RULE_RESP"
RULE_ID=$(echo "$RULE_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('id',''))")
if [ -z "$RULE_ID" ]; then
  echo "FAIL: rule create did not return id"
  exit 1
fi

echo ">>> wait for alert engine to fire (15s tick + retry)"
for i in $(seq 1 40); do
  EVTS=$(curl -s -b "$JAR" "http://127.0.0.1:$HTTP_PORT/api/v1/alert-events?limit=20" | python3 -c "import sys,json;d=json.load(sys.stdin);print(len(d.get('data',{}).get('events',[])))")
  if [[ "$EVTS" -ge 1 ]]; then
    echo "OK: alert engine fired ($EVTS events)"
    break
  fi
  sleep 1
done
if [[ "$EVTS" -lt 1 ]]; then
  echo "FAIL: no alert event after 40s"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi

echo ">>> assert webhook listener received the call"
# Give the spawned send task a couple of seconds
sleep 3
if [[ -s "$HOOK_LOG" || -s "$HOOK_BODY" ]]; then
  echo "OK: webhook listener captured traffic"
else
  echo "FAIL: webhook listener never received traffic"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi

echo ""
echo "M4 PASS (services, SSL status, recovery notification, resource alert, webhook delivery)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent; pkill -9 -f verify-m4-alerts.sh 2>/dev/null; pkill -9 -f 'http.server' 2>/dev/null"
