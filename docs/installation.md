# 安装部署指南 (生产环境)

本文档提供 **XLStatus `v0.2`** 的完整生产环境部署指南，涵盖 Docker Compose 三种拓扑、Linux systemd 一键安装脚本（含原子暂存、健康检查自动回滚与沙箱加固）、PostgreSQL 生产库初始化、Nginx/Caddy 反向代理以及 gRPC TLS/mTLS 双向认证配置。

---

## 1. 平台与环境要求

| 组件 / 方式 | 平台与版本要求 |
|---|---|
| **Linux systemd 安装器** (`install-server.sh` / `install-agent.sh`) | Linux `x86_64`、`arm64` (`aarch64`)、`i386` (`i686`)，GLIBC `<= 2.36` 兼容（Ubuntu 22.04+ / Debian 12+ / RHEL 9+） |
| **多平台预编译二进制** (GitHub Releases) | Linux (`x86_64`, `arm64`, `i386`)、Windows (`x86_64`, `arm64`, `i386` `.exe`)、macOS (`x86_64`, `arm64`)、FreeBSD (`x86_64`, `i386`) |
| **Docker 部署** | Docker `20.10+` 与 Docker Compose `v2` |
| **数据库后端** | SQLite `3.40+`（内置 WAL + 外键约束）或 PostgreSQL `15+` |
| **源码编译** | Rust Stable 工具链、`protoc` (Protobuf 编译器)、Node.js `20+`、Corepack、`pnpm 10+` |

---

## 2. Docker Compose 生产部署

仓库内置三套开箱即用的 Docker Compose 编排文件，按需选择：

| 编排文件 | 包含组件 | 适用场景 |
|---|---|---|
| `docker-compose.yml` | `server` (SQLite) + `web` + 可选 `agent-demo` profile | 标准单机生产/内网部署，数据持久化至 `./data/xlstatus.db` |
| `docker-compose.pg.yml` | `postgres` (PG 15) + `server` + `web` + 可选 `agent-demo` profile | 高并发/大规模节点生产部署，带独立 PostgreSQL 数据卷与健康检查 |
| `docker-compose.simple.yml` | 仅 `server` (SQLite，命名卷 `xlstatus-data`) | 仅部署后端控制面 API/gRPC（如前端托管在外部或仅用 API/MCP） |

### 2.1 初始化 Docker Secret 与环境变量

所有 Compose 编排均强制使用 Docker Secret 文件 `/run/secrets/xlstatus_seed_admin_password` 注入首次管理员密码，**绝不将管理员密码写入 `.env` 或容器环境变量**：

```bash
# 1. 创建仅属主可读的管理员初始密码文件
mkdir -p .secrets
printf '%s\n' 'ReplaceWithStrongAdminPassword!2026' > .secrets/xlstatus_seed_admin_password
chmod 700 .secrets
chmod 600 .secrets/xlstatus_seed_admin_password

# 2. 复制并配置 .env
cp .env.example .env
chmod 600 .env
```

编辑 `.env`（生产环境必须使用高熵随机密钥，且在后续升级中保持不变）：

```env
# 使用 openssl rand -hex 32 生成，长度 >= 32 字符且不同字节数 >= 4
SESSION_SECRET=8f4b2e9a1c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f
SECRET_ENCRYPTION_KEY=1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b
POSTGRES_PASSWORD=9d8c7b6a5f4e3d2c1b0a9f8e7d6c5b4a3f2e1d0c9b8a7f6e5d4c3b2a

# 浏览器可访问的后端 API 公网地址（编译进 Next.js 前端 bundle）
XLSTATUS_PUBLIC_API_URL=https://status.example.com

# 允许跨域访问 API 与建立 WebSocket 的精确前端 Origin 列表
XLSTATUS_CORS_ALLOWED_ORIGINS=https://status.example.com

# 宿主机发布 Agent gRPC 50051 端口的绑定地址（默认 127.0.0.1）
# 仅在已启用 gRPC TLS/mTLS 或位于可信内网/VPN 时改为 0.0.0.0
XLSTATUS_GRPC_PUBLISH_ADDRESS=127.0.0.1
```

### 2.2 启动与验证

- **SQLite 全栈 (`docker-compose.yml`)**：
  ```bash
  docker compose up -d --build
  curl -fsS http://127.0.0.1:8080/healthz
  ```
- **PostgreSQL 全栈 (`docker-compose.pg.yml`)**：
  ```bash
  docker compose -f docker-compose.pg.yml up -d --build
  curl -fsS http://127.0.0.1:8080/healthz
  ```
- **仅 Server (`docker-compose.simple.yml`)**：
  ```bash
  docker compose -f docker-compose.simple.yml up -d --build
  curl -fsS http://127.0.0.1:8080/healthz
  ```

> **安全边界说明**：
> - `docker-compose.yml` 与 `docker-compose.pg.yml` 默认将 HTTP API (`127.0.0.1:8080:8080`)、Web UI (`127.0.0.1:3000:3000`) 与 PostgreSQL (`127.0.0.1:5432:5432`) 仅发布在宿主机回环接口，建议由宿主机 Nginx / Caddy 统一提供 HTTPS 接入。
> - 修改 `.env` 中的 `XLSTATUS_PUBLIC_API_URL` 后，必须执行 `docker compose up -d --build web` 重新构建前端镜像，仅 `restart` 容器不会更新前端静态 bundle。

---

## 3. Linux systemd 一键安装 (`install-server.sh` & `install-agent.sh`)

GitHub Release (`v0.2`) 随附 Linux systemd 自动化安装与升级脚本 `install-server.sh`（仓库内路径 `deploy/install.sh`）与 `install-agent.sh`（仓库内路径 `deploy/install-agent.sh`）。

### 3.1 一键安装 Server (`install-server.sh`)

#### 交互式安装

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/install-server.sh | sudo VERSION=v0.2 bash
```

在交互式终端下，脚本会依次引导配置版本号、安装目录、数据目录、监听地址、数据库后端（SQLite / PostgreSQL）、CORS 允许源、密钥以及初始管理员密码。

#### 无人值守安装 (`INTERACTIVE=false`)

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/install-server.sh | \
  sudo INTERACTIVE=false \
    VERSION=v0.2 \
    HTTP_BIND=127.0.0.1:8080 \
    GRPC_BIND=127.0.0.1:50051 \
    DATABASE_URL="sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc" \
    DATABASE_CREATE_IF_MISSING=true \
    CORS_ALLOWED_ORIGINS="https://status.example.com" \
    ADMIN_USERNAME=admin \
    ADMIN_PASSWORD='ReplaceWithStrongAdminPassword!2026' \
    bash
```

若使用本地源码编译好的二进制安装，可传入 `BINARY_PATH=target/release/xlstatus-server bash deploy/install.sh`。

#### 默认安装路径与文件权限

| 路径 | 用途 | 权限与属主 |
|---|---|---|
| `/opt/xlstatus/xlstatus-server` | Server 实际二进制文件 | `0755 root:root` |
| `/usr/local/bin/xlstatus-server` | 指向 `/opt/xlstatus/xlstatus-server` 的符号链接 | symlink |
| `/etc/xlstatus/server.toml` | Server 配置文件（含高熵密钥） | `0600 xlstatus:xlstatus` |
| `/var/lib/xlstatus` | SQLite 数据库与运行工作目录 | `xlstatus:xlstatus` |
| `/etc/systemd/system/xlstatus.service` | systemd 服务单元文件 | `0644 root:root` |
| `/run/xlstatus/bootstrap.env` | 首次启动一次性管理员种子密码文件（健康检查通过后立即销毁） | `0600 root:root` |

#### `install-server.sh` 环境变量一览

| 变量 | 默认值 | 说明 |
|---|---|---|
| `VERSION` | `v0.2` | 下载的 GitHub Release 版本标签（支持 `latest`） |
| `BINARY_PATH` | 空 | 本地已编译二进制路径；设置后跳过网络下载 |
| `INSTALL_DIR` | `/opt/xlstatus` | Server 二进制存放目录 |
| `DATA_DIR` | `/var/lib/xlstatus` | 数据与工作目录 |
| `CONFIG_FILE` | `/etc/xlstatus/server.toml` | 生成或保留的 TOML 配置文件路径 |
| `HTTP_BIND` | `127.0.0.1:8080` | HTTP API 监听地址 |
| `GRPC_BIND` | `127.0.0.1:50051` | Agent gRPC 监听地址 |
| `DATABASE_URL` | `sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc` | SQLite 或 PostgreSQL 连接串 |
| `DATABASE_CREATE_IF_MISSING` | `true` | SQLite 库文件不存在时是否自动创建 |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000` | 逗号分隔的前端 Origin 列表（严禁使用 `*`） |
| `SESSION_SECRET` | 自动生成 64 位 hex | 会话与 Agent JWT 签名密钥 |
| `SECRET_ENCRYPTION_KEY` | 自动生成 64 位 hex | 数据库存储加密主密钥 |
| `ADMIN_USERNAME` | `admin` | 初始管理员用户名 |
| `ADMIN_PASSWORD` | 空 | 初始管理员密码（通过临时 `/run/xlstatus/bootstrap.env` 注入并自动擦除） |
| `INTERACTIVE` | `auto` | 设为 `false` 禁用交互提示 |
| `RECONFIGURE` | `false` | 升级时默认 `false` 保留已有配置与密钥；设为 `true` 强制重写配置 |
| `INSTALL_DEPS` | `true` | 是否自动通过 `apt-get` / `yum` 安装 `curl`、`ca-certificates`、`sqlite3` |
| `START_SERVICE` | `true` | 安装后是否立即启动/重启 `xlstatus` 并执行健康检查 |

---

### 3.2 一键安装 Agent (`install-agent.sh`)

#### 方式 1：从GitHub Release 安装并自动注册

先在管理后台生成 `xle_...` 注册令牌，然后在目标节点执行：

```bash
curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/install-agent.sh | \
  sudo VERSION=v0.2 \
    SERVER_URL=https://status.example.com \
    GRPC_SERVER=https://status.example.com:50051 \
    ENROLLMENT_TOKEN=xle_你的注册令牌 \
    AGENT_NAME="$(hostname)" \
    bash
```

#### 方式 2：通过 Server 参数化 Bootstrap 端点安装

管理后台 **系统设置 -> Agent 接入** 会生成指向当前主控 `/install-agent.sh` 或 `/api/v1/agents/install.sh` 的一键命令：

```bash
curl -fsSL 'https://status.example.com/api/v1/agents/install.sh?enrollment_token=xle_...&agent_name=node-01&version=v0.2' | sudo bash
```

> **Bootstrap 安全防护机制**：
> - `/install-agent.sh` 与 `/api/v1/agents/install.sh` 属于公开端点，响应强制附带 `Cache-Control: no-store`。
> - 为防止反射注入与跨站重定向，查询串最大 `16 KiB`、`Host` 头最大 `512 B`、控制面 URL 最大 `2048 B`，且显式传入的 `server_url` 和 `grpc_server` 的主机名**必须与当前请求 `Host` 完全一致**（端口可不同），严禁包含路径、查询参数、片段、凭据或控制字符。
> - 若主控位于 HTTPS 反向代理后，请启用 `XLSTATUS_TRUST_PROXY_HEADERS=1` 并设置 `XLSTATUS_TRUSTED_PROXIES`，使 Bootstrap 自动推导 `https://` 地址。
> - `install-agent.sh` 会通过标准输入 (`printf '%s' "$ENROLLMENT_TOKEN" | xlstatus-agent enroll --token-stdin ...`) 传递注册令牌，避免令牌暴露在宿主机 `ps` 进程列表中。

---

### 3.3 原子暂存 (Atomic Staging) 与健康检查失败自动回滚

`install-server.sh` 和 `install-agent.sh` 均采用事务级文件变更与自动回滚机制（由 `test-run/verify-install-upgrade-safety.sh` 持续回归验证）：

1. **二进制预检与原子替换**：
   - 下载或复制的新二进制先写入目标目录同文件系统的临时文件（如 `/opt/xlstatus/.xlstatus-server.XXXXXX`）。
   - 校验文件非空且具备合法 Linux ELF 魔数（`7f454c46`）；Agent 安装器还会额外执行 `--help` 探测以确认宿主机 `glibc` 兼容性。
   - 校验通过后备份旧文件（保留符号链接或普通文件属性），再通过 `mv -f` 原子替换目标路径。
2. **升级配置保护 (`RECONFIGURE=false`)**：
   - 重复执行 `install-server.sh` 升级时，若 `/etc/xlstatus/server.toml` 已存在且未显式指定 `RECONFIGURE=true`，脚本会**完整保留现有 `server.toml` 与 `xlstatus.service`**，防止误轮换 `SESSION_SECRET` 或 `SECRET_ENCRYPTION_KEY` 造成历史加密数据失效。
3. **一次性管理员密码擦除**：
   - 若传入 `ADMIN_PASSWORD`，脚本仅将其写入权限为 `0600` (`root:root`) 的 `/run/xlstatus/bootstrap.env`。
   - 当服务启动并通过 `/healthz` 检查完成管理员种子化后，脚本立即删除 `/run/xlstatus/bootstrap.env` 并再次执行 `systemctl restart xlstatus`，确保运行中进程环境不再持有明文密码。
4. **故障自动回滚 (`rollback_install` / `rollback_agent_install`)**：
   - 如果 `systemctl start/restart` 失败，或在 40 秒内 `/healthz` 未返回成功响应，`EXIT` trap 会立即触发回滚：恢复旧二进制、符号链接、旧配置文件、旧 systemd 单元文件，恢复服务原本的 `enabled`/`active` 状态，并重新验证旧版本 `/healthz` 恢复正常。

---

### 3.4 systemd 沙箱安全加固 (Unit Sandboxing)

仓库提供的 `deploy/xlstatus-server.service`、`deploy/xlstatus-agent.service` 以及安装器生成的单元文件默认启用 systemd 最小权限沙箱。生产环境建议保持或对齐以下安全指令：

```ini
[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory=/var/lib/xlstatus
ExecStart=/usr/local/bin/xlstatus-server
Restart=on-failure
RestartSec=5s
UMask=0077

# 仅设置 CONFIG_FILE，不要混入 DATABASE_URL
Environment="CONFIG_FILE=/etc/xlstatus/server.toml"
Environment="RUST_LOG=info"

# systemd 安全沙箱指令
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictRealtime=true
LockPersonality=true
ReadWritePaths=/var/lib/xlstatus
```

| 沙箱指令 | 防护作用 |
|---|---|
| `NoNewPrivileges=true` | 禁止进程及其子进程通过 `setuid`/`setgid` 或文件 capability 获取新特权 |
| `PrivateTmp=true` | 为服务分配独立私有的 `/tmp` 与 `/var/tmp` 命名空间，服务停止后自动清理 |
| `ProtectSystem=strict` | 将整个根文件系统挂载为只读，仅显式列出的 `ReadWritePaths` 可写 |
| `ProtectHome=true` | 将 `/home`、`/root` 和 `/run/user` 设为不可访问 |
| `ProtectKernelTunables=true` | 禁止修改 `/proc/sys`、`/sys` 等内核运行参数 |
| `ProtectKernelModules=true` | 禁止加载或卸载内核模块 |
| `ProtectControlGroups=true` | 将 `/sys/fs/cgroup` 挂载为只读 |
| `RestrictSUIDSGID=true` | 禁止在文件系统上创建带 `setuid`/`setgid` 位的二进制文件 |
| `LockPersonality=true` | 锁定进程执行域 (`personality` 系统调用) |
| `UMask=0077` | 确保服务新创建的数据库、临时备份或日志默认仅属主可读写 (`0600`/`0700`) |

---

## 4. PostgreSQL 生产数据库初始化

XLStatus 会在启动时自动执行内置数据库迁移（位于 `crates/server/migrations/postgres/`），但不会自动创建 PostgreSQL 角色与空数据库。

### 4.1 创建专用数据库与角色

```bash
sudo -u postgres psql <<'SQL'
CREATE USER xlstatus WITH PASSWORD 'ReplaceWithStrongPostgresPassword!2026';
CREATE DATABASE xlstatus OWNER xlstatus;
GRANT ALL PRIVILEGES ON DATABASE xlstatus TO xlstatus;
SQL
```

连通性测试：

```bash
psql 'postgresql://xlstatus:ReplaceWithStrongPostgresPassword!2026@127.0.0.1:5432/xlstatus' -c 'SELECT 1;'
```

### 4.2 配置 `server.toml` 使用 PostgreSQL

```toml
[database]
url = "postgresql://xlstatus:ReplaceWithStrongPostgresPassword!2026@127.0.0.1:5432/xlstatus"
create_if_missing = false
```

如果从现有 SQLite 实例迁移到 PostgreSQL，请先让相同版本的 `xlstatus-server` 连接目标 PostgreSQL 自动建好空表结构，再使用 `scripts/import_sqlite_to_postgres.py` 进行事务级流式导入。完整步骤、表白名单校验与原子回滚说明见 [运维手册](./operations.md#sqlite-到-postgresql-迁移)。

---

## 5. 反向代理 (Nginx / Caddy) 与可信代理配置

生产环境推荐将 Web UI (`127.0.0.1:3000`) 与 HTTP API (`127.0.0.1:8080`) 置于 Nginx 或 Caddy 之后，通过统一 HTTPS 域名对外服务。

### 5.1 启用后端可信代理头解析

当 `xlstatus-server` 位于反向代理后方时，必须显式告知 Server 信任代理转发的真实客户端 IP（`X-Forwarded-For` / `X-Real-IP` / `CF-Connecting-IP`）与协议头（`X-Forwarded-Proto` / `Forwarded`），以便登录限流、WAF 封禁、审计日志和 Agent Bootstrap 链接正确工作：

在 systemd 单元或环境变量中配置：

```env
XLSTATUS_TRUST_PROXY_HEADERS=1
# 仅填入反向代理所在宿主机或容器网段（支持精确 IP 或 CIDR）
XLSTATUS_TRUSTED_PROXIES=127.0.0.1,::1,172.16.0.0/12
```

> **安全机制**：只有当直接连接 `xlstatus-server` 的 TCP 对端 IP (`peer_addr`) 命中 `XLSTATUS_TRUSTED_PROXIES` 且 `XLSTATUS_TRUST_PROXY_HEADERS=1` 时，Server 才会采信长度受限（`<= 1024 B`）的转发头；未命中可信代理的伪造 `X-Forwarded-For` 会被直接忽略。

### 5.2 Nginx 配置示例（同域名分路径转发 + WebSocket + gRPC）

```nginx
server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name status.example.com;

    ssl_certificate     /etc/ssl/certs/status.example.com.pem;
    ssl_certificate_key /etc/ssl/private/status.example.com.key;

    # 1. API、健康检查、MCP 与 Agent 安装引导转发至 8080
    location ~ ^/(api|mcp|healthz|install-agent\.sh)(/|$) {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        client_max_body_size 520m; # 支持最大 512 MiB 的 SQLite 在线备份恢复与文件传输
    }

    # 2. 实时指标流与 Web 终端 WebSocket (/ws/servers, /ws/terminal/:id)
    location /ws/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header Origin $http_origin;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
    }

    # 3. 其余前端路由转发至 Next.js (3000)
    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### 5.3 Caddy 配置示例 (`Caddyfile`)

```caddyfile
status.example.com {
    @backend {
        path /api/* /mcp /mcp/* /healthz /install-agent.sh /ws/*
    }
    handle @backend {
        reverse_proxy 127.0.0.1:8080
    }
    handle {
        reverse_proxy 127.0.0.1:3000
    }
}
```

使用同域名反向代理时，构建前端镜像或静态包请将 `NEXT_PUBLIC_API_URL` 设为 `https://status.example.com`，并将 `https://status.example.com` 加入后端的 `cors_allowed_origins`。

---

## 6. 生产环境 gRPC TLS 与 mTLS 双向认证配置

当远端 Agent 跨公网直连 `xlstatus-server` 的 `50051` 端口时，**强烈建议启用 gRPC TLS 或 mTLS（双向证书认证）**。

### 6.1 生成自签名 CA、Server 证书与 Agent 客户端证书

```bash
sudo mkdir -p /etc/xlstatus/tls
cd /etc/xlstatus/tls

# 1. 生成根 CA
openssl req -x509 -newkey rsa:4096 -days 3650 -nodes \
  -keyout ca.key -out ca.crt -subj "/CN=XLStatus Root CA"

# 2. 生成 Server gRPC 证书（SAN 必须包含 Agent 连接的域名或 IP）
openssl req -newkey rsa:4096 -nodes \
  -keyout grpc-server.key -out grpc-server.csr \
  -subj "/CN=status.example.com" \
  -addext "subjectAltName=DNS:status.example.com,IP:203.0.113.10"

openssl x509 -req -in grpc-server.csr -days 825 \
  -CA ca.crt -CAkey ca.key -CAcreateserial \
  -copy_extensions copyall -out grpc-server.crt

# 3. （可选，用于 mTLS）生成 Agent 客户端证书
openssl req -newkey rsa:4096 -nodes \
  -keyout agent-client.key -out agent-client.csr \
  -subj "/CN=xlstatus-agent"

openssl x509 -req -in agent-client.csr -days 825 \
  -CA ca.crt -CAkey ca.key -CAcreateserial \
  -out agent-client.crt

sudo chown -R xlstatus:xlstatus /etc/xlstatus/tls
sudo chmod 600 /etc/xlstatus/tls/*.key
```

### 6.2 在 Server 端启用 TLS / mTLS

在 `/etc/xlstatus/server.toml` 中配置：

```toml
[server]
http_bind = "127.0.0.1:8080"
grpc_bind = "0.0.0.0:50051"
# 必须同时配置 cert_path 和 key_path 才能启用 gRPC TLS
grpc_tls_cert_path = "/etc/xlstatus/tls/grpc-server.crt"
grpc_tls_key_path = "/etc/xlstatus/tls/grpc-server.key"
# 可选：配置客户端 CA 后强制开启 mTLS，拒绝未携带合法客户端证书的 Agent 连接
grpc_tls_client_ca_path = "/etc/xlstatus/tls/ca.crt"
grpc_reflection_enabled = false
cors_allowed_origins = ["https://status.example.com"]
```

如果使用 `systemd` 沙箱，确认 `xlstatus` 用户对 `/etc/xlstatus/tls/` 具有读取权限，随后重启服务：

```bash
sudo systemctl restart xlstatus
```

### 6.3 在 Agent 端配置 TLS / mTLS 接入

将 `ca.crt`（以及启用 mTLS 时的 `agent-client.crt` 和 `agent-client.key`，私钥权限设为 `0600`）分发到 Agent 节点的 `/etc/xlstatus-agent/tls/` 目录，并在注册或运行 `install-agent.sh` 时传入对应参数：

```bash
sudo SERVER_URL="https://status.example.com" \
  GRPC_SERVER="https://status.example.com:50051" \
  GRPC_TLS_CA_PATH="/etc/xlstatus-agent/tls/ca.crt" \
  GRPC_TLS_DOMAIN_NAME="status.example.com" \
  GRPC_TLS_CLIENT_CERT_PATH="/etc/xlstatus-agent/tls/agent-client.crt" \
  GRPC_TLS_CLIENT_KEY_PATH="/etc/xlstatus-agent/tls/agent-client.key" \
  ENROLLMENT_TOKEN="xle_..." \
  AGENT_NAME="$(hostname)" \
  bash deploy/install-agent.sh
```

或者直接使用 `xlstatus-agent enroll` CLI 参数：

```bash
printf '%s' 'xle_...' | sudo xlstatus-agent enroll \
  --server https://status.example.com \
  --grpc-server https://status.example.com:50051 \
  --grpc-tls-ca-path /etc/xlstatus-agent/tls/ca.crt \
  --grpc-tls-domain-name status.example.com \
  --grpc-tls-client-cert-path /etc/xlstatus-agent/tls/agent-client.crt \
  --grpc-tls-client-key-path /etc/xlstatus-agent/tls/agent-client.key \
  --token-stdin \
  --name "$(hostname)" \
  --config /etc/xlstatus-agent/agent.json
```
