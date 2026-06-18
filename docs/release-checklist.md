# 发布检查清单

发布前按本清单检查。没有通过的项目不要标记为发布完成。

## 代码

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

Web：

```bash
cd web
pnpm install --frozen-lockfile
pnpm lint
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

## 安装与运行

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent
test-run/verify-m9-install.sh
```

Linux x86_64 smoke：

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
```

退出码应为 `124`。

## 配置

- `config.example.toml` 包含 `database`、`server`、`security` 三段。
- `server.cors_allowed_origins` 已写入示例和安装脚本。
- SQLite 示例使用 `?mode=rwc` 或 `create_if_missing = true`。
- PostgreSQL 文档说明了先创建用户和数据库。
- systemd unit 不混用 `DATABASE_URL` 和 `CONFIG_FILE`。

## 文档

- `docs/README.md` 是唯一文档入口。
- 快速开始、安装、配置、Web、Agent、运维、排障全部能互相跳转。
- 文档没有引用已删除的历史归档、旧里程碑报告或过期命令。
- 从源码构建包含后端、Agent 和前端步骤。
- CORS、`config.toml`、SQLite 创建行为、PostgreSQL 新站初始化均有说明。

## 安全

- 生产文档使用随机 `SESSION_SECRET`。
- 示例提醒首次登录后更换 seed 管理员密码。
- Agent 配置说明包含私钥和 `0600` 权限要求。
- CORS 禁止 `*`。

## 发布后

- 推送 `main`。
- 创建 tag。
- 构建并上传 Linux x86_64 二进制。
- 在目标生产环境跑至少一次 24 小时稳定性观察。
