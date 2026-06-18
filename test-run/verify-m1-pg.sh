#!/usr/bin/env bash
# M1 verification: identical repository behavior on SQLite AND PostgreSQL.
# 1) Boots a temporary PostgreSQL container, lets the server run migrations,
#    seeds admin, and logs in.
# 2) Then runs the same /api/v1/auth/login flow against SQLite.
#
# Pass criteria: both backends return 200 for valid login and 401 for wrong pw.

set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
HTTP_PORT="${HTTP_PORT:-18091}"
GRPC_PORT="${GRPC_PORT:-15061}"
LOG_DIR="${LOG_DIR:-/tmp/xls-m1}"
PG_PORT="${PG_PORT:-55432}"

mkdir -p "$LOG_DIR"

# -----------------------------------------------------------------
# PostgreSQL
# -----------------------------------------------------------------
echo ">>> postgres container"
if ! docker ps --format '{{.Names}}' | grep -q "^xls-pg\$"; then
  docker rm -f xls-pg 2>/dev/null || true
  docker run -d --name xls-pg -p "${PG_PORT}:5432" \
    -e POSTGRES_USER=xlstatus -e POSTGRES_PASSWORD=xlstatus -e POSTGRES_DB=xlstatus \
    postgres:15 >/dev/null
fi
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if docker exec xls-pg pg_isready -U xlstatus 2>/dev/null | grep -q "accepting"; then
    echo "pg ready"
    break
  fi
  sleep 1
done
docker exec xls-pg psql -U xlstatus -d xlstatus -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null

cat > "$LOG_DIR/cfg-pg.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"

[database]
url = "postgres://xlstatus:xlstatus@127.0.0.1:$PG_PORT/xlstatus"

[security]
session_secret = "test-secret-key-for-development-only-2026"
session_ttl_hours = 24
TOML

pkill -9 -f xlstatus-server 2>/dev/null || true
sleep 1
echo ">>> start server (postgres)"
nohup env CONFIG_FILE="$LOG_DIR/cfg-pg.toml" \
  XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
  "$ROOT/target/debug/xlstatus-server" \
  > "$LOG_DIR/pg-server.log" 2>&1 < /dev/null &
disown $!
sleep 5

# Valid login -> 200
PG_OK=$(curl -s -o "$LOG_DIR/pg-login.json" -w '%{http_code}' \
  -X POST -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"admin-pw"}')
if [[ "$PG_OK" != "200" ]]; then
  echo "FAIL: postgres login HTTP $PG_OK"
  tail -20 "$LOG_DIR/pg-server.log"
  exit 1
fi
echo "OK: postgres login = 200"

# Wrong password -> 401
PG_BAD=$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"wrong"}')
[[ "$PG_BAD" == "401" ]] || { echo "FAIL: postgres wrong-pw HTTP $PG_BAD"; exit 1; }
echo "OK: postgres wrong-pw = 401"

pkill -9 -f xlstatus-server 2>/dev/null || true
sleep 1

# -----------------------------------------------------------------
# SQLite (sanity check same flow)
# -----------------------------------------------------------------
rm -f "$LOG_DIR/x.db"
cat > "$LOG_DIR/cfg-sq.toml" <<TOML
[server]
http_bind = "127.0.0.1:$HTTP_PORT"
grpc_bind = "127.0.0.1:$GRPC_PORT"

[database]
url = "sqlite://$LOG_DIR/x.db?mode=rwc"

[security]
session_secret = "test-secret-key-for-development-only-2026"
session_ttl_hours = 24
TOML

echo ">>> start server (sqlite)"
nohup env CONFIG_FILE="$LOG_DIR/cfg-sq.toml" \
  XLSTATUS_SEED_ADMIN_USERNAME=admin XLSTATUS_SEED_ADMIN_PASSWORD=admin-pw \
  "$ROOT/target/debug/xlstatus-server" \
  > "$LOG_DIR/sq-server.log" 2>&1 < /dev/null &
disown $!
sleep 4

SQ_OK=$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"admin-pw"}')
[[ "$SQ_OK" == "200" ]] || { echo "FAIL: sqlite login HTTP $SQ_OK"; tail -20 "$LOG_DIR/sq-server.log"; exit 1; }
echo "OK: sqlite login = 200"

SQ_BAD=$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST -H "Content-Type: application/json" \
  "http://127.0.0.1:$HTTP_PORT/api/v1/auth/login" \
  -d '{"username":"admin","password":"wrong"}')
[[ "$SQ_BAD" == "401" ]] || { echo "FAIL: sqlite wrong-pw HTTP $SQ_BAD"; exit 1; }
echo "OK: sqlite wrong-pw = 401"

echo ""
echo "M1 PASS (SQLite + PostgreSQL identical behavior on auth)"
echo "Server PID: $(pgrep -f xlstatus-server | head -1)"
echo "Stop with:  pkill -9 -f xlstatus-server"
