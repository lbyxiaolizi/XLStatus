#!/usr/bin/env bash
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_eq() {
  local expected="$1"
  local actual="$2"
  local label="$3"
  [ "$actual" = "$expected" ] || fail "$label: expected '$expected', got '$actual'"
}

assert_call() {
  local calls="$1"
  local expected="$2"
  local label="$3"
  grep -Fqx "$expected" "$calls" || fail "$label: missing systemctl call '$expected'"
}

assert_symlink() {
  local path="$1"
  local target="$2"
  local label="$3"
  [ -L "$path" ] || fail "$label: $path is not a symbolic link"
  assert_eq "$target" "$(readlink "$path")" "$label target"
}

assert_hardening_matches() {
  local generated="$1"
  local reference="$2"
  local label="$3"
  local key expected actual

  for key in NoNewPrivileges PrivateTmp ProtectSystem ProtectHome ReadWritePaths; do
    expected="$(grep -E "^${key}=" "$reference" | tr -d '"')"
    actual="$(grep -E "^${key}=" "$generated" | tr -d '"')"
    assert_eq "$expected" "$actual" "$label $key"
  done
}

test_server_installer() (
  # shellcheck source=../deploy/install.sh
  source "$ROOT/deploy/install.sh"

  MOCK_ACTIVE=false
  MOCK_ENABLED=false
  SYSTEMCTL_CALLS="$TMP_DIR/server-systemctl.calls"
  : > "$SYSTEMCTL_CALLS"

  systemctl() {
    printf '%s\n' "$*" >> "$SYSTEMCTL_CALLS"
    case "$1" in
      is-active) is_truthy_value "$MOCK_ACTIVE" ;;
      is-enabled) is_truthy_value "$MOCK_ENABLED" ;;
      start|restart) MOCK_ACTIVE=true ;;
      stop) MOCK_ACTIVE=false ;;
      enable) MOCK_ENABLED=true ;;
      disable) MOCK_ENABLED=false ;;
      *) return 0 ;;
    esac
  }

  wait_for_healthz() {
    return 0
  }

  CONFIG_FILE="$TMP_DIR/server.toml"
  SYSTEMD_UNIT_FILE="$TMP_DIR/xlstatus.service"
  cat > "$CONFIG_FILE" <<'TOML'
[database]
url = "sqlite:///srv/xlstatus/data.db?mode=rwc"

[server] # existing production listener
http_bind = "0.0.0.0:19090"
grpc_bind = '10.0.0.8:15051'

[security]
session_secret = "test-only-secret-that-is-at-least-32-bytes"
TOML
  printf '%s\n' 'existing-unit-must-not-be-rewritten' > "$SYSTEMD_UNIT_FILE"

  HTTP_BIND="127.0.0.1:8080"
  GRPC_BIND="0.0.0.0:50051"
  RECONFIGURE=false
  MOCK_ACTIVE=true
  MOCK_ENABLED=true
  load_existing_install_state
  assert_eq true "$PRESERVE_CONFIG" "existing config preservation"
  assert_eq true "$PRESERVE_SYSTEMD_UNIT" "existing unit preservation"
  assert_eq true "$SERVICE_WAS_ACTIVE" "existing active state"
  assert_eq true "$SERVICE_WAS_ENABLED" "existing enabled state"
  assert_eq "0.0.0.0:19090" "$EFFECTIVE_HTTP_BIND" "preserved HTTP bind"
  assert_eq "10.0.0.8:15051" "$EFFECTIVE_GRPC_BIND" "preserved gRPC bind"
  assert_eq "http://127.0.0.1:19090/healthz" "$(health_url)" "health URL"

  RECONFIGURE=true
  load_existing_install_state
  assert_eq false "$PRESERVE_CONFIG" "explicit reconfiguration"
  assert_eq false "$PRESERVE_SYSTEMD_UNIT" "unit replacement during reconfiguration"

  DATA_DIR_SYSTEMD="/var/lib/xlstatus"
  CONFIG_FILE_SYSTEMD="/etc/xlstatus/server.toml"
  ADMIN_USERNAME_SYSTEMD="admin"
  BOOTSTRAP_ENV_FILE="/run/xlstatus/bootstrap.env"
  GENERATED_UNIT="$TMP_DIR/generated-xlstatus.service"
  write_server_systemd_unit "$GENERATED_UNIT"
  assert_hardening_matches "$GENERATED_UNIT" "$ROOT/deploy/xlstatus-server.service" "server unit hardening"

  reset_server_transaction() {
    cleanup_backups
    BINARY_TMP=""
    CONFIG_TMP=""
    UNIT_TMP=""
    BINARY_TARGET=""
    BINARY_CHANGED=false
    CONFIG_CHANGED=false
    UNIT_CHANGED=false
    BINARY_EXISTED=false
    CONFIG_EXISTED=false
    UNIT_EXISTED=false
    BINARY_LINK_EXISTED=false
    BINARY_LINK_TARGET=""
    SERVICE_WAS_ACTIVE=false
    SERVICE_WAS_ENABLED=false
    ROLLBACK_HTTP_BIND=""
    : > "$SYSTEMCTL_CALLS"
  }

  reset_server_transaction
  INSTALL_DIR="$TMP_DIR/existing-install"
  BINARY_LINK="$TMP_DIR/bin/xlstatus-server"
  mkdir -p "$INSTALL_DIR" "$(dirname "$BINARY_LINK")"
  printf '\177ELFold-version\n' > "$INSTALL_DIR/xlstatus-server"
  cp "$INSTALL_DIR/xlstatus-server" "$TMP_DIR/expected-old-server"
  ln -s "$INSTALL_DIR/xlstatus-server" "$BINARY_LINK"
  BINARY_TMP="$(mktemp "$INSTALL_DIR/.xlstatus-server.XXXXXX")"
  printf '\177ELFnew-version\n' > "$BINARY_TMP"
  activate_staged_binary
  grep -q 'new-version' "$INSTALL_DIR/xlstatus-server" || fail "new server binary was not activated"
  SERVICE_WAS_ACTIVE=true
  SERVICE_WAS_ENABLED=true
  MOCK_ACTIVE=true
  MOCK_ENABLED=true
  ROLLBACK_HTTP_BIND="0.0.0.0:19090"
  rollback_install
  cmp -s "$TMP_DIR/expected-old-server" "$INSTALL_DIR/xlstatus-server" || fail "old server binary was not restored"
  assert_symlink "$BINARY_LINK" "$INSTALL_DIR/xlstatus-server" "existing server binary link rollback"
  assert_call "$SYSTEMCTL_CALLS" 'enable xlstatus' "enabled server rollback"
  assert_call "$SYSTEMCTL_CALLS" 'restart xlstatus' "active server rollback"

  reset_server_transaction
  OLD_INSTALL_DIR="$TMP_DIR/old-custom-server-install"
  INSTALL_DIR="$TMP_DIR/new-default-server-install"
  BINARY_LINK="$TMP_DIR/bin/xlstatus-server"
  mkdir -p "$OLD_INSTALL_DIR" "$INSTALL_DIR"
  printf '\177ELFold-custom-version\n' > "$OLD_INSTALL_DIR/xlstatus-server"
  ln -sfn "$OLD_INSTALL_DIR/xlstatus-server" "$BINARY_LINK"
  BINARY_TMP="$(mktemp "$INSTALL_DIR/.xlstatus-server.XXXXXX")"
  printf '\177ELFnew-default-version\n' > "$BINARY_TMP"
  activate_staged_binary
  MOCK_ACTIVE=false
  MOCK_ENABLED=true
  rollback_install
  [ ! -e "$INSTALL_DIR/xlstatus-server" ] || fail "new server target remained after rollback"
  assert_symlink "$BINARY_LINK" "$OLD_INSTALL_DIR/xlstatus-server" "custom server install link rollback"

  reset_server_transaction
  INSTALL_DIR="$TMP_DIR/server-symlink-install"
  BINARY_LINK="$TMP_DIR/bin/xlstatus-server"
  mkdir -p "$INSTALL_DIR"
  printf '\177ELFold-symlink-version\n' > "$INSTALL_DIR/old-server-payload"
  ln -s 'old-server-payload' "$INSTALL_DIR/xlstatus-server"
  ln -sfn "$INSTALL_DIR/xlstatus-server" "$BINARY_LINK"
  BINARY_TMP="$(mktemp "$INSTALL_DIR/.xlstatus-server.XXXXXX")"
  printf '\177ELFnew-regular-version\n' > "$BINARY_TMP"
  activate_staged_binary
  [ ! -L "$INSTALL_DIR/xlstatus-server" ] || fail "server symlink target was not replaced during upgrade"
  MOCK_ACTIVE=false
  MOCK_ENABLED=true
  rollback_install
  assert_symlink "$INSTALL_DIR/xlstatus-server" 'old-server-payload' "server binary object rollback"
  grep -q 'old-symlink-version' "$INSTALL_DIR/xlstatus-server" || fail "server symlink payload changed"
  assert_call "$SYSTEMCTL_CALLS" 'disable xlstatus' "disabled server rollback"
  assert_call "$SYSTEMCTL_CALLS" 'stop xlstatus' "inactive server rollback"

  reset_server_transaction
  CONFIG_FILE="$TMP_DIR/server-link.toml"
  SYSTEMD_UNIT_FILE="$TMP_DIR/server-link.service"
  printf '%s\n' 'old-config' > "$TMP_DIR/original-server.toml"
  printf '%s\n' 'old-unit' > "$TMP_DIR/original-server.service"
  ln -s 'original-server.toml' "$CONFIG_FILE"
  ln -s 'original-server.service' "$SYSTEMD_UNIT_FILE"
  CONFIG_EXISTED=true
  CONFIG_BACKUP="$(backup_file_object "$CONFIG_FILE" "$TMP_DIR/.server.toml.backup.XXXXXX")"
  CONFIG_CHANGED=true
  rm "$CONFIG_FILE"
  printf '%s\n' 'new-config' > "$CONFIG_FILE"
  UNIT_EXISTED=true
  UNIT_BACKUP="$(backup_file_object "$SYSTEMD_UNIT_FILE" "$TMP_DIR/.xlstatus.service.backup.XXXXXX")"
  UNIT_CHANGED=true
  rm "$SYSTEMD_UNIT_FILE"
  printf '%s\n' 'new-unit' > "$SYSTEMD_UNIT_FILE"
  MOCK_ACTIVE=false
  MOCK_ENABLED=false
  rollback_install
  assert_symlink "$CONFIG_FILE" 'original-server.toml' "server config symlink rollback"
  assert_symlink "$SYSTEMD_UNIT_FILE" 'original-server.service' "server unit symlink rollback"
)

test_agent_installer() (
  # shellcheck source=../deploy/install-agent.sh
  source "$ROOT/deploy/install-agent.sh"

  MOCK_ACTIVE=false
  MOCK_ENABLED=false
  SYSTEMCTL_CALLS="$TMP_DIR/agent-systemctl.calls"
  : > "$SYSTEMCTL_CALLS"

  systemctl() {
    printf '%s\n' "$*" >> "$SYSTEMCTL_CALLS"
    case "$1" in
      is-active) is_truthy_value "$MOCK_ACTIVE" ;;
      is-enabled) is_truthy_value "$MOCK_ENABLED" ;;
      start|restart) MOCK_ACTIVE=true ;;
      stop) MOCK_ACTIVE=false ;;
      enable) MOCK_ENABLED=true ;;
      disable) MOCK_ENABLED=false ;;
      *) return 0 ;;
    esac
  }

  BINARY_TARGET_SYSTEMD="/usr/local/bin/xlstatus-agent"
  CONFIG_FILE_SYSTEMD="/etc/xlstatus-agent/agent.json"
  CONFIG_DIR_SYSTEMD="/etc/xlstatus-agent"
  AGENT_DATA_DIR_SYSTEMD="/var/lib/xlstatus-agent"
  AGENT_WORK_DIR_SYSTEMD="/opt/xlstatus-agent"
  GENERATED_UNIT="$TMP_DIR/generated-xlstatus-agent.service"
  write_agent_systemd_unit "$GENERATED_UNIT"
  assert_hardening_matches "$GENERATED_UNIT" "$ROOT/deploy/xlstatus-agent.service" "agent unit hardening"

  reset_agent_transaction() {
    cleanup_backups
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
    : > "$SYSTEMCTL_CALLS"
  }

  reset_agent_transaction
  BINARY_TARGET="$TMP_DIR/token-failure/xlstatus-agent"
  CONFIG_FILE="$TMP_DIR/token-failure/agent.json"
  SYSTEMD_UNIT_FILE="$TMP_DIR/token-failure/xlstatus-agent.service"
  mkdir -p "$(dirname "$BINARY_TARGET")"
  printf '%s\n' 'old-agent-binary' > "$BINARY_TARGET"
  printf '%s\n' 'old-agent-config' > "$CONFIG_FILE"
  printf '%s\n' 'old-agent-unit' > "$SYSTEMD_UNIT_FILE"
  cp "$BINARY_TARGET" "$TMP_DIR/expected-old-agent"
  BINARY_EXISTED=true
  BINARY_BACKUP="$(backup_file_object "$BINARY_TARGET" "$TMP_DIR/token-failure/.xlstatus-agent.backup.XXXXXX")"
  BINARY_CHANGED=true
  printf '%s\n' 'new-agent-binary' > "$BINARY_TARGET"
  SERVICE_WAS_ACTIVE=true
  SERVICE_WAS_ENABLED=true
  MOCK_ACTIVE=true
  MOCK_ENABLED=true
  rollback_agent_install
  cmp -s "$TMP_DIR/expected-old-agent" "$BINARY_TARGET" || fail "agent binary was not restored after token/enrollment failure"
  grep -qx 'old-agent-config' "$CONFIG_FILE" || fail "agent config changed after token/enrollment failure"
  grep -qx 'old-agent-unit' "$SYSTEMD_UNIT_FILE" || fail "agent unit changed after token/enrollment failure"
  assert_call "$SYSTEMCTL_CALLS" 'enable xlstatus-agent' "enabled agent rollback"
  assert_call "$SYSTEMCTL_CALLS" 'restart xlstatus-agent' "active agent rollback"

  reset_agent_transaction
  STAGE_DIR="$TMP_DIR/post-config-failure"
  mkdir -p "$STAGE_DIR"
  BINARY_TARGET="$STAGE_DIR/xlstatus-agent"
  CONFIG_FILE="$STAGE_DIR/agent.json"
  SYSTEMD_UNIT_FILE="$STAGE_DIR/xlstatus-agent.service"
  printf '%s\n' 'old-binary-payload' > "$STAGE_DIR/old-agent-binary"
  printf '%s\n' 'old-config-payload' > "$STAGE_DIR/old-agent-config"
  printf '%s\n' 'old-unit-payload' > "$STAGE_DIR/old-agent-unit"
  ln -s 'old-agent-binary' "$BINARY_TARGET"
  ln -s 'old-agent-config' "$CONFIG_FILE"
  ln -s 'old-agent-unit' "$SYSTEMD_UNIT_FILE"

  BINARY_EXISTED=true
  BINARY_BACKUP="$(backup_file_object "$BINARY_TARGET" "$STAGE_DIR/.xlstatus-agent.backup.XXXXXX")"
  BINARY_CHANGED=true
  NEW_BINARY="$(mktemp "$STAGE_DIR/.xlstatus-agent.XXXXXX")"
  printf '%s\n' 'new-agent-binary' > "$NEW_BINARY"
  mv -f "$NEW_BINARY" "$BINARY_TARGET"

  CONFIG_TMP="$(mktemp "$STAGE_DIR/.agent.json.XXXXXX")"
  printf '%s\n' 'new-agent-config' > "$CONFIG_TMP"
  activate_staged_config
  [ ! -L "$CONFIG_FILE" ] || fail "agent config symlink was not replaced during upgrade"

  UNIT_TMP="$(mktemp "$STAGE_DIR/.xlstatus-agent.service.XXXXXX")"
  printf '%s\n' 'new-agent-unit' > "$UNIT_TMP"
  activate_staged_unit
  [ ! -L "$SYSTEMD_UNIT_FILE" ] || fail "agent unit symlink was not replaced during upgrade"

  MOCK_ACTIVE=false
  MOCK_ENABLED=true
  rollback_agent_install
  assert_symlink "$BINARY_TARGET" 'old-agent-binary' "agent binary symlink rollback"
  assert_symlink "$CONFIG_FILE" 'old-agent-config' "agent config symlink rollback"
  assert_symlink "$SYSTEMD_UNIT_FILE" 'old-agent-unit' "agent unit symlink rollback"
  grep -qx 'old-config-payload' "$CONFIG_FILE" || fail "agent config payload changed during staged enrollment"
  assert_call "$SYSTEMCTL_CALLS" 'disable xlstatus-agent' "disabled agent rollback"
  assert_call "$SYSTEMCTL_CALLS" 'stop xlstatus-agent' "inactive agent rollback"

  reset_agent_transaction
  FRESH_DIR="$TMP_DIR/fresh-install-failure"
  mkdir -p "$FRESH_DIR"
  BINARY_TARGET="$FRESH_DIR/xlstatus-agent"
  CONFIG_FILE="$FRESH_DIR/agent.json"
  SYSTEMD_UNIT_FILE="$FRESH_DIR/xlstatus-agent.service"
  printf '%s\n' 'new-agent-binary' > "$BINARY_TARGET"
  printf '%s\n' 'new-agent-config' > "$CONFIG_FILE"
  printf '%s\n' 'new-agent-unit' > "$SYSTEMD_UNIT_FILE"
  BINARY_CHANGED=true
  CONFIG_CHANGED=true
  UNIT_CHANGED=true
  SERVICE_WAS_ACTIVE=false
  SERVICE_WAS_ENABLED=false
  MOCK_ACTIVE=true
  MOCK_ENABLED=true
  rollback_agent_install
  [ ! -e "$BINARY_TARGET" ] || fail "fresh agent binary remained after failed installation"
  [ ! -e "$CONFIG_FILE" ] || fail "fresh agent config remained after failed installation"
  [ ! -e "$SYSTEMD_UNIT_FILE" ] || fail "fresh agent unit remained after failed installation"
  assert_eq false "$MOCK_ACTIVE" "fresh agent active state rollback"
  assert_eq false "$MOCK_ENABLED" "fresh agent enabled state rollback"
  assert_call "$SYSTEMCTL_CALLS" 'disable xlstatus-agent' "fresh agent disable rollback"
  assert_call "$SYSTEMCTL_CALLS" 'stop xlstatus-agent' "fresh agent stop rollback"

  reset_agent_transaction
  BINARY_TARGET="$TMP_DIR/trap-failure/xlstatus-agent"
  mkdir -p "$(dirname "$BINARY_TARGET")"
  printf '%s\n' 'old-before-trap' > "$BINARY_TARGET"
  if (
    BINARY_EXISTED=true
    BINARY_BACKUP="$(backup_file_object "$BINARY_TARGET" "$TMP_DIR/trap-failure/.xlstatus-agent.backup.XXXXXX")"
    BINARY_CHANGED=true
    printf '%s\n' 'new-before-trap' > "$BINARY_TARGET"
    SERVICE_WAS_ACTIVE=false
    SERVICE_WAS_ENABLED=false
    MOCK_ACTIVE=false
    MOCK_ENABLED=false
    trap cleanup_agent_install EXIT
    exit 23
  ); then
    fail "agent failure trap unexpectedly returned success"
  else
    status=$?
  fi
  assert_eq 23 "$status" "agent failure exit status"
  grep -qx 'old-before-trap' "$BINARY_TARGET" || fail "agent EXIT trap did not restore old binary"
)

test_server_installer
test_agent_installer

echo "OK: installer upgrade preservation, hardening, and rollback"
