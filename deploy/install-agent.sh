#!/bin/bash
set -e

# XLStatus Agent Installation Script
# Usage: bash install-agent.sh

VERSION="${VERSION:-v1.0.0}"
SERVER_URL="${SERVER_URL:-http://localhost:8080}"
AGENT_NAME="${AGENT_NAME:-$(hostname)}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   Installing XLStatus Agent                                  ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"

# Check if running as root
if [ "$EUID" -ne 0 ]; then
  echo "❌ This script must be run as root"
  exit 1
fi

# Detect OS
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

if [ "$OS" != "linux" ] || [ "$ARCH" != "x86_64" ]; then
  echo "❌ Unsupported platform: $OS $ARCH"
  exit 1
fi

echo "✓ Detected: Linux x86_64"

# Install dependencies
echo ""
echo "📦 Installing dependencies..."
if command -v apt-get &> /dev/null; then
  apt-get update
  apt-get install -y curl ca-certificates procps
elif command -v yum &> /dev/null; then
  yum install -y curl ca-certificates procps-ng
fi

# Create user
echo ""
echo "👤 Creating xlstatus-agent user..."
if ! id -u xlstatus-agent &> /dev/null; then
  useradd --system --shell /bin/false xlstatus-agent
fi

# Install agent binary
echo ""
echo "📥 Installing XLStatus Agent binary..."

if [ -n "$BINARY_PATH" ] && [ -f "$BINARY_PATH" ]; then
  # User provided a binary
  cp "$BINARY_PATH" /usr/local/bin/xlstatus-agent
  chmod +x /usr/local/bin/xlstatus-agent
  echo "✓ Binary installed from: $BINARY_PATH"
else
  # Try to download from GitHub releases
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-agent-linux-x86_64"
  echo "   Trying to download from: $DOWNLOAD_URL"

  if curl -fsSL "$DOWNLOAD_URL" -o /usr/local/bin/xlstatus-agent 2>/dev/null; then
    chmod +x /usr/local/bin/xlstatus-agent
    echo "✓ Binary downloaded and installed"
  else
    echo "❌ Failed to download agent binary from GitHub releases"
    echo ""
    echo "Please either:"
    echo "  1. Build from source:"
    echo "     cd /path/to/xlstatus"
    echo "     cargo build --release --bin xlstatus-agent"
    echo "     BINARY_PATH=target/release/xlstatus-agent bash deploy/install-agent.sh"
    echo ""
    echo "  2. Or use Docker:"
    echo "     docker compose up -d"
    exit 1
  fi
fi

# Create config directory
mkdir -p /etc/xlstatus-agent
mkdir -p /var/lib/xlstatus-agent

# Get enrollment token
echo ""
echo "🔑 Please provide the enrollment token from your server:"
read -r ENROLLMENT_TOKEN

if [ -z "$ENROLLMENT_TOKEN" ]; then
  echo "❌ Enrollment token is required"
  exit 1
fi

# Create config
cat > /etc/xlstatus-agent/config.toml << EOF
server_url = "$SERVER_URL"
agent_name = "$AGENT_NAME"
enrollment_token = "$ENROLLMENT_TOKEN"

[logging]
level = "info"
EOF

chown xlstatus-agent:xlstatus-agent /etc/xlstatus-agent/config.toml
chmod 600 /etc/xlstatus-agent/config.toml

# Install systemd service
echo ""
echo "🔧 Installing systemd service..."
cat > /etc/systemd/system/xlstatus-agent.service << 'EOF'
[Unit]
Description=XLStatus Agent
After=network.target

[Service]
Type=simple
User=xlstatus-agent
Group=xlstatus-agent
ExecStart=/usr/local/bin/xlstatus-agent
Restart=on-failure
RestartSec=5s

Environment="RUST_LOG=info"

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable xlstatus-agent

# Start agent
echo ""
echo "🚀 Starting agent..."
systemctl start xlstatus-agent

sleep 2

if systemctl is-active --quiet xlstatus-agent; then
  echo ""
  echo "╔══════════════════════════════════════════════════════════════╗"
  echo "║                                                              ║"
  echo "║   ✅ XLStatus Agent installed successfully!                  ║"
  echo "║                                                              ║"
  echo "╚══════════════════════════════════════════════════════════════╝"
  echo ""
  echo "📝 Agent name: $AGENT_NAME"
  echo "🌐 Server URL: $SERVER_URL"
  echo ""
  echo "📝 Useful commands:"
  echo "   - Status:  systemctl status xlstatus-agent"
  echo "   - Logs:    journalctl -u xlstatus-agent -f"
  echo "   - Restart: systemctl restart xlstatus-agent"
  echo ""
else
  echo "❌ Failed to start agent"
  echo "   Check logs: journalctl -u xlstatus-agent -f"
  exit 1
fi
