# 协议设计

## 端口与传输

| 端口 | 协议 | 用途 |
|------|------|------|
| 8080 | HTTP/1.1 | Axum REST、WebSocket、MCP、静态前端反代 |
| 50051 | HTTP/2 | Tonic Agent gRPC、reflection、health |
| 3000 | HTTP | Next.js dev server，仅开发 |

生产部署推荐用 Caddy 或 Nginx：

- HTTPS/WSS 入口反代到 `:8080`。
- gRPC HTTP/2 入口反代到 `:50051`。

## Agent gRPC

服务：

```proto
service AgentService {
  rpc Session(stream ClientMessage) returns (stream ServerMessage);
  rpc IoStream(stream IoFrame) returns (stream IoFrame);
}
```

`ClientMessage`：

- `Hello`
- `HostInfoUpdate`
- `StateSample`
- `BatchStateUpdate`
- `TaskResult`
- `Ping`
- `JwtRefreshRequest`

`ServerMessage`：

- `HelloAck`
- `TaskSpec`
- `TaskCancellation`
- `ConfigUpdate`
- `Pong`
- `JwtChallenge`
- `ForceDisconnect`

认证：

- Agent enrollment 后持有 Ed25519 私钥。
- Agent 使用短期 JWT 连接 gRPC，放入 metadata `authorization: bearer <jwt>`。
- Server 在 JWT 接近过期时下发 challenge。
- Agent 用私钥签名 nonce，Server 验证 public key 后签发新 JWT。

性能要求：

- Agent 状态上报走 bounded channel，避免无界内存。
- Server 对每条 stream 的发送必须串行化。
- 支持离线 `BatchStateUpdate`，但批量大小受上限控制。

验收：

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext localhost:50051 describe xlstatus.v1.AgentService
grpcurl -plaintext -d '{"service":"xlstatus.v1.AgentService"}' \
  localhost:50051 grpc.health.v1.Health/Check
```

## Dashboard WebSocket

端点：

- `/ws/servers`
- `/ws/terminal/{session_id}`
- `/ws/file/{session_id}`
- `/ws/transfers`

服务器状态消息：

```json
{
  "type": "server.patch",
  "seq": 1024,
  "server_id": "018f...",
  "online": true,
  "state": {
    "cpu": 0.32,
    "mem_used": 123456789,
    "net_in_speed": 2048,
    "net_out_speed": 1024,
    "load1": 0.42
  }
}
```

订阅规则：

- 初次连接发送 `server.snapshot`。
- 后续发送 `server.patch`。
- Agent 离线发送 `server.offline`。
- 服务端按当前用户权限过滤每条消息。

## MCP

端点：

- `POST /mcp`
- `GET /mcp/download/{token}`
- `POST /mcp/upload/{token}`

支持方法：

- `initialize`
- `notifications/initialized`
- `ping`
- `tools/list`
- `tools/call`

工具：

- `meta.whoami`
- `server.list`
- `server.get`
- `server.exec`
- `fs.list`
- `fs.read`
- `fs.write`
- `fs.delete`
- `fs.download_url`
- `fs.upload_url`

限制：

- 默认关闭。
- 只接受 PAT。
- request body 最大 8 MiB。
- 每 token 限流。
- 临时 URL 默认 300 秒，最大 600 秒，单次使用。
- 文件最大 100 MiB。

## 协议演进

- Proto package 使用 `xlstatus.v1`。
- 字段只追加不复用编号。
- 删除字段先 deprecate，至少跨一个 minor 版本。
- REST 使用 `/api/v1` 前缀。
- WebSocket 消息必须包含 `type` 和递增 `seq`。

