# API 概览

本文档是当前接口的发布级索引，不是完整 OpenAPI。路由实现以 `crates/server/src/api/` 和 `crates/server/src/main.rs` 为准。

## 基础

- HTTP API 默认监听：`0.0.0.0:8080`
- Agent gRPC 默认监听：`0.0.0.0:50051`
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
| `POST` | `/api/v1/enrollment-tokens` | 创建 enrollment token |
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

### 告警

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/alert-rules` | 告警规则列表 |
| `POST` | `/api/v1/alert-rules` | 创建告警规则 |
| `DELETE` | `/api/v1/alert-rules/:id` | 删除告警规则 |
| `GET` | `/api/v1/alert-events` | 告警事件 |

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

### 文件、配置和终端

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/servers/:id/files` | 文件列表 |
| `GET` | `/api/v1/servers/:id/files/read` | 读取文件 |
| `POST` | `/api/v1/servers/:id/files/write` | 写入文件 |
| `POST` | `/api/v1/servers/:id/files/delete` | 删除文件 |
| `GET` | `/api/v1/servers/:id/files/download-url` | 获取下载 URL |
| `GET` | `/api/v1/servers/:id/files/upload-url` | 获取上传 URL |
| `GET` | `/api/v1/servers/:id/config` | 读取 Agent 配置 |
| `POST` | `/api/v1/servers/:id/config` | 应用 Agent 配置 |
| `POST` | `/api/v1/servers/:id/force-update` | 触发更新 |
| `POST` | `/api/v1/terminal/sessions` | 创建终端会话 |
| `GET` | `/ws/terminal/:session_id` | 终端 WebSocket |

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

## gRPC

Agent gRPC 服务定义在 `proto/xlstatus/v1/agent.proto`，生成代码在 `crates/proto-gen/`。默认消息大小限制为 `256 MiB`。

典型流程：

1. Agent 通过 HTTP enrollment 获取身份。
2. Agent 建立 gRPC session。
3. Server 通过 session 下发任务、IO、配置和操作。
4. Agent 持续上报主机状态和任务结果。

## Agent 安装链接

`GET /api/v1/agents/install.sh` 接收查询参数并返回一个很小的 bootstrap shell 脚本。真正的 `install-agent.sh` 放在 GitHub Release 资产中，bootstrap 只负责导出参数并下载执行 GitHub 脚本。

支持的参数：

| 参数 | 说明 |
|---|---|
| `server_url` | Dashboard HTTP API 地址 |
| `grpc_server` | Agent gRPC 地址 |
| `enrollment_token` | enrollment token |
| `agent_name` | Agent 名称；默认 `$(hostname)` |
| `version` | GitHub Release 版本，默认 `v1.0.0` |
| `script_url` | 可选，自定义 GitHub 脚本地址 |

示例：

```bash
curl -fsSL 'http://dashboard.example.com:8080/api/v1/agents/install.sh?server_url=http%3A%2F%2Fdashboard.example.com%3A8080&grpc_server=http%3A%2F%2Fdashboard.example.com%3A50051&enrollment_token=xle_...&agent_name=%24(hostname)&version=v1.0.0' | sudo bash
```

`enrollment_token` 会进入 URL，建议使用短有效期 token。

## CORS 和 Cookie

Dashboard 使用 Cookie 会话和 CSRF。跨源访问时：

- 后端必须配置精确的 `CORS_ALLOWED_ORIGINS`。
- 不能使用 `*`。
- 前端 `NEXT_PUBLIC_API_URL` 必须指向浏览器可访问的 API 地址。
