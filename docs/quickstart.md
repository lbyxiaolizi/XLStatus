# Quick Start

## Docker Compose

```bash
docker compose up -d
curl -fsS http://localhost:8080/healthz
docker compose ps
```

Open:

- API: http://localhost:8080
- Web UI: http://localhost:3000

The default Compose file seeds `admin` / `admin123` for local testing.
SQLite mode creates `./data/xlstatus.db` on first startup.

PostgreSQL variant:

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
```

The PostgreSQL Compose file creates the `xlstatus` role and database on an empty volume; XLStatus applies application migrations on first startup.

## Build From Source

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
```

Run the server:

```bash
mkdir -p ./data
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
DATABASE_CREATE_IF_MISSING=true \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

If the SQLite file is missing and neither `?mode=rwc` nor `DATABASE_CREATE_IF_MISSING=true` is set, an interactive run asks whether to create it. Non-interactive systemd/Docker starts fail with a clear message so data is not created in the wrong place.

Minimal PostgreSQL new-site setup:

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL

DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
SESSION_SECRET="$(openssl rand -hex 32)" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

The first startup runs embedded application migrations. See [configuration.md](./configuration.md) for all configuration options.

Enroll and run an agent:

```bash
xlstatus-agent enroll \
  --server http://localhost:8080 \
  --grpc-server http://localhost:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config ./agent.json

xlstatus-agent run --config ./agent.json
```

## Verify M9 Install Flow

```bash
cargo build --bin xlstatus-server --bin xlstatus-agent
test-run/verify-m9-install.sh
```

## Troubleshooting

```bash
docker compose logs server
docker compose logs web
sudo journalctl -u xlstatus -f
sudo journalctl -u xlstatus-agent -f
```

See [installation.md](./installation.md), [agent-setup.md](./agent-setup.md), and [troubleshooting.md](./troubleshooting.md).
