# 开发指南

本文档面向继续开发 XLStatus 的贡献者和维护者。

## 环境

- Rust stable
- Node.js 20+
- Corepack 和 pnpm
- SQLite
- 可选：PostgreSQL 15+
- 可选：Docker 和 Docker Compose v2

## 代码结构

```text
crates/server/      HTTP API、gRPC Server、数据库和后台任务
crates/agent/       Agent CLI、注册、采集和 gRPC 会话
crates/shared/      共享类型
crates/proto-gen/   Protobuf 生成代码
crates/tsdb/        时序存储
proto/              Protobuf 定义
web/                Next.js Web UI
deploy/             systemd 和安装脚本
test-run/           可重复验收脚本
```

## 常用命令

```bash
cargo fmt
cargo check --workspace
cargo test --workspace

cd web
pnpm install --frozen-lockfile
pnpm lint
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

## 本地运行

Server：

```bash
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
DATABASE_CREATE_IF_MISSING=true \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="dev-secret" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
cargo run --bin xlstatus-server
```

Web：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

Agent：

```bash
cargo run --bin xlstatus-agent -- enroll \
  --server http://localhost:8080 \
  --grpc-server http://localhost:50051 \
  --token xle_... \
  --name dev-agent \
  --config ./agent.json

cargo run --bin xlstatus-agent -- run --config ./agent.json
```

## 验收脚本

`test-run/` 中的脚本覆盖 M0-M9 的主要行为：

```bash
test-run/verify-m0.sh
test-run/verify-m1-pg.sh
test-run/verify-m2-reconnect.sh
test-run/verify-m2-revoke.sh
test-run/verify-m3-metrics.sh
test-run/verify-m3-tsdb.sh
test-run/verify-m3-ws.sh
test-run/verify-m4-alerts.sh
test-run/verify-m5-files.sh
test-run/verify-m5-scheduler.sh
test-run/verify-m5-task.sh
test-run/verify-m5-terminal.sh
test-run/verify-m6-ddns.sh
test-run/verify-m6-mcp.sh
test-run/verify-m6-nat.sh
test-run/verify-m7-ui.sh
test-run/verify-m8-migrations.sh
test-run/verify-m8-tsdb-load.sh
test-run/verify-m9-install.sh
```

运行前先看脚本头部说明。有些脚本会启动本地服务、占用端口或要求 Docker/PostgreSQL。

## 数据库迁移

迁移文件位于：

```text
crates/server/migrations/sqlite/
crates/server/migrations/postgres/
```

新增迁移时保持 SQLite 和 PostgreSQL 两套一致。对于可重复启动场景，`CREATE TABLE`、`CREATE INDEX` 应使用 `IF NOT EXISTS`；SQLite 不支持的 `ALTER TABLE ... IF NOT EXISTS` 需要在 Rust 迁移逻辑或兼容 SQL 中处理。

## 前端约定

- 用户可见共享文案集中在 `web/lib/i18n.ts`。
- 当前唯一 locale 是 `zh-CN`。
- API enum、scope、协议字段不翻译。
- `NEXT_PUBLIC_API_URL` 修改后需要重新构建生产包。

## 文档约定

`docs/` 只放当前发布文档。历史记录、临时报告和一次性验证输出不要再放回 `docs/`，避免用户阅读路径变乱。
