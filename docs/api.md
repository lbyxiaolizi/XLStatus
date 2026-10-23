# XLStatus API、WebSocket、MCP 与 gRPC 协议参考

本文档完整描述 XLStatus `v0.2` 的接口体系，涵盖 HTTP REST API（`/api/v1/*`）、实时 WebSocket 流（`/ws/servers`、`/api/v1/ws`）、AI-Native MCP 端点（`/mcp`、`/api/v1/mcp/*`）以及 Agent gRPC 控制/数据平面协议（`xlstatus.v1.AgentControl`、`xlstatus.v1.NatTunnel`）。

---

## 1. 认证、授权与安全请求头

### 1.1 四种认证通道

| 通道 | 适用场景 | 凭证传输方式 | 写操作 CSRF 要求 | 高危操作 TOTP 要求 |
| :--- | :--- | :--- | :--- | :--- |
| **Web Cookie 会话** | 浏览器控制台（`web/`） | `Cookie: xlstatus_session=<token>` | 必须携带 `X-CSRF-Token: <hmac>`（与 `xlstatus_csrf` Cookie 一致） | 启用 2FA 后，敏感操作必须携带 `X-TOTP-Code: <6位动态码>` |
| **Personal Access Token (PAT)** | 自动化脚本、CI/CD、MCP 客户端 | `Authorization: Bearer xlp_<secret>` | 免除（非浏览器环境） | 需具备对应细粒度 `scopes` 与 `server_ids` 节点白名单 |
| **Agent 短期 JWT** | 已注册 `xlstatus-agent` 节点 | `Authorization: Bearer <jwt>`（HTTP 与 gRPC Metadata） | 免除 | 仅通过 Ed25519 挑战-响应签发（`HS256` 固定、4096 字节上限） |
| **临时传输令牌 (`xlt_`)** | 远端文件单次流式上传/下载 | Query 参数 `?token=xlt_<secret>` | 免除 | 严格单次原子消费（`julianday(expires_at) > julianday(?)`）+ 客户端 IP 前置限流 |

### 1.2 细粒度 PAT 权限作用域（Scopes）与节点白名单（`server_ids`）

创建 PAT 时可指定细粒度 `scopes` 数组以及可选的 `server_ids`（UUID 数组）节点白名单：
- 当 `server_ids` 为 `null` 时，PAT 可访问当前用户角色可见的全部节点（管理员为全集群，普通成员为 `owner_user_id` 归属节点）。
- 当 `server_ids` 指定了节点 UUID 列表时，所有节点查询、指标读取、远程执行、服务拨测、告警规则、任务调度与 MCP 工具均在每次请求时强制校验目标节点子集且未吊销（对于服务与告警规则，还强制要求 `cover_mode == "specific"` 且 `exclude_server_ids` 为空）。
- 用户角色变更、密码重置或账号删除时，其关联的所有活跃 Session 与 PAT 立即自动失效。

| Scope 标识 | 权限说明 |
| :--- | :--- |
| `server:read` | 读取可见节点列表、详情、TSDB 历史指标、拓扑与分组信息 |
| `server:write` | 修改节点元数据、分组与配置 |
| `server:exec` | 在节点上执行远程 Shell 命令、批量任务或 MCP SRE 诊断 Playbook |
| `transfer:read` | 浏览远端目录（`fs.list`）、读取文件（`fs.read`）及签发临时下载链接 |
| `transfer:write` | 远端文件写入、删除与签发临时上传链接（注：MCP PAT 接口默认禁用写文件工具） |
| `service:read` / `service:write` | 查看或管理 HTTP/TCP/ICMP 服务监控探针及可用率历史 |
| `alert:read` / `alert:write` | 查看或管理告警规则、告警事件与通知渠道组 |
| `task:read` / `task:write` | 查看或管理批量/Cron 定时任务及触发执行 |
| `nat:read` / `nat:write` | 查看或管理 NAT 反向隧道端口映射 |
| `ddns:read` / `ddns:write` | 查看或管理 DDNS 动态域名解析配置 |

### 1.3 全局安全响应头与标准 JSON 响应包络

服务端全局中间件自动为所有响应注入以下安全头：
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `Referrer-Policy: strict-origin-when-cross-origin`
- `Permissions-Policy: accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()`
- 对所有 `/api/*`、`/mcp/*` 与安装脚本响应默认强制设置 `Cache-Control: no-store`。

所有 REST API 均返回统一的 JSON 包络结构：

```json
{
  "success": true,
  "data": { ... },
  "error": null
}
```

---

## 2. 公开接口、健康检查与引导脚本

| 方法 | 路径 | 认证 | 说明 |
| :--- | :--- | :--- | :--- |
| `GET` | `/healthz` | 公开 | 进程存活与就绪探针，返回 `200 OK` |
| `GET` | `/api/v1/public/status` | 公开 | 公开状态页聚合数据（仅返回公开未吊销节点的精简 DTO `PublicAgentSummary`、脱敏后的服务 SLA 与品牌主题配置） |
| `GET` | `/api/v1/public/servers` | 公开 | 公开节点列表 |
| `GET` | `/api/v1/public/servers/:id` | 公开 | 单个公开节点详情 |
| `GET` | `/api/v1/public/servers/:id/metrics` | 公开 | 单个公开节点 TSDB 历史指标（`range`: `15m`/`1h`/`6h`/`24h`） |
| `GET` | `/api/v1/public/servers/:id/services` | 公开 | 单个公开节点关联的服务监控 SLA 历史 |
| `GET` | `/api/v1/openapi.json` | 公开 | OpenAPI 3.0 规范文档 |
| `GET` | `/install.sh` | 公开 | 参数化 Server 安装引导脚本（校验 `Host` 头防缓存投毒） |
| `GET` | `/install-agent.sh` | 公开 | 参数化 Agent 安装引导脚本 |

---

## 3. 身份认证、2FA、会话、用户与 PAT 管理

| 方法 | 路径 | 权限 / 安全要求 | 说明 |
| :--- | :--- | :--- | :--- |
| `POST` | `/api/v1/auth/login` | 公开（WAF 限流） | 用户名 + 密码（+ 可选 `totp_code`）登录；对不存在的账号执行 Argon2id 虚拟哈希校验以消除计时枚举；单用户最多保留 `32` 个活跃会话 |
| `POST` | `/api/v1/auth/logout` | 已登录会话 + CSRF | 注销当前会话并清除 Cookie |
| `GET` | `/api/v1/auth/me` | Session / PAT | 获取当前登录用户、角色、2FA 状态与认证上下文 |
| `POST` | `/api/v1/auth/password` | Session + CSRF + TOTP | 修改当前用户密码（限制 `<= 1024` 字节且禁止 NUL 字符），成功后递增 `token_version` 并吊销旧会话 |
| `GET` | `/api/v1/auth/oauth/providers` | 公开 | 获取已启用的 OAuth2 / OIDC 登录提供商列表 |
| `GET` | `/api/v1/auth/oauth/:provider/start` | 公开 | 发起 OAuth2 PKCE + State 授权跳转 |
| `POST` | `/api/v1/auth/oauth/:provider/exchange` | 公开 | 使用一次性授权码交换会话 |
| `POST` | `/api/v1/auth/2fa/setup` | Session + CSRF | 生成加密存储（`HKDF-SHA256` `enc:v2:`）的 TOTP 密钥与 `otpauth://` 二维码 URI |
| `POST` | `/api/v1/auth/2fa/enable` | Session + CSRF | 校验 6 位动态码并正式启用 TOTP 2FA |
| `POST` | `/api/v1/auth/2fa/disable` | Session + CSRF + TOTP | 校验动态码并关闭 TOTP 2FA |
| `GET` | `/api/v1/auth/sessions` | Session | 列出当前账号的活跃登录会话 |
| `DELETE` | `/api/v1/auth/sessions/:id` | Session + CSRF + TOTP | 吊销指定会话 |
| `GET` / `POST` | `/api/v1/pats` | Session（创建需 CSRF + TOTP） | 列出或创建细粒度 PAT（`xlp_...` 明文仅在创建时返回一次） |
| `DELETE` | `/api/v1/pats/:id` | Session + CSRF + TOTP | 吊销并删除指定 PAT |
| `GET` / `POST` | `/api/v1/users` | Admin Session + TOTP | 管理员查看或创建系统用户 |
| `PUT` / `DELETE` | `/api/v1/users/:id` | Admin Session + TOTP | 修改用户角色/密码或删除用户（自动级联吊销该用户所有 Session 与 PAT） |

---

## 4. 节点管理、TSDB 指标、远程文件与 Web 终端

### 4.1 节点与指标查询

| 方法 | 路径 | Scope / 权限 | 说明 |
| :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/servers` | `server:read` | 分页获取可见节点列表与实时遥测快照（`limit` 自动钳制在 `1..=500`） |
| `GET` | `/api/v1/servers/:id` | `server:read` | 获取单节点硬件、系统与实时状态详情 |
| `PATCH` / `DELETE` | `/api/v1/servers/:id` | `server:write`（删除需 TOTP） | 更新节点名称、标签、公开可见性、到期时间或删除节点（同步断开 gRPC 会话并清理分组） |
| `POST` | `/api/v1/servers/:id/revoke` | Admin Session + TOTP | 吊销节点凭证并立即终止其 gRPC 控制流与 IO 流 |
| `GET` | `/api/v1/servers/:id/metrics` | `server:read` | 查询节点 TSDB 时序指标（CPU、内存、磁盘、网络、负载、进程、温度） |
| `GET` / `POST` | `/api/v1/server-groups` | `server:read` / `server:write` | 管理服务器分组与节点拓扑归属 |
| `GET` | `/api/v1/inventory/topology` | `server:read` | 获取分组与节点拓扑视图 |

### 4.2 远端文件、配置下发与 OTA 升级

| 方法 | 路径 | Scope / 权限 | 说明 |
| :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/servers/:id/files` | `transfer:read` | 浏览节点受控根目录（`file_roots`）下的文件列表 |
| `GET` | `/api/v1/servers/:id/files/content` | `transfer:read` | 读取远端普通文件内容（Agent 端强制 `O_NOFOLLOW \| O_NONBLOCK` + `S_IFREG` 校验，防止 FIFO/设备节点挂起） |
| `PUT` | `/api/v1/servers/:id/files/content` | Session + TOTP | 写入远端文件（拒绝 `setuid`/`setgid`/`sticky` 位与符号链接逃逸） |
| `DELETE` | `/api/v1/servers/:id/files` | Session + TOTP | 删除远端文件或目录（严格拦截 `"/"`、`"//"`、`"/."` 等根目录别名与 `..` 遍历段） |
| `POST` | `/api/v1/servers/:id/files/download-url` | `transfer:read` | 签发单次有效的临时下载令牌 URL（`/api/v1/temp-download?token=xlt_...`） |
| `POST` | `/api/v1/servers/:id/files/upload-url` | Session + TOTP | 签发单次有效的临时上传令牌 URL（`/api/v1/temp-upload?token=xlt_...`） |
| `POST` | `/api/v1/servers/:id/config` | Session + TOTP | 向在线 Agent 推送采集周期与安全开关配置（受 Agent 本地单调安全锁约束） |
| `POST` | `/api/v1/servers/:id/force-update` | Session + TOTP | 触发 Agent 安全 OTA 升级（强制校验 HTTPS/Loopback URL、无路径穿越与 64 位 SHA-256 摘要） |

### 4.3 交互式 WebSocket 终端

| 方法 | 路径 | 权限 / 安全要求 | 说明 |
| :--- | :--- | :--- | :--- |
| `POST` | `/api/v1/terminal/sessions` | Session + CSRF + TOTP | 在目标在线节点上创建一次性终端会话（返回 `session_id`） |
| `GET` | `/api/v1/terminal/sessions/:id/ws` | Session + Origin 校验 | 升级为双向 WebSocket 终端流（拒绝 PAT 升级；校验 `Origin` 防 CSWSH；单次原子附着；支持 `terminal.input` 与 `terminal.resize` `1..=500`） |

---

## 5. 服务拨测、告警自动化、批量任务、NAT 与 DDNS

| 领域 | 核心路由 | 安全与特性说明 |
| :--- | :--- | :--- |
| **服务监控 (`services`)** | `GET/POST /api/v1/services`<br>`PUT/DELETE /api/v1/services/:id`<br>`POST /api/v1/services/test`<br>`GET /api/v1/services/history`<br>`GET /api/v1/services/:id/history` | 支持 HTTP(S)、TCP、ICMP 探针；内置 DNS IP 绑定（`.resolve_to_addrs`）、禁用重定向、禁用环境代理与 `64 KiB` 流式响应上限；历史错误信息自动脱敏内网 URL 与凭据 |
| **告警与通知 (`alerts` / `notifications`)** | `GET/POST /api/v1/alerts/rules`<br>`GET /api/v1/alerts/events`<br>`GET/POST /api/v1/notifications/channels`<br>`GET/POST /api/v1/notifications/groups` | 支持离线、CPU/内存/磁盘阈值、证书到期与流量上限规则；Webhook 投递采用单遍模板渲染 + RFC 8259 JSON 转义；自愈触发任务严格校验租户归属一致性 |
| **批量与定时任务 (`tasks`)** | `GET/POST /api/v1/tasks`<br>`PUT/DELETE /api/v1/tasks/:id`<br>`POST /api/v1/tasks/:id/execute` | 支持 `Command`、`HttpGet`、`TcpPing`、`IcmpPing`；Cookie 会话创建/编辑/删除/执行 Shell 任务强制校验 TOTP；非 Shell 任务内置 SSRF 与参数注入拦截 |
| **NAT 反向隧道 (`nat`)** | `GET/POST /api/v1/nat`<br>`PUT/DELETE /api/v1/nat/:id` | 拦截 `< 1024` 特权端口与控制面端口（`3000`/`8080`/`50051`）；数据平面强制校验 `frame.agent_id` 防跨节点隧道劫持；单映射源 IP 追踪表上限 `4096` |
| **动态域名 (`ddns`)** | `GET/POST /api/v1/ddns`<br>`PUT/DELETE /api/v1/ddns/:id`<br>`POST /api/v1/ddns/:id/trigger` | 支持 Cloudflare、HE.net、DNSPod 与通用 Webhook；API Token 采用 `HKDF-SHA256` 静态加密存储并在响应中完全脱敏 |

---

## 6. AI-Native MCP (`2024-11-05`) 接口

详细工具清单、资源 URI 与客户端接入指南见 [docs/mcp.md](./mcp.md)。

| 方法 | 路径 | 认证方式 | 说明 |
| :--- | :--- | :--- | :--- |
| `POST` | `/mcp` | **仅限 PAT** (`Bearer xlp_...`) | 标准 MCP `2024-11-05` JSON-RPC 2.0 端点，支持 `initialize`、`ping`、`tools/list`、`tools/call`、`resources/list`、`resources/templates/list`、`resources/read`、`prompts/list`、`prompts/get` |
| `GET` | `/api/v1/mcp/catalog` | Session 或 PAT | 获取完整 MCP 工具（19 个）、资源（6 个）与 Prompt（4 个）目录定义（供前端 `/mcp` 工作台使用） |
| `POST` | `/api/v1/mcp/studio` | Session 或 PAT | Web AI 运维演练场执行入口（当使用 Cookie Session 调用 `server.exec` 或 `server.batch_exec` 时，强制要求 `X-TOTP-Code` 二次验证） |
| `GET` / `POST` | `/api/v1/mcp/info`<br>`/api/v1/mcp/tools`<br>`/api/v1/mcp/execute` | **仅限 PAT** (`Bearer xlp_...`) | 轻量 REST 风格 MCP 辅助接口 |

---

## 7. 实时 WebSocket 流与 Agent gRPC 接口

### 7.1 实时 WebSocket 流
- `GET /ws/servers` 与 `GET /api/v1/ws`：
  - 支持 Cookie Session（强制校验 `Origin` 头防跨站劫持）或 PAT 认证。
  - 限制单帧/单消息最大 `4096` 字节。
  - 在每次广播事件下发前通过 `realtime_event_visible` 动态复核当前用户的租户归属权、PAT `server_ids` 白名单及节点未吊销状态；`BroadcastHub` 自动递归剥离任何潜在敏感字段名。

### 7.2 Agent 注册与 gRPC 双流协议 (`proto/xlstatus/v1/`)
1. **HTTP 注册与挑战-响应签发**：
   - `POST /api/v1/agent/enroll`：凭一次性/限次注册令牌（`xle_...`）注册 Ed25519 公钥。
   - `POST /api/v1/agent/auth/challenge`：获取单次有效随机挑战值（服务端 Challenge 池设有容量上限防内存耗尽）。
   - `POST /api/v1/agent/auth/token`：验证 Ed25519 签名并签发短期 `HS256` Agent JWT。
2. **gRPC 服务 (`127.0.0.1:50051`)**：
   - `AgentControl::Session`：双向控制流，上报 `HostInfo`、`HostState`（每帧校验大小与字段边界）、`TaskResult`，接收服务端下发的 `ServerMessage`（Shell/探针任务、文件操作、配置更新、OTA 升级）。
   - `AgentControl::OpenIoStream`：双向数据平面流，承载交互式终端（`TerminalOpen`/`TerminalData`/`TerminalResize`/`TerminalClose`）与 NAT 隧道数据帧（`NatOpen`/`NatData`/`NatClose`，强制校验 `agent_id` 归属）。
