# 安全设计

## 身份认证

浏览器：

- 使用 HttpOnly、SameSite Cookie session。
- 登录密码使用 Argon2id。
- 支持 token_version，修改密码或强制登出时使旧 session 失效。

PAT：

- 明文前缀使用 `xlp_`。
- 只在创建时返回明文。
- 存储 hash。
- 支持过期时间、吊销、last_used_at、last_used_ip。

Agent：

- 首次注册使用一次性 enrollment token，默认 1 小时过期，使用后立即失效。
- Agent 本地生成 Ed25519 keypair，私钥保存在配置目录，权限建议 0600。
- Server 只保存 Agent public key，不保存私钥。
- Agent gRPC 会话使用 5 分钟短期 JWT，通过 metadata `authorization: bearer <jwt>` 传递。
- JWT 接近过期时，Server 下发 nonce challenge，Agent 用私钥签名后换取新 JWT。
- TLS 默认建议开启，生产部署必须通过 HTTPS/gRPC TLS 或可信反代加密。
- Agent 吊销后，Server 标记 session revoked 并通过 gRPC 推送 `ForceDisconnect`。

验收标准：

- PAT 不能访问个人资料修改、PAT 自我管理、密码修改接口。
- Agent 认证失败不暴露 server 是否存在。
- Agent 私钥泄漏时，管理员可以吊销该 Agent 的所有会话并要求重新 enrollment。

## 权限模型

角色：

- Admin：全局管理。
- Member：只能管理自己拥有的服务器和资源。

PAT scope：

- `inventory:read`
- `inventory:delete`
- `server:read`
- `server:write`
- `server:delete`
- `server:exec`
- `service:read`
- `service:write`
- `service:delete`
- `alert:read`
- `alert:write`
- `alert:delete`
- `task:read`
- `task:write`
- `task:delete`
- `task:exec`
- `ddns:read`
- `ddns:write`
- `ddns:delete`
- `nat:read`
- `nat:write`
- `nat:delete`
- `notification:read`
- `notification:write`
- `notification:delete`
- `transfer:read`
- `transfer:write`
- `admin:*`

Server allowlist：

- PAT 可选绑定服务器 ID 列表。
- 空列表表示不按服务器收窄，但仍受用户归属和 scope 限制。

验收标准：

- 每个 handler 都明确声明所需 scope。
- 覆盖全部服务器的任务、服务监控、告警规则必须检查 PAT allowlist，避免越权 fanout。

## CSRF 和 Web 安全

- 所有 Cookie session 的写请求必须带 CSRF token。
- PAT Bearer 请求不走 CSRF。
- CORS 默认关闭跨域凭据。
- WebSocket 校验 Origin 和 session。
- 设置安全 Header：CSP、X-Frame-Options、Referrer-Policy、Content-Type-Options。

失败场景：

- CSRF 缺失返回 403。
- Origin 不可信关闭 WebSocket。

验收标准：

- 自动化测试覆盖 GET 不改变状态、POST/PATCH/DELETE 需要 CSRF。

## SSRF 防护

适用范围：

- 通知 Webhook。
- DDNS Webhook。
- HTTP GET 服务监控。

规则：

- URL scheme 只允许 http 和 https。
- 拒绝 localhost、loopback、private、link-local、multicast、reserved、unspecified。
- DNS 解析后校验每个 IP。
- 不自动跟随重定向；如后续支持重定向，每跳都重新校验。

验收标准：

- `127.0.0.1`、`localhost`、`10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`、`169.254.0.0/16` 都被拒绝。

## 远程执行安全

Shell/Exec：

- Agent 可通过 `disable_command_execute` 全局拒绝。
- Exec 环境变量使用 allowlist，不继承所有 Agent 环境。
- 支持超时和最大输出。
- 超时 kill process group。

Terminal：

- 会话必须绑定当前用户、服务器和权限。
- 断开时关闭 PTY。

文件：

- 路径必须绝对路径。
- 拒绝删除文件系统根目录。
- 写入支持 `if_match_sha256` 乐观锁。
- 大文件传输使用一次性 token。

验收标准：

- 所有 exec、terminal、file write/delete、transfer 都写 audit log。
- audit log 不保存命令 stdin、env、文件内容明文，只保存 hash 和元数据。

## MCP 安全

- 默认关闭。
- 只接受 PAT。
- 不接受 Cookie session。
- 单 token 限流。
- request body 上限 8 MiB。
- 临时 URL 默认 TTL 300 秒，最大 600 秒，单次使用。
- 禁用 MCP 或吊销 token 时，未使用 URL 和进行中传输立即失效。

验收标准：

- 工具调用 scope 不足返回 tool error。
- 无效 tool 也计入限流。

## NAT 安全

- NAT 域名不能与 Dashboard 保留 host 冲突。
- Member 只能绑定自己有权访问的 Agent。
- 禁用 NAT 配置后立即拒绝新请求。
- 隧道字节流不写日志，只记录元数据。

验收标准：

- 反向代理部署时，通过 `reserved_hosts` 防止成员抢占 Dashboard 域名。
