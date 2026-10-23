# XLStatus 权威文档索引 (Documentation Index)

欢迎查阅 **XLStatus** 官方技术与运维文档库。`docs/` 目录收录了当前发布版本（`v0.3+`）在安装部署、配置治理、Web 控制台、Agent 接入、AI-Native MCP 运维、日常排障与架构研发所需的全部权威参考文档。

---

## 按角色推荐阅读路径 (Reading Paths by Role)

### 1. 运维工程师 / SRE (Operator & SRE)
面向生产集群搭建、节点纳管、高可用数据库迁移、告警自愈与日常巡检：
1. **[快速开始 (Quickstart)](./quickstart.md)** — 使用 Docker Compose 或本地源码在数分钟内启动 Server、Web UI 与 Agent。
2. **[安装与部署 (Installation)](./installation.md)** — 源码构建、`systemd` 守护进程安装、PostgreSQL 生产建站与远端 Linux 验收。
3. **[配置参考 (Configuration)](./configuration.md)** — `config.toml` 与环境变量二选一模式、CORS 白名单、SQLite/PostgreSQL 连接池与 TLS 配置。
4. **[Agent 接入指南 (Agent)](./agent.md)** — Enrollment Token 签发、一键安装脚本、Ed25519 密钥与 JWT 挑战认证、`systemd` 托管。
5. **[日常运维手册 (Operations)](./operations.md)** — 健康检查、备份与恢复、SQLite → PostgreSQL 在线导入、平滑升级与远端 Smoke 验证。
6. **[故障排查 (Troubleshooting)](./troubleshooting.md)** — 进程退出码诊断、端口冲突、数据库锁、CORS 拒绝与 Agent 断连定位。
7. **[安全修复与边界基线 (Security)](../security.md)** — 进程组隔离、gRPC 默认回环绑定、SSRF 防护、TOTP 二次验证与安全审计记录。

### 2. AI 与自动化工程师 (AI & Automation Engineer)
面向 LLM 智能体接入、自动化巡检诊断、MCP 工具调用与可编程 API 编排：
1. **[AI-Native 集群运维与 MCP 指南 (MCP Reference)](./mcp.md)** — 完整解析 MCP `2024-11-05` 协议实现、3 种传输模式、19 个工具、6 个只读 SRE 诊断剧本、6 类 `xlstatus://` 资源、4 个 AI SRE 提示词模板及客户端配置（Antigravity / Cursor / Windsurf / Claude Desktop / Claude Code）。
2. **[HTTP / WebSocket / gRPC / MCP 接口概览 (API)](./api.md)** — 公共与受保护端点、请求体字节预算、PAT Scope 权限矩阵与一次性文件传输令牌。
3. **[安全修复与边界基线 (Security)](../security.md)** — 理解 PAT `server_ids` 租户隔离、文件写操作禁用策略与敏感指令 TOTP 校验边界。

### 3. 前端与后端研发工程师 (Frontend & Backend Developer)
面向二次开发、协议扩展、架构理解与版本发布验收：
1. **[系统架构参考 (Architecture)](./architecture.md)** — 控制面、数据面、Web 工作台与 AI MCP 拓扑，以及五大核心数据流与安全信任边界。
2. **[项目结构说明 (Project Structure)](./project-structure.md)** — Rust Workspace 六大 Crate、Next.js App Router、Protobuf 协议定义、部署脚本与验收测试目录全景。
3. **[Web 前端指南 (Web UI)](./web.md)** — Next.js 构建、运行、`NEXT_PUBLIC_API_URL` 注入、中英双语 i18n 字典与安全头部策略。
4. **[开发与测试指南 (Development)](./development.md)** — 本地联调、代码规范、`test-run/verify-m0..m9` 全链路验收脚本说明。
5. **[发布前检查清单 (Release Checklist)](./release-checklist.md)** — 版本号同步、跨平台编译、安装脚本校验与 Release 资产核对。

---

## 全量文档目录 (Full Document Catalog)

| 文档 | 核心主题与涵盖范围 | 适用对象 |
|---|---|---|
| **[quickstart.md](./quickstart.md)** | 最短路径启动 Server、Web UI 与 Agent，完成首台节点上线 | 全员 |
| **[installation.md](./installation.md)** | Docker Compose、源码编译、`systemd` 安装脚本、PostgreSQL 部署 | 运维 / SRE |
| **[configuration.md](./configuration.md)** | 配置加载优先级、环境变量模式、`config.toml`、数据库与 CORS 规则 | 运维 / SRE |
| **[web.md](./web.md)** | Next.js 16 前端构建、运行、API 地址绑定、CSP/安全头与双语 i18n | 前端 / 运维 |
| **[agent.md](./agent.md)** | Agent 注册流、Ed25519 密钥落盘、gRPC 会话配置与 `systemd` 服务管理 | 运维 / SRE |
| **[mcp.md](./mcp.md)** | **旗舰指南**：MCP `2024-11-05` 协议、19 工具、6 诊断剧本、资源、Prompts 与 IDE 接入 | AI / SRE / 研发 |
| **[api.md](./api.md)** | HTTP REST API、WebSocket 实时流、Agent gRPC 与 MCP 端点参考 | AI / 研发 |
| **[architecture.md](./architecture.md)** | 控制面与数据面架构、组件拆解、5 大核心业务数据流（含 Mermaid 图）与信任边界 | 研发 / 架构师 |
| **[project-structure.md](./project-structure.md)** | 仓库目录布局、`crates/*`、`web/*`、`proto/*`、`scripts/*`、`test-run/*` 详解 | 研发 |
| **[operations.md](./operations.md)** | 启停维护、数据备份、SQLite → PostgreSQL 迁移脚本、版本升级与回滚 | 运维 / SRE |
| **[troubleshooting.md](./troubleshooting.md)** | 常见启动失败、CORS 报错、Agent 离线、终端连接与数据库排障 | 运维 / SRE |
| **[development.md](./development.md)** | 本地开发环境搭建、Cargo/pnpm 常用命令、M0–M9 自动化验收脚本 | 研发 |
| **[release-checklist.md](./release-checklist.md)** | 发版前质量门禁、多平台二进制产物清单与安装器回归检查 | 维护者 |
| **[../security.md](../security.md)** | 已验证的安全修复记录、协议级字节上限、SSRF 防护与残余风险边界 | 全员 / 安全审计 |

---

## 系统核心能力概览 (System Capabilities Summary)

- **控制面 (`xlstatus-server`)**：基于 Rust、Axum 与 Tonic 构建的单二进制服务，统一承载 REST API、WebSocket 实时推送、Agent 双向 gRPC 信道、告警评估引擎、Cron 任务调度器、DDNS 同步器、NAT 反向隧道管理器以及原生 MCP `2024-11-05` 引擎。
- **采集与执行端 (`xlstatus-agent`)**：运行于受控主机的轻量级守护进程，基于 `sysinfo` 采集 CPU、内存、Swap、磁盘、网络吞吐、连接数与负载；通过双向 gRPC Stream (`Session` & `IoStream`) 执行远程 Shell、HTTP/TCP/ICMP 拨测、文件传输、交互式 PTY 终端与 NAT 流量转发。
- **时序与关系存储 (`xlstatus-tsdb` + SQLite / PostgreSQL)**：内置数据库 Schema 自动迁移，支持零外部依赖的嵌入式 SQLite（WAL 模式）或生产级 PostgreSQL，配合 `xlstatus-tsdb` 提供 1h / 24h / 7d / 30d 降采样指标查询。
- **Web 运维工作台 (`web/`)**：基于 Next.js 16 (App Router)、React 19 与 Tailwind CSS v4 构建，包含实时集群大盘、3D/SVG 世界地图、Web 终端、文件管理器、服务拨测 SLA、告警规则、NAT/DDNS 管理、MCP AI Ops Studio 与公开状态页（含 MJPEG 摘要流）。
- **AI-Native MCP 体系 (`crates/server/src/mcp/` + `scripts/xlstatus-mcp-stdio.mjs`)**：原生实现 Model Context Protocol `2024-11-05`，支持 Streamable HTTP (`POST /mcp`)、Web AI Ops Studio 与 Stdio 桥接器三种接入形态，内置 19 项工具、6 套只读 SRE 诊断剧本、6 类 `xlstatus://` 上下文资源与 4 套动态注入实时遥测的 SRE 提示词。
- **跨平台分发**：GitHub Release 提供 Linux (`x86_64` / `arm64` / `i386`)、Windows (`x86_64` / `arm64` / `i386`)、macOS (`x86_64` / `arm64`) 与 FreeBSD (`x86_64` / `i386`) 预编译二进制，配套 `deploy/install.sh` 与 `deploy/install-agent.sh` 一键部署脚本（默认 fallback 版本 `v0.3`）。

---

## 核心运行不变量与铁律 (Core Operational Invariants)

在配置、部署或二次开发 XLStatus 时，必须严格遵守以下系统不变量：

1. **配置源互斥 (`DATABASE_URL` vs `CONFIG_FILE`)**：
   - `DATABASE_URL` 与 `CONFIG_FILE` **绝不合并**。只要环境变量中设置了 `DATABASE_URL`，Server 立即进入纯环境变量配置模式并完全忽略 `config.toml`。
2. **显式 SQLite 建库授权**：
   - SQLite 新建数据库文件必须显式携带 `?mode=rwc`（环境变量模式）或在 `config.toml` 中设置 `create_if_missing = true`。非交互式环境下 Server 绝不会静默创建未授权的数据库文件。
3. **严格的 CORS 与 WebSocket Origin 校验**：
   - Web 前端的 `NEXT_PUBLIC_API_URL` 在构建期写入浏览器 bundle（修改后必须重新 `pnpm build`），仅用于告知浏览器请求目标；Server 侧仍必须通过 `CORS_ALLOWED_ORIGINS` 或 `server.cors_allowed_origins` 精确放行前端 Origin（禁止使用通配符 `*`）。WebSocket 握手（`/api/v1/ws`、`/ws/servers`、终端流）同样强制校验 `Origin` 白名单。
4. **Agent gRPC 默认回环绑定**：
   - Docker Compose 与 `systemd` 安装脚本默认将宿主机的 Agent gRPC 端口绑定在 `127.0.0.1:50051`。如需跨公网接入 Agent，必须显式配置 `XLSTATUS_GRPC_PUBLISH_ADDRESS=0.0.0.0` 并配合 TLS/mTLS 或可信 VPN/WireGuard 隧道。
5. **Agent 身份与撤销即时生效**：
   - Agent Enrollment Token (`xle_...`) 仅用于首次注册换取 `agent_id` 并登记 Ed25519 公钥；后续每次建立 gRPC 连接前均通过 Ed25519 签名 Challenge-Response 换取短期 JWT。管理员撤销 Agent 后，控制面立即写入 `revoked_at`、下发 `ForceDisconnect`，并从 `SessionRegistry` 与 `IoRegistry` 同步摘除，杜绝任何迟到任务或幽灵流继续收发数据。
6. **PAT 最小权限与多租户节点隔离**：
   - Personal Access Token (`xlp_...`) 同时受 **Scope 集合**与可选 **`server_ids` 节点白名单**双重约束。MCP 与 REST API 在查询集群概览、读取遥测、列出服务/告警/任务或执行命令时，均会在 `McpExecutor` 与数据库查询层强制过滤不可见节点，且禁止通过 PAT 执行远端文件写入/删除操作。
