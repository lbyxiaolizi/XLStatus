[English](./README.md) | 简体中文

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/badge/release-v0.3-10b981.svg)](https://github.com/lbyxiaolizi/XLStatus/releases)
[![Rust](https://img.shields.io/badge/Rust-Axum%20%2B%20Tonic%20gRPC-orange.svg)](./crates/)
[![Frontend](https://img.shields.io/badge/Next.js%2016-React%2019%20%2B%20Strict%20TS-000000.svg)](./web/)
[![MCP](https://img.shields.io/badge/MCP-2024--11--05-6366f1.svg)](./docs/mcp.md)

**XLStatus** 是一个基于 **Rust**（`Axum` + `Tonic` gRPC）与 **Next.js 16**（`React 19` + 全严格模式 TypeScript）构建的 **高安全、AI 原生（AI-Native）自托管基础设施监控与集群运维平台**。

它将主机实时遥测、可定制公开状态页、分布式与本地服务拨测、多条件告警与自愈联动、批量/定时任务调度、WebSocket PTY 终端、安全远程文件管理、多厂商 DDNS、NAT 反向隧道以及原生 **Model Context Protocol（`MCP 2024-11-05`）** 服务端整合进同一套经过深度安全审计的部署栈中。

> **当前正式版本**：`v0.3` —— 推荐从 [文档索引](./docs/README.md)、[AI 与 MCP 运维指南](./docs/mcp.md) 以及 [安全架构与加固白皮书](./security.md) 开始阅读。

---

## 系统架构拓扑

```mermaid
flowchart TB
  subgraph Clients["运维人员、公开访客与 AI 智能体"]
    WebUI["Next.js 16 运维工作台<br/>(Priority+ 自适应导航 · ⌘K 命令面板 · MCP 演练场)"]
    PublicStatus["公开状态页 (/status)<br/>(SSR · 卡片 / 表格 / 全球地图三视图)"]
    HttpMCP["Streamable HTTP MCP 客户端<br/>(Claude Code · Cursor · Windsurf · Antigravity)"]
    StdioMCP["Stdio MCP 客户端<br/>(Claude Desktop · MCP Inspector)"]
  end

  subgraph Bridge["MCP 传输桥接层"]
    StdioScript["scripts/xlstatus-mcp-stdio.mjs<br/>(零依赖 Node.js Stdio 桥接器)"]
  end

  subgraph ControlPlane["xlstatus-server (Rust · Axum HTTP/WS + Tonic gRPC)"]
    AuthLayer["纵深防御鉴权边界<br/>(Argon2id · 一次性 TOTP · HMAC CSRF · 细粒度 PAT)"]
    RestWS["REST API (/api/v1/*) 与实时流 (/ws/servers)"]
    McpEngine["MCP 2024-11-05 引擎 (/mcp)<br/>(19 运维工具 · 6 实时资源 · 4 SRE 提示词)"]
    CoreServices["告警引擎 · Cron 调度器 · DDNS · NAT 隧道管理器"]
    GrpcServer["Tonic gRPC 控制与数据平面 (:50051)<br/>(Ed25519 注册挑战 · JWT · 会话/任务/IO 注册表)"]
  end

  subgraph Storage["持久化与时序存储"]
    DB[("SQLite / PostgreSQL<br/>(HKDF-SHA256 enc:v2: 静态密钥加密)")]
    TSDB[("内置时序数据库 TSDB<br/>(多窗口聚合降采样与过期清理)")]
  end

  subgraph Fleet["xlstatus-agent 节点集群 (Linux · macOS · Windows · FreeBSD)"]
    Agent["xlstatus-agent 守护进程<br/>(0600 私钥配置 · 单向不可逆本地安全锁)"]
    Subsystems["主机遥测采集 · HTTP/TCP/ICMP 探针 · 进程组级 Shell<br/>WebSocket PTY · O_NOFOLLOW 文件操作 · NAT 本地拨号"]
  end

  WebUI <-->|"Cookie + CSRF + x-totp-code"| AuthLayer
  PublicStatus <-->|"脱敏公开 DTO"| RestWS
  HttpMCP <-->|"POST /mcp (Bearer xlp_*)"| AuthLayer
  StdioMCP <--> StdioScript <-->|"POST /mcp (Bearer xlp_*)"| AuthLayer

  AuthLayer --> RestWS & McpEngine
  RestWS & McpEngine --> CoreServices & GrpcServer
  RestWS & McpEngine & CoreServices --> DB & TSDB
  GrpcServer <-->|"双向 gRPC 流 (TLS/mTLS)"| Agent
  Agent --> Subsystems
```

---

## 六大核心能力矩阵

| 核心支柱 | 关键特性与架构亮点 |
| :--- | :--- |
| **1. 实时集群遥测与公开状态页** | 秒级采集 CPU、内存、Swap、磁盘占用、网络实时吞吐（`rx_bps` / `tx_bps`）与累计流量、系统负载（`1m/5m/15m`）、TCP/UDP 连接数、进程数、GPU 与温度。内置基于 `d3-geo` 的全球节点地图（`WorldServerMap`），以及支持 **卡片 / 表格 / 全球地图** 三视图切换、地区与状态胶囊筛选、SLA 可用率历史条与 SSR 元数据的公开状态页（`/status`）。 |
| **2. 服务拨测、SLA 与告警自愈联动** | 支持分布式（Agent 节点发起）与本地（Server 发起）**HTTP(S)、TCP、ICMP** 拨测，自动追踪 TLS 证书剩余有效期与延迟分布。支持多条件组合告警规则、持续时间窗口、恢复通知、单遍 RFC 8259 安全转义的 JSON Webhook 投递，以及故障自动触发自愈任务（`trigger_task`）。 |
| **3. 交互式集群运维工具箱** | 批量/Cron 任务调度工作台（内置一键 SRE 排障剧本）；带快捷诊断指令栏的 `xterm.js` WebSocket PTY 终端；基于 `O_NOFOLLOW \| O_NONBLOCK` 与 `S_IFREG` 校验的安全远程文件管理器及一次性临时传输链接（`xlt_`）；带单 IP 限流的 NAT 反向 TCP 隧道；以及支持 **Cloudflare、HE.net、DNSPod、Webhook** 的多厂商 DDNS。 |
| **4. AI-Native 集群副驾驶与 MCP `2024-11-05` 服务端** | 原生实现 `/mcp` JSON-RPC 2.0 服务端与 `/mcp` Web 智能运维工作台。提供 **19 个面向 AI 优化的运维工具**（`cluster.diagnose` 全集群根因扫描、`cluster.overview`、`server.resolve` 自然语言节点解析、`server.inspect` 6 大跨平台只读排障剧本、`server.exec`、`server.batch_exec`、`server.metrics` 等）、**6 类 `xlstatus://` 实时遥测资源** 与 **4 套自动注入实时指标的 SRE 提示词模板**，同时支持 Streamable HTTP（`POST /mcp`）与零依赖 Node.js `stdio` 桥接器（`scripts/xlstatus-mcp-stdio.mjs`）。 |
| **5. 现代化自适应前端工作台（`web/`）** | 基于 **Next.js 16 + React 19** 构建，通过 TypeScript 全严格模式校验（`strict`、`noUncheckedIndexedAccess`、`exactOptionalPropertyTypes`、`noPropertyAccessFromIndexSignature`）。配备基于离屏测量与 `ResizeObserver` 的 **PC 端 Priority+ 自适应溢出顶部导航栏**（`Navigation.tsx`）、全局 **`⌘K` / `Ctrl+K` 命令面板**、Neo-Brutalist 亮/暗双主题以及分类设置中心。 |
| **6. 三轮高权限纵深防御安全架构** | 历经三轮高权限专项审计加固：Argon2id + 常量时间防枚举登录、单时间步长一次性消费 TOTP 2FA（含防爆破锁定与敏感操作 `x-totp-code` 二次验证）、HMAC-SHA256 CSRF、细粒度 PAT Scope + `server_ids` 节点白名单、`HKDF-SHA256` 静态密钥加密（`enc:v2:`）、Ed25519 挑战-应答注册（`xle_`）、绑定 `SocketAddr` 的 DNS 防重绑定 SSRF 防护（封堵 IPv4/IPv6 私网、环回、链路本地、SIIT、NAT64、6to4 与云元数据地址），以及 Agent 本地 `0600` 配置与单向不可逆安全锁。 |

---

## 快速开始（Docker Compose）

### 1. 默认部署栈（SQLite）

```bash
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus

# 创建初始管理员强密码与会话签名密钥，并设置严格文件权限
mkdir -p .secrets
printf '%s\n' 'replace-with-a-strong-initial-password' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password

test -f .env || (umask 077; printf 'SESSION_SECRET=%s\nPOSTGRES_PASSWORD=%s\n' \
  "$(openssl rand -hex 32)" "$(openssl rand -hex 24)" > .env)

docker compose up -d
curl -fsS http://localhost:8080/healthz
```

启动后即可访问：
- **运维工作台（Web UI）**：`http://localhost:3000`
- **公开状态页**：`http://localhost:3000/status`
- **AI 集群副驾驶与 MCP 工作台**：`http://localhost:3000/mcp`
- **HTTP API 与 MCP 协议端点**：`http://localhost:8080`（`POST http://localhost:8080/mcp`）

> **Agent gRPC（`:50051`）安全说明**：Docker Compose 默认仅将宿主机 gRPC 端口发布在 `127.0.0.1:50051`。如需跨公网接入远端 Agent，**仅应在已启用 gRPC TLS/mTLS 或可信私网（WireGuard/Tailscale）的前提下**设置 `XLSTATUS_GRPC_PUBLISH_ADDRESS=0.0.0.0`。请妥善保管 `.env` 文件，升级时切勿重新生成 `SESSION_SECRET`（服务端依赖该密钥通过 `HKDF-SHA256` 派生数据库静态加密密钥）。

### 2. PostgreSQL 部署栈

```bash
docker compose -f docker-compose.pg.yml up -d
```

---

## Release 一键安装脚本（Linux systemd）

### 安装 Server

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.3/install-server.sh \
  | sudo VERSION=v0.3 bash
```

### 注册并安装 Agent

可在后台 **设置 -> Agent 安装** 页面一键生成带参数的引导命令，或直接执行：

```bash
sudo SERVER_URL=https://status.example.com \
  GRPC_SERVER=https://grpc.status.example.com:50051 \
  VERSION=v0.3 \
  ENROLLMENT_TOKEN=xle_your_single_use_enrollment_token \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.3/install-agent.sh | bash'
```

Server 与 Agent 安装脚本均采用同目录原子暂存与替换机制，内置 systemd 内核级沙箱加固（`NoNewPrivileges`、`ProtectSystem`、`ProtectHome`、`UMask=0077`），并在健康检查失败时自动回滚至升级前状态。

---

## AI Agent 与 MCP 快速接入

XLStatus 原生实现 **MCP `2024-11-05`** 规范。您可以在 **访问令牌（`/tokens`）** 或 **AI 运维工作台（`/mcp` -> 一键接入向导）** 中选择权限预设（*只读巡检* / *标准排障* / *全权限自治*）及可选的节点白名单（`server_ids`），一键签发 `xlp_...` 令牌。

### 方式一：Streamable HTTP 直连（`POST /mcp`）
适用于 **Claude Code、Cursor、Windsurf、Antigravity** 等支持 HTTP MCP 的客户端：

```json
{
  "mcpServers": {
    "xlstatus": {
      "url": "http://localhost:8080/mcp",
      "headers": {
        "Authorization": "Bearer xlp_your_personal_access_token"
      }
    }
  }
}
```

### 方式二：Stdio 桥接器接入（`scripts/xlstatus-mcp-stdio.mjs`）
适用于 **Claude Desktop** 或仅支持 `stdio` 传输的客户端（需 Node.js 20+，零第三方依赖）：

```json
{
  "mcpServers": {
    "xlstatus": {
      "command": "node",
      "args": ["/absolute/path/to/XLStatus/scripts/xlstatus-mcp-stdio.mjs"],
      "env": {
        "XLSTATUS_MCP_URL": "http://localhost:8080/mcp",
        "XLSTATUS_PAT": "xlp_your_personal_access_token"
      }
    }
  }
}
```

完整的工具参数 Schema、动态资源 URI 与 SRE 提示词说明请参阅 **[AI 与 MCP 运维指南](./docs/mcp.md)**。

---

## 源码构建与验证

### 从源码构建

```bash
# 1. 编译 Rust Server 与 Agent 生产二进制
cargo build --release --bin xlstatus-server --bin xlstatus-agent

# 2. 构建 Next.js 16 前端运维工作台
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

### 全量质量与安全验证

```bash
# Rust 格式检查、Workspace 编译检查与 830+ 项单元/集成测试
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked

# 前端 ESLint、全严格模式 TypeScript 校验、安全单元测试与生产构建
cd web
pnpm lint
pnpm typecheck
pnpm test:unit
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

仓库在 [`test-run/`](./test-run/) 目录下提供可重复执行的端到端验收脚本（如 `verify-m6-mcp.sh`、`verify-m7-ui.sh`）。部分脚本会启动本地临时服务或依赖 Docker/PostgreSQL，运行前请阅读脚本头部说明。

---

## 文档导航

| 文档 | 内容说明 |
| :--- | :--- |
| **[文档索引](./docs/README.md)** | 权威阅读顺序、部署模式约定与目录导航 |
| **[快速开始](./docs/quickstart.md)** | 最短路径在本地启动 Server、Web UI 与 Agent |
| **[AI 与 MCP 运维指南](./docs/mcp.md)** | MCP `2024-11-05` 架构、19 项工具、6 类资源、4 套 SRE 提示词与客户端接入 |
| **[安全架构与加固参考](./security.md)** | 威胁模型、三轮高权限安全加固矩阵与生产残余风险边界 |
| **[安装部署](./docs/installation.md)** | Docker Compose、源码部署、systemd 服务与 PostgreSQL 新站指南 |
| **[配置参考](./docs/configuration.md)** | `config.toml`、环境变量、CORS、TLS/mTLS 与 Agent 安全开关 |
| **[Agent 接入](./docs/agent.md)** | Ed25519 注册挑战（`xle_`）、本地单向安全锁与跨平台运行指南 |
| **[Web 前端](./docs/web.md)** | Next.js 16 工作台架构、严格类型约束与 [`web/README.md`](./web/README.md) |
| **[API 概览](./docs/api.md)** | REST（`/api/v1/*`）、实时 WebSocket（`/ws/servers`）、gRPC 与 `/mcp` 接口契约 |
| **[运维手册](./docs/operations.md)** | 健康检查、SQLite 备份恢复、SQLite 迁移 PostgreSQL 与版本升级 |
| **[故障排查](./docs/troubleshooting.md)** | 启动退出、CORS、数据库锁、证书与 Agent 断连定位指南 |
| **[开发指南](./docs/development.md)** | Workspace 模块划分、编码规范与本地测试工具链 |
| **[架构说明](./docs/architecture.md)** | 组件职责边界、数据流向与发布拓扑 |
| **[项目结构](./docs/project-structure.md)** | Crate 职责说明与仓库目录约定 |
| **[发布检查清单](./docs/release-checklist.md)** | 正式发布前的质量门禁与构建产物核对清单 |

---

## 仓库目录结构

```text
crates/
  server/       Axum HTTP/WS 服务端、Tonic gRPC 控制面、MCP 2024-11-05 引擎、SQLite/PG 仓储层
  agent/        主机遥测采集、服务拨测、进程组级 Shell、PTY 终端、安全文件操作、NAT 本地拨号
  shared/       跨端共享领域模型、DTO 与协议辅助工具
  proto-gen/    预编译 Tonic/Prost gRPC 代码绑定
  tsdb/         内置时序数据库引擎（支持窗口聚合降采样与自动过期清理）
  xtask/        Workspace 自动化辅助构建任务
web/            Next.js 16 (React 19 + Strict TypeScript) 运维工作台与公开状态页
proto/          Agent <-> Server 双向 gRPC Protobuf 协议定义
scripts/        零依赖 MCP stdio 桥接器 (xlstatus-mcp-stdio.mjs) 与 SQLite -> PostgreSQL 迁移脚本
deploy/         加固版 systemd 服务单元模板与 Linux 原子安装/回滚脚本
docs/           当前版本的安装、运维、API、MCP 与架构文档
test-run/       进程隔离的端到端验收与冒烟测试脚本
```

## 开源协议

MIT。详见 [LICENSE](./LICENSE)。
