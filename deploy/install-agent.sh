#!/bin/bash
set -e

# XLStatus Agent Installation Script
# Usage: bash install-agent.sh

VERSION="${VERSION:-v0.1.0-alpha.3}"
SERVER_URL="${SERVER_URL:-http://localhost:8080}"
GRPC_SERVER="${GRPC_SERVER:-}"
GRPC_TLS_CA_PATH="${GRPC_TLS_CA_PATH:-}"
GRPC_TLS_DOMAIN_NAME="${GRPC_TLS_DOMAIN_NAME:-}"
GRPC_TLS_CLIENT_CERT_PATH="${GRPC_TLS_CLIENT_CERT_PATH:-}"
GRPC_TLS_CLIENT_KEY_PATH="${GRPC_TLS_CLIENT_KEY_PATH:-}"
AGENT_NAME="${AGENT_NAME:-$(hostname)}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path
CONFIG_FILE="${CONFIG_FILE:-/etc/xlstatus-agent/agent.json}"
ENROLLMENT_TOKEN="${ENROLLMENT_TOKEN:-}"

normalize_arch() {
  case "$1" in
    x86_64|amd64) printf 'x86_64' ;;
    aarch64|arm64) printf 'arm64' ;;
    i386|i486|i586|i686) printf 'i386' ;;
    *) return 1 ;;
  esac
}

resolve_version() {
  if [ "$VERSION" != "latest" ]; then
    return
  fi
  local api_url="https://api.github.com/repos/lbyxiaolizi/XLStatus/releases?per_page=20"
  local latest
  latest="$(curl -fsSL "$api_url" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  if [ -z "$latest" ]; then
    echo "❌ Failed to resolve latest GitHub Release"
    exit 1
  fi
  VERSION="$latest"
}

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

if [ "$OS" != "linux" ]; then
  echo "❌ Unsupported OS for the systemd installer: $OS"
  exit 1
fi

if ! ASSET_ARCH="$(normalize_arch "$ARCH")"; then
  echo "❌ Unsupported architecture: $ARCH"
  exit 1
fi

echo "✓ Detected: Linux $ASSET_ARCH"

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
  resolve_version
  echo "✓ Release version: $VERSION"
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-agent-linux-${ASSET_ARCH}"
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

echo ""
echo "🔎 Validating agent binary..."
if ! BINARY_CHECK_OUTPUT="$(/usr/local/bin/xlstatus-agent --help 2>&1 >/dev/null)"; then
  echo "❌ Installed agent binary cannot run on this system"
  if [ -n "$BINARY_CHECK_OUTPUT" ]; then
    echo "$BINARY_CHECK_OUTPUT"
  fi
  echo ""
  echo "This usually means the release binary requires a newer glibc than this Linux distribution provides."
  echo "Try a newer XLStatus release or build the agent from source on this host:"
  echo "  cargo build --release --bin xlstatus-agent"
  echo "  BINARY_PATH=target/release/xlstatus-agent bash deploy/install-agent.sh"
  exit 1
fi
echo "✓ Binary is runnable"

# Create config directory
mkdir -p "$(dirname "$CONFIG_FILE")"
mkdir -p /var/lib/xlstatus-agent

# Get enrollment token
if [ -z "$ENROLLMENT_TOKEN" ]; then
  echo ""
  echo "🔑 Please provide the enrollment token from your server:"
  read -r ENROLLMENT_TOKEN
fi

if [ -z "$ENROLLMENT_TOKEN" ]; then
  echo "❌ Enrollment token is required"
  exit 1
fi

# Enroll and create config. The agent writes JSON and stores the Ed25519 keypair.
echo ""
echo "🔐 Enrolling agent..."
ENROLL_ARGS=(
  enroll
  --server "$SERVER_URL"
  --token-stdin
  --name "$AGENT_NAME"
  --config "$CONFIG_FILE"
)
if [ -n "$GRPC_SERVER" ]; then
  ENROLL_ARGS+=(--grpc-server "$GRPC_SERVER")
fi
if [ -n "$GRPC_TLS_CA_PATH" ]; then
  ENROLL_ARGS+=(--grpc-tls-ca-path "$GRPC_TLS_CA_PATH")
fi
if [ -n "$GRPC_TLS_DOMAIN_NAME" ]; then
  ENROLL_ARGS+=(--grpc-tls-domain-name "$GRPC_TLS_DOMAIN_NAME")
fi
if [ -n "$GRPC_TLS_CLIENT_CERT_PATH" ]; then
  ENROLL_ARGS+=(--grpc-tls-client-cert-path "$GRPC_TLS_CLIENT_CERT_PATH")
fi
if [ -n "$GRPC_TLS_CLIENT_KEY_PATH" ]; then
  ENROLL_ARGS+=(--grpc-tls-client-key-path "$GRPC_TLS_CLIENT_KEY_PATH")
fi
printf '%s' "$ENROLLMENT_TOKEN" | /usr/local/bin/xlstatus-agent "${ENROLL_ARGS[@]}"

chown root:root "$CONFIG_FILE"
chmod 600 "$CONFIG_FILE"

# Install systemd service
echo ""
echo "🔧 Installing systemd service..."
cat > /etc/systemd/system/xlstatus-agent.service << EOF
[Unit]
Description=XLStatus Agent
After=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=/usr/local/bin/xlstatus-agent run --config $CONFIG_FILE
Restart=always
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
  echo ""
  systemctl status xlstatus-agent --no-pager || true
  echo ""
  journalctl -u xlstatus-agent -n 80 --no-pager || true
  exit 1
fi
