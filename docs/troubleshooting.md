# 故障排查指南 (Troubleshooting)

本文档按 **症状 (Symptom) -> 根因 (Cause) -> 修复步骤 (Fix)** 的结构，汇总 **XLStatus `v0.3`** 在启动、数据库、密钥校验、CORS/WebSocket、TOTP 二步验证、Agent gRPC、NAT 穿透以及 MCP AI 接口中的常见故障定位方法。

---

## 1. Server 启动后立即退出 (Startup Exits)

### 症状
前台运行 `xlstatus-server` 后未持续占用终端而是直接返回 Shell，或 `systemctl status xlstatus` 显示 `Result: exit-code`。

### 常见根因与修复

#### 原因 A：`8080` 或 `50051` 端口已被占用
日志中出现：
```text
Error: gRPC server failed
Caused by:
  failed to bind gRPC server to 127.0.0.1:50051
```
**修复**：
```bash
# 查找占用 8080 或 50051 的进程
sudo ss -tlnp | grep -E ':(8080|50051)\b' || true
```
停止冲突进程，或修改 `server.toml` / 环境变量中的 `http_bind` (`HTTP_BIND`) 与 `grpc_bind` (`GRPC_BIND`)（注意两者不能使用相同的 `IP:Port`）。

#### 原因 B：`DATABASE_URL` 覆盖了 `CONFIG_FILE`
你修改了 `/etc/xlstatus/server.toml`，但配置完全没生效。
**根因**：只要环境变量中存在 `DATABASE_URL`，`xlstatus-server` 就会强制进入环境变量模式并忽略 `CONFIG_FILE`。
**修复**：
检查 `sudo systemctl cat xlstatus` 或当前 Shell 环境，执行 `unset DATABASE_URL`，仅保留 `Environment="CONFIG_FILE=/etc/xlstatus/server.toml"`。

---

## 2. `SESSION_SECRET` 或 `SECRET_ENCRYPTION_KEY` 熵校验失败

### 症状
启动时立即报错退出：
- `SESSION_SECRET/security.session_secret is using an insecure default value`
- `SESSION_SECRET/security.session_secret must be at least 32 characters`
- `SESSION_SECRET/security.session_secret has insufficient entropy`
- `SECRET_ENCRYPTION_KEY/security.secret_encryption_key has insufficient entropy`
- 或启动时报错：`Secret migration failed: failed to decrypt secret`

### 根因
1. **启动安全强校验**：`v0.3` 强制要求 `SESSION_SECRET` 与 `SECRET_ENCRYPTION_KEY` 长度至少为 **32 字符**、至少包含 **4 个不同字节**（例如全 `a` 的 32 位字符串会被拒绝），且严禁使用 `replace-with-a-long-random-secret`、`CHANGE_ME_IN_PRODUCTION` 等文档占位符。
2. **密钥被意外更换**：如果升级或重装时更换了 `SECRET_ENCRYPTION_KEY`（或未配置 `SECRET_ENCRYPTION_KEY` 时更换了 `SESSION_SECRET`），数据库中已用旧密钥加密的 `xlsec:v1:...` 密文（DDNS、Cloudflared、GeoIP、TOTP）将无法解密。

### 修复
1. 新部署使用 `openssl rand -hex 32` 生成 64 位高熵十六进制随机串写入 `/etc/xlstatus/server.toml` 或 `.env`。
2. 如果是升级后出现 `failed to decrypt secret`，请从备份恢复升级前的 `/etc/xlstatus/server.toml` 或 `.env` 中的原始 `SECRET_ENCRYPTION_KEY` / `SESSION_SECRET`。

---

## 3. SQLite `mode=rwc` 与目录权限错误

### 症状
启动时报错无法打开或创建 SQLite 数据库文件（如 `unable to open database file` 或提示缺失数据库文件）。

### 根因
在非交互环境（systemd / Docker）下，若 SQLite 文件不存在，且既没有在 URL 中声明 `?mode=rwc` 也没有开启 `create_if_missing = true`（或 `DATABASE_CREATE_IF_MISSING=true`），Server 会拒绝静默建库以防路径拼写错误导致数据丢失；此外 systemd 沙箱要求数据目录归属 `xlstatus:xlstatus` 且列入 `ReadWritePaths`。

### 修复
1. 确保 `server.toml` 配置如下（注意绝对路径使用三个斜杠 `sqlite:///`）：
   ```toml
   [database]
   url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
   create_if_missing = true
   ```
2. 修复宿主机数据目录权限：
   ```bash
   sudo mkdir -p /var/lib/xlstatus
   sudo chown -R xlstatus:xlstatus /var/lib/xlstatus
   sudo chmod 700 /var/lib/xlstatus
   ```

---

## Web UI 和 CORS

### 症状
- 浏览器访问控制台提示 `Failed to fetch`。
- 用户名密码正确，登录接口返回 `200`，但页面不跳转或刷新后立即掉登录。
- 终端 (`/ws/terminal/:id`) 或实时监控 (`/ws/servers`) 连接立即断开（HTTP `403` `WebSocket Origin is not allowed`）。

### 根因与修复

#### 原因 A：前端 `NEXT_PUBLIC_API_URL` 仍指向 `localhost:8080`
在远端服务器部署时，若构建前端镜像时未设置 `XLSTATUS_PUBLIC_API_URL`（或 `NEXT_PUBLIC_API_URL`），浏览器可能会向用户自己电脑的 `localhost:8080` 发请求。
**修复**：
编辑 `.env` 设置浏览器可访问的真实后端地址，并**重新构建**前端镜像：
```env
XLSTATUS_PUBLIC_API_URL=https://status.example.com
XLSTATUS_CORS_ALLOWED_ORIGINS=https://status.example.com
```
```bash
docker compose build --no-cache web
docker compose up -d web server
```

#### 原因 B：后端 `CORS_ALLOWED_ORIGINS` 未包含前端精确 Origin 或误用 `*`
后端同时对 HTTP API 与 WebSocket 握手执行严格的 `Origin` 校验，且禁止通配符 `*`。
**验证命令**：
```bash
curl -i -H 'Origin: https://status.example.com' http://127.0.0.1:8080/api/v1/public/status
```
响应头必须包含：
```http
access-control-allow-origin: https://status.example.com
access-control-allow-credentials: true
```
**修复**：将浏览器地址栏的精确 Origin（含协议与非标准端口，不含末尾斜杠）加入 `CORS_ALLOWED_ORIGINS` 或 `server.cors_allowed_origins`，重启 `xlstatus-server`。

#### 原因 C：纯 HTTP 环境下开启了 `SECURITY_COOKIE_SECURE=true`
在 `env` 或 `config.toml` 模式下，`cookie_secure` 默认为 `true`。如果你在无 HTTPS 的内网通过 `http://192.168.1.10:3000` 访问，浏览器会拒绝保存和发送带 `Secure` 标记的 `xlstatus_session` 与 `xlstatus_csrf` Cookie，导致登录后立即掉线。
**修复**：生产环境请配置 Nginx/Caddy HTTPS；仅限本地/内网纯 HTTP 调试时，显式设置 `SECURITY_COOKIE_SECURE=false`（或 `cookie_secure = false`）。

---

## 5. TOTP 二步验证 (2FA) 锁定、高危操作 `403` 与 WAF 误封

### 症状
1. 登录时连续提示 TOTP 验证码无效，或账号因多次失败被 WAF 暂时封禁 (`429 Too Many Requests` / `403 Forbidden`)。
2. 管理员已登录，但执行数据库备份、恢复、创建 PAT 时返回 `403` 提示需要 TOTP 验证码。

### 根因与修复

1. **服务器时间漂移**：
   TOTP 基于 30 秒时间窗口计算。若服务器系统时钟偏差超过 30 秒，验证码必然失效。
   ```bash
   timedatectl status
   sudo timedatectl set-ntp true
   ```
2. **高危操作 Step-up 验证 (`X-TOTP-Code`)**：
   开启 2FA 的管理员调用 `/api/v1/maintenance/*`、`/api/v1/tokens` 等敏感接口时，必须在 HTTP 请求头携带当前 6 位动态码：`X-TOTP-Code: 123456`（Web UI 会自动弹出二次验证输入框）。
3. **反代未配置可信代理导致所有用户共享 `127.0.0.1` 触发 WAF 封禁**：
   若使用 Nginx/Caddy 反代但未设置 `XLSTATUS_TRUST_PROXY_HEADERS=1` 和 `XLSTATUS_TRUSTED_PROXIES=127.0.0.1,::1`，Server 看到的客户端 IP 全是 `127.0.0.1`，某人输错密码会导致全站限流。
   **应急解封与修复**：
   配置好可信代理环境变量后，若管理员自身 IP 已被封禁，可在宿主机清空 `waf_bans` 表对应记录：
   ```bash
   # SQLite 应急清理 WAF 封禁
   sudo sqlite3 /var/lib/xlstatus/xlstatus.db "DELETE FROM waf_bans;"
   ```

---

## 6. Agent 无法上线或 gRPC 连接断开

### 症状
管理后台显示节点 `offline`，或 `journalctl -u xlstatus-agent` 持续打印 `reconnecting in Ns`。

### 常见根因与修复

1. **Server gRPC 仅监听在回环地址 `127.0.0.1:50051`**：
   出于安全默认策略，Docker Compose (`XLSTATUS_GRPC_PUBLISH_ADDRESS=127.0.0.1`) 和 `install-server.sh` (`GRPC_BIND=127.0.0.1:50051`) 默认仅在回环地址暴露 gRPC。
   **修复**：若远端 Agent 需跨主机连接，请先按 [安装部署](./installation.md#6-生产环境-grpc-tls-与-mtls-双向认证配置) 配置 TLS/mTLS，再将 Server 的 `grpc_bind`（或 `XLSTATUS_GRPC_PUBLISH_ADDRESS`）改为 `0.0.0.0:50051`，并放行防火墙 `50051/tcp`。
2. **TLS / mTLS 证书域名 (`SAN`) 或客户端证书不匹配**：
   - 若 Server 启用了 `grpc_tls_cert_path`，Agent 的 `--grpc-server` 必须以 `https://` 开头，并通过 `--grpc-tls-ca-path` 指定根 CA、`--grpc-tls-domain-name` 匹配服务端证书 SAN。
   - 若 Server 启用了 `grpc_tls_client_ca_path` (mTLS)，Agent 必须同时提供 `--grpc-tls-client-cert-path` 与 `--grpc-tls-client-key-path`。
3. **Agent 已被吊销 (`force_disconnect`)**：
   若日志包含 `force_disconnect`，说明管理员已在后台吊销该节点。需在后台生成新的 `xle_...` 注册令牌，重新运行 `xlstatus-agent enroll`。

---

## 7. NAT 穿透公网端口报错或外部无法访问

### 症状
- 创建 NAT 映射时报错：`public_port ... is a privileged port (<= 1023)`、`below the configured NAT public port minimum` 或 `collides with a reserved control plane port`。
- 或报错：`NAT local_host must resolve to the Agent loopback interface`。
- 或映射创建成功，但从外部公网无法连接该 NAT 端口。

### 根因与修复

1. **特权端口与控制面保留端口保护**：
   - 公网端口严禁 `<= 1023`，且必须 `>= XLSTATUS_NAT_PUBLIC_PORT_MIN`（默认 `1024`）。
   - 公网端口严禁与控制面端口（`8080`、`50051`、`3000`、`5432` 及 `XLSTATUS_NAT_RESERVED_PORTS`）重合。
2. **Agent 本地转发目标默认仅限 Loopback**：
   出于防内网横向渗透考虑，NAT 映射的 `local_host` 默认只允许 `127.0.0.1`、`::1` 或 `localhost`。
3. **Server 端 NAT 默认仅绑定 `127.0.0.1` 且来源 IP 默认仅限回环**：
   `xlstatus-server` 的 `XLSTATUS_NAT_BIND_HOST` 默认为 `127.0.0.1`，`XLSTATUS_NAT_ALLOWED_SOURCES` 默认为 `127.0.0.0/8,::1/128`。
   **修复**：若需通过公网暴露 NAT 映射端口，请在 Server 环境变量中显式配置：
   ```env
   XLSTATUS_NAT_BIND_HOST=0.0.0.0
   # 全局放行公网连入，同时强烈建议在每条 NAT 映射上单独设置 allowed_sources 白名单 CIDR
   XLSTATUS_NAT_ALLOWED_SOURCES=0.0.0.0/0,::/0
   ```

---

## 8. MCP AI 客户端连接返回 `401` / `403` 或缺少工具

### 症状
- Claude Desktop / Cursor / Windsurf 连接 `/mcp` 返回 `HTTP 403` 或 `HTTP 401`。
- `tools/list` 中看不到 `server.exec` 或 `fs.write`。
- 调用 `fs.write`、`fs.delete` 或 `fs.upload_url` 时返回 `403 Forbidden`。

### 根因与修复

1. **`/mcp` 强制要求 `Authorization: Bearer xlp_...` (PAT 令牌)**：
   出于安全隔离设计，`/mcp` 与 `/api/v1/mcp/execute` **拒绝纯浏览器 Cookie 会话访问**（返回 `403`）。必须在 **MCP 工作台 (`/mcp`)** 或 **系统设置 -> 个人访问令牌** 中生成 `xlp_...` 令牌，并在请求头传入 `Authorization: Bearer xlp_...`（或传给 `scripts/xlstatus-mcp-stdio.mjs` 的 `XLSTATUS_PAT`）。
2. **`tools/list` 按 PAT Scope 动态裁剪**：
   `/mcp` 的 `tools/list` 只会返回当前 PAT 权限范围 (`scopes`) 允许调用的工具：
   - 需要查看节点/指标：勾选 `server:read`
   - 需要执行远程命令 (`server.exec`)：勾选 `server:exec`
   - 需要读取/列出/下载文件 (`fs.read`、`fs.list`、`fs.download_url`)：勾选 `transfer:read`
   - 同时检查创建 PAT 时是否绑定了 `server_ids` 服务器白名单（若绑定了白名单，则只能操作列表内的节点）。
3. **高危文件改动工具对 PAT 永久只读保护**：
   为防止 AI 客户端出现幻觉或提示词注入导致远端服务器文件被篡改或删库，**所有 PAT 令牌均被禁止通过 MCP 调用 `fs.write`、`fs.delete` 与 `fs.upload_url`**（在 `tools/list` 中隐藏且调用时返回 `403`）。文件写入与删除仅允许人类管理员在 Web 控制台或 MCP Studio 演练场中通过管理员会话执行。
