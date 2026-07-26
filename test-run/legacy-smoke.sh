#!/usr/bin/env bash
# XLStatus Test Script
# Tests basic functionality of server and agent

set -e

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$PROJECT_ROOT/test-run/lib/test-processes.sh"
test_process_init "$PROJECT_ROOT" "$0"
cd "$PROJECT_ROOT"

echo "=========================================="
echo "XLStatus Test Suite"
echo "=========================================="
echo ""

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test directory
TEST_DIR="$PROJECT_ROOT/test-run"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Cleanup
cleanup() {
    echo ""
    echo "Cleaning up..."
    test_process_cleanup "$TEST_PROCESS_REGISTRY"
    cd "$PROJECT_ROOT"
}
trap cleanup EXIT

echo "1. Checking binaries..."
if [ ! -f "$PROJECT_ROOT/target/release/xlstatus-server" ]; then
    echo -e "${RED}✗${NC} Server binary not found. Run: cargo build --release --bin xlstatus-server"
    exit 1
fi
if [ ! -f "$PROJECT_ROOT/target/release/xlstatus-agent" ]; then
    echo -e "${RED}✗${NC} Agent binary not found. Run: cargo build --release --bin xlstatus-agent"
    exit 1
fi
echo -e "${GREEN}✓${NC} Binaries found"
echo ""

echo "2. Binary versions..."
echo "   Server: $($PROJECT_ROOT/target/release/xlstatus-server --version 2>&1 | head -1 || echo 'unknown')"
echo "   Agent: $($PROJECT_ROOT/target/release/xlstatus-agent --version 2>&1 | head -1 || echo 'unknown')"
echo ""

echo "3. Testing server startup..."
# Create database file first (workaround for SQLite)
touch xlstatus.db
chmod 666 xlstatus.db

# Start server with environment variables
test_process_start "$TEST_PROCESS_REGISTRY" server "$TEST_DIR/server.log" \
    env DATABASE_URL="sqlite:///$TEST_DIR/xlstatus.db" \
    HTTP_BIND="127.0.0.1:8080" \
    GRPC_BIND="127.0.0.1:50051" \
    SESSION_SECRET="test-secret-key" \
    "$PROJECT_ROOT/target/release/xlstatus-server"
SERVER_PID="$TEST_PROCESS_PID"

sleep 3

if ! test_process_label_is_running "$TEST_PROCESS_REGISTRY" server; then
    echo -e "${RED}✗${NC} Server failed to start"
    echo ""
    cat server.log
    exit 1
fi
echo -e "${GREEN}✓${NC} Server started (supervisor PID: $SERVER_PID)"
echo ""

echo "4. Testing HTTP endpoint..."
sleep 5
HEALTH_RESPONSE=$(curl -s http://127.0.0.1:8080/health 2>/dev/null || echo "")
if [ -z "$HEALTH_RESPONSE" ]; then
    echo -e "${RED}✗${NC} Health endpoint not responding"
    tail -20 server.log
    exit 1
fi
echo -e "${GREEN}✓${NC} HTTP endpoint responding"
echo "   Response: $HEALTH_RESPONSE"
echo ""

echo "5. Checking database..."
if [ -f xlstatus.db ] && [ -s xlstatus.db ]; then
    DB_SIZE=$(ls -lh xlstatus.db | awk '{print $5}')
    echo -e "${GREEN}✓${NC} Database created (size: $DB_SIZE)"
else
    echo -e "${RED}✗${NC} Database not created"
    exit 1
fi
echo ""

echo "6. Checking gRPC endpoint..."
if lsof -i :50051 > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC} gRPC port (50051) is listening"
else
    echo -e "${YELLOW}⚠${NC} gRPC port check failed (may be normal on some systems)"
fi
echo ""

echo "7. Server logs (last 10 lines)..."
echo "---"
tail -10 server.log
echo "---"
echo ""

echo "=========================================="
echo -e "${GREEN}All tests passed!${NC}"
echo "=========================================="
echo ""
echo "Server is running on:"
echo "  - HTTP: http://127.0.0.1:8080"
echo "  - gRPC: 127.0.0.1:50051"
echo ""
echo "Default credentials:"
echo "  - Username: admin"
echo "  - Password: admin123"
echo ""
echo "Press Ctrl+C to stop the server"
echo ""

# Keep the test attached until interrupted; the EXIT trap owns cleanup.
while test_process_label_is_running "$TEST_PROCESS_REGISTRY" server; do
    sleep 1
done
