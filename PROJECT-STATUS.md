# XLStatus 项目状态

最后整理：2026-06-19

当前发布文档入口：[docs/README.md](./docs/README.md)

## 当前结论

M0-M9 的仓库内可重复验收脚本已经保留在 `test-run/`。真实生产可用性仍需要在目标部署环境执行长时间稳定性观察，尤其是 24 小时 wall-clock soak、备份恢复演练和正式 release 包安装验证。

## 主要能力

- Server：HTTP API、WebSocket、Agent gRPC、后台监控和调度。
- Agent：注册、主机状态上报、任务执行、终端和文件相关操作。
- Web UI：Next.js 管理面板，当前默认简体中文。
- 数据库：SQLite 和 PostgreSQL，应用表由内置迁移自动创建。
- 部署：Docker Compose、源码前台运行、systemd 安装脚本。

## 验收脚本

| 脚本 | 范围 |
|---|---|
| `test-run/verify-m0.sh` | 基础启动、健康检查和页面 smoke |
| `test-run/verify-m1-pg.sh` | SQLite/PostgreSQL auth 流 |
| `test-run/verify-m2-reconnect.sh` | Agent 重连 |
| `test-run/verify-m2-revoke.sh` | Agent 撤销 |
| `test-run/verify-m3-metrics.sh` | Agent 状态上报 |
| `test-run/verify-m3-tsdb.sh` | 指标查询 |
| `test-run/verify-m3-ws.sh` | WebSocket 实时推送 |
| `test-run/verify-m4-alerts.sh` | 服务监控和告警 |
| `test-run/verify-m5-files.sh` | 文件操作 |
| `test-run/verify-m5-scheduler.sh` | 定时任务 |
| `test-run/verify-m5-task.sh` | 任务下发 |
| `test-run/verify-m5-terminal.sh` | Web Terminal |
| `test-run/verify-m6-ddns.sh` | DDNS |
| `test-run/verify-m6-mcp.sh` | MCP |
| `test-run/verify-m6-nat.sh` | NAT 反向隧道 |
| `test-run/verify-m7-ui.sh` | 前端页面静态验收 |
| `test-run/verify-m8-migrations.sh` | M8 迁移资产 |
| `test-run/verify-m8-tsdb-load.sh` | TSDB 负载与查询 bench |
| `test-run/verify-m9-install.sh` | 安装、配置、登录、enroll 和短 gRPC session |

## 发布前必须检查

详见 [docs/release-checklist.md](./docs/release-checklist.md)。

最低命令：

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace

cd web
pnpm install --frozen-lockfile
pnpm lint
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

Linux x86_64 smoke 见 [docs/operations.md](./docs/operations.md#远端-smoke)。
