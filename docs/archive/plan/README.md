# XLStatus 最终总计划

## 目标

XLStatus 是一套用 Rust 重新实现的自托管服务器监控与运维系统，功能完整对标 Nezha 当前监控系统，但不追求兼容 Nezha 的 API、数据库、Agent 协议、安装脚本或前端行为细节。

本目录是最终权威计划，已合并 `./plan` 的完整功能范围和 `./plan-mmx` 的工程执行细节。后续实现以本目录为准，`./plan-mmx` 仅作为历史参考。

第一版技术栈固定为：

- 后端：Rust、Tokio、Axum、Tonic、SQLx、SQLite/PostgreSQL
- Agent RPC：Tonic gRPC 双向流
- 前端：Next.js App Router、TypeScript
- 实时通信：WebSocket
- 指标存储：本地嵌入式 TSDB，预留外部高性能指标后端
- 首发部署：Docker 和 Linux x86_64

## 产品边界

- 必须覆盖 Dashboard、Agent、实时服务器状态、服务监控、告警、任务、通知、DDNS、NAT、MCP、Web Terminal、文件管理、文件传输、TSDB、用户和权限。
- 不复制 Nezha 品牌、图标、前端资产、接口字段或数据库结构。
- 不提供从现有 Nezha 实例无缝迁移的承诺。
- Windows 和 macOS Agent 写入长期规划，第一版只要求 Linux x86_64 生产可用。
- 存储层必须兼容 SQLite 和 PostgreSQL；SQLite 面向开发、小规模和单机轻量部署，PostgreSQL 是大流量生产推荐后端。
- 高 IO 场景不得把 Agent 指标明细直接写入关系数据库；服务器指标和服务历史优先进入 TSDB，关系库只保留配置、索引、审计和必要聚合。

## 规划文件

- [00-decisions.md](./00-decisions.md)：核心决策
- [01-scope.md](./01-scope.md)：功能对标范围
- [02-architecture.md](./02-architecture.md)：整体架构
- [03-backend.md](./03-backend.md)：后端规划
- [04-agent.md](./04-agent.md)：Agent 规划
- [05-frontend.md](./05-frontend.md)：前端规划
- [06-data-model.md](./06-data-model.md)：数据模型
- [07-security.md](./07-security.md)：安全设计
- [08-roadmap.md](./08-roadmap.md)：里程碑
- [09-test-plan.md](./09-test-plan.md)：测试计划
- [10-ops-release.md](./10-ops-release.md)：运维发布
- [11-workspace-layout.md](./11-workspace-layout.md)：workspace 和文件职责
- [12-dependencies.md](./12-dependencies.md)：依赖和 feature flags
- [13-protocols.md](./13-protocols.md)：gRPC、WebSocket、MCP 协议
- [14-api-contracts.md](./14-api-contracts.md)：REST API 合约
- [15-verification-commands.md](./15-verification-commands.md)：命令级验收
- [16-komari-nezha-gap.md](./16-komari-nezha-gap.md)：Komari / Nezha 对标缺口计划
- [99-changelog.md](./99-changelog.md)：规划变更日志

## 阶段总览

1. M0 脚手架：workspace、proto、Axum/Tonic hello、Next.js 起步。
2. M1 基础平台：SQLite/PostgreSQL、登录、RBAC、PAT、CSRF。
3. M2 Agent 接入：enrollment、Ed25519、短期 JWT、gRPC Session。
4. M3 实时监控：采集器、TSDB、WebSocket、Dashboard 图表。
5. M4 服务监控与告警：HTTP、TCP、ICMP、SSL、资源规则、通知。
6. M5 运维能力：任务、Web Terminal、文件管理、传输、配置和更新。
7. M6 网络与自动化：DDNS、NAT、MCP、临时 URL。
8. M7 前端完备：管理后台、公开状态页、权限视图和移动端查看。
9. M8 性能与高 IO：PostgreSQL 分区、批量写、外部指标后端预留、压测。
10. M9 发布稳定：Docker、systemd、一键安装、备份恢复、文档和长稳测试。
11. M10+ 对标补齐：通知管理、告警扩展、服务覆盖、服务器分组、OAuth2/2FA/WAF、主题、多语言、备份恢复、GeoIP、Cloudflare Tunnel 和兼容入口。

## 全局验收标准

- 单机 Docker 部署后，5 分钟内可以完成 Dashboard 初始化、Agent 接入、服务器状态展示。
- 生产部署可以通过配置切换到 PostgreSQL，无需改代码或重新编译。
- Linux x86_64 Agent 可以稳定上报 CPU、内存、磁盘、网络、负载、连接数、进程数、温度和 GPU 可用数据。
- 管理员可以配置服务监控、告警规则、通知渠道、任务、DDNS、NAT、MCP 和用户权限。
- 成员只能访问自己拥有或被授权的服务器和资源。
- 所有远程执行、文件写入、文件删除、MCP、NAT、PAT 操作都有权限校验和审计记录。
- 30 天服务可用性和 1d、7d、30d 服务器指标查询可用。
