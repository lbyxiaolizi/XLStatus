---
title: REST API 设计
status: stable
audience: [human, agent]
related_milestones: [M1]
---

# 07. REST API 设计

Dashboard ↔ Server REST API。所有端点以 `/api/v1/` 开头，JSON 通信，统一信封。

## 命名规范

- 前缀：`/api/v1/`
- 资源用复数名词：`/agents`、`/monitor-tasks`、`/alert-rules`
- ID：UUID v7（时间排序，便于索引）
- 字段：snake_case（JSON），数据库列也用 snake_case
- 操作：HTTP 动词（GET/POST/PATCH/DELETE）

## 响应信封

**成功（单对象）**：
```json
{ "data": { ... } }
```

**成功（列表 + 分页）**：
```json
{
  "data": [...],
  "page": { "next_cursor": "uuid" }
}
```

**错误**：
```json
{
  "error": {
    "code": "agent_not_found",
    "message": "Agent not found",
    "details": { "agent_id": "..." }
  }
}
```

## 状态码

| 场景 | 码 | error.code |
|------|----|------------|
| 成功（读/部分更新） | 200 | — |
| 成功（创建） | 201 | — |
| 成功（删除） | 204 | — |
| 校验失败 | 400 | `validation_error` |
| 未认证 | 401 | `unauthenticated` |
| 权限不足 | 403 | `forbidden` |
| 资源不存在 | 404 | `not_found` |
| 冲突（重名/重复） | 409 | `conflict` |
| 限流 | 429 | `rate_limited` |
| 服务器错误 | 500 | `internal_error` |

## 速率限制

| 端点 | 限制 | 实现 |
|------|------|------|
| `POST /auth/login` | 5 / min / IP | `tower_governor` |
| `POST /auth/refresh` | 30 / min / user | `tower_governor` |
| 其他 `/api/v1/*` | 1000 / min / user | `tower_governor` |
| Agent gRPC | 单 agent_id 单连接 | `SessionRegistry.register` 抢占 |

## 资源清单

### AuthUser

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| POST | `/auth/login` | 公开 | 用户名+密码登录，返回 access + refresh cookie |
| POST | `/auth/refresh` | 公开 | 用 refresh cookie 换新 access（rotation） |
| POST | `/auth/logout` | 自己 | 撤销当前 refresh session |
| GET  | `/auth/me` | 自己 | 当前用户信息 |

### Agent

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET    | `/agents`                       | viewer/admin | 列表 |
| POST   | `/agents`                       | admin        | 手动登记（enroll 用在 agent 端） |
| GET    | `/agents/{id}`                  | viewer/admin | 详情 |
| PATCH  | `/agents/{id}`                  | admin        | 改 name / tags |
| DELETE | `/agents/{id}`                  | admin        | 软删除 |
| POST   | `/agents/{id}/enrollment-token` | admin        | 颁发一次性 token |
| DELETE | `/agents/{id}/sessions`         | admin        | 踢出（撤销所有 agent_sessions） |
| GET    | `/agents/{id}/host`             | viewer/admin | 静态主机信息 |

### Agent Enroll（特殊，agent 端调用）

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| POST | `/agent/enroll` | 公开（持 token） | agent 首次注册，返回 agent_id + 首次 JWT |

### Sample（时序）

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET | `/agents/{id}/samples?from=...&to=...&metric=cpu&resolution=1m` | viewer/admin | 时序数据 |

### MonitorTask

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET    | `/monitor-tasks`                            | viewer/admin | 列表 |
| POST   | `/monitor-tasks`                            | admin        | 创建 |
| GET    | `/monitor-tasks/{id}`                       | viewer/admin | 详情 |
| PATCH  | `/monitor-tasks/{id}`                       | admin        | 改 |
| DELETE | `/monitor-tasks/{id}`                       | admin        | 删 |
| GET    | `/monitor-tasks/{id}/results?from=...&to=...` | viewer/admin | 历史结果 |

### AlertRule

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET    | `/alert-rules`                  | viewer/admin | 列表 |
| POST   | `/alert-rules`                  | admin        | 创建 |
| GET    | `/alert-rules/{id}`             | viewer/admin | 详情 |
| PATCH  | `/alert-rules/{id}`             | admin        | 改 |
| DELETE | `/alert-rules/{id}`             | admin        | 删 |
| GET    | `/alert-rules/{id}/events`      | viewer/admin | 该规则的触发历史 |

### AlertEvent

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET | `/alert-events?from=...&to=...&agent_id=...` | viewer/admin | 全量事件查询 |

### Notifier

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET    | `/notifiers`        | admin | 列表 |
| POST   | `/notifiers`        | admin | 创建 |
| GET    | `/notifiers/{id}`   | admin | 详情 |
| PATCH  | `/notifiers/{id}`   | admin | 改 |
| DELETE | `/notifiers/{id}`   | admin | 删 |
| POST   | `/notifiers/{id}/test` | admin | 发送测试消息 |

### User

| 方法 | 路径 | 角色 | 说明 |
|------|------|------|------|
| GET    | `/users`             | admin | 列表 |
| POST   | `/users`             | admin | 创建 |
| PATCH  | `/users/{id}/password` | admin / 自己 | 改密 |
| DELETE | `/users/{id}`        | admin | 软删除 |

## 关键端点示例

### `POST /auth/login`

请求：
```json
{ "username": "admin", "password": "..." }
```

响应（200）：
```json
{
  "data": {
    "user": { "id": "uuid", "username": "admin", "is_admin": true },
    "access_expires_at": "2026-06-16T11:00:00Z"
  }
}
```
Set-Cookie:
- `access_token=<jwt>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=900`
- `refresh_token=<token>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=604800`

错误（401）：
```json
{ "error": { "code": "unauthenticated", "message": "Invalid credentials" } }
```

### `POST /agent/enroll`

请求：
```json
{
  "token": "et_xxxxxxxx",
  "name": "my-host",
  "public_key": "base64(32 bytes ed25519 public key)"
}
```

响应（200）：
```json
{
  "data": {
    "agent_id": "0192...uuid",
    "agent_jwt": "eyJhbGciOiJIUzI1NiIs...",
    "agent_jwt_expires_at": "2026-06-16T11:05:00Z"
  }
}
```

错误（401）：
```json
{ "error": { "code": "enrollment_token_invalid", "message": "Token expired or used" } }
```

### `GET /agents/{id}/samples`

请求：
```
GET /api/v1/agents/0192.../samples?metric=cpu&from=2026-06-15T00:00:00Z&to=2026-06-16T00:00:00Z&resolution=5m
```

响应（200）：
```json
{
  "data": {
    "metric": "cpu",
    "resolution": "5m",
    "points": [
      { "ts": "2026-06-15T00:00:00Z", "value": 0.42 },
      { "ts": "2026-06-15T00:05:00Z", "value": 0.45 }
    ]
  }
}
```

支持的 metric：`cpu`, `mem_used`, `mem_used_pct`, `swap_used`, `disk_used`, `disk_used_pct`, `net_in_speed`, `net_out_speed`, `load1`, `load5`, `load15`, `uptime`, `tcp_conn_count`, `process_count`

支持的 resolution：`1m`, `5m`, `1h`, `1d`（PG 模式命中 CAGG；SQLite 走物化表）