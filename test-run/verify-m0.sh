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

mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR/x.db" "$LOG_DIR/server.log" "$LOG_DIR/web.log" "$LOG_DIR/web.html"

echo ">>> build"
(cd "$ROOT" && cargo build -p xlstatus-server -p xlstatus-agent 2>&1 | tail -3)

cat > "$LOG_DIR/cfg.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"

[database]
url = "sqlite://$LOG_DIR/x.db?mode=rwc"

[security]
session_secret = "test-secret-key-for-development-only-2026"
session_ttl_hours = 24
TOML

# Kill any leftover processes
pkill -9 -f xlstatus-server 2>/dev/null || true
pkill -9 -f "next dev" 2>/dev/null || true
sleep 1

# Use a dedicated shell so we can exec the long-lived process inside a tty session
echo ">>> start server (port $HTTP_PORT / gRPC $GRPC_PORT)"
nohup env CONFIG_FILE="$LOG_DIR/cfg.toml" \
  XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
  "$ROOT/target/debug/xlstatus-server" \
  > "$LOG_DIR/server.log" 2>&1 < /dev/null &
SRV_PID=$!
disown $SRV_PID
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
(cd "$ROOT/web" && nohup pnpm dev -p "$WEB_PORT" > "$LOG_DIR/web.log" 2>&1 < /dev/null &)
disown 2>/dev/null || true

# Wait for web to be ready
WEB=000
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  WEB=$(curl -s -o "$LOG_DIR/web.html" -w '%{http_code}' "http://127.0.0.1:$WEB_PORT/" 2>/dev/null || echo 000)
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
echo "Server PID:    $(pgrep -f xlstatus-server | head -1)"
echo "Web PID:       $(pgrep -f 'next dev' | head -1)"
echo "Logs in:       $LOG_DIR"
echo "Stop with:     pkill -9 -f xlstatus-server; pkill -9 -f 'next dev'"
