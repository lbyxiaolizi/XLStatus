# 运维手册

本文档覆盖服务运行后的日常操作：健康检查、日志、端口、备份、升级和远端 smoke。

## 健康检查

```bash
curl -fsS http://localhost:8080/healthz
sudo systemctl status xlstatus
sudo journalctl -u xlstatus -n 100 --no-pager
```

Agent：

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

## 端口

默认：

- HTTP API: `8080`
- Agent gRPC: `50051`
- Web UI: `3000`

检查占用：

```bash
sudo ss -tlnp | grep -E ':(8080|50051|3000)\b' || true
```

如果 Server 前台运行后直接退出，优先检查端口占用。当前版本会把 HTTP/gRPC 绑定错误打印为 `Error:`。

## systemd

Server：

```bash
sudo systemctl start xlstatus
sudo systemctl stop xlstatus
sudo systemctl restart xlstatus
sudo systemctl status xlstatus
sudo journalctl -u xlstatus -f
```

Agent：

```bash
sudo systemctl restart xlstatus-agent
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -f
```

## 前台复现

用 systemd 同样的配置前台运行：

```bash
sudo -u xlstatus CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

服务正常时不会自动退出。

## 备份

SQLite：

```bash
sudo systemctl stop xlstatus
sudo cp /var/lib/xlstatus/xlstatus.db /var/lib/xlstatus/xlstatus.db.$(date +%Y%m%d%H%M%S).bak
sudo tar czf xlstatus-config-data.tgz /etc/xlstatus /var/lib/xlstatus
sudo systemctl start xlstatus
```

PostgreSQL：

```bash
pg_dump 'postgresql://xlstatus:replace-with-a-strong-db-password@localhost:5432/xlstatus' > xlstatus.sql
```

同时备份 `/etc/xlstatus/server.toml` 或部署环境中的 `SECRET_ENCRYPTION_KEY`。数据库中的 DDNS/API token、cloudflared token、GeoIP ipinfo token 和 TOTP secret 使用该密钥加密；只有数据库备份而没有匹配的 `SECRET_ENCRYPTION_KEY` 时，这些历史密文无法恢复。

## 恢复

SQLite：

```bash
sudo systemctl stop xlstatus
sudo cp /path/to/xlstatus.db.bak /var/lib/xlstatus/xlstatus.db
sudo chown xlstatus:xlstatus /var/lib/xlstatus/xlstatus.db
sudo systemctl start xlstatus
curl -fsS http://localhost:8080/healthz
```

PostgreSQL：

```bash
psql 'postgresql://xlstatus:replace-with-a-strong-db-password@localhost:5432/xlstatus' < xlstatus.sql
```

恢复前确认服务使用的 `SECRET_ENCRYPTION_KEY` 与备份创建时一致。若必须轮换密钥，应先用旧密钥完成恢复和启动迁移验证，再按受控流程重新加密并更新密钥。

恢复建议使用创建备份时相同的应用版本，升级迁移前先在测试环境验证。

## SQLite 到 PostgreSQL 迁移

仓库提供 `scripts/import_sqlite_to_postgres.py` 用于把 XLStatus SQLite
备份导入到已经初始化过 schema 的 PostgreSQL 数据库。脚本面向 Docker
Compose 部署，默认通过 `docker exec xlstatus-postgres psql ...` 写入数据。

迁移前提：

- 使用和 SQLite 备份匹配的 XLStatus 版本先完成一次测试迁移。
- 已备份 SQLite 数据库、部署配置和 `.env` / secret 文件。
- 迁移后继续使用原来的 `SESSION_SECRET` 和 `SECRET_ENCRYPTION_KEY`。缺少匹配的 `SECRET_ENCRYPTION_KEY` 时，历史加密密文无法恢复。
- PostgreSQL schema 必须先由 XLStatus Server 初始化。仅启动空的 PostgreSQL 容器还不够。
- 不要把 PostgreSQL `5432` 发布到公网。容器内部访问即可；远端维护优先使用 SSH tunnel、VPN 或受控防火墙。

推荐流程：

```bash
# 1. 停止写入并做 SQLite 在线外备份
docker compose stop web server
mkdir -p /root/xlstatus-backups/$(date -u +%Y%m%dT%H%M%SZ)-pre-pg
sqlite3 /path/to/xlstatus.db ".backup /root/xlstatus-backups/YYYYMMDDTHHMMSSZ-pre-pg/xlstatus.sqlite3"

# 2. 启动 PostgreSQL
docker compose -f docker-compose.pg.yml up -d postgres

# 3. 临时启动 Server，让应用迁移创建 PostgreSQL schema
docker compose -f docker-compose.pg.yml up -d server
curl -fsS http://127.0.0.1:8080/healthz
docker compose -f docker-compose.pg.yml stop server

# 4. 导入 SQLite 备份。--truncate 会清空目标 PG public schema 中的应用表。
python3 scripts/import_sqlite_to_postgres.py \
  /root/xlstatus-backups/YYYYMMDDTHHMMSSZ-pre-pg/xlstatus.sqlite3 \
  --container xlstatus-postgres \
  --truncate

# 5. 启动 Server 和 Web，并做健康检查
docker compose -f docker-compose.pg.yml up -d server web
curl -fsS http://127.0.0.1:8080/healthz
```

脚本行为和注意事项：

- 默认不会清空目标 PostgreSQL。如果目标表已有数据且没有传 `--truncate`，脚本会拒绝导入。
- `--truncate` 会执行 `TRUNCATE ... RESTART IDENTITY CASCADE`，只能在确认备份可用、目标库可替换时使用。
- 脚本按 PostgreSQL 外键关系计算导入顺序，只导入 SQLite 和 PostgreSQL 都存在的列。
- SQLite 的 `0/1` 会转换为 PostgreSQL boolean，空字符串会按 nullable UUID/timestamp 转为 `NULL`。
- 导入后脚本会逐表核对已导入行数；任何 COPY 或行数不一致都会以非零状态退出。
- 如果脚本提示 PostgreSQL 没有表，先启动同版本 XLStatus Server 跑完迁移，再重新执行导入。

## 升级

源码安装的常规流程：

```bash
git pull --ff-only
cargo build --release --bin xlstatus-server
sudo systemctl stop xlstatus
sudo install -m 0755 target/release/xlstatus-server /usr/local/bin/xlstatus-server
sudo systemctl start xlstatus
curl -fsS http://localhost:8080/healthz
```

升级前先备份数据库和 `/etc/xlstatus/server.toml`。

## 远端 smoke

在 Linux 目标机：

```bash
cargo build --release --bin xlstatus-server
mkdir -p ./data
timeout 8s env \
  DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
  DATABASE_CREATE_IF_MISSING=true \
  HTTP_BIND="127.0.0.1:8080" \
  GRPC_BIND="127.0.0.1:50051" \
  CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
  SESSION_SECRET="$(openssl rand -hex 32)" \
  XLSTATUS_SEED_ADMIN_USERNAME="admin" \
  XLSTATUS_SEED_ADMIN_PASSWORD="replace-with-a-strong-initial-password" \
  ./target/release/xlstatus-server
echo $?
```

退出码含义：

- `124`：服务持续运行到 timeout，smoke 通过。
- `1`：启动失败，看终端中的 `Error:`。
- `0`：服务任务意外正常结束，应当按故障处理。

## 日志级别

默认：

```env
RUST_LOG=info
```

临时调试：

```bash
RUST_LOG=debug CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```
