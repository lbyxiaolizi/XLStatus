# 规划变更日志

## 2026-06-20

- 新增 [16-komari-nezha-gap.md](./16-komari-nezha-gap.md)，把 Komari / Nezha 调研结果落成 P0/P1/P2 对标缺口计划。
- 在 [08-roadmap.md](./08-roadmap.md) 增加 M10-M15 后续里程碑：通知告警闭环、多租户资源管理、账号安全、备份 GeoIP 维护、主题多语言、provider 生态和兼容入口。
- 在 [14-api-contracts.md](./14-api-contracts.md) 补充 OAuth2、2FA、server transfer、通知成员、备份恢复、GeoIP、Cloudflared、主题和 MJPEG 状态图等未来 API 合约。
- 落地第一批对标功能：通知渠道/通知组管理和测试发送、常见通知 provider 模板、服务器批量分组/所有权转移、活跃会话列表和撤销、登录失败 WAF 封禁、证书到期告警条件、资源告警扩展和服务覆盖策略。
- 补齐备份和维护的可用子集：管理员维护状态接口、SQLite 备份下载、SQLite 备份上传校验/在线逻辑恢复、SQLite VACUUM 手动触发，以及设置页维护入口。
- 补齐 TOTP 两步验证基础闭环：标准 TOTP 密钥生成、启用/停用接口、设置页入口，以及登录二次验证码校验。
- 补齐 OAuth2/OIDC 基础闭环：配置驱动的通用 OIDC provider、账号绑定、已绑定账号登录、OAuth callback 前端同步页。
- 补齐 OAuth2/OIDC 绑定管理：当前账号绑定列表 API、provider 解绑 API，以及设置页已绑定/未绑定状态展示。
- 补齐 GeoIP/IP 变化基础闭环：GeoIP provider 测试接口、设置页查询入口、Agent IP 变化历史记录和基础通知发送。
- 补齐 Server group 基础闭环：独立分组表、CRUD 和成员管理 API、服务器页创建/删除/加入分组与 group/tag 双筛选。
- 补齐服务器批量管理：`servers/batch` 增加批量删除和移动到 Server group，服务器页选择工具栏提供对应操作。
- 补齐公开状态页私有站点开关：系统设置表和 API、设置页匿名访问切换、public API 访问拦截。
- 补齐任务触发闭环：告警 fired/recovered 和服务失败/恢复可配置任务 ID，并复用任务执行、批量结果和 task_runs 记录。
- 补齐服务器资产和公开隐私字段：服务器详情可维护私有备注、公开说明、账单/流量额度、公开状态页显示和游客隐藏，公开 API 不再泄露私有备注。
- 补齐服务器资产提醒：告警规则新增服务器到期和流量额度百分比条件，支持通知组和任务触发。
- 补齐 TSDB 手动维护入口：维护状态返回 TSDB 后端/状态/样本数，设置页可触发 retention compact 并查看移除样本数。
- 补齐 OAuth2/OIDC 失败 WAF 记录：OAuth 登录入口和 callback 会拦截活跃封禁 IP，失败路径写入 `oauth_failed` 并参与认证失败封禁计数。
- 补齐 PAT 暴力尝试 WAF 记录：`xlp_` Bearer token 失败写入 `pat_failed`，活跃封禁 IP 的 PAT 请求写入 `pat_blocked` 并直接拒绝。
- 补齐 Agent 鉴权失败 WAF 记录：gRPC `Session`/`IoStream` 失败写入 `agent_auth_failed`，活跃封禁 IP 写入 `agent_auth_blocked` 并拒绝建流。
- 补齐 GeoIP provider 持久化配置：系统设置保存默认 provider 和 ipinfo token，设置页可保存并用于 GeoIP 测试接口默认查询。
- 补齐管理员主动 WAF 封禁：新增批量创建 WAF ban API，设置页支持手动输入 IP 列表或从活跃会话来源 IP 一键封禁。
- 补齐 OpenAPI 和前端类型校验入口：新增 `GET /api/v1/openapi.json`，前端增加 `pnpm typecheck` / `pnpm api:contract` 校验脚本。
- 补齐 Server group 编辑 UI：服务器页常驻分组管理区支持创建、选择、编辑名称/颜色/排序、删除和成员数量查看。
- 补齐敏感操作二次校验：开启 TOTP 后，高风险管理操作要求 `x-totp-code`，设置页和服务器批量操作会按需弹出验证码。
- 补齐公开状态页地理聚合视图：公开页可切换世界地图，按公开资产字段聚合服务器地区并展示未识别项。
- 补齐任务覆盖和结果通知：任务 selector 支持排除服务器、触发来源服务器、Server group 和 tag 条件，任务执行结果可按通知组推送。
- 补齐 GeoIP MMDB 维护：新增 MMDB 状态、上传、直链下载更新和本机路径导入，`mmdb` provider 可读取本地数据库执行查询。
- 补齐 Agent IP 变化通知过滤：系统设置支持通知开关、服务器范围、通知组和 severity，IP 变化通知按配置发送。
- 补齐 DDNS 自定义 resolver：系统设置支持 DoH resolver URL，DDNS 检查可先按自定义 resolver 判断记录是否已同步。
- 补齐 OAuth2/OIDC provider 兼容选项：支持 token 鉴权方式、userinfo token 传递方式、授权请求额外参数和自定义 claim 字段映射。
- 补齐服务器所有权转移记录：批量转移写入 `server_owner_transfers` 和审计日志，管理员可在服务器页查看、重试失败记录或取消未完成记录。
- 补齐维护归档和动态 TSDB retention：完整归档下载包含 manifest、SQLite 快照和 TSDB 样本 JSON，设置页可保存 TSDB retention 并立即应用。
- 补齐公开状态页品牌和 MJPEG：系统设置维护站点名、Logo、favicon、背景、主题色和自定义 head/body，公开页应用品牌并提供 MJPEG 状态图流。
- 补齐通知 provider 预设扩展：新增 Email Webhook 模板，并支持通过 `XLSTATUS_NOTIFICATION_PROVIDER_PRESETS` 注入额外 webhook provider 模板。
- 补齐 Cloudflare Tunnel 管理入口：设置页可保存 token、启动/停止 cloudflared、查看状态、pid、错误和最近日志。
- 补齐主题模板和 i18n 底座：新增主题目录/导入/选择/删除 API，设置页主题管理面板，公开页和控制面应用选中主题；新增中英文 locale 持久化、导航/命令面板/公共错误动态切换。

## 2026-06-16 合并 plan 与 plan-mmx

作者：Codex

变更原因：

- 用户要求阅读 `./plan-mmx` 并合并两份计划，打造一份最终总计划。
- 结论是 `./plan` 的功能范围更完整，`./plan-mmx` 的工程执行细节更强，因此以 `./plan` 为权威目录吸收 `plan-mmx` 的优点。

主要变更：

- 新增 [00-decisions.md](./00-decisions.md)，形成核心决策表。
- 新增 [11-workspace-layout.md](./11-workspace-layout.md)，明确 workspace 和文件职责。
- 新增 [12-dependencies.md](./12-dependencies.md)，明确 Rust/Web 依赖和 feature 策略。
- 新增 [13-protocols.md](./13-protocols.md)，合并 Agent gRPC、Dashboard WS、MCP 协议。
- 新增 [14-api-contracts.md](./14-api-contracts.md)，给出 REST 合约和端点清单。
- 新增 [15-verification-commands.md](./15-verification-commands.md)，加入命令级验收。
- 更新 [README.md](./README.md)，将 `./plan` 标记为最终权威计划。
- 更新 [07-security.md](./07-security.md)，Agent 身份升级为 enrollment token + Ed25519 + 短期 JWT。
- 更新 [08-roadmap.md](./08-roadmap.md)，从 M1-M6 扩展为 M0-M9。

保留决策：

- 完整对标范围不缩水：Web Terminal、文件管理、DDNS、NAT、MCP 都进入 v1。
- PostgreSQL 支持前置到 M1，不放到后期补丁。
- 数据库后端运行时通过 `DATABASE_URL` 切换，而不是依赖重新编译。
