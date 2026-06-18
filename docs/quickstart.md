# 快速开始

本页用于把 XLStatus 在本地跑起来。生产或公网部署请继续阅读 [安装部署](./installation.md) 和 [配置参考](./configuration.md)。

## Docker Compose

SQLite 版本：

```bash
docker compose up -d
curl -fsS http://localhost:8080/healthz
docker compose ps
```

PostgreSQL 版本：

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
docker compose -f docker-compose.pg.yml ps
```

访问：

- Web UI: `http://localhost:3000`
- API: `http://localhost:8080`
- 公开状态页: `http://localhost:3000/status`

默认本地账号：`admin` / `admin123`。

Compose 已设置：

```env
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

因此默认 Web UI 可以访问 API。SQLite Compose 会创建 `./data/xlstatus.db`；PostgreSQL Compose 会在空 volume 上创建数据库用户和库，然后由 XLStatus 执行应用迁移。

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

## 前台运行 Server

SQLite：

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

服务正常时，这个进程会一直运行。另开终端检查：

```bash
curl -fsS http://localhost:8080/healthz
```

短时间 smoke test：

```bash
timeout 8s env \
  DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
  DATABASE_CREATE_IF_MISSING=true \
  HTTP_BIND="0.0.0.0:8080" \
  GRPC_BIND="0.0.0.0:50051" \
  CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
  SESSION_SECRET="replace-me" \
  XLSTATUS_SEED_ADMIN_USERNAME="admin" \
  XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
  ./target/release/xlstatus-server
echo $?
```

期望退出码是 `124`，表示服务持续运行到 timeout。如果它直接回到 shell，查看输出中的 `Error:`，常见原因是 `8080` 或 `50051` 被占用。

## 使用 config.toml

```bash
cp config.example.toml ./config.toml
SESSION_SECRET_VALUE="$(openssl rand -hex 32)"
sed -i.bak "s/replace-with-a-long-random-secret/${SESSION_SECRET_VALUE}/" ./config.toml
CONFIG_FILE=./config.toml ./target/release/xlstatus-server
```

不要同时设置 `DATABASE_URL`。一旦设置 `DATABASE_URL`，服务端会切换到环境变量模式并忽略 `CONFIG_FILE`。

## 运行 Web UI

开发模式：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

生产式本地运行：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

如果 Next.js 端口不是 `3000`，请先把对应来源加入后端 CORS，例如：

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3001,http://127.0.0.1:3001
```

## PostgreSQL 新站

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

启动：

```bash
DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="$(openssl rand -hex 32)" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
./target/release/xlstatus-server
```

XLStatus 会自动创建应用表。新站数据库应保持为空，除非是在恢复同版本备份。

## 注册 Agent

在 Dashboard 里创建 enrollment token 后：

```bash
xlstatus-agent enroll \
  --server http://localhost:8080 \
  --grpc-server http://localhost:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config ./agent.json

xlstatus-agent run --config ./agent.json
```

更多内容见 [Agent 接入](./agent.md)。
