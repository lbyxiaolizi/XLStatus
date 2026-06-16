---
title: Cargo Workspace 目录结构
status: stable
audience: [human, agent]
---

# 04. Workspace 目录结构

完整目录树，按 M0 落地顺序排列。

## 根目录

```
XLStatus/
├── Cargo.toml                    # workspace 根
├── Cargo.lock                    # git tracked
├── .gitignore                    # /target, .env
├── CLAUDE.md                     # 已存在
├── README.md                     # 项目根 README
├── plan-mmx/                     # 本规划目录
├── proto/                        # protobuf 源文件
│   └── xlstatus/v1/
│       ├── common.proto
│       └── agent.proto
├── crates/                       # 所有 Rust crate
├── web/                          # Next.js 14 前端
├── docs/
│   ├── api.md
│   ├── ws-protocol.md
│   └── architecture.md
├── docker-compose.yml
├── Dockerfile.agent
├── Dockerfile.server
├── Caddyfile
├── .env.example
└── .env                          # git ignored
```

## crates/ 完整结构

```
crates/
├── shared/                        # 跨 crate 公共类型
│   ├── src/
│   │   ├── lib.rs
│   │   ├── ids.rs                 # newtype: AgentId, UserId, AlertRuleId, MonitorTaskId, NotifierId, EnrollmentTokenId
│   │   ├── sample.rs              # StateSample / HostInfo 域类型
│   │   ├── api_envelope.rs        # { data } / { data, page } / { error: { code, message, details } }
│   │   ├── error.rs               # ApiError + From impl
│   │   ├── time.rs                # ServerTime / ServerTimestamp 类型
│   │   ├── proto_convert.rs       # proto ↔ domain 转换（边界层）
│   │   └── crypto.rs              # base64url, sha256 助手
│   └── Cargo.toml
│
├── proto-gen/                     # tonic-build 编译期生成
│   ├── build.rs                   # 调 tonic_build::compile_protos
│   ├── src/
│   │   └── lib.rs                 # pub use tonic; pub mod xlstatus { tonic::include_proto!("xlstatus.v1") }
│   └── Cargo.toml
│
├── agent/                         # 探针端二进制
│   ├── src/
│   │   ├── main.rs                # clap: enroll / run / version
│   │   ├── config.rs              # toml + CLI
│   │   ├── enroll.rs              # 一次性注册
│   │   ├── keys.rs                # 私钥持久化（0600, zeroize on drop）
│   │   ├── auth.rs                # challenge 签名 / JWT 缓存 / metadata 注入
│   │   ├── grpc_client.rs         # tonic client + reconnect + JWT refresh
│   │   ├── reporter.rs            # 周期性 state 上报 + 攒批 + backpressure
│   │   ├── collector/
│   │   │   ├── mod.rs             # 统一 trait Sample
│   │   │   ├── cpu.rs             # sysinfo
│   │   │   ├── mem.rs             # sysinfo
│   │   │   ├── disk.rs            # sysinfo
│   │   │   ├── net.rs             # #[cfg(target_os)] 三平台分支
│   │   │   ├── load.rs            # sysinfo + sys/loadavg
│   │   │   ├── gpu.rs             # nvml/amd-smi 包装（可选）
│   │   │   ├── temperature.rs     # sysinfo sensors
│   │   │   ├── conn_count.rs      # /proc/net/tcp 等
│   │   │   └── host.rs            # 一次性：platform/arch/cpu列表/...
│   │   ├── task_runner.rs         # HTTP/TCP/Ping/SSL
│   │   └── backoff.rs             # 指数退避 + 抖动
│   └── Cargo.toml
│
├── server/                        # 中心节点二进制
│   ├── src/
│   │   ├── main.rs                # tokio main: 加载配置 / tracing / migrate / spawn axum :8080 + tonic :50051 / alert engine
│   │   ├── config.rs              # toml
│   │   ├── grpc_server/
│   │   │   ├── mod.rs             # tonic Server 装配 + TLS 配置
│   │   │   ├── service.rs         # AgentService impl（Session RPC）
│   │   │   ├── interceptor.rs     # JWT 校验 + 重放时间窗
│   │   │   ├── auth.rs            # 短期 JWT 签发
│   │   │   └── proto_to_domain.rs # 收到 proto → domain
│   │   ├── api/
│   │   │   ├── mod.rs             # Router 装配
│   │   │   ├── auth.rs            # /auth/login, /refresh, /logout, /me
│   │   │   ├── agent_enroll.rs    # POST /agent/enroll（首次注册）
│   │   │   ├── agents.rs          # CRUD + enrollment token + sessions
│   │   │   ├── samples.rs         # 时序查询
│   │   │   ├── monitor.rs         # 监控任务 CRUD + results
│   │   │   ├── alerts.rs          # 规则 CRUD + 事件
│   │   │   └── notifiers.rs       # 通知渠道 CRUD + test
│   │   ├── ws/
│   │   │   ├── mod.rs             # WS endpoint
│   │   │   ├── hub.rs             # 广播：Arc<DashMap<AgentId, Vec<ClientTx>>>
│   │   │   ├── client.rs          # 单连接状态机
│   │   │   └── messages.rs        # WS 消息 schema
│   │   ├── domain/
│   │   │   ├── mod.rs
│   │   │   ├── session.rs         # 在线 agent 缓存
│   │   │   ├── enrollment.rs      # token 签发/校验
│   │   │   ├── alert.rs           # 规则引擎
│   │   │   ├── notifier.rs        # trait + 实现
│   │   │   ├── sample_batch.rs    # 攒批 + sqlx Copy/insert
│   │   │   └── task_dispatch.rs   # 任务分配
│   │   ├── store/
│   │   │   ├── mod.rs             # trait
│   │   │   ├── sqlite.rs          # #[cfg(feature="storage-sqlite")]
│   │   │   ├── postgres.rs        # #[cfg(feature="storage-postgres")]
│   │   │   ├── timescaledb.rs     # PG 模式下的 hypertable/CAGG
│   │   │   └── models.rs          # 内部 row 类型
│   │   ├── auth/
│   │   │   ├── mod.rs
│   │   │   ├── password.rs        # argon2id
│   │   │   ├── jwt.rs             # access / refresh（web）
│   │   │   ├── agent_jwt.rs       # agent 专用 5min JWT
│   │   │   └── session_cookie.rs  # cookie 读写
│   │   ├── ratelimit.rs           # tower-governor 集成
│   │   └── telemetry.rs           # tracing-subscriber 初始化
│   ├── migrations/
│   │   ├── 20260101000001_init.sql
│   │   ├── 20260101000002_indexes.sql
│   │   ├── 20260101000003_pg_hypertable.sql
│   │   └── 20260101000004_seed_admin.sql
│   └── Cargo.toml
│
└── xtask/                         # 开发期辅助
    └── src/bin/
        ├── mock_agent.rs          # 50 个虚拟 agent 压测
        └── seed.rs                # 演示数据
```

## web/ 结构（Next.js 14 App Router）

```
web/
├── app/
│   ├── (public)/
│   │   └── login/page.tsx
│   ├── (authed)/
│   │   ├── layout.tsx
│   │   ├── dashboard/page.tsx
│   │   ├── dashboard/[id]/page.tsx
│   │   ├── services/page.tsx
│   │   ├── alerts/page.tsx
│   │   ├── tasks/page.tsx
│   │   └── settings/page.tsx
│   ├── api/                       # BFF（SSR 数据预取）
│   └── layout.tsx
├── components/
│   ├── ui/                        # shadcn/ui
│   ├── charts/
│   │   ├── MetricLineChart.tsx
│   │   ├── Sparkline.tsx
│   │   └── NetworkAreaChart.tsx
│   ├── server/
│   │   ├── ServerCard.tsx
│   │   ├── StatusBadge.tsx
│   │   └── HostInfoPanel.tsx
│   └── forms/
├── lib/
│   ├── api-client.ts              # fetch 包装
│   ├── ws.ts                      # useWebSocket hook
│   └── auth.ts
├── messages/
│   └── zh-CN.json
├── package.json
├── tsconfig.json
├── next.config.mjs
└── tailwind.config.ts
```

## M0 起步必建清单

按顺序执行（在 `11-roadmap.md` 的 M0 步详细列出）：

```bash
mkdir -p proto/xlstatus/v1
mkdir -p crates/{shared,proto-gen,agent,server,xtask}/src
mkdir -p crates/server/{migrations,src/{grpc_server,api,ws,domain,store,auth}}
mkdir -p crates/agent/src/collector
mkdir -p crates/xtask/src/bin
mkdir -p web
```