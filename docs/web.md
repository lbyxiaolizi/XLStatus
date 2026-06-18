# Web 前端

`web/` 是 XLStatus 的 Next.js 管理面板，当前界面语言为简体中文。

## 要求

- Node.js 20+
- Corepack
- `package.json` 中锁定的 pnpm
- 可访问的 XLStatus Server HTTP API

## 安装依赖

```bash
cd web
corepack enable
pnpm install --frozen-lockfile
```

## 开发运行

先启动 Server，然后：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

访问：

- Dashboard: `http://localhost:3000`
- Public Status: `http://localhost:3000/status`

如果 Next.js 使用其他端口，例如 `3001`，Server CORS 也要同步：

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3001,http://127.0.0.1:3001
```

## 生产构建

```bash
cd web
NEXT_PUBLIC_API_URL=https://api.example.com pnpm build
NEXT_PUBLIC_API_URL=https://api.example.com pnpm start
```

`NEXT_PUBLIC_API_URL` 会写入浏览器 bundle。修改后需要重新构建。

如果没有设置 `NEXT_PUBLIC_API_URL`，浏览器端会按当前页面主机名推导 API：

```text
http://<当前页面主机名>:8080
```

例如访问 `http://example.com:3000/status` 时，默认 API 是
`http://example.com:8080`。如果 API 使用其他域名、HTTPS 或反向代理路径，请显式设置
`NEXT_PUBLIC_API_URL`。

## Docker Compose 远端部署

Compose 会把 `XLSTATUS_PUBLIC_API_URL` 作为 Web 镜像的 build arg 传入：

```bash
cp .env.example .env
```

`.env`：

```env
XLSTATUS_PUBLIC_API_URL=http://example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://example.com:3000,http://localhost:3000,http://127.0.0.1:3000
```

启动或修改后重新构建：

```bash
docker compose up -d --build
```

只改 `environment:` 或只重启容器不会更新已经构建好的浏览器 bundle。

## CORS 配合

如果用户浏览器打开的是：

```text
https://status.example.com
```

而 API 是：

```text
https://api.example.com
```

Server 必须配置：

```toml
[server]
cors_allowed_origins = ["https://status.example.com"]
```

`NEXT_PUBLIC_API_URL` 只告诉前端请求哪里；CORS 决定后端是否允许这个浏览器来源。

## i18n

配置文件：

```text
web/lib/i18n.ts
```

当前设置：

- 默认语言：`zh-CN`
- 支持语言：`zh-CN`
- App Router 不使用旧 Pages Router 的 `next.config.ts i18n` 字段
- 根布局设置 `<html lang="zh-CN">`
- 共享用户文案放在 `zhCN` 字典

后端协议值、数据库枚举、PAT scope 和路由片段不要翻译，例如 `server:read`、`admin`、`viewer`。

## 主要页面

- `/status`：未登录公开状态页。
- `/login`：登录。
- `/`：登录后的 Dashboard。
- 管理区：服务器、服务监控、告警、任务、DDNS、NAT、Terminal、设置。

## 后端契约

前端使用 `web/lib/api.ts` 发起 API 请求。Dashboard 登录态依赖 Cookie、CSRF 和后端 CORS，不能用 `Access-Control-Allow-Origin: *`。

常见公共接口：

- `GET /healthz`
- `GET /api/v1/public/status`
- `POST /api/v1/auth/login`
- `POST /api/v1/auth/logout`

## 验证

```bash
cd web
pnpm lint
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

如果页面报 `Failed to fetch`，先看 [故障排查](./troubleshooting.md#web-ui-和-cors)。
