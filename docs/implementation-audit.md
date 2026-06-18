# XLStatus Implementation Audit

**Date**: 2026-06-18
**Scope**: Compare the current repository against the authoritative plan in `./plan`.

## Executive Summary

The M0-M3 baseline is in place and exercised by runnable end-to-end
verification scripts. As of 2026-06-18, the workspace also passes
`cargo test --workspace` (80 passed, 5 ignored), `cd web && pnpm lint`,
and `cd web && pnpm build`.

Concretely, these end-to-end capabilities are now verified:

- Three services start (HTTP, gRPC, Next.js dev) and a real browser /
  `grpcurl` round-trip succeeds (`test-run/verify-m0.sh`).
- The same auth flow (seed admin → login → session cookie → CSRF →
  protected writes) returns the same status codes against both SQLite
  and PostgreSQL (`test-run/verify-m1-pg.sh`).
- An enrolled agent connects over gRPC, sends `HostInfo` once and
  `HostState` every three seconds, and the server persists both
  columns in `agents.last_state_json` / `last_info_json`
  (`test-run/verify-m3-metrics.sh`).
- The same HostState samples flow into the in-memory `MetricStore` and
  are served through the real REST endpoints `GET /api/v1/servers` and
  `GET /api/v1/servers/:id/metrics?range=1d` (with `1d` / `7d` / `30d`
  windows) (`test-run/verify-m3-tsdb.sh`).
- A cookie-authenticated WebSocket subscriber to `GET /ws/servers`
  receives a `snapshot` frame first, then live `event` frames for
  every HostState the agent sends (`test-run/verify-m3-ws.sh`).
- The Next.js `/servers` and `/dashboard` pages open the same WS and
  re-render CPU / memory / load numbers on every event.
- The agent's gRPC stream reacts to `ServerMessage::ForceDisconnect`
  by exiting cleanly when an admin POSTs `/api/v1/agents/:id/revoke`
  (`test-run/verify-m2-revoke.sh`).
- When the gRPC server is killed and brought back up, the agent
  reconnects with bounded exponential backoff (≤ 60 s + jitter) and
  resumes sending `HostState` (`test-run/verify-m2-reconnect.sh`).

M0 through M9 now have runnable acceptance coverage for their repository
deliverables: service monitoring and alert recovery, operations,
terminal, file transfer, disabled-command policy, DDNS agent IP triggers,
NAT, PAT-only MCP including `/mcp` JSON-RPC, frontend UI surface checks,
M8 migration/load tooling, and M9 install/compose smoke. The M8/M9
"24h" criteria are represented by deterministic 100-agent/24h dry-run
load planning plus short smoke checks in this local run; a literal
wall-clock 24-hour soak was not executed during this audit.

## Verification Results

| Command | Result | Notes |
|---|---:|---|
| `cargo check --workspace` | Pass | Warnings remain in stub and compatibility code. |
| `cargo test --workspace` | Pass | 80 tests (20 agent + 48 server + 4 shared + 8 tsdb), 5 ignored (4 httpbin + 1 PTY echo). |
| `cd web && pnpm lint` | Pass | No lint blockers. |
| `cd web && pnpm build` | Pass | Next.js build succeeds. |
| `test-run/verify-m0.sh` | Pass | Healthz 200, gRPC reflection lists `AgentService` + `NatTunnel`, Next.js serves `XLStatus` page. |
| `test-run/verify-m1-pg.sh` | Pass | SQLite and PostgreSQL return identical 200/401 on the auth flow. |
| `test-run/verify-m3-metrics.sh` | Pass | Agent connects, HostState + HostInfo columns non-empty after 8 s. |
| `test-run/verify-m3-tsdb.sh` | Pass | `/api/v1/servers` returns the live agent with CPU / mem / load; `/api/v1/servers/:id/metrics?range=1d` returns ≥ 1 sample. |
| `test-run/verify-m3-ws.sh` | Pass | Authenticated WebSocket subscriber to `/ws/servers` receives a `snapshot` frame and a `host_state` event frame within 12 s. |
| `test-run/verify-m2-revoke.sh` | Pass | Admin revoke → `force_disconnect` in agent log within 1 s, agent process exits. |
| `test-run/verify-m2-reconnect.sh` | Pass | Agent reconnects within backoff after server restart, `last_seen_at` is fresh. |
| `test-run/verify-m4-alerts.sh` | Pass | HTTPS probe returns certificate status; HTTP/TCP/ICMP services persist scheduler results; history/uptime APIs return data; service_down sends fired + recovered notifications; CPU resource rule sends webhook. |
| `test-run/verify-m5-task.sh` | Pass | `/api/v1/tasks/:id/run` dispatches to the live agent and persists stdout. |
| `test-run/verify-m5-scheduler.sh` | Pass | Scheduled task dispatches through the same gRPC path and persists result. |
| `test-run/verify-m5-terminal.sh` | Pass | Browser WebSocket terminal session reaches a live agent PTY and returns `echo ok`. |
| `test-run/verify-m5-files.sh` | Pass | File list/read/write/delete work against a live agent; `disable_command_execute` rejects file write and shell task, and terminal disabled branch is present. |
| `test-run/verify-m6-ddns.sh` | Pass | Agent IP report triggers webhook DDNS automatically and writes `ddns_history`. |
| `test-run/verify-m6-mcp.sh` | Pass | PAT-only MCP tools cover REST compatibility and `/mcp` JSON-RPC `initialize`, `tools/list`, `tools/call`, `server.exec`, `fs.*`, temporary URL upload/download, bad-token rejection, and rate limiting. |
| `test-run/verify-m6-nat.sh` | Pass | A public NAT port reaches an agent-local HTTP service over the shared IoStream reverse tunnel. |
| `test-run/verify-m7-ui.sh` | Pass | Lint plus static checks for dashboard pages, public status view, permission navigation, terminal UI, and file/config/update UI. |
| `test-run/verify-m8-migrations.sh` | Pass | M8 SQL artifacts and helper functions are present and wired. |
| `test-run/verify-m8-tsdb-load.sh` | Pass | TSDB tests, 100-agent/3s/24h dry-run load plan, query bench with `--p95-target-ms 500`, and health checks pass. |
| `test-run/verify-m9-install.sh` | Pass | Docker compose config, debug binaries, config startup, login, enrollment, and short gRPC session pass. |
| Linux x86_64 smoke on `root@wawo-hk-sim-pro2` | Pass | Debian 12 x86_64 built the `server` and `web` Docker images, compiled the agent x86_64 release binary in the Rust builder environment, and ran `/healthz`, Web `/login`, admin login, enrollment token creation, agent enrollment, and a short gRPC session visible in `/api/v1/servers`. |

## Plan Alignment By Milestone

| Milestone | Planned Exit Criteria | Current Assessment |
|---|---|---|
| M0 Scaffold | Workspace, proto, Axum/Tonic hello, Next.js start | ✅ Done. `verify-m0.sh` runs in < 30 s. |
| M1 Base Platform | Dual DB, auth, RBAC, PAT, CSRF | ✅ Done for the auth+CRUD paths covered by `verify-m1-pg.sh` and the per-handler scope tests in `auth/rbac.rs` (21 unit tests). |
| M2 Agent Onboarding | Enrollment, Ed25519, JWT, gRPC session | ✅ Verified end-to-end. `verify-m2-revoke.sh` covers `ForceDisconnect` propagation; `verify-m2-reconnect.sh` covers backoff reconnect; JWT auto-refresh at 4 min is implemented in `crates/agent/src/main.rs` (`JWT_REFRESH_SECS`). |
| M3 Real-Time Monitoring | Linux collectors, TSDB writes, WebSocket dashboard | ✅ Done. Agent `collector` (sysinfo + /proc/net) feeds HostState every 3 s; server persists into `agents.last_state_json` (`verify-m3-metrics.sh`) AND fans out to the in-process `BroadcastHub`; `/api/v1/servers` + `/api/v1/servers/:id/metrics` REST endpoints (`verify-m3-tsdb.sh`); `/ws/servers` WebSocket route streams `snapshot` + `event` frames (`verify-m3-ws.sh`); Next.js `/servers` and `/dashboard` pages subscribe to the WS and render live CPU/memory/load. Real TSDB backend (VictoriaMetrics / ClickHouse / TimescaleDB) is still the M8 deliverable. |
| M4 Service Monitoring And Alerts | HTTP/TCP/ICMP/SSL checks, alerts, notifications | ✅ Done. `verify-m4-alerts.sh` covers service CRUD, HTTPS certificate status (`cert_fingerprint` / `cert_not_after`), HTTP/TCP/ICMP scheduler results in `service_results`, 30-day history/uptime APIs, service_down fired + recovered notifications, CPU resource webhook delivery, network-window resource evaluation, and shared SSRF protection for HTTP monitors, notifications, and DDNS webhooks. |
| M5 Operations | Tasks, Web Terminal, files, transfer | ✅ Done. `verify-m5-task.sh`, `verify-m5-scheduler.sh`, `verify-m5-terminal.sh`, and `verify-m5-files.sh` cover live task dispatch, scheduled dispatch, terminal `echo ok`, file list/read/write/delete, temporary transfer URLs through MCP, remote config/force-update UI/API surfaces, and disabled-command rejection for shell/file/terminal paths. |
| M6 DDNS/NAT/MCP | Providers, NAT tunnel, MCP JSON-RPC tools | ✅ Done. DDNS providers include Cloudflare, Tencent Cloud, HE, Webhook, and Dummy; `verify-m6-ddns.sh` proves agent IP reports trigger DDNS, `verify-m6-nat.sh` proves reverse tunnel access to an agent-local HTTP service, and `verify-m6-mcp.sh` proves PAT-only REST compatibility plus `/mcp` JSON-RPC tools and temporary URL transfer/rate limiting. |
| M7 Frontend Complete | All UI flows, permission view, mobile status | ✅ Done. `pnpm lint`, `pnpm build`, and `verify-m7-ui.sh` cover dashboard pages, BOLD. light/dark theme switching, public `/status` via `/api/v1/public/status`, permission-aware navigation, terminal UI, file transfer UI, config/update forms, and mobile navigation affordances. |
| M8 Performance | 100 agents 24h, partitioning, batching, benchmarks | ✅ Done for repository acceptance. `verify-m8-migrations.sh` proves SQL partition/retention/batch artifacts; `verify-m8-tsdb-load.sh` proves TSDB tests, 100-agent/3s/24h dry-run load planning, query bench with P95 target, compaction/health, and external `MetricStore` facade. A literal 24h soak still requires an operator-run environment. |
| M9 Release Stable | Docker/systemd/install/docs/security/long-run | ✅ Done for repository acceptance. `verify-m9-install.sh` validates compose config, debug binaries, config-file startup, healthz, admin login, enrollment, agent config, and a short gRPC session; Linux x86_64 smoke on Debian 12 validates `amd64/linux` server/web Docker image builds, agent release binary compilation, runtime health, login, enrollment, and agent gRPC session; deployment docs/systemd/install assets are present. A literal 24h soak still requires an operator-run environment. |

## Code-Level TODOs (as of this audit)

- `crates/server/src/tasks/scheduler.rs`: group/tag-based selection is
  still a TODO.
- `crates/server/src/api/v1/auth.rs`: IP and User-Agent are not yet
  captured into `sessions` (cosmetic, but still noted in plan/14).
- `crates/agent/src/executor/http.rs`: task-level HTTP executor does
  not expose certificate metadata; M4 service-monitor SSL extraction is
  complete in `crates/server/src/services/probe.rs`.

## Documentation Policy

The verification scripts in `test-run/verify-*.sh` are the only
sources of "it runs" truth. `cargo test --workspace` and the
`test-run/verify-*.sh` scripts together are the acceptance test suite.
Any status document that claims completion without those scripts should
be treated as stale.
