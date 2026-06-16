# M5 任务执行 - 新增文件清单

## Protobuf 定义

```
proto/xlstatus/v1/agent.proto
  - 扩展了 TaskType 枚举
  - 新增 10+ 任务消息类型
  - 新增 10+ 结果消息类型
  - 新增 IoStream RPC
  - 新增 ConfigUpdate, ForceUpdate 消息
```

## 共享库

```
crates/shared/src/tasks.rs (NEW)
  - TaskType, TaskStatus, CoverMode 枚举
  - ServerSelector 结构
  - 4 种任务 Payload（Shell, HTTP, ICMP, TCP）
  - Task, TaskRun, Transfer, AuditLog 模型
```

## Agent 执行器

```
crates/agent/src/executor/mod.rs (NEW)
crates/agent/src/executor/shell.rs (NEW)
  - execute_shell_command()
  - 支持超时、环境变量、输出限制
  - 4 个单元测试

crates/agent/src/executor/http.rs (NEW)
  - execute_http_get()
  - 支持自定义头、TLS 验证
  - 4 个单元测试

crates/agent/src/executor/icmp.rs (NEW)
  - execute_icmp_ping()
  - 跨平台 ping 解析
  - 3 个单元测试

crates/agent/src/executor/tcp.rs (NEW)
  - execute_tcp_ping()
  - TCP 连接延迟测试
  - 3 个单元测试

crates/agent/src/executor/terminal.rs (NEW)
  - TerminalSession 结构
  - PTY 会话管理（Unix）
  - Shell 自动检测
  - 3 个单元测试

crates/agent/src/executor/files.rs (NEW)
  - list_files()
  - read_file()
  - write_file()
  - delete_path()
  - 6 个单元测试
```

## Server 基础设施

```
crates/server/src/db/repository/tasks.rs (NEW)
  - TaskRepository（7 个方法）
  - TaskRunRepository（3 个方法）
  - AuditLogRepository（2 个方法）

crates/server/src/tasks/mod.rs (NEW)
crates/server/src/tasks/scheduler.rs (NEW)
  - TaskScheduler 结构
  - Cron 调度循环
  - 服务器选择器解析
  - 手动触发支持
  - 2 个单元测试

crates/server/src/api/v1/tasks.rs (NEW)
  - 7 个 REST 端点
  - 完整的 CRUD 操作
  - 执行历史查询
```

## 数据库迁移

```
crates/server/migrations/sqlite/005_tasks.sql (NEW)
  - 8 个表定义
  - 索引优化

crates/server/migrations/postgres/005_tasks.sql (NEW)
  - 8 个表定义
  - 3 个分区表
  - 索引优化
```

## 文档

```
M5-PROGRESS.md (NEW)
  - 详细完成报告
  - 功能清单
  - 技术亮点
  - 测试覆盖

M5-SUMMARY.md (NEW)
  - 简洁总结
  - 代码统计
  - 验证命令

M5-FILES-CREATED.md (THIS FILE)
  - 文件清单
```

## 统计数据

- **新增文件**: 18 个
- **代码行数**: 
  - Agent 执行器: ~1800 行
  - Server 基础设施: ~1200 行
  - 测试代码: ~600 行
- **总计**: ~3600 行

## 依赖更新

```toml
# crates/agent/Cargo.toml
+ reqwest = { version = "0.11", features = ["json"] }
+ portable-pty = "0.8"

# crates/server/Cargo.toml
+ cron = "0.12"
```

## 更新的文件

```
crates/shared/src/lib.rs
  + pub mod tasks;

crates/agent/src/main.rs
  + mod executor;

crates/server/src/grpc/mod.rs
  + type IoStreamStream
  + async fn io_stream()
  + 处理 HostInfoUpdate 和 GeoIpReport

crates/agent/Cargo.toml
  + 新增依赖

crates/server/Cargo.toml
  + 新增依赖

CLAUDE.md
  + 更新项目状态
```

---

**所有文件已编译通过** ✅  
**项目构建成功** ✅
