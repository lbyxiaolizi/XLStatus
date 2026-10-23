# 配置参考手册

本文档提供 **XLStatus `v0.2`** 服务端 (`xlstatus-server`)、Web 前端 (`web/`) 与 Agent 的完整配置参考，涵盖配置加载优先级、完整 `config.toml` 示例、全量环境变量参考表、应用层密钥加密 (`HKDF-SHA256` + `AES-256-GCM`) 以及启动诊断脱敏规则。

---

## 1. 配置加载优先级与互斥规则 (`Config::load`)

`xlstatus-server` 启动时严格按以下三步顺序决定配置来源（定义于 [`crates/server/src/config.rs`](../crates/server/src/config.rs)）：

1. **模式 1：环境变量模式 (`DATABASE_URL` 已设置)**
   只要进程环境变量中存在 `DATABASE_URL`，Server 立即进入**纯环境变量模式**，从环境变量读取 `HTTP_BIND`、`GRPC_BIND`、`SESSION_SECRET`、`SECRET_ENCRYPTION_KEY`、`OIDC_*` 等参数，并执行生产安全校验 (`validate_deployment_security`)。
2. **模式 2：TOML 配置文件模式 (`DATABASE_URL` 未设置，且配置文件存在)**
   当未设置 `DATABASE_URL` 时，Server 检查 `CONFIG_FILE` 环境变量（未设置时默认为当前目录下的 `config.toml`）。若该路径存在，则解析该 TOML 文件并执行生产安全校验。
3. **模式 3：本地开发回退默认模式**
   当 `DATABASE_URL` 未设置且 `CONFIG_FILE`（或 `./config.toml`）不存在时，回退到本地开发默认配置（`sqlite://dev.db`、`127.0.0.1:8080`、`127.0.0.1:50051`、`cookie_secure = false`）。

> **核心互斥规则（切勿混用）**：
> **环境变量模式 (`DATABASE_URL`) 与 TOML 配置文件模式 (`CONFIG_FILE`) 绝不会合并！**
> 如果你在 systemd 单元或 shell 环境中设置了 `DATABASE_URL`，那么 `CONFIG_FILE=/etc/xlstatus/server.toml` 会被完全忽略。使用 `server.toml` 部署时，请确保环境中**没有**设置 `DATABASE_URL`。
> （注：一次性管理员种子变量 `XLSTATUS_SEED_ADMIN_*`、反向代理信任变量 `XLSTATUS_TRUST_PROXY_HEADERS` / `XLSTATUS_TRUSTED_PROXIES`、NAT 策略变量 `XLSTATUS_NAT_*` 与出站策略变量独立于 `Config::load()`，在两种模式下均可由环境变量控制。）

---

## 2. 完整 `config.toml` 注解示例

推荐从仓库根目录的 `config.example.toml` 复制并修改权限为 `0600`：

```bash
sudo cp config.example.toml /etc/xlstatus/server.toml
sudo chown xlstatus:xlstatus /etc/xlstatus/server.toml
sudo chmod 600 /etc/xlstatus/server.toml
```

以下为包含所有配置段（`[database]`、`[server]`、`[security]`、`[oauth2]`，以及运行时 `[tsdb]` 与 `[rate_limit]` 策略说明）的完整注解示例：

```toml
# ============================================================================
# [database] 数据库连接配置
# ============================================================================
[database]
# SQLite 示例（推荐使用三斜杠绝对路径 + ?mode=rwc）：
url = "sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc"
# 为 true 时，若 SQLite 文件或父目录不存在则首次启动自动创建
create_if_missing = true

# PostgreSQL 示例（需预先创建数据库与角色）：
# url = "postgresql://xlstatus:ReplaceWithStrongDbPassword@127.0.0.1:5432/xlstatus"
# create_if_missing = false

# ============================================================================
# [server] HTTP API、Agent gRPC、TLS/mTLS 与 CORS 配置
# ============================================================================
[server]
# HTTP API 与 WebSocket 监听地址（端口号必须在 1..=65535，且不可与 grpc_bind 相同）
http_bind = "127.0.0.1:8080"

# Agent 双向 gRPC 与 IoStream 监听地址
# 默认建议绑定 127.0.0.1:50051；公网直连时改为 "0.0.0.0:50051" 并启用下方 TLS/mTLS
grpc_bind = "127.0.0.1:50051"

# 可选：gRPC 服务端 TLS 证书与私钥（PEM 格式，启用 TLS 时两项必须同时设置）
# grpc_tls_cert_path = "/etc/xlstatus/tls/grpc-server.crt"
# grpc_tls_key_path  = "/etc/xlstatus/tls/grpc-server.key"

# 可选：gRPC 客户端根 CA 证书（PEM 格式，设置后启用双向 mTLS，要求提供了 cert/key）
# grpc_tls_client_ca_path = "/etc/xlstatus/tls/agent-ca.crt"

# 是否启用 gRPC Server Reflection（生产环境建议保持 false，仅在内网 grpcurl 调试时开启）
grpc_reflection_enabled = false

# 允许跨域访问 API 及建立 WebSocket (/ws/*) 的精确浏览器 Origin 列表
# 严禁填写 "*"（系统使用 Cookie 会话与 CSRF 保护，通配符会被直接拒绝）
cors_allowed_origins = [
  "http://localhost:3000",
  "http://127.0.0.1:3000",
  "https://status.example.com",
]

# ============================================================================
# [security] 会话签名、Cookie 安全属性与存储加密密钥
# ============================================================================
[security]
# 会话 Cookie 与 Agent 短期 JWT 的签名主密钥
# 要求：长度 >= 32 字符，不同字节数 >= 4，且不能为示例占位符
# 生成命令：openssl rand -hex 32
session_secret = "replace-with-a-64-char-hex-secret-generated-by-openssl-rand-hex-32"

# 数据库敏感字段（DDNS API Token、Cloudflared Token、GeoIP Token、TOTP 密钥）加密主密钥
# 若省略，则回退使用 session_secret；生产环境强烈建议独立设置并离线备份
# 生成命令：openssl rand -hex 32
secret_encryption_key = "replace-with-another-64-char-hex-secret-from-openssl-rand-hex-32"

# 登录会话有效期（小时），合法范围：1 ..= 8760 (1 年)
session_ttl_hours = 24

# 是否为会话与 CSRF Cookie 附加 Secure 属性
# 生产 HTTPS 部署保持 true；本地纯 HTTP (http://localhost:3000) 调试时设为 false
cookie_secure = true

# ============================================================================
# [oauth2] OAuth2 / OIDC 单点登录配置（可选）
# ============================================================================
[oauth2]
# OAuth2 登录或绑定完成后的前端回调地址
frontend_redirect_url = "https://status.example.com/oauth/callback"

# 支持配置一个或多个 OIDC/OAuth2 提供商
[[oauth2.providers]]
id = "oidc"
display_name = "企业 SSO"
auth_url = "https://idp.example.com/oauth2/authorize"
token_url = "https://idp.example.com/oauth2/token"
userinfo_url = "https://idp.example.com/oauth2/userinfo"
client_id = "xlstatus-web"
client_secret = "replace-with-oidc-client-secret"
redirect_url = "https://status.example.com/api/v1/auth/oauth/oidc/callback"
scopes = ["openid", "profile", "email"]
# 可选："client_secret_post"（默认）或 "client_secret_basic"
token_auth_method = "client_secret_post"
# 可选："bearer"（默认）
userinfo_auth_method = "bearer"
subject_field = "sub"
email_field = "email"
name_field = "name"
username_field = "preferred_username"

[oauth2.providers.extra_auth_params]
# prompt = "select_account"

# ============================================================================
# [tsdb] 与 [rate_limit] 运行时策略说明（通过系统设置 API / 环境变量生效）
# ============================================================================
# 注：TSDB 保留期与限流参数由服务端内置安全策略及数据库 `system_settings` 动态管理，
# 无需写入 TOML（TOML 解析器忽略未声明段或通过 API 在线调整）：
#
# - [tsdb] 时序存储：
#   默认保留期为 30 天（合法范围 1..=3650 天），后台每 60 分钟自动执行一次过期样本压缩
#   (TSDB_COMPACTION_INTERVAL = 3600s)，也可在管理后台「系统设置 -> 维护与备份」中在线修改
#   `POST /api/v1/maintenance/tsdb/retention` 或手动触发 `POST /api/v1/maintenance/tsdb/compact`。
#
# - [rate_limit] 速率限制与防爆破：
#   登录接口、TOTP 验证、Agent 注册、MCP 调用、临时文件传输 URL 均内置按可信客户端 IP 的
#   滑动窗口限流与 WAF 自动封禁机制；NAT 端口映射支持按映射配置连接数/带宽/字节窗口限流。
```

---

## 3. 全量环境变量参考表

### 3.1 核心控制面配置 (`Config::load` 环境变量模式)

| 环境变量 | 默认值 | 必填 / 说明 |
|---|---|---|
| `DATABASE_URL` | 无（设置后触发环境变量模式） | **模式开关**。例如 `sqlite:///var/lib/xlstatus/xlstatus.db?mode=rwc` 或 `postgresql://xlstatus:pwd@127.0.0.1:5432/xlstatus`。 |
| `CONFIG_FILE` | `config.toml` | 当 `DATABASE_URL` **未设置**时读取的 TOML 配置文件路径。 |
| `DATABASE_CREATE_IF_MISSING` | `false` | 仅对 SQLite 生效。设为 `1`、`true`、`yes`、`y`、`on` 时允许自动创建缺失的数据库文件与父目录。 |
| `HTTP_BIND` | `127.0.0.1:8080` | HTTP API、WebSocket 与 `/mcp` 监听套接字地址。端口范围 `1..=65535`，不得与 `GRPC_BIND` 冲突。 |
| `GRPC_BIND` | `127.0.0.1:50051` | Agent gRPC 监听套接字地址。Docker 容器内设为 `0.0.0.0:50051`，宿主机侧由 `XLSTATUS_GRPC_PUBLISH_ADDRESS` 控制发布接口。 |
| `GRPC_TLS_CERT_PATH` | 空 | 可选。gRPC 服务端 PEM 证书路径，必须与 `GRPC_TLS_KEY_PATH` 同时设置。 |
| `GRPC_TLS_KEY_PATH` | 空 | 可选。gRPC 服务端 PEM 私钥路径，必须与 `GRPC_TLS_CERT_PATH` 同时设置。 |
| `GRPC_TLS_CLIENT_CA_PATH` | 空 | 可选。gRPC 客户端证书根 CA 路径。设置后开启双向 mTLS（要求已设置服务端证书与私钥）。 |
| `GRPC_REFLECTION_ENABLED` | `false` | 是否开启 gRPC 反射服务（供 `grpcurl list` 调试使用，生产环境保持 `false`）。 |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000,http://[::1]:3000` | 英文逗号分隔的前端精确 Origin 列表。严禁包含 `*`。 |
| `SESSION_SECRET` | 无（生产必填） | 会话与 Agent JWT 签名密钥。长度必须 `>= 32` 字符且不同字节数 `>= 4`，严禁使用示例占位符。 |
| `SECRET_ENCRYPTION_KEY` | 回退至 `SESSION_SECRET` | 数据库敏感字段加密主密钥。建议单独生成 64 位 hex 随机串并妥善备份。 |
| `SESSION_TTL_HOURS` | `24` | Cookie 会话过期时间（小时），合法范围 `1..=8760`。 |
| `SECURITY_COOKIE_SECURE` | `true`（环境变量/TOML 模式） | 是否为会话与 CSRF Cookie 添加 `Secure` 标志。本地纯 HTTP 调试需设为 `false`。 |
| `XLSTATUS_ALLOW_INSECURE_DEFAULTS` | 未启用 | **已废弃/禁止用于绕过密钥校验**：当前 `v0.2` 强制执行 `validate_deployment_security()`，任何低于 32 字符、低熵或命中弱默认占位符的密钥都会直接阻止进程启动。 |

### 3.2 管理员种子化、反向代理与出站安全环境变量

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `XLSTATUS_SEED_ADMIN_USERNAME` | 空（兼容别名 `ADMIN_USERNAME`） | 首次启动时自动创建的管理员用户名（若同名用户已存在则跳过，不覆盖现有密码）。 |
| `XLSTATUS_SEED_ADMIN_PASSWORD_FILE` | 空（兼容别名 `ADMIN_PASSWORD_FILE`） | **推荐**。包含初始管理员密码的文件路径（自动去除末尾换行，禁止空文件）。优先于明文环境变量。 |
| `XLSTATUS_SEED_ADMIN_PASSWORD` | 空（兼容别名 `ADMIN_PASSWORD`） | 初始管理员明文密码环境变量。容器与 systemd 部署请优先使用 `XLSTATUS_SEED_ADMIN_PASSWORD_FILE`。 |
| `XLSTATUS_TRUST_PROXY_HEADERS` | `false` | 设为 `1` / `true` 时，允许从命中 `XLSTATUS_TRUSTED_PROXIES` 的反向代理读取 `X-Forwarded-For`、`X-Real-IP`、`CF-Connecting-IP`、`X-Forwarded-Proto` 与 `Forwarded`。 |
| `XLSTATUS_TRUSTED_PROXIES` | 空 | 逗号或空白分隔的可信反向代理 IP 或 CIDR 列表（如 `127.0.0.1,::1,10.0.0.0/8`）。未配置时忽略所有代理转发头。 |
| `XLSTATUS_ALLOW_PRIVATE_WEBHOOKS` | `false` | Webhook 专用私网出站开关。设为 `1` / `true` 时，仅允许告警通知 Webhook 与 DDNS Webhook 访问私网/loopback 地址，不影响服务探测、OAuth 或 GeoIP。 |
| `XLSTATUS_ALLOW_PRIVATE_OUTBOUND` | `false` | **高危全局开关**。设为 `1` / `true` 时允许所有服务端出站路径（HTTP/TCP/ICMP 探测、OAuth、GeoIP、DDNS、Webhook）访问私网地址。仅限受控内网环境使用。 |
| `XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL` | `false` | 默认 `force-update` 仅允许从官方 GitHub Release 下载二进制；设为 `1` / `true` 时允许管理员指定自定义 HTTPS 升级包 URL。 |
| `XLSTATUS_DATA_DIR` | `data` | 数据目录路径（用于默认 GeoIP MMDB 文件存储等）。 |
| `XLSTATUS_GEOIP_MMDB_PATH` | `<XLSTATUS_DATA_DIR>/GeoLite2-City.mmdb` | 自定义本地 MaxMind GeoIP `.mmdb` 数据库文件路径。 |
| `XLSTATUS_GEOIP_MMDB_URL` | 空 | 在线更新 GeoIP 数据库时使用的默认下载 URL。 |
| `XLSTATUS_NOTIFICATION_PROVIDER_PRESETS` | 空 | 可选的 JSON 格式通知渠道预设配置。 |

### 3.3 NAT 穿透与端口转发控制变量

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `XLSTATUS_NAT_BIND_HOST` | `127.0.0.1` | Server 端为 NAT 映射监听公网端口时绑定的宿主 IP（生产对外暴露 NAT 端口时可设为 `0.0.0.0`）。 |
| `XLSTATUS_NAT_PUBLIC_PORT_MIN` | `1024` | 允许创建的 NAT 映射最小公网端口号（特权端口 `<= 1023` 始终禁止）。 |
| `XLSTATUS_NAT_RESERVED_PORTS` | `8080,50051,3000,5432` 等控制面端口 | 额外禁止被 NAT 映射占用的保留端口列表（逗号或空白分隔）。 |
| `XLSTATUS_NAT_MAX_ACTIVE_TUNNELS` | `1024` | 单台 Server 允许同时维持的全局最大并发活跃 NAT 隧道数。 |
| `XLSTATUS_NAT_ALLOWED_SOURCES` | `127.0.0.0/8,::1/128` | 全局允许连入 NAT 公网端口的来源 CIDR 列表（公网开放 NAT 时需设为 `0.0.0.0/0,::/0` 并在单条映射上配置细粒度 `allowed_sources`）。 |
| `XLSTATUS_AGENT_MAX_NAT_TUNNELS` | `256` | **Agent 端变量**：单个 Agent 允许并发打开的最大本地转发流数量。 |

### 3.4 OAuth2 / OIDC 环境变量（环境变量模式）

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `OAUTH2_FRONTEND_REDIRECT_URL` | `http://localhost:3000/oauth/callback` | OAuth 登录/绑定完成后跳回的前端 URL。 |
| `OIDC_ENABLED` | `false` | 设为 `true` 时从环境变量加载单个 OIDC 提供商配置。 |
| `OIDC_PROVIDER_ID` | `oidc` | 提供商唯一标识 ID。 |
| `OIDC_DISPLAY_NAME` | 空 | 登录按钮显示的名称。 |
| `OIDC_AUTH_URL` | 必填（当 `OIDC_ENABLED=true`） | IdP 授权端点 URL。 |
| `OIDC_TOKEN_URL` | 必填（当 `OIDC_ENABLED=true`） | IdP Token 交换端点 URL。 |
| `OIDC_USERINFO_URL` | 必填（当 `OIDC_ENABLED=true`） | IdP UserInfo 端点 URL。 |
| `OIDC_CLIENT_ID` | 必填（当 `OIDC_ENABLED=true`） | OAuth2 Client ID。 |
| `OIDC_CLIENT_SECRET` | 空 | OAuth2 Client Secret。 |
| `OIDC_REDIRECT_URL` | 必填（当 `OIDC_ENABLED=true`） | 后端回调 URL（如 `https://status.example.com/api/v1/auth/oauth/oidc/callback`）。 |
| `OIDC_SCOPES` | `openid,profile,email` | 逗号或空格分隔的 OAuth Scope 列表。 |
| `OIDC_TOKEN_AUTH_METHOD` | `client_secret_post` | Token 端点认证方式（`client_secret_post` 或 `client_secret_basic`）。 |
| `OIDC_USERINFO_AUTH_METHOD` | `bearer` | UserInfo 请求认证方式。 |
| `OIDC_EXTRA_AUTH_PARAMS` | 空 | 附加授权参数，支持 JSON 对象或 `k1=v1,k2=v2` 格式。 |
| `OIDC_SUBJECT_FIELD` / `OIDC_EMAIL_FIELD` / `OIDC_NAME_FIELD` / `OIDC_USERNAME_FIELD` | `sub` / `email` / `name` / `preferred_username` | UserInfo JSON 字段映射名。 |

### 3.5 Web 前端与 Docker Compose 专用变量

| 环境变量 | 作用位置 | 说明 |
|---|---|---|
| `NEXT_PUBLIC_API_URL` | `web/` 构建与运行期 | 浏览器端请求后端 API 与 WebSocket 的根地址（如 `https://status.example.com` 或 `http://localhost:8080`）。若为空，浏览器默认请求当前页面主机名的 `8080` 端口。**修改后必须重新执行 `pnpm build` 或 `docker compose build web`**。 |
| `XLSTATUS_PUBLIC_API_URL` | `.env` (Docker Compose) | Compose 构建 `web` 镜像时传给 `NEXT_PUBLIC_API_URL` 的构建参数。 |
| `XLSTATUS_CORS_ALLOWED_ORIGINS` | `.env` (Docker Compose) | Compose 传给 `server` 容器 `CORS_ALLOWED_ORIGINS` 的前端源列表。 |
| `XLSTATUS_GRPC_PUBLISH_ADDRESS` | `.env` (Docker Compose) | Compose 将 `50051` 端口发布到宿主机的绑定 IP，默认 `127.0.0.1`。 |
| `XLSTATUS_URL` / `XLSTATUS_PAT` | `scripts/xlstatus-mcp-stdio.mjs` | 本地 MCP Stdio 桥接脚本连接的 Server 地址与 `xlp_...` 个人访问令牌。 |

---

## 4. 数据库存储加密 (`HKDF-SHA256` + `AES-256-GCM`) 与密钥管理

XLStatus 在应用层对写入数据库的第三方凭据与二步验证密钥进行透明加密（实现于 [`crates/server/src/secrets.rs`](../crates/server/src/secrets.rs)）：

1. **受保护的数据范围**：
   - `ddns_configs` 表中的 `api_token`、`api_key`、`api_secret`、`webhook_url`
   - `system_settings` 表中的 `geoip_ipinfo_token`、`cloudflared_token`
   - `users` 表中的 `totp_secret`
2. **密钥派生与算法 (`HKDF-SHA256` -> `AES-256-GCM`)**：
   - 主密钥材料优先取自 `SECRET_ENCRYPTION_KEY`（或 `security.secret_encryption_key`）；若未配置则回退使用 `SESSION_SECRET`。
   - 启动时通过 **`HKDF-SHA256`**（固定盐 `xlstatus.secrets.salt.v1` 与上下文信息 `XLStatus secret encryption key v1`）派生出 256 位（32 字节）对称密钥，且该密钥在 `SecretCrypto` 析构时通过 `write_volatile` 强制清零。
   - 每次加密均使用系统密码学随机数生成器 (`SystemRandom`) 产生唯一 96-bit Nonce，绑定附加认证数据 (`AAD = "xlstatus.secrets.v1"`)，输出带版本前缀的密文格式（`xlsec:v1:<24位hex_nonce>:<hex_ciphertext_and_tag>`，兼容 `enc:v2:` 密钥派生规范）。
   - 明文上限为 `16 KiB` (`MAX_SECRET_PLAINTEXT_BYTES`)，在分配内存前严格校验密文长度与十六进制格式，抵御超大密文内存耗尽攻击。
3. **启动自动迁移 (`migrate_plaintext_secrets`)**：
   - 每次 `xlstatus-server` 启动并完成数据库迁移后，会自动扫描上述表中尚未加密的历史明文记录，并将其原地升级为加密格式。

---

## 5. 启动诊断与日志自动脱敏规则 (Secret Redaction)

为了防止配置错误或数据库连接失败时将敏感凭据泄露到 `journalctl`、容器日志或标准错误输出，`xlstatus-server` 实施了严格的启动脱敏策略（由集成测试 [`crates/server/tests/startup_secret_redaction.rs`](../crates/server/tests/startup_secret_redaction.rs) 验证）：

- **结构化 `Debug` 输出脱敏**：
  `ServerConfig`、`DatabaseConfig`、`SecurityConfig`、`OidcProviderConfig`、`SecretCrypto` 和 `DdnsSecretColumns` 均手写了自定义 `fmt::Debug` 实现。在任何日志中打印配置结构体时，`database.url`、`session_secret`、`secret_encryption_key`、`grpc_tls_key_path`、`client_secret` 与 `extra_auth_params` 均固定输出为 `"[REDACTED]"`。
- **TOML 解析错误脱敏**：
  若 `config.toml` 存在语法错误（例如未闭合引号的 `url = "postgresql://user:secret@..."`），解析器统一返回 `"Failed to parse configuration file"`，绝不在错误信息中回显原始文件片段或数据库密码。
- **数据库连接与 `PGPASSFILE` 错误脱敏**：
  连接数据库时仅记录后端类型标签（`database_backend = "sqlite"` 或 `"postgresql"`），并在 SQLx 底层解析连接串或 `.pgpass` 失败前拦截并擦除用户名、密码、`sslpassword` 等敏感参数。
