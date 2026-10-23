# 项目结构说明 (Project Structure Reference)

本文档完整描述 **XLStatus** 代码仓库的目录层次、Rust Workspace 各 Crate 模块划分、Next.js 16 前端架构、Protobuf 协议文件、运维脚本与自动化验收体系。

---

## 1. 仓库顶层目录概览 (Top-Level Workspace Layout)

```text
XLStatus/
├── .github/workflows/            # GitHub Actions CI、跨平台构建与 Release 发布流水线
├── crates/                       # Rust Workspace 核心后端与 Agent 源码 (6 个 Crates)
├── deploy/                       # systemd 服务单元模板与 Server/Agent 一键安装脚本
├── docs/                         # 权威技术与运维文档库 (README / mcp / architecture / operations 等)
├── proto/                        # gRPC / Protobuf 跨端协议定义 (proto/xlstatus/v1/*.proto)
├── scripts/                      # MCP Stdio 桥接器与 SQLite -> PostgreSQL 迁移工具
├── test-run/                     # M0–M9 端到端验收脚本与进程监管测试套件
├── web/                          # Next.js 16 (App Router) 运维工作台与公开状态页
├── Cargo.toml                    # Rust Workspace 根清单与统一依赖版本锁
├── Cargo.lock                    # Rust 依赖精确锁定文件
├── config.example.toml           # Server 配置文件完整注释示例
├── .env.example                  # Docker Compose 与环境变量配置示例
├── docker-compose.yml            # 默认 SQLite 生产编排 (Server + Web)
├── docker-compose.pg.yml         # PostgreSQL 生产编排 (Postgres + Server + Web)
├── docker-compose.simple.yml     # 轻量级本地验证编排
├── Dockerfile.server             # xlstatus-server 容器镜像构建文件
├── Dockerfile.agent              # xlstatus-agent 容器镜像构建文件
├── security.md                   # 已验证的安全修复记录与残余风险边界基线
├── README.md                     # 英文项目主入口
└── README.zh-CN.md               # 中文项目主入口
```

---

## 2. Rust Workspace 模块详解 (`crates/`)

Rust 工作区由 6 个职责单一的 Crate 组成：

```text
crates/
├── server/       # 控制面核心服务 (HTTP REST / WebSocket / gRPC / MCP / 自治后台引擎)
├── agent/        # 受控主机守护进程 (Enrollment / 指标采集 / 任务执行 / PTY 终端 / NAT 转发)
├── tsdb/         # 时序指标存储门面 (MetricStore，支持多时间窗采样查询与聚合)
├── shared/       # Server 与 Agent 共享的强类型 ID、RBAC 权限模型、NAT/DDNS/Task/Terminal 契约
├── proto-gen/    # 编译 proto/xlstatus/v1/*.proto 并导出 Tonic/Prost 生成代码
└── xtask/        # 研发辅助与构建验证工具任务
```

### 2.1 `crates/server` — 控制面核心服务

```text
crates/server/
├── Cargo.toml
├── migrations/
│   ├── sqlite/                   # SQLite 内置自动迁移 SQL 脚本
│   └── postgres/                 # PostgreSQL 内置自动迁移 SQL 脚本
├── tests/
│   └── startup_secret_redaction.rs # 启动日志与敏感配置脱敏回归测试
└── src/
    ├── main.rs                   # 进程入口、CLI 解析、数据库迁移、gRPC/HTTP 双栈启动
    ├── config.rs                 # 配置加载器（环境变量模式 vs config.toml 互斥解析）
    ├── security.rs               # 出站 URL SSRF 校验、DNS Pinning、WebSocket Origin 校验
    ├── secrets.rs                # 敏感凭据加密存储与脱敏工具
    ├── api/
    │   ├── mod.rs                # 路由树挂载、CORS、请求体/路径长度门禁与中间件链
    │   ├── types.rs              # 统一 ApiResponse<T> 响应信封与分页结构
    │   └── v1/
    │       ├── mod.rs            # /api/v1 子路由聚合
    │       ├── auth.rs           # 登录/登出、Cookie 会话、TOTP 二次验证、WAF 封禁管理
    │       ├── oauth.rs          # OAuth2 / OIDC 登录、账号绑定/解绑与 Callback 校验
    │       ├── users.rs          # 用户 CRUD 与角色分配（集成于 auth/user 路由）
    │       ├── pat.rs            # Personal Access Token (xlp_) 签发、Scope 与 server_ids 绑定
    │       ├── agent.rs          # Enrollment Token (xle_) 管理与 Agent 首次注册接口
    │       ├── agent_jwt.rs      # Agent Ed25519 Challenge-Response 与短期 JWT 签发
    │       ├── agent_json.rs     # Agent 遥测 JSON 与 Dashboard 元数据安全解析器
    │       ├── servers.rs        # 服务器列表、详情、分组、标签与公开可见性配置
    │       ├── server_ops.rs     # 单机即席命令执行、强制升级与维护操作
    │       ├── terminal.rs       # Web 交互式终端 WebSocket 升级与 IoRegistry 桥接
    │       ├── transfers.rs      # 远端文件列表/读写/删除及一次性临时上传下载 URL (xlt_)
    │       ├── services.rs       # HTTP / TCP / ICMP 服务拨测监控项 CRUD
    │       ├── service_history.rs# 服务拨测 SLA 统计与历史探测记录查询
    │       ├── alerts.rs         # 告警规则与告警事件历史 API
    │       ├── notifications.rs  # Webhook 通知渠道与通知组 CRUD 及连通性测试
    │       ├── tasks.rs          # 定时 Cron 任务与即席批量任务编排 API
    │       ├── nat.rs            # NAT 反向隧道端口映射 CRUD 与源 IP 白名单校验
    │       ├── ddns.rs           # 动态域名解析 (DDNS) 配置管理与状态同步查询
    │       ├── cloudflared.rs    # Cloudflared 隧道辅助集成接口
    │       ├── geoip.rs          # 节点 GeoIP 地理位置解析与缓存接口
    │       ├── mcp.rs            # MCP 2024-11-05 JSON-RPC 网关 (/mcp)、Web Studio (/api/v1/mcp/studio) 与 Catalog
    │       ├── ws.rs             # /api/v1/ws 多频道实时事件订阅 WebSocket
    │       ├── public.rs         # 匿名公开状态页 (/api/v1/public/status)、公开节点详情与 MJPEG 流
    │       ├── settings.rs       # 站点全局设置、公开页开关与审计日志查询
    │       ├── themes.rs         # 状态页主题配置管理
    │       ├── maintenance.rs    # 数据库维护、状态清理与系统诊断接口
    │       └── openapi.rs        # OpenAPI 规范元数据导出
    ├── mcp/                      # AI-Native MCP 核心引擎模块
    │   ├── mod.rs                # MCP 模块导出
    │   ├── tools.rs              # 19 项 MCP Tools、5 项静态资源、1 项动态资源模板、4 项 SRE Prompts 定义
    │   └── executor.rs           # McpExecutor：统一执行工具、6 套只读诊断剧本、xlstatus:// 资源与动态提示词
    ├── grpc/
    │   ├── mod.rs                # Tonic AgentService 实现 (Session 控制流与 IoStream 数据流)
    │   └── session.rs            # SessionRegistry、IoRegistry (含 expected_agent_id 归属校验) 与 TaskResponseRegistry
    ├── realtime/
    │   ├── mod.rs                # BroadcastHub 进程内实时事件总线与最新快照缓存
    │   └── ws.rs                 # /ws/servers 实时服务器遥测 WebSocket 路由（首帧 Snapshot + 增量 Event）
    ├── services/
    │   ├── mod.rs                # 服务监控模块入口
    │   ├── monitor.rs            # ServiceMonitor 后台调度循环（支持 local 与 agent 分布式拨测）
    │   └── probe.rs              # 本地 HTTP / TCP / ICMP 拨测执行器与 TLS 证书有效期提取
    ├── alerts/
    │   ├── mod.rs                # 告警模块入口
    │   └── engine.rs             # AlertEngine 多条件规则评估、恢复检测、通知触发与故障自愈任务联动
    ├── notifications/
    │   ├── mod.rs                # 通知模块入口
    │   ├── policy.rs             # 通知渠道安全策略校验
    │   └── sender.rs             # NotificationSender (RFC 8259 JSON 转义、Form 编码模板渲染与防 SSRF 发送)
    ├── tasks/
    │   ├── mod.rs                # 任务模块入口与告警触发任务派发器
    │   └── scheduler.rs          # TaskScheduler 定时 Cron 调度器与 ServerSelector 目标解析
    ├── nat/
    │   ├── mod.rs                # NAT 模块入口
    │   ├── manager.rs            # NAT 管理器门面导出 (NatTunnelManager、端口保留校验与帧大小常量)
    │   ├── policy.rs             # NAT 端口范围、控制面保留端口与 Allowed Sources CIDR 策略校验
    │   └── tunnel.rs             # NatTunnelManager 公网 TCP 监听器生命周期与 IoRegistry 反向隧道桥接
    ├── ddns/
    │   ├── mod.rs                # DDNS 模块入口
    │   ├── manager.rs            # DdnsManager 后台 IP 变更检测与 DNS 记录同步循环
    │   ├── policy.rs             # DDNS 域名与 Provider 参数校验
    │   └── provider.rs           # Cloudflare 等 DNS 服务商 API 驱动实现
    ├── auth/
    │   ├── mod.rs                # 认证辅助函数（密码哈希、临时传输令牌生成）
    │   ├── middleware.rs         # AuthUser / AuthSession 提取器（统一兼容 Cookie Session 与 Bearer PAT）
    │   ├── session.rs            # 服务端会话管理与 CSRF Token 派生
    │   ├── jwt.rs                # Agent 短期 JWT 签发与验签
    │   ├── rbac.rs               # 基于角色 (Admin/Member/Viewer) 与細粒度 Scope 的权限判定
    │   └── totp.rs               # RFC 6238 TOTP 密钥生成、二维码 URI 与动态口令验证
    └── db/
        ├── mod.rs                # DatabaseBackend (Sqlite / Postgres) 双后端连接池与迁移器
        ├── macros.rs             # 跨 SQLite/PostgreSQL SQL 查询宏辅助
        ├── models.rs             # 数据库实体结构体定义
        └── repository/           # 领域仓储层 (agent, alerts, ddns, nat, pat, tasks, temporary_transfer, user)
```

### 2.2 `crates/agent` — 受控节点守护进程

```text
crates/agent/src/
├── main.rs                       # Agent CLI (enroll / run)、Ed25519 挑战认证、gRPC Session & IoStream 主循环
├── collector/
│   └── mod.rs                    # 基于 sysinfo 的 CPU、内存、Swap、磁盘、网络、进程数、连接数与负载采集
├── executor/
│   ├── mod.rs                    # 任务分发器入口
│   ├── shell.rs                  # 远程 Shell 执行器（Unix 独立进程组 PGID 隔离与超时/Drop 全组清理）
│   ├── http.rs                   # 分布式 HTTP GET 拨测与 TLS 证书到期时间提取
│   ├── tcp.rs                    # 分布式 TCP 端口连通性与延迟探测
│   ├── icmp.rs                   # 分布式 ICMP Ping 探测
│   ├── files.rs                  # 远端目录列表、文件分块读取/写入/删除与流式传输
│   └── terminal.rs               # 交互式 PTY 伪终端会话管理（打开、输入、窗口大小调整、关闭）
└── nat/
    └── mod.rs                    # Agent 端 NAT 反向隧道执行器（DNS Pinning 解析、私网防 SSRF 策略与本地 TCP 桥接）
```

### 2.3 `crates/tsdb`、`crates/shared` 与 `crates/proto-gen`

- **`crates/tsdb/src/lib.rs`**：实现 `MetricStore`，提供高频遥测样本写入、按 `QueryRange`（`Hour1`、`Day1`、`Day7`、`Day30`）检索及时间序列降采样。
- **`crates/shared/src/`**：
  - `ids.rs`：`AgentId`、`UserId` 等强类型 UUID 封装。
  - `authz.rs`：全系统 RBAC Scope 枚举与 PAT 权限常量定义。
  - `tasks.rs`：`ServerSelector`（按 ID、分组、标签筛选目标节点）。
  - `nat.rs`：`NatTunnelControlMessage`（Server 与 Agent 之间的 NAT 隧道握手控制帧）。
  - `ddns.rs`、`terminal.rs`、`error.rs`：跨端共享模型与统一错误枚举。
- **`crates/proto-gen/`**：`build.rs` 调用 `tonic-build` 编译 `proto/xlstatus/v1/{agent,common,nat}.proto`（逻辑上对应 XLStatus v1 统一 gRPC 协议 `proto/xlstatus.proto` 拆分后的三大模块）。

---

## 3. Web 运维工作台结构 (`web/`)

前端基于 **Next.js 16 (App Router)**、**React 19** 与 **Tailwind CSS v4** 构建：

```text
web/
├── Dockerfile                    # Next.js standalone 多阶段生产容器镜像
├── next.config.ts                # Next.js 构建配置与安全响应头策略
├── package.json                  # 前端依赖与脚本 (dev / build / start / lint / typecheck)
├── app/
│   ├── layout.tsx                # 根布局、主题注入与全局样式加载
│   ├── page.tsx                  # 根路由重定向入口
│   ├── (auth)/
│   │   └── login/page.tsx        # 登录页（支持用户名密码、TOTP 二步验证与 OAuth2/OIDC 跳转）
│   ├── oauth/
│   │   └── callback/page.tsx     # OAuth2 / OIDC 授权回调处理页
│   ├── status/
│   │   ├── page.tsx              # 匿名公开状态页服务端入口
│   │   ├── StatusPageClient.tsx  # 公开状态页交互式客户端大盘与实时指标视图
│   │   └── servers/[id]/         # 公开单节点详情与历史趋势页
│   ├── (dashboard)/              # 受保护的运维管理后台路由组
│   │   ├── layout.tsx            # 后台主框架布局、侧边导航栏与身份边界校验
│   │   ├── dashboard/page.tsx    # 集群总览大盘（实时汇总、世界地图、状态卡片）
│   │   ├── servers/              # 服务器资产列表与单节点深度遥测详情 ([id])
│   │   ├── terminal/page.tsx     # Web 交互式 PTY 终端与远端文件管理器
│   │   ├── services/page.tsx     # HTTP / TCP / ICMP 服务拨测监控与 SLA 统计
│   │   ├── alerts/page.tsx       # 告警规则编排与历史告警事件流
│   │   ├── notifications/page.tsx# Webhook 通知渠道、模板与通知组配置
│   │   ├── tasks/page.tsx        # 定时 Cron 任务与批量命令执行编排
│   │   ├── nat/page.tsx          # NAT 反向隧道端口映射管理
│   │   ├── ddns/page.tsx         # 动态 DNS (DDNS) 域名绑定管理
│   │   ├── mcp/page.tsx          # AI-Native MCP Ops Studio（交互式执行 Tools / Resources / Prompts 与配置生成）
│   │   ├── tokens/page.tsx       # Personal Access Token (PAT) 与 Agent Enrollment Token 管理
│   │   ├── users/page.tsx        # 多租户用户与 RBAC 角色管理
│   │   └── settings/page.tsx     # 系统设置、TOTP 绑定、OAuth 绑定、WAF 封禁与审计日志
│   ├── components/               # 业务级共享组件
│   │   ├── Navigation.tsx        # 响应式导航与快捷操作栏
│   │   ├── ServerDataProvider.tsx# WebSocket (/ws/servers) 实时状态上下文与缓存同步
│   │   ├── WorldServerMap.tsx    # 全球节点分布可视化地图
│   │   ├── M7Primitives.tsx      # 统一设计系统基础原语组件
│   │   ├── Dialogs.tsx / Toasts.tsx # 模态框与通知气泡
│   │   └── charts/               # 高性能轻量级 SVG 时序折线图与双轴图组件
│   └── lib/                      # 前端运行时工具与安全模块
│       ├── security.ts           # 前端 URL 安全清洗、跳转换行拦截与敏感输入防护 (*.test.mts 配套单测)
│       ├── use-server-realtime.ts# WebSocket 实时重连与增量状态合并 Hook
│       ├── server-cache.ts       # 客户端节点状态快照缓存
│       ├── format.ts             # 字节速率、百分比、运行时长格式化工具
│       └── dashboard-theme.ts    # 主题配色管理
└── lib/
    ├── api.ts                    # 强类型 HTTP API Client、自动附加 CSRF Header 与 TOTP 重试弹窗衔接
    ├── i18n.ts / use-i18n.ts     # 国际化入口（默认简体中文，支持英文切换）
    └── i18n/                     # 分页面中英双语词典 (en.ts, pages/*)
```

---

## 4. Protobuf、部署脚本、桥接器与验收测试

### 4.1 `proto/` — gRPC 协议定义
```text
proto/xlstatus/v1/
├── common.proto                  # HostInfo、HostState、网卡/磁盘/连接统计等公共遥测消息
├── agent.proto                   # AgentService (Session & IoStream)、ServerTask、TaskResult、IoFrame
└── nat.proto                     # NAT 隧道配置与协议枚举定义
```

### 4.2 `deploy/` — 生产部署与 systemd 脚本
```text
deploy/
├── install.sh                    # Server 一键安装与 systemd 托管脚本（默认监听 127.0.0.1:50051）
├── install-agent.sh              # Agent 一键下载、Enrollment 注册与 systemd 启动脚本
├── xlstatus-server.service       # Server systemd unit 模板
└── xlstatus-agent.service        # Agent systemd unit 模板
```

### 4.3 `scripts/` — AI MCP 桥接与数据库迁移工具
```text
scripts/
├── xlstatus-mcp-stdio.mjs        # 标准 MCP Stdio-to-HTTP 桥接器（1MiB 行缓冲保护、自动令牌脱敏）
├── import_sqlite_to_postgres.py  # SQLite 到 PostgreSQL 全量数据在线迁移脚本
└── tests/
    └── test_import_sqlite_to_postgres.py # 迁移脚本跨库类型转换与幂等性单元测试
```

### 4.4 `test-run/` — M0–M9 全链路自动化验收脚本
`test-run/` 收录了覆盖全系统里程碑的自动化回归与安全验证脚本（详见 `test-run/TESTING.md`）：
- `verify-m0.sh` / `verify-m1-pg.sh`：基础编译、SQLite 与 PostgreSQL 迁移及认证验收。
- `verify-m2-reconnect.sh` / `verify-m2-revoke.sh`：Agent 断线重连与吊销即时阻断验收。
- `verify-m3-metrics.sh` / `verify-m3-tsdb.sh` / `verify-m3-ws.sh`：指标采集、TSDB 聚合与 WebSocket 实时流验收。
- `verify-m4-alerts.sh`：服务拨测、告警引擎触发与恢复通知验收。
- `verify-m5-task.sh` / `verify-m5-scheduler.sh` / `verify-m5-files.sh` / `verify-m5-terminal.sh`：远程命令、定时调度、文件传输与交互式终端验收。
- `verify-m6-ddns.sh` / `verify-m6-nat.sh` / `verify-m6-mcp.sh`：DDNS、NAT 反向隧道与 MCP `2024-11-05` 全能力验收。
- `verify-m7-ui.sh` / `verify-m8-migrations.sh` / `verify-m8-tsdb-load.sh` / `verify-m9-install.sh`：前端 UI 规范、数据迁移、TSDB 压测与安装器升级回滚安全验收。
- `verify-process-cleanup-safety.sh` / `verify-install-upgrade-safety.sh`：子进程隔离清理与升级配置保留安全回归。
