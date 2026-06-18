#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PG="$ROOT/crates/server/migrations/postgres/008_m8_performance.sql"
SQLITE="$ROOT/crates/server/migrations/sqlite/008_m8_performance.sql"

for file in "$PG" "$SQLITE"; do
  test -s "$file"
done

required_pg=(
  "CREATE TABLE IF NOT EXISTS xlstatus_metric_retention_policies"
  "xlstatus_ensure_high_io_partitions"
  "xlstatus_apply_high_io_retention"
  "xlstatus_insert_service_results_batch"
  "xlstatus_insert_task_runs_batch"
  "xlstatus_insert_transfers_batch"
  "xlstatus_insert_audit_logs_batch"
  "ON CONFLICT DO NOTHING"
)

for pattern in "${required_pg[@]}"; do
  rg -q "$pattern" "$PG"
done

for table in service_results task_runs audit_logs transfers; do
  rg -q "'$table'" "$PG"
  rg -q "'$table'" "$SQLITE"
done

echo "M8 migration artifacts verified: partition helpers, retention policy, batch SQL functions"
