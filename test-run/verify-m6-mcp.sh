#!/usr/bin/env bash
# M6 MCP verification:
#   1) Start server + real agent.
#   2) Create a PAT with server + exec + transfer scopes.
#   3) Exercise MCP tools: server.list, server.exec, fs.write/read/list/delete,
#      fs.download_url and fs.upload_url, including real temporary transfer
#      GET/PUT, bad-token rejection, and temporary URL rate limiting.
#   4) Assert MCP endpoints reject cookie-only / no-PAT access.
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18100}"
GRPC_PORT="${GRPC_PORT:-15070}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m6-mcp}"
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
  --name "verify-m6-mcp-agent" \
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

echo ">>> create PAT for MCP"
PAT_RESP=$(curl -s -X POST -b "$JAR" -H "Content-Type: application/json" -H "X-CSRF-Token: $CSRF" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/tokens" \
  -d "{\"name\":\"m6-mcp\",\"scopes\":[\"server:read\",\"server:exec\",\"transfer:read\",\"transfer:write\"],\"server_ids\":[\"$AGENT_ID\"]}")
PAT=$(echo "$PAT_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('data',{}).get('token',''))")
[ -n "$PAT" ] || { echo "FAIL: empty PAT"; echo "$PAT_RESP"; exit 1; }

call_mcp() {
  local tool="$1"
  local args="$2"
  curl -s -X POST -H "Authorization: Bearer $PAT" -H "Content-Type: application/json" \
    "http://127.0.0.1:$HTTP_PORT/api/v1/mcp/execute" \
    -d "{\"tool\":\"$tool\",\"arguments\":$args}"
}

echo ">>> assert MCP rejects cookie-only access"
NO_PAT=$(curl -s -o /dev/null -w '%{http_code}' -X POST -b "$JAR" -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/mcp/execute" \
  -d '{"tool":"meta.whoami","arguments":{}}')
[[ "$NO_PAT" == "403" ]] || { echo "FAIL: MCP cookie-only status $NO_PAT"; exit 1; }
JSONRPC_NO_PAT=$(curl -s -o /dev/null -w '%{http_code}' -X POST -b "$JAR" -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/mcp" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}')
[[ "$JSONRPC_NO_PAT" == "403" ]] || { echo "FAIL: /mcp cookie-only status $JSONRPC_NO_PAT"; exit 1; }

echo ">>> JSON-RPC /mcp initialize + tools/list"
INIT=$(curl -s -X POST -H "Authorization: Bearer $PAT" -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/mcp" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}')
echo "  init: $INIT"
echo "$INIT" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d['result']['serverInfo']['name']=='XLStatus MCP Server'"
JSON_TOOLS=$(curl -s -X POST -H "Authorization: Bearer $PAT" -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/mcp" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}')
echo "  tools/list: $JSON_TOOLS"
echo "$JSON_TOOLS" | python3 -c "import sys,json;d=json.load(sys.stdin);names=[t['name'] for t in d['result']['tools']];assert 'server.exec' in names and 'fs.write' in names"

echo ">>> server.list"
LIST=$(call_mcp server.list '{}')
echo "  $LIST"
COUNT=$(echo "$LIST" | python3 -c "import sys,json;d=json.load(sys.stdin);print(len(d.get('data',{}).get('result',{}).get('servers',[])))")
[[ "$COUNT" -ge 1 ]] || { echo "FAIL: server.list returned 0"; exit 1; }

echo ">>> server.exec echo ok"
EXEC=$(call_mcp server.exec "{\"server_id\":\"$AGENT_ID\",\"command\":\"echo mcp-ok\",\"timeout\":10}")
echo "  $EXEC"
echo "$EXEC" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d.get('data',{}).get('result',{});assert r.get('status')=='success' and 'mcp-ok' in r.get('stdout',''), r"
JSON_EXEC=$(curl -s -X POST -H "Authorization: Bearer $PAT" -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/mcp" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"server.exec\",\"arguments\":{\"server_id\":\"$AGENT_ID\",\"command\":\"echo jsonrpc-ok\",\"timeout\":10}}}")
echo "  jsonrpc exec: $JSON_EXEC"
echo "$JSON_EXEC" | python3 -c "import sys,json;d=json.load(sys.stdin);r=d['result']['structuredContent'];assert r['status']=='success' and 'jsonrpc-ok' in r['stdout'], r"

TMP="/tmp/xlstatus-m6-mcp-file.txt"
echo ">>> fs.write/read/list/delete"
WRITE=$(call_mcp fs.write "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\",\"content\":\"hello-mcp-file\"}")
echo "  write: $WRITE"
echo "$WRITE" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d['success'] and d['data']['result']['status']=='success'"
READ=$(call_mcp fs.read "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\",\"max_size\":1024}")
echo "  read: $READ"
echo "$READ" | python3 -c "import sys,json;d=json.load(sys.stdin);assert 'hello-mcp-file' in d['data']['result']['content']"
LS=$(call_mcp fs.list "{\"server_id\":\"$AGENT_ID\",\"path\":\"/tmp\"}")
echo "  list: $LS"
echo "$LS" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d['data']['result']['status']=='success'"
DL=$(call_mcp fs.download_url "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\"}")
echo "  download_url: $DL"
DL_URL=$(echo "$DL" | python3 -c "import sys,json;u=json.load(sys.stdin)['data']['result']['url'];assert u.startswith('/api/v1/transfers/temp/download') and 'example.com' not in u;print(u)")
DL_BODY=$(curl -s "http://127.0.0.1:$HTTP_PORT$DL_URL")
[[ "$DL_BODY" == "hello-mcp-file" ]] || { echo "FAIL: temporary download body '$DL_BODY'"; exit 1; }
RL_RESP=$(call_mcp fs.download_url "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\"}")
RL_URL=$(echo "$RL_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['result']['url'])")
UL=$(call_mcp fs.upload_url "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\"}")
echo "  upload_url: $UL"
UL_URL=$(echo "$UL" | python3 -c "import sys,json;u=json.load(sys.stdin)['data']['result']['url'];assert u.startswith('/api/v1/transfers/temp/upload') and 'example.com' not in u;print(u)")
UPLOAD_RESP=$(curl -s -X PUT --data-binary 'hello-temp-upload' "http://127.0.0.1:$HTTP_PORT$UL_URL")
echo "  temp upload: $UPLOAD_RESP"
echo "$UPLOAD_RESP" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d['success'] and d['data']['bytes_written']==17"
READ_UP=$(call_mcp fs.read "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\",\"max_size\":1024}")
echo "  read_uploaded: $READ_UP"
echo "$READ_UP" | python3 -c "import sys,json;d=json.load(sys.stdin);assert 'hello-temp-upload' in d['data']['result']['content']"
BAD_URL="${DL_URL/token=/token=bad}"
BAD_STATUS=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$HTTP_PORT$BAD_URL")
[[ "$BAD_STATUS" == "403" ]] || { echo "FAIL: bad temp token status $BAD_STATUS"; exit 1; }
RL_STATUS=200
for _ in $(seq 1 12); do
  RL_STATUS=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$HTTP_PORT$RL_URL")
done
[[ "$RL_STATUS" == "429" ]] || { echo "FAIL: temp URL rate limit status $RL_STATUS"; exit 1; }
DEL=$(call_mcp fs.delete "{\"server_id\":\"$AGENT_ID\",\"path\":\"$TMP\"}")
echo "  delete: $DEL"
echo "$DEL" | python3 -c "import sys,json;d=json.load(sys.stdin);assert d['data']['result']['status']=='success'"

echo ""
echo "M6 MCP PASS (PAT-only MCP, server exec, fs read/write/list/delete, temporary URL transfer + rate limit)"
echo "Stop with: pkill -9 -f xlstatus-server; pkill -9 -f xlstatus-agent"
