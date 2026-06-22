# API 概览

本文档是当前接口的发布级索引，不是完整 OpenAPI。路由实现以 `crates/server/src/api/` 和 `crates/server/src/main.rs` 为准。

## 基础

- HTTP API 默认监听：`127.0.0.1:8080`
- Agent gRPC 默认监听：`127.0.0.1:50051`
- 健康检查：`GET /healthz`
- 认证方式：Dashboard 使用 Cookie 会话和 CSRF；Agent 使用注册后的密钥和 JWT 流程。
- 所有 HTTP 请求的原始 path 在进入路由 extractor 前最多 4096 字节，原始 query string 最多 16KiB；具体接口可定义更小的业务上限。
- 启用 `XLSTATUS_TRUST_PROXY_HEADERS=1` 且请求来源命中 `XLSTATUS_TRUSTED_PROXIES` 时，HTTP 审计 IP 会按 `x-forwarded-for`、`x-real-ip`、`cf-connecting-ip` 解析；这些 forwarded header 单项最多 1024 字节，超限或非法值会被忽略并回退 socket peer IP。

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

Agent 注册、JWT challenge 和 JWT 签发请求体上限为 4KiB。Agent 注册 `name` 会 trim 后保存，trim 后必须为 1 到 255 字节且不能包含控制字符；`enrollment_token` 必须是 `xle_` 加 64 位 ASCII hex；`public_key` 必须是 32 字节 Ed25519 hex 公钥。创建 enrollment token 和撤销 Agent 只允许管理员 Cookie session；若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`。JWT challenge/JWT 签发中的 `agent_id` 必须是 36 字节 canonical UUID 文本；JWT nonce 为 32 字节 hex，signature 为 64 字节 hex；JWT challenge 只有在 Agent 签名验证通过后才会被消费。管理员撤销 Agent 的 path `agent_id` 也必须是 36 字节 canonical UUID 文本，非法、simple 或大写 UUID 会在写入 `revoked_at` 前返回 400。管理员撤销 Agent 后，控制面会写入 `revoked_at`、向现有 session 发送 ForceDisconnect，并立即从进程内 session / IO registry 摘除该 Agent；撤销后的迟到注册或发送路径不会继续接收任务或 IO 帧，后续 JWT/gRPC 认证也会拒绝该 Agent。

登录请求体上限为 4KiB。`username` 最长 128 字节，`password` 最长 1024 字节，登录阶段的可选 TOTP code 必须是 6 位数字。

OAuth/OIDC provider id 最长 64 字节，只允许 ASCII 字母、数字、`-`、`_`、`.`；login start 与 callback 的原始 query 最长 16KiB。公开登录发起入口仍使用 `GET /api/v1/oauth2/:provider`；账号绑定发起入口使用受 CSRF 保护的 `POST /api/v1/oauth2/:provider/bind`，返回授权跳转 URL 并设置 OAuth state cookie。账号绑定发起和解绑只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`。OAuth/OIDC callback 公开参数有协议级长度边界：`state` 最长 4096 字节，`code` 最长 4096 字节，`error` / `error_description` 最长 1024 字节。OAuth start 的 `return_to` 只接受本地绝对路径，必须以单个 `/` 开头，不能包含反斜杠或控制字符，且最长 1024 字节；不满足时回退 `/dashboard`。OIDC token response 最多读取 16KiB，`access_token` 最长 8192 字节；userinfo response 最多读取 64KiB，`sub`、`email`、`name`、`preferred_username` 等归一化 claim 最长 1024 字节。OIDC userinfo token 默认只允许通过 Bearer header 传递；`userinfo_auth_method=query` / `access_token_query` 会被拒绝，只有显式设置 `XLSTATUS_ALLOW_OIDC_USERINFO_QUERY_TOKEN=1` 时才作为旧 provider 兼容逃生阀启用。

临时下载/上传公开入口的原始 query 最长 512 字节，`token` 必须是 `xlt_` 加 64 字节 hex 的一次性 bearer token。临时 URL 使用前会重新校验签发 session/PAT 仍有效、仍具备对应 scope、仍可访问目标服务器，且目标 Agent 必须未撤销；已撤销服务器的历史临时 URL 会在消费一次性 token 前返回 403。临时上传请求体最多 100MiB；临时下载单次最多读取 100MiB Agent 文件内容，Agent 返回的下载结果会按 100MiB 文件内容对应的 base64 文本预算校验。临时传输公开响应会发送 `Cache-Control: no-store`、`Pragma: no-cache` 和 `Expires: 0`，避免一次性 URL 响应被浏览器或代理缓存。

公开状态页默认允许匿名访问；管理员可通过 `public_site_enabled=false` 将公开状态页、公开服务器详情和 MJPEG 摘要流设为私有。公开状态页只返回显式公开且未撤销的服务器，以及归属于这些公开服务器的服务结果。公开服务器列表和公开服务列表会先按解析后的 dashboard metadata 判断公开可见性，再应用 100 条状态页摘要上限，避免私有对象或 Postgres TEXT metadata 字符串误命中挤占公开摘要。公开服务器详情路径 `:id` 必须是 36 字节 canonical UUID 文本；详情接口同样要求服务器未撤销，已撤销服务器即使历史元数据仍标记公开也会返回 404。公开服务读取历史 `service_servers` 关系时会复用单服务最多 64 台服务器预算，超过预算的历史脏服务会被跳过，避免匿名状态页无界读取服务关联。公开服务历史在 SQL 层按公开 `server_id` 过滤，每个服务最多返回最近 240 条结果。匿名状态页默认显示公开服务器的 CPU、内存、磁盘、网络、运行时间、连接数、进程数和监控图表；管理员可通过 `public_server_details_enabled=false` 关闭这些服务器详细信息，关闭后仍保留公开服务器和服务状态摘要。开启服务器详细信息时，公开状态摘要和公开服务器详情的监控图表最多各返回 240 个采样点。公开 MJPEG 摘要流最多允许 32 个并发连接，帧内容使用 1 秒短 TTL 进程内缓存，并会在每次发送帧前重新校验公开状态页开关；关闭公开页后既有匿名 MJPEG 长连接会停止。

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
| `GET` | `/api/v1/transfers/temp/tokens` | 临时传输 token 列表 |
| `POST` | `/api/v1/transfers/temp/tokens/:id/revoke` | 撤销临时传输 token |

系统设置读取/更新只允许管理员 Cookie session；更新系统设置时若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`。系统设置写入口请求体上限为 64KiB；单个历史 setting JSON 读取预算同为 64KiB，超预算或类型错误的历史公开开关会 fail-closed，公开 branding、GeoIP、DDNS 和 Cloudflared 运行时设置读取时也会重新执行写入口预算与形态校验，非法历史值按默认/空值降级或被拒绝。GeoIP IP 变化通知的 `geoip_ip_change_server_ids` 最多 64 个 UUID，会规范化为 canonical 文本，并要求目标 Agent 当前未撤销；历史设置读取不会自动删除已撤销服务器 ID。`geoip_ipinfo_token` 最长 4096 字节，Cloudflared token 最长 8192 字节；启动 Cloudflared 时 token 通过 `TUNNEL_TOKEN` 环境变量传给子进程，不会放入 `cloudflared` 命令行参数。公开 logo/favicon/background URL 只允许无 credentials/fragment 的 HTTP(S) URL，背景图 URL 还会拒绝 CSS `url()` 破坏字符；DDNS resolver URL 最长 2048 字节，写入前要求 HTTP(S)、包含 host、无 credentials、无 fragment。

用户创建/更新和 WAF 手动 ban 写入口请求体上限为 64KiB。用户、session 和 WAF ban path id 必须是 36 字节 canonical UUID 文本；非法或非 canonical ID 会在写入口执行前返回 400。用户 `username` 最长 128 字节，新密码必须为 8 到 1024 字节，`role` 最长 32 字节。TOTP setup/enable/disable 请求体上限为 1KiB，TOTP code 必须是 6 位数字。手动 WAF ban 最多接收 128 个 IP 输入项和 128 个唯一 IP，单个 IP 字段最长 4096 字节，reason 最长 255 字节，封禁时长限制为 1 到 43200 分钟。

PAT 创建请求体上限为 16KiB。PAT 名称最长 128 字节，scopes 最多 64 项、单项最长 128 字节，server allowlist 最多 64 个 UUID 并会规范化为 canonical 文本，且 allowlist 内服务器必须存在、未撤销；非管理员 PAT 只能引用自己拥有的服务器。PAT 吊销 path id 必须是 36 字节 canonical UUID 文本，非法、simple 或大写 UUID 会在 repository/SQL 前返回 400。`expires_at` 文本最长 64 字节且必须在未来 365 天内。运行时 `Authorization: Bearer` 里的 PAT 必须是 `xlp_` 加 64 位 hex，完整 header 最长 75 字节；超长或畸形 `xlp_` bearer 会在 token hash 和数据库查询前被拒绝。Cookie session token 必须是 64 位 hex，形态不匹配的 cookie 不会进入 hash/数据库查询。读取历史 PAT 时，`scopes` 和 `server_ids` 必须保持预期 JSON 形态，并继续满足 scopes 最多 64 项、单项最长 128 字节、server allowlist 最多 64 项、单项最长 64 字节的运行时资源预算；历史脏 `server_ids` 不会被解释为全局 allowlist，超预算或无效 PAT 行会被视为不可用。

临时传输 token 列表需要 `transfer:read`，撤销需要 `transfer:write`，并继续按 owner/admin 与 PAT server allowlist 过滤可见性。撤销 path `:id` 必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 repository/SQL 前返回 400。

历史 session 行必须保持 UUID 形态的 `id` / `user_id` 和 RFC3339 时间形态；无效历史 session 会被视为不可用，不会通过 Cookie 认证、OAuth bind 校验或临时 URL 签发者再校验。

系统设置写入口请求体上限为 64KiB，启用 TOTP 时必须携带有效 `x-totp-code`。公开站点名称最长 80 字节，公开 logo/favicon/background URL 各最长 500 字节且必须是无 credentials/fragment 的 HTTP(S) URL；公开背景 URL 还会拒绝会破坏 CSS `url()` 的引号、括号、反斜杠和控制字符。停用的自定义 head/body 字段最多只接受 1024 字节空白值。GeoIP ipinfo token 最长 4096 字节，GeoIP IP 变化服务器列表最多 64 个 UUID，通知组 ID 必须是 UUID，DDNS resolver URL 最长 2048 字节且必须是无 credentials/fragment 的 HTTP(S) URL。Cloudflared token 保存请求体上限为 16KiB，token 最长 8192 字节。

主题导入、更新、删除和选择写入口请求体上限为 64KiB，且只允许管理员 Cookie session 修改或选择主题；若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`。主题 path `:id` 必须是已规范化 slug：最长 64 字节且只允许小写字母、数字、`-`、`_`；导入 body 中的主题 id 会 trim 并转小写后保存为同一 slug 形态。自定义主题最多保存 32 个，序列化后的自定义主题目录最多 256KiB。主题名称最长 120 字节，描述最长 500 字节；每组 CSS 变量最多 60 项，只允许既定颜色变量名（如 `--bg-page`、`--accent-color`、`--btn-bg` 等）和安全颜色值（hex、`rgb(a)`、`hsl(a)`），会拒绝 `url()`、`var()`、自定义 CSS 片段和未知变量名。历史自定义主题读取时会清空 custom CSS、丢弃不安全变量，并过滤非法 id、内置主题冲突、重复 id、非法 target/name/description/timestamp 和超预算目录；历史 selected theme id 必须已经是规范 slug，否则会被忽略。

### Agent 和服务器

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/api/v1/enrollment-tokens` | 创建 enrollment token，启用 TOTP 时要求 `x-totp-code`，`expires_in_hours` 限制为 1 到 24 |
| `POST` | `/api/v1/agents/:id/revoke` | 撤销 Agent，启用 TOTP 时要求 `x-totp-code` |
| `GET` | `/api/v1/servers` | 服务器列表 |
| `GET` | `/api/v1/servers/:id` | 服务器详情 |
| `POST` | `/api/v1/servers/:id` | 更新服务器展示元数据 |
| `POST` | `/api/v1/servers/batch` | 批量服务器管理 |
| `GET` | `/api/v1/server-transfers` | 服务器所有权转移记录 |
| `POST` | `/api/v1/server-transfers/:id/retry` | 重试服务器所有权转移 |
| `POST` | `/api/v1/server-transfers/:id/cancel` | 取消服务器所有权转移 |
| `GET` | `/api/v1/server-groups` | 服务器分组列表 |
| `POST` | `/api/v1/server-groups` | 创建服务器分组 |
| `POST` / `PATCH` | `/api/v1/server-groups/:id` | 更新服务器分组 |
| `DELETE` | `/api/v1/server-groups/:id` | 删除服务器分组 |
| `POST` | `/api/v1/server-groups/:id/members` | 添加服务器分组成员 |
| `DELETE` | `/api/v1/server-groups/:id/members/:server_id` | 删除服务器分组成员 |
| `GET` | `/api/v1/servers/:id/metrics` | 指标查询 |
| `GET` | `/ws/servers` | 服务器实时 WebSocket |

服务器展示元数据更新、批量管理、服务器分组创建/更新/加成员请求体上限为 64KiB。服务器名称最长 128 字节，备注、公开说明、供应商、地域、套餐、价格、计费周期等展示 label 最长 512 字节；dashboard metadata 序列化后最多 16KiB。标签输入最多 64 项、单项最长 128 字节，保存后最多保留 8 个展示标签。服务器详情、更新、指标 path `:id`，所有权转移列表 query `server_id`，所有权转移 retry/cancel path `:id`，服务器分组 path `:id`、成员 path `:server_id`，批量 `server_ids`、`owner_user_id` 和 `group_id` 都必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 SQL/管理操作前返回 400。批量服务器 ID 和分组成员单次最多 200 个 UUID。批量删除、所有权转移、所有权转移 retry/cancel 属于敏感服务器写操作，启用 TOTP 时必须携带有效 `x-totp-code`。服务器分组创建、更新、删除、成员增删和批量 `move_group` 只允许 Cookie session；若账号已启用 TOTP，还必须携带有效 `x-totp-code`，PAT 不能改变会影响任务 group selector 展开的分组结构。服务器标签写入和批量 `set_tags` / `add_tags` / `remove_tags` 只允许 Cookie session；若账号已启用 TOTP，还必须携带有效 `x-totp-code`，PAT 不能改变会影响任务 tag selector 展开的标签集合。修改或清空会影响公开状态页的 `public_note`、`dashboard_visible`、`hide_for_guest`，以及批量 `set_dashboard_visible`，只允许 Cookie session；若账号已启用 TOTP，还必须携带有效 `x-totp-code`。`display_order` 必须在数据库 `INTEGER` 范围内。服务器分组新增成员和批量 `move_group` 都要求目标 Agent 当前未撤销；删除历史成员仍按可见性允许，以便清理撤销后的残留分组关系。服务器分组列表会先按分组 owner、有效成员 owner 和 PAT server allowlist 过滤后再应用 `limit` / `offset`；分组详情返回的 `server_ids` 只包含该分组 owner 名下且当前凭据可见的 Agent。

`/ws/servers` 实时 WebSocket 需要 `server:read`，升级前校验 `Origin`，初始快照和后续事件都会按当前 Cookie session / PAT 可见服务器集合过滤；非管理员只能接收自己名下服务器事件，PAT 还会受 server allowlist 限制。

Enrollment token 创建请求体上限为 4KiB，`expires_in_hours` 必须在 1 到 24 小时之间，默认 1 小时；创建入口只允许管理员 Cookie session。

历史 Agent 行必须保持 UUID 形态的 `id` / `owner_user_id` 和 RFC3339 时间形态；无效历史 Agent 行会被视为不可用或在列表中跳过，不会打崩服务器列表、实时快照、公开状态页或后台读取路径。

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

服务创建、更新、删除和测试探测请求体上限为 128KiB。服务监控配置变更和测试探测只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能保存、删除服务监控或触发测试探测。服务名称最长 128 字节，target 最长 2048 字节，`interval_seconds` 必须在 10 到 86400 秒之间，`timeout_seconds` 必须在 1 到 30 秒之间。服务详情、更新、删除、历史和 uptime path `:id` 必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 SQL 前返回 400。单个服务最多关联或排除 64 台服务器，失败/恢复触发任务各最多 32 个；创建或更新服务时，显式关联和排除的服务器必须存在且未撤销；引用的触发任务 ID 必须是 36 字节 canonical UUID 文本，且任务必须属于当前用户、任务选择器也必须对当前凭据可见。后台服务监控、告警引擎服务类条件和公开状态页读取历史服务关联时都会重新校验 `service_servers` 数量，畸形或超预算历史行会被跳过并记录 warning；后台服务监控读取历史 enabled 配置时也会重新校验 service/owner/notification UUID、probe type、cover mode、名称/target/interval/timeout、排除服务器 JSON 和触发任务 JSON，不会把 malformed `exclude_server_ids_json` 解释为空排除列表，也不会把负数时间转换成大正数。后台一次服务探测最多下发到 64 台 Agent；`specific`、`all` 和 `exclude` 覆盖模式只会在服务 owner 拥有的未撤销 Agent 集合内展开，缺少有效 owner 的历史远端服务不会全局展开。本地主控 ICMP 探测只把校验后的 IP literal 传给系统 `ping`，并在 `timeout_seconds + 2` 秒后终止等待；`ping` stdout/stderr 在解析或错误展示前最多读取 4096 字节。Agent 侧远端 HTTP/TCP/ICMP 探测也会在执行端把任务 timeout 夹取到 1 到 30 秒，0 使用默认 10 秒；Agent 侧 ICMP 只把解析后的 IP literal 传给系统 `ping`，设置 `kill_on_drop`，在 `timeout_seconds + 2` 秒后停止等待，`ping` stdout/stderr 在解析或错误展示前最多读取 4096 字节。服务列表会在 SQL 层按当前凭据可见服务器过滤后再分页/count；服务列表和详情中的 `last_status`、`last_check_at`、证书摘要字段只从当前凭据可见服务器的 `service_results.server_id` 派生；服务历史和 uptime 会在 SQL 层按当前凭据可见的 `service_results.server_id` 过滤后再应用 `limit` / `offset` 或聚合。

### 告警

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/alert-rules` | 告警规则列表 |
| `POST` | `/api/v1/alert-rules` | 创建告警规则 |
| `DELETE` | `/api/v1/alert-rules/:id` | 删除告警规则 |
| `GET` | `/api/v1/alert-events` | 告警事件 |

告警规则创建请求体上限为 64KiB。告警规则创建和删除只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能保存或删除告警规则。规则名称最长 128 字节，单条规则最多 32 个条件、单个条件 JSON 最长 4KiB，删除规则 path `:id` 必须是 36 字节 canonical UUID 文本；失败/恢复触发任务各最多 32 个 36 字节 canonical UUID。告警条件数值会被限制在运行时安全范围内：`consecutive_failures` 为 1 到 100，`max_latency_ms` 为 1 到 86400000，`days_before` 为 0 到 3650，`offline_seconds` 和资源条件 `duration_seconds` 最长 31536000 秒，流量百分比必须大于 0 且不超过 1000000，资源阈值必须是有限数且绝对值不超过 1000000000000。直接引用 Agent 的规则条件必须指向当前凭据可见且未撤销的服务器，直接引用 Service 的规则条件必须使用 36 字节 canonical UUID `service_id` 且服务对当前凭据可见，引用的触发任务必须属于当前用户，且其任务选择器也必须对当前凭据可见。`GET /api/v1/alert-rules` 会先按规则 owner 与 PAT server allowlist 过滤，再应用 `limit` / `offset`；无 server allowlist 的请求会在 SQL 层按 owner/id/count/page 查询，带 server allowlist 的请求会分批扫描候选规则并在可见性过滤后分页，避免不可见规则挤占页大小。后台读取历史 enabled 规则时会重新校验 rule/owner/notification/task UUID、trigger mode、规则名称、conditions JSON、单条件预算和条件数值边界；畸形或超预算历史规则会被跳过并记录 warning，不会让单条脏规则阻断整轮告警评估，也不会把畸形触发任务 JSON 静默解释为空列表。后台评估历史规则时会重新校验每个条件引用的 Agent 或 Service 必须仍属于规则 owner，跨 owner、已删除或已撤销资源条件会被跳过。后台评估 `ServiceDown`、`ServiceLatency` 和 `CertificateExpiry` 服务类条件时，会按服务当前覆盖范围读取 `service_results.server_id`：`specific` 只读取当前绑定且属于服务 owner 的未撤销服务器，`local` 只读取 `server_id IS NULL` 的本地主控结果，`all` / `exclude` 只读取服务 owner 拥有的未撤销 Agent 集合及其排除集，不会使用已移除服务器、其他 owner 服务器或历史脏全局结果。

`GET /api/v1/alert-events` 会按规则 owner 与 PAT server allowlist 在 SQL 层过滤后再应用 `limit`。非管理员只能读取自己规则产生的事件；带 server allowlist 的 PAT 只能读取 `agent_id` 命中 allowlist 的事件，服务类事件如果没有可推断的唯一 `agent_id` 会保守隐藏。

通知创建、更新、删除、通知组创建/更新/删除、通知组成员增删和手动测试通知只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能保存第三方通知凭据、修改通知扇出或触发测试 Webhook 出站请求。通知创建、更新、通知组创建/更新和通知组加成员请求体上限为 128KiB。通知名称最长 128 字节，Webhook URL 最长 2048 字节，headers JSON 最长 16KiB、最多 32 个 header、单个 header name 最长 128 字节、value 最长 4096 字节，body template 最长 64KiB，渲染后的 URL 最长 4096 字节、请求体最长 128KiB。运行时通知消息在模板渲染前会限制 title 512 字节、message 4096 字节、timestamp 128 字节、metadata 最多 32 项、metadata key 128 字节、metadata value 4096 字节，且 metadata key/value 不能包含换行。单个通知组最多 32 个渠道；通知、通知组和通知组成员增删 path/body id 必须是 36 字节 canonical UUID 文本，非法、simple、大写或带空格 UUID 会在 SQL 或测试发送前返回 400。通知组成员增删会校验 group 和 notification 都属于当前用户。告警、任务和 GeoIP IP 变更后台通知单次最多发送 32 个 Webhook，后台读取通知组时会同时按触发源 owner 过滤通知组和组内通知渠道。手动测试通知按用户和通知渠道设置 30 秒冷却。

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

任务创建/更新请求体上限为 256KiB。任务名称最长 128 字节，Shell 命令最长 8192 字节，`payload_json` 最长 64KiB，`server_selector_json` 最长 16KiB。任务创建、更新、删除和手动运行只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能保存、删除或触发远程任务。任务详情、更新、删除、手动运行和运行历史 path `:id` 必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 repository/调度前返回 400。选择器中每类显式 ID 最多 64 项、标签最多 32 项；显式 `server_ids` 和 `exclude_server_ids` 必须是任务 owner 名下未撤销服务器。一次任务执行最多解析并下发到 64 台服务器。后台定时任务每轮最多扫描 1024 条 enabled 且带 schedule 的任务；读取历史定时任务时会重新执行任务定义预算校验，畸形或超预算历史行会被跳过并记录 warning，不会让单条脏任务阻断整轮调度读取。任务执行时，group selector 只展开任务 owner 名下未撤销 Agent 成员，历史跨 owner、已删除或已撤销分组成员不会进入目标集合。Agent Shell 执行端会把 timeout 夹取到 1 到 60 秒，0 使用默认 30 秒；stdout/stderr 采集预算最高 64KiB，0 使用默认 64KiB，子进程设置 `kill_on_drop` 且超时后主动 kill。任务列表和任务运行记录的 `limit` 会限制在 1 到 500；带 server allowlist 的 PAT 会先按任务选择器或运行记录 `server_id` 过滤，再应用 `offset` / `limit`。任务运行历史持久化前会把 Agent 返回的 stdout/stderr 分别截断到 64KiB、error 截断到 16KiB，并标记 `output_truncated`。

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

服务器文件、临时 URL、配置应用和强制更新 POST 请求体上限为 3MiB，path `:id` 必须是 36 字节 canonical UUID 文本；非法、simple 或大写 UUID 会在 Agent 可见性校验和 token 写入前返回 400。文件路径最长 4096 字节；直接文件写入解码后最多 2MiB，大文件应使用临时上传 URL；文件读取单次最多 2MiB，Agent 返回的文件读取结果按对应 base64 文本预算校验，文件列表 Agent 返回 JSON 最长 2MiB，写入/删除等小结果最长 4KiB；临时下载/上传 URL 签发、配置应用和强制更新都要求目标 Agent 未撤销，撤销后的历史服务器只能继续在管理视图中清理旧记录，不能签发新的公开 bearer URL 或下发新的配置/更新消息；文件写入、删除和临时上传 URL 签发只允许 Cookie session，启用 TOTP 时必须携带有效 `x-totp-code`，PAT 不能修改 Agent 文件系统或签发上传 bearer URL；配置 patch 序列化后最多 128KiB。远程配置 patch 只接受明确 allowlist 字段；`name`、`report_interval_seconds`、`ip_report_interval_seconds` 可由 `server:write` 自动化更新，控制面地址、gRPC TLS 路径、安全开关和 `file_allowed_roots` 等敏感字段只允许 Cookie session，启用 TOTP 时必须携带有效 `x-totp-code`；`agent_id`、`public_key`、`private_key` 不允许远程修改。Agent 执行端会再次校验远程配置 patch 最大 128KiB、字段 allowlist、身份字段拒绝、名称/URL/路径/roots/间隔/布尔类型边界，并保留本地 `private_key`。强制更新下载 URL 最长 2048 字节。

终端 session 创建请求体上限为 4KiB，创建终端会话只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能创建交互式终端会话。创建 body `agent_id` 和 WebSocket path `:session_id` 都必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 Agent 可见性校验、registry 查找或 IO 打开前返回 400。创建和 WebSocket 升级前都会重新校验目标 Agent 当前未撤销。终端 WebSocket 单条浏览器文本消息最多 16KiB，单次输入转发给 Agent 前最多保留 8KiB；Agent 终端输出单帧最多 64KiB，关闭原因最多 1024 字节，错误消息最多 4096 字节。

强制更新需要 `server:exec` 权限、明确版本、HTTPS 下载 URL 和 SHA-256 校验和；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`。默认只允许 `https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/xlstatus-agent-*` 这类官方 Agent release 资产；自托管更新源必须显式设置 `XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL=1`，但仍要求 HTTPS 和 SHA-256。Agent 记录强制更新请求前也会拒绝 `latest`、非法版本、非 HTTPS、URL credentials/query/fragment、超长 URL 和非 SHA-256 checksum。

维护导出、SQLite VACUUM、TSDB compact 和 TSDB retention 均只允许管理员 Cookie session，敏感写操作仍要求 TOTP。维护导出附件响应会发送 `Cache-Control: no-store`、`Pragma: no-cache` 和 `Expires: 0`，避免数据库备份或完整归档被浏览器、代理或 CDN 缓存。SQLite 备份导出、完整归档和恢复验证的临时数据库文件只会写入本进程创建的私有临时目录，Unix 下目录权限为 `0700`，操作结束后会清理。恢复请求体上限为 512MiB。TSDB retention 请求体上限为 4KiB，`retention_days` 必须在 1 到 3650 天之间；超出范围会拒绝而不是静默修正。

### DDNS、NAT、MCP

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/ddns/configs` | DDNS 配置列表 |
| `POST` | `/api/v1/ddns/configs` | 创建 DDNS 配置，只允许管理员 Cookie session，启用 TOTP 时要求 `x-totp-code` |
| `DELETE` | `/api/v1/ddns/configs/:id` | 删除 DDNS 配置，只允许管理员 Cookie session，启用 TOTP 时要求 `x-totp-code` |
| `GET` | `/api/v1/ddns/configs/:id/history` | DDNS 历史 |
| `POST` | `/api/v1/ddns/reload` | 重载 DDNS providers，只允许管理员 Cookie session，启用 TOTP 时要求 `x-totp-code` |
| `POST` | `/api/v1/ddns/check-now` | 立即检查 DDNS，只允许管理员 Cookie session，启用 TOTP 时要求 `x-totp-code` |
| `GET` | `/api/v1/nat/mappings/all` | NAT 运行时映射总览 |
| `POST` | `/api/v1/nat/mappings` | 创建 NAT 映射 |
| `GET` | `/api/v1/nat/mappings/agent/:agent_id` | Agent NAT 映射 |
| `GET` | `/api/v1/nat/mappings/:id` | NAT 映射详情 |
| `POST` | `/api/v1/nat/mappings/:id` | 更新 NAT 映射 |
| `DELETE` | `/api/v1/nat/mappings/:id` | 删除 NAT 映射 |
| `GET` | `/api/v1/mcp/tools` | MCP 工具列表 |
| `POST` | `/api/v1/mcp/execute` | 执行 MCP 工具 |
| `GET` | `/api/v1/mcp/info` | MCP 信息 |
| `POST` | `/mcp` | MCP JSON-RPC |

NAT 创建/更新请求体上限为 64KiB，并支持安全策略字段：`allowed_sources`、`max_active_tunnels`、`idle_timeout_seconds`、`max_bytes_per_tunnel`、`max_bandwidth_bytes_per_second`、`rate_limit_window_seconds`、`max_connections_per_window`、`max_bytes_per_window`。NAT 映射创建、更新和删除只允许 Cookie session；若账号已启用 TOTP，请求必须携带有效 `x-totp-code`，PAT 不能改变 NAT 监听和转发配置。`agent_id` 和 mapping path id 必须是 36 字节 canonical UUID 文本；非法、simple 或大写 UUID 会在 repository/SQL 前返回 400。创建映射、启用映射或保持启用状态更新时，目标 Agent 必须未撤销。读取历史映射仍按当前凭据可见性允许；删除也要求敏感 Cookie/TOTP，以避免自动化 PAT 改变运行时监听面。NAT listener 热加载、新连接策略查询和 `/api/v1/nat/mappings/all` 运行时总览只加载未撤销 Agent 的 enabled 映射，且历史映射读取会重新校验端口、UUID、协议、`local_host` 默认 loopback 策略、文本长度、`allowed_sources` 和策略数值预算；超预算、负数、端口越界、未知协议或默认策略下非 loopback 目标的历史脏映射会被视为不可加载。Agent 撤销后会触发 NAT manager reload。`local_host` 最长 253 字节，默认只能是 Agent 本机 loopback 目标，如需转发到 Agent 所在内网其他主机，必须显式启用私网 NAT 目标环境变量；Agent 执行端收到 NAT Open 后也会重新解析目标并默认拒绝非 loopback 地址。`description` 最长 1024 字节，`allowed_sources` 最长 4096 字节、最多 64 个 IP/CIDR 条目、单条最长 128 字节。单 mapping `max_active_tunnels` 最高 1024，`idle_timeout_seconds` 和 `rate_limit_window_seconds` 最高 86400 秒，单隧道/窗口字节上限最高 1TiB，带宽上限最高 1GiB/s，窗口连接数最高 100000。窗口字段按 mapping 和来源 IP 计数，用于限制窗口内连接数和累计双向流量。

DDNS 配置创建、删除、provider reload 和手动 check-now 属于敏感 DDNS 写入/执行操作，只允许管理员 Cookie session；若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`，管理员 PAT 不能执行这些操作。DDNS 配置创建请求体上限为 64KiB。`provider` 只允许 `cloudflare`、`tencent_cloud`、`he`、`webhook`、`dummy`；`agent_id` 和 config path id 必须是 36 字节 canonical UUID 文本，非法、simple 或大写 UUID 会在 repository/SQL 前返回 400；创建时如果提供 `agent_id`，目标必须是当前配置 owner 名下未撤销 Agent。名称最长 128 字节，域名最长 253 字节，`record_id` / `zone_id` 各最长 128 字节，`api_token` / `api_key` / `api_secret` 各最长 4096 字节，`webhook_url` 最长 2048 字节且 webhook provider 必填并继续执行出站 SSRF 校验。后台 DDNS 执行时会重新校验历史配置中的 `agent_id` 必须解析为现存未撤销 Agent，且配置 owner 必须与 Agent owner 一致；不满足的历史配置会被跳过。Agent IP report 和历史 `last_state_json.primary_ip` 进入 DDNS provider 前必须是最长 64 字节、可解析为 `IpAddr` 的规范化 IP 文本，非法或超长值会被忽略。

MCP POST 入口请求体上限为 1MiB。`/mcp` JSON-RPC batch 最多 16 项，空 batch 或超过上限会返回 `Invalid Request`。当前 MCP 工具集包含 `meta.whoami`、`server.list`、`server.get`、`server.exec`、`fs.list`、`fs.read` 和 `fs.download_url`；MCP 不提供文件写入、删除或临时上传 URL 签发，旧客户端直接调用 `fs.write`、`fs.delete` 或 `fs.upload_url` 会失败，这类写侧文件操作必须使用 Cookie session 的 REST 接口，启用 TOTP 时还需要 `x-totp-code`。`server.list` 会先按 owner/admin 与 PAT server allowlist 过滤后再应用 `limit` / `offset`；管理员 PAT 可列出 allowlist 命中的跨 owner 服务器，非管理员 PAT 仍只列出自己拥有且 allowlist 命中的服务器。`server.get`、`server.exec`、`fs.list`、`fs.read` 和 `fs.download_url` 的 `server_id` 必须是 36 字节 canonical UUID 文本；非法、simple、大写或带空格 UUID 会在 Agent 查询、任务派发或临时 token 写入前返回错误。`server.exec` 命令最长 8192 字节，timeout 被限制在 1 到 60 秒，默认 30 秒；Agent 返回的 exec stdout/stderr 各最多 64KiB、error 最多 4KiB。MCP `server.exec`、`fs.list/read` 文件任务和 `fs.download_url` 签发临时 URL 都要求目标 Agent 当前未撤销；`fs.read` 单次最多读取 1MiB，返回 base64 文本按该预算校验；`fs.list` Agent 返回 JSON 最长 1MiB。

GeoIP 测试接口请求体上限为 4KiB，只允许管理员 Cookie session；若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`。GeoIP MMDB update/upload 属于敏感维护写操作，只允许管理员 Cookie session；若管理员已启用 TOTP，请求必须携带有效 `x-totp-code`。GeoIP MMDB update 请求体上限为 16KiB，`source_url` 最长 2048 字节，`source_path` 最长 4096 字节；本地 `source_path` 必须是普通文件，读取前会按 128MiB 上限检查文件大小。GeoIP JSON provider 响应最多读取 16KiB，返回的 `raw` JSON 会限制字符串长度、数组项数、对象字段数和嵌套深度；MMDB 下载和上传文件上限均为 128MiB，下载路径会在读取过程中按上限中止。

## gRPC

Agent gRPC 服务定义在 `proto/xlstatus/v1/agent.proto`，生成代码在 `crates/proto-gen/`。默认消息大小限制为 `256 MiB`。该传输上限只用于兼容临时大文件传输；HTTP 文件操作、MCP、任务运行历史和后台服务监控会在消费 `TaskResult` 前按各自业务预算校验或截断 Agent 返回文本。gRPC `authorization` metadata 只接受 `Bearer <JWT>`，JWT compact token 最长 4096 字节且必须是 3 段 base64url 形态；超长或畸形 token 会在 JWT 解码/HMAC 校验前被拒绝并记录 Agent 认证失败。可信代理 IP metadata 单项最多 1024 字节，超限时会被忽略并回退 socket peer IP。Agent `HostState` / `HostInfoUpdate` 遥测写入 `agents.last_state_json` / `last_info_json`、内存 TSDB、告警快照和实时 WebSocket 前会按业务预算归一化：每类磁盘/网卡/温度/HostInfo 磁盘数组最多保留 64 项，遥测字符串字段最长 128 字节且按 UTF-8 边界截断，单个持久化 JSON 最长 256KiB；发生裁剪时会带 `telemetry_truncated=true`。Agent `GeoIpReport` 的 `ipv4` / `ipv6` 字段最长 64 字节，必须解析为有效 `IpAddr` 并规范化后才会进入 GeoIP 事件、DDNS manager 或 realtime WebSocket；非法值不会广播。Agent 撤销后，Server 侧 session / IO registry 会立即移除并标记该 Agent，后续任务下发、终端/NAT IO 帧和迟到注册都会被拒绝；该进程内标记配合数据库 `revoked_at` 与 gRPC 重新认证拒绝一起收敛撤权窗口。

典型流程：

1. Agent 通过 HTTP enrollment 获取身份。
2. Agent 建立 gRPC session。
3. Server 通过 session 下发任务、IO、配置和操作。
4. Agent 持续上报主机状态和任务结果。

## Agent 安装链接

`GET /api/v1/agents/install.sh` 接收查询参数并返回一个很小的 bootstrap shell 脚本。真正的 `install-agent.sh` 放在 GitHub Release 资产中，bootstrap 只负责导出参数并下载执行 GitHub 脚本。bootstrap 响应会发送 `Cache-Control: no-store`、`Pragma: no-cache` 和 `Expires: 0`，避免包含 enrollment token 或控制面参数的脚本被缓存。

安全约束：公开 bootstrap 的原始 query 最长 16KiB，请求 `Host` 必须是最长 512 字节的纯 authority，不能包含 userinfo、path、query、fragment、反斜杠或控制字符。`server_url` 与 `grpc_server` 最长 2048 字节，只能是 `http` / `https` origin URL，不能包含 path、query、fragment 或 userinfo；若显式传入，host 必须与本次请求的 `Host` 相同，端口可以不同。未传 `server_url` 时使用当前请求 Host；未传 `grpc_server` 时在同 Host 上推导 `:50051`。会回显到 shell 脚本的参数会 trim 后校验长度并拒绝控制字符。

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
