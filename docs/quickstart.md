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

PostgreSQL variant:

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
```

## Build From Source

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
```

Run the server:

```bash
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
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
