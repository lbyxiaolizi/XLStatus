#!/bin/bash
set -e

# XLStatus Installation Script
# Usage: bash install.sh

VERSION="${VERSION:-v0.1.0-alpha.3}"
INSTALL_DIR="${INSTALL_DIR:-/opt/xlstatus}"
DATA_DIR="${DATA_DIR:-/var/lib/xlstatus}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path
CONFIG_FILE="${CONFIG_FILE:-/etc/xlstatus/server.toml}"
HTTP_BIND="${HTTP_BIND:-0.0.0.0:8080}"
GRPC_BIND="${GRPC_BIND:-0.0.0.0:50051}"
DATABASE_URL="${DATABASE_URL:-sqlite://$DATA_DIR/xlstatus.db?mode=rwc}"
DATABASE_CREATE_IF_MISSING="${DATABASE_CREATE_IF_MISSING:-true}"
CORS_ALLOWED_ORIGINS="${CORS_ALLOWED_ORIGINS:-http://localhost:3000,http://127.0.0.1:3000}"
SESSION_SECRET="${SESSION_SECRET:-}"
ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
INSTALL_DEPS="${INSTALL_DEPS:-true}"
START_SERVICE="${START_SERVICE:-true}"
INTERACTIVE="${INTERACTIVE:-auto}"

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

trim() {
  local value="$*"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

toml_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

cors_origins_toml() {
  local csv="$1"
  local first=1
  local result="["
  local origin escaped

  IFS=',' read -ra origins <<< "$csv"
  for raw_origin in "${origins[@]}"; do
    origin="$(trim "$raw_origin")"
    [ -z "$origin" ] && continue

    if [ "$origin" = "*" ]; then
      echo "❌ CORS_ALLOWED_ORIGINS cannot contain '*'; XLStatus uses cookie credentials." >&2
      exit 1
    fi

    escaped="$(toml_escape "$origin")"
    if [ "$first" -eq 1 ]; then
      result="${result}\"${escaped}\""
      first=0
    else
      result="${result}, \"${escaped}\""
    fi
  done

  result="${result}]"
  printf '%s' "$result"
}

bind_port() {
  local bind="$1"
  printf '%s' "${bind##*:}"
}

warn_if_port_busy() {
  local name="$1"
  local bind="$2"
  local port
  port="$(bind_port "$bind")"

  if command -v ss >/dev/null 2>&1 && ss -tlnp 2>/dev/null | grep -Eq ":${port}[[:space:]]"; then
    echo "⚠️  $name port $port appears to be in use. XLStatus may fail to start."
    ss -tlnp 2>/dev/null | grep -E ":${port}[[:space:]]" || true
  fi
}

is_truthy_value() {
  case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|y|on) return 0 ;;
    *) return 1 ;;
  esac
}

is_interactive() {
  case "$(printf '%s' "$INTERACTIVE" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|y|on) return 0 ;;
    0|false|no|n|off) return 1 ;;
  esac

  [ -t 0 ] || [ -r /dev/tty ]
}

prompt_read() {
  local prompt="$1"
  local silent="${2:-false}"

  if [ -r /dev/tty ]; then
    if [ "$silent" = "true" ]; then
      read -r -s -p "$prompt" PROMPT_REPLY </dev/tty
      echo >/dev/tty
    else
      read -r -p "$prompt" PROMPT_REPLY </dev/tty
    fi
  else
    if [ "$silent" = "true" ]; then
      read -r -s -p "$prompt" PROMPT_REPLY
      echo
    else
      read -r -p "$prompt" PROMPT_REPLY
    fi
  fi
}

prompt_value() {
  local var_name="$1"
  local label="$2"
  local current="${!var_name}"

  prompt_read "$label [$current]: "
  if [ -n "$PROMPT_REPLY" ]; then
    printf -v "$var_name" '%s' "$PROMPT_REPLY"
  fi
}

prompt_secret() {
  local var_name="$1"
  local label="$2"

  prompt_read "$label: " true
  if [ -n "$PROMPT_REPLY" ]; then
    printf -v "$var_name" '%s' "$PROMPT_REPLY"
  fi
}

prompt_bool() {
  local var_name="$1"
  local label="$2"
  local current="${!var_name}"
  local hint="[y/N]"

  if is_truthy_value "$current"; then
    hint="[Y/n]"
  fi

  prompt_read "$label $hint: "
  case "$(printf '%s' "$PROMPT_REPLY" | tr '[:upper:]' '[:lower:]')" in
    y|yes|1|true|on) printf -v "$var_name" 'true' ;;
    n|no|0|false|off) printf -v "$var_name" 'false' ;;
  esac
}

configure_interactively() {
  if ! is_interactive; then
    return
  fi

  echo ""
  echo "🧭 Interactive configuration"
  echo "   Press Enter to keep the value shown in brackets."
  echo "   Set INTERACTIVE=false to skip prompts for unattended installs."
  echo ""

  prompt_value VERSION "Release version to download"
  prompt_value BINARY_PATH "Local server binary path; leave empty to download from GitHub Releases"
  prompt_value INSTALL_DIR "Install directory"
  prompt_value DATA_DIR "Data directory"
  prompt_value CONFIG_FILE "Config file path"
  prompt_value HTTP_BIND "HTTP bind address"
  prompt_value GRPC_BIND "gRPC bind address"

  local db_backend="sqlite"
  if [[ "$DATABASE_URL" == postgres://* || "$DATABASE_URL" == postgresql://* ]]; then
    db_backend="postgres"
  fi

  prompt_read "Database backend (sqlite/postgres) [$db_backend]: "
  if [ -n "$PROMPT_REPLY" ]; then
    db_backend="$(printf '%s' "$PROMPT_REPLY" | tr '[:upper:]' '[:lower:]')"
  fi

  case "$db_backend" in
    postgres|postgresql|pg)
      if [[ "$DATABASE_URL" != postgres://* && "$DATABASE_URL" != postgresql://* ]]; then
        DATABASE_URL="postgresql://xlstatus:change-this-password@localhost:5432/xlstatus"
      fi
      prompt_value DATABASE_URL "PostgreSQL DATABASE_URL"
      DATABASE_CREATE_IF_MISSING=false
      ;;
    sqlite|"")
      local sqlite_path="$DATA_DIR/xlstatus.db"
      prompt_read "SQLite database file [$sqlite_path]: "
      if [ -n "$PROMPT_REPLY" ]; then
        sqlite_path="$PROMPT_REPLY"
      fi
      DATABASE_URL="sqlite://$sqlite_path?mode=rwc"
      prompt_bool DATABASE_CREATE_IF_MISSING "Create SQLite database if missing"
      ;;
    *)
      echo "❌ Unsupported database backend: $db_backend"
      exit 1
      ;;
  esac

  prompt_value CORS_ALLOWED_ORIGINS "Web UI CORS allowed origins"

  if [ -z "$SESSION_SECRET" ]; then
    prompt_secret SESSION_SECRET "Session secret; leave empty to generate one"
  else
    prompt_secret SESSION_SECRET "Session secret; leave empty to keep current value"
  fi

  prompt_value ADMIN_USERNAME "Seed admin username"
  if [ -z "$ADMIN_PASSWORD" ]; then
    prompt_secret ADMIN_PASSWORD "Seed admin password; leave empty to skip admin bootstrap"
  else
    prompt_secret ADMIN_PASSWORD "Seed admin password; leave empty to keep current value"
  fi

  prompt_bool INSTALL_DEPS "Install OS dependencies"
  prompt_bool START_SERVICE "Start xlstatus service after install"
}

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
  echo "❌ Unsupported OS for the systemd installer: $OS"
  exit 1
fi

if ! ASSET_ARCH="$(normalize_arch "$ARCH")"; then
  echo "❌ Unsupported architecture: $ARCH"
  exit 1
fi

echo "✓ Detected: Linux $ASSET_ARCH"

configure_interactively

# Install dependencies
echo ""
if is_truthy_value "$INSTALL_DEPS"; then
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
else
  echo "📦 Skipping dependency installation"
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
  resolve_version
  echo "✓ Release version: $VERSION"
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-server-linux-${ASSET_ARCH}"
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
CORS_ALLOWED_ORIGINS_TOML="$(cors_origins_toml "$CORS_ALLOWED_ORIGINS")"

cat > "$CONFIG_FILE" << EOF
[database]
url = "$DATABASE_URL"
create_if_missing = $DATABASE_CREATE_IF_MISSING

[server]
http_bind = "$HTTP_BIND"
grpc_bind = "$GRPC_BIND"
cors_allowed_origins = $CORS_ALLOWED_ORIGINS_TOML

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
WorkingDirectory=$DATA_DIR
ExecStart=/usr/local/bin/xlstatus-server
Restart=on-failure
RestartSec=5s

Environment="CONFIG_FILE=$CONFIG_FILE"
Environment="RUST_LOG=info"
Environment="XLSTATUS_SEED_ADMIN_USERNAME=$ADMIN_USERNAME"
$(if [ -n "$ADMIN_PASSWORD" ]; then printf 'Environment="XLSTATUS_SEED_ADMIN_PASSWORD=%s"\n' "$ADMIN_PASSWORD"; fi)

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
if is_truthy_value "$START_SERVICE"; then
  echo "🚀 Starting XLStatus..."
  warn_if_port_busy "HTTP" "$HTTP_BIND"
  warn_if_port_busy "gRPC" "$GRPC_BIND"
  systemctl start xlstatus

  # Wait for service to be ready
  sleep 3

  if systemctl is-active --quiet xlstatus; then
    echo "✓ XLStatus is running"
  else
    echo "❌ Failed to start XLStatus"
    echo ""
    systemctl status xlstatus --no-pager || true
    echo ""
    journalctl -u xlstatus -n 80 --no-pager || true
    exit 1
  fi
else
  echo "🚀 Skipping service start"
fi

HTTP_PORT="$(bind_port "$HTTP_BIND")"
if is_truthy_value "$START_SERVICE" && command -v curl >/dev/null 2>&1; then
  if curl -fsS "http://127.0.0.1:${HTTP_PORT}/healthz" >/dev/null; then
    echo "✓ Health check passed"
  else
    echo "⚠️  Service is active, but /healthz did not respond on 127.0.0.1:${HTTP_PORT}"
  fi
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
echo "📍 API URL: http://$(hostname -I | awk '{print $1}'):${HTTP_PORT}"
echo "🔑 Admin user: $ADMIN_USERNAME"
echo "⚙️  Config file: $CONFIG_FILE"
echo "🌐 CORS origins: $CORS_ALLOWED_ORIGINS"
echo ""
echo "📝 Useful commands:"
echo "   - Start:   systemctl start xlstatus"
echo "   - Stop:    systemctl stop xlstatus"
echo "   - Status:  systemctl status xlstatus"
echo "   - Logs:    journalctl -u xlstatus -f"
echo ""
echo "📚 Documentation: https://docs.xlstatus.io"
echo ""
