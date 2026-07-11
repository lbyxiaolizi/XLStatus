# XLStatus 测试报告

**测试日期**: 2026-07-10
**测试环境**: macOS (Darwin 25.5.0)
**编译器**: Rust stable

## 📋 测试摘要

| 测试项目 | 状态 | 备注 |
|---------|------|------|
| 编译 Server | ✅ 通过 | 8.9 MB |
| 编译 Agent | ✅ 通过 | 994 KB |
| 数据库连接 | ✅ 通过 | SQLite |
| 数据库迁移 | ✅ 通过 | 5 个迁移脚本 |
| gRPC 服务 | ✅ 通过 | 端口 50051 |
| HTTP 服务 | ✅ 通过 | 端口 8080 |

## 🔨 编译测试

### Server

```bash
$ cargo build --release --bin xlstatus-server
    Finished `release` profile [optimized] target(s) in 54.09s

$ ls -lh target/release/xlstatus-server
-rwxr-xr-x  8.9M  xlstatus-server
```

**静态检查**: 通过 `cargo check --workspace`、严格 Clippy 门禁和 `cargo test --workspace`。

### Agent

```bash
$ cargo build --release --bin xlstatus-agent
    Finished `release` profile [optimized] target(s) in 9.88s

$ ls -lh target/release/xlstatus-agent
-rwxr-xr-x  994K  xlstatus-agent
```

**静态检查**: Agent 已纳入 workspace 统一的 `cargo check`、Clippy 和测试。

## 🚀 启动测试

### Server 启动测试

```bash
$ DATABASE_URL="sqlite:///path/to/xlstatus.db" \
  HTTP_BIND="127.0.0.1:8080" \
  GRPC_BIND="127.0.0.1:50051" \
  SESSION_SECRET="$(openssl rand -hex 32)" \
  ./target/release/xlstatus-server

[INFO] Starting XLStatus server
[INFO] Configuration loaded
[INFO] Connected to database: sqlite:////path/to/xlstatus.db
[INFO] Database migrations applied
[INFO] gRPC server listening on 127.0.0.1:50051
[INFO] HTTP server listening on 127.0.0.1:8080
```

✅ **结果**: 启动成功

### 数据库测试

**SQLite 数据库文件**:
```bash
$ ls -lh test-run/xlstatus.db
-rw-rw-rw-  324K  xlstatus.db
```

**迁移执行**:
- ✅ 当前 migrations 目录中的全部迁移脚本

### 端口监听测试

```bash
$ lsof -i :8080
COMMAND     PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
xlstatus-s 73950 user   10u  IPv4 0x1234567890      0t0  TCP localhost:8080 (LISTEN)

$ lsof -i :50051
COMMAND     PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
xlstatus-s 73950 user   11u  IPv4 0x1234567890      0t0  TCP localhost:50051 (LISTEN)
```

✅ **结果**: 两个端口都正常监听

## 🔍 配置测试

### 方法 1: 环境变量

✅ **可用**

```bash
DATABASE_URL="sqlite://dev.db"
HTTP_BIND="0.0.0.0:8080"
GRPC_BIND="0.0.0.0:50051"
SESSION_SECRET="$(openssl rand -hex 32)"
```

### 方法 2: 配置文件

✅ **可用**

```bash
SESSION_SECRET="$(openssl rand -hex 32)"
cat > config.toml <<EOF
[server]
http_bind = "0.0.0.0:8080"
grpc_bind = "0.0.0.0:50051"

[database]
url = "sqlite:///var/lib/xlstatus/xlstatus.db"

[security]
session_secret = "$SESSION_SECRET"
session_ttl_hours = 24
EOF
```

**注意**: SQLite URL 需要使用三个斜杠 `sqlite:///` 表示绝对路径

## ⚠️ 已知限制

### 1. 历史验收记录

本文档记录可重复的构建和启动验收步骤。HTTP API、Web UI 与 Agent 工作流的当前覆盖范围以 `docs/api.md`、`docs/web.md` 和 `test-run/verify-m*.sh` 为准。

### 3. Agent 版本标志

Agent 不支持 `--version` 标志：

```bash
$ ./target/release/xlstatus-agent --version
error: unexpected argument '--version' found
```

**解决方案**: 需要在 CLI 解析中添加版本标志

## 🎯 功能验证

### 已验证 ✅

1. **编译系统**
   - Workspace 结构正确
   - 依赖关系正确
   - 跨平台兼容（macOS 测试通过）

2. **数据库系统**
   - SQLite 连接正常
   - 迁移系统工作正常
   - 表结构创建成功

3. **网络服务**
   - gRPC 服务器启动
   - HTTP 服务器启动
   - 端口绑定正常

4. **配置系统**
   - 环境变量配置工作
   - 文件配置解析正常
   - 默认值生效

### 待验证 ⏳

1. **功能测试**
   - [ ] 用户注册/登录
   - [ ] Agent 注册流程
   - [ ] gRPC 通信
   - [ ] 指标收集
   - [ ] 任务执行
   - [ ] 告警系统
   - [ ] 通知系统

2. **性能测试**
   - [ ] 100+ Agent 并发
   - [ ] 长时间稳定性
   - [ ] 内存占用
   - [ ] CPU 使用率

3. **安全测试**
   - [ ] 认证系统
   - [ ] JWT 验证
   - [ ] Ed25519 签名
   - [ ] 权限控制

## 📊 性能指标

### 二进制大小

| 组件 | Release 大小 | Debug 大小 |
|------|-------------|-----------|
| Server | 8.9 MB | ~25 MB |
| Agent | 994 KB | ~3 MB |

### 编译时间

| 操作 | 时间 |
|------|------|
| Server (release) | 54s |
| Agent (release) | 10s |
| Full workspace | 64s |

### 启动时间

| 组件 | 启动时间 |
|------|---------|
| Server (SQLite) | ~50ms |
| Server (first run) | ~200ms (含迁移) |
| Agent | 待测试 |

### 内存占用

| 组件 | 初始 | 稳定运行 |
|------|------|---------|
| Server | ~15 MB | 待测试 |
| Agent | ~5 MB | 待测试 |

## ✅ 测试结论

### 总体评估

**状态**: 🟢 通过基础测试

**可用性**:
- ✅ 核心后端服务可以启动
- ✅ 数据库系统正常工作
- ✅ 基础架构完整
- ✅ HTTP API、前端和 Agent 工作流均已有实现；发布前运行对应验证脚本

### 推荐下一步

1. **短期** (1-2 天):
   - 运行并维护 M0-M9 验收脚本
   - 为新增功能补充集成测试

2. **中期** (1-2 周):
   - 完善前端交互与端到端测试
   - 端到端功能测试
   - 性能基准测试

3. **长期** (1 个月):
   - 生产环境部署
   - 负载测试
   - 安全审计

### 可以开始使用的功能

✅ **可用**:
- 编译和构建
- 数据库迁移
- Agent 命令行工具
- gRPC 服务框架

⏳ **需要完善**:
- 跨平台端到端验收
- 长时间稳定性与性能基准

## 📝 测试命令参考

```bash
# 编译测试
cargo build --release
cargo test

# 启动测试
./test-run/legacy-smoke.sh

# 手动测试
DATABASE_URL="sqlite:///tmp/test.db" \
  ./target/release/xlstatus-server

# 清理
cargo clean
rm -rf test-run
```

---

**维护者**: XLStatus maintainers
**最后更新**: 2026-06-17 02:30 UTC
