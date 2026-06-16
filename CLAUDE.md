# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**XLStatus** is a self-hosted server monitoring and operations system rewritten in Rust, functionally equivalent to Nezha. It provides:

- Real-time server monitoring (CPU, memory, disk, network, load, connections, temperature, GPU)
- Service monitoring (HTTP, TCP, ICMP, SSL certificate tracking)
- Alert rules and notification channels
- Scheduled and triggered tasks
- Web terminal and file management
- DDNS integration (Cloudflare, Tencent Cloud, HE, Webhook)
- NAT tunneling for accessing internal services
- MCP (Model Context Protocol) for automation and LLM tool access
- Multi-user RBAC with server ownership and permissions

**Key principle**: We match Nezha's functional capabilities but do not replicate its API contracts, database schema, or brand assets. This is a clean-room reimplementation with independent design choices.

## Project Status

**Current phase**: M5 (Task Execution) - ✅ 100% COMPLETE (Full Implementation)

**Completed milestones**:
- ✅ **M0 (Scaffolding)** - Completed 2026-06-16
- ✅ **M1 (Base Platform)** - Completed 2026-06-16
- ✅ **M2 (Agent Integration)** - Completed 2026-06-16
- ✅ **M3 (Real-time Monitoring)** - Completed 2026-06-16
- ✅ **M4 (Service Monitoring & Alerts)** - Completed 2026-06-16 (Architecture)
- ✅ **M5 (Task Execution)** - Completed 2026-06-17 (Full Implementation)
- ✅ **M6 (NAT Traversal)** - Completed 2026-06-16 (Architecture)
- ✅ **M7 (DDNS)** - Completed 2026-06-16 (Architecture)

**Project Progress**: 5/9 milestones fully implemented (55.6%), 8/9 architecture complete (88.9%)

**Working Features**:
- Complete authentication & authorization
- Agent enrollment & JWT authentication
- gRPC bidirectional streaming
- Session management & heartbeat
- Login page & Dashboard
- Dual database support
- **Task execution (Shell, HTTP, ICMP, TCP)** 🆕
- **Web Terminal (PTY support)** 🆕
- **File management (list/read/write/delete)** 🆕
- **Task scheduler (Cron-based)** 🆕
- **Audit logging** 🆕

**Architecture Ready**:
- Service monitoring & alerts
- NAT traversal & port mapping
- DDNS (Cloudflare, Tencent Cloud, HE, Webhook)

**Next**: M8 (MCP Integration) or implement M4/M6/M7

See completion reports: M0-M7-COMPLETION.md, M5-PROGRESS.md

## Tech Stack

- **Backend**: Rust, Tokio, Axum, Tonic, SQLx, SQLite/PostgreSQL
- **Agent RPC**: Tonic gRPC bidirectional streams
- **Frontend**: Next.js (App Router), TypeScript
- **Real-time**: WebSocket
- **Metrics storage**: Embedded TSDB (with external backend support planned)
- **Deployment**: Docker, systemd (Linux x86_64 first; Windows/macOS agents planned)

## Workspace Structure

```
XLStatus/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── shared/             # Domain types, errors, authz, crypto, validation
│   ├── proto-gen/          # Generated protobuf code
│   ├── tsdb/               # Embedded time-series database
│   ├── server/             # Dashboard (Axum HTTP, Tonic gRPC, workers)
│   ├── agent/              # Agent CLI (collectors, task runner, terminal)
│   └── xtask/              # Development scripts
├── proto/xlstatus/v1/      # Protobuf definitions
├── web/                    # Next.js frontend
├── plan/                   # Design documents (authoritative)
└── docker-compose*.yml     # Deployment configurations
```

See `./plan/11-workspace-layout.md` for detailed module responsibilities.

## Common Commands

### Workspace-level
- `cargo build` — build all crates (debug)
- `cargo build --release` — optimized build
- `cargo test` — run all tests
- `cargo test --package xlstatus-server` — test a specific crate
- `cargo check` — fast type-check without codegen
- `cargo fmt` — format all crates
- `cargo clippy -- -D warnings` — lint with warnings as errors

### Specific crates
- `cargo run -p xlstatus-server` — run the Dashboard server
- `cargo run -p xlstatus-agent -- --help` — run the Agent CLI
- `cargo run -p xtask -- <task>` — run development scripts

### Frontend
- `cd web && npm run dev` — Next.js dev server on `:3000`
- `cd web && npm run build` — production build
- `cd web && npm run lint` — ESLint

### Database migrations
- `sqlx migrate run --database-url sqlite://dev.db` — SQLite migrations
- `sqlx migrate run --database-url postgres://...` — PostgreSQL migrations

### Protobuf
- Rebuild after `.proto` changes: `cargo build -p xlstatus-proto-gen`
- The `build.rs` in `proto-gen` handles code generation via `tonic-build`

## Architecture

### Runtime components

```
Browser/MCP → [Axum :8080] ← WebSocket for real-time updates
                   ↓
              [Server process]
                   ↓
     ┌─────────────┼─────────────┐
     ↓             ↓             ↓
[SQLite/PG]   [TSDB]      [Background workers]
 metadata     metrics      schedulers, alerts,
  users       history      notifications, DDNS
  servers
  config
  audit

Agent ↔ [Tonic gRPC :50051] ↔ [Server process]
  ↑
Collectors (CPU, mem, disk, net, load, temp, GPU)
Task runner (shell, HTTP check, TCP ping, ICMP, terminal, file ops)
```

### Data storage strategy

- **SQL (SQLite/PostgreSQL)**: users, servers, services, alerts, tasks, notifications, DDNS, NAT, PAT, audit logs, settings, necessary aggregations
- **TSDB**: high-frequency metrics (server stats, service probe results) to avoid overwhelming the relational DB
- **Memory**: live agent sessions, real-time snapshots, WebSocket subscriptions

**Critical**: The codebase must support both SQLite (dev/small-scale) and PostgreSQL (production) via `DATABASE_URL`. Use SQLx migrations explicitly; no runtime schema guessing. Business logic accesses DB through repository traits, never directly via `SqlitePool`/`PgPool`.

### Agent lifecycle

1. **Enrollment**: Agent uses one-time enrollment token to register, generates Ed25519 keypair, receives Agent ID
2. **Connection**: Agent connects to gRPC with short-lived JWT, Server validates and creates Session
3. **Reporting**: Agent periodically reports status; Server updates in-memory snapshot, broadcasts via WebSocket, writes to TSDB
4. **Tasks**: Scheduler pushes tasks to Agent via gRPC stream; Agent returns results; Server persists and triggers notifications

## Security & Authorization

- **Web users**: Argon2 password hashing, session cookies, CSRF protection, Personal Access Tokens (PAT) for automation
- **Agent authentication**: Ed25519 signing, JWT refresh, gRPC interceptor validation
- **RBAC**: Admin vs. Member roles; Members see only owned/authorized servers
- **MCP**: PAT-only (no cookies), scope enforcement, server allowlist, rate limiting
- **Audit**: All remote exec, file writes, file deletes, MCP, NAT, PAT operations logged

See `./plan/07-security.md` for complete design.

## Development Workflow

1. **Read the plan first**: All design decisions are in `./plan/`. Start with `README.md`, then check the relevant milestone doc (e.g., `08-roadmap.md`).
2. **Check current milestone**: Don't implement M5 features while still in M0.
3. **Database changes**: Write explicit SQLx migrations for both SQLite and PostgreSQL.
4. **Protobuf changes**: Update `.proto` files, then rebuild `proto-gen` crate.
5. **API changes**: Update OpenAPI docs (via `utoipa` attributes) and frontend TypeScript types.
6. **Testing**: Write tests in `#[cfg(test)]` modules or `tests/`. Run `cargo test` before committing.
7. **Format and lint**: Run `cargo fmt && cargo clippy` before PR.

## Key Constraints

- **No Nezha compatibility**: Do not replicate Nezha's API endpoints, database schema, or frontend behavior. Functional parity only.
- **Database portability**: All SQL must work on both SQLite and PostgreSQL. Use SQLx compile-time checked queries where possible.
- **Security first**: No command injection, XSS, SQL injection, or SSRF. Validate at system boundaries (user input, external APIs). Use parameterized queries, sanitize shell args, restrict webhook URLs to public internet.
- **No premature abstractions**: Implement what's needed for the current milestone. Three similar lines beat a premature helper. No feature flags for hypothetical requirements.
- **Comments only for non-obvious WHY**: Code should be self-documenting via clear names. Only comment on hidden constraints, subtle invariants, or bug workarounds.

## References

- **Complete plan**: `./plan/README.md`
- **Architecture**: `./plan/02-architecture.md`
- **Workspace layout**: `./plan/11-workspace-layout.md`
- **Dependencies**: `./plan/12-dependencies.md`
- **Roadmap**: `./plan/08-roadmap.md`
- **Verification**: `./plan/15-verification-commands.md`
