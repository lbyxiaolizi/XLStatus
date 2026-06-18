# M8 High IO And Performance

This note records the M8 performance plumbing that is locally verifiable
without a long-running PostgreSQL environment.

## Implemented

- `xlstatus-tsdb` exposes an external backend interface via
  `MetricBackend` and `MetricStore::from_backend`.
- `MetricStore` supports `write_batch`, `write_json_batch`, `compact`,
  `health`, and `backend_name`.
- PostgreSQL migration stubs in
  `crates/server/migrations/postgres/008_m8_performance.sql` provide
  safe/idempotent retention metadata, recent-window indexes,
  partition-management helpers, and JSONB batch insert helpers for:
  `service_results`, `task_runs`, `transfers`, and `audit_logs`.
- SQLite migration stubs in
  `crates/server/migrations/sqlite/008_m8_performance.sql` provide the
  local retention-policy metadata and matching recent-window indexes.
- The server migration runner now applies both `008_m8_performance.sql`
  files after `007_m4_m6.sql`, so the helpers run during normal startup.
- `cargo run -p xtask -- mock-agents` generates deterministic
  mock agent metric load. Use `--dry-run --output <file>` for a fast
  local proof, or omit `--dry-run` to write the generated samples into
  the local in-memory `MetricStore`.
- `cargo run -p xtask -- query-bench` seeds the in-memory
  backend and runs deterministic 1d/7d/30d query windows.

## Verification

```bash
test-run/verify-m8-migrations.sh
test-run/verify-m8-tsdb-load.sh
```

The migration verifier checks for the expected safe SQL artifacts. The
TSDB/load verifier runs the TSDB unit tests, validates deterministic
mock-agent sample counts, and executes the local query bench.

## Caveats

- Existing PostgreSQL heap tables are not converted to partitioned
  parents automatically. The partition helper functions no-op unless a
  table is already a `RANGE (created_at)` partitioned parent, which keeps
  the migration safe for existing deployments.
- Batch helper functions exist in SQL and TSDB, but high-IO server
  repository paths still need a scoped server-source change to call them
  from production write paths.
- `plan/15-verification-commands.md` names the package
  `xlstatus-xtask`, but the current workspace package is still `xtask`.
  The verification scripts use the checked-in package name.
- The `mock-agents` tool is deterministic local load generation. Full
  100-agent/24-hour acceptance still requires a live server, real agent
  enrollment/JWT flow, and environment-level observability.
