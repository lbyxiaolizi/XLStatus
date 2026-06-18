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
- Public status: http://localhost:3000/status

The default Compose file seeds `admin` / `admin123` for local testing.
SQLite mode creates `./data/xlstatus.db` on first startup.
The web UI uses the BOLD. neo-brutalist palette and stores the explicit light/dark choice in `localStorage.darkMode`.
Compose also sets `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000`, so the browser can call the API at `http://localhost:8080`.

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
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

Run the server:

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

If the SQLite file is missing and neither `?mode=rwc` nor `DATABASE_CREATE_IF_MISSING=true` is set, an interactive run asks whether to create it. Non-interactive systemd/Docker starts fail with a clear message so data is not created in the wrong place.

Equivalent `config.toml` flow:

```bash
cp config.example.toml ./config.toml
SESSION_SECRET_VALUE="$(openssl rand -hex 32)"
sed -i.bak "s/replace-with-a-long-random-secret/${SESSION_SECRET_VALUE}/" ./config.toml
CONFIG_FILE=./config.toml ./target/release/xlstatus-server
```

When using `CONFIG_FILE`, do not set `DATABASE_URL` in the same process. Setting `DATABASE_URL` switches the server to environment-variable configuration mode.

Minimal PostgreSQL new-site setup:

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL

DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="$(openssl rand -hex 32)" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

The first startup runs embedded application migrations. See [configuration.md](./configuration.md) for all configuration options.

Run the Web UI from source in another terminal:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

Open `http://localhost:3000/status` before login to confirm the public status API is reachable. After login, use the navigation theme switch to choose the BOLD. light or dark palette.
If you run the Web UI on a different port, add that exact origin to `CORS_ALLOWED_ORIGINS` before starting the server.
Use matching hostname styles for local cookie sessions: pair `localhost` with `localhost`, or `127.0.0.1` with `127.0.0.1`.

For a production-style local run after `pnpm build`:

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

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
