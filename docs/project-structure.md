# 项目结构

根目录只保留当前开发和发布需要的入口、配置、源码、文档和测试脚本。

```text
XLStatus/
├── .github/workflows/        # CI 和 release workflow
├── crates/                   # Rust workspace
├── deploy/                   # systemd unit 和安装脚本
├── docs/                     # 当前文档和历史归档
├── proto/                    # gRPC protobuf 定义
├── test-run/                 # 验收和 smoke 脚本
├── web/                      # Next.js UI
├── Cargo.toml                # Rust workspace manifest
├── docker-compose*.yml       # SQLite/PostgreSQL/简化 Compose
├── Dockerfile.server         # Server 镜像
├── Dockerfile.agent          # Agent 镜像
├── config.example.toml       # Server TOML 示例
├── README.md                 # 英文入口
└── README.zh-CN.md           # 中文入口
```

## Rust workspace

```text
crates/
├── server/       # HTTP API、认证、数据库、gRPC Server、后台任务
├── agent/        # Agent CLI、注册、采集、执行器和 gRPC client
├── shared/       # 跨 server/agent 共享的数据类型和错误类型
├── proto-gen/    # 编译 proto 并导出生成代码
├── tsdb/         # 指标时序存储 facade
└── xtask/        # 开发/验证辅助任务
```

### Server 重点目录

```text
crates/server/src/api/v1/      # HTTP route handlers
crates/server/src/auth/        # session、JWT、RBAC、TOTP
crates/server/src/db/          # DB models 和 repository
crates/server/src/grpc/        # Agent gRPC service/session
crates/server/src/services/    # 服务探测和监控循环
crates/server/src/alerts/      # 告警规则执行
crates/server/src/tasks/       # 任务调度
crates/server/migrations/      # SQLite/PostgreSQL migrations
```

### Agent 重点目录

```text
crates/agent/src/main.rs             # CLI、enroll、run、gRPC session
crates/agent/src/collector/          # 主机指标采集
crates/agent/src/executor/           # shell/http/tcp/icmp/file/terminal 执行器
```

## Web

```text
web/app/(dashboard)/        # 登录后的管理页面
web/app/status/             # 公开状态页
web/app/components/         # 共享 UI 组件和地图组件
web/lib/api.ts              # API client、base URL、CSRF
web/lib/i18n.ts             # 共享中文文案
web/Dockerfile              # Next.js standalone 镜像
```

生产环境修改 `NEXT_PUBLIC_API_URL` 后必须重新 build。

## Docs

```text
docs/README.md              # 当前唯一文档索引
docs/quickstart.md          # 本地快速启动
docs/installation.md        # Docker/source/systemd/release 安装
docs/configuration.md       # 配置模式、CORS、数据库
docs/agent.md               # Agent 注册和安装
docs/web.md                 # Web 构建和部署
docs/api.md                 # 接口概览
docs/operations.md          # 运维和升级
docs/troubleshooting.md     # 排障
docs/development.md         # 开发和验证
docs/release-checklist.md   # 发布检查
docs/archive/               # 历史规划材料，不作为当前用户文档路径
```

新增用户文档时，优先补到 `docs/` 并从 `docs/README.md` 链接。临时报告、一次性验证输出和早期方案草稿放入 archive 或不入库。

## Test Run

`test-run/` 保留 M0-M9 的验收脚本。部分脚本会启动服务、写本地数据库、占用固定端口或依赖 Docker/PostgreSQL；运行前先读脚本头部注释。
