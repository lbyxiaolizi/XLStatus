# Troubleshooting

This page covers the deployment paths that exist in the current repository: Docker Compose, `xlstatus.service`, and `xlstatus-agent.service`.

## Server

Check process health:

```bash
curl -fsS http://localhost:8080/healthz
sudo systemctl status xlstatus
sudo journalctl -u xlstatus -n 100 --no-pager
```

Run in the foreground with the same config used by systemd:

```bash
sudo -u xlstatus CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

Common causes:

- Port conflict: `sudo ss -tlnp | grep -E '8080|50051'`
- Bad config path: confirm `Environment="CONFIG_FILE=..."` in `/etc/systemd/system/xlstatus.service`
- SQLite permissions: `sudo chown -R xlstatus:xlstatus /var/lib/xlstatus`
- PostgreSQL URL mismatch: test the exact `DATABASE_URL` with `psql`

The server currently has no `--config`, `--validate`, or `--version` CLI flags. Use `CONFIG_FILE` or environment variables.

## Docker Compose

Render the effective config:

```bash
docker compose config
docker compose -f docker-compose.pg.yml config
```

Check logs:

```bash
docker compose logs server
docker compose logs web
docker compose -f docker-compose.pg.yml logs postgres
```

Reset local SQLite data:

```bash
docker compose down
rm -f ./data/xlstatus.db
docker compose up -d
```

Reset the PostgreSQL volume:

```bash
docker compose -f docker-compose.pg.yml down -v
docker compose -f docker-compose.pg.yml up -d
```

## Agent

Check service state:

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

Run manually:

```bash
sudo /usr/local/bin/xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

Re-enroll:

```bash
sudo /usr/local/bin/xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config /etc/xlstatus-agent/agent.json
```

Common causes:

- `--server` points to gRPC instead of HTTP.
- `--grpc-server` is missing or unreachable.
- Enrollment token expired or was already used.
- Config file is missing private key material or has loose permissions.

## Backup And Restore

There are no implemented `xlstatus-server backup` or `restore` subcommands yet.

SQLite backup:

```bash
sudo systemctl stop xlstatus
sudo cp /var/lib/xlstatus/xlstatus.db /var/lib/xlstatus/xlstatus.db.$(date +%Y%m%d%H%M%S).bak
sudo tar czf xlstatus-config-data.tgz /etc/xlstatus /var/lib/xlstatus
sudo systemctl start xlstatus
```

SQLite restore:

```bash
sudo systemctl stop xlstatus
sudo cp /path/to/xlstatus.db.bak /var/lib/xlstatus/xlstatus.db
sudo chown xlstatus:xlstatus /var/lib/xlstatus/xlstatus.db
sudo systemctl start xlstatus
curl -fsS http://localhost:8080/healthz
```

PostgreSQL backup:

```bash
pg_dump 'postgres://xlstatus:xlstatus_password@localhost:5432/xlstatus' > xlstatus.sql
```

PostgreSQL restore:

```bash
psql 'postgres://xlstatus:xlstatus_password@localhost:5432/xlstatus' < xlstatus.sql
```

Restore into the same application version that created the backup unless a migration plan has been tested.
