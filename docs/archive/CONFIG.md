# XLStatus Configuration Guide

**Version**: v1.0.0
**Last Updated**: 2026-06-17

---

## Table of Contents

1. [Server Configuration](#server-configuration)
2. [Agent Configuration](#agent-configuration)
3. [Environment Variables](#environment-variables)
4. [Database Setup](#database-setup)
5. [Docker Configuration](#docker-configuration)
6. [Production Deployment](#production-deployment)

---

## Server Configuration

### Configuration File

Create a `config.toml` file for the server:

```toml
# XLStatus Server Configuration

[server]
# HTTP API listening address
http_bind = "0.0.0.0:8080"

# gRPC service listening address
grpc_bind = "0.0.0.0:50051"

# Session secret (MUST change in production)
# Generate with: openssl rand -base64 32
session_secret = "change-me-in-production-use-random-secret"

[database]
# Database connection URL
# SQLite: sqlite:///path/to/xlstatus.db
# PostgreSQL: postgres://user:password@host:port/database
url = "sqlite:///data/xlstatus.db"

# Maximum number of connections
max_connections = 10

# Connection timeout (seconds)
connect_timeout = 30

[auth]
# Session lifetime (seconds)
session_lifetime = 86400  # 24 hours

# JWT expiration time (seconds)
jwt_lifetime = 3600  # 1 hour

# Password hashing parameters (Argon2)
password_memory_cost = 65536  # 64 MB
password_time_cost = 3
password_parallelism = 4

[agent]
# Agent heartbeat timeout (seconds)
heartbeat_timeout = 60

# Agent reconnect interval (seconds)
reconnect_interval = 30

[logging]
# Log level: error, warn, info, debug, trace
level = "info"

# Log format: json, pretty
format = "pretty"

[metrics]
# Metrics retention period (days)
retention_days = 30

# Sampling interval (seconds)
sample_interval = 60

[security]
# Enable CORS
enable_cors = true

# Allowed origins
cors_origins = ["http://localhost:3000", "https://yourdomain.com"]

# CSRF protection
enable_csrf = true

[features]
# Enable service monitoring
enable_service_monitor = true

# Enable alerts
enable_alerts = true

# Enable NAT traversal
enable_nat = false

# Enable DDNS
enable_ddns = false

# Enable MCP protocol
enable_mcp = false

[defaults]
# Default admin username
admin_username = "admin"

# Default admin password (created on first startup)
admin_password = "admin123"
```

### Usage

```bash
# Start with configuration file
./xlstatus-server --config /path/to/config.toml

# Environment variables take precedence
DATABASE_URL="sqlite:///data/xlstatus.db" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="your-secret" \
./xlstatus-server
```

---

## Agent Configuration

### Configuration File

Create a `config.toml` file for the agent:

```toml
# XLStatus Agent Configuration

[agent]
# Agent name (unique identifier)
name = "agent-1"

# Server connection URL
server_url = "http://localhost:8080"

# Server gRPC address
server_grpc = "localhost:50051"

# Report interval (seconds)
report_interval = 10

# Reconnect interval on failure (seconds)
reconnect_interval = 30

[auth]
# Agent ID (obtained after enrollment)
agent_id = ""

# Agent private key path (Ed25519)
private_key_path = "/etc/xlstatus/agent.key"

[collectors]
# Enable CPU monitoring
enable_cpu = true

# Enable memory monitoring
enable_memory = true

# Enable disk monitoring
enable_disk = true

# Enable network monitoring
enable_network = true

# Enable load monitoring
enable_load = true

# Enable temperature monitoring
enable_temperature = true

# Enable GPU monitoring (if available)
enable_gpu = true

[logging]
# Log level: error, warn, info, debug, trace
level = "info"

# Log format: json, pretty
format = "pretty"
```

### Usage

```bash
# Enroll agent (first time)
./xlstatus-agent enroll \
  --server http://localhost:8080 \
  --token <enrollment-token>

# Start agent
./xlstatus-agent --config /path/to/config.toml

# Or with environment variables
SERVER_URL="http://localhost:8080" \
AGENT_NAME="my-server" \
./xlstatus-agent
```

---

## Environment Variables

### Server Environment Variables

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `DATABASE_URL` | Database connection string | `sqlite:///data/xlstatus.db` | Yes |
| `HTTP_BIND` | HTTP listening address | `0.0.0.0:8080` | Yes |
| `GRPC_BIND` | gRPC listening address | `0.0.0.0:50051` | Yes |
| `SESSION_SECRET` | Session encryption key | - | Yes |
| `RUST_LOG` | Log level | `info` | No |
| `MAX_CONNECTIONS` | Database max connections | `10` | No |

### Agent Environment Variables

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `SERVER_URL` | Server HTTP URL | `http://localhost:8080` | Yes |
| `AGENT_NAME` | Agent identifier | `agent-1` | Yes |
| `REPORT_INTERVAL` | Report interval (seconds) | `10` | No |
| `RUST_LOG` | Log level | `info` | No |

---

## Database Setup

### SQLite (Development)

```bash
# Create database directory
mkdir -p /data

# SQLite will auto-create the database file
DATABASE_URL="sqlite:///data/xlstatus.db" ./xlstatus-server
```

### PostgreSQL (Production)

```bash
# Create database
createdb xlstatus

# Run migrations
export DATABASE_URL="postgres://user:password@localhost:5432/xlstatus"
sqlx migrate run --source crates/server/migrations

# Start server
DATABASE_URL="postgres://user:password@localhost:5432/xlstatus" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="$(openssl rand -base64 32)" \
./xlstatus-server
```

---

## Docker Configuration

### Simple Server Only

Create `docker-compose.simple.yml`:

```yaml
services:
  server:
    build:
      context: .
      dockerfile: Dockerfile.server
    container_name: xlstatus-server
    ports:
      - "8080:8080"
      - "50051:50051"
    volumes:
      - xlstatus-data:/data
    environment:
      - DATABASE_URL=sqlite:///data/xlstatus.db
      - RUST_LOG=info
      - HTTP_BIND=0.0.0.0:8080
      - GRPC_BIND=0.0.0.0:50051
      - SESSION_SECRET=change-me-in-production
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:8080/api/info || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

volumes:
  xlstatus-data:
    driver: local
```

### Full Stack (Server + Web + Agent)

See `docker-compose.yml` for the complete configuration.

---

## Production Deployment

### Security Checklist

- [ ] Change default admin password
- [ ] Generate secure `SESSION_SECRET`
- [ ] Use PostgreSQL instead of SQLite
- [ ] Enable HTTPS (via reverse proxy)
- [ ] Configure firewall rules
- [ ] Set up regular backups
- [ ] Enable audit logging
- [ ] Review CORS settings
- [ ] Disable debug logging

### Generate Secure Secret

```bash
# Generate session secret
openssl rand -base64 32

# Example output:
# 8X9Kp2mQ7vN4jR6sT1wY3zL5hG8bC0dE9fA2gH4iJ6k=
```

### Nginx Reverse Proxy

```nginx
server {
    listen 80;
    server_name xlstatus.yourdomain.com;

    # Redirect to HTTPS
    return 301 https://$server_name$request_uri;
}

server {
    listen 443 ssl http2;
    server_name xlstatus.yourdomain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # HTTP API
    location /api {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # WebSocket (for real-time updates)
    location /ws {
        proxy_pass http://localhost:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }

    # Web frontend
    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

### Systemd Service

Create `/etc/systemd/system/xlstatus-server.service`:

```ini
[Unit]
Description=XLStatus Server
After=network.target

[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory=/opt/xlstatus
Environment="DATABASE_URL=postgres://xlstatus:password@localhost/xlstatus"
Environment="HTTP_BIND=0.0.0.0:8080"
Environment="GRPC_BIND=0.0.0.0:50051"
Environment="SESSION_SECRET=your-secret-here"
Environment="RUST_LOG=info"
ExecStart=/opt/xlstatus/xlstatus-server
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable xlstatus-server
sudo systemctl start xlstatus-server
sudo systemctl status xlstatus-server
```

---

## Configuration Examples

### Development

```bash
# Quick start for development
DATABASE_URL="sqlite://dev.db" \
HTTP_BIND="127.0.0.1:8080" \
GRPC_BIND="127.0.0.1:50051" \
SESSION_SECRET="dev-secret" \
RUST_LOG=debug \
./xlstatus-server
```

### Production

```bash
# Production with PostgreSQL
DATABASE_URL="postgres://xlstatus:secure-password@postgres.internal:5432/xlstatus" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="$(cat /etc/xlstatus/session.secret)" \
RUST_LOG=warn \
MAX_CONNECTIONS=50 \
./xlstatus-server
```

### Docker

```bash
# Simple Docker run
docker run -d \
  --name xlstatus-server \
  -p 8080:8080 \
  -p 50051:50051 \
  -v xlstatus-data:/data \
  -e DATABASE_URL=sqlite:///data/xlstatus.db \
  -e HTTP_BIND=0.0.0.0:8080 \
  -e GRPC_BIND=0.0.0.0:50051 \
  -e SESSION_SECRET=your-secret \
  xlstatus:latest
```

---

## Troubleshooting

### Common Issues

1. **Port already in use**
   ```bash
   # Check what's using the port
   lsof -i :8080
   lsof -i :50051
   ```

2. **Database connection failed**
   ```bash
   # Test PostgreSQL connection
   psql $DATABASE_URL

   # Check SQLite file permissions
   ls -la /data/xlstatus.db
   ```

3. **Session secret error**
   ```bash
   # Generate a new secret
   export SESSION_SECRET=$(openssl rand -base64 32)
   ```

4. **Agent cannot connect**
   ```bash
   # Check server is reachable
   curl http://localhost:8080/api/info

   # Check gRPC port
   grpcurl -plaintext localhost:50051 list
   ```

---

## References

- [Main README](./README.md)
- [Docker Compose Guide](./DOCKER-COMPOSE-GUIDE.md)
- [Project Status](./FINAL-STATUS.md)
- [Architecture Documentation](./plan/02-architecture.md)
- [Security Design](./plan/07-security.md)

---

**Last Updated**: 2026-06-17
**Version**: v1.0.0
