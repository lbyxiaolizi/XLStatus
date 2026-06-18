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
HTTP_BIND=0.0.0.0:8080
GRPC_BIND=0.0.0.0:50051
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
SESSION_SECRET=replace-with-a-long-random-secret
SESSION_TTL_HOURS=24
XLSTATUS_SEED_ADMIN_USERNAME=admin
XLSTATUS_SEED_ADMIN_PASSWORD=admin123
```

| 变量 | 说明 |
|---|---|
| `DATABASE_URL` | SQLite 或 PostgreSQL URL。设置后启用环境变量模式。 |
| `DATABASE_CREATE_IF_MISSING` | 仅 SQLite 使用。`1`、`true`、`yes`、`y`、`on` 视为开启。 |
| `HTTP_BIND` | HTTP API 监听地址，默认 `0.0.0.0:8080`。 |
| `GRPC_BIND` | Agent gRPC 监听地址，默认 `0.0.0.0:50051`。 |
| `CORS_ALLOWED_ORIGINS` | 允许访问 API 的浏览器来源，多个来源用英文逗号分隔。 |
| `SESSION_SECRET` | 会话和 Agent JWT 签名密钥。生产必须使用长随机值。 |
| `SESSION_TTL_HOURS` | Cookie 会话有效期，默认 `24` 小时。 |
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
http_bind = "0.0.0.0:8080"
grpc_bind = "0.0.0.0:50051"
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
]

[security]
session_secret = "replace-with-a-long-random-secret"
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
XLSTATUS_SEED_ADMIN_PASSWORD=admin123
```

如果该用户名已存在，不会覆盖密码。生产环境应在首次登录后更换密码，并移除明文 seed 变量。
