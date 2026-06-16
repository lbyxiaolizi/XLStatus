#!/bin/bash
set -e

# XLStatus Installation Script
# Usage: bash install.sh

VERSION="${VERSION:-v1.0.0}"
INSTALL_DIR="${INSTALL_DIR:-/opt/xlstatus}"
DATA_DIR="${DATA_DIR:-/var/lib/xlstatus}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path

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
cat > /etc/xlstatus.toml << EOF
[database]
url = "sqlite://$DATA_DIR/xlstatus.db"

[server]
bind_address = "0.0.0.0:8080"
grpc_address = "0.0.0.0:50051"

[logging]
level = "info"
EOF

chown xlstatus:xlstatus /etc/xlstatus.toml
chmod 600 /etc/xlstatus.toml
echo "✓ Configuration created"

# Install systemd service
echo ""
echo "🔧 Installing systemd service..."
cat > /etc/systemd/system/xlstatus.service << 'EOF'
[Unit]
Description=XLStatus Server
After=network.target

[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory=/opt/xlstatus
ExecStart=/usr/local/bin/xlstatus-server
Restart=on-failure
RestartSec=5s

Environment="DATABASE_URL=sqlite:///var/lib/xlstatus/xlstatus.db"
Environment="RUST_LOG=info"

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

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

# Create default admin user
echo ""
echo "👤 Creating default admin user..."
echo "   Username: admin"
echo "   Password: admin123 (please change this!)"

# Get server info
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   ✅ XLStatus Server installed successfully!                 ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "📍 Dashboard URL: http://$(hostname -I | awk '{print $1}'):8080"
echo "🔑 Default login: admin / admin123"
echo ""
echo "📝 Useful commands:"
echo "   - Start:   systemctl start xlstatus"
echo "   - Stop:    systemctl stop xlstatus"
echo "   - Status:  systemctl status xlstatus"
echo "   - Logs:    journalctl -u xlstatus -f"
echo ""
echo "📚 Documentation: https://docs.xlstatus.io"
echo ""
