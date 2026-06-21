# API 概览

本文档是当前接口的发布级索引，不是完整 OpenAPI。路由实现以 `crates/server/src/api/` 和 `crates/server/src/main.rs` 为准。

## 基础

- HTTP API 默认监听：`127.0.0.1:8080`
- Agent gRPC 默认监听：`127.0.0.1:50051`
- 健康检查：`GET /healthz`
- 认证方式：Dashboard 使用 Cookie 会话和 CSRF；Agent 使用注册后的密钥和 JWT 流程。

## 公共接口

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/healthz` | 服务健康检查 |
| `GET` | `/api/v1/public/status` | 公开状态页数据 |
| `POST` | `/api/v1/auth/login` | 登录 |
| `POST` | `/api/v1/agents/enroll` | Agent 注册 |
| `POST` | `/api/v1/agents/jwt/challenge` | Agent JWT challenge |
| `POST` | `/api/v1/agents/jwt` | Agent JWT 签发 |
| `GET` | `/install-agent.sh` | 带参数 Agent 安装 bootstrap |
| `GET` | `/api/v1/agents/install.sh` | 带参数 Agent 安装 bootstrap |
| `GET` | `/api/v1/transfers/temp/download` | 临时下载 |
| `PUT` | `/api/v1/transfers/temp/upload` | 临时上传 |

Agent 注册、JWT challenge 和 JWT 签发请求体上限为 4KiB。JWT nonce 为 32 字节 hex，signature 为 64 字节 hex；JWT challenge 只有在 Agent 签名验证通过后才会被消费。

OAuth/OIDC callback 公开参数有协议级长度边界：`state` 最长 4096 字节，`code` 最长 4096 字节，`error` / `error_description` 最长 1024 字节。OAuth start 的 `return_to` 只接受本地路径且最长 1024 字节。OIDC token response 最多读取 16KiB，`access_token` 最长 8192 字节；userinfo response 最多读取 64KiB，`sub`、`email`、`name`、`preferred_username` 等归一化 claim 最长 1024 字节。

公开状态页只返回显式公开服务器和归属于公开服务器的服务结果。公开服务历史在 SQL 层按公开 `server_id` 过滤，每个服务最多返回最近 240 条结果；公开服务器详情监控图表最多返回 240 个采样点。

## 需要登录的接口

### 用户和认证

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/auth/logout` | 退出登录 |
| `POST` | `/api/v1/users` | 创建用户 |
| `GET` | `/api/v1/tokens` | PAT 列表 |
| `POST` | `/api/v1/tokens` | 创建 PAT |
| `DELETE` | `/api/v1/tokens/:id` | 吊销 PAT |

### Agent 和服务器

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/enrollment-tokens` | 创建 enrollment token，`expires_in_hours` 限制为 1 到 24 |
| `POST` | `/api/v1/agents/:id/revoke` | 撤销 Agent |
| `GET` | `/api/v1/servers` | 服务器列表 |
| `GET` | `/api/v1/servers/:id` | 服务器详情 |
| `GET` | `/api/v1/servers/:id/metrics` | 指标查询 |
| `GET` | `/ws/servers` | 服务器实时 WebSocket |

### 服务监控

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/services` | 服务列表 |
| `POST` | `/api/v1/services` | 创建服务监控 |
| `POST` | `/api/v1/services/test-probe` | 测试探测 |
| `GET` | `/api/v1/services/:id` | 服务详情 |
| `POST` | `/api/v1/services/:id` | 更新服务 |
| `DELETE` | `/api/v1/services/:id` | 删除服务 |
| `GET` | `/api/v1/services/:id/history` | 历史结果 |
| `GET` | `/api/v1/services/:id/uptime` | 可用率 |

服务创建、更新和测试探测请求体上限为 128KiB。服务名称最长 128 字节，target 最长 2048 字节，`interval_seconds` 必须在 10 到 86400 秒之间，`timeout_seconds` 必须在 1 到 30 秒之间。单个服务最多关联或排除 64 台服务器，失败/恢复触发任务各最多 32 个；后台一次服务探测最多下发到 64 台 Agent。

### 告警

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/alert-rules` | 告警规则列表 |
| `POST` | `/api/v1/alert-rules` | 创建告警规则 |
| `DELETE` | `/api/v1/alert-rules/:id` | 删除告警规则 |
| `GET` | `/api/v1/alert-events` | 告警事件 |

告警规则创建请求体上限为 64KiB。规则名称最长 128 字节，单条规则最多 32 个条件、单个条件 JSON 最长 4KiB，失败/恢复触发任务各最多 32 个 UUID。

通知创建、更新、通知组创建/更新和通知组加成员请求体上限为 128KiB。通知名称最长 128 字节，Webhook URL 最长 2048 字节，headers JSON 最长 16KiB、最多 32 个 header、单个 header name 最长 128 字节、value 最长 4096 字节，body template 最长 64KiB，渲染后的 URL 最长 4096 字节、请求体最长 128KiB。单个通知组最多 32 个渠道；告警、任务和 GeoIP IP 变更后台通知单次最多发送 32 个 Webhook。

### 任务

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/tasks` | 任务列表 |
| `POST` | `/api/v1/tasks` | 创建任务 |
| `GET` | `/api/v1/tasks/:id` | 任务详情 |
| `POST` | `/api/v1/tasks/:id` | 更新任务 |
| `DELETE` | `/api/v1/tasks/:id` | 删除任务 |
| `POST` | `/api/v1/tasks/:id/run` | 手动运行任务 |
| `GET` | `/api/v1/tasks/:id/runs` | 任务运行记录 |

任务创建/更新请求体上限为 256KiB。任务名称最长 128 字节，Shell 命令最长 8192 字节，`payload_json` 最长 64KiB，`server_selector_json` 最长 16KiB。选择器中每类显式 ID 最多 64 项、标签最多 32 项；一次任务执行最多解析并下发到 64 台服务器。

### 文件、配置和终端

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/servers/:id/files` | 文件列表 |
| `POST` | `/api/v1/servers/:id/files/read` | 读取文件 |
| `POST` | `/api/v1/servers/:id/files/write` | 写入文件 |
| `POST` | `/api/v1/servers/:id/files/delete` | 删除文件 |
| `POST` | `/api/v1/servers/:id/files/download-url` | 获取下载 URL |
| `POST` | `/api/v1/servers/:id/files/upload-url` | 获取上传 URL |
| `GET` | `/api/v1/servers/:id/config` | 读取 Agent 配置 |
| `POST` | `/api/v1/servers/:id/config` | 应用 Agent 配置 |
| `POST` | `/api/v1/servers/:id/force-update` | 触发 Agent 更新 |
| `POST` | `/api/v1/terminal/sessions` | 创建终端会话 |
| `GET` | `/ws/terminal/:session_id` | 终端 WebSocket |

服务器文件、临时 URL、配置应用和强制更新 POST 请求体上限为 3MiB。文件路径最长 4096 字节；直接文件写入解码后最多 2MiB，大文件应使用临时上传 URL；文件读取单次最多 2MiB；配置 patch 序列化后最多 128KiB。强制更新下载 URL 最长 2048 字节。

强制更新需要 `server:exec` 权限、明确版本、HTTPS 下载 URL 和 SHA-256 校验和。默认只允许 `https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/xlstatus-agent-*` 这类官方 Agent release 资产；自托管更新源必须显式设置 `XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL=1`，但仍要求 HTTPS 和 SHA-256。

### DDNS、NAT、MCP

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/ddns/configs` | DDNS 配置列表 |
| `POST` | `/api/v1/ddns/configs` | 创建 DDNS 配置 |
| `DELETE` | `/api/v1/ddns/configs/:id` | 删除 DDNS 配置 |
| `GET` | `/api/v1/ddns/configs/:id/history` | DDNS 历史 |
| `POST` | `/api/v1/ddns/reload` | 重载 DDNS providers |
| `POST` | `/api/v1/ddns/check-now` | 立即检查 DDNS |
| `GET` | `/api/v1/nat/mappings/all` | NAT 映射总览 |
| `POST` | `/api/v1/nat/mappings` | 创建 NAT 映射 |
| `GET` | `/api/v1/nat/mappings/agent/:agent_id` | Agent NAT 映射 |
| `GET` | `/api/v1/nat/mappings/:id` | NAT 映射详情 |
| `POST` | `/api/v1/nat/mappings/:id` | 更新 NAT 映射 |
| `DELETE` | `/api/v1/nat/mappings/:id` | 删除 NAT 映射 |
| `GET` | `/api/v1/mcp/tools` | MCP 工具列表 |
| `POST` | `/api/v1/mcp/execute` | 执行 MCP 工具 |
| `GET` | `/api/v1/mcp/info` | MCP 信息 |
| `POST` | `/mcp` | MCP JSON-RPC |

NAT 创建/更新支持安全策略字段：`allowed_sources`、`max_active_tunnels`、`idle_timeout_seconds`、`max_bytes_per_tunnel`、`max_bandwidth_bytes_per_second`、`rate_limit_window_seconds`、`max_connections_per_window`、`max_bytes_per_window`。窗口字段按 mapping 和来源 IP 计数，用于限制窗口内连接数和累计双向流量。`local_host` 默认只能是 Agent 本机 loopback 目标；如需转发到 Agent 所在内网其他主机，必须显式启用私网 NAT 目标环境变量。

MCP POST 入口请求体上限为 1MiB。`/mcp` JSON-RPC batch 最多 16 项，空 batch 或超过上限会返回 `Invalid Request`。`server.exec` 命令最长 8192 字节，timeout 被限制在 1 到 60 秒，默认 30 秒。

GeoIP 测试接口请求体上限为 4KiB。GeoIP JSON provider 响应最多读取 16KiB，返回的 `raw` JSON 会限制字符串长度、数组项数、对象字段数和嵌套深度；MMDB 下载和上传文件上限均为 128MiB，下载路径会在读取过程中按上限中止。

## gRPC

Agent gRPC 服务定义在 `proto/xlstatus/v1/agent.proto`，生成代码在 `crates/proto-gen/`。默认消息大小限制为 `256 MiB`。

典型流程：

1. Agent 通过 HTTP enrollment 获取身份。
2. Agent 建立 gRPC session。
3. Server 通过 session 下发任务、IO、配置和操作。
4. Agent 持续上报主机状态和任务结果。

## Agent 安装链接

`GET /api/v1/agents/install.sh` 接收查询参数并返回一个很小的 bootstrap shell 脚本。真正的 `install-agent.sh` 放在 GitHub Release 资产中，bootstrap 只负责导出参数并下载执行 GitHub 脚本。

安全约束：`server_url` 与 `grpc_server` 只能是 `http` / `https` origin URL，不能包含 path、query、fragment 或 userinfo；若显式传入，host 必须与本次请求的 `Host` 相同，端口可以不同。未传 `server_url` 时使用当前请求 Host；未传 `grpc_server` 时在同 Host 上推导 `:50051`。

支持的参数：

| 参数 | 说明 |
|---|---|
| `server_url` | Dashboard HTTP API origin，必须与请求 Host 同主机 |
| `grpc_server` | Agent gRPC origin，必须与请求 Host 同主机，端口可不同 |
| `grpc_tls_ca_path` | 可选，Agent 侧用于验证 gRPC 服务端的 PEM CA 路径 |
| `grpc_tls_domain_name` | 可选，Agent 侧 gRPC TLS 证书校验的服务名覆盖 |
| `grpc_tls_client_cert_path` | 可选，Agent 侧 mTLS 客户端 PEM 证书路径 |
| `grpc_tls_client_key_path` | 可选，Agent 侧 mTLS 客户端 PEM 私钥路径 |
| `enrollment_token` | enrollment token |
| `agent_name` | Agent 名称；默认 `$(hostname)` |
| `version` | GitHub Release 版本，默认 `v0.1.0-alpha.3`；后台设置页默认会从 GitHub Releases 获取最新非草稿版本后传入 |

示例：

```bash
curl -fsSL 'http://dashboard.example.com:8080/api/v1/agents/install.sh?server_url=http%3A%2F%2Fdashboard.example.com%3A8080&grpc_server=http%3A%2F%2Fdashboard.example.com%3A50051&enrollment_token=xle_...&agent_name=%24(hostname)&version=v0.1.0-alpha.3' | sudo bash
```

`enrollment_token` 会进入 URL，建议使用短有效期 token。若需要让 Agent 连接到不同主机名的控制面，不要使用这个公开 bootstrap 端点；请直接从 GitHub Release 下载 `install-agent.sh` 并通过环境变量传入 `SERVER_URL` / `GRPC_SERVER`。

## CORS 和 Cookie

Dashboard 使用 Cookie 会话和 CSRF。跨源访问时：

- 后端必须配置精确的 `CORS_ALLOWED_ORIGINS`。
- 不能使用 `*`。
- 前端 `NEXT_PUBLIC_API_URL` 必须指向浏览器可访问的 API 地址。
