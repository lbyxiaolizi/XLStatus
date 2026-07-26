#!/usr/bin/env bash

# Process ownership helpers for long-running verification services. Each
# process is supervised in a dedicated session and identified by a random
# token before its process group can receive a signal.

_XLSTATUS_TEST_PROCESS_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_XLSTATUS_TEST_PROCESS_SUPERVISOR="$_XLSTATUS_TEST_PROCESS_LIB_DIR/process-supervisor.py"

test_process_init() {
  local root="$1"
  local script_path="$2"
  local key
  local name
  local registry_dir
  local lock_owner

  script_path="$(cd "$(dirname "$script_path")" && pwd)/$(basename "$script_path")"
  key="$(printf '%s\n' "$root:$script_path" | cksum | awk '{print $1}')"
  name="$(basename "$script_path" .sh)"
  registry_dir="${TMPDIR:-/tmp}/xlstatus-test-processes-${UID:-$(id -u)}"
  mkdir -p "$registry_dir"
  chmod 700 "$registry_dir"
  TEST_PROCESS_REGISTRY="$registry_dir/${name}-${key}.tsv"
  TEST_PROCESS_LOCK_DIR="$registry_dir/${name}-${key}.lock"
  if ! mkdir "$TEST_PROCESS_LOCK_DIR" 2>/dev/null; then
    lock_owner="$(cat "$TEST_PROCESS_LOCK_DIR/owner" 2>/dev/null || true)"
    if [[ "$lock_owner" =~ ^[0-9]+$ ]] && kill -0 "$lock_owner" 2>/dev/null; then
      echo "FAIL: another $name verification is already running (PID $lock_owner)" >&2
      return 1
    fi
    rm -rf "$TEST_PROCESS_LOCK_DIR"
    if ! mkdir "$TEST_PROCESS_LOCK_DIR" 2>/dev/null; then
      echo "FAIL: another $name verification acquired the process registry lock" >&2
      return 1
    fi
  fi
  printf '%s\n' "$$" > "$TEST_PROCESS_LOCK_DIR/owner"
  test_process_cleanup "$TEST_PROCESS_REGISTRY"
  chmod 600 "$TEST_PROCESS_REGISTRY"
}

test_process_is_owned() {
  local pid="$1"
  local token="$2"
  local command_line
  local process_group_id

  [[ "$pid" =~ ^[0-9]+$ ]] && [[ "$pid" -gt 1 ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1

  process_group_id="$(ps -p "$pid" -o pgid= 2>/dev/null | tr -d '[:space:]')"
  [[ "$process_group_id" == "$pid" ]] || return 1

  command_line="$(COLUMNS=4096 ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command_line" == *"$_XLSTATUS_TEST_PROCESS_SUPERVISOR"* ]] || return 1
  [[ "$command_line" == *"$token"* ]]
}

test_process_start() {
  local registry="$1"
  local label="$2"
  local log_file="$3"
  shift 3

  local attempt
  local pid
  local token
  token="xlstatus-test-${$}-${RANDOM}-${RANDOM}-$(date +%s)"

  nohup python3 "$_XLSTATUS_TEST_PROCESS_SUPERVISOR" "$token" "$@" \
    > "$log_file" 2>&1 < /dev/null &
  pid=$!

  for attempt in $(seq 1 50); do
    if test_process_is_owned "$pid" "$token"; then
      break
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid" 2>/dev/null || true
      echo "FAIL: $label process exited before ownership registration" >&2
      return 1
    fi
    sleep 0.02
  done

  if ! test_process_is_owned "$pid" "$token"; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "FAIL: could not establish isolated ownership for $label process" >&2
    return 1
  fi

  printf '%s\t%s\t%s\n' "$pid" "$token" "$label" >> "$registry"
  TEST_PROCESS_PID="$pid"
  TEST_PROCESS_TOKEN="$token"
  disown "$pid" 2>/dev/null || true
}

test_process_stop() {
  local pid="$1"
  local token="$2"
  local label="${3:-process}"
  local attempt

  if ! test_process_is_owned "$pid" "$token"; then
    return 0
  fi

  kill -TERM -- "-$pid" 2>/dev/null || true
  for attempt in $(seq 1 30); do
    if ! test_process_is_owned "$pid" "$token"; then
      return 0
    fi
    sleep 0.1
  done

  if test_process_is_owned "$pid" "$token"; then
    echo "WARN: $label did not stop after SIGTERM; sending SIGKILL" >&2
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
}

test_process_cleanup() {
  local registry="$1"
  local pid
  local token
  local label

  [[ -f "$registry" ]] || {
    : > "$registry"
    return 0
  }

  while IFS=$'\t' read -r pid token label; do
    [[ -n "$pid" && -n "$token" ]] || continue
    test_process_stop "$pid" "$token" "${label:-process}"
  done < "$registry"
  : > "$registry"
}

test_process_stop_label() {
  local registry="$1"
  local wanted_label="$2"
  local next_registry="${registry}.next.$$"
  local pid
  local token
  local label

  : > "$next_registry"
  while IFS=$'\t' read -r pid token label; do
    [[ -n "$pid" && -n "$token" ]] || continue
    if [[ "$label" == "$wanted_label" ]]; then
      test_process_stop "$pid" "$token" "$label"
    else
      printf '%s\t%s\t%s\n' "$pid" "$token" "$label" >> "$next_registry"
    fi
  done < "$registry"
  mv "$next_registry" "$registry"
}

test_process_label_is_running() {
  local registry="$1"
  local wanted_label="$2"
  local pid
  local token
  local label

  while IFS=$'\t' read -r pid token label; do
    if [[ "$label" == "$wanted_label" ]] && test_process_is_owned "$pid" "$token"; then
      return 0
    fi
  done < "$registry"
  return 1
}

test_process_print_cleanup() {
  local registry="$1"
  printf 'Stop test processes with: bash %q cleanup %q\n' \
    "$_XLSTATUS_TEST_PROCESS_LIB_DIR/test-processes.sh" "$registry"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  if [[ "${1:-}" != "cleanup" || -z "${2:-}" ]]; then
    echo "usage: $0 cleanup REGISTRY" >&2
    exit 2
  fi
  test_process_cleanup "$2"
fi
