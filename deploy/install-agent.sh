#!/bin/bash
set -e

# XLStatus Agent Installation Script
# Usage: bash install-agent.sh

VERSION="${VERSION:-v0.2}"
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
BINARY_TARGET="${BINARY_TARGET:-/usr/local/bin/xlstatus-agent}"
AGENT_DATA_DIR="${AGENT_DATA_DIR:-/var/lib/xlstatus-agent}"
AGENT_WORK_DIR="${AGENT_WORK_DIR:-/opt/xlstatus-agent}"
SYSTEMD_UNIT_FILE="${SYSTEMD_UNIT_FILE:-/etc/systemd/system/xlstatus-agent.service}"
BINARY_TMP=""
CONFIG_TMP=""
UNIT_TMP=""
BINARY_BACKUP=""
CONFIG_BACKUP=""
UNIT_BACKUP=""
BINARY_CHANGED=false
CONFIG_CHANGED=false
UNIT_CHANGED=false
BINARY_EXISTED=false
CONFIG_EXISTED=false
UNIT_EXISTED=false
INSTALL_COMMITTED=false
SERVICE_WAS_ACTIVE=false
SERVICE_WAS_ENABLED=false

cleanup_staged_files() {
  if [ -n "$BINARY_TMP" ]; then
    rm -f "$BINARY_TMP"
  fi
  if [ -n "$CONFIG_TMP" ]; then
    rm -f "$CONFIG_TMP"
  fi
  if [ -n "$UNIT_TMP" ]; then
    rm -f "$UNIT_TMP"
  fi
}

cleanup_backups() {
  [ -z "$BINARY_BACKUP" ] || rm -f "$BINARY_BACKUP"
  [ -z "$CONFIG_BACKUP" ] || rm -f "$CONFIG_BACKUP"
  [ -z "$UNIT_BACKUP" ] || rm -f "$UNIT_BACKUP"
  BINARY_BACKUP=""
  CONFIG_BACKUP=""
  UNIT_BACKUP=""
}

backup_file_object() {
  local path="$1"
  local pattern="$2"
  local backup

  backup="$(mktemp "$pattern")"
  rm -f "$backup"
  cp -a "$path" "$backup"
  printf '%s' "$backup"
}

is_truthy_value() {
  case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|y|on) return 0 ;;
    *) return 1 ;;
  esac
}

restore_changed_file() {
  local path="$1"
  local backup="$2"
  local existed="$3"

  if is_truthy_value "$existed" \
    && [ -n "$backup" ] \
    && { [ -e "$backup" ] || [ -L "$backup" ]; }; then
    rm -f "$path"
    mv "$backup" "$path"
  else
    rm -f "$path"
  fi
}

systemd_value_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g'
}

reject_multiline_value() {
  local name="$1"
  local value="$2"
  if [[ "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    echo "❌ $name must not contain newline characters" >&2
    exit 1
  fi
}

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

activate_staged_binary() {
  local check_output
  local binary_magic

  chmod 755 "$BINARY_TMP"
  binary_magic="$(od -An -N4 -tx1 "$BINARY_TMP" | tr -d ' \n')"
  if [ ! -s "$BINARY_TMP" ] || [ "$binary_magic" != "7f454c46" ]; then
    echo "❌ Staged XLStatus Agent file is not a Linux ELF binary" >&2
    exit 1
  fi
  if ! check_output="$("$BINARY_TMP" --help 2>&1 >/dev/null)"; then
    echo "❌ Staged XLStatus Agent binary cannot run on this system" >&2
    if [ -n "$check_output" ]; then
      echo "$check_output" >&2
    fi
    exit 1
  fi
  if [ -e "$BINARY_TARGET" ] || [ -L "$BINARY_TARGET" ]; then
    BINARY_EXISTED=true
    BINARY_BACKUP="$(backup_file_object "$BINARY_TARGET" "$(dirname "$BINARY_TARGET")/.xlstatus-agent.backup.XXXXXX")"
  fi

  BINARY_CHANGED=true
  mv -f "$BINARY_TMP" "$BINARY_TARGET"
  BINARY_TMP=""
}

activate_staged_config() {
  if [ -e "$CONFIG_FILE" ] || [ -L "$CONFIG_FILE" ]; then
    CONFIG_EXISTED=true
    CONFIG_BACKUP="$(backup_file_object "$CONFIG_FILE" "$(dirname "$CONFIG_FILE")/.agent.json.backup.XXXXXX")"
  fi

  CONFIG_CHANGED=true
  mv -f "$CONFIG_TMP" "$CONFIG_FILE"
  CONFIG_TMP=""
}

activate_staged_unit() {
  if [ -e "$SYSTEMD_UNIT_FILE" ] || [ -L "$SYSTEMD_UNIT_FILE" ]; then
    UNIT_EXISTED=true
    UNIT_BACKUP="$(backup_file_object "$SYSTEMD_UNIT_FILE" "$(dirname "$SYSTEMD_UNIT_FILE")/.xlstatus-agent.service.backup.XXXXXX")"
  fi

  UNIT_CHANGED=true
  mv -f "$UNIT_TMP" "$SYSTEMD_UNIT_FILE"
  UNIT_TMP=""
}

load_existing_install_state() {
  SERVICE_WAS_ACTIVE=false
  SERVICE_WAS_ENABLED=false

  if systemctl is-active --quiet xlstatus-agent; then
    SERVICE_WAS_ACTIVE=true
  fi
  if systemctl is-enabled --quiet xlstatus-agent; then
    SERVICE_WAS_ENABLED=true
  fi
}

write_agent_systemd_unit() {
  local output="$1"

  cat > "$output" << EOF
[Unit]
Description=XLStatus Agent
After=network.target

[Service]
Type=simple
User=root
Group=root
WorkingDirectory="$AGENT_WORK_DIR_SYSTEMD"
ExecStart="$BINARY_TARGET_SYSTEMD" run --config "$CONFIG_FILE_SYSTEMD"
Restart=always
RestartSec=5s

# Environment
Environment="RUST_LOG=info"

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths="$AGENT_DATA_DIR_SYSTEMD" "$CONFIG_DIR_SYSTEMD"

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=xlstatus-agent

[Install]
WantedBy=multi-user.target
EOF
}

rollback_agent_install() {
  local restore_failed=false

  echo "↩ Rolling back the XLStatus Agent installation..." >&2

  if ! is_truthy_value "$SERVICE_WAS_ENABLED"; then
    if ! systemctl disable xlstatus-agent >/dev/null 2>&1 \
      && systemctl is-enabled --quiet xlstatus-agent; then
      restore_failed=true
    fi
  fi

  if is_truthy_value "$BINARY_CHANGED"; then
    restore_changed_file "$BINARY_TARGET" "$BINARY_BACKUP" "$BINARY_EXISTED" || restore_failed=true
    BINARY_BACKUP=""
    BINARY_CHANGED=false
  fi
  if is_truthy_value "$CONFIG_CHANGED"; then
    restore_changed_file "$CONFIG_FILE" "$CONFIG_BACKUP" "$CONFIG_EXISTED" || restore_failed=true
    CONFIG_BACKUP=""
    CONFIG_CHANGED=false
  fi
  if is_truthy_value "$UNIT_CHANGED"; then
    restore_changed_file "$SYSTEMD_UNIT_FILE" "$UNIT_BACKUP" "$UNIT_EXISTED" || restore_failed=true
    UNIT_BACKUP=""
    UNIT_CHANGED=false
    systemctl daemon-reload >/dev/null 2>&1 || restore_failed=true
  fi

  if is_truthy_value "$SERVICE_WAS_ENABLED"; then
    systemctl enable xlstatus-agent >/dev/null 2>&1 || restore_failed=true
  fi

  if is_truthy_value "$SERVICE_WAS_ACTIVE"; then
    if systemctl restart xlstatus-agent >/dev/null 2>&1 \
      && systemctl is-active --quiet xlstatus-agent; then
      echo "✓ Previous XLStatus Agent files restored and service recovered" >&2
    else
      echo "❌ Previous files were restored, but the XLStatus Agent service did not recover" >&2
      restore_failed=true
    fi
  elif ! systemctl stop xlstatus-agent >/dev/null 2>&1 \
    && systemctl is-active --quiet xlstatus-agent; then
    restore_failed=true
  fi

  if is_truthy_value "$restore_failed"; then
    systemctl status xlstatus-agent --no-pager >&2 || true
    journalctl -u xlstatus-agent -n 80 --no-pager >&2 || true
    return 1
  fi
}

finish_agent_install() {
  INSTALL_COMMITTED=true
  cleanup_backups
}

cleanup_agent_install() {
  local status=$?
  trap - EXIT
  set +e
  cleanup_staged_files
  if ! is_truthy_value "$INSTALL_COMMITTED"; then
    if is_truthy_value "$BINARY_CHANGED" \
      || is_truthy_value "$CONFIG_CHANGED" \
      || is_truthy_value "$UNIT_CHANGED"; then
      rollback_agent_install || true
    fi
  fi
  cleanup_backups
  exit "$status"
}

main() {

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   Installing XLStatus Agent                                  ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"

trap cleanup_agent_install EXIT

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

mkdir -p "$(dirname "$BINARY_TARGET")"
mkdir -p "$(dirname "$CONFIG_FILE")"
mkdir -p "$AGENT_DATA_DIR"
mkdir -p "$AGENT_WORK_DIR"
mkdir -p "$(dirname "$SYSTEMD_UNIT_FILE")"
load_existing_install_state

# Install agent binary
echo ""
echo "📥 Installing XLStatus Agent binary..."

if [ -n "$BINARY_PATH" ] && [ -f "$BINARY_PATH" ]; then
  BINARY_TMP="$(mktemp "$(dirname "$BINARY_TARGET")/.xlstatus-agent.XXXXXX")"
  cp "$BINARY_PATH" "$BINARY_TMP"
  activate_staged_binary
  echo "✓ Binary installed from: $BINARY_PATH"
else
  # Try to download from GitHub releases
  resolve_version
  echo "✓ Release version: $VERSION"
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-agent-linux-${ASSET_ARCH}"
  echo "   Trying to download from: $DOWNLOAD_URL"

  BINARY_TMP="$(mktemp "$(dirname "$BINARY_TARGET")/.xlstatus-agent.XXXXXX")"
  if curl -fsSL "$DOWNLOAD_URL" -o "$BINARY_TMP" 2>/dev/null; then
    activate_staged_binary
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
if ! BINARY_CHECK_OUTPUT="$("$BINARY_TARGET" --help 2>&1 >/dev/null)"; then
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
CONFIG_TMP="$(mktemp "$(dirname "$CONFIG_FILE")/.agent.json.XXXXXX")"
rm -f "$CONFIG_TMP"
ENROLL_ARGS=(
  enroll
  --server "$SERVER_URL"
  --token-stdin
  --name "$AGENT_NAME"
  --config "$CONFIG_TMP"
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
printf '%s' "$ENROLLMENT_TOKEN" | "$BINARY_TARGET" "${ENROLL_ARGS[@]}"

chown root:root "$CONFIG_TMP"
chmod 600 "$CONFIG_TMP"
activate_staged_config

# Install systemd service
echo ""
echo "🔧 Installing systemd service..."
reject_multiline_value BINARY_TARGET "$BINARY_TARGET"
reject_multiline_value CONFIG_FILE "$CONFIG_FILE"
reject_multiline_value AGENT_DATA_DIR "$AGENT_DATA_DIR"
reject_multiline_value AGENT_WORK_DIR "$AGENT_WORK_DIR"
BINARY_TARGET_SYSTEMD="$(systemd_value_escape "$BINARY_TARGET")"
CONFIG_FILE_SYSTEMD="$(systemd_value_escape "$CONFIG_FILE")"
CONFIG_DIR_SYSTEMD="$(systemd_value_escape "$(dirname "$CONFIG_FILE")")"
AGENT_DATA_DIR_SYSTEMD="$(systemd_value_escape "$AGENT_DATA_DIR")"
AGENT_WORK_DIR_SYSTEMD="$(systemd_value_escape "$AGENT_WORK_DIR")"

UNIT_TMP="$(mktemp "$(dirname "$SYSTEMD_UNIT_FILE")/.xlstatus-agent.service.XXXXXX")"
write_agent_systemd_unit "$UNIT_TMP"
chmod 644 "$UNIT_TMP"
activate_staged_unit

systemctl daemon-reload
systemctl enable xlstatus-agent

# Start agent
echo ""
echo "🚀 Starting agent..."
if systemctl is-active --quiet xlstatus-agent; then
  systemctl restart xlstatus-agent
else
  systemctl start xlstatus-agent
fi

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

finish_agent_install
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
