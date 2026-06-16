# Quick Start Guide

Get XLStatus up and running in 5 minutes.

## Prerequisites

- Linux x86_64 system
- Docker and Docker Compose (recommended) OR
- Rust 1.75+ for building from source

## Option 1: Docker Compose (Recommended)

### 1. Clone the Repository

```bash
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus
```

### 2. Start the Stack

**With SQLite (simpler, single-file database):**

```bash
docker compose up -d
```

**With PostgreSQL (better for production):**

```bash
docker compose -f docker-compose.pg.yml up -d
```

### 3. Access the Dashboard

Open your browser and navigate to:

```
http://localhost:8080
```

**Default credentials:**
- Username: `admin`
- Password: `admin123`

⚠️ **Important:** Change the default password immediately!

### 4. Check Status

```bash
# View logs
docker compose logs -f

# Check container status
docker compose ps

# Stop the stack
docker compose down
```

## Option 2: Install Script

### Server Installation

```bash
curl -fsSL https://install.xlstatus.io | bash
```

This will:
- Install XLStatus server to `/opt/xlstatus`
- Create systemd service
- Start the server on port 8080

### Agent Installation

On each server you want to monitor:

```bash
curl -fsSL https://install.xlstatus.io/agent | bash
```

You'll need:
- Server URL (e.g., `http://your-server:8080`)
- Enrollment token (get from Dashboard → Settings → Agents)

## Option 3: Build from Source

### 1. Install Dependencies

```bash
# Ubuntu/Debian
sudo apt-get install build-essential libssl-dev pkg-config

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Build the Server

```bash
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

cargo build --release --bin xlstatus-server
```

### 3. Run the Server

```bash
export DATABASE_URL=sqlite://./dev.db
export BIND_ADDRESS=0.0.0.0:8080
export GRPC_ADDRESS=0.0.0.0:50051

./target/release/xlstatus-server
```

### 4. Build the Web Interface

```bash
cd web
npm install
npm run build
npm start
```

Access at http://localhost:3000

## Next Steps

1. **Change Default Password**
   - Go to Settings → Users
   - Change admin password

2. **Add Your First Agent**
   - Go to Settings → Agents
   - Generate enrollment token
   - Install agent on target server

3. **Configure Service Monitoring**
   - Go to Services
   - Click "Add Service"
   - Configure HTTP/TCP/ICMP checks

4. **Set Up Alerts**
   - Go to Alerts
   - Create alert rules
   - Configure notification channels

5. **Explore Features**
   - View server metrics
   - Schedule tasks
   - Configure NAT port forwarding
   - Set up DDNS

## Troubleshooting

### Server won't start

```bash
# Check logs
docker compose logs server

# Or for systemd
journalctl -u xlstatus -f
```

### Agent won't connect

1. Verify server URL is correct
2. Check enrollment token
3. Verify port 50051 is accessible
4. Check agent logs: `journalctl -u xlstatus-agent -f`

### Database issues

```bash
# Reset SQLite database
rm ./data/xlstatus.db
docker compose restart server

# Reset PostgreSQL
docker compose -f docker-compose.pg.yml down -v
docker compose -f docker-compose.pg.yml up -d
```

## Getting Help

- 📚 [Full Documentation](./README.md)
- 🐛 [Report Issues](https://github.com/yourusername/xlstatus/issues)
- 💬 [Discord Community](https://discord.gg/xlstatus)

## What's Next?

- [Configuration Guide](./configuration.md)
- [Agent Setup](./agent-setup.md)
- [API Documentation](./api.md)
- [Security Best Practices](./security.md)
