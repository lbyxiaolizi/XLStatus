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
pg_dump 'postgresql://xlstatus:xlstatus_password@localhost:5432/xlstatus' > xlstatus.sql
```

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
psql 'postgresql://xlstatus:xlstatus_password@localhost:5432/xlstatus' < xlstatus.sql
```

恢复建议使用创建备份时相同的应用版本，升级迁移前先在测试环境验证。

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
  HTTP_BIND="0.0.0.0:8080" \
  GRPC_BIND="0.0.0.0:50051" \
  CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
  SESSION_SECRET="replace-me" \
  XLSTATUS_SEED_ADMIN_USERNAME="admin" \
  XLSTATUS_SEED_ADMIN_PASSWORD="admin123" \
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
