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
- 公开状态页: http://localhost:3000/status

默认 Compose 配置会为本地测试创建 `admin` / `admin123`。
SQLite 模式会在首次启动时创建 `./data/xlstatus.db`。
Web UI 使用 BOLD. 新粗野主义配色，并把显式的深色/浅色选择保存到 `localStorage.darkMode`。
Compose 已设置 `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000`，浏览器可以直接访问 `http://localhost:8080` 上的 API。

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
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="replace-me" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

如果 SQLite 数据库文件不存在且没有设置 `?mode=rwc` 或 `DATABASE_CREATE_IF_MISSING=true`，交互式运行会询问是否新建；systemd/Docker 等非交互环境会直接报错，避免误建数据目录。

等价的 `config.toml` 路径：

```bash
cp config.example.toml ./config.toml
SESSION_SECRET_VALUE="$(openssl rand -hex 32)"
sed -i.bak "s/replace-with-a-long-random-secret/${SESSION_SECRET_VALUE}/" ./config.toml
CONFIG_FILE=./config.toml ./target/release/xlstatus-server
```

使用 `CONFIG_FILE` 时，不要在同一个进程里设置 `DATABASE_URL`。一旦设置 `DATABASE_URL`，服务端会切换到环境变量配置模式并忽略 `CONFIG_FILE`。

PostgreSQL 新站最短路径：

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

首次启动会自动执行应用表迁移。更多配置项见 [configuration.zh-CN.md](./configuration.zh-CN.md)。

另开一个终端从源码运行 Web UI：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

登录前先打开 `http://localhost:3000/status` 验证公共状态 API 是否可访问。登录后可在导航栏切换 BOLD. 浅色/深色配色。
如果 Web UI 使用其他端口，请在启动服务端前把精确来源加入 `CORS_ALLOWED_ORIGINS`。
本地调试时建议主机名保持一致：前端使用 `localhost` 时 API 也使用 `localhost`；前端使用 `127.0.0.1` 时 API 也使用 `127.0.0.1`。

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
