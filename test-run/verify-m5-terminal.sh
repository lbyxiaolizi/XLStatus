#!/usr/bin/env bash
# M5 terminal verification:
#   1) Start server + real agent.
#   2) Create a terminal session for the live agent.
#   3) Open /ws/terminal/:session_id, send `echo ok`, and assert the
#      output stream emits the command result.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18102}"
GRPC_PORT="${GRPC_PORT:-15072}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m5-terminal}"
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
  --name "verify-m5-terminal-agent" \
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

echo ">>> create terminal session"
TERM_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/terminal/sessions" \
  -d "{\"agent_id\":\"$AGENT_ID\",\"cols\":80,\"rows\":24}")
echo "  $TERM_RESP"
SESSION_ID=$(echo "$TERM_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('session_id',''))")
[ -n "$SESSION_ID" ] || { echo "FAIL: empty terminal session id"; exit 1; }
SESSION_COOKIE=$(awk '$6=="xlstatus_session" {print $7}' "$JAR")
[ -n "$SESSION_COOKIE" ] || { echo "FAIL: empty session cookie"; exit 1; }

python3 - <<PY > "$LOG_DIR/terminal.out" 2>&1
import base64, json, os, socket, struct, sys, time

host = "127.0.0.1"
port = $HTTP_PORT
session = "$SESSION_ID"
cookie = "xlstatus_session=$SESSION_COOKIE"

key = base64.b64encode(os.urandom(16)).decode()
req = (
    f"GET /ws/terminal/{session} HTTP/1.1\\r\\n"
    f"Host: {host}:{port}\\r\\n"
    f"Upgrade: websocket\\r\\n"
    f"Connection: Upgrade\\r\\n"
    f"Sec-WebSocket-Key: {key}\\r\\n"
    f"Sec-WebSocket-Version: 13\\r\\n"
    f"Cookie: {cookie}\\r\\n"
    f"\\r\\n"
)
s = socket.create_connection((host, port), timeout=15)
s.sendall(req.encode())
buf = b""
while b"\\r\\n\\r\\n" not in buf:
    chunk = s.recv(4096)
    if not chunk:
        break
    buf += chunk
header, _, rest = buf.partition(b"\\r\\n\\r\\n")
if b"101" not in header.split(b"\\r\\n", 1)[0]:
    print("FAIL_HANDSHAKE", header.decode(errors="replace"), file=sys.stderr)
    sys.exit(1)

def decode_frame(data):
    if len(data) < 2:
        return None, data
    b1, b2 = data[0], data[1]
    opcode = b1 & 0x0F
    masked = (b2 & 0x80) != 0
    length = b2 & 0x7F
    idx = 2
    if length == 126:
        if len(data) < idx + 2:
            return None, data
        length = struct.unpack("!H", data[idx:idx+2])[0]
        idx += 2
    elif length == 127:
        if len(data) < idx + 8:
            return None, data
        length = struct.unpack("!Q", data[idx:idx+8])[0]
        idx += 8
    if masked:
        if len(data) < idx + 4:
            return None, data
        mask = data[idx:idx+4]
        idx += 4
    else:
        mask = b""
    if len(data) < idx + length:
        return None, data
    payload = data[idx:idx+length]
    if mask:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return (opcode, payload), data[idx+length:]

def send_frame(opcode, payload):
    data = payload.encode()
    b1 = 0x80 | opcode
    mask = os.urandom(4)
    length = len(data)
    if length < 126:
        header = struct.pack("!BB", b1, 0x80 | length)
    elif length < 65536:
        header = struct.pack("!BBH", b1, 0x80 | 126, length)
    else:
        header = struct.pack("!BBQ", b1, 0x80 | 127, length)
    masked = bytes(data[i] ^ mask[i % 4] for i in range(len(data)))
    s.sendall(header + mask + masked)

# consume ready frame(s)
deadline = time.time() + 12
got_ok = False
data = rest
sent = False
while time.time() < deadline and not got_ok:
    try:
        s.settimeout(max(0.1, deadline - time.time()))
        chunk = s.recv(65536)
    except socket.timeout:
        break
    if not chunk:
        break
    data += chunk
    while True:
        frame, data = decode_frame(data)
        if frame is None:
            break
        opcode, payload = frame
        if opcode != 0x1:
            continue
        obj = json.loads(payload)
        if obj.get("type") == "ready" and not sent:
            send_frame(0x1, json.dumps({"type":"input","data":"echo ok\n"}))
            sent = True
        if obj.get("type") == "output" and "ok" in obj.get("data",""):
            got_ok = True
            break

print(json.dumps({"ok": got_ok, "sent": sent}))
PY

OUT=$(cat "$LOG_DIR/terminal.out")
echo "  $OUT"
echo "$OUT" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d.get('ok') and d.get('sent')"

echo ""
echo "M5 TERMINAL PASS (Web Terminal /ws/terminal sends echo ok)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
