# 配置说明

本文档描述当前 XLStatus 二进制实际读取的配置项，重点覆盖 `config.toml`、环境变量、SQLite/PostgreSQL 数据库、Web UI CORS 和 Agent 配置。

## 加载顺序

服务器按下面顺序加载配置：

1. 如果设置了 `DATABASE_URL`，使用环境变量模式。
2. 如果没有设置 `DATABASE_URL`，并且 `CONFIG_FILE` 指向存在的 TOML 文件，读取该 TOML 文件。
3. 如果两者都没有，使用开发默认值。

注意：环境变量模式和 TOML 文件模式不会合并。设置了 `DATABASE_URL` 时，`CONFIG_FILE` 会被忽略。

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

| 变量 | 是否必需 | 说明 |
|---|---:|---|
| `DATABASE_URL` | 环境变量模式必需 | SQLite 或 PostgreSQL 连接 URL。设置后进入环境变量配置模式。 |
| `DATABASE_CREATE_IF_MISSING` | 否 | 仅 SQLite 使用。`1`、`true`、`yes`、`y`、`on` 视为开启。 |
| `HTTP_BIND` | 否 | HTTP API 监听地址，默认 `0.0.0.0:8080`。 |
| `GRPC_BIND` | 否 | Agent gRPC 监听地址，默认 `0.0.0.0:50051`。 |
| `CORS_ALLOWED_ORIGINS` | 否 | 允许访问 API 的浏览器源，多个源用英文逗号分隔。默认允许本地 Next.js `3000` 端口。 |
| `SESSION_SECRET` | 生产必需 | 会话和 Agent JWT 签名密钥。生产环境必须使用长随机值。 |
| `SESSION_TTL_HOURS` | 否 | Cookie 会话有效期，默认 `24` 小时。 |
| `XLSTATUS_SEED_ADMIN_USERNAME` | 否 | 可选的首个管理员用户名。 |
| `XLSTATUS_SEED_ADMIN_PASSWORD` | 否 | 可选的首个管理员密码。仅在用户不存在时用于初始化。 |

## config.toml

复制根目录的 [../config.example.toml](../config.example.toml) 到目标路径，例如 `/etc/xlstatus/server.toml`：

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

使用配置文件启动：

```bash
CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

当前服务端没有 `--config`、`--validate` 或 `--version` CLI 参数；请通过 `CONFIG_FILE` 或环境变量传入配置。

## Web UI CORS

当前 Web UI 是浏览器应用。如果 Web UI 和 API 不同源，例如：

- Web UI: `http://localhost:3000`
- API: `http://localhost:8080`

那么 API 必须允许 Web UI 的浏览器源，否则浏览器会拦截请求，页面会出现 `Failed to fetch` 或登录预检失败。

环境变量写法：

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

TOML 写法：

```toml
[server]
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
]
```

常见场景：

```bash
# 默认 Next.js 本地端口
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000

# Next.js 改到 3001
CORS_ALLOWED_ORIGINS=http://localhost:3001,http://127.0.0.1:3001

# 生产反代域名
CORS_ALLOWED_ORIGINS=https://status.example.com
```

不要使用通配符。XLStatus Dashboard 使用 Cookie 会话和 CSRF，后端会拒绝 `*` 形式的 CORS 源。

本地调试时尽量保持主机名一致：如果前端用 `http://localhost:3000` 打开，API 也用 `http://localhost:8080`；如果前端用 `http://127.0.0.1:3000`，API 也用 `http://127.0.0.1:8080`。

`NEXT_PUBLIC_API_URL` 是 Web UI 配置，用来告诉浏览器 API 地址：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

它不等价于后端 CORS。后端仍然需要通过 `CORS_ALLOWED_ORIGINS` 或 `server.cors_allowed_origins` 放行浏览器打开 Web UI 的源。

## Web UI i18n

Web UI 国际化配置位于 [../web/lib/i18n.ts](../web/lib/i18n.ts)。

当前设置：

- 默认语言：`zh-CN`
- 支持语言：`zh-CN`
- App Router i18n 配置由 `web/lib/i18n.ts` 导出；`web/next.config.ts` 不使用 Pages Router 时代的旧 `i18n` 字段
- 共享用户可见文案放在 `zhCN` 字典中
- 后端协议值、枚举值和 scope 字符串，例如 `server:read`，应保持原样，不要翻译

根布局会设置 `<html lang="zh-CN">`，日期格式也使用 `zh-CN`。

## SQLite

SQLite 适合单节点安装：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
create_if_missing = true
```

数据库文件不存在时的行为：

- URL 包含 `?mode=rwc` 时，允许创建文件。
- `create_if_missing = true` 或 `DATABASE_CREATE_IF_MISSING=true` 时，允许创建文件。
- 两者都没有设置且运行在交互式终端时，服务端会询问是否创建数据库文件。
- 两者都没有设置且运行在 systemd/Docker 等非交互环境时，启动失败并给出明确提示，避免误建数据目录。

父目录会在允许创建时自动创建。systemd 安装时请确保服务用户有写权限：

```bash
sudo mkdir -p /var/lib/xlstatus
sudo chown -R xlstatus:xlstatus /var/lib/xlstatus
```

如果你希望数据库文件不存在时直接失败，可使用 `?mode=rw`：

```toml
[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rw"
create_if_missing = false
```

## PostgreSQL

PostgreSQL 适合生产或集中备份的部署。XLStatus 会自动执行应用表迁移，但不会创建 PostgreSQL 用户和数据库，需要提前准备：

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

测试连接：

```bash
psql 'postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' -c 'select 1;'
```

TOML 配置：

```toml
[database]
url = "postgresql://xlstatus:change-this-password@localhost:5432/xlstatus"
create_if_missing = false
```

环境变量写法：

```bash
DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus'
```

新站首次启动会通过内置迁移创建全部应用表。除非是恢复同版本备份，否则请保持数据库为空。

## Agent

Agent 配置通过注册生成，保存为 JSON：

```bash
xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json
```

生成结构示例：

```json
{
  "server": "http://dashboard.example.com:8080",
  "grpc_server": "http://dashboard.example.com:50051",
  "agent_id": "...",
  "name": "web-01",
  "public_key": "...",
  "private_key": "..."
}
```

运行：

```bash
xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

Agent 配置包含私钥，建议权限保持为 `0600`。

## Docker Compose

Compose 文件通过环境变量配置服务端，可查看渲染后的完整配置：

```bash
docker compose config
docker compose -f docker-compose.pg.yml config
```

SQLite Compose 设置了 `DATABASE_CREATE_IF_MISSING=true` 并使用 `?mode=rwc`，因此新的本地 volume 会自动创建数据库。

PostgreSQL Compose 使用官方 `postgres:15` 镜像，通过 `POSTGRES_USER`、`POSTGRES_PASSWORD` 和 `POSTGRES_DB` 在空 volume 上创建用户和数据库，然后由 XLStatus 执行应用迁移。

`agent-demo` 服务默认放在 `agent-demo` profile 后面，因为它需要先挂载已经注册好的 `/etc/xlstatus-agent/agent.json`。
