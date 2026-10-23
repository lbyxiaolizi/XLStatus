# 系统技术架构参考 (Technical Architecture Reference)

本文档提供 **XLStatus** 当前生产发布形态的完整技术架构说明，涵盖控制面（Control Plane）、数据面（Data Plane）、Web 运维工作台（Web Workbench）与 AI-Native MCP 引擎的拓扑关系、核心组件职责、五大关键业务数据流及安全信任边界。

---

## 1. 总体系统拓扑 (Control Plane, Data Plane, Web Workbench & AI MCP Topology)

XLStatus 采用**集中式控制面 + 反向长连接 Agent 数据面 + 原生 AI MCP 接口层**的三层解耦架构。被监控主机无需开放任何入站公网端口（除可选的 NAT 映射目标服务外），所有状态上报、远程命令、终端 PTY、文件传输、服务拨测与 NAT 反向隧道均复用 Agent 主动向 Server 发起的 gRPC 双向流。

```mermaid
flowchart TB
    subgraph Clients["客户端与消费层 (Clients & Consumers)"]
        Browser["运维浏览器 (Dashboard & Terminal)"]
        PublicUser["匿名访客 (Public Status & MJPEG)"]
        AIClient["AI 客户端 (Antigravity / Cursor / Claude)"]
        StdioBridge["Stdio 桥接器 (scripts/xlstatus-mcp-stdio.mjs)"]
    end

    subgraph WebLayer["Web 运维工作台 (web/ - Next.js 16 App Router)"]
        NextApp["Next.js UI (Dashboard / MCP Studio / Status Page)"]
    end

    subgraph ControlPlane["控制面 (xlstatus-server - Rust / Axum / Tonic)"]
        direction TB
        HttpApi["Axum HTTP & REST API (:8080)\nAuth / RBAC / CSRF / TOTP / WAF"]
        WsHub["WebSocket 实时网关\n/ws/servers & /api/v1/ws & /terminal"]
        McpGateway["MCP 2024-11-05 网关\nPOST /mcp & /api/v1/mcp/studio"]
        McpExec["McpExecutor 统一执行引擎\n19 Tools · 6 Playbooks · 6 Resources · 4 Prompts"]
        GrpcServer["Tonic gRPC Server (:50051)\nSession (ReportState) & IoStream (OpenIoStream)"]
        Registries["进程内会话与流注册表\nSessionRegistry · IoRegistry · TaskResponseRegistry"]
        BroadcastHub["BroadcastHub 实时事件总线\n最新快照缓存 + 增量分发"]
        BgEngines["后台自治引擎\nServiceMonitor · AlertEngine · TaskScheduler · DdnsManager · NatTunnelManager"]
    end

    subgraph Storage["持久化与时序存储层 (Storage Layer)"]
        RelDB[("SQLite (WAL) / PostgreSQL\n元数据 · 用户 · 告警 · 任务 · 审计")]
        Tsdb[("xlstatus-tsdb (MetricStore)\n时序采样 · 1h/24h/7d/30d 聚合")]
    end

    subgraph DataPlane["数据面 (xlstatus-agent - 受控节点守护进程)"]
        AgentCore["Agent 核心会话循环\nEd25519 密钥 · JWT 自动续期 · 指数退避重连"]
        Collector["HostCollector (sysinfo)\nCPU / RAM / Swap / Disk / Net / Load / GeoIP"]
        Executors["Task & IO 执行器\nShell (PGID 隔离) · HTTP/TCP/ICMP 拨测 · Files · PTY Terminal"]
        NatWorker["NAT 反向隧道 Worker\nDNS 锁定解析 · SSRF 防护 · 本地目标转发"]
    end

    Browser -->|"HTTPS / Cookie + CSRF"| NextApp
    Browser -->|"REST / WebSocket"| HttpApi
    Browser -->|"WS 实时遥测与终端"| WsHub
    PublicUser -->|"GET /api/v1/public/*"| HttpApi
    AIClient -->|"Direct HTTP JSON-RPC (Bearer xlp_)"| McpGateway
    AIClient -->|"stdio JSON-RPC"| StdioBridge
    StdioBridge -->|"POST /mcp (Bearer xlp_)"| McpGateway
    NextApp -->|"Interactive Studio"| McpGateway

    McpGateway --> McpExec
    McpExec --> RelDB
    McpExec --> Tsdb
    McpExec --> Registries

    HttpApi --> RelDB
    HttpApi --> Tsdb
    HttpApi --> Registries
    WsHub --> BroadcastHub
    WsHub --> Registries

    BgEngines --> RelDB
    BgEngines --> Registries
    GrpcServer --> Registries
    GrpcServer --> Tsdb
    GrpcServer --> BroadcastHub
    GrpcServer --> RelDB

    AgentCore <-->|"gRPC Bidirectional Stream\nSession & IoStream (Bearer JWT)"| GrpcServer
    AgentCore --> Collector
    AgentCore --> Executors
    AgentCore --> NatWorker
```

---

## 2. 核心组件拆解 (Component Breakdown)

| 组件 / Crate | 路径 | 核心职责与架构定位 |
|---|---|---|
| **`xlstatus-server`** | `crates/server` | **控制面核心二进制**。基于 Axum 提供 REST API、WebSocket 实时流、公开状态页（含 MJPEG 流）与 MCP `2024-11-05` 端点；基于 Tonic 提供 Agent gRPC 服务；内置 SQLite/PostgreSQL 自动迁移、WAF 封禁、RBAC/PAT 鉴权、TOTP 二次验证，以及服务拨测、告警引擎、定时任务、DDNS、NAT 隧道等后台自治循环。 |
| **`xlstatus-agent`** | `crates/agent` | **数据面节点守护进程**。负责使用 Enrollment Token (`xle_`) 完成首次注册并生成本地 `0600` 权限的 Ed25519 密钥对；通过 Challenge-Response 换取短期 JWT 建立 gRPC `Session` 与 `IoStream` 双向流；周期性采集主机硬件与实时指标，并按需执行远程 Shell（独立进程组隔离）、服务探测、文件读写、PTY 终端会话与 NAT 端口转发。 |
| **`xlstatus-tsdb`** | `crates/tsdb` | **时序指标存储引擎 (`MetricStore`)**。为高频上报的 `HostState` 指标提供统一的写入与多时间窗口查询门面（`15m` / `1h` / `6h` / `24h` / `7d` / `30d`），支持统计聚合（`min` / `max` / `avg` / `latest`）与降采样输出。 |
| **`xlstatus-shared`** | `crates/shared` | **跨端领域模型与安全契约库**。定义强类型 ID（`AgentId`、`UserId`）、RBAC 角色与 Scope 常量（`authz.rs`）、任务选择器（`ServerSelector`）、终端协议帧、DDNS 模型以及 NAT 控制帧（`NatTunnelControlMessage`）。 |
| **`xlstatus-proto-gen`** | `crates/proto-gen` | **Protobuf 与 gRPC 编译层**。通过 `tonic-build` 编译 `proto/xlstatus/v1/{agent,common,nat}.proto`，向 `server` 与 `agent` 导出强类型的 Rust 消息结构与 `AgentService` 服务端/客户端桩代码。 |
| **`web`** | `web/` | **Next.js 16 现代化运维工作台**。采用 App Router、React 19 与 Tailwind CSS v4 构建，提供实时监控大盘、世界节点地图、在线 Web 终端、文件传输器、服务 SLA 大盘、告警与通知编排、NAT/DDNS 配置、MCP AI Ops Studio 以及匿名公开状态页。 |

---

## 3. 五大核心业务数据流 (Key Data Flows)

### 3.1 Agent 注册认证与 gRPC 双向流生命周期
**(Agent Enrollment & gRPC Bidirectional Stream: `ReportState` / `Session` & `OpenIoStream` / `IoStream`)**

Agent 接入采用**一次性注册令牌 + Ed25519 非对称密钥 Challenge-Response + 短期 JWT** 的零明文口令传输机制，确保即使 Enrollment Token 过期，已注册节点仍可通过私钥安全续期连接，且管理员撤销后立即阻断。

```mermaid
sequenceDiagram
    autonumber
    actor Admin as 管理员 (Dashboard)
    participant Server as xlstatus-server (HTTP :8080 / gRPC :50051)
    participant DB as SQLite / PostgreSQL
    participant Agent as xlstatus-agent (受控主机)

    Admin->>Server: POST /api/v1/agents/tokens (需 Admin Session + TOTP)
    Server->>DB: 持久化一次性/短效 Enrollment Token (xle_ + 64位hex)
    Server-->>Admin: 返回 xle_... 及一键安装命令

    Note over Agent: 首次启动: 生成 Ed25519 密钥对并保存至 agent.toml (0600)
    Agent->>Server: POST /api/v1/agents/enroll {enrollment_token, name, public_key}
    Server->>DB: 校验 xle_ 令牌 -> 创建 Agent 记录并绑定 Ed25519 公钥
    Server-->>Agent: 返回 canonical agent_id (UUID)

    Note over Agent,Server: 每次建立/重连 gRPC 会话前的 Challenge-Response 认证
    Agent->>Server: POST /api/v1/agents/jwt/challenge\n{agent_id, request_nonce, request_timestamp, Ed25519 签名}
    Server->>DB: 校验 Agent 未撤销 + 验证请求签名与 ±120s 时间窗 + 防重放
    Server-->>Agent: 返回服务端 challenge_nonce

    Agent->>Server: POST /api/v1/agents/jwt\n{agent_id, nonce: challenge_nonce, Ed25519 签名}
    Server->>DB: 验证 challenge 签名并原子消费 nonce
    Server-->>Agent: 签发短期 Agent JWT

    Note over Agent,Server: 建立两条复用的 gRPC 双向流 (携带 Authorization: Bearer <JWT>)
    Agent->>Server: gRPC AgentService/Session (ReportState 控制与状态流)
    Server->>Server: SessionRegistry.register(agent_id, tx)
    loop 周期性上报与控制下发
        Agent-->>Server: AgentMessage {Heartbeat | HostState | HostInfoUpdate | GeoIpReport | TaskResult}
        Server-->>Agent: ServerMessage {ServerTask | ConfigUpdate | ForceUpdate | ForceDisconnect}
    end

    Agent->>Server: gRPC AgentService/IoStream (OpenIoStream 数据流)
    Server->>Server: IoRegistry.register_agent(agent_id, io_tx)
    loop 终端 PTY / 大文件传输 / NAT 隧道帧复用
        Agent->>Server: IoFrame (stream_id, sequence, agent_id, IoData / IoClose / IoError)
        Server-->>Agent: IoFrame (stream_id, sequence, agent_id, IoData / IoClose / IoError)
    end
```

### 3.2 实时遥测采集与分发流水线 (Real-time Telemetry Pipeline)

Agent 采集的主机指标在到达控制面后，同时写入关系库最新状态快照、时序数据库 `xlstatus-tsdb` 与内存级 `BroadcastHub`，从而实现 Web 大盘“首屏零闪烁秒开 + 增量毫秒级推送”。

```mermaid
flowchart LR
    subgraph AgentNode["xlstatus-agent"]
        Sysinfo["collector::mod\n(CPU/Mem/Disk/Net/Load)"]
    end

    subgraph ServerPipeline["xlstatus-server 遥测管线"]
        GrpcIn["gRPC Session 流\n校验 JSON <= 256KiB & 字段清洗"]
        AgentRepo["AgentRepository\n更新 last_state_json & last_seen_at"]
        MetricStore["xlstatus-tsdb (MetricStore)\n写入时序样本点"]
        BHub["BroadcastHub\n更新 latest_snapshot + 广播 RealtimeEvent"]
        WsRoutes["WebSocket 路由\n/ws/servers & /api/v1/ws"]
    end

    subgraph Consumers["消费端"]
        DashboardUI["Web Dashboard\n先读 Snapshot 再合并增量 Event"]
        McpTools["MCP 工具与资源\ncluster.overview / server.metrics"]
    end

    Sysinfo -->|"HostState"| GrpcIn
    GrpcIn --> AgentRepo
    GrpcIn --> MetricStore
    GrpcIn --> BHub
    BHub -->|"1. 连接建立即推 Snapshot\n2. 增量推送 Event + 30s Ping"| WsRoutes
    WsRoutes -->|"Origin 校验 + RBAC 租户过滤"| DashboardUI
    AgentRepo --> McpTools
    MetricStore --> McpTools
```

- **双 WebSocket 端点**：
  - `/ws/servers`（`crates/server/src/realtime/ws.rs`）：连接建立后立即下发当前用户可见节点的 `{"type":"snapshot","events":[...]}`，随后实时转发 `{"type":"event","event":{...}}` 与 30 秒一次的 `{"type":"ping"}`，并动态校验节点可见性与撤销状态。
  - `/api/v1/ws`（`crates/server/src/api/v1/ws.rs`）：支持客户端按需发送 `{"type":"subscribe","channels":["servers","alerts"]}` 订阅多频道实时事件。

### 3.3 服务拨测与告警自愈流水线 (Service Monitoring & Alert Engine Pipeline)

XLStatus 支持**控制面本地探测 (`local`)** 与 **分布式 Agent 节点探测 (`agent`)** 两种拨测模式，并将拨测结果与主机遥测统一汇入告警评估引擎，触发安全转义的 Webhook 通知与自动化自愈任务。

```mermaid
flowchart TD
    subgraph Probes["服务拨测调度 (ServiceMonitor)"]
        LocalProbe["控制面本地探测器\nHTTP / TCP / ICMP (DNS Pinning 防 SSRF)"]
        AgentProbe["Agent 远端探测器\n下发 ServerTask (HttpGet / TcpPing / IcmpPing)"]
    end

    SvcDB[("service_results 表\n状态 · 延迟 · HTTP 状态码 · TLS 证书到期时间")]

    subgraph Alerting["告警评估引擎 (AlertEngine)"]
        RuleEval["多条件规则评估器\nServiceDown · ServiceLatency · CertificateExpiry\nServerOffline · ServerResource · ServerExpiry · ServerTrafficQuota"]
        StateMachine["触发状态机\nTriggerMode (Once / Always) · 恢复检测"]
    end

    subgraph Actions["通知与自愈动作 (Notifications & Self-Healing)"]
        WebhookSender["NotificationSender\nRFC 8259 JSON 转义 / URL 编码模板\nDNS Pinning 防 SSRF 校验"]
        SelfHeal["spawn_triggered_tasks\n自动触发 failure_task_ids / recovery_task_ids"]
        AlertEventDB[("alert_events 表\n记录 firing / resolved 事件")]
    end

    LocalProbe --> SvcDB
    AgentProbe --> SvcDB
    SvcDB --> RuleEval
    RuleEval --> StateMachine
    StateMachine -->|"Firing / Resolved"| AlertEventDB
    StateMachine -->|"通知组分发"| WebhookSender
    StateMachine -->|"故障自愈 / 恢复回调"| SelfHeal
```

- **RFC 8259 安全模板渲染**：`NotificationSender` 在渲染自定义 JSON Webhook 模板（如 `{{title}}`、`{{message}}`、`{{severity}}`）时，严格按 RFC 8259 对双引号、反斜杠与控制字符进行转义；对 `application/x-www-form-urlencoded` 模板进行百分号编码，杜绝告警载荷注入破坏 JSON 结构。
- **出站 SSRF 防护**：无论是本地 HTTP 拨测还是 Webhook 通知发送，均通过 `crates/server/src/security.rs` 执行 DNS 解析结果校验与 IP 绑定（DNS Pinning），默认拦截指向回环、链路本地、云元数据（`169.254.169.254`）与未授权私网地址的请求。

### 3.4 交互式终端与 NAT 反向隧道多路复用 (`IoRegistry` Multiplexing)

终端会话与 NAT 内网穿透共享 `AgentService/IoStream` 通道，由控制面的 `IoRegistry` 按 `stream_id` 进行多路复用，并强制执行每个流的 `agent_id` 归属权校验。

```mermaid
sequenceDiagram
    autonumber
    participant Client as 外部访问者 / Web 终端
    participant Server as xlstatus-server (NatTunnelManager / Terminal API)
    participant IoReg as IoRegistry (多路复用器)
    participant Agent as xlstatus-agent (IoStream & NatWorker / PTY)
    participant Target as 目标内网服务 / 本地 PTY Shell

    alt 场景 A: 交互式 Web 终端 (Terminal PTY)
        Client->>Server: WS /api/v1/servers/:id/terminal (Cookie + RBAC + Origin 校验)
        Server->>IoReg: subscribe_stream(stream_id, expected_agent_id)
        Server->>Agent: Session 下发 TerminalTask(OPEN, session_id, cols, rows)
        Agent->>Target: 启动 PTY 子进程 (sh/bash)
        Client->>Agent: 经 IoRegistry 转发终端输入 IoFrame (stream_id, agent_id, IoData)
        Agent-->>Client: 经 IoRegistry 推送终端输出 IoFrame (stream_id, agent_id, IoData)
    else 场景 B: NAT 反向隧道 (Reverse TCP Tunnel)
        Client->>Server: 连接公网映射端口 (public_port, 校验 allowed_sources CIDR)
        Server->>IoReg: 注册隧道 stream_id ("nat:<mapping_id>:<conn_uuid>") 并绑定 expected_agent_id
        Server->>Agent: 发送首帧 NatTunnelControlMessage::Open {local_host, local_port}
        Agent->>Agent: resolve_agent_nat_target (DNS Pinning + 私网策略校验)
        Agent->>Target: 建立本地 TCP 连接 (10s 超时, 最多 64 并发隧道)
        Client->>Target: 双向透传 TCP 字节流 <-> IoFrame (单帧上限 64KiB)
        Target-->>Client: 双向透传 IoFrame <-> TCP 字节流 (单帧上限 64KiB)
    end
```

- **单流归属隔离 (`expected_agent_id`)**：`IoRegistry` 在注册任意终端或 NAT `stream_id` 时都会记录目标 `agent_id`。当 Agent 上报 `IoFrame` 时，若帧来源的认证 `agent_id` 与该 `stream_id` 绑定的 `expected_agent_id` 不一致，控制面直接丢弃并拒绝跨节点串流劫持。
- **Agent 端 DNS 锁定拨号**：Agent 在处理 `NatTunnelControlMessage::Open` 时，使用 `resolve_agent_nat_target` 完成主机名解析与目标地址策略校验，防止恶意映射通过 DNS 重绑定攻击 Agent 所在宿主机。

### 3.5 AI-Native MCP 统一执行引擎 (`McpExecutor`)

XLStatus 将 AI 运维能力深度内建于控制面核心，`McpExecutor` 统一编排 **Tools（工具）**、**Resources（`xlstatus://` 资源）** 与 **Prompts（SRE 提示词模板）**，并与 Web/REST 共享同一套 RBAC 与 PAT 节点隔离模型。

```mermaid
flowchart LR
    subgraph Transports["三种接入传输层"]
        T1["Streamable HTTP\nPOST /mcp (Bearer xlp_)"]
        T2["Stdio Bridge\nscripts/xlstatus-mcp-stdio.mjs"]
        T3["Web AI Ops Studio\nGET /catalog & POST /studio"]
    end

    subgraph AuthGuard["认证与权限门禁 (api/v1/mcp.rs)"]
        ScopeCheck["Scope 最小权限校验\nserver:read / server:exec / service:read\nalert:read / task:read / nat:read / transfer:read"]
        TotpCheck["Cookie Studio 敏感操作门禁\nserver.exec / batch_exec 强制校验 x-totp-code"]
    end

    subgraph Engine["McpExecutor (crates/server/src/mcp/executor.rs)"]
        Isolation["租户与节点白名单过滤\nagent_visible_to_auth (owner_user_id + PAT server_ids)"]
        Tools["19 项 MCP Tools\n含 6 个 server.inspect 只读诊断剧本"]
        Resources["5 个静态资源 + 1 个动态模板\nxlstatus://..."]
        Prompts["4 个 AI SRE 动态提示词\nsre.* (自动注入实时遥测)"]
    end

    T2 --> T1
    T1 --> ScopeCheck
    T3 --> ScopeCheck
    T3 --> TotpCheck
    ScopeCheck --> Isolation
    TotpCheck --> Isolation
    Isolation --> Tools
    Isolation --> Resources
    Isolation --> Prompts
```

---

## 4. 安全与信任边界汇总 (Security & Trust Boundaries)

1. **浏览器与 Web 控制台边界**：
   - 登录态采用 HttpOnly Session Cookie，所有状态变更请求（`POST` / `PUT` / `PATCH` / `DELETE`）强制校验 `x-csrf-token` 头部。
   - CORS 严禁配置 `*`，WebSocket 升级请求强制验证 `Origin` 是否在 `server.cors_allowed_origins` 白名单内。
   - 当管理员启用 TOTP 后，签发 Enrollment Token、撤销 Agent、绑定/解绑 OAuth 以及在 Web MCP Studio 中调用 `server.exec` / `server.batch_exec` 均强制要求提供有效的 `x-totp-code`。
2. **Agent 与控制面边界**：
   - Agent 配置文件（包含 Ed25519 私钥）权限强制锁定为 `0600`。
   - Agent gRPC 端口在 Docker Compose 与 `systemd` 安装脚本中默认绑定 `127.0.0.1:50051`，防止未加密 gRPC 意外暴露公网。
   - 远端 Shell 执行在 Unix 上为每个任务创建独立进程组（PGID），任务超时、取消或流断开时由 `Drop` 守卫向整个进程组发送终止信号，防止后台子进程残留。
3. **PAT 与 AI MCP 边界**：
   - PAT 令牌（`xlp_...`）仅存储 SHA-256 哈希，支持细粒度 Scope 与 `server_ids` 节点白名单。
   - 为防范提示词注入（Prompt Injection）导致远端文件被篡改，MCP 层彻底移除了文件写入、删除与上传 URL 生成工具（`fs.write`、`fs.delete`、`fs.upload_url`），仅开放只读文件系统能力（`fs.list`、`fs.read`、`fs.download_url`）。
4. **协议级输入与内存保护边界**：
   - HTTP 原始路径上限 `4096` 字节，Query String 上限 `16KiB`；MCP JSON-RPC 请求体上限 `1MiB`，单次批处理最多 `16` 项；Stdio 桥接器单行上限 `1MiB` 并自动脱敏 `xlp_` / `xle_` 令牌。

---

## 5. 发布与部署拓扑 (Release & Deployment Topologies)

### 5.1 Docker Compose 拓扑
仓库提供三套开箱即用的 Compose 编排文件：
- `docker-compose.yml`：标准 SQLite（挂载 `./data`）+ Server + Web 生产编排。
- `docker-compose.pg.yml`：PostgreSQL + Server + Web 生产高可用编排。
- `docker-compose.simple.yml`：快速本地验证编排。

> [!IMPORTANT]
> 远端部署修改 `XLSTATUS_PUBLIC_API_URL` 或 `XLSTATUS_CORS_ALLOWED_ORIGINS` 后，必须重新执行 `docker compose build web` 以更新前端 bundle 中的 API 地址。

### 5.2 systemd 原生守护进程拓扑
- `deploy/install.sh`：自动检测 Linux 架构（`x86_64` / `arm64` / `i386`），安装 `xlstatus-server` 与 `/etc/systemd/system/xlstatus-server.service`。
- `deploy/install-agent.sh`：支持从 GitHub Release（默认 fallback `v0.3`）下载对应架构二进制，完成节点 enrollment 并注册 `xlstatus-agent.service`。

### 5.3 GitHub Release 多平台构建矩阵
推送 `v*` 标签后，`.github/workflows/` 自动交叉编译并发布 10 种平台架构的 `server` 与 `agent` 二进制资产：
- **Linux**：`x86_64`、`arm64`、`i386`
- **Windows**：`x86_64.exe`、`arm64.exe`、`i386.exe`
- **macOS (Darwin)**：`x86_64`、`arm64`
- **FreeBSD**：`x86_64`、`i386`
- **安装器脚本**：`install-server.sh`、`install-agent.sh`
