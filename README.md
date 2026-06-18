English | [简体中文](./README.zh-CN.md)

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

Self-hosted server monitoring and operations system written in Rust. XLStatus provides real-time monitoring, service health checks, task scheduling, and automation capabilities.

## Current Status

XLStatus is in active development. The workspace has runnable acceptance coverage in `test-run/`; literal 24-hour soak runs still need to be performed in a target deployment environment.

Start with the current documentation index: [docs/README.md](./docs/README.md).

## ✨ Features

- **Real-time Server Monitoring** - CPU, memory, disk, network, load, connections, and temperature data from enrolled agents
- **Service Monitoring** - HTTP, TCP, ICMP health checks with HTTPS certificate fingerprint and expiry tracking
- **Alert Rules** - resource, offline, service status, latency, recovery, and webhook notification flows
- **Task Scheduler** - cron-based and on-demand task execution through live agents
- **NAT Traversal** - access to internal services through reverse tunneling
- **DDNS Integration** - DNS updates for Cloudflare, Tencent Cloud, HE, Webhook, and Dummy providers
- **MCP Integration** - Model Context Protocol REST compatibility and `/mcp` JSON-RPC tools
- **Web Dashboard** - Next.js management interface for servers, services, alerts, tasks, DDNS, NAT, terminal, and settings
- **Public Status Page** - unauthenticated `/status` view backed by `/api/v1/public/status`
- **BOLD Theme UI** - BOLD.-style neo-brutalist palette with explicit light/dark switching
- **Multi-user RBAC** - role-based access control, PAT scopes, CSRF protection, and server allowlists

## 🚀 Quick Start

### Using Docker Compose (Recommended)

Docker files and Compose files are validated by the M9 smoke script. Use this for local development/testing first, then run your own 24-hour soak before production.

```bash
# Clone the repository
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

# Start with SQLite
docker compose up -d

# Or start with PostgreSQL
docker compose -f docker-compose.pg.yml up -d
```

Open:

- Web UI: http://localhost:3000
- API: http://localhost:8080

Default credentials: `admin` / `admin123`

The public status page is available before login at `http://localhost:3000/status`. The dashboard theme uses the BOLD. palette; switch between light and dark modes from the navigation bar.

SQLite Compose creates `./data/xlstatus.db` on first startup. PostgreSQL Compose creates the `xlstatus` role and database on an empty volume, then XLStatus applies application migrations.
Compose sets `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000` so the Web UI can call the API at `http://localhost:8080`.

When running SQLite from source, keep `?mode=rwc` or set `DATABASE_CREATE_IF_MISSING=true`. If the database file is missing and auto-create is not enabled, interactive runs ask whether to create it and non-interactive runs exit with a clear error. PostgreSQL new-site setup is covered in the [Installation Guide](./docs/installation.md#postgresql-new-site).

A foreground server should keep running. For smoke tests, wrap it with `timeout 8s`; exit code `124` means it stayed alive until the timeout. If it returns directly to the shell, check the printed `Error:` or systemd logs, especially for `8080`/`50051` port conflicts.

### Using Install Script

**Note**: Pre-built binaries are not yet available. Build from source first.

```bash
# Build from source
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus
cargo build --release

# Install server
sudo BINARY_PATH=target/release/xlstatus-server bash deploy/install.sh

# Install agent on monitored servers
sudo BINARY_PATH=target/release/xlstatus-agent bash deploy/install-agent.sh
```

## 📚 Documentation

- [Documentation Index](./docs/README.md)
- [Architecture](./plan/02-architecture.md)
- [Installation Guide](./docs/installation.md)
- [Configuration](./docs/configuration.md)
- [API Documentation](./docs/api.md)
- [Agent Setup](./docs/agent.md)
- [Operations](./docs/operations.md)
- [Troubleshooting](./docs/troubleshooting.md)

## ⚙️ Configuration Essentials

XLStatus supports two server configuration modes:

- Environment variables: set `DATABASE_URL`, then provide values such as `HTTP_BIND`, `GRPC_BIND`, `CORS_ALLOWED_ORIGINS`, and `SESSION_SECRET`.
- TOML file: copy [config.example.toml](./config.example.toml) to `config.toml` or `/etc/xlstatus/server.toml`, then start with `CONFIG_FILE=/path/to/server.toml`.

Do not set `DATABASE_URL` when using `CONFIG_FILE`; `DATABASE_URL` selects environment-variable mode.

When the Web UI and API run on different origins, the API must allow the Web UI origin:

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

The Web UI uses `NEXT_PUBLIC_API_URL` to know where the API is:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

See [docs/configuration.md](./docs/configuration.md) for the full matrix, including SQLite creation behavior and PostgreSQL new-site initialization.

## 🛠️ Development

### Prerequisites

- Rust 1.75+
- Node.js 20+ with Corepack/pnpm
- PostgreSQL 15+ or SQLite 3.40+

### Build from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

# Build server
cargo build --release --bin xlstatus-server

# Build agent
cargo build --release --bin xlstatus-agent

# Build web interface
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

### Run in Development

```bash
# Terminal 1: Start server
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000 cargo run --bin xlstatus-server

# Terminal 2: Start web interface
cd web
corepack enable
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev

# Terminal 3: Start agent
cargo run --bin xlstatus-agent
```

If the Web UI uses a different port, add that exact origin to `CORS_ALLOWED_ORIGINS` before starting the server.

For a production-style source run, start the Rust server first, then run:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

## 📦 Tech Stack

### Backend
- **Rust** - Systems programming language
- **Tokio** - Async runtime
- **Axum** - Web framework
- **Tonic** - gRPC framework
- **SQLx** - Database toolkit (SQLite/PostgreSQL)

### Frontend
- **Next.js 16** - React framework
- **TypeScript** - Type safety
- **Tailwind CSS** - Utility-first CSS

### Infrastructure
- **gRPC** - Agent communication
- **WebSocket** - Real-time updates
- **Docker** - Containerization

## 🏗️ Project Structure

```
XLStatus/
├── crates/
│   ├── server/          # Dashboard server
│   ├── agent/           # Monitoring agent
│   ├── shared/          # Shared types and utilities
│   ├── proto-gen/       # Generated protobuf code
│   └── tsdb/            # Time-series database
├── web/                 # Next.js web interface
├── proto/               # Protobuf definitions
├── deploy/              # Deployment scripts and configs
└── docs/                # Documentation
```

## 🔒 Security

- Argon2 password hashing
- Ed25519 agent authentication
- JWT-based sessions
- CSRF protection
- Audit logging
- Rate limiting

## 📊 Performance

- Verified dry-run load plan: 100 agents with 3-second reporting intervals over a 24-hour window
- Planned target: 1000+ service monitors with 30-second checks
- Planned target: query response P95 < 500ms for 30-day data
- Literal wall-clock 24-hour stability should still be run in the target deployment environment

## 🤝 Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) first.

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by Nezha monitoring system
- Built with modern Rust and React ecosystem
- Community feedback and contributions

## 📞 Support

- 📧 Email: support@xlstatus.io
- 💬 Discord: https://discord.gg/xlstatus
- 🐛 Issues: https://github.com/yourusername/xlstatus/issues

## 🗺️ Roadmap

- [x] M0-M9: scaffold, base platform, agent onboarding, real-time monitoring, service monitoring and alerts, operations, DDNS/NAT/MCP, frontend, performance tooling, and release smoke are covered by runnable acceptance scripts
- [ ] Multi-node Dashboard clustering
- [ ] Windows and macOS agent support
- [ ] Mobile applications

---

Made with ❤️ by the XLStatus team
