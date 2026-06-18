# Installation Guide

This guide reflects the current M9 release-stability artifacts in this repository. XLStatus is still under active development; use these steps for repeatable local or lab deployment, and review [implementation-audit.md](./implementation-audit.md) before production use.

## Requirements

- Linux x86_64 for systemd installs
- Docker 20.10+ and Docker Compose v2 for container installs
- Rust toolchain when building binaries from source
- Node.js 20+ only when building the web image or running the frontend from source

## Docker Compose

SQLite stack:

```bash
docker compose up -d
docker compose ps
curl -fsS http://localhost:8080/healthz
```

PostgreSQL stack:

```bash
docker compose -f docker-compose.pg.yml up -d
docker compose -f docker-compose.pg.yml ps
curl -fsS http://localhost:8080/healthz
```

The Compose files build three images from the repository:

- `server`: Rust dashboard API and gRPC server, HTTP on `8080`, gRPC on `50051`.
- `web`: Next.js dashboard on `3000`.
- `agent-demo`: disabled by default behind the `agent-demo` profile because the agent must be enrolled before it has a usable config.

The server accepts these environment variables:

```env
DATABASE_URL=sqlite:///data/xlstatus.db?mode=rwc
HTTP_BIND=0.0.0.0:8080
GRPC_BIND=0.0.0.0:50051
SESSION_SECRET=change-me-in-production
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=admin123
```

## Source Build

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
```

Run a local server with SQLite:

```bash
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

The server also supports `CONFIG_FILE=/path/to/server.toml`:

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"

[server]
http_bind = "0.0.0.0:8080"
grpc_bind = "0.0.0.0:50051"

[security]
session_secret = "replace-me-with-a-long-random-secret"
session_ttl_hours = 24
```

## systemd Server Install

Pre-built release binaries are not published yet, so build first and pass `BINARY_PATH`:

```bash
cargo build --release --bin xlstatus-server
sudo BINARY_PATH=target/release/xlstatus-server \
  ADMIN_PASSWORD='admin123' \
  bash deploy/install.sh
```

The script installs:

- Binary symlink: `/usr/local/bin/xlstatus-server`
- Config: `/etc/xlstatus/server.toml`
- Data directory: `/var/lib/xlstatus`
- Unit: `/etc/systemd/system/xlstatus.service`

Useful commands:

```bash
sudo systemctl status xlstatus
sudo journalctl -u xlstatus -f
curl -fsS http://localhost:8080/healthz
```

## Agent Install

Create an enrollment token in the dashboard or with the admin API, then install and enroll the agent:

```bash
cargo build --release --bin xlstatus-agent
sudo BINARY_PATH=target/release/xlstatus-agent \
  SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=http://dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash deploy/install-agent.sh
```

The agent CLI currently supports:

```bash
xlstatus-agent enroll --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json

xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

The enrollment config is JSON and contains the agent id, dashboard HTTP URL, gRPC URL, public key, and private key. Keep it mode `0600`.

## Backup And Restore

The planned `xlstatus-server backup` and `restore` subcommands are not implemented yet. For current builds, use database and filesystem backups.

SQLite:

```bash
sudo systemctl stop xlstatus
sudo cp /var/lib/xlstatus/xlstatus.db /var/lib/xlstatus/xlstatus.db.$(date +%Y%m%d%H%M%S).bak
sudo tar czf xlstatus-config-data.tgz /etc/xlstatus /var/lib/xlstatus
sudo systemctl start xlstatus
```

PostgreSQL:

```bash
pg_dump 'postgres://xlstatus:xlstatus_password@localhost:5432/xlstatus' > xlstatus.sql
```

Restore into the same application version that produced the backup, then start the service and check `/healthz`.

## OpenAPI

The workspace includes `utoipa`, but the server does not currently expose or generate a complete OpenAPI document. Treat [api.md](./api.md) as a hand-maintained reference for now and update it when routes change.

## Verification

Run the deterministic M9 install verifier after building debug binaries:

```bash
cargo build --bin xlstatus-server --bin xlstatus-agent
test-run/verify-m9-install.sh
```

The script checks server startup, `/healthz`, admin login, enrollment-token creation, agent enrollment, and a short agent gRPC run.
