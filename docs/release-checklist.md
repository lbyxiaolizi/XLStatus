# 发布检查清单 (Release Checklist)

在创建 Git Release Tag（如 `v0.2`）并发布新版本前，必须逐项完成本清单中的核验步骤。任何一项未通过均不得打标发布。

---

## 1. 版本号全仓同步核验 (Version Sync)

GitHub Actions (`.github/workflows/build.yml` 中的 `Verify release version metadata` 步骤) 会在推送 `v*` 标签时强制校验以下 6 处版本号完全同步（对于二维标签如 `v0.2`，Cargo 与 npm 清单版本应为 `0.2.0`）：

1. **Rust Workspace 全部 6 个 Crate (`crates/*/Cargo.toml`)**：
   - `xlstatus-shared`、`xlstatus-proto-gen`、`xlstatus-tsdb`、`xlstatus-server`、`xlstatus-agent`、`xtask` 的 `version` 均等于 `0.2.0`。
2. **Web 前端清单 (`web/package.json`)**：
   - `"version": "0.2.0"`。
3. **Server 安装脚本 (`deploy/install.sh`)**：
   - `VERSION="${VERSION:-v0.2}"`。
4. **Agent 安装脚本 (`deploy/install-agent.sh`)**：
   - `VERSION="${VERSION:-v0.2}"`。
5. **Server 内置 Agent Bootstrap 默认版本 (`crates/server/src/main.rs`)**：
   - `const DEFAULT_AGENT_INSTALL_VERSION: &str = "v0.2";`。
6. **Web 设置页 Agent 安装卡片回退版本 (`web/app/(dashboard)/settings/sections/AgentInstallCard.tsx`)**：
   - `const DEFAULT_AGENT_VERSION = "v0.2";`。

### 一键核验脚本

```bash
TAG="v0.2"
MANIFEST_VERSION="0.2.0"

cargo metadata --locked --no-deps --format-version 1 \
  | jq -e --arg version "$MANIFEST_VERSION" 'all(.packages[]; .version == $version)'
jq -e --arg version "$MANIFEST_VERSION" '.version == $version' web/package.json
grep -F "VERSION=\"\${VERSION:-$TAG}\"" deploy/install.sh
grep -F "VERSION=\"\${VERSION:-$TAG}\"" deploy/install-agent.sh
grep -F "DEFAULT_AGENT_INSTALL_VERSION: &str = \"$TAG\"" crates/server/src/main.rs
grep -F "DEFAULT_AGENT_VERSION = \"$TAG\"" 'web/app/(dashboard)/settings/sections/AgentInstallCard.tsx'
```

---

## 2. 后端、前端与脚本门禁检查 (CI Gate Checks)

### 2.1 Rust 格式化、SQLx 错误传播、Clippy 与全量测试

```bash
# 1. 格式检查
cargo fmt --check

# 2. 严禁静默吞掉 SQLx try_get 解码错误
! rg --line-number --multiline 'try_get(?:<[^>]+>)?\([^)]*\)\s*\.(?:ok\(\)|unwrap_or(?:_default|_else)?\s*\()' crates/server/src

# 3. 锁定依赖检查与严格 Clippy
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity

# 4. 运行全量单元与集成测试（含 PostgreSQL 契约测试）
XLSTATUS_TEST_PG_URL="postgres://xlstatus:xlstatus@127.0.0.1:5432/xlstatus_test" \
  cargo test --workspace --locked
```

### 2.2 Web 前端与 MCP Stdio 桥接检查

```bash
cd web
pnpm install --frozen-lockfile
pnpm lint
pnpm test:unit
pnpm typecheck
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

### 2.3 部署编排、迁移脚本与 Shell 安全性检查

```bash
# 1. Shell 语法检查
find deploy scripts test-run -type f -name '*.sh' -print0 | xargs -0 -n1 bash -n

# 2. 进程组隔离清理与安装器原子升级/回滚验证
bash test-run/verify-process-cleanup-safety.sh
bash test-run/verify-install-upgrade-safety.sh

# 3. Docker Compose 三套编排语法验证
SESSION_SECRET=ci-only-session-secret-at-least-32-bytes \
SECRET_ENCRYPTION_KEY=ci-only-encryption-key-at-least-32-bytes \
POSTGRES_PASSWORD=ci-only-postgres-password \
  sh -c '
    docker compose -f docker-compose.yml config --quiet &&
    docker compose -f docker-compose.pg.yml config --quiet &&
    docker compose -f docker-compose.simple.yml config --quiet
  '

# 4. SQLite -> PostgreSQL 迁移工具单元测试
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

---

## 3. 安全回归测试清单 (Security Regression Suite)

发布前确认以下安全边界测试全部通过：

- [x] **启动凭据脱敏 (`crates/server/tests/startup_secret_redaction.rs`)**：
  无效 `DATABASE_URL`、异常 `PGPASSFILE`、语法错误的 `config.toml` 在报错退出时绝不泄漏用户名、密码或密钥。
- [x] **密钥熵与弱默认值拦截 (`crates/server/src/config.rs` & `secrets.rs`)**：
  `SESSION_SECRET` 与 `SECRET_ENCRYPTION_KEY` 低于 32 字符、不同字节数 `< 4` 或命中示例占位符时拒绝启动；数据库敏感字段使用 `HKDF-SHA256` 派生 `AES-256-GCM` 密钥加密。
- [x] **在线恢复沙箱加固 (`crates/server/src/api/v1/maintenance.rs`)**：
  强制校验 SQLite 魔数头、`PRAGMA trusted_schema = OFF`、`PRAGMA integrity_check`，拒绝含 `trigger`、`view`、`VIRTUAL TABLE` 的备份文件。
- [x] **前端与 MCP 桥接安全 (`web/app/lib/security.test.mts`)**：
  开放重定向拦截、URL 路径段防穿越编码、自定义主题 CSS 变量防注入、错误消息 `xlp_`/`xle_` 令牌脱敏、MCP Stdio 1 MiB 行长限制全部通过。
- [x] **端到端发布安装与 MCP 权限回归**：
  ```bash
  bash test-run/verify-m6-mcp.sh
  bash test-run/verify-m7-ui.sh
  bash test-run/verify-m9-install.sh
  ```

---

## 4. 多平台 Release 构建矩阵核验

推送 `v*` 标签后，GitHub Actions 会自动构建并发布以下 **10 个目标平台** 的 `xlstatus-server` 与 `xlstatus-agent` 二进制及安装脚本：

| 操作系统 (`os_name`) | 架构 (`arch`) | Rust Target | 构建方式与附加检查 |
|---|---|---|---|
| `linux` | `x86_64` | `x86_64-unknown-linux-gnu` | `cargo build --release` + `GLIBC <= 2.36` 符号校验 + 打包 `install-server.sh` / `install-agent.sh` |
| `linux` | `arm64` | `aarch64-unknown-linux-gnu` | `cargo build --release` + `GLIBC <= 2.36` 符号校验 |
| `linux` | `i386` | `i686-unknown-linux-gnu` | `cargo zigbuild --release` |
| `windows` | `x86_64` | `x86_64-pc-windows-msvc` | MSVC `x64` (`xlstatus-*-windows-x86_64.exe`) |
| `windows` | `arm64` | `aarch64-pc-windows-msvc` | MSVC `arm64` (`xlstatus-*-windows-arm64.exe`) |
| `windows` | `i386` | `i686-pc-windows-msvc` | MSVC `x86` (`xlstatus-*-windows-i386.exe`) |
| `darwin` | `x86_64` | `x86_64-apple-darwin` | macOS Intel 原生构建 |
| `darwin` | `arm64` | `aarch64-apple-darwin` | macOS Apple Silicon 原生构建 |
| `freebsd` | `x86_64` | `x86_64-unknown-freebsd` | `cargo zigbuild --release` |
| `freebsd` | `i386` | `i686-unknown-freebsd` | `cargo zigbuild --release` |

---

## 5. 发布后验证步骤 (Post-Release Smoke)

1. 确认 GitHub Release 页面已完整上传 20 个二进制资产及 `install-server.sh`、`install-agent.sh`。
2. 在干净的 Linux 主机或容器中执行一键安装与 `/healthz` 验证：
   ```bash
   curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/install-server.sh | \
     sudo INTERACTIVE=false VERSION=v0.2 ADMIN_PASSWORD='SmokeTestStrongPassword!2026' bash
   curl -fsS http://127.0.0.1:8080/healthz
   ```
3. 登录 Web UI，生成注册令牌并验证 Agent 一键接入与 MCP Studio 调用正常。
