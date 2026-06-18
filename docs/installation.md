# Installation Guide

This guide reflects the current M9 release-stability artifacts in this repository. XLStatus is still under active development; use these steps for repeatable local or lab deployment, and review [implementation-audit.md](./implementation-audit.md) before production use.

## Requirements

- Linux x86_64 for systemd installs
- Docker 20.10+ and Docker Compose v2 for container installs
- Rust toolchain when building binaries from source
- Node.js 20+ with Corepack/pnpm when building the web image or running the frontend from source

## Docker Compose

SQLite stack:

```bash
docker compose up -d
docker compose ps
curl -fsS http://localhost:8080/healthz
```

The first SQLite startup creates `./data/xlstatus.db` because Compose sets `DATABASE_CREATE_IF_MISSING=true` and uses `?mode=rwc`.
The full Compose stacks also set `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000` for the bundled Web UI.

PostgreSQL stack:

```bash
docker compose -f docker-compose.pg.yml up -d
docker compose -f docker-compose.pg.yml ps
curl -fsS http://localhost:8080/healthz
```

On a new Compose volume, the `postgres:15` image creates the `xlstatus` role and database from `POSTGRES_USER`, `POSTGRES_PASSWORD`, and `POSTGRES_DB`; XLStatus then runs its embedded schema migrations on first server start.

The Compose files build three images from the repository:

- `server`: Rust dashboard API and gRPC server, HTTP on `8080`, gRPC on `50051`.
- `web`: Next.js dashboard on `3000`.
- `agent-demo`: disabled by default behind the `agent-demo` profile because the agent must be enrolled before it has a usable config.

The server accepts these environment variables:

```env
DATABASE_URL=sqlite:///data/xlstatus.db?mode=rwc
DATABASE_CREATE_IF_MISSING=true
HTTP_BIND=0.0.0.0:8080
GRPC_BIND=0.0.0.0:50051
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
SESSION_SECRET=change-me-in-production
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=admin123
```

## Source Build

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

Run a local server with SQLite:

```bash
mkdir -p ./data
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
DATABASE_CREATE_IF_MISSING=true \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

If you omit both `?mode=rwc` and `DATABASE_CREATE_IF_MISSING=true`, an interactive terminal asks whether to create the SQLite file. Non-interactive starts fail with a clear message so systemd or Docker do not silently create data in the wrong place.

You can use a TOML file instead of environment variables:

```bash
cp config.example.toml ./config.toml
CONFIG_FILE=./config.toml ./target/release/xlstatus-server
```

Do not set `DATABASE_URL` when using `CONFIG_FILE`; `DATABASE_URL` selects environment-variable configuration mode.

### PostgreSQL New Site

Install PostgreSQL 15+ and create an empty role/database before the first XLStatus start:

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

Check the connection:

```bash
psql 'postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' -c 'select 1;'
```

Start XLStatus against that database:

```bash
DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="$(openssl rand -hex 32)" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

XLStatus creates application tables through embedded migrations. Do not pre-load unrelated tables into a fresh XLStatus database.

Run the web dashboard from source after the server is healthy:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

For a production-style source run after `pnpm build`:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

Open the dashboard at `http://localhost:3000`; the frontend calls the server API configured by `NEXT_PUBLIC_API_URL`.
The unauthenticated public status page is `http://localhost:3000/status` and reads `GET /api/v1/public/status`.
The management dashboard uses the BOLD. neo-brutalist palette; the navigation bar exposes explicit light/dark choices and stores the preference in `localStorage.darkMode`.
If the Web UI runs on a different host, scheme, or port, add that exact browser origin to `CORS_ALLOWED_ORIGINS` or `server.cors_allowed_origins` before starting the API server.

The server also supports `CONFIG_FILE=/path/to/server.toml`:

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true

[server]
http_bind = "0.0.0.0:8080"
grpc_bind = "0.0.0.0:50051"
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
]

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

To install systemd mode with PostgreSQL, create the PostgreSQL role/database first, then pass the URL:

```bash
sudo BINARY_PATH=target/release/xlstatus-server \
  DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
  DATABASE_CREATE_IF_MISSING=false \
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
pg_dump 'postgresql://xlstatus:xlstatus_password@localhost:5432/xlstatus' > xlstatus.sql
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
