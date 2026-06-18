# 快速开始

## Docker Compose

```bash
docker compose up -d
curl -fsS http://localhost:8080/healthz
docker compose ps
```

访问：

- API: http://localhost:8080
- Web UI: http://localhost:3000

默认 Compose 配置会为本地测试创建 `admin` / `admin123`。

PostgreSQL 版本：

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
```

## 从源码构建

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
```

运行 Server：

```bash
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

注册并运行 Agent：

```bash
xlstatus-agent enroll \
  --server http://localhost:8080 \
  --grpc-server http://localhost:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config ./agent.json

xlstatus-agent run --config ./agent.json
```

## 验证 M9 安装流程

```bash
cargo build --bin xlstatus-server --bin xlstatus-agent
test-run/verify-m9-install.sh
```

## 故障排查

```bash
docker compose logs server
docker compose logs web
sudo journalctl -u xlstatus -f
sudo journalctl -u xlstatus-agent -f
```

更多内容见 [installation.md](./installation.md)、[agent-setup.md](./agent-setup.md) 和 [troubleshooting.md](./troubleshooting.md)。
