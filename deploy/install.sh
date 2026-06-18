#!/bin/bash
set -e

# XLStatus Installation Script
# Usage: bash install.sh

VERSION="${VERSION:-v1.0.0}"
INSTALL_DIR="${INSTALL_DIR:-/opt/xlstatus}"
DATA_DIR="${DATA_DIR:-/var/lib/xlstatus}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path
CONFIG_FILE="${CONFIG_FILE:-/etc/xlstatus/server.toml}"
HTTP_BIND="${HTTP_BIND:-0.0.0.0:8080}"
GRPC_BIND="${GRPC_BIND:-0.0.0.0:50051}"
DATABASE_URL="${DATABASE_URL:-sqlite://$DATA_DIR/xlstatus.db?mode=rwc}"
DATABASE_CREATE_IF_MISSING="${DATABASE_CREATE_IF_MISSING:-true}"
SESSION_SECRET="${SESSION_SECRET:-}"
ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   Installing XLStatus Server                                 ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# Check if running as root
if [ "$EUID" -ne 0 ]; then
  echo "❌ This script must be run as root"
  exit 1
fi

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

if [ "$OS" != "linux" ]; then
  echo "❌ Unsupported OS: $OS (only Linux is supported)"
  exit 1
fi

if [ "$ARCH" != "x86_64" ]; then
  echo "❌ Unsupported architecture: $ARCH (only x86_64 is supported)"
  exit 1
fi

echo "✓ Detected: Linux x86_64"

# Install dependencies
echo ""
echo "📦 Installing dependencies..."
if command -v apt-get &> /dev/null; then
  apt-get update
  apt-get install -y curl ca-certificates sqlite3
elif command -v yum &> /dev/null; then
  yum install -y curl ca-certificates sqlite
else
  echo "❌ Unsupported package manager"
  exit 1
fi

# Create user
echo ""
echo "👤 Creating xlstatus user..."
if ! id -u xlstatus &> /dev/null; then
  useradd --system --shell /bin/false --home-dir "$DATA_DIR" xlstatus
  echo "✓ User created"
else
  echo "✓ User already exists"
fi

# Create directories
echo ""
echo "📁 Creating directories..."
mkdir -p "$INSTALL_DIR"
mkdir -p "$DATA_DIR"
mkdir -p "$(dirname "$CONFIG_FILE")"
chown xlstatus:xlstatus "$DATA_DIR"
echo "✓ Directories created"

# Install binary
echo ""
echo "📥 Installing XLStatus Server binary..."

if [ -n "$BINARY_PATH" ] && [ -f "$BINARY_PATH" ]; then
  # User provided a binary
  cp "$BINARY_PATH" "$INSTALL_DIR/xlstatus-server"
  chmod +x "$INSTALL_DIR/xlstatus-server"
  ln -sf "$INSTALL_DIR/xlstatus-server" /usr/local/bin/xlstatus-server
  echo "✓ Binary installed from: $BINARY_PATH"
else
  # Try to download from GitHub releases
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-server-linux-x86_64"
  echo "   Trying to download from: $DOWNLOAD_URL"

  if curl -fsSL "$DOWNLOAD_URL" -o "$INSTALL_DIR/xlstatus-server" 2>/dev/null; then
    chmod +x "$INSTALL_DIR/xlstatus-server"
    ln -sf "$INSTALL_DIR/xlstatus-server" /usr/local/bin/xlstatus-server
    echo "✓ Binary downloaded and installed"
  else
    echo "❌ Failed to download binary from GitHub releases"
    echo ""
    echo "Please either:"
    echo "  1. Build from source:"
    echo "     cd /path/to/xlstatus"
    echo "     cargo build --release --bin xlstatus-server"
    echo "     BINARY_PATH=target/release/xlstatus-server bash deploy/install.sh"
    echo ""
    echo "  2. Or use Docker:"
    echo "     docker compose up -d"
    exit 1
  fi
fi

# Create config
echo ""
echo "⚙️  Creating configuration..."
if [ -z "$SESSION_SECRET" ]; then
  if command -v openssl >/dev/null 2>&1; then
    SESSION_SECRET="$(openssl rand -hex 32)"
  else
    SESSION_SECRET="$(date +%s)-$(hostname)-change-me"
  fi
fi

cat > "$CONFIG_FILE" << EOF
[database]
url = "$DATABASE_URL"
create_if_missing = $DATABASE_CREATE_IF_MISSING

[server]
http_bind = "$HTTP_BIND"
grpc_bind = "$GRPC_BIND"

[security]
session_secret = "$SESSION_SECRET"
session_ttl_hours = 24
EOF

chown xlstatus:xlstatus "$CONFIG_FILE"
chmod 600 "$CONFIG_FILE"
echo "✓ Configuration created"

# Install systemd service
echo ""
echo "🔧 Installing systemd service..."
cat > /etc/systemd/system/xlstatus.service << EOF
[Unit]
Description=XLStatus Server
After=network.target

[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory=/var/lib/xlstatus
ExecStart=/usr/local/bin/xlstatus-server
Restart=on-failure
RestartSec=5s

Environment="CONFIG_FILE=/etc/xlstatus/server.toml"
Environment="RUST_LOG=info"
Environment="XLSTATUS_SEED_ADMIN_USERNAME=admin"
$(if [ -n "$ADMIN_PASSWORD" ]; then printf 'Environment="XLSTATUS_SEED_ADMIN_PASSWORD=%s"\n' "$ADMIN_PASSWORD"; fi)

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF
sed -i.bak "s|CONFIG_FILE=/etc/xlstatus/server.toml|CONFIG_FILE=$CONFIG_FILE|" /etc/systemd/system/xlstatus.service
sed -i.bak "s|XLSTATUS_SEED_ADMIN_USERNAME=admin|XLSTATUS_SEED_ADMIN_USERNAME=$ADMIN_USERNAME|" /etc/systemd/system/xlstatus.service

systemctl daemon-reload
systemctl enable xlstatus
echo "✓ Systemd service installed"

# Start service
echo ""
echo "🚀 Starting XLStatus..."
systemctl start xlstatus

# Wait for service to be ready
sleep 3

if systemctl is-active --quiet xlstatus; then
  echo "✓ XLStatus is running"
else
  echo "❌ Failed to start XLStatus"
  echo "   Check logs: journalctl -u xlstatus -f"
  exit 1
fi

echo ""
echo "👤 Admin bootstrap:"
echo "   Username: $ADMIN_USERNAME"
if [ -n "$ADMIN_PASSWORD" ]; then
  echo "   Password: provided through ADMIN_PASSWORD"
else
  echo "   Password: not seeded; set ADMIN_PASSWORD before first start to auto-create an admin"
fi

# Get server info
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   ✅ XLStatus Server installed successfully!                 ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "📍 Dashboard URL: http://$(hostname -I | awk '{print $1}'):8080"
echo "🔑 Admin user: $ADMIN_USERNAME"
echo ""
echo "📝 Useful commands:"
echo "   - Start:   systemctl start xlstatus"
echo "   - Stop:    systemctl stop xlstatus"
echo "   - Status:  systemctl status xlstatus"
echo "   - Logs:    journalctl -u xlstatus -f"
echo ""
echo "📚 Documentation: https://docs.xlstatus.io"
echo ""
