English | [简体中文](./README.zh-CN.md)

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

XLStatus is a self-hosted server monitoring and operations panel written in Rust and Next.js. It combines real-time host metrics, public status pages, service probes, alerting, task execution, file operations, terminal sessions, DDNS, NAT tunnels, and MCP tools in one deployable stack.

The current release is `v0.1`. Start from the documentation index for installation, operations, and development details: [docs/README.md](./docs/README.md).

## Features

- Real-time agent monitoring for CPU, memory, disk, network, load, connections, process count, GPU, and temperatures.
- Service monitoring with HTTP, TCP, ICMP, HTTPS certificate metadata, uptime history, and alert rules.
- Agent operations: task scheduler, command execution, file read/write/download/upload, web terminal, config push, and force update hooks.
- Network utilities: DDNS providers, NAT reverse tunnels, GeoIP metadata, and public world map distribution.
- Dashboard: Chinese-first Next.js UI, RBAC, PAT scopes, CSRF protection, audit-friendly API boundaries, public `/status` page, and theme settings.
- Release path: Docker Compose, source builds, Linux systemd install scripts, and multi-platform GitHub Release assets.

## Quick Start

```bash
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus
mkdir -p .secrets
printf '%s\n' 'replace-with-a-strong-initial-password' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password
docker compose up -d
curl -fsS http://localhost:8080/healthz
```

Open:

- Web UI: `http://localhost:3000`
- API: `http://localhost:8080`
- Public status: `http://localhost:3000/status`

Docker Compose publishes Agent gRPC on `0.0.0.0:50051` by default so remote
agents can connect without per-IP firewall allowlisting. Keep `8080` and
`3000` behind localhost or your reverse proxy in production.

Set a strong initial password in `.secrets/xlstatus_seed_admin_password` before first start.

For PostgreSQL:

```bash
docker compose -f docker-compose.pg.yml up -d
```

## Release Install Scripts

Server:

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1/install-server.sh | sudo bash
```

Agent:

```bash
sudo SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=https://grpc.dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1/install-agent.sh | bash'
```

The dashboard Settings page can generate a parameterized Agent bootstrap link. By default it fetches the newest non-draft GitHub Release version and falls back to `v0.1` if GitHub is unavailable.

## Documentation

- [Documentation Index](./docs/README.md)
- [Quick Start](./docs/quickstart.md)
- [Installation](./docs/installation.md)
- [Configuration](./docs/configuration.md)
- [Agent Setup](./docs/agent.md)
- [Web Frontend](./docs/web.md)
- [API Overview](./docs/api.md)
- [Operations](./docs/operations.md)
- [Troubleshooting](./docs/troubleshooting.md)
- [Development](./docs/development.md)
- [Release Checklist](./docs/release-checklist.md)
- [Architecture](./docs/architecture.md)
- [Project Structure](./docs/project-structure.md)

Historical planning notes live under [docs/archive](./docs/archive/) and are not part of the current user documentation path.

## Source Build

```bash
cargo build --release --bin xlstatus-server --bin xlstatus-agent

corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

Run the release binaries with either environment variables or `CONFIG_FILE`; see [docs/configuration.md](./docs/configuration.md).

## Verification

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace

cd web
pnpm lint
pnpm typecheck
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

The repository also keeps acceptance scripts in `test-run/`. Some scripts start local services, require Docker/PostgreSQL, or occupy fixed ports, so read each script header before running it.

## Repository Layout

```text
crates/       Rust workspace crates for server, agent, shared code, proto-gen, TSDB, and xtask
web/          Next.js dashboard and public status UI
proto/        gRPC protobuf definitions
deploy/       systemd unit templates and Linux install scripts
docs/         current documentation plus archived planning notes
test-run/     repeatable acceptance and smoke scripts
```

## License

MIT. See [LICENSE](./LICENSE).
