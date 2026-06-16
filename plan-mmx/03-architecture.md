---
title: 架构总览
status: stable
audience: [human, agent]
---

# 03. 架构总览

## 系统图

```
┌─────────────────┐  gRPC :50051 (HTTP/2)     ┌──────────────────────────┐
│  xlstatus-agent │ ─────────────────────────▶│  xlstatus-server         │
│  (Rust CLI)     │ ◀─── stream 双向 ──────── │                          │
│  on each host   │   proto binary + TLS       │  ┌─────────────────────┐ │
└─────────────────┘                           │  │ tonic gRPC :50051   │ │
                                              │  │  ├ agent.proto 流   │ │
┌─────────────────┐  HTTPS + Cookie          │  │  └ interceptor(验签)│ │
│  Web Dashboard  │ ─────────────────────────▶│  │                     │ │
│  (Next.js 14)   │ ◀─── WS :8080 live ───── │  │ axum 0.7 :8080      │ │
└─────────────────┘                           │  │  ├ /api/v1/* REST   │ │
                                              │  │  └ /api/v1/ws/servers│ │
                                              │  │                     │ │
                                              │  │ Domain              │ │
                                              │  │  ├ session 在线态   │ │
                                              │  │  ├ enrollment      │ │
                                              │  │  ├ alert 规则引擎  │ │
                                              │  │  └ notifier 调度   │ │
                                              │  │                     │ │
                                              │  │ Store (sqlx trait) │ │
                                              │  │  ├ sqlite.rs       │ │
                                              │  │  └ postgres.rs+TS  │ │
                                              │  └─────────────────────┘ │
                                              └──────────┬───────────────┘
                                                         ▼
                                            SQLite | PostgreSQL+TS
```

## 通信矩阵

| 通信方 | 协议 | 端口 | 序列化 | 鉴权 |
|--------|------|------|--------|------|
| Agent → Server | gRPC (HTTP/2) | 50051 | proto binary | Ed25519 challenge + 短期 JWT（metadata `authorization`） |
| Dashboard → Server | HTTPS REST | 8080 | JSON | Cookie (access JWT) + refresh rotation |
| Server → Dashboard | WebSocket | 8080 | JSON | Cookie 同上 |
| Server → Agent | gRPC stream | 50051 | proto binary | JWT |

## 端口分配

| 端口 | 协议 | 服务 |
|------|------|------|
| 8080 | HTTP/1.1 | axum REST + WebSocket |
| 50051 | HTTP/2 | tonic gRPC（含 reflection） |
| 443 | HTTPS | Caddy 反代（生产） |

## 为什么分两个端口

- gRPC 用 HTTP/2，axum REST 用 HTTP/1.1，反代时不能简单合并
- 分开后调试简单：`grpcurl :50051` + `curl :8080`
- TLS 由 Caddy 在 :443 终结，:50051 也可走 Caddy stream 代理（生产配置）

## 部署拓扑（生产）

```
                    ┌─────────────┐
   Internet ──────▶ │   Caddy     │ :443
                    │  (ACME TLS) │
                    └──┬──────┬───┘
                       │      │ stream
                       ▼      ▼
                :8080 HTTP  :50051 gRPC
              ┌─────────────────────┐
              │  xlstatus-server    │
              │  (Rust binary)      │
              └──────────┬──────────┘
                         ▼
                ┌─────────────────┐
                │ SQLite | PG+TS  │
                └─────────────────┘

  [agent 主机]                    [浏览器]
  xlstatus-agent  ──gRPC──▶       Next.js ──REST+WS──▶
  :随机端口（出）                   :3000
```

## 进程清单

| 进程 | 部署位置 | 二进制 | 端口 |
|------|----------|--------|------|
| `xlstatus-server` | 中心节点 1 台 | `target/release/xlstatus-server` | :8080, :50051 |
| `xlstatus-agent` | 每台被监控机器 | `target/release/xlstatus-agent` | 出站 |
| `xlstatus-web`（Next.js） | 同 server 或独立 | `next start` | :3000（Caddy 反代） |
| `caddy` | 边缘 | `caddy run` | :443, :80 |
| PostgreSQL + TimescaleDB（可选） | 数据节点 | docker 或裸跑 | :5432 |