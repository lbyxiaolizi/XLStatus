---
title: 鉴权模型
status: stable
audience: [human, agent]
related_milestones: [M1, M2]
---

# 09. 鉴权模型

三层独立鉴权：Web Dashboard（密码+JWT+refresh rotation）、Agent gRPC（短期 JWT+Ed25519 challenge）、Agent 首次注册（一次性 enrollment token）。

## 密码学原语

| 算法 | 用途 | 库 |
|------|------|-----|
| **argon2id** | Web 用户密码哈希 | `argon2 = "0.5"`，OWASP 参数 m=19456, t=2, p=1 |
| **Ed25519** | Agent 私钥签名（challenge-response） | `ed25519-dalek = "2"` |
| **HS256** | Web access JWT / agent access JWT | `jsonwebtoken = "9"` |
| **SHA-256** | refresh token 哈希、challenge 哈希 | `sha2 = "0.10"` |
| **constant-time compare** | 全部 token / signature 比较 | `subtle = "2"` |

**全部 secret 走 env 注入**，`.env` 文件 git ignored，`.env.example` 列出变量名。

---

## A. Web Dashboard 鉴权（D3）

### 流程

```
┌──────────┐                              ┌──────────┐                ┌──────────┐
│ Browser  │ POST /auth/login             │  Server  │                │  users / │
│          │ { username, password }       │          │                │ user_    │
│          │─────────────────────────────▶│          │                │ sessions │
│          │                              │ argon2   │                │          │
│          │ Set-Cookie: access (15min)   │ verify   │                │          │
│          │ Set-Cookie: refresh (7d)     │          │                │          │
│          │◀─────────────────────────────│          │                │          │
└──────────┘                              │          │                │          │
                                          │ 写 sessions 行 (refresh hash) │
                                          │                            │
┌──────────┐                              │          │                │          │
│ Browser  │ GET /api/v1/agents           │ 解析 cookie                │          │
│          │ Cookie: access_token         │ 中间件注入 AuthUser         │          │
│          │─────────────────────────────▶│  200 + JSON                │          │
│          │◀─────────────────────────────│                            │          │
└──────────┘                              │                            │          │
                                          │                            │          │
┌──────────┐                              │          │                │          │
│ Browser  │ (access 过期)                 │ 401                       │          │
│          │ POST /auth/refresh           │ 解析 refresh cookie        │          │
│          │ Cookie: refresh_token        │ 校验 hash + 未撤销        │          │
│          │─────────────────────────────▶│ 轮换：旧 revoked + 新签发  │          │
│          │ Set-Cookie: access + refresh │                            │          │
│          │◀─────────────────────────────│                            │          │
└──────────┘                              │                            │          │
```

### Cookie 规格

| Cookie | 有效期 | HttpOnly | Secure | SameSite | Path |
|--------|--------|----------|--------|----------|------|
| `access_token` | 15 min | ✓ | ✓ | Lax | `/` |
| `refresh_token` | 7 days | ✓ | ✓ | Lax | `/` |

### Refresh Token Rotation

- refresh 是一次性 32 字节随机（base64url）
- DB 存 `sha256(refresh)`，不存明文
- 每次 refresh 调用：
  1. 校验 hash 匹配 + 未过期 + 未撤销
  2. 旧 session 标记 `revoked_at = now()`
  3. 创建新 session（新 refresh hash）
  4. 签新 access JWT
- **重放检测**：如果已 revoked 的 refresh token 又被使用 → 撤销该 user 全部 session（防 refresh 泄漏）

### 文件

- `crates/server/src/auth/password.rs` — argon2 包装
- `crates/server/src/auth/jwt.rs` — access JWT 签发/校验
- `crates/server/src/auth/session_cookie.rs` — cookie 读写
- `crates/server/src/api/auth.rs` — 4 个端点 handler

---

## B. Agent 鉴权（D4 + D9）

### 阶段 1：Enrollment（一次性）

```
┌──────────┐                          ┌──────────┐                  ┌──────────────┐
│  Admin   │ 登录 dashboard            │  Server  │                  │  agents /    │
│  Browser │ POST /agents              │          │                  │  enrollment_ │
│          │ { name: "devbox" }        │          │                  │  tokens      │
│          │─────────────────────────▶ │          │                  │              │
│          │◀─ { agent_id }            │          │                  │              │
│          │                           │          │                  │              │
│          │ POST /agents/{id}/        │ 生成 32B 随机 token        │              │
│          │       enrollment-token    │ 1h 过期 一次性             │              │
│          │─────────────────────────▶ │ 存 hash                   │              │
│          │◀─ { token: "et_xxx..." }  │                          │              │
└──────────┘                          │                          │              │
                                       │                          │              │
┌──────────┐                          │                          │              │
│  Agent   │ POST /agent/enroll        │ 校验 token               │              │
│  CLI     │ { token, name, public_key }│ 标记 used + used_by     │              │
│          │─────────────────────────▶ │ 创建 agents 行           │              │
│          │◀─ { agent_id, agent_jwt } │ 创建 agent_sessions 行   │              │
│          │                           │ 返回 5min JWT            │              │
└──────────┘ 存私钥到 /var/lib/        │                          │              │
            xlstatus/agent.key         │                          │              │
            (mode 0600)                │                          │              │
```

### 阶段 2：gRPC 持续连接

```
┌──────────┐                          ┌──────────┐
│  Agent   │ gRPC :50051               │  Server  │
│          │ metadata:                 │          │
│          │   authorization: Bearer   │ Interceptor:
│          │     <agent_jwt>           │   提取 JWT │
│          │ stream ClientMessage ───▶ │   验签    │
│          │ ◀── stream ServerMessage  │   注入 agent_id │
│          │                           │   到 extensions │
└──────────┘                          │          │
                                       │ Service:
                                       │   Hello 校验
                                       │   State 攒批写盘
                                       │   Task 下发
```

### 阶段 3：JWT 续签（5min 边界）

```
┌──────────┐                          ┌──────────┐
│  Agent   │ 4:55 收到 ServerMessage   │  Server  │
│          │ JwtChallenge { nonce }    │          │
│          │◀────────────────────────│          │
│          │                           │          │
│          │ sign = ed25519_sign(      │          │
│          │   priv,                  │          │
│          │   sha256("xlstatus-jwt-  │          │
│          │    refresh-v1:" + nonce))│          │
│          │ )                        │          │
│          │                           │          │
│          │ ClientMessage:            │          │
│          │   JwtRefreshRequest {     │          │
│          │     nonce, signature }    │          │
│          │─────────────────────────▶│ 验签     │
│          │                           │ 新 5min  │
│          │                           │ JWT 签发│
│          │                           │ (可选：推│
│          │                           │  Server-│
│          │                           │ Message │
│          │                           │ 含新 JWT│
│          │                           │  或让客 │
│          │                           │ 户端重 │
│          │                           │ 连)    │
└──────────┘                          │          │
```

### 阶段 4：吊销

```
Admin → DELETE /api/v1/agents/{id}/sessions
        ↓
Server:
  1. UPDATE agent_sessions SET revoked_at = NOW() WHERE agent_id = ? AND revoked_at IS NULL
  2. 通过 gRPC stream 推 ServerMessage: ForceDisconnect { AGENT_REVOKED }
  3. Agent 收到后清理连接、退出
```

### JWT 格式（agent_jwt.rs）

```rust
#[derive(Serialize, Deserialize)]
struct AgentClaims {
    sub: String,        // agent_id (UUID)
    iat: i64,           // issued at (unix seconds)
    exp: i64,           // iat + 300 (5 minutes)
    jti: String,        // UUID v7, 防重放
    scope: String,      // "agent.session"
}
```

### 私钥管理

- **私钥**：`/var/lib/xlstatus/agent.key`，32 字节 Ed25519 secret
- 权限：mode 0600（仅 root 可读）
- 内存中：`Zeroizing<SecretKey>` 包装，`Drop` 时清零
- **Server 端只存公钥**（`agents.public_key`）
- 私钥泄漏 → admin 调 `DELETE /agents/{id}/sessions` + 用户重新 `enroll`

### 为什么每条消息不签名

- 5 min JWT 短期 + 一次性 challenge 续签，安全性足够
- 高频 state 推送（10 Hz 可达）下每条签名会成为瓶颈
- gRPC 框架已对单连接做完整 TLS + stream 编号，本身有强完整性
- v1 不做，未来如需可在 service 内逐条签 + 验

---

## C. 传输安全（D13）

- **生产强制 HTTPS**：Caddy 反代 + 自动 ACME
- **Cookie**：`HttpOnly; Secure; SameSite=Lax; Path=/`
- **gRPC**：生产配置 Caddy stream 代理到 :50051（终结 TLS）；dev 可 `-plaintext`
- **HSTS 头**：`max-age=31536000; includeSubDomains; preload`
- **WebSocket**：wss://

---

## D. 防攻击（D14 + D15 + 其它）

| 风险 | 缓解 |
|------|------|
| 撞库 | argon2 + 登录限流 5/min/IP + 失败日志告警 |
| JWT 重放 | 5min 短期 + jti 去重 + 可选 revoked_jti 缓存 |
| Agent JWT 重放 | 5min 短期 + `force_disconnect` 即时吊销 |
| CSRF | `SameSite=Lax` cookie + 自定义 header (`X-Requested-With`) |
| XSS | React 默认转义 + `HttpOnly` cookie + CSP 头 |
| Agent 私钥泄漏 | 落盘 0600 + `zeroize` + admin 吊销 |
| 越权 | axum 中间件按角色 + resource 范围校验 |
| 弱密码 | 注册/改密时 zxcvbn 强度校验（v1 简单规则：≥10 字符 + 字母数字） |
| 敏感日志 | 不记录 token、密码、private key |
| DoS | `tower_governor` 限流 + 单 agent_id 单连接 |

---

## E. 环境变量清单

```bash
# .env.example

# 必需
XLSTATUS_JWT_SECRET=             # HS256 secret，32+ 字节随机
XLSTATUS_DB_URL=                 # sqlite://data.db 或 postgres://...

# 可选
XLSTATUS_HTTP_BIND=0.0.0.0:8080
XLSTATUS_GRPC_BIND=0.0.0.0:50051
XLSTATUS_TLS_CERT=                # 生产 PEM
XLSTATUS_TLS_KEY=                 # 生产 PEM
XLSTATUS_LOG_LEVEL=info           # trace/debug/info/warn/error
XLSTATUS_ARGON2_MEM_KIB=19456     # 默认 OWASP
XLSTATUS_ARGON2_TIME_COST=2
XLSTATUS_ARGON2_PARALLELISM=1
XLSTATUS_AGENT_JWT_TTL_S=300      # 默认 5min
XLSTATUS_WEB_ACCESS_TTL_S=900     # 15min
XLSTATUS_WEB_REFRESH_TTL_S=604800 # 7d
XLSTATUS_RATE_LIMIT_LOGIN=5
XLSTATUS_RATE_LIMIT_API=1000
```