# 开发者与贡献指南 (Development Guide)

本文档面向参与 **XLStatus `v0.3`** 开发的贡献者与维护者，介绍 Rust Workspace 各模块职责、Next.js 前端工程规范、数据库双后端迁移约定、开发辅助工具 (`xtask`) 以及合并前必须通过的 CI/CD 质量门禁。

---

## 1. 开发环境准备

| 工具 | 版本要求 | 说明 |
|---|---|---|
| **Rust 工具链** | Stable (`rustup default stable`) | 包含 `cargo`、`rustfmt`、`clippy` |
| **Protobuf 编译器** | `protoc` (`libprotoc 3+`) | 编译 `proto/` 下的 gRPC 消息与服务定义（`crates/proto-gen` 构建期依赖） |
| **Node.js & pnpm** | Node.js `20+`（CI 使用 Node `24`），`pnpm@10.23.0` | 通过 `corepack enable` 自动激活 `web/package.json` 指定的 `pnpm` 版本 |
| **SQLite 3** | `3.40+` | 本地开发、单元与集成测试及 M0-M9 验收脚本使用 |
| **PostgreSQL** | `15+`（可选，或使用 Docker） | 运行 `XLSTATUS_TEST_PG_URL` 门控的 PostgreSQL 迁移幂等性与列宽契约测试 |
| **Python 3** | `3.10+` | 运行 `scripts/import_sqlite_to_postgres.py` 单元测试与验收脚本断言 |

---

## 2. Rust Workspace 模块架构 (`crates/`)

仓库根目录 `Cargo.toml` 使用 Resolver v2 管理 6 个 Workspace Crate：

| Crate 名称 | 目录路径 | 核心职责 |
|---|---|---|
| `xlstatus-server` | `crates/server` | 核心控制面二进制：Axum HTTP API、WebSocket 实时流与终端桥接、Tonic gRPC 服务端 (`AgentService` + `NatTunnel`)、SQLite/PostgreSQL 双后端仓储、RBAC/PAT/TOTP/OAuth2 认证、MCP `2024-11-05` 引擎、告警/探测/定时任务调度器、DDNS 与 NAT 穿透管理、HKDF/AES-256-GCM 密钥加密 |
| `xlstatus-agent` | `crates/agent` | 受管节点守护进程二进制：`enroll` 注册（Ed25519 密钥对生成）、`run` 主循环（指数退避重连、短期 JWT 挑战签名刷新）、主机静态信息与秒级动态指标采集、HTTP/TCP/ICMP 探针与 Shell 任务执行、PTY 终端、受控文件操作及 NAT 反向隧道 `IoStream` 桥接 |
| `xlstatus-tsdb` | `crates/tsdb` | 高并发时序指标存储引擎：提供 `MetricStore` 与 `InMemoryMetricStore`，支持按时间窗口 (`1h`/`6h`/`1d`/`7d`/`30d`) 聚合查询、批量写入、保留期动态调整与后台定期过期样本压缩 (`compact`) |
| `xlstatus-shared` | `crates/shared` | Server 与 Agent 共享的强类型领域模型：ID 新类型 (`UserId`, `AgentId`, `ServerId`)、角色与 PAT Scope 枚举、DDNS 与 NAT 共享契约结构 |
| `xlstatus-proto-gen` | `crates/proto-gen` | 基于 `tonic-build` / `prost` 从 `proto/` 生成的 gRPC Rust 代码（`AgentService`、`NatTunnel`、`HostInfo`、`HostState`、`IoFrame` 等） |
| `xtask` (`xlstatus-xtask`) | `crates/xtask` | 本地开发与性能基准工具：生成确定性模拟 Agent 指标流 (`mock-agents`)、执行 TSDB 查询延迟 P95 基准测试 (`query-bench`)、验证 TSDB 压缩 (`tsdb-compact`) 与健康状态 (`tsdb-health`) |

### 使用 `xtask` 进行本地基准与模拟测试

```bash
# 查看 xtask 支持的子命令
cargo run -p xtask -- help

# 模拟 100 个 Agent 在 24 小时内按 3s 间隔生成的 2,880,000 条样本（dry-run 输出报告）
cargo run -p xtask -- mock-agents --count 100 --interval 3s --duration 24h --dry-run

# 运行 TSDB 多窗口查询基准测试，要求 P95 <= 500ms
cargo run -p xtask -- query-bench --period 1d,7d,30d --agents 128 --samples 1000 --p95-target-ms 500
```

---

## 3. Web 前端开发规范 (`web/`)

前端基于 **Next.js 16 (App Router)** + **React 19** + **TypeScript 5** + **Tailwind CSS 4** 构建。

### 3.1 常用开发命令

```bash
cd web
pnpm install --frozen-lockfile

# 启动本地开发服务器 (http://localhost:3000)
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev

# 运行 ESLint 静态检查
pnpm lint

# 运行 TypeScript 严格类型与 API 契约检查
pnpm typecheck

# 运行前端与 MCP Stdio 桥接安全单元测试 (Node.js 原生测试运行器)
pnpm test:unit

# 构建生产包
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```

### 3.2 前端工程与安全准则

1. **安全工具函数 (`web/app/lib/security.ts`)**：
   - 所有登录跳转参数 (`return_to`、`next`、`redirect`) 必须经过 `resolveLoginRedirectParam` / `sanitizeRedirectPath` 过滤，阻断开放重定向 (`//evil.example.com`、`javascript:`、`/login` 循环等)。
   - 所有动态拼接进 API URL 的路径段必须经过 `encodePathSegment` 编码，防止 `.` / `..` / `%2e%2e` 路径穿越。
   - 所有主题自定义 CSS 变量名与值必须经过 `sanitizeThemeVariables` / `sanitizeCssVariableValue` 过滤，阻断 CSS 注入与 `<style>` 逃逸。
   - 任何错误提示或日志展示必须通过 `redactSensitiveTokens` 自动擦除 `xlp_...` 与 `xle_...` 令牌。
2. **国际化 (`web/lib/i18n.ts`)**：
   - 当前唯一生产语言为简体中文 (`zh-CN`)，所有用户可见文案统一收敛在 `web/lib/i18n.ts` 的 `zhCN` 字典中。
   - 后端协议枚举值、PAT Scope 字符串（如 `server:read`、`server:exec`）、MCP 工具名（如 `cluster.diagnose`）保持原样，不作翻译。
3. **弹窗焦点陷阱与滚动锁 (`web/app/lib/body-scroll-lock.ts`)**：
   - 使用基于引用计数的 `createBodyScrollLockManager` 处理嵌套模态框滚动锁定，确保任意顺序关闭弹窗都能恢复原始 `overflow` 状态。

---

## 4. 数据库迁移与 SQLx 解码安全红线

### 4.1 双后端同步迁移 (`sqlite` & `postgres`)

数据库迁移脚本分别位于：

- SQLite：`crates/server/migrations/sqlite/`
- PostgreSQL：`crates/server/migrations/postgres/`

新增或修改表结构时，**必须同时更新两套迁移脚本**，并在 [`scripts/import_sqlite_to_postgres.py`](../scripts/import_sqlite_to_postgres.py) 的 `XLSTATUS_TABLES` 白名单中保持表名同步。

- 所有 `CREATE TABLE`、`CREATE INDEX` 必须带 `IF NOT EXISTS` 以保证重复启动幂等性。
- 注意 PostgreSQL `INT4` (`i32`) 与 `INT8` (`i64`) 的精确类型匹配，避免在 PostgreSQL 后端解码时报列类型不匹配错误。

### 4.2 严禁静默吞掉 SQLx `try_get` 解码错误（CI 强制门禁）

在 `crates/server/src` 中读取数据库行时，**严禁**对 `row.try_get(...)` 使用 `.ok()`、`.unwrap_or(...)`、`.unwrap_or_default()` 或 `.unwrap_or_else(...)` 将列解码失败静默转为 `None` 或默认值！

- **错误写法（会被 CI 直接拦截）**：
  ```rust
  // ❌ 严禁：如果列名拼写错误或 INT4/INT8 类型不匹配，错误会被静默吞掉！
  let count: Option<i64> = row.try_get("count").ok();
  ```
- **正确写法**：
  ```rust
  // ✅ 正确：如果数据库列本身可空，显式解码为 Option<T> 并用 ? 向上传播解码错误
  let count: Option<i64> = row.try_get("count")?;
  ```

---

## 5. CI/CD 门禁检查清单 (Pre-PR Gate Checks)

在提交 Pull Request 前，请在本地完整运行以下与 `.github/workflows/build.yml` 完全一致的门禁命令：

```bash
# 1. Rust 代码格式检查
cargo fmt --check

# 2. 检查是否违规静默吞掉 SQLx try_get 解码错误
! rg --line-number --multiline 'try_get(?:<[^>]+>)?\([^)]*\)\s*\.(?:ok\(\)|unwrap_or(?:_default|_else)?\s*\()' crates/server/src

# 3. Workspace 编译与严格 Clippy 门禁（锁定 Cargo.lock）
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity

# 4. 运行全量 Rust 单元与集成测试（设置 XLSTATUS_TEST_PG_URL 可同时激活 PostgreSQL 专项测试）
cargo test --workspace --locked

# 5. Shell 脚本语法检查与安全回归测试
find deploy scripts test-run -type f -name '*.sh' -print0 | xargs -0 -n1 bash -n
bash test-run/verify-process-cleanup-safety.sh
bash test-run/verify-install-upgrade-safety.sh

# 6. Python SQLite -> PostgreSQL 迁移工具单元测试
python3 -m unittest discover -s scripts/tests -p 'test_*.py'

# 7. Web 前端 Lint、单元测试、TypeScript 类型检查与生产构建
cd web
pnpm lint
pnpm test:unit
pnpm typecheck
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

关于 `test-run/` 下 M0 至 M9 端到端验收脚本的详细用法与端口分配，请阅读 [`test-run/TESTING.md`](../test-run/TESTING.md)。
