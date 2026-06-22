# API 概览

本文档是当前接口的发布级索引，不是完整 OpenAPI。路由实现以 `crates/server/src/api/` 和 `crates/server/src/main.rs` 为准。

## 基础

- HTTP API 默认监听：`127.0.0.1:8080`
- Agent gRPC 默认监听：`127.0.0.1:50051`
- 健康检查：`GET /healthz`
- 认证方式：Dashboard 使用 Cookie 会话和 CSRF；Agent 使用注册后的密钥和 JWT 流程。
- 所有 HTTP 请求的原始 path 在进入路由 extractor 前最多 4096 字节，原始 query string 最多 16KiB；具体接口可定义更小的业务上限。

## 公共接口

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/healthz` | 服务健康检查 |
| `GET` | `/api/v1/public/status` | 公开状态页数据 |
| `GET` | `/api/v1/public/servers/:id` | 公开服务器详情 |
| `GET` | `/api/v1/public/mjpeg` | 公开状态页 MJPEG 摘要流 |
| `POST` | `/api/v1/auth/login` | 登录 |
| `POST` | `/api/v1/agents/enroll` | Agent 注册 |
| `POST` | `/api/v1/agents/jwt/challenge` | Agent JWT challenge |
| `POST` | `/api/v1/agents/jwt` | Agent JWT 签发 |
| `GET` | `/install-agent.sh` | 带参数 Agent 安装 bootstrap |
| `GET` | `/api/v1/agents/install.sh` | 带参数 Agent 安装 bootstrap |
| `GET` | `/api/v1/transfers/temp/download` | 临时下载 |
| `PUT` | `/api/v1/transfers/temp/upload` | 临时上传 |

Agent 注册、JWT challenge 和 JWT 签发请求体上限为 4KiB。JWT nonce 为 32 字节 hex，signature 为 64 字节 hex；JWT challenge 只有在 Agent 签名验证通过后才会被消费。

登录请求体上限为 4KiB。`username` 最长 128 字节，`password` 最长 1024 字节，登录阶段的可选 TOTP code 必须是 6 位数字。

OAuth/OIDC provider id 最长 64 字节，只允许 ASCII 字母、数字、`-`、`_`、`.`；start 与 callback 的原始 query 最长 16KiB。OAuth/OIDC callback 公开参数有协议级长度边界：`state` 最长 4096 字节，`code` 最长 4096 字节，`error` / `error_description` 最长 1024 字节。OAuth start 的 `return_to` 只接受本地路径且最长 1024 字节。OIDC token response 最多读取 16KiB，`access_token` 最长 8192 字节；userinfo response 最多读取 64KiB，`sub`、`email`、`name`、`preferred_username` 等归一化 claim 最长 1024 字节。

临时下载/上传公开入口的原始 query 最长 512 字节，`token` 必须是 `xlt_` 加 64 字节 hex 的一次性 bearer token。临时上传请求体最多 100MiB；临时下载单次最多读取 100MiB Agent 文件内容，Agent 返回的下载结果会按 100MiB 文件内容对应的 base64 文本预算校验。

公开状态页只返回显式公开服务器和归属于公开服务器的服务结果。公开服务器列表和公开服务列表会在 SQL 层先过滤公开可见服务器/服务，再应用 100 条状态页摘要上限，避免私有对象挤占公开摘要。公开服务器详情路径 `:id` 必须是 36 字节 UUID。公开服务历史在 SQL 层按公开 `server_id` 过滤，每个服务最多返回最近 240 条结果。是否向匿名状态页显示服务器 CPU、内存、磁盘、网络、运行时间、连接数、进程数和详情页监控图表由 `public_server_details_enabled` 设置控制；开启时公开服务器详情监控图表最多返回 240 个采样点，关闭时仍保留公开服务器和服务状态摘要。公开 MJPEG 摘要流最多允许 32 个并发连接，帧内容使用 1 秒短 TTL 进程内缓存，并会在每次发送帧前重新校验公开状态页开关；关闭公开页后既有匿名 MJPEG 长连接会停止。

## 需要登录的接口

### 用户和认证

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/auth/logout` | 退出登录 |
| `GET` | `/api/v1/auth/totp/status` | TOTP 状态 |
| `POST` | `/api/v1/auth/totp/setup` | 生成或轮换 TOTP secret |
| `POST` | `/api/v1/auth/totp/enable` | 启用 TOTP |
| `POST` | `/api/v1/auth/totp/disable` | 停用 TOTP |
| `POST` | `/api/v1/users` | 创建用户 |
| `GET` | `/api/v1/users` | 用户列表 |
| `POST` | `/api/v1/users/:id` | 更新用户 |
| `DELETE` | `/api/v1/users/:id` | 删除用户 |
| `GET` | `/api/v1/sessions` | 会话列表 |
| `DELETE` | `/api/v1/sessions/:id` | 删除会话 |
| `GET` | `/api/v1/waf/bans` | WAF ban 列表 |
| `POST` | `/api/v1/waf/bans` | 手动创建 WAF ban |
| `DELETE` | `/api/v1/waf/bans/:id` | 删除 WAF ban |
| `GET` | `/api/v1/settings` | 系统设置 |
| `POST` / `PATCH` | `/api/v1/settings` | 更新系统设置 |
| `GET` | `/api/v1/themes` | 主题列表 |
| `POST` / `PUT` | `/api/v1/themes/import` | 导入自定义主题 |
| `POST` / `PATCH` | `/api/v1/themes/:id` | 更新自定义主题 |
| `DELETE` | `/api/v1/themes/:id` | 删除自定义主题 |
| `POST` | `/api/v1/themes/:id/select` | 选择主题 |
| `GET` | `/api/v1/tokens` | PAT 列表 |
| `POST` | `/api/v1/tokens` | 创建 PAT |
| `DELETE` | `/api/v1/tokens/:id` | 吊销 PAT |

用户创建/更新和 WAF 手动 ban 写入口请求体上限为 64KiB。用户 `username` 最长 128 字节，新密码必须为 8 到 1024 字节，`role` 最长 32 字节。TOTP setup/enable/disable 请求体上限为 1KiB，TOTP code 必须是 6 位数字。手动 WAF ban 最多接收 128 个 IP 输入项和 128 个唯一 IP，单个 IP 字段最长 4096 字节，reason 最长 255 字节，封禁时长限制为 1 到 43200 分钟。

PAT 创建请求体上限为 16KiB。PAT 名称最长 128 字节，scopes 最多 64 项、单项最长 128 字节，server allowlist 最多 64 个 UUID 并会规范化为 canonical 文本，`expires_at` 文本最长 64 字节且必须在未来 365 天内。

系统设置写入口请求体上限为 64KiB。公开站点名称最长 80 字节，公开 logo/favicon/background URL 各最长 500 字节，停用的自定义 head/body 字段最多只接受 1024 字节空白值。GeoIP ipinfo token 最长 4096 字节，GeoIP IP 变化服务器列表最多 64 个 UUID，通知组 ID 必须是 UUID，DDNS resolver URL 最长 2048 字节且必须是无 credentials/fragment 的 HTTP(S) URL。Cloudflared token 保存请求体上限为 16KiB，token 最长 8192 字节。

主题导入、更新、删除和选择写入口请求体上限为 64KiB。自定义主题最多保存 32 个，序列化后的自定义主题目录最多 256KiB。主题 id 最长 64 字节且只允许小写字母、数字、`-`、`_`，名称最长 120 字节，描述最长 500 字节；每组 CSS 变量最多 60 项，变量名最长 80 字节且必须以 `--` 开头，变量值最长 240 字节且不能包含 `;`、`{`、`}`。自定义 CSS 字段不会保存。

### Agent 和服务器

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/enrollment-tokens` | 创建 enrollment token，`expires_in_hours` 限制为 1 到 24 |
| `POST` | `/api/v1/agents/:id/revoke` | 撤销 Agent |
| `GET` | `/api/v1/servers` | 服务器列表 |
| `GET` | `/api/v1/servers/:id` | 服务器详情 |
| `POST` | `/api/v1/servers/:id` | 更新服务器展示元数据 |
| `POST` | `/api/v1/servers/batch` | 批量服务器管理 |
| `GET` | `/api/v1/server-groups` | 服务器分组列表 |
| `POST` | `/api/v1/server-groups` | 创建服务器分组 |
| `POST` / `PATCH` | `/api/v1/server-groups/:id` | 更新服务器分组 |
| `DELETE` | `/api/v1/server-groups/:id` | 删除服务器分组 |
| `POST` | `/api/v1/server-groups/:id/members` | 添加服务器分组成员 |
| `DELETE` | `/api/v1/server-groups/:id/members/:server_id` | 删除服务器分组成员 |
| `GET` | `/api/v1/servers/:id/metrics` | 指标查询 |
| `GET` | `/ws/servers` | 服务器实时 WebSocket |

服务器展示元数据更新、批量管理、服务器分组创建/更新/加成员请求体上限为 64KiB。服务器名称最长 128 字节，备注、公开说明、供应商、地域、套餐、价格、计费周期等展示 label 最长 512 字节；dashboard metadata 序列化后最多 16KiB。标签输入最多 64 项、单项最长 128 字节，保存后最多保留 8 个展示标签。批量服务器 ID 和分组成员单次最多 200 个 UUID，并会规范化为 canonical 文本。`display_order` 必须在数据库 `INTEGER` 范围内。

`/ws/servers` 实时 WebSocket 需要 `server:read`，升级前校验 `Origin`，初始快照和后续事件都会按当前 Cookie session / PAT 可见服务器集合过滤；非管理员只能接收自己名下服务器事件，PAT 还会受 server allowlist 限制。

Enrollment token 创建请求体上限为 4KiB，`expires_in_hours` 必须在 1 到 24 小时之间，默认 1 小时；创建入口只允许管理员 Cookie session。

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

服务创建、更新和测试探测请求体上限为 128KiB。服务名称最长 128 字节，target 最长 2048 字节，`interval_seconds` 必须在 10 到 86400 秒之间，`timeout_seconds` 必须在 1 到 30 秒之间。单个服务最多关联或排除 64 台服务器，失败/恢复触发任务各最多 32 个；引用的触发任务必须属于当前用户，且其任务选择器也必须对当前凭据可见。后台一次服务探测最多下发到 64 台 Agent。服务列表会在 SQL 层按当前凭据可见服务器过滤后再分页/count；服务列表和详情中的 `last_status`、`last_check_at`、证书摘要字段只从当前凭据可见服务器的 `service_results.server_id` 派生；服务历史和 uptime 会在 SQL 层按当前凭据可见的 `service_results.server_id` 过滤后再应用 `limit` / `offset` 或聚合。

### 告警

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/alert-rules` | 告警规则列表 |
| `POST` | `/api/v1/alert-rules` | 创建告警规则 |
| `DELETE` | `/api/v1/alert-rules/:id` | 删除告警规则 |
| `GET` | `/api/v1/alert-events` | 告警事件 |

告警规则创建请求体上限为 64KiB。规则名称最长 128 字节，单条规则最多 32 个条件、单个条件 JSON 最长 4KiB，失败/恢复触发任务各最多 32 个 UUID；引用的触发任务必须属于当前用户，且其任务选择器也必须对当前凭据可见。后台评估 `ServiceDown`、`ServiceLatency` 和 `CertificateExpiry` 服务类条件时，会按服务当前覆盖范围读取 `service_results.server_id`：`specific` 只读取当前绑定服务器，`local` 只读取 `server_id IS NULL` 的本地主控结果，`all` / `exclude` 只读取当前未撤销 Agent 集合及其排除集，不会使用已移除服务器或历史脏全局结果。

`GET /api/v1/alert-events` 会按规则 owner 与 PAT server allowlist 在 SQL 层过滤后再应用 `limit`。非管理员只能读取自己规则产生的事件；带 server allowlist 的 PAT 只能读取 `agent_id` 命中 allowlist 的事件，服务类事件如果没有可推断的唯一 `agent_id` 会保守隐藏。

通知创建、更新、通知组创建/更新和通知组加成员请求体上限为 128KiB。通知名称最长 128 字节，Webhook URL 最长 2048 字节，headers JSON 最长 16KiB、最多 32 个 header、单个 header name 最长 128 字节、value 最长 4096 字节，body template 最长 64KiB，渲染后的 URL 最长 4096 字节、请求体最长 128KiB。单个通知组最多 32 个渠道；告警、任务和 GeoIP IP 变更后台通知单次最多发送 32 个 Webhook，后台读取通知组时会同时按触发源 owner 过滤通知组和组内通知渠道。手动测试通知按用户和通知渠道设置 30 秒冷却。

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

任务创建/更新请求体上限为 256KiB。任务名称最长 128 字节，Shell 命令最长 8192 字节，`payload_json` 最长 64KiB，`server_selector_json` 最长 16KiB。选择器中每类显式 ID 最多 64 项、标签最多 32 项；一次任务执行最多解析并下发到 64 台服务器。任务列表和任务运行记录的 `limit` 会限制在 1 到 500；带 server allowlist 的 PAT 会先按任务选择器或运行记录 `server_id` 过滤，再应用 `offset` / `limit`。任务运行历史持久化前会把 Agent 返回的 stdout/stderr 分别截断到 64KiB、error 截断到 16KiB，并标记 `output_truncated`。

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

服务器文件、临时 URL、配置应用和强制更新 POST 请求体上限为 3MiB。文件路径最长 4096 字节；直接文件写入解码后最多 2MiB，大文件应使用临时上传 URL；文件读取单次最多 2MiB，Agent 返回的文件读取结果按对应 base64 文本预算校验，文件列表 Agent 返回 JSON 最长 2MiB，写入/删除等小结果最长 4KiB；配置 patch 序列化后最多 128KiB。强制更新下载 URL 最长 2048 字节。

终端 session 创建请求体上限为 4KiB。终端 WebSocket 单条浏览器文本消息最多 16KiB，单次输入转发给 Agent 前最多保留 8KiB；Agent 终端输出单帧最多 64KiB，关闭原因最多 1024 字节，错误消息最多 4096 字节。

强制更新需要 `server:exec` 权限、明确版本、HTTPS 下载 URL 和 SHA-256 校验和。默认只允许 `https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/xlstatus-agent-*` 这类官方 Agent release 资产；自托管更新源必须显式设置 `XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL=1`，但仍要求 HTTPS 和 SHA-256。

维护导出、SQLite VACUUM、TSDB compact 和 TSDB retention 均只允许管理员 Cookie session，敏感写操作仍要求 TOTP。TSDB retention 请求体上限为 4KiB，`retention_days` 必须在 1 到 3650 天之间；超出范围会拒绝而不是静默修正。

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

NAT 创建/更新请求体上限为 64KiB，并支持安全策略字段：`allowed_sources`、`max_active_tunnels`、`idle_timeout_seconds`、`max_bytes_per_tunnel`、`max_bandwidth_bytes_per_second`、`rate_limit_window_seconds`、`max_connections_per_window`、`max_bytes_per_window`。`agent_id` 必须是 UUID 并会规范化为 canonical 文本；`local_host` 最长 253 字节，默认只能是 Agent 本机 loopback 目标，如需转发到 Agent 所在内网其他主机，必须显式启用私网 NAT 目标环境变量。`description` 最长 1024 字节，`allowed_sources` 最长 4096 字节、最多 64 个 IP/CIDR 条目、单条最长 128 字节。单 mapping `max_active_tunnels` 最高 1024，`idle_timeout_seconds` 和 `rate_limit_window_seconds` 最高 86400 秒，单隧道/窗口字节上限最高 1TiB，带宽上限最高 1GiB/s，窗口连接数最高 100000。窗口字段按 mapping 和来源 IP 计数，用于限制窗口内连接数和累计双向流量。

DDNS 配置创建请求体上限为 64KiB。`provider` 只允许 `cloudflare`、`tencent_cloud`、`he`、`webhook`、`dummy`；`agent_id` 必须是 UUID 并会规范化为 canonical 文本。名称最长 128 字节，域名最长 253 字节，`record_id` / `zone_id` 各最长 128 字节，`api_token` / `api_key` / `api_secret` 各最长 4096 字节，`webhook_url` 最长 2048 字节且 webhook provider 必填并继续执行出站 SSRF 校验。

MCP POST 入口请求体上限为 1MiB。`/mcp` JSON-RPC batch 最多 16 项，空 batch 或超过上限会返回 `Invalid Request`。`server.exec` 命令最长 8192 字节，timeout 被限制在 1 到 60 秒，默认 30 秒；Agent 返回的 exec stdout/stderr 各最多 64KiB、error 最多 4KiB。MCP `fs.read` 单次最多读取 1MiB，返回 base64 文本按该预算校验；`fs.list` Agent 返回 JSON 最长 1MiB，`fs.write/delete` 小结果最长 4KiB。

GeoIP 测试接口请求体上限为 4KiB。GeoIP MMDB update 请求体上限为 16KiB，`source_url` 最长 2048 字节，`source_path` 最长 4096 字节；本地 `source_path` 必须是普通文件，读取前会按 128MiB 上限检查文件大小。GeoIP JSON provider 响应最多读取 16KiB，返回的 `raw` JSON 会限制字符串长度、数组项数、对象字段数和嵌套深度；MMDB 下载和上传文件上限均为 128MiB，下载路径会在读取过程中按上限中止。

## gRPC

Agent gRPC 服务定义在 `proto/xlstatus/v1/agent.proto`，生成代码在 `crates/proto-gen/`。默认消息大小限制为 `256 MiB`。该传输上限只用于兼容临时大文件传输；HTTP 文件操作、MCP、任务运行历史和后台服务监控会在消费 `TaskResult` 前按各自业务预算校验或截断 Agent 返回文本。

典型流程：

1. Agent 通过 HTTP enrollment 获取身份。
2. Agent 建立 gRPC session。
3. Server 通过 session 下发任务、IO、配置和操作。
4. Agent 持续上报主机状态和任务结果。

## Agent 安装链接

`GET /api/v1/agents/install.sh` 接收查询参数并返回一个很小的 bootstrap shell 脚本。真正的 `install-agent.sh` 放在 GitHub Release 资产中，bootstrap 只负责导出参数并下载执行 GitHub 脚本。

安全约束：公开 bootstrap 的原始 query 最长 16KiB，请求 `Host` authority 最长 512 字节。`server_url` 与 `grpc_server` 最长 2048 字节，只能是 `http` / `https` origin URL，不能包含 path、query、fragment 或 userinfo；若显式传入，host 必须与本次请求的 `Host` 相同，端口可以不同。未传 `server_url` 时使用当前请求 Host；未传 `grpc_server` 时在同 Host 上推导 `:50051`。会回显到 shell 脚本的参数会 trim 后校验长度并拒绝控制字符。

支持的参数：

| 参数 | 说明 |
|---|---|
| `server_url` | Dashboard HTTP API origin，必须与请求 Host 同主机 |
| `grpc_server` | Agent gRPC origin，必须与请求 Host 同主机，端口可不同 |
| `grpc_tls_ca_path` | 可选，Agent 侧用于验证 gRPC 服务端的 PEM CA 路径，最长 1024 字节 |
| `grpc_tls_domain_name` | 可选，Agent 侧 gRPC TLS 证书校验的服务名覆盖，最长 253 字节 |
| `grpc_tls_client_cert_path` | 可选，Agent 侧 mTLS 客户端 PEM 证书路径，最长 1024 字节 |
| `grpc_tls_client_key_path` | 可选，Agent 侧 mTLS 客户端 PEM 私钥路径，最长 1024 字节 |
| `enrollment_token` | enrollment token，最长 128 字节 |
| `agent_name` | Agent 名称，最长 255 字节；默认 `$(hostname)` |
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
