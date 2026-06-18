# XLStatus Documentation

本目录是 XLStatus 当前文档入口。功能完成度以 [implementation-audit.md](./implementation-audit.md) 和 [PROJECT-STATUS.md](../PROJECT-STATUS.md) 为准；旧的完成报告已经移动到 [archive/](./archive/)。

## 推荐阅读顺序

1. [实现审计](./implementation-audit.md) - 对照 `plan/` 的当前验收状态。
2. [项目状态](../PROJECT-STATUS.md) - M0-M9 总览、验证脚本和剩余风险。
3. [快速开始](./quickstart.md) / [中文快速开始](./quickstart.zh-CN.md) - 本地启动与基本操作。
4. [安装指南](./installation.md) - Docker Compose、systemd 和 agent 安装。
5. [配置说明](./configuration.md) / [中文配置说明](./configuration.zh-CN.md) - 当前二进制实际读取的配置项、`config.toml`、CORS 和数据库初始化。

## 当前文档

| 文档 | 用途 |
|---|---|
| [agent-setup.md](./agent-setup.md) | Agent 注册、运行与排障 |
| [api.md](./api.md) | HTTP API 参考 |
| [configuration.md](./configuration.md) | 服务端与 Agent 配置、CORS、数据库初始化 |
| [configuration.zh-CN.md](./configuration.zh-CN.md) | 中文配置说明 |
| [implementation-audit.md](./implementation-audit.md) | 当前实现验收审计 |
| [installation.md](./installation.md) | 安装与部署 |
| [quickstart.md](./quickstart.md) | 英文快速开始 |
| [quickstart.zh-CN.md](./quickstart.zh-CN.md) | 中文快速开始 |
| [rbac.md](./rbac.md) | RBAC 与 PAT scope |
| [troubleshooting.md](./troubleshooting.md) | 故障排查 |

## 验收与里程碑

- [milestones/](./milestones/) 保存 M0-M9 的里程碑记录。
- [performance/](./performance/) 保存 M8 性能相关说明。
- [archive/](./archive/) 保存早期状态报告、旧配置指南和历史 Docker/Linux 验证说明。

## 计划文档

`plan/` 仍是产品和技术计划的基线，尤其是：

- [路线图](../plan/08-roadmap.md)
- [测试计划](../plan/09-test-plan.md)
- [验证命令](../plan/15-verification-commands.md)

## 目录约定

- 根目录保留项目入口、构建文件、Compose 文件和当前状态摘要。
- `docs/` 放当前文档。
- `docs/archive/` 放历史报告或可能早于当前实现审计的说明。
- `test-run/` 放可重复验收脚本和本地 smoke 脚本。
