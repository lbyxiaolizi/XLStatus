# Agent Setup Guide

The current XLStatus agent is a single Linux x86_64 binary with two implemented commands: `enroll` and `run`.

## Network

Agents need outbound access to:

- Dashboard HTTP API, usually `http://SERVER:8080`
- Dashboard gRPC endpoint, usually `http://SERVER:50051`

## Enroll

Generate an enrollment token from the dashboard/admin API, then run:

```bash
sudo xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config /etc/xlstatus-agent/agent.json
```

The config file is JSON and includes sensitive private-key material. Keep ownership `root:root` and permissions `0600`.

## Run

```bash
sudo xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

The agent sends host info once, then sends host state every 3 seconds. It reconnects with bounded exponential backoff if the gRPC stream closes.

## systemd

Use `deploy/install-agent.sh` for the common path:

```bash
sudo BINARY_PATH=/tmp/xlstatus-agent \
  SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=http://dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash deploy/install-agent.sh
```

Manual unit:

```ini
[Unit]
Description=XLStatus Agent
After=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=/usr/local/bin/xlstatus-agent run --config /etc/xlstatus-agent/agent.json
Restart=always
RestartSec=5s
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
```

## Docker

The agent image can be built locally, but it still needs an enrolled config:

```bash
docker build -f Dockerfile.agent -t xlstatus-agent:local .
docker run --rm \
  --network host \
  -v /etc/xlstatus-agent:/etc/xlstatus-agent:ro \
  xlstatus-agent:local
```

## Troubleshooting

Check config:

```bash
sudo test -s /etc/xlstatus-agent/agent.json
sudo ls -l /etc/xlstatus-agent/agent.json
```

Check service logs:

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

Common causes:

- `--server` must be the dashboard HTTP URL, not the gRPC URL.
- `--grpc-server` must be a tonic-compatible URL such as `http://host:50051`.
- Enrollment tokens are single-use and expire.
- Re-enrollment creates a new agent identity and keypair.
