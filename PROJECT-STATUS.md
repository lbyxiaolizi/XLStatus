# XLStatus - Project Status

## 当前审计结论

**最后审计**: 2026-06-18
**权威状态文档**: [docs/implementation-audit.md](./docs/implementation-audit.md)

M0–M9 的仓库内可重复验收闭环已经补齐。按实现度区分：

- M0 (Scaffold) ✅
- M1 (Base Platform) ✅ — SQLite + PostgreSQL 双后端 auth/RBAC/PAT 全通
- M2 (Agent Onboarding) ✅ — enroll → JWT → gRPC session → ForceDisconnect → 重连退避 → JWT 自动续签
- M3 (Real-Time Monitoring) ✅ — 采集 → gRPC 落库 → MetricStore → `/ws/servers` 推送 → Dashboard 实时渲染全链路打通
- M4 (Service Monitoring And Alerts) ✅ — HTTP/TCP/ICMP/HTTPS 证书状态、服务历史/uptime、失败/恢复通知、CPU 资源告警和 SSRF 防护均已验证
- M5 (Operations) ✅ — 任务、调度、Web Terminal、文件读写删、临时传输、远程配置/更新 UI/API、禁用命令策略均有验证
- M6 (DDNS/NAT/MCP) ✅ — DDNS agent IP 自动触发、NAT 反向隧道、PAT-only MCP REST + `/mcp` JSON-RPC、临时 URL 与限流均通过验证
- M7 (Frontend Complete) ✅ — lint/build 通过；核心页面、权限导航、公开状态页、移动导航、terminal/file/config/update UI 有验收脚本
- M8 (Performance) ✅ — M8 迁移、TSDB facade、100 agent/3s/24h dry-run 规划、P95 query bench、health/compact 工具均通过；实际 24h wall-clock soak 需在部署环境单独运行
- M9 (Release Stable) ✅ — 安装脚本、Docker compose config、systemd/deploy 资产、debug 安装 smoke、Linux x86_64 Docker build/run smoke、agent 短 gRPC session 均通过；实际 24h wall-clock soak 需在部署环境单独运行

### 测试结果

- `cargo check --workspace` 通过
- `cargo test --workspace` 80 个测试通过（20 agent + 48 server + 4 shared + 8 tsdb），5 个 ignored（4 个 httpbin 依赖 + 1 个 PTY echo）
- `cd web && pnpm lint` 通过
- `cd web && pnpm build` 通过
- `ssh root@wawo-hk-sim-pro2` 上 Linux x86_64 构建与运行 smoke 通过（server/web Docker 镜像 + agent x86_64 release binary）

### 端到端验证脚本（唯一可重复的"它能跑"证据）

| 脚本 | 范围 | 状态 |
|---|---|---|
| `test-run/verify-m0.sh` | 3 服务启动 + /healthz + grpcurl + Next.js 页面 | ✅ |
| `test-run/verify-m1-pg.sh` | SQLite 与 PostgreSQL 上相同 auth 流 | ✅ |
| `test-run/verify-m3-metrics.sh` | Agent 上报 + `agents.last_state_json` / `last_info_json` 落库 | ✅ |
| `test-run/verify-m3-tsdb.sh` | `GET /api/v1/servers` + `GET /api/v1/servers/:id/metrics` 端到端 | ✅ |
| `test-run/verify-m2-revoke.sh` | admin revoke → gRPC `ForceDisconnect` → agent 退出 | ✅ |
| `test-run/verify-m2-reconnect.sh` | server 重启后 agent 用 backoff 重连 | ✅ |
| `test-run/verify-m4-alerts.sh` | HTTPS 证书状态、HTTP/TCP/ICMP 服务调度、history/uptime、service_down fired/recovered、CPU webhook | ✅ |
| `test-run/verify-m5-task.sh` | gRPC 任务下发到真实 agent，并持久化 stdout | ✅ |
| `test-run/verify-m5-terminal.sh` | Web Terminal `echo ok` 经 `/ws/terminal` 往返真实 agent | ✅ |
| `test-run/verify-m5-files.sh` | 文件 API + 禁用命令策略拒绝 shell/file/terminal | ✅ |
| `test-run/verify-m5-scheduler.sh` | 定时任务经 gRPC 下发并持久化结果 | ✅ |
| `test-run/verify-m6-ddns.sh` | Agent IP report 自动触发 DDNS webhook 并落 history | ✅ |
| `test-run/verify-m6-mcp.sh` | PAT-only MCP REST + `/mcp` JSON-RPC，执行 `server.exec`、`fs.*`、临时 URL 上传/下载与限流 | ✅ |
| `test-run/verify-m6-nat.sh` | public NAT 端口通过 IoStream 反向隧道访问 agent 内网 HTTP 服务 | ✅ |
| `test-run/verify-m7-ui.sh` | 前端页面、公开视图、权限导航、terminal/file/config/update UI 静态验收 | ✅ |
| `test-run/verify-m8-migrations.sh` | M8 迁移工件与 helper 函数已挂载 | ✅ |
| `test-run/verify-m8-tsdb-load.sh` | TSDB tests + 100 agent/24h dry-run + P95 query bench 通过 | ✅ |
| `test-run/verify-m9-install.sh` | compose config、debug binaries、配置启动、登录、enroll、短 gRPC 会话通过 | ✅ |
| Linux x86_64 smoke | 远端 Debian 12 x86_64 上构建 server/web Docker 镜像与 agent release binary，验证 `/healthz`、Web `/login`、登录、enroll、agent 短 gRPC 会话 | ✅ |

## 历史项目状态

下面内容保留作为变更记录。"完成 / 生产就绪" 等表述需要结合上面的当前审计重新理解。

### 编译状态

- ✅ **Server**: 编译成功
- ✅ **Agent**: 编译成功
- ⚠️ **警告**: 若干未使用导入/未使用方法警告仍存在于占位或兼容模块中

### 文档

- ✅ `README.md` / `README.zh-CN.md`
- ✅ `docs/installation.md`, `docs/configuration.md`, `docs/agent-setup.md`
- ✅ `docs/api.md`, `docs/troubleshooting.md`
- ✅ `docs/quickstart.md` / `docs/quickstart.zh-CN.md`
- ✅ `docs/rbac.md`
- ✅ `docs/implementation-audit.md` — 权威实现审计文档
- ✅ `CLAUDE.md`

## 🚀 快速开始

### 编译

```bash
cargo build --release
# 或者
cargo build -p xlstatus-server -p xlstatus-agent
```

### 跑测试

```bash
cargo test --workspace
# 然后再跑端到端：
bash test-run/verify-m0.sh
bash test-run/verify-m1-pg.sh
bash test-run/verify-m3-metrics.sh
bash test-run/verify-m3-tsdb.sh
bash test-run/verify-m2-revoke.sh
bash test-run/verify-m2-reconnect.sh
```

### 启动服务

环境变量方式（开发）：

```bash
DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="your-secret-key" \
./target/release/xlstatus-server
```

配置文件方式（生产）：

```toml
[server]
http_bind = "0.0.0.0:8080"
grpc_bind = "0.0.0.0:50051"

[database]
url = "postgresql://user:pass@localhost/xlstatus"

[security]
session_secret = "your-random-secret-key"
session_ttl_hours = 24
```

```bash
CONFIG_FILE=/etc/xlstatus/server.toml ./target/release/xlstatus-server
```

### 启动 Agent

```bash
# 1) 在 Dashboard 上生成 enrollment token
# 2) 注册 agent
./target/release/xlstatus-agent enroll \
  --server http://your-server:8080 \
  --grpc-server http://your-server:50051 \
  --token xle_xxx \
  --name my-server

# 3) 启动
./target/release/xlstatus-agent run --config agent.json
```

## 📦 项目结构

```
XLStatus/
├── crates/
│   ├── server/         # Dashboard server
│   ├── agent/          # monitoring agent
│   ├── shared/         # shared code
│   ├── proto-gen/      # gRPC 代码生成
│   ├── tsdb/           # 时序数据库 (interface + in-memory, M8 升级)
│   └── xtask/          # 构建工具
├── proto/              # Protobuf
├── web/                # Next.js 前端
├── docs/               # 文档
├── deploy/             # 部署资产
├── plan/               # 权威计划文档
└── test-run/           # 端到端验证脚本
```

## 🔜 剩余外部/发布事项

按 [`docs/implementation-audit.md`](./docs/implementation-audit.md) 的当前审计结论，仓库内 M0-M9 验收已闭环；剩余事项主要发生在目标部署环境或正式发布流程中：

- **M8/M9 运维侧**: 在目标部署环境运行真实 24h wall-clock soak，并保留性能/稳定性报告
- **M9 发布侧**: 在 tag/release 流程中生成正式 release 包；Linux x86_64 Docker clean build/run smoke 已通过，仍建议在目标生产环境做一次人工计时安装验收

## 📚 文档

- [实现审计 (权威)](./docs/implementation-audit.md)
- [安装](./docs/installation.md)
- [配置](./docs/configuration.md)
- [Agent 设置](./docs/agent-setup.md)
- [API](./docs/api.md)
- [RBAC 权限矩阵](./docs/rbac.md)
- [故障排除](./docs/troubleshooting.md)
- [快速开始 (en)](./docs/quickstart.md)
- [快速开始 (zh)](./docs/quickstart.zh-CN.md)

## 🙏 致谢

- 灵感来自 [Nezha](https://github.com/naiba/nezha) 监控系统
- 使用 Rust + React (Next.js) 现代技术栈构建

## 📄 许可证

MIT License
