English | [简体中文](./README.zh-CN.md)

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

Self-hosted server monitoring and operations system written in Rust. XLStatus provides real-time monitoring, service health checks, task scheduling, and automation capabilities.

## Current Status

XLStatus is in active development. The workspace currently passes `cargo check --workspace`, `cargo test --workspace`, `cd web && pnpm lint`, and `cd web && pnpm build`. M0-M9 have runnable acceptance coverage in `test-run/`; literal 24-hour soak runs still need to be performed in a target deployment environment.

Before deploying or extending the project, read the current implementation audit: [docs/implementation-audit.md](./docs/implementation-audit.md).

## ✨ Features

- **Real-time Server Monitoring** - CPU, memory, disk, network, load, connections, and temperature data from enrolled agents
- **Service Monitoring** - HTTP, TCP, ICMP health checks with HTTPS certificate fingerprint and expiry tracking
- **Alert Rules** - resource, offline, service status, latency, recovery, and webhook notification flows
- **Task Scheduler** - cron-based and on-demand task execution through live agents
- **NAT Traversal** - access to internal services through reverse tunneling
- **DDNS Integration** - DNS updates for Cloudflare, Tencent Cloud, HE, Webhook, and Dummy providers
- **MCP Integration** - Model Context Protocol REST compatibility and `/mcp` JSON-RPC tools
- **Web Dashboard** - Next.js management interface for servers, services, alerts, tasks, DDNS, NAT, terminal, and settings
- **Public Status Page** - public status overview for exposed resources
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

Access the dashboard at http://localhost:8080

Default credentials: `admin` / `admin123`

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

- [Current Implementation Audit](./docs/implementation-audit.md)
- [Documentation Index](./docs/README.md)
- [Architecture](./plan/02-architecture.md)
- [Installation Guide](./docs/installation.md)
- [Configuration](./docs/configuration.md)
- [API Documentation](./docs/api.md)
- [Agent Setup](./docs/agent-setup.md)
- [Troubleshooting](./docs/troubleshooting.md)

## 🛠️ Development

### Prerequisites

- Rust 1.75+
- Node.js 20+
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
cd web
npm install
npm run build
```

### Run in Development

```bash
# Terminal 1: Start server
cargo run --bin xlstatus-server

# Terminal 2: Start web interface
cd web
npm run dev

# Terminal 3: Start agent
cargo run --bin xlstatus-agent
```

## 📦 Tech Stack

### Backend
- **Rust** - Systems programming language
- **Tokio** - Async runtime
- **Axum** - Web framework
- **Tonic** - gRPC framework
- **SQLx** - Database toolkit (SQLite/PostgreSQL)

### Frontend
- **Next.js 14** - React framework
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
