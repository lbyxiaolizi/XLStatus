# 配置参考

本文档描述当前 `xlstatus-server` 和 `web/` 实际读取的配置项。

## 加载顺序

Server 有两种配置模式：

1. 如果设置了 `DATABASE_URL`，进入环境变量模式。
2. 如果没有设置 `DATABASE_URL`，且 `CONFIG_FILE` 指向存在的 TOML 文件，读取该 TOML。
3. 如果两者都没有，使用开发默认值。

环境变量模式和 TOML 模式不会合并。使用 `CONFIG_FILE` 时不要同时设置 `DATABASE_URL`。

## 环境变量

```env
DATABASE_URL=sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc
DATABASE_CREATE_IF_MISSING=true
HTTP_BIND=127.0.0.1:8080
GRPC_BIND=127.0.0.1:50051
GRPC_TLS_CERT_PATH=/etc/xlstatus/tls/grpc-server.crt
GRPC_TLS_KEY_PATH=/etc/xlstatus/tls/grpc-server.key
GRPC_TLS_CLIENT_CA_PATH=/etc/xlstatus/tls/agent-ca.crt
GRPC_REFLECTION_ENABLED=false
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
SESSION_SECRET=replace-with-a-long-random-secret
SECRET_ENCRYPTION_KEY=replace-with-a-different-long-random-secret
SESSION_TTL_HOURS=24
SECURITY_COOKIE_SECURE=true
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=replace-with-a-strong-initial-password
```

| 变量 | 说明 |
|---|---|
| `DATABASE_URL` | SQLite 或 PostgreSQL URL。设置后启用环境变量模式。 |
| `DATABASE_CREATE_IF_MISSING` | 仅 SQLite 使用。`1`、`true`、`yes`、`y`、`on` 视为开启。 |
| `HTTP_BIND` | HTTP API 监听地址，默认 `127.0.0.1:8080`。 |
| `GRPC_BIND` | Agent gRPC 监听地址，默认 `127.0.0.1:50051`。 |
| `GRPC_TLS_CERT_PATH` | 可选的 gRPC 服务端 PEM 证书路径。必须与 `GRPC_TLS_KEY_PATH` 同时设置，设置后 gRPC 使用 TLS。 |
| `GRPC_TLS_KEY_PATH` | 可选的 gRPC 服务端 PEM 私钥路径。必须与 `GRPC_TLS_CERT_PATH` 同时设置。 |
| `GRPC_TLS_CLIENT_CA_PATH` | 可选的 gRPC 客户端证书 CA 路径。设置后启用 mTLS，Agent 必须提交该 CA 签发的客户端证书。 |
| `GRPC_REFLECTION_ENABLED` | 是否启用 gRPC reflection。默认关闭；只建议在可信网络调试时开启。 |
| `CORS_ALLOWED_ORIGINS` | 允许访问 API 的浏览器来源，多个来源用英文逗号分隔。 |
| `SESSION_SECRET` | 会话和 Agent JWT 签名密钥。生产必须使用长随机值。 |
| `SECRET_ENCRYPTION_KEY` | 可选的数据库 secret 加密主密钥，用于 DDNS/API token、cloudflared token、GeoIP ipinfo token、TOTP secret 的应用层加密。不设置时使用 `SESSION_SECRET`；生产建议单独设置长随机值并备份。 |
| `SESSION_TTL_HOURS` | Cookie 会话有效期，默认 `24` 小时。 |
| `SECURITY_COOKIE_SECURE` | 是否为 session/CSRF Cookie 添加 `Secure`。环境变量和 TOML 部署默认开启，本地无配置开发默认关闭。 |
| `XLSTATUS_SEED_ADMIN_USERNAME` | 可选的首个管理员用户名。 |
| `XLSTATUS_SEED_ADMIN_PASSWORD` | 可选的首个管理员密码。只在用户不存在时创建。 |

## config.toml

推荐从根目录示例复制：

```bash
cp config.example.toml /etc/xlstatus/server.toml
```

完整示例：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true

[server]
http_bind = "127.0.0.1:8080"
grpc_bind = "127.0.0.1:50051"
# grpc_tls_cert_path = "/etc/xlstatus/tls/grpc-server.crt"
# grpc_tls_key_path = "/etc/xlstatus/tls/grpc-server.key"
# grpc_tls_client_ca_path = "/etc/xlstatus/tls/agent-ca.crt"
grpc_reflection_enabled = false
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
]

[security]
session_secret = "replace-with-a-long-random-secret"
secret_encryption_key = "replace-with-a-different-long-random-secret"
session_ttl_hours = 24
```

启动：

```bash
CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

## SQLite

推荐：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true
```

数据库文件不存在时：

- URL 包含 `?mode=rwc`：SQLite 允许创建文件。
- `create_if_missing = true` 或 `DATABASE_CREATE_IF_MISSING=true`：XLStatus 允许创建父目录和数据库文件。
- 两者都没有，且是交互式终端：服务端会询问是否创建。
- 两者都没有，且是 systemd/Docker：服务端失败退出并提示缺失文件。

如果你希望文件不存在时直接失败：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rw"
create_if_missing = false
```

systemd 权限：

```bash
sudo mkdir -p /var/lib/xlstatus
sudo chown -R xlstatus:xlstatus /var/lib/xlstatus
```

## PostgreSQL

新站先创建用户和数据库：

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

TOML：

```toml
[database]
url = "postgresql://xlstatus:change-this-password@localhost:5432/xlstatus"
create_if_missing = false
```

XLStatus 连接成功后会自动执行内置迁移。新站数据库应保持为空。

## CORS

Web UI 和 API 不同源时，后端必须允许 Web UI 的浏览器来源。

本地默认：

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

TOML：

```toml
[server]
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
]
```

生产域名：

```toml
[server]
cors_allowed_origins = ["https://status.example.com"]
```

不要使用 `*`。Dashboard 使用 Cookie 会话和 CSRF，服务端会拒绝通配符 CORS。

## Web UI 配置

`web/` 使用：

```env
NEXT_PUBLIC_API_URL=http://localhost:8080
```

这个值会进入浏览器 bundle。它不等于后端 CORS：前端要知道 API 地址，后端也要允许 Web UI 的浏览器来源。

如果不设置 `NEXT_PUBLIC_API_URL`，浏览器端会默认请求当前页面主机名的 `8080`
端口。Docker Compose 使用下面两个变量生成 Web 构建参数和后端 CORS：

```env
XLSTATUS_PUBLIC_API_URL=http://example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://example.com:3000,http://localhost:3000,http://127.0.0.1:3000
```

修改后需要重新构建 Web 镜像：

```bash
docker compose up -d --build
```

## i18n

Web UI 国际化配置位于 `web/lib/i18n.ts`。

- 默认语言：`zh-CN`
- 支持语言：`zh-CN`
- `<html lang="zh-CN">` 由根布局设置
- 用户可见共享文案放在 `zhCN` 字典
- 后端协议值、枚举值、scope 字符串不翻译，例如 `server:read`

## 管理员初始化

首次启动可设置：

```env
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=replace-with-a-strong-initial-password
```

如果该用户名已存在，不会覆盖密码。生产环境必须使用强随机初始密码，并在首次登录后移除明文 seed 变量。
