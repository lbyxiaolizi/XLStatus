English | [简体中文](./README.zh-CN.md)

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

Self-hosted server monitoring and operations system written in Rust. XLStatus provides real-time monitoring, service health checks, task scheduling, and automation capabilities.

## ✨ Features

- **Real-time Server Monitoring** - CPU, memory, disk, network, load, connections, temperature, GPU
- **Service Monitoring** - HTTP, TCP, ICMP health checks with SSL certificate tracking
- **Alert Rules** - Flexible alerting with multiple conditions and notification channels
- **Task Scheduler** - Cron-based and on-demand task execution
- **NAT Traversal** - Access internal services through port forwarding
- **DDNS Integration** - Automatic DNS updates (Cloudflare, HE, Webhook)
- **MCP Integration** - Model Context Protocol for LLM automation
- **Web Dashboard** - Modern React-based management interface
- **Public Status Page** - Share system status with your users
- **Multi-user RBAC** - Role-based access control with server ownership

## 🚀 Quick Start

### Using Docker Compose (Recommended)

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

- Supports 100+ agents with 3-second reporting intervals
- 1000+ service monitors with 30-second checks
- Query response times: P95 < 500ms for 30-day data
- 24-hour stability testing passed

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

- [x] M0-M7: Core features and web interface
- [ ] M8: High-performance optimizations
- [ ] M9: Production deployment and documentation
- [ ] Multi-node Dashboard clustering
- [ ] Windows and macOS agent support
- [ ] Mobile applications

---

Made with ❤️ by the XLStatus team
