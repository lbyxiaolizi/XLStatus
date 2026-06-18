# API Documentation

XLStatus REST API reference for server automation and integration.

## Table of Contents

- [Overview](#overview)
- [Authentication](#authentication)
- [Base URL](#base-url)
- [Response Format](#response-format)
- [Error Handling](#error-handling)
- [Rate Limiting](#rate-limiting)
- [API Endpoints](#api-endpoints)

## Overview

The XLStatus API provides programmatic access to:
- Server and agent management
- Service monitoring configuration
- Alert rules and notifications
- Task scheduling and execution
- Metrics querying
- User management

**API Version**: v1
**Protocol**: REST over HTTP/HTTPS
**Content-Type**: `application/json`

## Authentication

### Public Status

`GET /api/v1/public/status` is intentionally unauthenticated. It powers the public `/status` web page and returns a read-only snapshot of recent server state and enabled service checks.

Example response:

```json
{
  "success": true,
  "data": {
    "servers": [
      {
        "id": "agent-id",
        "name": "web-01",
        "status": "online",
        "last_seen_at": "2026-06-18T12:00:00Z",
        "cpu_percent": 12.3,
        "memory_used": 123456789,
        "memory_total": 987654321,
        "load_1": 0.42
      }
    ],
    "services": [
      {
        "id": "service-id",
        "name": "Website",
        "service_type": "http",
        "kind": "http",
        "type": "http",
        "target": "https://example.com",
        "last_status": "success",
        "last_check_at": "2026-06-18T12:00:00Z"
      }
    ],
    "updated_at": "2026-06-18T12:00:00Z"
  },
  "error": null
}
```

All management endpoints under `/api/v1/servers`, `/api/v1/services`, `/api/v1/alert-rules`, `/api/v1/tasks`, `/api/v1/ddns`, `/api/v1/nat`, and `/api/v1/tokens` require an authenticated session or a PAT with the required scope.

### Session Cookies

For web applications:

```bash
# Login
curl -X POST https://dashboard.example.com/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin123"}' \
  -c cookies.txt

# Use session
curl https://dashboard.example.com/api/servers \
  -b cookies.txt
```

### Personal Access Tokens (Recommended)

For automation and scripts:

#### Generate Token

Dashboard UI: **Settings → Access Tokens → Generate Token**

Or via API:
```bash
curl -X POST https://dashboard.example.com/api/auth/tokens \
  -H "Content-Type: application/json" \
  -u admin:admin123 \
  -d '{
    "name": "CI/CD Pipeline",
    "expires_in": 7776000,
    "scopes": ["server:read", "task:write"]
  }'
```

Response:
```json
{
  "token": "pat_xxxxxxxxxxxxxxxxxx",
  "name": "CI/CD Pipeline",
  "expires_at": "2026-09-17T00:00:00Z",
  "scopes": ["server:read", "task:write"]
}
```

#### Use Token

```bash
curl https://dashboard.example.com/api/servers \
  -H "Authorization: Bearer pat_xxxxxxxxxxxxxxxxxx"
```

### Token Scopes

| Scope | Description |
|-------|-------------|
| `inventory:read` / `inventory:delete` | Inventory read / remove agents |
| `server:read` / `server:write` / `server:delete` / `server:exec` | Server CRUD and exec |
| `service:read` / `service:write` / `service:delete` | Service probe CRUD |
| `alert:read` / `alert:write` / `alert:delete` | Alert rule CRUD |
| `task:read` / `task:write` / `task:delete` / `task:exec` | Scheduled task CRUD and run-now |
| `ddns:read` / `ddns:write` / `ddns:delete` | Dynamic DNS CRUD |
| `nat:read` / `nat:write` / `nat:delete` | NAT mapping CRUD |
| `notification:read` / `notification:write` / `notification:delete` | Notification group CRUD |
| `transfer:read` / `transfer:write` | File transfer endpoints |
| `admin:*` | Admin-only catch-all (admin role required) |

Notes:

- Only the literal scope names above are accepted at PAT creation. Unknown names, the bare `*` wildcard, malformed scopes (`taskwrite`), and empty scope lists are rejected.
- The `admin:*` scope can only be issued to users with the `admin` role.
- A PAT may optionally include a `server_ids` allowlist (UUID list). When set, every server the PAT touches (e.g. `server_selector_json` of a task, or `agent_id` of a NAT mapping) must lie within the allowlist; otherwise the handler returns `403`.
- Cookie sessions (admin or member) implicitly satisfy every scope check at runtime; PATs must carry the matching scope or a namespace wildcard (e.g. `task:*`).
- The central validators live in `crates/server/src/auth/rbac.rs` (`KNOWN_SCOPES`, `validate_pat_scopes`, `validate_server_ids`, `can_access_server*`, `has_scope`) and are reused by the task, NAT, and PAT endpoints. 21 unit tests cover the negative paths.

For the design rationale, allowlist semantics, and per-endpoint scope map, see [rbac.md](./rbac.md).

## Base URL

```
https://dashboard.example.com/api
```

All endpoints are relative to this base URL.

## Response Format

### Success Response

```json
{
  "data": {
    "id": "srv_abc123",
    "name": "web-server-01",
    "status": "online"
  }
}
```

### Paginated Response

```json
{
  "data": [...],
  "pagination": {
    "page": 1,
    "per_page": 20,
    "total": 100,
    "total_pages": 5
  }
}
```

### Error Response

```json
{
  "error": {
    "code": "INVALID_INPUT",
    "message": "Invalid server name",
    "details": {
      "field": "name",
      "reason": "Name must be 3-50 characters"
    }
  }
}
```

## Error Handling

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| 200 | OK - Request succeeded |
| 201 | Created - Resource created |
| 204 | No Content - Request succeeded, no data returned |
| 400 | Bad Request - Invalid input |
| 401 | Unauthorized - Authentication required |
| 403 | Forbidden - Insufficient permissions |
| 404 | Not Found - Resource not found |
| 409 | Conflict - Resource already exists |
| 422 | Unprocessable Entity - Validation failed |
| 429 | Too Many Requests - Rate limit exceeded |
| 500 | Internal Server Error - Server error |
| 503 | Service Unavailable - Server overloaded |

### Error Codes

| Code | Description |
|------|-------------|
| `INVALID_INPUT` | Request validation failed |
| `UNAUTHORIZED` | Authentication failed |
| `FORBIDDEN` | Insufficient permissions |
| `NOT_FOUND` | Resource not found |
| `CONFLICT` | Resource already exists |
| `RATE_LIMITED` | Too many requests |
| `INTERNAL_ERROR` | Server error |

## Rate Limiting

| Limit | Requests per Minute |
|-------|---------------------|
| Unauthenticated | 60 |
| Authenticated | 300 |
| API Token | 1200 |

**Headers**:
```
X-RateLimit-Limit: 300
X-RateLimit-Remaining: 250
X-RateLimit-Reset: 1718612345
```

## API Endpoints

### Authentication

#### POST /api/auth/login

Login with username and password.

**Request**:
```json
{
  "username": "admin",
  "password": "admin123"
}
```

**Response** (200):
```json
{
  "data": {
    "user_id": "usr_abc123",
    "username": "admin",
    "role": "admin"
  }
}
```

#### POST /api/auth/logout

Logout and invalidate session.

**Response** (204): No content

#### GET /api/auth/me

Get current user info.

**Response** (200):
```json
{
  "data": {
    "id": "usr_abc123",
    "username": "admin",
    "email": "admin@example.com",
    "role": "admin",
    "created_at": "2026-06-01T00:00:00Z"
  }
}
```

#### POST /api/auth/tokens

Generate personal access token.

**Request**:
```json
{
  "name": "CI/CD Pipeline",
  "expires_in": 7776000,
  "scopes": ["server:read", "task:write"]
}
```

**Response** (201):
```json
{
  "data": {
    "token": "pat_xxxxxxxxxxxxxxxxxx",
    "name": "CI/CD Pipeline",
    "scopes": ["server:read", "task:write"],
    "expires_at": "2026-09-17T00:00:00Z",
    "created_at": "2026-06-17T00:00:00Z"
  }
}
```

#### GET /api/auth/tokens

List personal access tokens (token values not shown).

**Response** (200):
```json
{
  "data": [
    {
      "id": "tok_abc123",
      "name": "CI/CD Pipeline",
      "scopes": ["server:read", "task:write"],
      "last_used_at": "2026-06-17T10:00:00Z",
      "expires_at": "2026-09-17T00:00:00Z",
      "created_at": "2026-06-17T00:00:00Z"
    }
  ]
}
```

#### DELETE /api/auth/tokens/:id

Revoke personal access token.

**Response** (204): No content

### Servers

#### GET /api/servers

List all servers.

**Query Parameters**:
- `status` (optional): Filter by status (`online`, `offline`)
- `page` (optional): Page number (default: 1)
- `per_page` (optional): Items per page (default: 20, max: 100)

**Response** (200):
```json
{
  "data": [
    {
      "id": "srv_abc123",
      "name": "web-server-01",
      "status": "online",
      "agent_version": "1.0.0",
      "ip_address": "192.168.1.100",
      "platform": "linux",
      "arch": "x86_64",
      "last_seen_at": "2026-06-17T12:00:00Z",
      "created_at": "2026-06-01T00:00:00Z"
    }
  ],
  "pagination": {
    "page": 1,
    "per_page": 20,
    "total": 5,
    "total_pages": 1
  }
}
```

#### GET /api/servers/:id

Get server details.

**Response** (200):
```json
{
  "data": {
    "id": "srv_abc123",
    "name": "web-server-01",
    "status": "online",
    "agent_version": "1.0.0",
    "ip_address": "192.168.1.100",
    "platform": "linux",
    "arch": "x86_64",
    "cpu_cores": 4,
    "total_memory": 8589934592,
    "total_disk": 107374182400,
    "uptime": 86400,
    "last_seen_at": "2026-06-17T12:00:00Z",
    "created_at": "2026-06-01T00:00:00Z",
    "current_stats": {
      "cpu_usage": 25.5,
      "memory_usage": 60.2,
      "disk_usage": 45.0,
      "network_rx": 1048576,
      "network_tx": 2097152,
      "load_1": 1.5,
      "load_5": 1.2,
      "load_15": 1.0
    }
  }
}
```

#### PUT /api/servers/:id

Update server settings.

**Request**:
```json
{
  "name": "web-server-01-updated",
  "note": "Production web server"
}
```

**Response** (200):
```json
{
  "data": {
    "id": "srv_abc123",
    "name": "web-server-01-updated",
    "note": "Production web server"
  }
}
```

#### DELETE /api/servers/:id

Delete server.

**Response** (204): No content

#### GET /api/servers/:id/metrics

Get server metrics history.

**Query Parameters**:
- `metric` (required): Metric name (`cpu`, `memory`, `disk`, `network`, `load`)
- `from` (required): Start timestamp (ISO 8601)
- `to` (required): End timestamp (ISO 8601)
- `resolution` (optional): Data resolution (`1m`, `5m`, `1h`) (default: auto)

**Response** (200):
```json
{
  "data": {
    "metric": "cpu",
    "resolution": "5m",
    "points": [
      {
        "timestamp": "2026-06-17T12:00:00Z",
        "value": 25.5
      },
      {
        "timestamp": "2026-06-17T12:05:00Z",
        "value": 30.2
      }
    ]
  }
}
```

### Enrollment Tokens

#### POST /api/enrollment-tokens

Generate enrollment token for agent registration.

**Request**:
```json
{
  "expires_in": 3600,
  "note": "For production servers"
}
```

**Response** (201):
```json
{
  "data": {
    "token": "enroll_xxxxxxxxxxxxxxxxxx",
    "expires_at": "2026-06-17T13:00:00Z",
    "created_at": "2026-06-17T12:00:00Z"
  }
}
```

#### GET /api/enrollment-tokens

List active enrollment tokens (not expired, not used).

**Response** (200):
```json
{
  "data": [
    {
      "id": "tok_abc123",
      "token": "enroll_xxxxxxxxxx",
      "note": "For production servers",
      "used": false,
      "expires_at": "2026-06-17T13:00:00Z",
      "created_at": "2026-06-17T12:00:00Z"
    }
  ]
}
```

### Services

#### GET /api/services

List service monitors.

**Response** (200):
```json
{
  "data": [
    {
      "id": "svc_abc123",
      "name": "Website Health",
      "type": "http",
      "enabled": true,
      "status": "up",
      "interval": 30,
      "timeout": 10,
      "last_check_at": "2026-06-17T12:00:00Z",
      "created_at": "2026-06-01T00:00:00Z"
    }
  ]
}
```

#### POST /api/services

Create service monitor.

**Request**:
```json
{
  "name": "Website Health",
  "type": "http",
  "enabled": true,
  "interval": 30,
  "timeout": 10,
  "config": {
    "url": "https://example.com",
    "method": "GET",
    "expected_status": 200,
    "verify_ssl": true
  },
  "notification_channels": ["slack"]
}
```

**Response** (201):
```json
{
  "data": {
    "id": "svc_abc123",
    "name": "Website Health",
    "type": "http",
    "enabled": true
  }
}
```

#### GET /api/services/:id

Get service monitor details.

#### PUT /api/services/:id

Update service monitor.

#### DELETE /api/services/:id

Delete service monitor.

**Response** (204): No content

#### GET /api/services/:id/history

Get service check history.

**Query Parameters**:
- `from` (required): Start timestamp
- `to` (required): End timestamp
- `page`, `per_page`: Pagination

**Response** (200):
```json
{
  "data": [
    {
      "timestamp": "2026-06-17T12:00:00Z",
      "status": "up",
      "response_time": 150,
      "status_code": 200,
      "error": null
    }
  ]
}
```

### Alert Rules

#### GET /api/alerts

List alert rules.

**Response** (200):
```json
{
  "data": [
    {
      "id": "alt_abc123",
      "name": "High CPU Usage",
      "enabled": true,
      "conditions": [
        {
          "type": "cpu",
          "operator": "gt",
          "value": 80,
          "duration": 300
        }
      ],
      "servers": ["srv_abc123", "srv_def456"],
      "notification_channels": ["slack", "email"],
      "cooldown": 600,
      "created_at": "2026-06-01T00:00:00Z"
    }
  ]
}
```

#### POST /api/alerts

Create alert rule.

**Request**:
```json
{
  "name": "High CPU Usage",
  "enabled": true,
  "conditions": [
    {
      "type": "cpu",
      "operator": "gt",
      "value": 80,
      "duration": 300
    }
  ],
  "servers": ["srv_abc123"],
  "notification_channels": ["slack"],
  "cooldown": 600
}
```

**Response** (201):
```json
{
  "data": {
    "id": "alt_abc123",
    "name": "High CPU Usage",
    "enabled": true
  }
}
```

#### PUT /api/alerts/:id

Update alert rule.

#### DELETE /api/alerts/:id

Delete alert rule.

**Response** (204): No content

#### GET /api/alerts/history

Get alert history.

**Query Parameters**:
- `from`, `to`: Time range
- `status`: Filter by status (`triggered`, `resolved`)
- `page`, `per_page`: Pagination

**Response** (200):
```json
{
  "data": [
    {
      "id": "alh_abc123",
      "alert_id": "alt_abc123",
      "alert_name": "High CPU Usage",
      "server_id": "srv_abc123",
      "server_name": "web-server-01",
      "status": "triggered",
      "value": 85.5,
      "triggered_at": "2026-06-17T12:00:00Z",
      "resolved_at": null,
      "notified": true
    }
  ]
}
```

### Tasks

#### GET /api/tasks

List tasks.

**Response** (200):
```json
{
  "data": [
    {
      "id": "tsk_abc123",
      "name": "Daily Backup",
      "type": "shell",
      "enabled": true,
      "schedule": "0 2 * * *",
      "servers": ["srv_abc123"],
      "last_run_at": "2026-06-17T02:00:00Z",
      "last_status": "success",
      "created_at": "2026-06-01T00:00:00Z"
    }
  ]
}
```

#### POST /api/tasks

Create task.

**Request**:
```json
{
  "name": "Daily Backup",
  "type": "shell",
  "enabled": true,
  "schedule": "0 2 * * *",
  "servers": ["srv_abc123"],
  "config": {
    "command": "/opt/backup.sh",
    "timeout": 3600,
    "shell": "/bin/bash"
  },
  "notification_on": ["failure"]
}
```

**Response** (201):
```json
{
  "data": {
    "id": "tsk_abc123",
    "name": "Daily Backup",
    "enabled": true
  }
}
```

#### PUT /api/tasks/:id

Update task.

#### DELETE /api/tasks/:id

Delete task.

**Response** (204): No content

#### POST /api/tasks/:id/trigger

Manually trigger task execution.

**Response** (202):
```json
{
  "data": {
    "execution_id": "exe_abc123",
    "status": "pending",
    "triggered_at": "2026-06-17T12:00:00Z"
  }
}
```

#### GET /api/tasks/:id/executions

Get task execution history.

**Query Parameters**:
- `page`, `per_page`: Pagination

**Response** (200):
```json
{
  "data": [
    {
      "id": "exe_abc123",
      "task_id": "tsk_abc123",
      "server_id": "srv_abc123",
      "status": "success",
      "started_at": "2026-06-17T02:00:00Z",
      "completed_at": "2026-06-17T02:05:00Z",
      "duration": 300,
      "output": "Backup completed successfully",
      "exit_code": 0
    }
  ]
}
```

### Notifications

#### GET /api/notifications/channels

List notification channels.

**Response** (200):
```json
{
  "data": [
    {
      "id": "ntf_abc123",
      "name": "slack",
      "type": "slack",
      "enabled": true,
      "created_at": "2026-06-01T00:00:00Z"
    }
  ]
}
```

#### POST /api/notifications/channels

Create notification channel.

**Request**:
```json
{
  "name": "slack",
  "type": "slack",
  "enabled": true,
  "config": {
    "webhook_url": "https://hooks.slack.com/services/YOUR/WEBHOOK/URL",
    "channel": "#alerts",
    "username": "XLStatus"
  }
}
```

**Response** (201):
```json
{
  "data": {
    "id": "ntf_abc123",
    "name": "slack",
    "type": "slack",
    "enabled": true
  }
}
```

#### POST /api/notifications/test

Test notification channel.

**Request**:
```json
{
  "channel_id": "ntf_abc123",
  "message": "Test notification"
}
```

**Response** (200):
```json
{
  "data": {
    "success": true,
    "sent_at": "2026-06-17T12:00:00Z"
  }
}
```

### Users (Admin Only)

#### GET /api/users

List users.

**Response** (200):
```json
{
  "data": [
    {
      "id": "usr_abc123",
      "username": "admin",
      "email": "admin@example.com",
      "role": "admin",
      "enabled": true,
      "created_at": "2026-06-01T00:00:00Z"
    }
  ]
}
```

#### POST /api/users

Create user.

**Request**:
```json
{
  "username": "john",
  "email": "john@example.com",
  "password": "secure-password",
  "role": "member"
}
```

**Response** (201):
```json
{
  "data": {
    "id": "usr_def456",
    "username": "john",
    "email": "john@example.com",
    "role": "member"
  }
}
```

#### PUT /api/users/:id

Update user.

#### DELETE /api/users/:id

Delete user.

**Response** (204): No content

## Examples

### Python

```python
import requests

# Authentication
token = "pat_xxxxxxxxxxxxxxxxxx"
headers = {"Authorization": f"Bearer {token}"}
base_url = "https://dashboard.example.com/api"

# List servers
response = requests.get(f"{base_url}/servers", headers=headers)
servers = response.json()["data"]

for server in servers:
    print(f"{server['name']}: {server['status']}")

# Trigger task
task_id = "tsk_abc123"
response = requests.post(
    f"{base_url}/tasks/{task_id}/trigger",
    headers=headers
)
print(f"Execution ID: {response.json()['data']['execution_id']}")
```

### JavaScript

```javascript
const token = 'pat_xxxxxxxxxxxxxxxxxx';
const baseURL = 'https://dashboard.example.com/api';
const headers = {
  'Authorization': `Bearer ${token}`,
  'Content-Type': 'application/json'
};

// List servers
const response = await fetch(`${baseURL}/servers`, { headers });
const { data: servers } = await response.json();

servers.forEach(server => {
  console.log(`${server.name}: ${server.status}`);
});

// Create service monitor
const service = {
  name: 'API Health',
  type: 'http',
  enabled: true,
  interval: 30,
  timeout: 10,
  config: {
    url: 'https://api.example.com/health',
    method: 'GET',
    expected_status: 200
  }
};

await fetch(`${baseURL}/services`, {
  method: 'POST',
  headers,
  body: JSON.stringify(service)
});
```

### cURL

```bash
TOKEN="pat_xxxxxxxxxxxxxxxxxx"
BASE_URL="https://dashboard.example.com/api"

# List servers
curl "${BASE_URL}/servers" \
  -H "Authorization: Bearer ${TOKEN}"

# Get server metrics
curl "${BASE_URL}/servers/srv_abc123/metrics?metric=cpu&from=2026-06-17T00:00:00Z&to=2026-06-17T12:00:00Z" \
  -H "Authorization: Bearer ${TOKEN}"

# Trigger task
curl -X POST "${BASE_URL}/tasks/tsk_abc123/trigger" \
  -H "Authorization: Bearer ${TOKEN}"
```

## WebSocket API

Real-time updates via WebSocket.

**Endpoint**: `wss://dashboard.example.com/ws`

**Authentication**: Send token in first message

```javascript
const ws = new WebSocket('wss://dashboard.example.com/ws');

ws.onopen = () => {
  ws.send(JSON.stringify({
    type: 'auth',
    token: 'pat_xxxxxxxxxxxxxxxxxx'
  }));

  ws.send(JSON.stringify({
    type: 'subscribe',
    topics: ['servers', 'alerts']
  }));
};

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  console.log(message);
};
```

**Message types**:
- `server_status`: Server online/offline
- `server_metrics`: New metrics data
- `alert_triggered`: Alert triggered
- `alert_resolved`: Alert resolved
- `task_started`: Task execution started
- `task_completed`: Task execution completed

## Next Steps

- [Installation Guide](./installation.md) - Set up XLStatus
- [Configuration Guide](./configuration.md) - Configure the system
- [Troubleshooting](./troubleshooting.md) - Solve common issues
