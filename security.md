# XLStatus 安全修复记录

本文件记录已经确认并完成回归验证的安全修复。未列为“已修复”的项目不应据此视为已经审计通过。

## 当前验证

- `cargo fmt --all -- --check`、`cargo check --workspace --locked` 和 workspace 严格 Clippy 通过。
- `cargo test --workspace --locked`：704 项通过，5 项因外网或交互式 PTY 要求忽略，0 失败。
- Web `lint`、`typecheck`、生产构建和 M7 UI 静态验收通过。
- PostgreSQL 导入协议 8 项测试、全部 Shell 语法、进程隔离清理、安装升级回滚和 M9 Server/Agent 安装链路通过。
- 三套 Compose 均通过 `docker compose config`，展开后的宿主 gRPC `host_ip` 均为 `127.0.0.1`。

### 1. 远程 Shell 超时或取消后代进程残留

状态：已修复同一进程组内的默认路径；daemonize 逃逸见残余边界。

深入复核确认：

- 已授权的远程 Shell 任务通过 `sh -c` 执行，命令可以派生后台子进程。
- 旧超时路径只终止直接 shell；后台进程能够在任务已记录为超时后继续运行、写文件或消耗资源。
- 输出达到上限时旧读取器会提前关闭 pipe，可能以 `SIGPIPE` 改变命令退出行为，而不是只截断返回内容。

本轮修复：

- Unix 平台为每个 Shell 任务建立独立进程组，超时时向整个进程组发送终止信号，并 `wait` 回收直接子进程。
- 进程组 guard 保持 armed 直到正常完成；Agent shutdown、任务 abort 或上层 future 被丢弃时，Drop 仍会终止整个组。
- 输出读取达到上限后继续排空 pipe，但不再增长内存缓冲区。
- 回归测试分别确认超时和 future abort 后后台后代进程不能创建 marker，并确认截断读取仍会完整 drain 写端。

残余边界：

- Unix 后代若主动调用 `setsid()`/daemonize 脱离初始进程组，普通 PGID 信号无法继续约束它；需要在容器、cgroup/systemd scope 等 OS 级 containment 中运行 Agent 才能覆盖该路径。
- Windows 当前仍只终止直接子进程；完整后代清理需要 Windows Job Object。
- Shell 任务本身是管理员授权的高权限能力，仍应限制可操作账号、Agent 范围和命令来源。

### 2. 部署默认公开明文 Agent gRPC

状态：已修复默认暴露。

深入复核确认：

- 三套 Compose 都把宿主 `50051` 发布到所有网络接口，Release/systemd 安装器也默认监听所有接口；同时 Server/Agent 允许 `http://` gRPC。
- Agent JWT 通过 gRPC authorization metadata 发送；未启用 TLS 时，安全性完全依赖底层网络可信。

本轮修复：

- 三套 Compose 的宿主 gRPC 发布地址默认改为 `127.0.0.1`，容器网络内通信保持不变。
- Release/systemd 安装器的新安装默认也改为 `127.0.0.1:50051`；既有升级会保留原配置。
- 远端接入必须显式设置 `XLSTATUS_GRPC_PUBLISH_ADDRESS=0.0.0.0`，文档要求同时使用 TLS/mTLS 或可信 VPN。
- Server 回归测试锁定三套 Compose 的 loopback 默认值。

残余边界：

- 显式选择公网发布时，程序不会自动签发证书；部署者仍必须配置 gRPC TLS/mTLS、VPN 和防火墙。

### 3. 已取消请求遗留任务响应注册项

状态：已修复。

深入复核确认：

- 任务、文件、探测和 MCP 下发会先在全局 `TaskResponseRegistry` 注册 oneshot waiter，再等待 Agent 回复。
- 旧实现只在显式发送失败、回复或 timeout 分支移除；HTTP 客户端断连或上层任务 abort 会直接丢弃 future，使注册项无限期保留。
- 重复取消请求可造成进程内 HashMap 持续增长，形成可用性风险。

本轮修复：

- 注册接口返回 cancellation-safe receiver，Drop 使用短临界区同步移除自身注册项，不依赖另一个异步清理任务成功调度。
- 每次注册带独立身份 token，旧 receiver Drop 不会误删同 task ID 的新注册。
- 回归测试确认同名重注册隔离，并确认真实双 Agent 派发被 abort 后 pending 数从 2 归零。

### 4. 验收脚本按模糊进程名强杀

状态：已修复。

深入复核确认：

- 多个 `test-run` 脚本曾用 `pkill -9 -f xlstatus-server`、`xlstatus-agent` 或按端口枚举 PID 清场，可能终止同一开发机上不属于本次验收的进程。

本轮修复：

- 长期测试服务由独立 session supervisor 启动，并把 PID、随机 token 和标签写入当前脚本专属注册表。
- 清理前同时校验 PID 是进程组长、命令行为指定 supervisor 且包含匹配 token，随后只向该进程组发送 TERM/KILL。
- 隔离测试使用两个同名 `sleep` 进程，确认注册组及其后代被清理，而未注册控制进程保持存活；CI 同步执行该测试。

### 5. 系统 HTTP 代理绕过出站地址固定

状态：已修复。

- `reqwest` 默认继承 `HTTP_PROXY` / `HTTPS_PROXY` 等环境变量。请求经过代理时，代理会独立解析目标主机，原有的私网地址检查和 `resolve_to_addrs` DNS 固定不能约束实际连接地址。
- Server 的统一安全出站 client builder 和 Agent HTTP 探测 client 现在显式禁用系统代理，确保连接只使用本机已经校验的地址。
- 需要代理出站的生产环境不能依赖这些环境变量绕过校验；应在受控网关实施等价的目标限制并单独设计可信代理配置。

### 6. Agent IO 断线后终端、NAT 和远程任务残留

状态：已修复当前 session 生命周期。

- Server 在当前 IO 流断开、被替换或 Agent 撤销时，会原子移除该 Agent 的全部 stream subscriber，并通知终端/NAT consumer 结束。订阅投递具有关闭通知和超时，满队列不会再永久阻塞 gRPC 收包。
- Agent 会跟踪每个 session 的心跳、指标、IP 上报和远程任务 handle；session 结束时请求取消并等待任务退出。Shell future 被取消后会触发其进程组清理 guard。
- Agent 同时 drain PTY/NAT session map，关闭 PTY、终止 NAT reader 并释放 writer channel，打破 JoinHandle 持有 map 的引用环。
- 回归测试覆盖 IO 流替换、撤销、并发订阅、满队列断线唤醒，以及 Agent 任务取消和 NAT 引用环回收。

### 7. 安装失败留下半升级状态

状态：已修复本机文件与 systemd 状态。

- Server/Agent 安装器先在同目录暂存 binary、配置和 unit，再原子替换正式文件；备份会保留符号链接对象和权限。
- 后续 enrollment、unit、restart 或健康检查失败时，会恢复旧文件以及原 active/inactive、enabled/disabled 状态；全新安装失败会删除未提交文件。
- 生成的两个 systemd unit 已与仓库 unit 的 `NoNewPrivileges`、`PrivateTmp`、`ProtectSystem`、`ProtectHome` 和 `ReadWritePaths` 加固保持一致。
- Agent enrollment 已在控制面消费的 token 或创建的记录属于远端副作用，无法由本机事务撤销，失败后仍需在 Dashboard 中检查并清理。

### 8. PostgreSQL 导入级联影响未知表

状态：已修复。

- 导入脚本只处理明确的 XLStatus table allowlist，未知 `public` 表不会被直接锁定、检查、复制或列为 TRUNCATE 目标。
- `TRUNCATE` 不再使用 `CASCADE`。如果未知表通过外键引用 XLStatus 表，PostgreSQL 会拒绝操作并回滚整个导入事务，而不会级联清空未知数据。
- TRUNCATE、流式 COPY、逐表计数校验和 COMMIT 仍位于同一 PostgreSQL session/transaction；失败会恢复目标库原状态。

## 长期残余风险

- 生产环境仍应为远端 Agent gRPC 使用 TLS/mTLS。明文 `http://` gRPC 只能用于 loopback、同一容器网络或明确可信的私网。
- 安装链接包含短期 enrollment token，应仅通过受信渠道分发，并保持较短有效期。
- Windows 远程 Shell 后代进程的完整生命周期仍需要 Job Object；当前仅保证直接子进程终止。
- 对不受信任的 Shell 命令，Unix Agent 也应运行在容器或独立 cgroup/systemd scope 中，防止 `setsid()` 后代逃逸普通进程组。
