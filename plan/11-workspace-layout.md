# Workspace 目录结构

## 根目录

```text
XLStatus/
├── Cargo.toml
├── crates/
├── proto/
├── web/
├── plan/
├── docker-compose.yml
├── docker-compose.pg.yml
├── Dockerfile.server
├── Dockerfile.agent
├── Caddyfile
└── README.md
```

## Rust crates

```text
crates/
├── shared/
│   ├── src/
│   │   ├── api_envelope.rs
│   │   ├── authz.rs
│   │   ├── crypto.rs
│   │   ├── error.rs
│   │   ├── ids.rs
│   │   ├── metrics.rs
│   │   ├── time.rs
│   │   └── lib.rs
├── proto-gen/
│   ├── build.rs
│   └── src/lib.rs
├── tsdb/
│   └── src/
│       ├── compact.rs
│       ├── metric_store.rs
│       ├── query.rs
│       ├── writer.rs
│       └── lib.rs
├── server/
│   ├── migrations/
│   │   ├── sqlite/
│   │   └── postgres/
│   └── src/
│       ├── api/
│       ├── auth/
│       ├── config.rs
│       ├── db/
│       ├── domain/
│       ├── grpc/
│       ├── mcp/
│       ├── ws/
│       ├── workers/
│       └── main.rs
├── agent/
│   └── src/
│       ├── auth.rs
│       ├── collector/
│       ├── config.rs
│       ├── enroll.rs
│       ├── grpc_client.rs
│       ├── keys.rs
│       ├── task_runner/
│       ├── terminal/
│       ├── transfer/
│       └── main.rs
└── xtask/
    └── src/main.rs
```

## Server 模块职责

- `api`：REST router、extractor、OpenAPI、统一响应。
- `auth`：Web session、CSRF、PAT、Agent JWT、RBAC、scope。
- `db`：`DatabaseBackend`、连接池、repository trait、migration runner。
- `domain`：业务服务层，禁止直接依赖 `SqlitePool` 或 `PgPool`。
- `grpc`：AgentService、interceptor、session registry、IO stream。
- `mcp`：JSON-RPC、tool registry、临时传输 URL、限流。
- `ws`：Dashboard WebSocket hub、订阅、权限过滤。
- `workers`：调度、告警、通知、DDNS、TSDB flush、维护。

## Agent 模块职责

- `collector`：CPU、内存、磁盘、网络、负载、连接数、进程数、温度、GPU。
- `auth`/`keys`：Ed25519 key、enrollment、JWT refresh。
- `grpc_client`：Tonic client、重连、backpressure、流发送串行化。
- `task_runner`：HTTP/TCP/Ping/SSL、Shell、Exec、Config、Update。
- `terminal`：PTY 会话。
- `transfer`：文件读写、删除、大文件 IO stream。

## Proto 目录

```text
proto/xlstatus/v1/
├── common.proto
├── agent.proto
├── task.proto
├── io.proto
└── mcp.proto
```

## Web 目录

```text
web/
├── app/
│   ├── (public)/
│   └── (dashboard)/
├── components/
├── lib/
│   ├── api/
│   ├── realtime/
│   └── auth/
└── package.json
```

## M0 必建清单

- 根 `Cargo.toml` 改为 workspace。
- 创建 `crates/shared`、`crates/proto-gen`、`crates/server`、`crates/agent`、`crates/xtask`。
- 创建 `proto/xlstatus/v1/common.proto` 和 `agent.proto`。
- Server 同时启动 Axum `:8080` 和 Tonic `:50051`。
- Web 初始化 Next.js 项目并能显示 XLStatus。

