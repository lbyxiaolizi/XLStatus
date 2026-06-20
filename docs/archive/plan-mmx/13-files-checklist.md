---
title: 关键文件清单
status: stable
audience: [human, agent]
---

# 13. 关键文件清单

按改动频率分组。交接时按此清单定位改动点。

## 高频（业务实现推进时持续改）

| 文件 | 内容 | 何时改 |
|------|------|--------|
| `proto/xlstatus/v1/common.proto` | HostInfo / StateSample / TaskSpec 共享类型 | 加新字段时 |
| `proto/xlstatus/v1/agent.proto` | AgentService 定义 | 加 RPC 或消息体时 |
| `crates/proto-gen/build.rs` | 编译期生成 | 几乎不改 |
| `crates/server/src/grpc_server/service.rs` | Session RPC 实现 | 加新消息处理时 |
| `crates/server/src/grpc_server/interceptor.rs` | JWT 校验 | 鉴权变更时 |
| `crates/server/src/grpc_server/proto_to_domain.rs` | proto→domain 转换 | proto 字段变更时 |
| `crates/server/src/api/auth.rs` | /auth/* 4 端点 | 加新登录方式时 |
| `crates/server/src/api/agent_enroll.rs` | /agent/enroll | 改注册流程时 |
| `crates/server/src/api/agents.rs` | /agents/* CRUD | 加 agent 操作时 |
| `crates/server/src/api/samples.rs` | 时序查询 | 加新 metric 时 |
| `crates/server/src/api/monitor.rs` | 监控任务 CRUD | 加任务类型时 |
| `crates/server/src/api/alerts.rs` | 告警规则 | 加规则类型时 |
| `crates/server/src/api/notifiers.rs` | 通知渠道 | 加新 notifier 时 |
| `crates/server/src/domain/alert.rs` | 规则引擎 | 加新评估逻辑时 |
| `crates/server/src/domain/notifier.rs` | notifier trait + 实现 | 加新通知渠道时 |
| `crates/server/src/domain/task_dispatch.rs` | 任务分配 | 改分配策略时 |
| `crates/server/src/domain/sample_batch.rs` | 攒批 + 写盘 | 调写盘策略时 |
| `crates/server/src/domain/session.rs` | 在线 agent 缓存 | 改会话管理时 |
| `crates/server/src/ws/hub.rs` | 广播中心 | 加广播策略时 |
| `crates/server/src/ws/client.rs` | 单连接状态机 | 改 WS 协议时 |
| `crates/server/src/ws/messages.rs` | WS 消息 schema | 协议变更时 |
| `crates/server/src/store/sqlite.rs` | SQLite 仓储 | 加新查询时 |
| `crates/server/src/store/postgres.rs` | PG 仓储 | 加新查询时 |
| `crates/server/src/store/timescaledb.rs` | hypertable/CAGG | 加聚合视图时 |
| `crates/server/migrations/*.sql` | 数据库 schema | 加表/加索引时 |
| `crates/agent/src/grpc_client.rs` | tonic client + 重连 | 改传输时 |
| `crates/agent/src/collector/cpu.rs` | CPU 采集 | 改算法时 |
| `crates/agent/src/collector/mem.rs` | 内存采集 | 改算法时 |
| `crates/agent/src/collector/disk.rs` | 磁盘采集 | 改算法时 |
| `crates/agent/src/collector/net.rs` | 网络采集 | 改平台分支时 |
| `crates/agent/src/collector/load.rs` | 负载采集 | 改算法时 |
| `crates/agent/src/collector/gpu.rs` | GPU 采集 | 加新 vendor 时 |
| `crates/agent/src/collector/temperature.rs` | 温度采集 | 改 sysinfo sensors 时 |
| `crates/agent/src/collector/conn_count.rs` | 连接数采集 | 改平台分支时 |
| `crates/agent/src/collector/host.rs` | 静态主机信息 | 加字段时 |
| `crates/agent/src/enroll.rs` | 注册流程 | 改注册协议时 |
| `crates/agent/src/auth.rs` | challenge 签名 | 改鉴权时 |
| `crates/agent/src/reporter.rs` | 状态上报 | 改采样策略时 |
| `crates/agent/src/task_runner.rs` | 任务执行 | 加新任务类型时 |
| `web/app/(authed)/dashboard/page.tsx` | 列表页 | 改列表 UI 时 |
| `web/app/(authed)/dashboard/[id]/page.tsx` | 详情页 | 改详情 UI 时 |
| `web/components/charts/MetricLineChart.tsx` | 折线图 | 改图表样式时 |
| `web/components/charts/Sparkline.tsx` | 迷你图 | 改图表样式时 |
| `web/components/server/ServerCard.tsx` | 卡片 | 改卡片布局时 |
| `web/lib/api-client.ts` | fetch 包装 | 加端点时 |
| `web/lib/ws.ts` | WS hook | 改 WS 协议时 |
| `web/lib/auth.ts` | 认证 | 改认证流程时 |

## 低频（仅在底座变更时改）

| 文件 | 何时改 |
|------|--------|
| `Cargo.toml`（workspace 根） | 加/升级 crate 版本 |
| `crates/server/Cargo.toml` | features 调整 |
| `crates/agent/Cargo.toml` | features 调整 |
| `crates/proto-gen/Cargo.toml` | 几乎不改 |
| `crates/shared/Cargo.toml` | 几乎不改 |
| `crates/xtask/Cargo.toml` | 加新辅助工具时 |
| `web/package.json` | 加新 npm 依赖 |
| `web/next.config.mjs` | 改 Next.js 配置 |
| `web/tailwind.config.ts` | 改 Tailwind 主题 |
| `docker-compose.yml` | 加新服务 |
| `Dockerfile.server` | 改 server 镜像构建 |
| `Dockerfile.agent` | 改 agent 镜像构建 |
| `Caddyfile` | 改反代配置 |
| `.env.example` | 加新环境变量 |
| `proto/xlstatus/v1/*.proto`（package 名 / import） | 改命名空间时 |

## 复用的现成依赖（不重写）

| 用途 | crate / 库 |
|------|-----------|
| CPU/内存/磁盘/进程 | `sysinfo`（net 走平台分支） |
| 路由/中间件 | `axum` + `tower-http`（REST + Dashboard WS） |
| gRPC | `tonic` + `tonic-build`（Agent ↔ Server） |
| gRPC 调试 | `tonic-reflection`（grpcurl） |
| proto 序列化 | `prost` |
| 数据库 + migration | `sqlx` |
| 密码哈希 | `argon2` |
| Ed25519 签名 | `ed25519-dalek` |
| JWT 签发/校验 | `jsonwebtoken` |
| SHA-256 | `sha2` |
| 常量时间比较 | `subtle` |
| 零内存清零 | `zeroize` |
| 限流 | `tower_governor` |
| tracing | `tracing` + `tracing-subscriber` |
| 配置加载 | `config` |
| CLI | `clap` |
| 异步运行时 | `tokio` |
| 图表（前端） | `Recharts` |
| UI 组件（前端） | `shadcn/ui` |
| 终端（前端，预留） | `xterm.js` |
| 数据获取（前端） | `@tanstack/react-query` |
| 表单（前端） | `react-hook-form` + `zod` |
| 图标（前端） | `lucide-react` |
| Tailwind 工具 | `clsx` + `tailwind-merge` + `class-variance-authority` |

## 临时文件（M0 后清理）

| 文件 | 处理时机 |
|------|----------|
| `proto/xlstatus/v1/*.proto` 的 helloworld 占位 | M2 完整版替换 |
| `crates/server/src/main.rs` 里的 `Greeter` helloworld | M0 退出标准达成后删除 |
| `web/app/page.tsx` 的欢迎页 | M3 替换为登录页 |
| `crates/xtask/src/bin/seed.rs` | M9 可选删除 |