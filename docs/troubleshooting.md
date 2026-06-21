# 故障排查

先按症状定位，再看对应修复步骤。

## Server 打印 listening 后直接退出

正常情况下，前台运行的 Server 会一直占用终端。如果直接返回 shell，说明 HTTP 或 gRPC 任务退出了。

先查端口：

```bash
sudo ss -tlnp | grep -E ':(8080|50051)\b' || true
```

常见原因：

- `8080` 已被其他 HTTP 服务占用。
- `50051` 已被旧进程或其他 gRPC 服务占用。
- `CONFIG_FILE` 不存在或权限错误。
- SQLite 数据目录不可写。
- PostgreSQL URL 错误或数据库未创建。

当前版本会把内部错误向上传递，前台通常会看到：

```text
Error: gRPC server failed
Caused by:
  failed to bind gRPC server to 127.0.0.1:50051
```

systemd：

```bash
sudo systemctl status xlstatus --no-pager
sudo journalctl -u xlstatus -n 100 --no-pager
```

## Server active 但 healthz 不通

```bash
curl -v http://localhost:8080/healthz
sudo ss -tlnp | grep ':8080'
```

检查：

- `http_bind` 或 `HTTP_BIND` 是否不是 `8080`。
- 防火墙是否阻止外部访问。
- 反向代理是否转发到正确端口。

## config.toml 没生效

如果设置了 `DATABASE_URL`，Server 会进入环境变量模式并忽略 `CONFIG_FILE`。

检查 systemd unit：

```bash
sudo systemctl cat xlstatus
```

推荐只保留：

```ini
Environment="CONFIG_FILE=/etc/xlstatus/server.toml"
```

不要再混入 `DATABASE_URL`，除非你明确要走环境变量模式。

## SQLite 文件不存在

推荐：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true
```

非交互环境如果没有 `?mode=rwc` 或 `create_if_missing = true`，会失败退出，避免误建数据到错误目录。

权限修复：

```bash
sudo mkdir -p /var/lib/xlstatus
sudo chown -R xlstatus:xlstatus /var/lib/xlstatus
```

## 已有 SQLite 数据库迁移失败

先确认二进制是最新构建：

```bash
./target/release/xlstatus-server --help
git rev-parse --short HEAD
```

当前迁移里的索引创建应使用 `IF NOT EXISTS`。如果旧数据库仍报类似 `index ... already exists`：

1. 先备份数据库文件。
2. 确认运行的是最新二进制。
3. 重新前台启动并查看完整错误。

不要直接删除生产数据库。

## PostgreSQL 无法启动

测试连接：

```bash
psql 'postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' -c 'select 1;'
```

新建用户和库：

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

新站数据库保持为空，让 XLStatus 自动执行应用迁移。

## Web UI 和 CORS

症状：

- 页面显示 `Failed to fetch`
- 登录不跳转
- 浏览器控制台出现 CORS 或 preflight 错误

检查 API：

```bash
curl -i http://localhost:8080/api/v1/public/status
```

检查 CORS：

```bash
curl -i \
  -H 'Origin: http://localhost:3000' \
  http://localhost:8080/api/v1/public/status
```

应包含：

```http
access-control-allow-origin: http://localhost:3000
access-control-allow-credentials: true
```

检查登录预检：

```bash
curl -i \
  -X OPTIONS \
  -H 'Origin: http://localhost:3000' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type' \
  http://localhost:8080/api/v1/auth/login
```

修复：

- 把 Web UI 的精确来源加入 `CORS_ALLOWED_ORIGINS` 或 `server.cors_allowed_origins`。
- 本地调试保持主机名一致：`localhost` 配 `localhost`，`127.0.0.1` 配 `127.0.0.1`。
- 修改 CORS 后重启 Server。
- 确认前端构建时设置了正确的 `NEXT_PUBLIC_API_URL`。

远端 Docker Compose 常见误区：

- `CORS_ALLOWED_ORIGINS` 只决定后端是否放行浏览器来源。
- `NEXT_PUBLIC_API_URL` 决定浏览器实际请求哪个 API 地址，而且会写入 Web 构建产物。
- 如果浏览器开发者工具里看到请求 `http://localhost:8080/...`，说明 Web 镜像是用错误 API 地址构建的。

修复示例：

```bash
cp .env.example .env
```

`.env`：

```env
XLSTATUS_PUBLIC_API_URL=http://example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://example.com:3000,http://localhost:3000,http://127.0.0.1:3000
```

重新构建并启动：

```bash
docker compose up -d --build
```

如果已经有旧镜像仍不生效，可以强制重建 Web：

```bash
docker compose build --no-cache web
docker compose up -d web
```

## Docker Compose

查看渲染配置：

```bash
docker compose config
docker compose -f docker-compose.pg.yml config
```

查看日志：

```bash
docker compose logs server
docker compose logs web
docker compose -f docker-compose.pg.yml logs postgres
```

重置本地 SQLite：

```bash
docker compose down
rm -f ./data/xlstatus.db
docker compose up -d
```

重置本地 PostgreSQL：

```bash
docker compose -f docker-compose.pg.yml down -v
docker compose -f docker-compose.pg.yml up -d
```

## Agent 离线

检查服务：

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

手动运行：

```bash
sudo /usr/local/bin/xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

常见原因：

- `--server` 填错，应该是 HTTP API 地址。
- `--grpc-server` 不可达。
- enrollment token 过期或已使用。
- Agent 配置文件缺少私钥或权限异常。

重新注册：

```bash
sudo /usr/local/bin/xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name "$(hostname)" \
  --config /etc/xlstatus-agent/agent.json
```
