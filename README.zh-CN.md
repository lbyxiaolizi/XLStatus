# XLStatus

[English](./README.md) | 简体中文

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

使用 Rust 编写的自托管服务器监控和运维系统。XLStatus 提供实时监控、服务健康检查、任务调度和自动化功能。

## ✨ 功能特性

- **实时服务器监控** - CPU、内存、磁盘、网络、负载、连接数、温度、GPU
- **服务监控** - HTTP、TCP、ICMP 健康检查，支持 SSL 证书跟踪
- **告警规则** - 灵活的告警条件和多种通知渠道
- **任务调度** - 基于 Cron 和按需执行的任务系统
- **NAT 穿透** - 通过端口转发访问内网服务
- **DDNS 集成** - 自动 DNS 更新（Cloudflare、HE、Webhook）
- **MCP 集成** - Model Context Protocol，支持 LLM 自动化
- **Web 管理面板** - 基于 React 的现代化管理界面
- **公开状态页** - 与用户分享系统状态
- **多用户 RBAC** - 基于角色的访问控制和服务器所有权

## 🚀 快速开始

### 使用 Docker Compose（推荐）

```bash
# 克隆仓库
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

# 使用 SQLite 启动
docker compose up -d

# 或使用 PostgreSQL 启动
docker compose -f docker-compose.pg.yml up -d
```

访问控制面板：http://localhost:8080

默认账号：`admin` / `admin123`

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

- [架构设计](./plan/02-architecture.md)
- [安装指南](./docs/installation.md)
- [配置说明](./docs/configuration.md)
- [API 文档](./docs/api.md)
- [Agent 设置](./docs/agent-setup.md)
- [故障排除](./docs/troubleshooting.md)

## 🛠️ 开发

### 前置要求

- Rust 1.75+
- Node.js 20+
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
cd web
npm install
npm run build
```

### 开发模式运行

```bash
# 终端 1: 启动服务器
cargo run --bin xlstatus-server

# 终端 2: 启动 Web 界面
cd web
npm run dev

# 终端 3: 启动 Agent
cargo run --bin xlstatus-agent
```

## 📦 技术栈

### 后端
- **Rust** - 系统编程语言
- **Tokio** - 异步运行时
- **Axum** - Web 框架
- **Tonic** - gRPC 框架
- **SQLx** - 数据库工具包（SQLite/PostgreSQL）

### 前端
- **Next.js 14** - React 框架
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

- 支持 100+ Agent，3 秒上报间隔
- 1000+ 服务监控，30 秒检查周期
- 查询响应时间：30 天数据 P95 < 500ms
- 通过 24 小时稳定性测试

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

- [x] M0-M7：核心功能和 Web 界面
- [ ] M8：高性能优化
- [ ] M9：生产部署和文档
- [ ] 多节点 Dashboard 集群
- [ ] Windows 和 macOS Agent 支持
- [ ] 移动应用

---

由 XLStatus 团队用 ❤️ 制作
