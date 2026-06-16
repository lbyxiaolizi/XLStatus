# 实施里程碑

## 总览

| 里程碑 | 主题 | 关键产物 | 退出标准 |
|--------|------|----------|----------|
| M0 | 脚手架 | workspace + proto + Axum/Tonic hello + Next.js | 三服务可同时启动 |
| M1 | 基础平台 | DB 双后端 + Web Auth + RBAC + PAT | SQLite/PostgreSQL CRUD 都通过 |
| M2 | Agent 接入 | Enrollment + Ed25519 + JWT + gRPC Session | Agent enroll/run 全流程通 |
| M3 | 实时监控 | 采集器 + TSDB + WS + Dashboard 图表 | 浏览器实时看到状态变化 |
| M4 | 服务监控与告警 | HTTP/TCP/ICMP/SSL + 规则引擎 + 通知 | 服务失败和恢复能通知 |
| M5 | 运维能力 | 任务 + Web Terminal + 文件管理 + 传输 | 能远程执行、终端、传文件 |
| M6 | 网络与自动化 | DDNS + NAT + MCP | MCP 和 NAT 可用 |
| M7 | 前端完备 | 管理后台 + 公开状态页 + 权限视图 | 核心流程 UI 闭环 |
| M8 | 高 IO 与性能 | PG 分区 + 批量写 + 压测 + 外部指标预留 | 100 Agent 24h 稳定 |
| M9 | 发布稳定 | Docker + systemd + 安装脚本 + 文档 | 新机器 5 分钟接入 |

## M0 脚手架

交付：

- 根 `Cargo.toml` 改为 workspace。
- 创建 `crates/shared`、`crates/proto-gen`、`crates/server`、`crates/agent`、`crates/xtask`。
- 创建 `proto/xlstatus/v1/common.proto` 和 `agent.proto`。
- Server 同时启动 Axum `:8080 /healthz` 和 Tonic `:50051`。
- Tonic 开启 health 和 reflection。
- Web 初始化 Next.js。

验收：

- `cargo build --workspace` 通过。
- `curl http://localhost:8080/healthz` 返回 200。
- `grpcurl -plaintext localhost:50051 list` 能看到 `xlstatus.v1.AgentService`。
- `curl http://localhost:3000` 能看到 XLStatus 页面。

## M1 基础平台

交付：

- SQLite 和 PostgreSQL migrations。
- `DatabaseBackend`、连接池、repository trait、migration runner。
- 配置文件和环境变量加载。
- tracing 日志和 request_id。
- 用户表、初始化管理员、登录、刷新、登出。
- Cookie session、refresh rotation、CSRF。
- RBAC、PAT scope、server allowlist。
- Next.js 登录页和管理后台骨架。

验收：

- 管理员可以登录、刷新、登出。
- 管理员可以创建成员用户。
- PAT 可以创建、列出、吊销。
- 同一套 repository 测试在 SQLite 和 PostgreSQL 上通过。
- Cookie HttpOnly、SameSite、CSRF 校验通过安全测试。

## M2 Agent 接入

交付：

- Agent enrollment token API。
- Agent Ed25519 keypair 生成和 0600 落盘。
- Agent JWT 签发、校验、challenge refresh。
- gRPC `Session` 和 `IoStream` 骨架。
- Agent reconnect、backpressure、流发送串行化。
- Server Agent registry、session 替换、吊销。

验收：

- Agent `enroll` 后能保存 ID 和 key。
- Agent `run` 后能建立 gRPC Session。
- Server 能看到 `last_seen_at` 更新。
- 5 分钟 JWT 续签不中断状态流。
- 管理员吊销 Agent 后，Agent 收到 `ForceDisconnect` 并退出连接。

## M3 实时监控

交付：

- Linux x86_64 采集器：CPU、内存、Swap、磁盘、网络、负载、连接数、进程数、温度、GPU。
- HostInfo 和 HostState 上报。
- TSDB 初版写入服务器指标。
- WebSocket `/ws/servers`。
- 服务器列表、详情页、指标图表。
- Agent 离线检测。

验收：

- Dashboard + 本机 Agent 能显示实时状态。
- CPU、内存、网络、负载数字持续变化。
- Agent 断开 30 秒内显示离线。
- 1d 指标图可查询。
- 指标写入不进入 SQL 高频明细表。

## M4 服务监控与告警

交付：

- HTTP GET、ICMP Ping、TCP Ping、SSL 证书探测。
- 服务监控调度器和结果聚合。
- 30 天服务历史和可用率。
- 告警规则：资源、离线、周期流量、服务状态、延迟。
- 通知渠道、通知组、通知模板。
- 失败、恢复、延迟越界通知。

验收：

- 可配置 HTTPS 服务并看到证书状态。
- CPU 或离线规则能触发通知。
- 服务恢复会发送恢复通知。
- SSRF 防护覆盖通知、DDNS webhook、HTTP monitor。

## M5 运维能力

交付：

- 定时任务、触发任务、手动批量执行。
- TaskResult 聚合和审计。
- Web Terminal。
- 文件列表、读取、写入、删除。
- 100 MiB 文件传输。
- Agent 远程配置读取和应用。
- Agent 强制更新接口。

验收：

- 管理员可以在 UI 打开 Agent shell 并执行 `echo ok`。
- 可以上传、下载、删除测试文件。
- 批量任务正确返回 success、failure、offline。
- 禁用命令执行后，终端、exec、文件写入都被拒绝。

## M6 网络与自动化

交付：

- DDNS provider：Cloudflare、Tencent Cloud、HE、Webhook、Dummy。
- Agent IP 变化触发 DDNS。
- NAT 域名反代和 Agent 隧道。
- MCP JSON-RPC endpoint。
- MCP tools：meta.whoami、server.list、server.get、server.exec、fs.list、fs.read、fs.write、fs.delete、fs.download_url、fs.upload_url。
- MCP 临时 URL 和限流。

验收：

- IP 变化可更新测试 DDNS provider。
- Dashboard 域名可代理到 Agent 内网 HTTP 服务。
- MCP client 可以列服务器、执行命令、读写文件。
- MCP 默认关闭，启用后只接受 PAT。

## M7 前端完备

交付：

- 管理后台所有资源页面。
- 公开状态页。
- 服务可用性视图。
- 权限控制和成员视图。
- 移动端状态查看。
- 表单校验、危险操作确认、错误提示。

验收：

- 管理员所有核心配置可在 UI 完成。
- 成员看不到无权服务器、服务、任务、通知和审计。
- 未登录访客只看到公开资源。

## M8 高 IO 与性能

交付：

- PostgreSQL 分区表和保留策略。
- service_results、task_runs、audit_logs、transfers 批量写入。
- 连接池指标和慢查询日志。
- TSDB compact、retention、query bench。
- `MetricStore` 外部后端接口。
- mock agents 压测工具。

验收：

- 100 Agent、3 秒上报、24 小时稳定。
- 1000 服务监控任务按 30 秒周期调度稳定。
- 1d/7d/30d 查询 P95 小于 500 ms。
- PostgreSQL 连接池耗尽时返回可观测错误，不阻塞 Agent 状态流。

## M9 发布稳定

交付：

- Dockerfile、docker-compose、docker-compose.pg。
- Linux x86_64 Dashboard 和 Agent release 包。
- systemd unit。
- 一键安装脚本。
- 备份、恢复、维护命令。
- OpenAPI、管理员手册、Agent 手册、故障排查。
- 安全测试、长稳测试、性能报告。

验收：

- 新机器按文档 5 分钟内完成 Dashboard 和 Agent 接入。
- `docker compose up` 可一次启动全栈。
- 关键安全测试全部通过。
- 24 小时长稳无 panic、无明显内存泄漏。

## 后续版本

- Linux arm64 Agent。
- Windows Agent。
- macOS Agent。
- MySQL/MariaDB 元数据后端评估。
- VictoriaMetrics、ClickHouse 或 TimescaleDB 外部指标后端实现。
- 多节点 Dashboard。
- 更细粒度的文件沙箱策略。
- 插件化通知 provider 和 DDNS provider。
