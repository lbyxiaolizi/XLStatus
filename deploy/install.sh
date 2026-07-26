#!/bin/bash
set -e

# XLStatus Installation Script
# Usage: bash install.sh

VERSION="${VERSION:-v0.2}"
INSTALL_DIR="${INSTALL_DIR:-/opt/xlstatus}"
DATA_DIR="${DATA_DIR:-/var/lib/xlstatus}"
BINARY_PATH="${BINARY_PATH:-}"  # User can provide compiled binary path
CONFIG_FILE="${CONFIG_FILE:-/etc/xlstatus/server.toml}"
BOOTSTRAP_ENV_FILE="/run/xlstatus/bootstrap.env"
HTTP_BIND="${HTTP_BIND:-127.0.0.1:8080}"
GRPC_BIND="${GRPC_BIND:-127.0.0.1:50051}"
DATABASE_URL="${DATABASE_URL:-sqlite://$DATA_DIR/xlstatus.db?mode=rwc}"
DATABASE_CREATE_IF_MISSING="${DATABASE_CREATE_IF_MISSING:-true}"
CORS_ALLOWED_ORIGINS="${CORS_ALLOWED_ORIGINS:-http://localhost:3000,http://127.0.0.1:3000}"
SESSION_SECRET="${SESSION_SECRET:-}"
SECRET_ENCRYPTION_KEY="${SECRET_ENCRYPTION_KEY:-}"
ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
INSTALL_DEPS="${INSTALL_DEPS:-true}"
START_SERVICE="${START_SERVICE:-true}"
INTERACTIVE="${INTERACTIVE:-auto}"
RECONFIGURE="${RECONFIGURE:-false}"
BINARY_TMP=""
CONFIG_TMP=""
UNIT_TMP=""
BINARY_BACKUP=""
CONFIG_BACKUP=""
UNIT_BACKUP=""
BINARY_TARGET=""
BINARY_LINK="/usr/local/bin/xlstatus-server"
BINARY_LINK_EXISTED=false
BINARY_LINK_TARGET=""
BINARY_CHANGED=false
CONFIG_CHANGED=false
UNIT_CHANGED=false
BINARY_EXISTED=false
CONFIG_EXISTED=false
UNIT_EXISTED=false
INSTALL_COMMITTED=false
SERVICE_WAS_ACTIVE=false
SERVICE_WAS_ENABLED=false
PRESERVE_CONFIG=false
PRESERVE_SYSTEMD_UNIT=false
SYSTEMD_UNIT_FILE="${SYSTEMD_UNIT_FILE:-/etc/systemd/system/xlstatus.service}"
EFFECTIVE_HTTP_BIND="$HTTP_BIND"
EFFECTIVE_GRPC_BIND="$GRPC_BIND"
ROLLBACK_HTTP_BIND=""

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

systemd_env_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g'
}

systemd_env_file_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

reject_multiline_value() {
  local name="$1"
  local value="$2"
  if [[ "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    echo "❌ $name must not contain newline characters" >&2
    exit 1
  fi
}

generate_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
    return
  fi
  if [ -r /dev/urandom ]; then
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
    return
  fi
  echo "❌ Unable to generate a secure random secret. Install openssl or provide SESSION_SECRET and SECRET_ENCRYPTION_KEY." >&2
  exit 1
}

activate_staged_binary() {
  local binary_magic

  chmod 755 "$BINARY_TMP"
  binary_magic="$(od -An -N4 -tx1 "$BINARY_TMP" | tr -d ' \n')"
  if [ ! -s "$BINARY_TMP" ] || [ "$binary_magic" != "7f454c46" ]; then
    echo "❌ Staged XLStatus Server file is not a Linux ELF binary" >&2
    exit 1
  fi

  if [ -L "$BINARY_LINK" ]; then
    BINARY_LINK_EXISTED=true
    BINARY_LINK_TARGET="$(readlink "$BINARY_LINK")"
  elif [ -e "$BINARY_LINK" ]; then
    echo "❌ $BINARY_LINK exists and is not a symbolic link; refusing to replace it" >&2
    exit 1
  fi

  BINARY_TARGET="$INSTALL_DIR/xlstatus-server"
  if [ -e "$BINARY_TARGET" ] || [ -L "$BINARY_TARGET" ]; then
    BINARY_EXISTED=true
    BINARY_BACKUP="$(backup_file_object "$BINARY_TARGET" "$INSTALL_DIR/.xlstatus-server.backup.XXXXXX")"
  fi

  BINARY_CHANGED=true
  mv -f "$BINARY_TMP" "$BINARY_TARGET"
  BINARY_TMP=""
  ln -sfn "$BINARY_TARGET" "$BINARY_LINK"
}

read_server_string_setting() {
  local key="$1"
  local config_path="$2"

  awk -v wanted="$key" '
    function trim(value) {
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+$/, "", value)
      return value
    }
    /^[[:space:]]*\[/ {
      section = $0
      sub(/[[:space:]]*#.*/, "", section)
      section = trim(section)
      in_server = (section == "[server]")
      next
    }
    in_server {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      if (line !~ ("^" wanted "[[:space:]]*=")) {
        next
      }
      sub(("^" wanted "[[:space:]]*=[[:space:]]*"), "", line)
      line = trim(line)
      quote = substr(line, 1, 1)
      if (quote == "\"" || quote == "\047") {
        line = substr(line, 2)
        end = index(line, quote)
        if (end > 0) {
          print substr(line, 1, end - 1)
          exit
        }
      }
    }
  ' "$config_path"
}

health_url() {
  local bind="$EFFECTIVE_HTTP_BIND"
  local host port

  if [[ "$bind" =~ ^\[(.*)\]:([0-9]+)$ ]]; then
    host="${BASH_REMATCH[1]}"
    port="${BASH_REMATCH[2]}"
  elif [[ "$bind" =~ ^([^:]+):([0-9]+)$ ]]; then
    host="${BASH_REMATCH[1]}"
    port="${BASH_REMATCH[2]}"
  else
    return 1
  fi

  case "$host" in
    0.0.0.0) host="127.0.0.1" ;;
    ::) host="::1" ;;
  esac
  if [[ "$host" == *:* ]]; then
    host="[$host]"
  fi
  printf 'http://%s:%s/healthz' "$host" "$port"
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

write_server_systemd_unit() {
  local output="$1"

  cat > "$output" << EOF
[Unit]
Description=XLStatus Server
After=network.target

[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory="$DATA_DIR_SYSTEMD"
ExecStart=/usr/local/bin/xlstatus-server
Restart=on-failure
RestartSec=5s

Environment="CONFIG_FILE=$CONFIG_FILE_SYSTEMD"
Environment="RUST_LOG=info"
Environment="XLSTATUS_SEED_ADMIN_USERNAME=$ADMIN_USERNAME_SYSTEMD"
EnvironmentFile=-$BOOTSTRAP_ENV_FILE

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths="$DATA_DIR_SYSTEMD"

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=xlstatus-server

[Install]
WantedBy=multi-user.target
EOF
}

rollback_install() {
  local restore_failed=false

  echo "↩ Rolling back the XLStatus installation..." >&2

  if ! is_truthy_value "$SERVICE_WAS_ENABLED"; then
    if ! systemctl disable xlstatus >/dev/null 2>&1 \
      && systemctl is-enabled --quiet xlstatus; then
      restore_failed=true
    fi
  fi

  if is_truthy_value "$BINARY_CHANGED"; then
    restore_changed_file "$BINARY_TARGET" "$BINARY_BACKUP" "$BINARY_EXISTED" || restore_failed=true
    if is_truthy_value "$BINARY_LINK_EXISTED"; then
      ln -sfn "$BINARY_LINK_TARGET" "$BINARY_LINK" || restore_failed=true
    else
      rm -f "$BINARY_LINK" || restore_failed=true
    fi
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
    systemctl enable xlstatus >/dev/null 2>&1 || restore_failed=true
  fi

  if is_truthy_value "$SERVICE_WAS_ACTIVE"; then
    if [ -n "$ROLLBACK_HTTP_BIND" ]; then
      EFFECTIVE_HTTP_BIND="$ROLLBACK_HTTP_BIND"
    fi
    if systemctl restart xlstatus >/dev/null 2>&1 \
      && systemctl is-active --quiet xlstatus \
      && wait_for_healthz; then
      echo "✓ Previous XLStatus binary restored and service recovered" >&2
    else
      echo "❌ Previous files were restored, but the XLStatus service did not recover" >&2
      restore_failed=true
    fi
  else
    systemctl stop xlstatus >/dev/null 2>&1 || true
  fi

  if is_truthy_value "$restore_failed"; then
    systemctl status xlstatus --no-pager >&2 || true
    journalctl -u xlstatus -n 80 --no-pager >&2 || true
    return 1
  fi
}

finish_install() {
  INSTALL_COMMITTED=true
  cleanup_backups
}

cleanup_install() {
  local status=$?
  trap - EXIT
  set +e
  cleanup_staged_files
  if ! is_truthy_value "$INSTALL_COMMITTED"; then
    if is_truthy_value "$BINARY_CHANGED" \
      || is_truthy_value "$CONFIG_CHANGED" \
      || is_truthy_value "$UNIT_CHANGED"; then
      rollback_install || true
    fi
  fi
  cleanup_backups
  exit "$status"
}

load_existing_install_state() {
  SERVICE_WAS_ACTIVE=false
  SERVICE_WAS_ENABLED=false
  PRESERVE_CONFIG=false
  PRESERVE_SYSTEMD_UNIT=false
  ROLLBACK_HTTP_BIND=""
  EFFECTIVE_HTTP_BIND="$HTTP_BIND"
  EFFECTIVE_GRPC_BIND="$GRPC_BIND"

  if systemctl is-enabled --quiet xlstatus; then
    SERVICE_WAS_ENABLED=true
  fi

  if systemctl is-active --quiet xlstatus; then
    SERVICE_WAS_ACTIVE=true
    if [ -f "$CONFIG_FILE" ]; then
      ROLLBACK_HTTP_BIND="$(read_server_string_setting http_bind "$CONFIG_FILE")"
      ROLLBACK_HTTP_BIND="${ROLLBACK_HTTP_BIND:-127.0.0.1:8080}"
    fi
  fi

  if [ -f "$CONFIG_FILE" ] && ! is_truthy_value "$RECONFIGURE"; then
    PRESERVE_CONFIG=true
    EFFECTIVE_HTTP_BIND="$(read_server_string_setting http_bind "$CONFIG_FILE")"
    EFFECTIVE_GRPC_BIND="$(read_server_string_setting grpc_bind "$CONFIG_FILE")"
    EFFECTIVE_HTTP_BIND="${EFFECTIVE_HTTP_BIND:-127.0.0.1:8080}"
    EFFECTIVE_GRPC_BIND="${EFFECTIVE_GRPC_BIND:-127.0.0.1:50051}"
    if [ -e "$SYSTEMD_UNIT_FILE" ] || [ -L "$SYSTEMD_UNIT_FILE" ]; then
      PRESERVE_SYSTEMD_UNIT=true
    fi
  fi
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

wait_for_healthz() {
  local url
  if ! url="$(health_url)"; then
    echo "❌ Invalid HTTP bind address in preserved configuration: $EFFECTIVE_HTTP_BIND" >&2
    return 1
  fi

  if ! command -v curl >/dev/null 2>&1; then
    return 2
  fi

  for _ in {1..80}; do
    if curl -fsS --noproxy '*' "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done

  return 1
}

write_bootstrap_env_file() {
  if [ -z "$ADMIN_PASSWORD" ]; then
    rm -f "$BOOTSTRAP_ENV_FILE" 2>/dev/null || true
    return
  fi

  local bootstrap_dir
  bootstrap_dir="$(dirname "$BOOTSTRAP_ENV_FILE")"
  mkdir -p "$bootstrap_dir"
  chmod 700 "$bootstrap_dir"
  {
    printf 'XLSTATUS_SEED_ADMIN_USERNAME="%s"\n' "$(systemd_env_file_escape "$ADMIN_USERNAME")"
    printf 'XLSTATUS_SEED_ADMIN_PASSWORD="%s"\n' "$(systemd_env_file_escape "$ADMIN_PASSWORD")"
  } > "$BOOTSTRAP_ENV_FILE"
  chown root:root "$BOOTSTRAP_ENV_FILE"
  chmod 600 "$BOOTSTRAP_ENV_FILE"
}

clear_bootstrap_env_after_seed() {
  if [ -z "$ADMIN_PASSWORD" ]; then
    return
  fi

  rm -f "$BOOTSTRAP_ENV_FILE" 2>/dev/null || true
  rmdir "$(dirname "$BOOTSTRAP_ENV_FILE")" 2>/dev/null || true
  echo "✓ Removed temporary admin bootstrap environment"

  echo "↻ Restarting XLStatus to clear bootstrap secrets from the process environment..."
  if ! systemctl restart xlstatus; then
    echo "❌ XLStatus failed to restart after clearing bootstrap secrets"
    systemctl status xlstatus --no-pager || true
    journalctl -u xlstatus -n 80 --no-pager || true
    exit 1
  fi
  if systemctl is-active --quiet xlstatus && wait_for_healthz; then
    echo "✓ XLStatus restarted without bootstrap secrets"
  else
    echo "❌ XLStatus failed after clearing bootstrap secrets"
    echo ""
    systemctl status xlstatus --no-pager || true
    echo ""
    journalctl -u xlstatus -n 80 --no-pager || true
    exit 1
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
  if [ -f "$CONFIG_FILE" ] && ! is_truthy_value "$RECONFIGURE"; then
    echo "   Existing configuration will be preserved (set RECONFIGURE=true to edit it)."
    prompt_bool INSTALL_DEPS "Install OS dependencies"
    prompt_bool START_SERVICE "Restart xlstatus service after install"
    return
  fi
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
  if [ -z "$SECRET_ENCRYPTION_KEY" ]; then
    prompt_secret SECRET_ENCRYPTION_KEY "Secret encryption key; leave empty to generate one"
  else
    prompt_secret SECRET_ENCRYPTION_KEY "Secret encryption key; leave empty to keep current value"
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

main() {
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                                                              ║"
echo "║   Installing XLStatus Server                                 ║"
echo "║                                                              ║"
echo "╚══════════════════════════════════════════════════════════════╝"

trap cleanup_install EXIT

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
load_existing_install_state

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

if is_truthy_value "$START_SERVICE" && ! command -v curl >/dev/null 2>&1; then
  echo "❌ curl is required to verify the service before committing an upgraded binary"
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
  BINARY_TMP="$(mktemp "$INSTALL_DIR/.xlstatus-server.XXXXXX")"
  cp "$BINARY_PATH" "$BINARY_TMP"
  activate_staged_binary
  echo "✓ Binary installed from: $BINARY_PATH"
else
  # Try to download from GitHub releases
  resolve_version
  echo "✓ Release version: $VERSION"
  DOWNLOAD_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/xlstatus-server-linux-${ASSET_ARCH}"
  echo "   Trying to download from: $DOWNLOAD_URL"

  BINARY_TMP="$(mktemp "$INSTALL_DIR/.xlstatus-server.XXXXXX")"
  if curl -fsSL "$DOWNLOAD_URL" -o "$BINARY_TMP" 2>/dev/null; then
    activate_staged_binary
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
reject_multiline_value DATA_DIR "$DATA_DIR"
reject_multiline_value CONFIG_FILE "$CONFIG_FILE"
reject_multiline_value ADMIN_USERNAME "$ADMIN_USERNAME"
reject_multiline_value ADMIN_PASSWORD "$ADMIN_PASSWORD"
reject_multiline_value BOOTSTRAP_ENV_FILE "$BOOTSTRAP_ENV_FILE"
DATA_DIR_SYSTEMD="$(systemd_env_escape "$DATA_DIR")"
CONFIG_FILE_SYSTEMD="$(systemd_env_escape "$CONFIG_FILE")"
ADMIN_USERNAME_SYSTEMD="$(systemd_env_escape "$ADMIN_USERNAME")"

if is_truthy_value "$PRESERVE_CONFIG"; then
  echo "✓ Preserving existing configuration: $CONFIG_FILE"
  echo "  Set RECONFIGURE=true to replace it explicitly."
else
  if [ -z "$SESSION_SECRET" ]; then
    SESSION_SECRET="$(generate_secret)"
  fi
  if [ -z "$SECRET_ENCRYPTION_KEY" ]; then
    SECRET_ENCRYPTION_KEY="$(generate_secret)"
  fi
  reject_multiline_value DATABASE_URL "$DATABASE_URL"
  reject_multiline_value HTTP_BIND "$HTTP_BIND"
  reject_multiline_value GRPC_BIND "$GRPC_BIND"
  reject_multiline_value CORS_ALLOWED_ORIGINS "$CORS_ALLOWED_ORIGINS"
  reject_multiline_value SESSION_SECRET "$SESSION_SECRET"
  reject_multiline_value SECRET_ENCRYPTION_KEY "$SECRET_ENCRYPTION_KEY"
  CORS_ALLOWED_ORIGINS_TOML="$(cors_origins_toml "$CORS_ALLOWED_ORIGINS")"
  DATABASE_URL_TOML="$(toml_escape "$DATABASE_URL")"
  HTTP_BIND_TOML="$(toml_escape "$HTTP_BIND")"
  GRPC_BIND_TOML="$(toml_escape "$GRPC_BIND")"
  SESSION_SECRET_TOML="$(toml_escape "$SESSION_SECRET")"
  SECRET_ENCRYPTION_KEY_TOML="$(toml_escape "$SECRET_ENCRYPTION_KEY")"

  if [ -e "$CONFIG_FILE" ] || [ -L "$CONFIG_FILE" ]; then
    CONFIG_EXISTED=true
    CONFIG_BACKUP="$(backup_file_object "$CONFIG_FILE" "$(dirname "$CONFIG_FILE")/.server.toml.backup.XXXXXX")"
  fi
  CONFIG_TMP="$(mktemp "$(dirname "$CONFIG_FILE")/.server.toml.XXXXXX")"
  cat > "$CONFIG_TMP" << EOF
[database]
url = "$DATABASE_URL_TOML"
create_if_missing = $DATABASE_CREATE_IF_MISSING

[server]
http_bind = "$HTTP_BIND_TOML"
grpc_bind = "$GRPC_BIND_TOML"
cors_allowed_origins = $CORS_ALLOWED_ORIGINS_TOML

[security]
session_secret = "$SESSION_SECRET_TOML"
secret_encryption_key = "$SECRET_ENCRYPTION_KEY_TOML"
session_ttl_hours = 24
EOF

  chown xlstatus:xlstatus "$CONFIG_TMP"
  chmod 600 "$CONFIG_TMP"
  CONFIG_CHANGED=true
  mv -f "$CONFIG_TMP" "$CONFIG_FILE"
  CONFIG_TMP=""

  echo "✓ Configuration created"
fi

# Install systemd service
echo ""
if is_truthy_value "$PRESERVE_SYSTEMD_UNIT"; then
  echo "🔧 Preserving existing systemd service: $SYSTEMD_UNIT_FILE"
else
  echo "🔧 Installing systemd service..."
  if [ -e "$SYSTEMD_UNIT_FILE" ] || [ -L "$SYSTEMD_UNIT_FILE" ]; then
    UNIT_EXISTED=true
    UNIT_BACKUP="$(backup_file_object "$SYSTEMD_UNIT_FILE" "$(dirname "$SYSTEMD_UNIT_FILE")/.xlstatus.service.backup.XXXXXX")"
  fi
  UNIT_TMP="$(mktemp "$(dirname "$SYSTEMD_UNIT_FILE")/.xlstatus.service.XXXXXX")"
  write_server_systemd_unit "$UNIT_TMP"

  chmod 644 "$UNIT_TMP"
  UNIT_CHANGED=true
  mv -f "$UNIT_TMP" "$SYSTEMD_UNIT_FILE"
  UNIT_TMP=""
fi

systemctl daemon-reload
systemctl enable xlstatus
if is_truthy_value "$PRESERVE_SYSTEMD_UNIT"; then
  echo "✓ Existing systemd service preserved"
else
  echo "✓ Systemd service installed"
fi

# Start service
echo ""
if is_truthy_value "$START_SERVICE"; then
  echo "🚀 Starting XLStatus..."
  write_bootstrap_env_file
  warn_if_port_busy "HTTP" "$EFFECTIVE_HTTP_BIND"
  warn_if_port_busy "gRPC" "$EFFECTIVE_GRPC_BIND"
  if systemctl is-active --quiet xlstatus; then
    if ! systemctl restart xlstatus; then
      echo "❌ Failed to restart XLStatus"
      systemctl status xlstatus --no-pager || true
      journalctl -u xlstatus -n 80 --no-pager || true
      exit 1
    fi
  else
    if ! systemctl start xlstatus; then
      echo "❌ Failed to start XLStatus"
      systemctl status xlstatus --no-pager || true
      journalctl -u xlstatus -n 80 --no-pager || true
      exit 1
    fi
  fi

  if ! systemctl is-active --quiet xlstatus; then
    echo "❌ Failed to start XLStatus"
    echo ""
    systemctl status xlstatus --no-pager || true
    echo ""
    journalctl -u xlstatus -n 80 --no-pager || true
    exit 1
  fi
  if ! wait_for_healthz; then
    echo "❌ XLStatus is active, but $(health_url) did not respond"
    if [ -n "$ADMIN_PASSWORD" ]; then
      echo "   Bootstrap secret remains root-only in $BOOTSTRAP_ENV_FILE"
    fi
    systemctl status xlstatus --no-pager || true
    journalctl -u xlstatus -n 80 --no-pager || true
    exit 1
  fi
  echo "✓ XLStatus is running and healthy"
  if [ -n "$ADMIN_PASSWORD" ]; then
    clear_bootstrap_env_after_seed
  fi
else
  echo "🚀 Skipping service start"
  if [ -n "$ADMIN_PASSWORD" ]; then
    rm -f "$BOOTSTRAP_ENV_FILE" 2>/dev/null || true
    echo "⚠️  ADMIN_PASSWORD was not persisted because START_SERVICE=false; provide it again for the first service start if admin bootstrap is still needed."
  fi
fi

finish_install
HTTP_PORT="$(bind_port "$EFFECTIVE_HTTP_BIND")"

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
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
