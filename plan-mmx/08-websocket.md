---
title: Dashboard WebSocket 协议
status: stable
audience: [human, agent]
related_milestones: [M3]
---

# 08. Dashboard WebSocket 协议

Server → Dashboard 单向 push，客户端通过 `subscribe`/`unsubscribe` 控制订阅。gRPC 是给 Agent 用的（见 `06-grpc-protocol.md`），本协议与 gRPC 完全独立。

## 端点

```
wss://server/api/v1/ws/servers
```

鉴权：浏览器自动带 cookie（access_token），服务端在 upgrade 阶段校验。

## 消息 schema

**所有消息均为 JSON**，无 proto。

### Client → Server

```jsonc
// 订阅指定 agent（空数组 = 全部）
{ "type": "subscribe",   "agent_ids": ["uuid", ...] }

// 取消订阅
{ "type": "unsubscribe", "agent_ids": ["uuid", ...] }

// 心跳
{ "type": "ping" }
```

### Server → Client

```jsonc
// 周期性 state 推送
{ "type": "state", "agent_id": "uuid", "state": { ... } }

// agent 上线
{ "type": "online", "agent_id": "uuid", "ts_ms": 1737012345000 }

// agent 离线（30s 内无 ping/pong 标记为离线）
{ "type": "offline", "agent_id": "uuid", "ts_ms": 1737012345000 }

// 告警事件
{ "type": "alert_event", "event": {
    "id": "uuid",
    "rule_id": "uuid",
    "agent_id": "uuid",
    "status": "firing" | "resolved",
    "triggered_at": "RFC3339",
    "resolved_at": "RFC3339" | null,
    "message": "..."
}}

// 心跳响应
{ "type": "pong" }
```

## state 字段

与 `common.proto::StateSample` 一致（JSON 表达）：

```json
{
  "ts_ms": 1737012345000,
  "cpu": 0.42,
  "mem_used": 8589934592,
  "swap_used": 0,
  "disk_used": 53687091200,
  "net_in_speed": 12345,
  "net_out_speed": 67890,
  "net_in_transfer": 1099511627776,
  "net_out_transfer": 549755813888,
  "uptime": 86400,
  "load1": 0.5, "load5": 0.4, "load15": 0.3,
  "tcp_conn_count": 42,
  "udp_conn_count": 5,
  "process_count": 215,
  "temperatures": [{"name":"cpu","temperature":45.5}],
  "gpu": [0.3]
}
```

## 心跳与离线检测

- 客户端 30s 发一次 `ping`，服务端回 `pong`
- 服务端在 30s 内没收到该 client 任何消息 → 关闭连接
- **agent 离线判断**：服务端 gRPC stream 30s 没收到 agent 消息 → 推 `offline` 消息给订阅的 dashboard

## 断线重连

前端 `useWebSocket` hook 行为：
- 指数退避：1s, 2s, 4s, 8s, 16s, 30s（max）
- 重连后自动重发 `subscribe`（从本地缓存读上次订阅列表）

## 实现位置

- 服务端：`crates/server/src/ws/{hub,client,messages}.rs`
- 客户端：`web/lib/ws.ts` + `web/components/dashboard/LiveServersProvider.tsx`

## 性能预算

- 100 agent × 1 push / 10s = 10 msg/s 进 hub
- 假设 10 个 dashboard 客户端 × 全部订阅 = 100 msg/s 出 hub
- 单 mpsc channel 即可，无需 broadcast 库

## 验证

```bash
# 用 websocat 测试（需安装）
websocat -H "Cookie: access_token=..." wss://localhost:8443/api/v1/ws/servers

# 发送订阅
> {"type":"subscribe","agent_ids":[]}
< {"type":"state","agent_id":"...","state":{...}}
< {"type":"online","agent_id":"...","ts_ms":...}
```