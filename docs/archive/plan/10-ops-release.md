# 运维与发布

## 部署目标

第一版支持：

- Docker Compose 部署 Dashboard。
- Docker Compose PostgreSQL profile，用于生产推荐部署。
- Linux x86_64 Agent 二进制安装。
- systemd 托管 Dashboard 和 Agent。

后续支持：

- Linux arm64。
- Windows service。
- macOS launchd。
- MySQL/MariaDB 元数据后端评估。
- 外部高性能指标后端。

## Docker

交付文件：

- `Dockerfile.server`
- `docker-compose.yml`
- `docker-compose.pg.yml`
- `.dockerignore`

容器约定：

- Dashboard 监听 `0.0.0.0:8008`。
- 数据目录挂载到 `/data`。
- 配置文件为 `/data/config.yaml`。
- SQLite 为 `/data/xlstatus.db`。
- PostgreSQL profile 使用独立 `postgres` service 和持久化 volume。
- TSDB 为 `/data/tsdb`。

健康检查：

- `GET /healthz`：进程存活。
- `GET /readyz`：数据库、TSDB、worker 状态。

验收标准：

- `docker compose up -d` 后 Dashboard 可访问。
- 数据目录删除前没有隐式丢失配置。

## systemd

Dashboard unit：

- `xlstatus-server.service`
- 运行用户：`xlstatus`
- WorkingDirectory：`/var/lib/xlstatus`
- ExecStart：`/usr/local/bin/xlstatus-server --config /etc/xlstatus/server.yaml`
- Restart：`on-failure`

Agent unit：

- `xlstatus-agent.service`
- ExecStart：`/usr/local/bin/xlstatus-agent run --config /etc/xlstatus/agent.yaml`
- Restart：`always`

验收标准：

- 安装脚本创建用户、目录、权限和 unit。
- `systemctl status` 可查看运行状态。

## 一键安装

Dashboard：

- 检查系统和架构。
- 下载 release 包。
- 创建配置和数据目录。
- 初始化管理员密码。
- 安装 systemd unit。
- 输出访问地址和下一步。

Agent：

- 从 Dashboard 生成安装命令。
- 包含 server_url、agent_id、一次性注册 secret。
- 安装二进制和配置。
- 启动 systemd service。

失败场景：

- 架构不支持时退出。
- 下载校验失败时退出。
- systemd 不可用时提示手动运行命令。

验收标准：

- 新 Linux x86_64 机器 5 分钟内完成 Dashboard 和 Agent 接入。

## 配置

Dashboard 配置：

- `listen_host`
- `listen_port`
- `public_url`
- `install_host`
- `location`
- `force_auth`
- `web_real_ip_header`
- `agent_real_ip_header`
- `reserved_hosts`
- `enable_mcp`
- `jwt_secret`
- `database_url`
- `database.max_connections`
- `database.min_connections`
- `database.acquire_timeout_seconds`
- `database.statement_timeout_seconds`
- `tsdb.data_path`
- `tsdb.retention_days`
- `memory_limit_mb`

Agent 配置：

- 见 [04-agent.md](./04-agent.md)。

验收标准：

- 配置支持环境变量覆盖。
- secret 支持只通过环境变量注入。

## 备份与恢复

备份内容：

- SQLite 数据库或 PostgreSQL dump。
- TSDB 数据目录。
- 配置文件。
- 加密 master key。

命令：

- `xlstatus-server backup --output backup.tar.zst`
- `xlstatus-server restore --input backup.tar.zst`
- `xlstatus-server maintenance compact`
- PostgreSQL 部署下调用 `pg_dump` 或使用 SQLx 逻辑导出；恢复时先校验 schema version。

失败场景：

- 恢复前版本不兼容时拒绝并提示。
- master key 缺失时拒绝恢复敏感配置。

验收标准：

- 备份恢复后用户、服务器、监控、任务、通知、历史指标可用。

## 日志和可观测性

日志：

- JSON 日志可选。
- 默认文本日志。
- 每个请求带 request_id。
- 敏感字段脱敏。

指标：

- worker 队列长度。
- Agent 在线数。
- TSDB 写入延迟。
- 通知发送成功率。
- DDNS 更新成功率。

健康状态：

- 数据库可读写。
- PostgreSQL 连接池 active、idle、wait 指标。
- TSDB 可写。
- worker 最近心跳。
- Agent registry 统计。

验收标准：

- 出错响应包含 request_id，可在日志中定位。

## 发布流程

版本产物：

- `xlstatus-server-linux-amd64.tar.gz`
- `xlstatus-agent-linux-amd64.tar.gz`
- Docker image。
- SBOM。
- checksums。

CI：

- rustfmt。
- clippy。
- unit tests。
- integration tests。
- Next.js typecheck。
- Playwright E2E。
- Docker image build。

发布门槛：

- 所有 M6 验收通过。
- 安全测试通过。
- 24 小时长稳测试通过。
- 文档完成安装、配置、备份恢复、故障排查。
