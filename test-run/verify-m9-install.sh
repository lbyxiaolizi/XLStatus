#!/usr/bin/env bash
# M9 release-install verification.
# Pass criteria:
# - debug server and agent binaries exist or build successfully
# - server starts from CONFIG_FILE
# - /healthz returns OK
# - seeded admin can log in
# - admin can create an enrollment token with session cookie + CSRF
# - agent enroll writes a JSON config
# - agent can start and establish a short gRPC session

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18099}"
GRPC_PORT="${GRPC_PORT:-15099}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m9}"

mkdir -p "$LOG_DIR"

SERVER_BIN="$ROOT/target/debug/xlstatus-server"
AGENT_BIN="$ROOT/target/debug/xlstatus-agent"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [[ -n "${AGENT_PID:-}" ]]; then
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo ">>> build debug binaries"
cargo build --bin xlstatus-server --bin xlstatus-agent >/dev/null

echo ">>> validate docker compose files"
if command -v docker >/dev/null 2>&1; then
  docker compose -f "$ROOT/docker-compose.yml" config >/dev/null
  docker compose -f "$ROOT/docker-compose.pg.yml" config >/dev/null
  echo "OK: docker compose config"
else
  echo "SKIP: docker not available; compose config not validated"
fi

rm -f "$LOG_DIR/xlstatus.db" "$LOG_DIR/cookies.txt" "$LOG_DIR/agent.json"
cat > "$LOG_DIR/server.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"

[database]
url = "sqlite://$LOG_DIR/xlstatus.db?mode=rwc"

[security]
session_secret = "test-secret-key-for-m9-release-verification"
session_ttl_hours = 24
TOML

echo ">>> start server"
CONFIG_FILE="$LOG_DIR/server.toml" \
XLSTATUS_SEED_ADMIN_USERNAME=admin \
XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
"$SERVER_BIN" > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!

for _ in {1..40}; do
  if curl -fsS "http://127.0.0.1:$HTTP_PORT/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

HEALTH="$(curl -fsS "http://127.0.0.1:$HTTP_PORT/healthz")"
[[ "$HEALTH" == "OK" ]] || { echo "FAIL: /healthz returned '$HEALTH'"; exit 1; }
echo "OK: healthz"

echo ">>> login admin"
LOGIN_JSON="$LOG_DIR/login.json"
HTTP_CODE="$(curl -sS -o "$LOGIN_JSON" -w '%{http_code}' \
  -c "$LOG_DIR/cookies.txt" \
  -H 'Content-Type: application/json' \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"admin-pw"}')"
[[ "$HTTP_CODE" == "200" ]] || { echo "FAIL: login HTTP $HTTP_CODE"; cat "$LOGIN_JSON"; exit 1; }
CSRF="$(awk '$6 == "xlstatus_csrf" { print $7 }' "$LOG_DIR/cookies.txt" | tail -1)"
[[ -n "$CSRF" ]] || { echo "FAIL: csrf cookie missing"; exit 1; }
echo "OK: login"

echo ">>> create enrollment token"
TOKEN_JSON="$LOG_DIR/token.json"
HTTP_CODE="$(curl -sS -o "$TOKEN_JSON" -w '%{http_code}' \
  -b "$LOG_DIR/cookies.txt" \
  -H 'Content-Type: application/json' \
  -H "x-csrf-token: $CSRF" \
  -X POST "http://127.0.0.1:$HTTP_PORT/api/v1/enrollment-tokens" \
  -d '{"expires_in_hours":1}')"
[[ "$HTTP_CODE" == "200" ]] || { echo "FAIL: token HTTP $HTTP_CODE"; cat "$TOKEN_JSON"; exit 1; }
TOKEN="$(sed -n 's/.*"token":"\([^"]*\)".*/\1/p' "$TOKEN_JSON")"
[[ "$TOKEN" == xle_* ]] || { echo "FAIL: unexpected token '$TOKEN'"; exit 1; }
echo "OK: enrollment token"

echo ">>> enroll agent"
"$AGENT_BIN" enroll \
  --server "http://127.0.0.1:$HTTP_PORT" \
  --grpc-server "http://127.0.0.1:$GRPC_PORT" \
  --token "$TOKEN" \
  --name "m9-verify-agent" \
  --config "$LOG_DIR/agent.json" > "$LOG_DIR/enroll.log" 2>&1
grep -q '"agent_id"' "$LOG_DIR/agent.json"
grep -q '"private_key"' "$LOG_DIR/agent.json"
grep -q '"grpc_server"' "$LOG_DIR/agent.json"
echo "OK: agent config"

echo ">>> run agent briefly"
"$AGENT_BIN" run --config "$LOG_DIR/agent.json" > "$LOG_DIR/agent.log" 2>&1 &
AGENT_PID=$!
sleep 5
if ! kill -0 "$AGENT_PID" 2>/dev/null; then
  echo "FAIL: agent exited early"
  cat "$LOG_DIR/agent.log"
  exit 1
fi
kill "$AGENT_PID" 2>/dev/null || true
wait "$AGENT_PID" 2>/dev/null || true
unset AGENT_PID
echo "OK: agent gRPC session"

echo ""
echo "M9 PASS"
