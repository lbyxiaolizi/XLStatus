# Komari / Nezha 对标缺口计划

## 目标

本文件把 2026-06-20 对 `komari-monitor/komari` 和 `nezhahq/nezha`
的功能调研结果转化为 XLStatus 后续必须补齐的实施清单。这里记录的是
产品能力，不要求复制上游 API、数据库、前端资产或具体实现。

优先级定义：

- P0：影响核心监控、告警、通知和自动化闭环，必须优先实现。
- P1：影响生产可用性、多租户管理、安全运营和日常运维。
- P2：增强生态、外观、兼容和高级运营体验。

## P0 核心闭环

本轮已落地：

- 通知渠道和通知组 CRUD、测试发送接口、`/notifications` 管理页。
- 告警规则支持 HTTPS 证书即将到期条件 `certificate_expiry`。
- 告警资源规则扩展到 Swap、load5/load15、网络速度、累计流量、TCP/UDP、进程数、温度和 GPU。
- 服务监控覆盖策略支持主控、全部在线服务器、指定服务器和排除指定服务器。
- 告警 fired/recovered 与服务失败/恢复可配置触发任务，并复用同一任务执行和 task_runs 记录模型。

### 通知渠道和通知组

必须实现：

- 通知渠道 CRUD：Webhook GET/POST、JSON/Form body、自定义 Header、TLS 校验开关、指标单位格式化。
- 通知组 CRUD：组内多渠道成员管理。
- 通知测试接口和 UI 按钮。
- 通知发送结果可审计，失败记录不泄露敏感 body 明文。
- 前端 `/notifications` 页面，支持创建、编辑、删除、测试、加入/移出通知组。

验收：

- 管理员可在 UI 新建 webhook 通知，点击测试后收到请求。
- 告警规则绑定通知组后，触发和恢复都会发送通知。
- 通知 URL 指向 localhost、私网、链路本地、保留地址或非 HTTP/HTTPS 时被拒绝。

### 告警指标扩展

必须实现：

- 资源规则补齐：GPU、Swap、网络入/出/总速度、累计流量、周期流量、load1/load5/load15、TCP/UDP 连接数、进程数、温度。
- 服务证书规则：证书变化、即将过期、已过期。
- 规则持续窗口和采样不足保护。

验收：

- 任一资源规则均可配置、保存、触发和恢复。
- HTTPS 证书到期和证书变化可触发告警。
- 告警绑定任务后，失败和恢复各自能触发指定任务。

### 服务监控覆盖策略

必须实现：

- 覆盖全部服务器、仅指定服务器、排除指定服务器。
- 新 Agent 默认加入指定监控的选项。
- 服务隐藏公开页、展示排序、延迟上下限通知。
- 按 server 的 1d/7d/30d 历史和 30 天 Up/Down 条。

验收：

- Agent 离线时该探测点不参与本轮成功判定。
- 指定排除列表后，被排除 Agent 不执行探测。
- 公开状态页不展示隐藏服务。

### 任务触发和批量结果

已落地子集：

- Cron 任务、手动任务、告警 fired/recovered 触发任务和服务失败/恢复触发任务共用 `dispatch_task_to_agents` 执行模型。
- 告警规则支持 `failure_task_ids` / `recovery_task_ids`。
- 服务监控支持 `failure_task_ids` / `recovery_task_ids`，仅在状态转换时触发，避免每轮重复执行。
- 批量执行返回 success、failure、offline、timeout 分类并写入 `task_runs`。
- 任务覆盖范围支持排除服务器、触发来源服务器、Server group 和 tag 条件，并在调度时限制在任务 owner 名下。
- 任务执行结果支持通知组推送，失败/离线/超时默认通知，成功结果可通过 `push_successful` 开关推送。

必须实现：

- 已全部落地，以下保留验收项。

验收：

- 批量执行返回 success、failure、offline、timeout 分类。
- 同一 Agent 的任务结果流串行化，避免并发写流损坏。
- Agent 禁用命令执行时，任务、终端、文件写入都被拒绝并记录 rejected/failure。

## P1 生产管理

本轮已落地：

- 基于服务器 tag 的批量分组设置、追加、移除。
- 独立 Server group CRUD、成员加入/移出 API 和服务器页基础管理入口。
- 批量设置公开状态页可见性。
- 管理员批量转移服务器所有者，PAT allowlist 仍参与单机授权检查。
- 设置页活跃会话列表和会话撤销。
- 登录失败 WAF 封禁记录、自动封禁、管理员查看和解除封禁。
- 通知 provider 预设模板：Telegram、Bark、ServerChan、Discord、Slack。
- 设置页维护入口、SQLite 备份下载 API、SQLite VACUUM 手动触发。
- TOTP 两步验证基础闭环：生成密钥、验证码启用/停用、登录二次校验。
- 通用 OIDC/OAuth2 基础闭环：配置驱动 provider、绑定入口、已绑定账号登录、OAuth callback 同步前端会话。
- GeoIP/IP 变化基础闭环：GeoIP provider 测试接口、设置页测试入口、Agent IP 变化历史记录和基础通知发送。
- 公开状态页私有站点开关：系统设置 API、设置页切换、匿名 public API 访问拦截。
- OAuth2/OIDC provider 兼容选项：支持 `client_secret_post`、`client_secret_basic`、public client、userinfo Bearer/query/none、授权 URL 额外参数，以及 subject/email/name/username claim 字段映射。

### 服务器分组、批量操作和所有权转移

已落地子集：

- `GET /api/v1/server-groups` 列出当前用户的 Server groups 和成员。
- `POST /api/v1/server-groups` 创建 Server group。
- `POST/PATCH /api/v1/server-groups/{id}` 更新名称、颜色和排序。
- `DELETE /api/v1/server-groups/{id}` 删除 Server group。
- `POST /api/v1/server-groups/{id}/members` 批量加入服务器。
- `DELETE /api/v1/server-groups/{id}/members/{server_id}` 移出服务器。
- 服务器页支持创建/删除 Server group、把选中服务器加入 group，并可按 Server group 或 tag 筛选。
- `POST /api/v1/servers/batch` 支持 `delete` 批量删除服务器。
- `POST /api/v1/servers/batch` 支持 `move_group` 将选中服务器移动到目标 Server group。
- 服务器页选择工具栏提供批量删除和移动到组操作。
- 服务器页提供常驻 Server group 管理区，支持创建、选择、编辑名称/颜色/排序、删除，以及查看成员数量。
- 服务器所有权转移写入 `server_owner_transfers` 操作记录，成功/失败/cancel/retry 写入审计日志。
- `GET /api/v1/server-transfers` 查看近期转移记录，`POST /api/v1/server-transfers/{id}/retry` 可重试失败/取消记录，`POST /api/v1/server-transfers/{id}/cancel` 可取消未完成记录。
- 批量所有权转移在事务内更新 owner、清理旧 owner 的 Server group membership，并保留 PAT allowlist 过滤。
- 服务器页管理员面板展示近期所有权转移记录，并提供失败记录重试和取消入口。

必须实现：

- 已全部落地，以下保留验收项。

验收：

- 成员只能看到和操作自己拥有或被授权的服务器。
- 批量操作对越权服务器静默拒绝或返回 permission_denied，不泄露无权资源细节。
- 所有权转移失败可回滚，成功后 Agent 使用新归属凭据重连。

### 账号安全和运营防护

已落地子集：

- `GET /api/v1/auth/totp/status` 查看当前账号 TOTP 状态。
- `POST /api/v1/auth/totp/setup` 生成标准 `otpauth://` TOTP 配置。
- `POST /api/v1/auth/totp/enable` 使用验证码启用 TOTP。
- `POST /api/v1/auth/totp/disable` 使用当前验证码停用 TOTP。
- 登录接口在密码验证后要求已启用账号提交 TOTP 验证码，验证通过后才签发 session。
- `GET /api/v1/oauth2/providers` 暴露已配置的 OAuth provider。
- `GET /api/v1/oauth2/{provider}` 发起已绑定账号的 OAuth 登录。
- `GET /api/v1/oauth2/{provider}/bind` 为当前登录用户发起 OAuth 绑定。
- `GET /api/v1/oauth2/callback` 完成授权码交换、账号绑定或 Cookie session 签发。
- `GET /api/v1/oauth2/bindings` 查看当前账号已绑定的 OAuth provider 和账号标识。
- `POST /api/v1/oauth2/{provider}/unbind` 解绑当前账号的 OAuth provider，支持清理已从配置移除的旧绑定。
- `GET /api/v1/profile` 供 OAuth callback 页同步前端本地用户状态。
- 设置页展示 OAuth provider 的已绑定/未绑定状态，并提供绑定和解绑操作。
- OAuth2/OIDC 登录入口和 callback 会检查活跃 WAF 封禁；state、provider error、缺少 code、token/userinfo 交换失败和未绑定账号都会写入 `oauth_failed` 并参与认证失败封禁计数。
- PAT Bearer token 鉴权失败会写入 `pat_failed`，与登录/OAuth 共享认证失败封禁计数；活跃封禁 IP 的 PAT 请求会被拦截并写入 `pat_blocked`。
- Agent gRPC Session/IoStream 鉴权失败会写入 `agent_auth_failed`，与登录/OAuth/PAT 共享认证失败封禁计数；活跃封禁 IP 的 Agent 流会被拦截并写入 `agent_auth_blocked`。
- 设置页 WAF 面板支持管理员手动批量创建 IP 封禁，也可从活跃会话一键封禁来源 IP。
- 敏感操作二次校验：开启 TOTP 后，用户管理、会话撤销、WAF 封禁管理、PAT 创建/撤销、SQLite 恢复/VACUUM、TSDB compact、服务器批量删除和所有权转移都要求 `x-totp-code` 验证。
- 可选强制认证模式：系统设置支持关闭公开状态页匿名访问，public API 会按开关拒绝匿名请求。
- OAuth2/OIDC provider 兼容选项支持 token endpoint 鉴权方式、userinfo token 传递方式、授权请求额外参数和自定义 claim 字段映射。

必须实现：

- 已全部落地，以下保留验收项。

验收：

- 用户可绑定 OAuth2 账号并使用 OAuth2 登录。
- 开启 2FA 后，敏感操作必须校验 TOTP。
- 连续失败登录会进入 WAF，管理员可查看和解除封禁。

### 服务器资产和公开隐私

已落地子集：

- 服务器详情和列表 API 返回私有 `remark`、独立 `public_note`、到期时间、续费价格、价格、币种、账单周期、自动续费、流量额度和额度类型。
- 服务器资产元数据支持 provider、region、plan、tag、accent color、display order 和公开状态页可见性。
- `hide_for_guest` 会让公开状态页列表、公开详情和公开指标接口都返回不可见。
- 公开 API 的兼容 `remark` 字段只来自 `public_note`，并清洗公开 `last_info` / `last_state` 中的私有备注键。
- 服务器详情页可编辑私有备注、公开说明、账单、流量额度、排序、状态页显示和游客隐藏。
- 告警规则新增 `server_expiry` 和 `server_traffic_quota` 条件，支持到期前 N 天提醒和流量额度百分比提醒，并复用通知组/任务触发。

验收：

- 公开 API 不返回管理员私有备注。（已满足）
- 到期前 N 天可发送提醒。（已满足）
- 流量额度达到配置百分比时发送提醒。（已满足）

### 备份、恢复和维护

已落地子集：

- `GET /api/v1/maintenance/status` 返回当前数据库后端和维护能力。
- `GET /api/v1/maintenance/backup` 支持管理员下载 SQLite 备份。
- `POST /api/v1/maintenance/restore` 支持管理员上传 SQLite 备份，先校验完整性、核心表和列兼容性，再执行在线逻辑恢复。
- `POST /api/v1/maintenance/sqlite-vacuum` 支持管理员触发 SQLite VACUUM。
- 设置页展示数据库维护能力，并提供 SQLite 备份下载、备份校验/恢复和 VACUUM 入口。
- `POST /api/v1/maintenance/tsdb-compact` 支持管理员手动执行当前 TSDB retention compact，并返回移除样本数。
- `POST /api/v1/maintenance/tsdb-retention` 支持管理员动态调整当前 TSDB retention，并持久化到系统设置。
- `GET /api/v1/maintenance/archive` 支持管理员下载完整维护归档，包含 manifest、SQLite 数据库快照和当前 TSDB 样本 JSON。
- 设置页展示 TSDB backend/status/sample count/retention，并提供 TSDB compact、retention 保存和完整归档下载入口。
- `GET /api/v1/openapi.json` 输出当前 REST 端点的 OpenAPI 3.1 JSON，前端集中 API 类型通过 `pnpm typecheck` / `pnpm api:contract` 校验。

必须实现：

- 已落地完整备份归档和动态 TSDB retention；真实外部 TSDB 后端维护入口需等待外部 backend 接入后落地。

验收：

- 备份恢复后用户、服务器、服务、任务、通知和历史指标可用。
- 维护任务可手动触发并输出审计事件。

### GeoIP 和 IP 变更

已落地子集：

- `GET /api/v1/geoip/test` 支持 `empty`、`geojs`、`ip-api`、`ipinfo` provider 测试任意 IP。
- `POST /api/v1/geoip/update` 预留 MMDB 更新入口，当前在未配置数据库源时返回未支持状态。
- Agent `GeoIpReport` 会写入 `agent_ip_events`，同一 Agent IP 变化时记录旧/新 IPv4、IPv6。
- Agent IP 变化后会向该 Agent owner 的通知渠道发送基础通知。
- 设置页提供 GeoIP 查询和 MMDB 更新入口。
- GeoIP provider 默认配置持久化到系统设置，支持 empty、geojs、ip-api、ipinfo、mmdb 选择和 ipinfo token 保存；测试接口未显式传 provider 时使用持久化默认值。
- MMDB provider 支持本地数据库查询、状态/版本读取、文件上传、直链下载更新和本机路径导入；设置页展示 MMDB 类型、构建时间、路径、大小和错误信息。
- Agent IP 变化通知支持系统设置开关、服务器范围过滤、指定通知组和通知级别配置，设置页可直接维护。
- DDNS 支持自定义 DoH resolver URL；配置后 DDNS 检查会先通过该 resolver 查询 A/AAAA，记录已同步状态或再调用 provider 更新。

必须实现：

- 已全部落地，以下保留验收项。

验收：

- 管理员可测试任意 IP 的 GeoIP 结果。
- IP 变化后在一个上报周期内触发通知。

## P2 生态和体验

### 主题、模板和多语言

已落地子集：

- 公开状态页品牌配置：站点名称、Logo、favicon、背景图、主题色、自定义 head/body 通过系统设置保存。
- 公开状态页会应用品牌配置到标题、favicon、theme-color、背景、Logo 和页面底部 custom body。
- 主题目录 API：`GET /api/v1/themes`、`POST/PUT /api/v1/themes/import`、`POST/PATCH /api/v1/themes/{id}`、`DELETE /api/v1/themes/{id}`、`POST /api/v1/themes/{id}/select`。
- 内置 BOLD Pink、Midnight Green、Clear Blue 主题；自定义主题以受校验 CSS variables 和 custom CSS 存入系统设置。
- 设置页主题模板面板支持刷新、JSON/文件导入、选择公开页主题、选择控制面主题、两端同时选择和删除自定义主题。
- 公开状态页 `GET /api/v1/public/status` 返回当前公开主题，前端应用 CSS variables 与 custom CSS。
- 控制面登录后会读取已选择的 Dashboard 主题并应用到全局 CSS variables 与专用 custom CSS。
- i18n 底座支持 `zh-CN` 和 `en-US`，提供 locale 持久化、动态翻译读取和语言切换事件。
- 导航、命令面板、通用错误提示、日期/数值空值文案会跟随语言切换。

必须实现：

- 已落地主题导入/选择/删除、公开页和控制面应用、中英文 i18n 底座；后续新增页面按同一资源模型继续补齐细颗粒页面文案。

验收：

- 管理员可在 UI 切换主题并立即影响公开状态页。
- 切换语言后导航、命令面板和通用错误提示会即时切换；页面级表单文案随后续页面改造逐步迁移到同一资源。

### 公开状态页增强

已落地子集：

- `GET /api/v1/settings` 和 `POST/PATCH /api/v1/settings` 支持 `public_site_enabled`。
- 设置页可切换公开状态页匿名访问。
- 关闭后 `/api/v1/public/status`、公开服务器详情和公开指标接口会拒绝匿名访问。
- 公开状态页支持可切换地理聚合视图，按公开 provider、region、plan、tag 和公开说明推断服务器地区并展示世界地图聚合。
- `GET /api/v1/public/mjpeg` 输出 multipart MJPEG 状态图流，展示站点名、整体状态、服务器和服务计数。
- 公开状态页支持自定义背景、Logo 和主题色。

必须实现：

- 已全部落地，以下保留验收项。

验收：

- 私有站点开启后，匿名用户无法访问公开服务器和服务数据。
- MJPEG URL 可持续输出更新的状态图片。

### 通知 provider 生态

已落地子集：

- 内置 Telegram、Bark、Email Webhook、ServerChan、Discord 和 Slack provider 模板。
- `XLSTATUS_NOTIFICATION_PROVIDER_PRESETS` 支持以 JSON 数组注入额外 provider 模板，复用现有 webhook 安全校验、模板渲染和测试发送能力。

必须实现：

- 已落地受控 webhook provider 模板扩展。任意 JavaScript provider 执行或后端插件默认不开放，避免绕过 SSRF 防护、RBAC 和审计。

验收：

- 常见 provider 不需要手写完整 webhook body 即可配置。
- 自定义 provider 的错误不会影响其他通知发送。

### 外部隧道、兼容和发现

已落地子集：

- Cloudflare Tunnel 管理入口：保存 token、启动/停止当前 server 托管的 `cloudflared tunnel run --token ...` 进程、查看状态、pid、最近日志和错误。
- 设置页提供 Cloudflare Tunnel token、启动、停止、状态和日志入口；敏感操作要求 TOTP。
- Agent auto-discovery key 由现有 enrollment token 和带参数安装脚本 URL 覆盖：管理员可生成限时 token，安装命令自动携带 server/grpc/enrollment/agent name。
- Nezha Agent 兼容入口完成评估：该协议需要独立 adapter 和鉴权模型；当前默认不暴露额外端口或兼容路由，避免绕过现有 enrollment、Ed25519、RBAC 和审计边界。

必须实现：

- 兼容入口必须默认关闭，开启后有单独鉴权和审计。

验收：

- 管理员可在 UI 启停 cloudflared 并看到状态。
- 兼容入口关闭时不暴露额外端口或路由。

### 扩展机制

已落地子集：

- 通知 provider 可通过 `XLSTATUS_NOTIFICATION_PROVIDER_PRESETS` 注入受控 webhook 模板扩展，复用现有 SSRF 防护、模板渲染、测试发送和审计路径。
- 公开页组件可通过品牌设置里的自定义 head/body 和背景/Logo/主题色进行受控扩展。
- DDNS 已支持自定义 DoH resolver，通知 provider、公开页和 DDNS 的轻量扩展均不绕过现有权限边界。

必须实现：

- 已落地通知 provider preset、公开页 head/body、公开页主题和 DDNS resolver 这类受控扩展点。
- 任意后端插件或受控 RPC 插件需先完成 ACL、审计和沙箱设计，当前不作为默认开放能力。

验收：

- 插件不能绕过 RBAC、PAT allowlist、CSRF、SSRF 防护和审计。

## 实施顺序

1. P0 通知管理 API/UI。
2. P0 告警指标、证书规则、任务触发。
3. P0 服务覆盖策略和任务覆盖策略。
4. P1 server group、批量操作、所有权转移。
5. P1 OAuth2、2FA、WAF、在线用户。
6. P1 资产字段、到期/流量提醒、备份恢复、GeoIP。
7. P2 主题、多语言、公开页增强、provider 生态、Cloudflare Tunnel 和兼容入口。

## 完成定义

本计划完成时，当前 XLStatus 应能覆盖 Komari 和 Nezha 的主要用户可见能力：

- 监控、告警、通知、任务、服务探测和公开状态页形成完整闭环。
- 多用户、多租户、PAT、WAF、2FA、OAuth2 满足生产安全运营。
- 管理员可通过 UI 完成服务器资产、分组、批量操作、备份恢复、GeoIP、DDNS、NAT、MCP 和主题配置。
- 所有新增能力都有对应 API、前端入口、权限校验、审计或日志、以及可重复验收命令。
