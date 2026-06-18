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
SQLite 模式会在首次启动时创建 `./data/xlstatus.db`。

PostgreSQL 版本：

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
```

PostgreSQL Compose 在空 volume 上会自动创建 `xlstatus` 用户和数据库；应用表由 XLStatus 首次启动时自动迁移。

## 从源码构建

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

运行 Server：

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

如果 SQLite 数据库文件不存在且没有设置 `?mode=rwc` 或 `DATABASE_CREATE_IF_MISSING=true`，交互式运行会询问是否新建；systemd/Docker 等非交互环境会直接报错，避免误建数据目录。

PostgreSQL 新站最短路径：

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

首次启动会自动执行应用表迁移。更多配置项见 [configuration.md](./configuration.md)。

另开一个终端从源码运行 Web UI：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

如果已经执行过 `pnpm build`，也可以用接近生产的方式本地运行：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
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
