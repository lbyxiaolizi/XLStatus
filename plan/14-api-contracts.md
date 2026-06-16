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

- `POST /api/v1/auth/login`
- `POST /api/v1/auth/refresh`
- `POST /api/v1/auth/logout`
- `GET /api/v1/profile`
- `PATCH /api/v1/profile`

Users：

- `GET /api/v1/users`
- `POST /api/v1/users`
- `PATCH /api/v1/users/{id}`
- `DELETE /api/v1/users/{id}`

API Tokens：

- `GET /api/v1/api-tokens`
- `POST /api/v1/api-tokens`
- `DELETE /api/v1/api-tokens/{id}`

Servers：

- `GET /api/v1/servers`
- `POST /api/v1/servers`
- `GET /api/v1/servers/{id}`
- `PATCH /api/v1/servers/{id}`
- `DELETE /api/v1/servers/{id}`
- `POST /api/v1/servers/{id}/enrollment-token`
- `POST /api/v1/servers/{id}/config`
- `GET /api/v1/servers/{id}/metrics`
- `POST /api/v1/servers/{id}/force-update`

Server groups：

- `GET /api/v1/server-groups`
- `POST /api/v1/server-groups`
- `PATCH /api/v1/server-groups/{id}`
- `DELETE /api/v1/server-groups/{id}`

Services：

- `GET /api/v1/services`
- `POST /api/v1/services`
- `GET /api/v1/services/{id}`
- `PATCH /api/v1/services/{id}`
- `DELETE /api/v1/services/{id}`
- `GET /api/v1/services/{id}/history`

Alert rules：

- `GET /api/v1/alert-rules`
- `POST /api/v1/alert-rules`
- `PATCH /api/v1/alert-rules/{id}`
- `DELETE /api/v1/alert-rules/{id}`

Tasks：

- `GET /api/v1/tasks`
- `POST /api/v1/tasks`
- `PATCH /api/v1/tasks/{id}`
- `DELETE /api/v1/tasks/{id}`
- `POST /api/v1/tasks/{id}/run`
- `GET /api/v1/task-runs`

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

## 限流

- 登录：每 IP 每分钟 5 次失败。
- PAT：每 token 每分钟 1000 请求，MCP 更严格。
- WebSocket：每用户最多 5 个服务器状态连接。
- Agent：同一 server_id 只允许一个活跃 gRPC session，新连接替换旧连接。

## OpenAPI

- M1 起生成 OpenAPI。
- 前端类型要么从 OpenAPI 生成，要么集中维护并在 CI 校验。

