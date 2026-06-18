# XLStatus

[English](./README.md) | 简体中文

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

使用 Rust 编写的自托管服务器监控和运维系统。XLStatus 提供实时监控、服务健康检查、任务调度和自动化功能。

## 当前状态

XLStatus 仍处于开发中。M0-M9 已有 `test-run/` 下的可重复验收覆盖；真实 24 小时长稳测试仍需在目标部署环境执行。

部署或继续开发前，请从当前文档索引开始：[docs/README.md](./docs/README.md)。

## ✨ 功能特性

- **实时服务器监控** - Agent 上报 CPU、内存、磁盘、网络、负载、连接数和温度数据
- **服务监控** - HTTP、TCP、ICMP 健康检查，并跟踪 HTTPS 证书指纹和过期时间
- **告警规则** - 资源、离线、服务状态、延迟、恢复和 webhook 通知流程
- **任务调度** - Cron 和按需任务通过在线 Agent 执行
- **NAT 穿透** - 通过反向隧道访问内网服务
- **DDNS 集成** - 支持 Cloudflare、腾讯云、HE、Webhook 和 Dummy Provider
- **MCP 集成** - 支持 MCP REST 兼容接口与 `/mcp` JSON-RPC 工具
- **Web 管理面板** - Next.js 管理服务器、服务、告警、任务、DDNS、NAT、Terminal 和设置
- **公开状态页** - 未登录即可访问 `/status`，数据来自 `/api/v1/public/status`
- **BOLD 主题 UI** - BOLD. 风格新粗野主义配色，支持显式浅色/深色切换
- **简体中文界面** - Web UI 默认 `zh-CN`，i18n 配置集中在 `web/lib/i18n.ts`
- **多用户 RBAC** - 角色、PAT scope、CSRF 和服务器 allowlist

## 🚀 快速开始

### 使用 Docker Compose（推荐）

Dockerfile 和 Compose 文件已由 M9 smoke 脚本做配置校验。建议先用于本地开发和测试，生产前仍需执行目标环境的 24 小时长稳验证。

```bash
# 克隆仓库
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

# 使用 SQLite 启动
docker compose up -d

# 或使用 PostgreSQL 启动
docker compose -f docker-compose.pg.yml up -d
```

访问：

- Web UI：http://localhost:3000
- API：http://localhost:8080

默认账号：`admin` / `admin123`

公开状态页登录前即可访问：`http://localhost:3000/status`。

SQLite Compose 首次启动会创建 `./data/xlstatus.db`。PostgreSQL Compose 会在空 volume 上创建 `xlstatus` 用户和数据库，应用表由 XLStatus 自动迁移。
Compose 已设置 `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000`，因此 Web UI 可以访问 `http://localhost:8080` 上的 API。

从源码直接运行 SQLite 时，推荐保留 `?mode=rwc` 或设置 `DATABASE_CREATE_IF_MISSING=true`；如果数据库文件不存在且未允许自动创建，交互式运行会询问是否新建，非交互运行会报错退出。PostgreSQL 新站初始化步骤见 [安装部署](./docs/installation.md#postgresql-新站)。

服务端前台运行时应该一直占用终端。用于 smoke test 时可以配合 `timeout 8s`；退出码 `124` 表示服务持续运行到超时，直接返回 shell 则应查看终端中的 `Error:` 或 systemd 日志，常见原因是 `8080`/`50051` 端口占用。

### 使用安装脚本

**注意**：暂无预编译二进制文件，需要先从源码构建。

```bash
# 从源码构建
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus
cargo build --release

# 安装服务器
sudo BINARY_PATH=target/release/xlstatus-server bash deploy/install.sh

# 在被监控服务器上安装 Agent
sudo BINARY_PATH=target/release/xlstatus-agent bash deploy/install-agent.sh
```

## 📚 文档

- [文档索引](./docs/README.md)
- [架构设计](./plan/02-architecture.md)
- [安装部署](./docs/installation.md)
- [配置参考](./docs/configuration.md)
- [API 文档](./docs/api.md)
- [Agent 接入](./docs/agent.md)
- [运维手册](./docs/operations.md)
- [故障排查](./docs/troubleshooting.md)

## ⚙️ 配置要点

服务端支持两种配置模式：

- 环境变量模式：设置 `DATABASE_URL` 后，再设置 `HTTP_BIND`、`GRPC_BIND`、`CORS_ALLOWED_ORIGINS`、`SESSION_SECRET` 等变量。
- TOML 文件模式：复制 [config.example.toml](./config.example.toml) 为 `config.toml` 或 `/etc/xlstatus/server.toml`，然后使用 `CONFIG_FILE=/path/to/server.toml` 启动。

使用 `CONFIG_FILE` 时不要同时设置 `DATABASE_URL`；一旦设置 `DATABASE_URL`，服务端会进入环境变量模式并忽略 TOML 文件。

当 Web UI 和 API 不同源时，API 必须允许 Web UI 的浏览器源：

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

Web UI 通过 `NEXT_PUBLIC_API_URL` 知道 API 地址：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

远端 Docker Compose 部署时，推荐复制 `.env.example` 为 `.env`：

```env
XLSTATUS_PUBLIC_API_URL=http://example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://example.com:3000,http://localhost:3000,http://127.0.0.1:3000
```

然后执行：

```bash
docker compose up -d --build
```

`NEXT_PUBLIC_API_URL` 会写入前端构建产物；如果改了 API 地址，必须重新 build Web 镜像。

完整配置矩阵、SQLite 新建数据库行为和 PostgreSQL 新站初始化见 [docs/configuration.md](./docs/configuration.md)。

## 🛠️ 开发

### 前置要求

- Rust 1.75+
- Node.js 20+，并启用 Corepack/pnpm
- PostgreSQL 15+ 或 SQLite 3.40+

### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

# 构建服务器
cargo build --release --bin xlstatus-server

# 构建 Agent
cargo build --release --bin xlstatus-agent

# 构建 Web 界面
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

### 开发模式运行

```bash
# 终端 1: 启动服务器
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000 cargo run --bin xlstatus-server

# 终端 2: 启动 Web 界面
cd web
corepack enable
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev

# 终端 3: 启动 Agent
cargo run --bin xlstatus-agent
```

如果 Web UI 使用其他端口，请在启动服务端前把精确来源加入 `CORS_ALLOWED_ORIGINS`。

如果要用源码方式运行接近生产的前端，先启动 Rust Server，再执行：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

## 📦 技术栈

### 后端
- **Rust** - 系统编程语言
- **Tokio** - 异步运行时
- **Axum** - Web 框架
- **Tonic** - gRPC 框架
- **SQLx** - 数据库工具包（SQLite/PostgreSQL）

### 前端
- **Next.js 16** - React 框架
- **TypeScript** - 类型安全
- **Tailwind CSS** - 实用优先的 CSS 框架

### 基础设施
- **gRPC** - Agent 通信
- **WebSocket** - 实时更新
- **Docker** - 容器化

## 🏗️ 项目结构

```
XLStatus/
├── crates/
│   ├── server/          # 控制面板服务器
│   ├── agent/           # 监控 Agent
│   ├── shared/          # 共享类型和工具
│   ├── proto-gen/       # 生成的 protobuf 代码
│   └── tsdb/            # 时序数据库
├── web/                 # Next.js Web 界面
├── proto/               # Protobuf 定义
├── deploy/              # 部署脚本和配置
└── docs/                # 文档
```

## 🔒 安全

- Argon2 密码哈希
- Ed25519 Agent 认证
- 基于 JWT 的会话
- CSRF 保护
- 审计日志
- 速率限制

## 📊 性能

- 已验证 dry-run 负载计划：100 Agent，3 秒上报间隔，24 小时时间窗口
- 计划目标：支持 1000+ 服务监控，30 秒检查周期
- 计划目标：30 天数据查询 P95 < 500ms
- 真实 wall-clock 24 小时稳定性仍需在目标部署环境执行

## 🤝 贡献

欢迎贡献！请先阅读我们的[贡献指南](CONTRIBUTING.md)。

## 📝 开源协议

本项目采用 MIT 协议 - 详见 [LICENSE](LICENSE) 文件。

## 🙏 致谢

- 灵感来自 Nezha 监控系统
- 使用现代 Rust 和 React 生态系统构建
- 社区反馈和贡献

## 📞 支持

- 📧 邮箱：support@xlstatus.io
- 💬 Discord：https://discord.gg/xlstatus
- 🐛 问题反馈：https://github.com/yourusername/xlstatus/issues

## 🗺️ 路线图

- [x] M0-M9：脚手架、基础平台、Agent 接入、实时监控、服务监控与告警、运维、DDNS/NAT/MCP、前端、高性能工具和发布 smoke 均有可重复验收脚本覆盖
- [ ] 多节点 Dashboard 集群
- [ ] Windows 和 macOS Agent 支持
- [ ] 移动应用

---

由 XLStatus 团队用 ❤️ 制作
