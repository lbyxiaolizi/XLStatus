# XLStatus Agent 节点守护进程指南

`xlstatus-agent` 是运行在被监控服务器上的轻量级、高安全性的 Rust 守护进程。它通过单条出站 HTTP 注册 + gRPC 双向长连接与 `xlstatus-server` 通信，无需在被控节点开放任何入站公网端口即可完成实时遥测上报、分布式服务拨测、受控远程命令执行、安全文件传输、交互式 PTY 终端与 NAT 反向隧道内网穿透。

---

## 1. 快速接入流程

### 1.1 使用 Linux 一键安装脚本（推荐）

在控制台 **设置 -> 节点与接入 -> Agent 安装向导**（或 `/tokens` 页面）生成短期注册令牌（`xle_...`），并在目标 Linux 主机执行：

```bash
sudo SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=https://grpc.dashboard.example.com:50051 \
  VERSION=v0.2 \
  ENROLLMENT_TOKEN=xle_your_enrollment_token \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/install-agent.sh | bash'
```

安装脚本会自动完成以下安全步骤：
1. 下载对应架构二进制并校验 ELF 魔数（`7f454c46`）。
2. 在同目录暂存二进制、配置与 `systemd` 单元文件，避免半升级损坏。
3. 通过 `--token-stdin`（避免在进程命令行参数中泄露 `xle_` 令牌）执行节点注册并生成本地 Ed25519 密钥对。
4. 启动 `xlstatus-agent.service` 并校验运行状态；若失败则自动回滚旧版本二进制与服务状态。

### 1.2 手动使用二进制注册与运行

```bash
# 1. 注册节点（推荐通过管道 + --token-stdin 传入注册令牌）
printf '%s\n' "xle_your_enrollment_token" | \
  ./xlstatus-agent enroll \
    --server http://127.0.0.1:8080 \
    --grpc-server http://127.0.0.1:50051 \
    --token-stdin \
    --name "hk-edge-01" \
    --config /etc/xlstatus/agent.yaml

# 2. 启动守护进程
./xlstatus-agent run --config /etc/xlstatus/agent.yaml
```

---

## 2. 配置文件参考（`agent.yaml`）

`xlstatus-agent` 在注册或更新配置时，会拒绝符号链接并以原子重命名方式将 `agent.yaml` 的文件权限强制锁定为 **`0600` (`rw-------`)**，确保主机上的非特权本地用户无法读取节点的 Ed25519 私钥与短期 JWT。

```yaml
# 服务端 HTTP API 地址（用于初次注册与挑战-响应刷新 JWT）
server_url: "https://status.example.com"

# 服务端 gRPC 地址（生产跨公网部署建议使用 https:// 并配置 TLS/mTLS）
grpc_server_url: "https://grpc.status.example.com:50051"

# 节点唯一 UUID（由 enroll 自动写入）
agent_id: "01965b20-8800-7000-8000-000000000001"

# 节点 Ed25519 私钥与当前缓存的短期 JWT（0600 权限保护）
private_key: "<base64-ed25519-private-key>"
jwt: "<short-lived-hs256-jwt>"

# 遥测与心跳周期（秒）
heartbeat_interval_secs: 15
report_interval_secs: 3

# 允许远程文件管理访问的根目录白名单（默认为空或指定受控目录）
file_roots:
  - "/var/log"
  - "/opt/app"

# 本地单调安全锁（一旦在本地设为 true，远端控制面 ApplyConfig 无法将其重新打开）
disable_command_execute: false
disable_force_update: false
disable_auto_update: false
disable_nat: false
disable_send_query: false

# 边界安全逃逸开关（默认均为 false，最严格模式）
allow_private_http_probes: false
allow_private_nat_targets: false

# 可选 gRPC TLS / mTLS 证书配置
tls_ca_cert_path: "/etc/xlstatus/ca.pem"
tls_domain_name: "grpc.status.example.com"
tls_client_cert_path: "/etc/xlstatus/agent.pem"
tls_client_key_path: "/etc/xlstatus/agent.key"
```

---

## 3. Agent 端纵深防御安全机制

由于 `xlstatus-agent` 具备主机级监控与运维能力，项目在 Agent 端内置了 7 层主机侧防御边界：

1. **本地单调安全锁（Monotonic Local Security Locks）**：
   - 若主机管理员在本地 `agent.yaml` 中将 `disable_command_execute`、`disable_force_update`、`disable_auto_update`、`disable_nat` 或 `disable_send_query` 设为 `true`，即便服务端控制面下发 `ApplyConfig` 尝试将其改为 `false`，Agent 也会通过逻辑或（`local || remote`）保持禁用状态，防止控制面被攻陷后突破主机本地最小权限策略。
2. **子进程环境变量净化与动态链接器防护（`executor/shell.rs` & `terminal.rs`）**：
   - 派生 Shell 命令或交互式 PTY 终端前，自动从子进程继承环境中剥离 `XLSTATUS_ENROLL_TOKEN`、`XLSTATUS_AGENT_JWT`、`XLSTATUS_PRIVATE_KEY`、`XLSTATUS_TOKEN` 等敏感凭证。
   - 对任务传入的 `env` 字典执行严格校验（最多 64 项、键长 `<= 256`、值长 `<= 4096`），并显式拦截 `LD_PRELOAD`、`LD_LIBRARY_PATH`、`LD_AUDIT`、`DYLD_INSERT_LIBRARIES`、`DYLD_LIBRARY_PATH`、`BASH_ENV`、`ENV`、`PROMPT_COMMAND`、`SHELLOPTS` 等高危注入变量。
3. **独立进程组与超时/断线孤儿进程清理**：
   - 每个远程 Shell 任务与 PTY 终端均在独立 Unix 进程组中启动。当任务超时、gRPC 会话断开或终端关闭时，Agent 会向整个进程组（`-pid`）发送终止信号并在后台线程回收子进程，防止后代进程残留。
   - 单 Agent 最大并发 PTY 终端会话数严格限制为 `MAX_TERMINAL_SESSIONS = 16`，终端尺寸钳制在 `1..=500`。
4. **防符号链接逃逸与防特殊文件挂起的远端文件操作（`executor/files.rs`）**：
   - 所有文件读写与删除操作均验证目标路径位于 `file_roots` 白名单内，逐级解析父目录防符号链接替换逃逸。
   - 在 Unix 上打开文件时强制使用 `O_NOFOLLOW | O_NONBLOCK` 标志并通过 `fstat` 校验目标必须为普通文件（`S_IFREG`），彻底阻断读取命名管道（FIFO，如 `/tmp/fifo`）或设备节点（如 `/dev/zero`）导致的线程永久阻塞或内存耗尽。
5. **NAT 反向隧道本地目标 DNS 绑定与云元数据阻断（`nat/mod.rs`）**：
   - Agent 收到 `NatOpen` 请求后，先解析目标主机名并对返回的全部 `SocketAddr` 执行安全策略校验：默认仅允许连接本机回环地址（`127.0.0.1` / `::1`）；即便显式开启 `allow_private_nat_targets: true`，也**永久禁止**连接云厂商元数据地址（`169.254.169.254`、`100.100.100.200`、`fd00:ec2::254`）、组播、未指定地址与公网 IP。
   - 校验通过后直接向已绑定的 `SocketAddr` 发起 TCP 连接，彻底消除 DNS Rebinding 时间窗。
6. **HTTP/ICMP/TCP 分布式探针加固（`executor/{http,icmp,tcp}.rs`）**：
   - Agent HTTP 探针默认禁止访问私网/环回/云元数据地址，禁用自动 HTTP 重定向（`redirect::Policy::none()`）与系统环境代理（`.no_proxy()`），绑定已校验的 `SocketAddr`，并将响应体读取上限限制为 `64 KiB`。
   - ICMP `ping` 探针严格拦截以 `-` 开头的目标主机名（防止命令行标志注入）及控制字符。
7. **安全 OTA 强制升级（`ForceUpdate`）**：
   - 仅允许从 `https://`（或本地测试回环地址）下载升级包，拒绝包含用户凭证（`user:pass@`）或 `..` 路径穿越段的 URL，限制下载体积上限，并在同目录临时文件完成 64 位 SHA-256 校验后才执行原子替换。
