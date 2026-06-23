[English](./README.md) | 简体中文

# XLStatus

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

XLStatus 是一个用 Rust 和 Next.js 编写的自托管服务器监控与运维面板。它把主机实时指标、公开状态页、服务探测、告警、任务执行、文件操作、Web 终端、DDNS、NAT 隧道和 MCP 工具整合到一个可部署的系统里。

当前正式版本是 `v0.1`。安装、运维和继续开发请从文档索引开始：[docs/README.md](./docs/README.md)。

## 功能特性

- Agent 实时监控 CPU、内存、磁盘、网络、负载、连接数、进程数、GPU 和温度。
- HTTP、TCP、ICMP 服务监控，支持 HTTPS 证书信息、可用率历史和告警规则。
- Agent 运维能力：任务调度、命令执行、文件读写/下载/上传、Web 终端、配置下发和强制更新钩子。
- 网络工具：DDNS Provider、NAT 反向隧道、GeoIP 元数据和真实世界地图分布。
- Dashboard：中文优先的 Next.js UI、RBAC、PAT scopes、CSRF、防护边界、公开 `/status` 页面和主题设置。
- 发布方式：Docker Compose、源码构建、Linux systemd 安装脚本和多平台 GitHub Release 资产。

## 快速开始

```bash
git clone https://github.com/lbyxiaolizi/XLStatus.git
cd XLStatus
mkdir -p .secrets
printf '%s\n' 'replace-with-a-strong-initial-password' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password
docker compose up -d
curl -fsS http://localhost:8080/healthz
```

访问：

- Web UI：`http://localhost:3000`
- API：`http://localhost:8080`
- 公开状态页：`http://localhost:3000/status`

Docker Compose 默认把 Agent gRPC 发布到 `0.0.0.0:50051`，远端 Agent
无需逐个添加来源 IP 白名单即可接入。生产环境仍建议让 `8080` 和 `3000`
只经本机或反向代理访问。

首次启动前请在 `.secrets/xlstatus_seed_admin_password` 写入强初始密码。

PostgreSQL 版本：

```bash
docker compose -f docker-compose.pg.yml up -d
```

## Release 安装脚本

Server：

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1/install-server.sh | sudo bash
```

Agent：

```bash
sudo SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=https://grpc.dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1/install-agent.sh | bash'
```

后台设置页可以生成带参数的 Agent bootstrap 链接。默认情况下会从 GitHub Releases 获取最新非草稿版本；如果 GitHub 不可用，则回退到 `v0.1`。

## 文档

- [文档索引](./docs/README.md)
- [快速开始](./docs/quickstart.md)
- [安装部署](./docs/installation.md)
- [配置参考](./docs/configuration.md)
- [Agent 接入](./docs/agent.md)
- [Web 前端](./docs/web.md)
- [API 概览](./docs/api.md)
- [运维手册](./docs/operations.md)
- [故障排查](./docs/troubleshooting.md)
- [开发指南](./docs/development.md)
- [发布检查清单](./docs/release-checklist.md)
- [架构说明](./docs/architecture.md)
- [项目结构](./docs/project-structure.md)

历史规划材料已归档到 [docs/archive](./docs/archive/)，不再作为当前用户文档路径。

## 源码构建

```bash
cargo build --release --bin xlstatus-server --bin xlstatus-agent

corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

Release 二进制可以通过环境变量或 `CONFIG_FILE` 运行；完整规则见 [docs/configuration.md](./docs/configuration.md)。

## 验证

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace

cd web
pnpm lint
pnpm typecheck
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

仓库还保留了 `test-run/` 下的验收脚本。部分脚本会启动本地服务、依赖 Docker/PostgreSQL 或占用固定端口，运行前请先阅读脚本头部说明。

## 仓库结构

```text
crates/       Rust workspace：server、agent、shared、proto-gen、TSDB 和 xtask
web/          Next.js Dashboard 和公开状态页
proto/        gRPC protobuf 定义
deploy/       systemd 模板和 Linux 安装脚本
docs/         当前文档和历史规划归档
test-run/     可重复验收和 smoke 脚本
```

## 协议

MIT。详见 [LICENSE](./LICENSE)。
