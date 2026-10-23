# 快速开始 (5 分钟上手指南)

本文档提供在 5 分钟内启动 **XLStatus `v0.2`** 完整控制面（Server + Web UI + Agent + MCP AI 桥接）的分步指南。你可以选择 **路径 A（Docker Compose 容器化启动）** 或 **路径 B（本地源码编译运行）**。

生产环境加固、反向代理与 systemd 守护进程部署请继续阅读 [安装部署](./installation.md) 与 [配置参考](./configuration.md)。

---

## 路径 A：Docker Compose 一键启动（推荐）

Docker Compose 默认将 HTTP API (`8080`)、Web UI (`3000`) 与 Agent gRPC (`50051`) 绑定在宿主机 `127.0.0.1` 回环地址，并通过 Docker Secret 文件挂载初始管理员密码，避免敏感凭据残留在环境变量或镜像层中。

### 1. 准备初始化密钥与 `.env`

在仓库根目录下创建 `.secrets/xlstatus_seed_admin_password` 与 `.env`：

```bash
# 1. 创建初始管理员密码文件（仅在管理员账号不存在时首次种子化使用）
mkdir -p .secrets
printf '%s\n' 'ReplaceWithStrongAdminPassword!2026' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password

# 2. 生成会话签名密钥、数据库字段加密主密钥与 PostgreSQL 密码
test -f .env || (
  umask 077
  printf 'SESSION_SECRET=%s\nSECRET_ENCRYPTION_KEY=%s\nPOSTGRES_PASSWORD=%s\n' \
    "$(openssl rand -hex 32)" \
    "$(openssl rand -hex 32)" \
    "$(openssl rand -hex 24)" > .env
)
```

> **重要提示**：
> - `SESSION_SECRET` 与 `SECRET_ENCRYPTION_KEY` 必须至少 32 字符且具备足够信息熵（至少 4 个不同字节）。升级或重启容器时**切勿重新生成或覆盖 `.env`**，否则会导致现有会话失效且数据库中已加密的第三方凭据（DDNS、Cloudflared、GeoIP、TOTP）无法解密。
> - 不要将首次管理员密码写入 `.env`，Compose 会通过 `/run/secrets/xlstatus_seed_admin_password` (`XLSTATUS_SEED_ADMIN_PASSWORD_FILE`) 安全读取。

### 2. 启动服务（二选一）

#### 方式 1：SQLite 默认版（轻量单机首选）

使用 `docker-compose.yml` 启动 `server` 与 `web`，SQLite 数据文件持久化在 `./data/xlstatus.db`：

```bash
docker compose up -d
curl -fsS http://127.0.0.1:8080/healthz
docker compose ps
```

#### 方式 2：PostgreSQL 生产版

使用 `docker-compose.pg.yml` 启动 `postgres` (PostgreSQL 15)、`server` 与 `web`（数据库端口仅绑定宿主机 `127.0.0.1:5432`）：

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://127.0.0.1:8080/healthz
docker compose -f docker-compose.pg.yml ps
```

### 3. 远端服务器运行 Docker Compose 的注意事项

如果你在远端 Linux 服务器上运行 Docker Compose 并通过浏览器远端访问，前端浏览器请求的是你配置的公网或内网地址而非 `localhost:8080`。请编辑 `.env`：

```env
SESSION_SECRET=<保持已生成的 64 位 hex 随机值>
SECRET_ENCRYPTION_KEY=<保持已生成的 64 位 hex 随机值>
POSTGRES_PASSWORD=<保持已生成的随机密码>
XLSTATUS_PUBLIC_API_URL=http://status.example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://status.example.com:3000,http://localhost:3000,http://127.0.0.1:3000
# 仅在已配置 gRPC TLS/mTLS 或可信 VPN 时才将 gRPC 暴露到 0.0.0.0
XLSTATUS_GRPC_PUBLISH_ADDRESS=127.0.0.1
```

修改 `XLSTATUS_PUBLIC_API_URL` 后必须重新构建 `web` 镜像（因为 `NEXT_PUBLIC_API_URL` 会在编译期注入前端产物）：

```bash
docker compose up -d --build
```

---

## 路径 B：本地从源码构建与运行

### 1. 环境要求

- **Rust**：Stable 工具链 (`cargo`, `rustc`) + `protoc` (Protobuf 编译器)
- **Node.js**：`20+`（推荐 Node 22/24），启用 `corepack enable` 与 `pnpm`
- **数据库**：SQLite 3（内置支持）或 PostgreSQL 15+

### 2. 编译 Server、Agent 与 Web 前端

```bash
# 编译后端与 Agent 二进制
cargo build --release --bin xlstatus-server --bin xlstatus-agent

# 安装前端依赖并构建生产包
corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

### 3. 启动 `xlstatus-server`

#### 方式 1：环境变量模式（SQLite）

```bash
mkdir -p ./data ./secrets
printf '%s\n' 'ReplaceWithStrongAdminPassword!2026' > ./secrets/admin_password
chmod 600 ./secrets/admin_password

DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
DATABASE_CREATE_IF_MISSING=true \
HTTP_BIND="127.0.0.1:8080" \
GRPC_BIND="127.0.0.1:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="$(openssl rand -hex 32)" \
SECRET_ENCRYPTION_KEY="$(openssl rand -hex 32)" \
SECURITY_COOKIE_SECURE=false \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD_FILE="$(pwd)/secrets/admin_password" \
./target/release/xlstatus-server
```

> **提示**：本地使用 `http://localhost:3000`（非 HTTPS）调试时，若采用环境变量或 `config.toml` 模式，请显式设置 `SECURITY_COOKIE_SECURE=false`（或 TOML 中 `cookie_secure = false`），否则浏览器不会在明文 HTTP 下发送带 `Secure` 属性的会话 Cookie。

#### 方式 2：配置文件模式 (`config.toml`)

```bash
cp config.example.toml ./config.toml
SESSION_SECRET_VAL="$(openssl rand -hex 32)"
SECRET_KEY_VAL="$(openssl rand -hex 32)"
sed -i.bak "s|sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc|sqlite://$(pwd)/data/xlstatus.db?mode=rwc|" ./config.toml
sed -i.bak "s/replace-with-a-long-random-secret/${SESSION_SECRET_VAL}/" ./config.toml
sed -i.bak "s/replace-with-a-different-long-random-secret/${SECRET_KEY_VAL}/" ./config.toml
sed -i.bak "s/cookie_secure = true/cookie_secure = false/" ./config.toml

CONFIG_FILE=./config.toml \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD_FILE="$(pwd)/secrets/admin_password" \
./target/release/xlstatus-server
```

> **配置互斥规则**：`DATABASE_URL` 与 `CONFIG_FILE` 不会合并。只要环境中存在 `DATABASE_URL`，Server 就会进入环境变量模式并忽略 `CONFIG_FILE`。详见 [配置参考](./configuration.md)。

另开终端验证健康检查：

```bash
curl -fsS http://127.0.0.1:8080/healthz
# 返回 OK
```

### 4. 启动 Next.js Web 前端 (`web`)

开发热重载模式：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

或生产模式运行（需先执行过 `pnpm build`）：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

---

## 首次使用：4 步走通完整闭环

服务启动后，各个入口地址如下：

- **公开状态页 (SSR)**：`http://localhost:3000/status`
- **管理控制台登录**：`http://localhost:3000/login`
- **后端健康检查**：`http://localhost:8080/healthz`
- **MCP JSON-RPC 2.0 端点**：`http://localhost:8080/mcp`

### 第 1 步：首次登录与安全设置

1. 打开浏览器访问 `http://localhost:3000/login`。
2. 输入管理员账号 `admin` 及你在 `.secrets/xlstatus_seed_admin_password`（或 `admin_password`）中设置的密码。
3. 登录后进入 **系统设置 (`/settings`) -> 账户与安全**，建议立即启用 **TOTP 两步验证 (2FA)**。启用后，所有高危运维操作（数据库备份恢复、创建 PAT、批量吊销等）都会要求输入动态验证码（`X-TOTP-Code`）。

### 第 2 步：注册并启动首个 Agent 节点

1. 在左侧导航进入 **系统设置 (`/settings`) -> Agent 接入**（或 **服务器 (`/servers`) -> 接入新节点**）。
2. 点击 **生成注册令牌 (Enrollment Token)**，获得形如 `xle_...` 的一次性注册令牌。
3. **Linux systemd 一键安装**（自动复制面板生成的带参命令）：
   ```bash
   curl -fsSL 'http://localhost:8080/api/v1/agents/install.sh?enrollment_token=xle_...&agent_name=node-01&version=v0.2' | sudo bash
   ```
4. **或者使用本地编译的 `xlstatus-agent` 手动注册并运行**：
   ```bash
   # 使用 --token-stdin 避免注册令牌暴露在 ps 进程命令行中
   printf '%s' 'xle_你的注册令牌' | ./target/release/xlstatus-agent enroll \
     --server http://127.0.0.1:8080 \
     --grpc-server http://127.0.0.1:50051 \
     --token-stdin \
     --name "$(hostname)" \
     --config ./agent.json

   # 启动 Agent 双向 gRPC 流与指标采集
   ./target/release/xlstatus-agent run --config ./agent.json
   ```
5. 返回控制台 **概览 (`/dashboard`)** 或 **服务器 (`/servers`)**，即可看到节点实时上线，并以秒级频率推送 CPU、内存、负载、磁盘、网络速率与连接数。

### 第 3 步：打开 Web 终端 (`/terminal`)

1. 点击左侧导航栏的 **终端 (`/terminal`)**，或在服务器详情页点击 **打开终端**。
2. 选择刚刚上线的 Agent 节点，点击 **新建终端会话**。
3. 浏览器通过 `/ws/terminal/:session_id` 与 Agent 的 `IoStream` 建立低延迟 PTY 通道，你可以直接在浏览器中执行交互式 Shell 命令、调整窗口大小或开启多标签会话。

### 第 4 步：1 键接入 AI 客户端 (`/mcp`)

XLStatus 内置原生 **Model Context Protocol (MCP `2024-11-05`)** 服务端与可视化工作台：

1. 点击左侧导航栏的 **MCP 工作台 (`/mcp`)**。
2. 在 **快速签发 MCP PAT** 卡片中勾选所需权限（如只读巡检 `server:read`、`service:read`、`alert:read`，或受控执行 `server:exec`、`transfer:read`），勾选允许访问的服务器范围，点击 **生成令牌**，复制生成的 `xlp_...` 个人访问令牌（仅显示一次）。
3. 在 **AI 客户端一键接入** 区域选择你的客户端（**Claude Desktop**、**Cursor**、**Windsurf**、**Cline**、**OpenCode** 或 **Antigravity**）：
   - **原生 Streamable HTTP 模式**：直接复制包含 `http://localhost:8080/mcp` 与 `Authorization: Bearer xlp_...` 的 JSON 配置粘贴到客户端。
   - **Stdio 桥接模式**：复制基于 `node scripts/xlstatus-mcp-stdio.mjs` 的配置，兼容仅支持 Stdio 的本地 AI 客户端。
4. 你也可以直接在页面右侧的 **工具演练场 (Studio Playground)** 中在线选择任意 MCP 工具（如 `cluster.diagnose`、`server.list`、`server.exec`），一键发起真实调用并查看结构化 JSON 响应与 cURL 命令。
