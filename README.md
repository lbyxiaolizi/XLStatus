English | [简体中文](./README.zh-CN.md)

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/badge/release-v0.3-10b981.svg)](https://github.com/lbyxiaolizi/XLStatus/releases)
[![Rust](https://img.shields.io/badge/Rust-Axum%20%2B%20Tonic%20gRPC-orange.svg)](./crates/)
[![Frontend](https://img.shields.io/badge/Next.js%2016-React%2019%20%2B%20Strict%20TS-000000.svg)](./web/)
[![MCP](https://img.shields.io/badge/MCP-2024--11--05-6366f1.svg)](./docs/mcp.md)

**XLStatus** is a **High-Security, AI-Native Self-Hosted Infrastructure Monitoring & Cluster Operations Platform** written in **Rust** (`Axum` + `Tonic` gRPC) and **Next.js 16** (`React 19` + Strict TypeScript).

It unifies real-time fleet telemetry, customizable public status pages, distributed & local service probes, automated alerting & self-healing, remote task scheduling, WebSocket PTY terminals, safe remote file operations, multi-provider DDNS, NAT reverse tunnels, and a full **Model Context Protocol (`MCP 2024-11-05`)** server into a single audited stack.

> **Current Release**: `v0.3` — Start with the [Documentation Index](./docs/README.md), [AI & MCP Guide](./docs/mcp.md), and [Security Architecture](./security.md).

---

## Architecture Topology

```mermaid
flowchart TB
  subgraph Clients["Operators, Public Viewers & AI Agents"]
    WebUI["Next.js 16 Ops Workbench<br/>(Priority+ Nav · ⌘K Palette · Studio)"]
    PublicStatus["Public Status Page (/status)<br/>(SSR · Cards / Table / Global Map)"]
    HttpMCP["Streamable HTTP MCP Clients<br/>(Claude Code · Cursor · Windsurf · Antigravity)"]
    StdioMCP["Stdio MCP Clients<br/>(Claude Desktop · MCP Inspector)"]
  end

  subgraph Bridge["MCP Transport Layer"]
    StdioScript["scripts/xlstatus-mcp-stdio.mjs<br/>(Zero-Dep Node.js Stdio Bridge)"]
  end

  subgraph ControlPlane["xlstatus-server (Rust · Axum HTTP/WS + Tonic gRPC)"]
    AuthLayer["Defense-in-Depth Auth Boundary<br/>(Argon2id · Single-Use TOTP · HMAC CSRF · Scoped PAT)"]
    RestWS["REST API (/api/v1/*) & Realtime WS (/ws/servers)"]
    McpEngine["MCP 2024-11-05 Engine (/mcp)<br/>(19 Tools · 6 Live Resources · 4 SRE Prompts)"]
    CoreServices["Alert Engine · Cron Scheduler · DDNS · NAT Manager"]
    GrpcServer["Tonic gRPC Control & Data Plane (:50051)<br/>(Ed25519 Enrollment · JWT · Session/Task/IO Registries)"]
  end

  subgraph Storage["Persistence & Telemetry Storage"]
    DB[("SQLite / PostgreSQL<br/>(HKDF-SHA256 enc:v2: Secrets)")]
    TSDB[("Embedded TSDB<br/>(Downsampled Metrics & Retention)")]
  end

  subgraph Fleet["xlstatus-agent Fleet (Linux · macOS · Windows · FreeBSD)"]
    Agent["xlstatus-agent Daemon<br/>(0600 Config · Monotonic Security Locks)"]
    Subsystems["Host Telemetry · HTTP/TCP/ICMP Probes · Process-Group Shell<br/>WebSocket PTY · O_NOFOLLOW File Ops · NAT Local Dialer"]
  end

  WebUI <-->|"Cookie + CSRF + x-totp-code"| AuthLayer
  PublicStatus <-->|"Sanitized Public DTO"| RestWS
  HttpMCP <-->|"POST /mcp (Bearer xlp_*)"| AuthLayer
  StdioMCP <--> StdioScript <-->|"POST /mcp (Bearer xlp_*)"| AuthLayer

  AuthLayer --> RestWS & McpEngine
  RestWS & McpEngine --> CoreServices & GrpcServer
  RestWS & McpEngine & CoreServices --> DB & TSDB
  GrpcServer <-->|"Bidirectional gRPC Stream (TLS/mTLS)"| Agent
  Agent --> Subsystems
```

---

## Six Core Pillars

| Pillar | Capabilities & Highlights |
| :--- | :--- |
| **1. Real-Time Fleet Telemetry & Public Status** | Sub-second streaming telemetry for CPU, RAM, swap, disk usage, network RX/TX throughput (`bps`) & cumulative traffic, load averages (`1m/5m/15m`), TCP/UDP connections, process count, GPU utilization, and temperatures. Includes an interactive `d3-geo` World Map (`WorldServerMap`) and a customizable public `/status` page supporting **Cards / Table / Global Map** views, region & status filter pills, SLA uptime bars, and SSR metadata. |
| **2. Service Probes, SLA & Alert Automation** | Distributed (Agent-side) and local (Server-side) **HTTP(S), TCP, and ICMP** probes with TLS certificate expiry tracking and latency histograms. Multi-condition alert rules with duration windows, recovery notifications, single-pass RFC 8259-escaped JSON Webhook delivery, and automated self-healing task triggers (`trigger_task`). |
| **3. Interactive Cluster Operations** | Fleet batch & cron task studio with one-click SRE playbooks; `xterm.js` WebSocket PTY terminal with quick diagnostic snippet toolbar; safe remote file manager (`O_NOFOLLOW \| O_NONBLOCK`, `S_IFREG` verification) with single-use temporary download/upload URLs (`xlt_`); NAT reverse TCP tunnels with per-IP rate limiting; and multi-provider DDNS (**Cloudflare, HE.net, DNSPod, Webhook**). |
| **4. AI-Native Cluster Copilot & MCP `2024-11-05` Server** | Native Model Context Protocol server at `/mcp` plus interactive Web Studio (`/mcp`). Exposes **19 specialized ops tools** (`cluster.diagnose`, `cluster.overview`, `server.resolve`, `server.inspect`, `server.exec`, `server.batch_exec`, `server.metrics`, etc.), **6 live `xlstatus://` telemetry resources**, and **4 live-context SRE prompts**, supported via both Streamable HTTP (`POST /mcp`) and a zero-dependency Node.js `stdio` bridge (`scripts/xlstatus-mcp-stdio.mjs`). |
| **5. Modern Adaptive Frontend (`web/`)** | Built on **Next.js 16 + React 19** with full strict TypeScript compliance (`strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noPropertyAccessFromIndexSignature`). Features a `ResizeObserver`-driven **PC Priority+ adaptive top navigation bar** (`Navigation.tsx`), global **`⌘K` / `Ctrl+K` Command Palette**, neo-brutalist light/dark themes, and a segmented settings center. |
| **6. Defense-in-Depth Security Architecture** | Hardened across 3 audit rounds: Argon2id + constant-time login mitigation, single-use TOTP 2FA with brute-force lockout & sensitive action step-up (`x-totp-code`), HMAC-SHA256 CSRF, fine-grained PAT scopes + `server_ids` allowlist, `HKDF-SHA256` at-rest secret encryption (`enc:v2:`), Ed25519 challenge-response agent enrollment (`xle_`), DNS-pinned SSRF protection (blocking IPv4/IPv6 private, loopback, link-local, SIIT, NAT64, 6to4, and cloud metadata IPs), and monotonic local agent security locks (`0600` config). |

---

## Quick Start (Docker Compose)

### 1. Default Stack (SQLite)

```bash
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus

# Create initial admin password secret and signing keys with strict permissions
mkdir -p .secrets
printf '%s\n' 'replace-with-a-strong-initial-password' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password

test -f .env || (umask 077; printf 'SESSION_SECRET=%s\nPOSTGRES_PASSWORD=%s\n' \
  "$(openssl rand -hex 32)" "$(openssl rand -hex 24)" > .env)

docker compose up -d
curl -fsS http://localhost:8080/healthz
```

Once running, open:
- **Ops Workbench (Web UI)**: `http://localhost:3000`
- **Public Status Page**: `http://localhost:3000/status`
- **AI Copilot & MCP Studio**: `http://localhost:3000/mcp`
- **HTTP API & MCP Endpoint**: `http://localhost:8080` (`POST http://localhost:8080/mcp`)

> **Security Note on Agent gRPC (`:50051`)**: Docker Compose binds the host gRPC port to `127.0.0.1:50051` by default. To accept remote Agents across networks, set `XLSTATUS_GRPC_PUBLISH_ADDRESS=0.0.0.0` **only** when gRPC is protected by TLS/mTLS or a trusted private network (WireGuard/Tailscale). Keep `.env` private and do not regenerate `SESSION_SECRET` across upgrades, as it derives the `HKDF-SHA256` master key for encrypted secrets at rest.

### 2. PostgreSQL Stack

```bash
docker compose -f docker-compose.pg.yml up -d
```

---

## Release Install Scripts (Linux systemd)

### Install Server

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.3/install-server.sh \
  | sudo VERSION=v0.3 bash
```

### Enroll & Install Agent

Generate a parameterized one-line bootstrap command from **Settings -> Agent Install** in the dashboard, or run:

```bash
sudo SERVER_URL=https://status.example.com \
  GRPC_SERVER=https://grpc.status.example.com:50051 \
  VERSION=v0.3 \
  ENROLLMENT_TOKEN=xle_your_single_use_enrollment_token \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.3/install-agent.sh | bash'
```

Both installers stage binaries and systemd units atomically in the target directory, enforce systemd sandboxing (`NoNewPrivileges`, `ProtectSystem`, `ProtectHome`, `UMask=0077`), and automatically roll back on health-check failure.

---

## AI Agent & MCP Quick Connect

XLStatus implements the official **MCP `2024-11-05`** specification. Issue a scoped Personal Access Token (`xlp_...`) from **Tokens** or the **MCP Studio (`/mcp` -> Connect Tab)** with a least-privilege preset (*Read-Only Audit*, *Standard Troubleshooting*, or *Full Autonomous Ops*) and optional node allowlist (`server_ids`).

### Option A: Streamable HTTP (`POST /mcp`)
For **Claude Code, Cursor, Windsurf, or Antigravity**:

```json
{
  "mcpServers": {
    "xlstatus": {
      "url": "http://localhost:8080/mcp",
      "headers": {
        "Authorization": "Bearer xlp_your_personal_access_token"
      }
    }
  }
}
```

### Option B: Stdio Bridge (`scripts/xlstatus-mcp-stdio.mjs`)
For **Claude Desktop** or stdio-only MCP clients (requires Node.js 20+, zero npm dependencies):

```json
{
  "mcpServers": {
    "xlstatus": {
      "command": "node",
      "args": ["/absolute/path/to/XLStatus/scripts/xlstatus-mcp-stdio.mjs"],
      "env": {
        "XLSTATUS_MCP_URL": "http://localhost:8080/mcp",
        "XLSTATUS_PAT": "xlp_your_personal_access_token"
      }
    }
  }
}
```

See the full **[AI & MCP Guide](./docs/mcp.md)** for tool schemas, resource URIs, and SRE prompt templates.

---

## Source Build & Verification

### Build from Source

```bash
# 1. Build Rust Server & Agent binaries
cargo build --release --bin xlstatus-server --bin xlstatus-agent

# 2. Build Next.js 16 Web Workbench
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

### Full Test & Verification Suite

```bash
# Rust formatting, workspace typecheck, and 830+ unit/integration tests
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked

# Frontend ESLint, strict TypeScript check, unit tests, and production build
cd web
pnpm lint
pnpm typecheck
pnpm test:unit
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

Repeatable end-to-end acceptance scripts are located in [`test-run/`](./test-run/) (e.g., `verify-m6-mcp.sh`, `verify-m7-ui.sh`). Review each script's header comments before execution.

---

## Documentation

| Guide | Description |
| :--- | :--- |
| **[Documentation Index](./docs/README.md)** | Recommended reading order and deployment conventions |
| **[Quick Start](./docs/quickstart.md)** | Fastest path to launch Server, Web UI, and Agent locally |
| **[AI & MCP Guide](./docs/mcp.md)** | MCP `2024-11-05` architecture, 19 tools, 6 resources, 4 prompts, and IDE setup |
| **[Security Architecture](./security.md)** | Threat model, 3-round security hardening reference, and residual risk boundaries |
| **[Installation](./docs/installation.md)** | Docker Compose, source deployment, systemd units, and PostgreSQL setup |
| **[Configuration](./docs/configuration.md)** | `config.toml`, environment variables, CORS, TLS/mTLS, and Agent security flags |
| **[Agent Setup](./docs/agent.md)** | Ed25519 enrollment (`xle_`), local security locks, and multi-platform operations |
| **[Web Frontend](./docs/web.md)** | Next.js 16 workbench architecture, strict TypeScript, and [`web/README.md`](./web/README.md) |
| **[API Overview](./docs/api.md)** | REST (`/api/v1/*`), Realtime WebSocket (`/ws/servers`), gRPC, and `/mcp` contracts |
| **[Operations](./docs/operations.md)** | Health checks, SQLite backup/restore, SQLite-to-PostgreSQL migration, and upgrades |
| **[Troubleshooting](./docs/troubleshooting.md)** | Diagnostics for startup errors, CORS, database locks, and Agent connectivity |
| **[Development](./docs/development.md)** | Workspace architecture, coding standards, and test harnesses |
| **[Architecture](./docs/architecture.md)** | Component boundaries, data flows, and deployment topology |
| **[Project Structure](./docs/project-structure.md)** | Crate responsibilities and repository directory layout |
| **[Release Checklist](./docs/release-checklist.md)** | Pre-release verification gates and artifact checks |

---

## Repository Layout

```text
crates/
  server/       Axum HTTP/WS server, Tonic gRPC control plane, MCP 2024-11-05 engine, SQLite/PG repos
  agent/        Host telemetry collector, probe runner, process-group shell, PTY, safe file ops, NAT dialer
  shared/       Shared domain types, DTOs, and protocol helpers
  proto-gen/    Compiled Tonic/Prost gRPC bindings
  tsdb/         Embedded time-series storage engine with downsampling and retention
  xtask/        Workspace automation tasks
web/            Next.js 16 (React 19 + Strict TypeScript) Ops Workbench and Public Status UI
proto/          Protobuf service and message definitions for Agent <-> Server gRPC
scripts/        Zero-dep MCP stdio bridge (xlstatus-mcp-stdio.mjs) and SQLite -> PostgreSQL migration tool
deploy/         Hardened systemd service templates and atomic Linux install scripts
docs/           Authoritative user, operator, and developer documentation
test-run/       Isolated acceptance and smoke verification suites
```

## License

MIT. See [LICENSE](./LICENSE).
