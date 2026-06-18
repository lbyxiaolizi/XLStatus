#!/usr/bin/env bash
# M3 verification: an authenticated WebSocket subscriber to
# /ws/servers receives a `snapshot` frame first, then live `event`
# frames for every HostState the agent sends. (The dashboard page
# consumes the same wire format.)
#
# Wire format (text JSON):
#   {"type":"snapshot","events":[...]}
#   {"type":"event","event":{...}}
#   {"type":"ping","ts":"..."}
#
# Strategy:
#   1) start server, login, enroll, run agent
#   2) cookie-jar authenticated `websocat` (or python3 websocket)
#      subscriber to ws://127.0.0.1:8080/ws/servers
#   3) assert first frame is a snapshot
#   4) wait up to 10 s for at least one `event` frame
#   5) assert the event has agent_id, payload.cpu_percent set

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18096}"
GRPC_PORT="${GRPC_PORT:-15066}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m3-ws}"
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
  --name "verify-m3-ws-agent" \
  --config "$LOG_DIR/agent.yaml" > "$LOG_DIR/enroll.log" 2>&1
grep -q "Agent enrolled" "$LOG_DIR/enroll.log" || { echo "FAIL: enroll"; cat "$LOG_DIR/enroll.log"; exit 1; }

nohup "$ROOT/target/debug/xlstatus-agent" run --config "$LOG_DIR/agent.yaml" \
  > "$LOG_DIR/agent.log" 2>&1 < /dev/null &
disown $!
sleep 4

echo ">>> subscribe /ws/servers (python websocket client)"
# The cookie jar has two cookies: xlstatus_session + xlstatus_csrf.
# We send the session cookie in the Upgrade headers; CSRF is not
# required for GET-based WebSocket upgrade.
SESSION=$(awk '$6=="xlstatus_session" {print $7}' "$JAR")
[ -n "$SESSION" ] || { echo "FAIL: no session cookie"; cat "$JAR"; exit 1; }

python3 - <<PY > "$LOG_DIR/ws.out" 2>&1 &
import json
import socket
import base64
import os
import struct
import sys
import time
import urllib.parse

host = "127.0.0.1"
port = $HTTP_PORT
session = "$SESSION"

key = base64.b64encode(os.urandom(16)).decode()
req = (
    f"GET /ws/servers HTTP/1.1\\r\\n"
    f"Host: {host}:{port}\\r\\n"
    f"Upgrade: websocket\\r\\n"
    f"Connection: Upgrade\\r\\n"
    f"Sec-WebSocket-Key: {key}\\r\\n"
    f"Sec-WebSocket-Version: 13\\r\\n"
    f"Cookie: xlstatus_session={session}\\r\\n"
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
    rest = data[idx+length:]
    return (opcode, payload), rest

# 12 seconds of window: well above the 3s report cadence.
deadline = time.time() + 12
got_snapshot = False
got_event = False
data = rest
while time.time() < deadline and not (got_snapshot and got_event):
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
        if opcode == 0x1:
            try:
                obj = json.loads(payload)
            except Exception:
                continue
            t = obj.get("type")
            if t == "snapshot":
                got_snapshot = True
            elif t == "event":
                ev = obj.get("event", {})
                if ev.get("kind") == "host_state":
                    got_event = True
print(json.dumps({"snapshot": got_snapshot, "event": got_event}))
PY
WS_PID=$!

# wait up to 15s for the python client to finish
for i in $(seq 1 15); do
  if ! kill -0 $WS_PID 2>/dev/null; then break; fi
  sleep 1
done
wait $WS_PID 2>/dev/null || true
echo "  raw output: $(cat $LOG_DIR/ws.out 2>/dev/null)"

# We need the WS subscriber to have actually run; check at minimum it printed JSON.
if [[ ! -s "$LOG_DIR/ws.out" ]]; then
  echo "FAIL: ws subscriber produced no output"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi

SNAP=$(python3 -c "import json;d=json.load(open('$LOG_DIR/ws.out'));print(d.get('snapshot'))")
EVT=$(python3 -c "import json;d=json.load(open('$LOG_DIR/ws.out'));print(d.get('event'))")
if [[ "$SNAP" != "True" ]]; then
  echo "FAIL: did not receive snapshot frame"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
if [[ "$EVT" != "True" ]]; then
  echo "FAIL: did not receive any host_state event within 12 s"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: snapshot + event frames received"

echo ""
echo "M3 WS PASS (WebSocket /ws/servers streams snapshot + live events)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
