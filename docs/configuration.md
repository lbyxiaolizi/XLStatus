# Configuration

This document describes the configuration that is implemented in the current XLStatus binaries.

## Server Loading Order

The server loads configuration in this order:

1. Environment variables when `DATABASE_URL` is set.
2. TOML file pointed to by `CONFIG_FILE`.
3. Development defaults.

Environment variables override the TOML file only when `DATABASE_URL` is present. If you want file-based configuration, set only `CONFIG_FILE`.

## Environment Variables

```env
DATABASE_URL=sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc
DATABASE_CREATE_IF_MISSING=true
HTTP_BIND=0.0.0.0:8080
GRPC_BIND=0.0.0.0:50051
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
SESSION_SECRET=replace-with-a-long-random-secret
SESSION_TTL_HOURS=24
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=admin123
```

`XLSTATUS_SEED_ADMIN_PASSWORD` is only used to seed an admin user when one does not already exist.

| Variable | Required | Description |
|---|---:|---|
| `DATABASE_URL` | Yes for env mode | SQLite or PostgreSQL connection URL. Setting this enables environment-variable configuration mode. |
| `DATABASE_CREATE_IF_MISSING` | No | SQLite-only convenience flag. Truthy values are `1`, `true`, `yes`, `y`, and `on`. |
| `HTTP_BIND` | No | HTTP API bind address. Default: `0.0.0.0:8080`. |
| `GRPC_BIND` | No | Agent gRPC bind address. Default: `0.0.0.0:50051`. |
| `CORS_ALLOWED_ORIGINS` | No | Comma-separated exact browser origins that may call the API. Default allows local Next.js on port `3000`. |
| `SESSION_SECRET` | Production: yes | Secret used for sessions and agent JWT signing. Generate a long random value for real deployments. |
| `SESSION_TTL_HOURS` | No | Cookie session lifetime. Default: `24`. |
| `XLSTATUS_SEED_ADMIN_USERNAME` | No | Optional first admin username. |
| `XLSTATUS_SEED_ADMIN_PASSWORD` | No | Optional first admin password. Used only when that user does not already exist. |

## TOML File

Copy [../config.example.toml](../config.example.toml) to your target path and edit it:

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
session_secret = "replace-with-a-long-random-secret"
session_ttl_hours = 24
```

Run with a config file:

```bash
CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

The server currently has no `--config`, `--validate`, or `--version` CLI flags.

Important loading behavior:

- If `DATABASE_URL` is set, XLStatus reads environment variables and ignores `CONFIG_FILE`.
- If `DATABASE_URL` is not set and `CONFIG_FILE` points to an existing file, XLStatus reads that TOML file.
- If neither is present, development defaults are used.
- Do not set both `DATABASE_URL` and `CONFIG_FILE` expecting a merge; choose one mode per process.

## Web UI CORS

When the Web UI and API run on different origins, for example `http://localhost:3000` and `http://localhost:8080`, the API must allow the browser origin. Configure exact origins with `CORS_ALLOWED_ORIGINS` or `server.cors_allowed_origins`.

Use the same hostname style for both URLs during local testing. For example, pair `http://localhost:3000` with `http://localhost:8080`, or pair `http://127.0.0.1:3000` with `http://127.0.0.1:8080`, so cookie sessions and CSRF checks behave consistently.

Wildcard CORS origins are not supported because XLStatus uses cookie credentials for the dashboard.

Common examples:

```bash
# Next.js dev server on the default port
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000

# Custom frontend port
CORS_ALLOWED_ORIGINS=http://localhost:3001,http://127.0.0.1:3001

# Public reverse-proxy origin
CORS_ALLOWED_ORIGINS=https://status.example.com
```

`NEXT_PUBLIC_API_URL` is a Web UI build/runtime setting. It tells browser code where the API lives:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

The API origin in `NEXT_PUBLIC_API_URL` and the browser origin used to open the Web UI must both be planned together. If the browser opens `http://localhost:3000`, the API server must include `http://localhost:3000` in CORS. If the browser opens `https://status.example.com`, include `https://status.example.com`.

## Web UI i18n

The Web UI locale configuration is implemented in [../web/lib/i18n.ts](../web/lib/i18n.ts).

Current settings:

- Default locale: `zh-CN`
- Supported locales: `zh-CN`
- App Router i18n configuration is exported from `web/lib/i18n.ts`; `web/next.config.ts` intentionally does not use the legacy Pages Router `i18n` field
- Shared user-visible strings should be added to `zhCN` in `web/lib/i18n.ts`
- Backend protocol values, enum values, and scope strings such as `server:read` should stay unchanged

The root layout sets `<html lang="zh-CN">`, and dates are formatted with the `zh-CN` locale.

## SQLite

SQLite is the simplest mode for a single-node install:

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true
```

Startup behavior when the database file is missing:

- If the URL contains `?mode=rwc`, XLStatus creates the file.
- If `create_if_missing = true` or `DATABASE_CREATE_IF_MISSING=true`, XLStatus creates the file.
- If neither is set and the server is started from an interactive terminal, XLStatus asks whether to create the file.
- If neither is set and the server runs non-interactively, startup fails with a clear message instead of silently creating data in the wrong place.

The parent directory is created automatically when creation is allowed. For systemd installs, make sure the service user can write it:

```bash
sudo mkdir -p /var/lib/xlstatus
sudo chown -R xlstatus:xlstatus /var/lib/xlstatus
```

Use `?mode=rw` when you intentionally want startup to fail if the file is missing:

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rw"
create_if_missing = false
```

## PostgreSQL

PostgreSQL is recommended when the database is managed separately, backed up centrally, or shared with production operations tooling.

XLStatus runs its own schema migrations after it connects, but it does not create the PostgreSQL role or database. Create them before first start:

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

Test the login:

```bash
psql 'postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' -c 'select 1;'
```

Then configure XLStatus:

```toml
[database]
url = "postgresql://xlstatus:change-this-password@localhost:5432/xlstatus"
create_if_missing = false
```

Equivalent environment variable:

```bash
DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus'
```

For a fresh site, the first XLStatus startup creates all application tables through embedded migrations. Keep the database empty before that first run unless you are restoring from a backup made by the same application version.

## Agent

The agent config is generated by enrollment and stored as JSON:

```bash
xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json
```

Generated shape:

```json
{
  "server": "http://dashboard.example.com:8080",
  "grpc_server": "http://dashboard.example.com:50051",
  "agent_id": "...",
  "name": "web-01",
  "public_key": "...",
  "private_key": "..."
}
```

Run:

```bash
xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

Keep the agent config at mode `0600`; it contains private key material.

## Docker Compose

The Compose files configure the server through environment variables. Render the effective setup with:

```bash
docker compose config
docker compose -f docker-compose.pg.yml config
```

The SQLite Compose files set `DATABASE_CREATE_IF_MISSING=true` and use `?mode=rwc`, so a new local data volume starts cleanly.

The PostgreSQL Compose file uses the official `postgres:15` image with `POSTGRES_USER`, `POSTGRES_PASSWORD`, and `POSTGRES_DB`; the image creates the role and database on an empty volume, then XLStatus applies its application migrations.

The demo agent service is behind the `agent-demo` profile because it needs a pre-enrolled config mounted at `/etc/xlstatus-agent/agent.json`.
