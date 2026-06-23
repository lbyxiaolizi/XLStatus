# XLStatus 文档

这里是当前发布版本的权威文档入口。`docs/` 只保留安装、配置、使用、运维、排障和开发所需内容；历史规划材料归档在 `docs/archive/`，不作为当前用户阅读路径。

## 推荐阅读顺序

1. [快速开始](./quickstart.md)：用 Docker Compose 或源码在本地跑起来。
2. [安装部署](./installation.md)：源码构建、systemd、PostgreSQL 新站和远端 Linux 验证。
3. [配置参考](./configuration.md)：`config.toml`、环境变量、CORS、SQLite/PostgreSQL、Web i18n。
4. [Web 前端](./web.md)：Next.js 构建、运行、API 地址和 CORS 配合。
5. [Agent 接入](./agent.md)：注册、运行、systemd 和常见问题。
6. [运维手册](./operations.md)：健康检查、日志、备份、升级和生产运行。
7. [故障排查](./troubleshooting.md)：服务直接退出、端口冲突、数据库、CORS、Agent。
8. [架构说明](./architecture.md)：当前组件、数据边界和发布拓扑。
9. [项目结构](./project-structure.md)：仓库目录、Rust workspace、Web 和 docs 约定。

## 文档目录

| 文档 | 内容 |
|---|---|
| [quickstart.md](./quickstart.md) | 最短路径启动 Server、Web UI 和 Agent |
| [installation.md](./installation.md) | 安装方式、源码构建、systemd、PostgreSQL 新站 |
| [configuration.md](./configuration.md) | 配置加载规则、环境变量、`config.toml`、数据库和 CORS |
| [web.md](./web.md) | 前端构建、运行、i18n 和部署注意事项 |
| [agent.md](./agent.md) | Agent 注册、运行和服务安装 |
| [api.md](./api.md) | 当前 HTTP、WebSocket、gRPC、MCP 接口概览 |
| [operations.md](./operations.md) | 运维命令、备份恢复、升级、远端 smoke |
| [troubleshooting.md](./troubleshooting.md) | 常见故障定位和修复 |
| [development.md](./development.md) | 本地开发、测试脚本和代码结构 |
| [release-checklist.md](./release-checklist.md) | 发布前检查清单 |
| [architecture.md](./architecture.md) | 当前运行架构和发布拓扑 |
| [project-structure.md](./project-structure.md) | 仓库结构和维护约定 |

## 当前状态

- Server：Rust、Axum、Tonic，提供 HTTP API、WebSocket 和 Agent gRPC 服务。
- Web UI：Next.js，当前语言为简体中文，i18n 配置位于 `web/lib/i18n.ts`。
- 数据库：SQLite 和 PostgreSQL，应用表由内置迁移自动创建。
- 发布方式：Docker Compose、本地源码运行、systemd 安装脚本、GitHub Release 资产。
- 平台：GitHub Release 构建 Linux、Windows、macOS、FreeBSD 多平台二进制；Server 和 Agent 的 systemd 安装脚本当前支持 Linux x86_64/arm64/i386。
- 当前 Release 安装 fallback 版本：`v0.1`；后台 Agent 安装页默认从 GitHub Releases 获取最新非草稿版本。

## 重要约定

- `DATABASE_URL` 和 `CONFIG_FILE` 不会合并。设置 `DATABASE_URL` 后，服务端进入环境变量配置模式并忽略 TOML 文件。
- Web UI 的 `NEXT_PUBLIC_API_URL` 只告诉浏览器 API 地址；后端仍需要通过 `CORS_ALLOWED_ORIGINS` 或 `server.cors_allowed_origins` 放行 Web UI 的浏览器来源。
- Server 前台运行时应该持续占用终端。用于 smoke test 时可配合 `timeout 8s`，退出码 `124` 表示服务持续运行到 timeout。
- SQLite 新建数据库需要 `?mode=rwc` 或 `create_if_missing = true`。非交互环境不会静默创建未授权的数据库文件。
