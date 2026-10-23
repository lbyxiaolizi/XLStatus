# XLStatus 自动化测试与端到端验收指南 (`TESTING.md`)

本文档是 **XLStatus `v0.2`** 自动化测试体系的权威指南，涵盖 **849+ 个单元与集成测试用例**（Rust Workspace、Next.js 前端安全与组件逻辑、MCP Stdio 桥接、Python 数据库迁移工具）以及 `test-run/` 目录下的 **21 个里程碑端到端 (E2E) 与安全回归验收脚本**。

---

## 1. 测试体系全景概览

| 测试层级 | 数量规模 | 运行命令 | 覆盖范围 |
|---|---|---|---|
| **Rust 单元与集成测试** (`crates/*`) | **825+** 个测试 | `cargo test --workspace --locked` | HTTP API、RBAC/PAT/TOTP/OAuth2 认证、MCP `2024-11-05` 引擎、gRPC 会话与 `IoStream`、告警与探针调度、DDNS、NAT 穿透、HKDF/AES-256-GCM 密钥加密、启动凭据脱敏、双数据库迁移与仓储契约 |
| **前端与 MCP 桥接单元测试** (`web/`) | **16+** 个测试组 | `cd web && pnpm test:unit` | 开放重定向拦截、URL 路径防穿越、CSS 变量防注入、敏感令牌掩码/脱敏、MCP Stdio 1 MiB 有界行读取器、嵌套弹窗滚动锁 |
| **Python 迁移工具测试** (`scripts/tests/`) | **8** 个单元测试 | `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` | SQLite -> PostgreSQL 标识符校验、表白名单过滤、流式分批读取、事务级 `TRUNCATE` 回滚、敏感凭据脱敏 |
| **里程碑与安全验收脚本** (`test-run/*.sh`) | **21** 个脚本 | `bash test-run/verify-*.sh` | M0–M9 真实进程端到端联调、进程组隔离清理安全验证、systemd 安装器升级保留与故障自动回滚验证 |

---

## 2. 运行单元与集成测试 (`849+` Tests)

### 2.1 Rust Workspace 测试（SQLite 默认 + PostgreSQL 专项）

```bash
# 运行全量 Rust 单元与集成测试（默认使用内存/临时 SQLite）
cargo test --workspace --locked

# 单独运行启动错误凭据脱敏集成测试
cargo test --test startup_secret_redaction --locked

# 启用 PostgreSQL 门控测试（验证 PG 迁移幂等性、列宽契约与 INT4/INT8 解码一致性）
XLSTATUS_TEST_PG_URL="postgres://xlstatus:xlstatus@127.0.0.1:5432/xlstatus_test" \
  cargo test --workspace --locked
```

### 2.2 Web 前端与 MCP Stdio 桥接单元测试

使用 Node.js 原生测试运行器执行 `web/app/lib/*.test.mts`：

```bash
cd web
pnpm test:unit
# 等价于: node --test app/lib/*.test.mts
pnpm lint
pnpm typecheck
cd ..
```

### 2.3 SQLite -> PostgreSQL 迁移工具单元测试

```bash
python3 -m unittest discover -s scripts/tests -p 'test_*.py' -v
```

---

## 3. 端到端与安全验收脚本参考 (`test-run/`)

`test-run/` 中的所有真实服务验收脚本均通过 [`test-run/lib/test-processes.sh`](./lib/test-processes.sh) 启动带独立进程组与唯一令牌 (`_XLSTATUS_TEST_PROCESS_TOKEN`) 的受控子进程树，绝不使用宽泛的 `pkill -f` 或 `pgrep -f`，确保不会误杀宿主机上的其它开发或生产进程。

### 3.1 安全回归脚本（无需外部网络，CI 必跑）

| 脚本路径 | 验证目标与通过标准 |
|---|---|
| `test-run/verify-process-cleanup-safety.sh` | **进程清理隔离性验证**：静态扫描全仓脚本确保不存在 `pkill -f` / `pgrep -f` / `kill_port`；启动同名对照进程与忽略 `SIGTERM` 的顽固孙进程，验证 `test_process_cleanup` 仅精确终止受管进程组且不漏杀孙进程。 |
| `test-run/verify-install-upgrade-safety.sh` | **安装器升级保护与回滚验证**：通过 Mock `systemctl` 验证 `deploy/install.sh` 与 `deploy/install-agent.sh` 在升级时默认保留已有配置与符号链接、对齐 systemd 安全沙箱指令（`NoNewPrivileges`、`ProtectSystem=strict` 等），并在健康检查或注册失败时 100% 原子恢复旧二进制、配置、单元文件与服务状态。 |

```bash
bash test-run/verify-process-cleanup-safety.sh
bash test-run/verify-install-upgrade-safety.sh
```

### 3.2 M0–M9 里程碑端到端验收脚本一览表

每个脚本均使用独立的临时目录（`/tmp/xls-*`）与非冲突高端口，支持通过环境变量覆盖 `HTTP_PORT`、`GRPC_PORT`、`LOG_DIR`：

| 脚本名称 | 默认端口 (`HTTP` / `gRPC`) | 核心端到端验收场景 |
|---|---|---|
| `verify-m0.sh` | `18091` / `15061` (Web `3017`) | **M0 基础连通性**：编译并启动 `xlstatus-server` 与 Next.js `web`；验证 `/healthz` 返回 `200`、`grpcurl` 反射列出 `AgentService` 与 `NatTunnel`、Web 首页 `/` 307 跳转 `/status` 渲染成功。 |
| `verify-m1-pg.sh` | `18091` / `15061` (PG `55432`) | **M1 双数据库一致性**：自动拉起临时 `postgres:15` 容器并跑通迁移、管理员种子化、正确密码登录 (`200`) 与错误密码拦截 (`401`)，再在 SQLite 上执行完全相同的认证流验证行为一致。 |
| `verify-m2-reconnect.sh` | `18095` / `15065` | **M2 Agent 断线自动重连**：Agent 注册并连接后终止 Server，断言 Agent 进入指数退避重连循环 (`reconnecting in`)；重启 Server 后断言 Agent 自动恢复会话并刷新 `last_seen_at`。 |
| `verify-m2-revoke.sh` | `18094` / `15064` | **M2 管理员吊销 Agent**：Agent 在线时调用 `POST /api/v1/agents/:id/revoke`，验证 Server 通过 gRPC 双向流实时推送 `force_disconnect`，Agent 立即主动断开并干净退出。 |
| `verify-m3-metrics.sh` | `18091` / `15061` | **M3 主机信息与状态持久化**：验证 Agent 上报 `HostInfo` 与 `HostState` 后，数据库 `agents.last_info_json`、`last_state_json` 与 `last_seen_at` 正确落盘。 |
| `verify-m3-tsdb.sh` | `18093` / `15063` | **M3 时序指标写入与查询**：验证 Agent 指标同时写入内存 `MetricStore`，并通过 `GET /api/v1/servers` 与 `GET /api/v1/servers/:id/metrics?range=1d` 返回实时样本序列。 |
| `verify-m3-ws.sh` | `18096` / `15066` | **M3 实时 WebSocket 推流**：携带会话 Cookie 订阅 `/ws/servers`，验证首帧接收 `snapshot` 快照帧，随后持续接收含 `cpu_percent` 的实时 `event` 增量帧。 |
| `verify-m4-alerts.sh` | `18097` / `15067` | **M4 服务探测、TLS 证书与告警通知**：验证 HTTP/TCP/ICMP 探针调度、HTTPS 证书有效期提取、服务宕机 (`service_down`) 与恢复通知投递，以及主机 CPU 阈值告警 Webhook 触发。 |
| `verify-m5-files.sh` | `18112` / `15082` | **M5 远程文件管理与临时传输 URL**：在 `file_allowed_roots` 白名单下验证远程目录列表、文件读写/删除，以及一次性临时下载/上传 URL 的正常传输与重放拦截。 |
| `verify-m5-task.sh` | `18098` / `15068` | **M5 即时远程任务分发**：通过 `POST /api/v1/tasks/:id/run` 向在线 Agent 分发 Shell 任务，验证 `TaskResult` 回调及 `GET /api/v1/tasks/:id/runs` 状态与 `stdout` 落盘。 |
| `verify-m5-scheduler.sh` | `18108` / `15078` | **M5 Cron 定时任务调度器**：配置定时 Shell 任务，等待服务端 `TaskScheduler` 自动触发派发并验证 `task_runs` 生成成功执行记录。 |
| `verify-m5-terminal.sh` | `18102` / `15072` | **M5 Web 交互式终端**：创建终端会话并连接 `/ws/terminal/:session_id`，通过 WebSocket 发送 `echo ok` 并断言从 Agent PTY `IoStream` 收到回显输出。 |
| `verify-m6-ddns.sh` | `18099` / `15069` | **M6 动态域名解析 (DDNS)**：验证 Agent 公网 IP 汇报自动触发 DDNS Webhook 更新，并在 `ddns_history` 中记录成功历史。 |
| `verify-m6-mcp.sh` | `18100` / `15070` | **M6 MCP `2024-11-05` 与 PAT 权限边界**：验证 `/mcp` (JSON-RPC 2.0 `initialize`、`tools/list`、`tools/call`) 与 `/api/v1/mcp/execute` 强制拒绝仅 Cookie 访问 (`403`)；验证 PAT Scope 过滤、`server.exec`、`fs.read/list/download_url`、PAT 写操作拦截 (`fs.write` 返回 `403`) 以及一次性下载令牌防重放 (`403`)。 |
| `verify-m6-nat.sh` | `18103` / `15073` | **M6 NAT 反向隧道端口转发**：启动模拟内网 HTTP 服务，通过 API 创建 NAT 端口映射，验证访问 Server 公网 NAT 端口时流量经 Agent `IoStream` 隧道正确转发至内网服务。 |
| `verify-m7-ui.sh` | 无（静态与契约验证） | **M7 前端完整性与可访问性验证**：运行 `pnpm lint`，核验所有仪表盘页面与公开状态页、API 客户端绑定、管理员/成员导航视图、弹窗焦点陷阱 (`useDialogFocusTrap`) 与嵌套滚动锁 (`createBodyScrollLockManager`)。 |
| `verify-m8-migrations.sh` | 无（SQL 静态校验） | **M8 高吞吐迁移产物验证**：核验 PostgreSQL 与 SQLite 的 `008_m8_performance.sql` 包含高 IO 表分区管理函数、保留期策略表与批量插入函数 (`xlstatus_insert_*_batch`)。 |
| `verify-m8-tsdb-load.sh` | 无（`xtask` 压测） | **M8 TSDB 负载与查询基准**：运行 `cargo test -p xlstatus-tsdb`、通过 `xtask mock-agents` 模拟 100 节点 24 小时共 `2,880,000` 条样本，并运行 `xtask query-bench` 验证 `1d,7d,30d` 查询 `P95 <= 500ms`。 |
| `verify-m9-install.sh` | `18099` / `15099` | **M9 发布与安装全链路冒烟**：串联运行 `verify-install-upgrade-safety.sh`、验证三套 `docker-compose*.yml` 配置语法、使用 `CONFIG_FILE` 启动 Server、验证 `/healthz`、管理员登录 + CSRF 创建注册令牌、Agent 注册生成 `agent.json` 并建立稳定 gRPC 会话。 |

---

## 4. 常用验收执行示例与进程清理

### 4.1 单独运行核心验收脚本

```bash
# 1. 验证 MCP 2024-11-05 协议、PAT 授权与文件工具边界
bash test-run/verify-m6-mcp.sh

# 2. 验证前端 UI 页面、无障碍焦点管理与构建规范
bash test-run/verify-m7-ui.sh

# 3. 验证 TSDB 288 万样本模拟与 P95 查询基准
bash test-run/verify-m8-tsdb-load.sh

# 4. 验证发布级安装、Docker Compose 配置与 Agent 注册全流程
bash test-run/verify-m9-install.sh
```

### 4.2 清理后台驻留的测试进程

部分 M0–M6 脚本在打印 `PASS` 后会保留测试用的 Server / Agent 进程供人工检查日志（日志位于 `/tmp/xls-*`），并在终端末尾输出一条专属的作用域清理命令，例如：

```bash
# 复制脚本末尾打印的 cleanup 命令即可精确停止该脚本启动的子进程组：
bash -c 'source "/path/to/XLStatus/test-run/lib/test-processes.sh"; test_process_cleanup "/tmp/xls-m6-mcp/.test-processes-verify-m6-mcp.tsv"'
```
