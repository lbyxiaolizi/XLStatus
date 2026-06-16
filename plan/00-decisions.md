# 核心决策

## 设计原则

- 功能范围以 [01-scope.md](./01-scope.md) 为准，必须完整覆盖 Nezha 类监控、告警和运维能力。
- 工程落地以本文件、[11-workspace-layout.md](./11-workspace-layout.md)、[12-dependencies.md](./12-dependencies.md)、[13-protocols.md](./13-protocols.md)、[14-api-contracts.md](./14-api-contracts.md) 为准。
- 性能和安全默认优先，但不以砍掉核心功能换取短期简单；Web Terminal、文件管理、NAT、MCP 都进入 v1，只允许分阶段交付。

## 决策表

| ID | 决策点 | 选定 | 理由 |
|----|--------|------|------|
| D1 | Agent 协议 | Tonic gRPC over HTTP/2 | 长连接、多路复用、二进制编码、背压和健康检查成熟，适合 Agent 状态流和任务流。 |
| D2 | Dashboard 协议 | Axum REST + WebSocket | REST 适合管理 API，WebSocket 适合浏览器实时状态、终端、传输进度。 |
| D3 | 前端 | Next.js App Router + TypeScript | 管理后台和公开状态页共用工程，适合 SSR/CSR 混合。 |
| D4 | Web 认证 | HttpOnly Cookie session + refresh rotation + CSRF | 浏览器安全默认好，能吊销，避免把长期 token 暴露给 JS。 |
| D5 | PAT | `xlp_` 明文前缀 + hash 存储 + scope + server allowlist | 支持自动化、MCP 和 CI，同时限制 blast radius。 |
| D6 | Agent 首次注册 | 一次性 enrollment token | 避免全局共享 Agent secret，便于每台机器独立吊销。 |
| D7 | Agent 长期身份 | Ed25519 keypair + server 存 public key | 比 UUID+共享 secret 更安全，可做 challenge-response 和无缝吊销。 |
| D8 | Agent 会话 | 5 分钟短期 JWT + gRPC metadata | 不对每条高频状态消息签名，减少 CPU；JWT 过期窗口短。 |
| D9 | 数据访问 | SQLx + repository trait | 避免 ORM 反射成本，保持 SQLite/PostgreSQL 双后端可控。 |
| D10 | 元数据存储 | SQLite + PostgreSQL 运行时切换 | SQLite 用于开发和小规模部署；PostgreSQL 是生产推荐，不通过 feature flag 锁死。 |
| D11 | 高频指标存储 | TSDB/MetricStore，不直接进 SQL 元数据层 | 避免大流量下关系库被 Agent 高频状态写入压垮。 |
| D12 | PostgreSQL 高 IO | 分区表 + 批量写入 + 连接池指标 | 面向服务结果、任务结果、审计和传输记录的写入压力。 |
| D13 | Dashboard WS | 单连接多订阅，服务端按权限过滤 | 浏览器资源占用低，权限边界集中在后端。 |
| D14 | 远程执行 | v1 支持，但默认强权限、审计和禁用开关 | 完整对标必须包含任务、终端和文件；以安全边界降低风险。 |
| D15 | MCP | 默认关闭，只接受 PAT | 自动化能力强但风险高，必须显式启用和细粒度授权。 |
| D16 | NAT | v1 支持，保留 host 防抢占 | 对标运维能力；反代部署必须防止成员抢占 Dashboard 域名。 |
| D17 | 通知/HTTP 出站 | 严格 SSRF 防护 | 通知、DDNS webhook、HTTP 探测都可能被滥用访问内网。 |
| D18 | 时间 | 服务端时间为准，Agent 时间仅作参考 | 防止 Agent 时钟漂移污染告警和审计。 |
| D19 | i18n | v1 中文优先，代码结构预留英文 | 先完成核心功能，避免 UI 文案系统阻塞后端。 |
| D20 | 发布 | Docker/Linux x86_64 先行 | 缩小首发平台面，保证可交付质量。 |

## 不采纳的方案

- 不使用 Nezha 原有 gRPC proto、REST API、数据库 schema 或前端资产。
- 不把 PostgreSQL 做成很晚期的可选补丁；M1 就必须建立双后端抽象。
- 不把 Web Terminal、文件管理、NAT、MCP 从 v1 移除，只允许按里程碑后置。
- 不把高频服务器指标长期写入 SQL 元数据表。

## 变更流程

1. 修改任何核心决策时同步更新本文件。
2. 若影响里程碑，更新 [08-roadmap.md](./08-roadmap.md)。
3. 若影响接口，更新 [13-protocols.md](./13-protocols.md) 或 [14-api-contracts.md](./14-api-contracts.md)。
4. 在 [99-changelog.md](./99-changelog.md) 记录原因和影响范围。

