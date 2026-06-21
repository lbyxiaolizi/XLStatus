# 安装部署

本文档覆盖当前仓库可用的部署路径：Docker Compose、源码运行、systemd 安装脚本、PostgreSQL 新站、Web UI 和 Agent。

## 平台要求

- Linux x86_64 / arm64 / i386：systemd 安装脚本当前支持的平台。
- Windows、macOS、FreeBSD：GitHub Release 发布 Server 和 Agent 二进制，可手动运行或自行接入系统服务。
- Docker 20.10+ 和 Docker Compose v2：用于容器部署。
- Rust 工具链：用于从源码构建 Server 和 Agent。
- Node.js 20+、Corepack、pnpm：用于构建 `web/`。
- SQLite 3.40+ 或 PostgreSQL 15+。

## Docker Compose

```bash
docker compose up -d
curl -fsS http://localhost:8080/healthz
```

PostgreSQL：

```bash
docker compose -f docker-compose.pg.yml up -d
curl -fsS http://localhost:8080/healthz
```

本地访问：

- Web UI: `http://localhost:3000`
- API: `http://localhost:8080`
- Public Status: `http://localhost:3000/status`

远端 Docker Compose 部署时，前端页面运行在用户浏览器里，不能让浏览器请求
`localhost:8080`。复制环境变量示例并填入浏览器可访问的公网地址：

```bash
cp .env.example .env
```

`.env` 示例：

```env
XLSTATUS_PUBLIC_API_URL=http://example.com:8080
XLSTATUS_CORS_ALLOWED_ORIGINS=http://example.com:3000,http://localhost:3000,http://127.0.0.1:3000
```

然后重新构建 Web 镜像：

```bash
docker compose up -d --build
curl -fsS http://example.com:8080/healthz
```

`XLSTATUS_PUBLIC_API_URL` 会作为 `NEXT_PUBLIC_API_URL` 写入 Next.js 浏览器 bundle。
如果之前已经用错误地址构建过，必须重新 build，单纯重启容器不会修改前端 bundle。

## 从源码构建

```bash
cargo build --release --bin xlstatus-server
cargo build --release --bin xlstatus-agent

corepack enable
cd web
pnpm install --frozen-lockfile
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
cd ..
```

前端构建时的 `NEXT_PUBLIC_API_URL` 会进入浏览器 bundle。生产环境请设置为用户浏览器能访问的 API 地址；远端访问时通常不是 `localhost`。

## GitHub Actions 自动构建

仓库包含 GitHub Actions workflow：

- PR 和 `main` push：运行 Rust 格式、workspace check/test、Web lint/build。
- tag `v*`：构建多平台 Server/Agent release 二进制，并发布 GitHub Release 资产。

Release 资产名称：

```text
xlstatus-server-<os>-<arch>[.exe]
xlstatus-agent-<os>-<arch>[.exe]
install-server.sh
install-agent.sh
```

当前 release 矩阵覆盖：

```text
linux:   x86_64, arm64, i386
windows: x86_64, arm64, i386
darwin:  x86_64, arm64
freebsd: x86_64, i386
```

Rust stable 当前没有 macOS i386 和 FreeBSD arm64 release target，因此这两个组合不会产出资产。
Windows 资产带 `.exe` 后缀。`install-server.sh` 和 `install-agent.sh` 是 Linux systemd 安装脚本，只随 Linux x86_64 构建上传一次。

安装脚本默认从下面路径下载二进制：

```text
https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/xlstatus-server-linux-<arch>
https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/xlstatus-agent-linux-<arch>
```

## systemd 安装 Server

Release 安装脚本默认下载 `v0.1.0-alpha.3` 的 Linux 二进制，并按当前机器选择 `x86_64`、`arm64` 或 `i386`：

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/install-server.sh | sudo bash
```

如果需要安装本地构建产物：

```bash
cargo build --release --bin xlstatus-server
sudo BINARY_PATH=target/release/xlstatus-server \
  ADMIN_USERNAME=admin \
  ADMIN_PASSWORD='replace-with-a-strong-initial-password' \
  CORS_ALLOWED_ORIGINS='http://localhost:3000,http://127.0.0.1:3000' \
  bash deploy/install.sh
```

如果直接运行：

```bash
sudo bash deploy/install.sh
```

脚本会进入交互式配置流程，依次询问安装目录、端口、数据库、CORS、管理员初始化和是否启动服务。无人值守安装时使用环境变量，并设置 `INTERACTIVE=false` 跳过提示：

```bash
sudo INTERACTIVE=false \
  VERSION=v0.1.0-alpha.3 \
  HTTP_BIND=127.0.0.1:8080 \
  GRPC_BIND=127.0.0.1:50051 \
  DATABASE_URL=sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc \
  DATABASE_CREATE_IF_MISSING=true \
  CORS_ALLOWED_ORIGINS=https://status.example.com \
  ADMIN_PASSWORD='replace-with-a-strong-initial-password' \
  bash deploy/install.sh
```

默认安装位置：

- 二进制：`/usr/local/bin/xlstatus-server`
- 配置：`/etc/xlstatus/server.toml`
- 数据：`/var/lib/xlstatus`
- 服务：`/etc/systemd/system/xlstatus.service`

运维命令：

```bash
sudo systemctl status xlstatus
sudo journalctl -u xlstatus -n 100 --no-pager
curl -fsS http://localhost:8080/healthz
```

脚本会把 `CORS_ALLOWED_ORIGINS` 写入 TOML 的 `server.cors_allowed_origins`，启动失败时会打印最近的 systemd 日志。

## 自定义安装参数

常用变量：

```bash
VERSION=v0.1.0-alpha.3
INSTALL_DIR=/opt/xlstatus
DATA_DIR=/var/lib/xlstatus
BINARY_PATH=target/release/xlstatus-server
CONFIG_FILE=/etc/xlstatus/server.toml
HTTP_BIND=127.0.0.1:8080
GRPC_BIND=127.0.0.1:50051
DATABASE_URL=sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc
DATABASE_CREATE_IF_MISSING=true
CORS_ALLOWED_ORIGINS=https://status.example.com
SESSION_SECRET="$(openssl rand -hex 32)"
ADMIN_USERNAME=admin
ADMIN_PASSWORD=replace-with-a-strong-initial-password
```

安装脚本会生成完整 `server.toml`。如果要走 `CONFIG_FILE` 模式，不要在 systemd unit 里额外设置 `DATABASE_URL`。

## PostgreSQL 新站

XLStatus 会执行应用迁移，但不会创建 PostgreSQL 用户和数据库。新站需要先准备：

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'change-this-password';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

验证连接：

```bash
psql 'postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' -c 'select 1;'
```

源码前台运行：

```bash
DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
HTTP_BIND="127.0.0.1:8080" \
GRPC_BIND="127.0.0.1:50051" \
CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
SESSION_SECRET="$(openssl rand -hex 32)" \
XLSTATUS_SEED_ADMIN_USERNAME="admin" \
XLSTATUS_SEED_ADMIN_PASSWORD="replace-with-a-strong-initial-password" \
./target/release/xlstatus-server
```

systemd 安装：

```bash
sudo BINARY_PATH=target/release/xlstatus-server \
  DATABASE_URL='postgresql://xlstatus:change-this-password@localhost:5432/xlstatus' \
  DATABASE_CREATE_IF_MISSING=false \
  CORS_ALLOWED_ORIGINS='https://status.example.com' \
  ADMIN_PASSWORD='replace-with-a-strong-initial-password' \
  bash deploy/install.sh
```

新站数据库应为空。恢复备份时，请使用和备份匹配的应用版本并先在测试环境验证。

## Web UI 部署

开发：

```bash
cd web
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

生产式运行：

```bash
cd web
NEXT_PUBLIC_API_URL=https://api.example.com pnpm build
NEXT_PUBLIC_API_URL=https://api.example.com pnpm start
```

如果不设置 `NEXT_PUBLIC_API_URL`，浏览器端默认使用当前页面主机名和 `8080`
端口，例如打开 `http://example.com:3000` 时会请求
`http://example.com:8080`。有反向代理、HTTPS 或 API 独立域名时，请显式设置
`NEXT_PUBLIC_API_URL` 并重新构建。

如果 Web UI 是 `https://status.example.com`，后端 CORS 必须包含这个精确来源：

```toml
[server]
cors_allowed_origins = ["https://status.example.com"]
```

## Agent 安装

先创建 enrollment token，然后：

```bash
sudo SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=http://dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/install-agent.sh | bash'
```

如果需要安装本地构建的 Agent：

```bash
cargo build --release --bin xlstatus-agent
sudo BINARY_PATH=target/release/xlstatus-agent \
  SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=http://dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash deploy/install-agent.sh
```

安装后检查：

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

后台“设置”页可以生成 enrollment token，并给出完整安装命令。真正的 Agent 安装脚本放在 GitHub Release 中，Server 只生成带参数的 bootstrap 链接，把 `SERVER_URL`、`GRPC_SERVER`、`ENROLLMENT_TOKEN`、`AGENT_NAME` 和 `VERSION` 注入后再拉取 GitHub 脚本。

带参数 bootstrap 是公开端点，因此 `SERVER_URL` 和 `GRPC_SERVER` 只能指向与本次请求 `Host` 相同的主机，端口可以不同。公开 query 最长 16KiB，请求 `Host` authority 最长 512 字节，控制面 URL 最长 2048 字节，回显到脚本的 token、Agent 名称和 TLS 参数会做长度校验并拒绝控制字符。需要连接到不同主机名时，请直接使用 GitHub Release 中的 `install-agent.sh`，通过环境变量传入控制面地址。

Server 提供的带参数入口：

```text
GET /install-agent.sh
GET /api/v1/agents/install.sh
```

手动使用带参数链接：

```bash
curl -fsSL 'http://dashboard.example.com:8080/api/v1/agents/install.sh?server_url=http%3A%2F%2Fdashboard.example.com%3A8080&grpc_server=http%3A%2F%2Fdashboard.example.com%3A50051&enrollment_token=xle_...&agent_name=%24(hostname)&version=v0.1.0-alpha.3' | sudo bash
```

这个 bootstrap 会下载并执行：

```text
https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/install-agent.sh
```

`enrollment_token` 会出现在安装链接里，应只给受信任的主机使用；令牌过期或使用后需要重新生成。

## 远端 Linux 验证

在目标服务器上：

```bash
git pull --ff-only
cargo build --release --bin xlstatus-server
mkdir -p ./data
timeout 8s env \
  DATABASE_URL="sqlite://$(pwd)/data/xlstatus.db?mode=rwc" \
  DATABASE_CREATE_IF_MISSING=true \
  HTTP_BIND="127.0.0.1:8080" \
  GRPC_BIND="127.0.0.1:50051" \
  CORS_ALLOWED_ORIGINS="http://localhost:3000,http://127.0.0.1:3000" \
  SESSION_SECRET="$(openssl rand -hex 32)" \
  XLSTATUS_SEED_ADMIN_USERNAME="admin" \
  XLSTATUS_SEED_ADMIN_PASSWORD="replace-with-a-strong-initial-password" \
  ./target/release/xlstatus-server
echo $?
```

退出码 `124` 代表服务持续运行到 timeout。退出码 `1` 或日志里的 `failed to bind` 通常是端口占用或绑定地址错误。
