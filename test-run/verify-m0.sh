#!/usr/bin/env bash
# M0 verification: 3 services start, healthz + grpcurl + web page all return.
# Run with: bash test-run/verify-m0.sh
# After it finishes, services keep running. Stop with the cleanup at the end.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18091}"
GRPC_PORT="${GRPC_PORT:-15061}"
WEB_PORT="${WEB_PORT:-3017}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m0}"

source "$ROOT/test-run/lib/test-processes.sh"
test_process_init "$ROOT" "$0"

mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR/x.db" "$LOG_DIR/server.log" "$LOG_DIR/web.log" "$LOG_DIR/web.html"

echo ">>> build"
(cd "$ROOT" && cargo build -p xlstatus-server -p xlstatus-agent 2>&1 | tail -3)

cat > "$LOG_DIR/cfg.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"
# Reflection is off by default; this script asserts service discovery works.
grpc_reflection_enabled = true

[database]
url = "sqlite://$LOG_DIR/x.db?mode=rwc"

[security]
session_secret = "test-secret-key-for-development-only-2026"
session_ttl_hours = 24
TOML

# Use a dedicated shell so we can exec the long-lived process inside a tty session
echo ">>> start server (port $HTTP_PORT / gRPC $GRPC_PORT)"
test_process_start "$TEST_PROCESS_REGISTRY" server "$LOG_DIR/server.log" \
  env CONFIG_FILE="$LOG_DIR/cfg.toml" \
  XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
  "$ROOT/target/debug/xlstatus-server"
SRV_PID="$TEST_PROCESS_PID"
sleep 5

HEALTH=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$HTTP_PORT/healthz")
if [[ "$HEALTH" != "200" ]]; then
  echo "FAIL: /healthz=$HEALTH"
  tail -20 "$LOG_DIR/server.log"
  exit 1
fi
echo "OK: /healthz = $HEALTH"

SERVICES=$(grpcurl -plaintext "127.0.0.1:$GRPC_PORT" list 2>&1)
if ! echo "$SERVICES" | grep -q "xlstatus.v1.AgentService"; then
  echo "FAIL: grpcurl missing AgentService"
  echo "$SERVICES"
  exit 1
fi
if ! echo "$SERVICES" | grep -q "xlstatus.v1.NatTunnel"; then
  echo "FAIL: grpcurl missing NatTunnel"
  echo "$SERVICES"
  exit 1
fi
echo "OK: grpcurl reflection lists AgentService + NatTunnel"

echo ">>> start web (port $WEB_PORT)"
test_process_start "$TEST_PROCESS_REGISTRY" web "$LOG_DIR/web.log" \
  bash -c 'cd "$1" && exec pnpm dev -p "$2"' _ "$ROOT/web" "$WEB_PORT"
WEB_PID="$TEST_PROCESS_PID"

# Wait for web to be ready
WEB=000
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  # Follow the root redirect: / issues a server-side 307 to /status.
  WEB=$(curl -sL -o "$LOG_DIR/web.html" -w '%{http_code}' "http://127.0.0.1:$WEB_PORT/" 2>/dev/null || echo 000)
  if [[ "$WEB" == "200" ]]; then break; fi
  sleep 1
done
if [[ "$WEB" != "200" ]]; then
  echo "FAIL: web status=$WEB after 20s"
  tail -20 "$LOG_DIR/web.log"
  exit 1
fi
if ! grep -q "XLStatus" "$LOG_DIR/web.html"; then
  echo "FAIL: page missing XLStatus heading"
  head -c 500 "$LOG_DIR/web.html"
  exit 1
fi
echo "OK: web page = 200 (contains XLStatus)"

echo ""
echo "M0 PASS"
echo "Server supervisor PID: $SRV_PID"
echo "Web supervisor PID:    $WEB_PID"
echo "Logs in:       $LOG_DIR"
test_process_print_cleanup "$TEST_PROCESS_REGISTRY"
