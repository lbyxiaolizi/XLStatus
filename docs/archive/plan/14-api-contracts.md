# REST API 合约

## 命名规范

- 前缀：`/api/v1`
- 资源名使用 kebab-case。
- 列表接口必须分页。
- 写接口必须返回审计可追踪的 `request_id`。

## 响应信封

成功：

```json
{
  "ok": true,
  "data": {},
  "request_id": "req_..."
}
```

失败：

```json
{
  "ok": false,
  "error": {
    "code": "forbidden",
    "message": "permission denied",
    "details": {}
  },
  "request_id": "req_..."
}
```

## 状态码

| 状态码 | 场景 |
--------|------|
| 200 | 查询成功 |
| 201 | 创建成功 |
| 204 | 删除或无 body 成功 |
| 400 | 参数格式错误 |
| 401 | 未认证 |
| 403 | 权限不足 |
| 404 | 资源不存在 |
| 409 | 并发冲突或乐观锁失败 |
| 422 | 业务校验失败 |
| 429 | 限流 |
| 503 | 数据库、队列或依赖不可用 |

## 核心端点

Auth：

- `GET /api/v1/openapi.json`
- `POST /api/v1/auth/login`
- `POST /api/v1/auth/refresh`
- `POST /api/v1/auth/logout`
- `GET /api/v1/profile`
- `PATCH /api/v1/profile`
- `GET /api/v1/oauth2/providers`
- `GET /api/v1/oauth2/{provider}`
- `GET /api/v1/oauth2/{provider}/bind`
- `GET /api/v1/oauth2/callback`
- `GET /api/v1/oauth2/bindings`
- `POST /api/v1/oauth2/{provider}/unbind`
- OAuth2/OIDC provider 配置支持 token endpoint 鉴权方式（`client_secret_post`、`client_secret_basic`、`none`）、userinfo token 传递方式、授权请求额外参数和自定义 claim 字段映射。
- OAuth2/OIDC login/callback 失败写入 WAF `oauth_failed`，并与密码登录失败共享认证失败封禁计数。
- PAT Bearer token 鉴权失败写入 WAF `pat_failed`，活跃封禁 IP 的 PAT 请求写入 `pat_blocked`，并与密码登录/OAuth2 共享认证失败封禁计数。
- `GET /api/v1/auth/totp/status`
- `POST /api/v1/auth/totp/setup`
- `POST /api/v1/auth/totp/enable`
- `POST /api/v1/auth/totp/disable`
- `GET /api/v1/2fa/generate`
- `POST /api/v1/2fa/enable`
- `POST /api/v1/2fa/disable`
- Agent gRPC `Session`/`IoStream` 鉴权失败写入 WAF `agent_auth_failed`，活跃封禁 IP 的 Agent 流写入 `agent_auth_blocked`，并与其他认证失败共享封禁计数。
- 启用 TOTP 的账号执行敏感写操作时必须提供 `x-totp-code` header；覆盖用户管理、会话撤销、WAF 封禁管理、PAT 创建/撤销、维护恢复/compact/vacuum、服务器批量删除和所有权转移。

Users：

- `GET /api/v1/users`
- `POST /api/v1/users`
- `PATCH /api/v1/users/{id}`
- `DELETE /api/v1/users/{id}`
- `GET /api/v1/sessions`
- `DELETE /api/v1/sessions/{id}`
- `GET /api/v1/waf/bans`
- `POST /api/v1/waf/bans`
- `DELETE /api/v1/waf/bans/{id}`

API Tokens：

- `GET /api/v1/api-tokens`
- `POST /api/v1/api-tokens`
- `DELETE /api/v1/api-tokens/{id}`

Servers：

- `GET /api/v1/servers`
- `POST /api/v1/servers`
- `POST /api/v1/servers/batch`
- `GET /api/v1/servers/{id}`
- `PATCH /api/v1/servers/{id}`
- `DELETE /api/v1/servers/{id}`
- `POST /api/v1/batch-delete/servers`
- `POST /api/v1/batch-move/servers`
- `GET /api/v1/server-transfers`
- `POST /api/v1/server-transfers/{id}/cancel`
- `POST /api/v1/server-transfers/{id}/retry`
- `GET /ws/server-transfers`
- `POST /api/v1/servers/{id}/enrollment-token`
- `POST /api/v1/servers/{id}/config`
- `GET /api/v1/servers/{id}/metrics`
- `POST /api/v1/servers/{id}/force-update`
- 批量 action 覆盖 `set_tags`、`add_tags`、`remove_tags`、`set_dashboard_visible`、`transfer_owner`、`move_group`、`delete`。
- 服务器所有权转移记录包含 `server_id`、from/to user、status、attempts、error 和时间戳；批量转移、retry、cancel 均写入审计日志，并继续受 PAT `server_ids` allowlist 约束。
- `PATCH /api/v1/servers/{id}` 支持资产/公开隐私字段：私有 `remark`、`public_note`、`hide_for_guest`、provider、region、plan、tags、accent color、display order、到期/续费、价格、币种、账单周期、自动续费和流量额度。
- 公开状态页服务器接口只返回 `public_note` 作为兼容 `remark`，并尊重 `dashboard_visible=false` 与 `hide_for_guest=true`。

Server groups：

- `GET /api/v1/server-groups`
- `POST /api/v1/server-groups`
- `POST /api/v1/server-groups/{id}`
- `PATCH /api/v1/server-groups/{id}`
- `DELETE /api/v1/server-groups/{id}`
- `POST /api/v1/server-groups/{id}/members`
- `DELETE /api/v1/server-groups/{id}/members/{server_id}`

Services：

- `GET /api/v1/services`
- `POST /api/v1/services`
- `GET /api/v1/services/{id}`
- `PATCH /api/v1/services/{id}`
- `DELETE /api/v1/services/{id}`
- `GET /api/v1/services/{id}/history`
- 服务创建/更新支持 `failure_task_ids` 和 `recovery_task_ids`。

Alert rules：

- `GET /api/v1/alert-rules`
- `POST /api/v1/alert-rules`
- `PATCH /api/v1/alert-rules/{id}`
- `DELETE /api/v1/alert-rules/{id}`
- 告警创建/列表支持 `failure_task_ids` 和 `recovery_task_ids`。
- 告警条件覆盖 `server_resource`、`server_offline`、`server_expiry`、`server_traffic_quota`、`service_down`、`service_latency`、`certificate_expiry`。

Tasks：

- `GET /api/v1/tasks`
- `POST /api/v1/tasks`
- `PATCH /api/v1/tasks/{id}`
- `DELETE /api/v1/tasks/{id}`
- `POST /api/v1/tasks/{id}/run`
- `GET /api/v1/task-runs`
- `server_selector_json` 支持 `server_ids`、`exclude_server_ids`、`group_ids`、`tag_names`/`tags` 和 `source_server`。
- `notification_group_id` 绑定任务结果通知组；失败/离线/超时默认通知，`push_successful=true` 时成功结果也通知。

Notifications：

- `GET /api/v1/notifications`
- `POST /api/v1/notifications`
- `PATCH /api/v1/notifications/{id}`
- `DELETE /api/v1/notifications/{id}`
- `POST /api/v1/notifications/{id}/test`
- `GET /api/v1/notification-groups`
- `POST /api/v1/notification-groups`
- `PATCH /api/v1/notification-groups/{id}`
- `DELETE /api/v1/notification-groups/{id}`
- `POST /api/v1/notification-groups/{id}/members`
- `DELETE /api/v1/notification-groups/{id}/members/{notification_id}`
- `GET /api/v1/notification-providers`

DDNS：

- `GET /api/v1/ddns`
- `POST /api/v1/ddns`
- `PATCH /api/v1/ddns/{id}`
- `DELETE /api/v1/ddns/{id}`
- `GET /api/v1/ddns/providers`

NAT：

- `GET /api/v1/nat`
- `POST /api/v1/nat`
- `PATCH /api/v1/nat/{id}`
- `DELETE /api/v1/nat/{id}`

Transfers：

- `GET /api/v1/transfers`
- `POST /api/v1/transfers/{id}/cancel`
- `POST /api/v1/transfers/{id}/retry`

Admin：

- `GET /api/v1/audit-logs`
- `GET /api/v1/settings`
- `PATCH /api/v1/settings`
- `GET /api/v1/waf`
- `DELETE /api/v1/waf/{id}`
- `GET /api/v1/online-users`
- `POST /api/v1/online-users/{id}/block`
- `GET /api/v1/backup`
- `GET /api/v1/maintenance/status`
- `GET /api/v1/maintenance/backup`
- `GET /api/v1/maintenance/archive`
- `POST /api/v1/maintenance/restore`
- `POST /api/v1/maintenance/sqlite-vacuum`
- `POST /api/v1/maintenance/tsdb-compact`
- `POST /api/v1/maintenance/tsdb-retention`
- 维护状态返回 TSDB backend/status/sample count、retention days 和是否支持动态 retention；完整归档包含 manifest、SQLite 快照和 TSDB 样本 JSON。
- `GET /api/v1/settings` / `POST/PATCH /api/v1/settings` 返回并更新 `public_site_enabled`、`geoip_provider` 和 ipinfo token 配置状态。
- `GET /api/v1/settings` / `POST/PATCH /api/v1/settings` 维护公开状态页品牌字段：站点名、Logo、favicon、背景、主题色、自定义 head/body。
- `GET /api/v1/settings` / `POST/PATCH /api/v1/settings` 同时维护 Agent IP 变化通知开关、通知组、服务器范围和 severity。
- `GET /api/v1/settings` / `POST/PATCH /api/v1/settings` 维护 `ddns_resolver_url`，供 DDNS 自定义 DoH resolver 使用。
- `POST /api/v1/restore`
- `POST /api/v1/maintenance`
- `GET /api/v1/geoip/status`
- `GET /api/v1/geoip/test`
- `GET /api/v1/geoip/test` 未传 provider 时使用系统默认 GeoIP provider；支持 `empty`、`geojs`、`ip-api`、`ipinfo`、`mmdb`。
- `POST /api/v1/geoip/update`
- `POST /api/v1/geoip/upload`
- MMDB 维护支持状态/版本读取、文件上传、直链下载更新和本机路径导入。
- `GET /api/v1/cloudflared/status`
- `POST /api/v1/cloudflared/token`
- `POST /api/v1/cloudflared/start`
- `POST /api/v1/cloudflared/stop`

Theme and public site：

- `GET /api/v1/themes`
- `POST /api/v1/themes/import`
- `PUT /api/v1/themes/import`
- `POST /api/v1/themes/{id}`
- `PATCH /api/v1/themes/{id}`
- `POST /api/v1/themes/{id}/select`
- `DELETE /api/v1/themes/{id}`
- `GET /api/v1/public/mjpeg`
- `GET /api/v1/public/mjpeg` 输出 multipart MJPEG 状态图流；`GET /api/v1/public/status` 返回公开页品牌配置和选中主题供前端应用。

## 限流

- 登录：每 IP 每分钟 5 次失败。
- PAT：每 token 每分钟 1000 请求，MCP 更严格。
- WebSocket：每用户最多 5 个服务器状态连接。
- Agent：同一 server_id 只允许一个活跃 gRPC session，新连接替换旧连接。

## OpenAPI

- `GET /api/v1/openapi.json` 输出 OpenAPI 3.1 JSON，覆盖当前主要 REST 端点和统一 `success/data/error` 响应信封。
- 前端 API 类型集中维护在 `web/lib/api.ts`，通过 `cd web && pnpm typecheck` 或 `cd web && pnpm api:contract` 校验。
