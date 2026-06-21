# Agent 接入

Agent 负责采集主机状态、维持 gRPC 会话、执行任务和承载部分运维能力。

## 构建

```bash
cargo build --release --bin xlstatus-agent
```

## 注册流程

1. 在 Dashboard 创建 enrollment token。
2. 在被监控主机上执行 `xlstatus-agent enroll`。
3. 使用生成的 JSON 配置运行 Agent。

示例：

```bash
xlstatus-agent enroll \
  --server http://dashboard.example.com:8080 \
  --grpc-server http://dashboard.example.com:50051 \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json
```

运行：

```bash
xlstatus-agent run --config /etc/xlstatus-agent/agent.json
```

## 配置文件

注册会生成 JSON：

```json
{
  "server": "http://dashboard.example.com:8080",
  "grpc_server": "http://dashboard.example.com:50051",
  "grpc_tls_ca_path": null,
  "grpc_tls_domain_name": null,
  "grpc_tls_client_cert_path": null,
  "grpc_tls_client_key_path": null,
  "agent_id": "...",
  "name": "web-01",
  "public_key": "...",
  "private_key": "..."
}
```

这个文件包含私钥，建议权限为 `0600`。

## gRPC TLS/mTLS

生产环境如果 gRPC 不在可信内网或 WireGuard/VPC 内，建议把 `--grpc-server` 配置为 `https://...`，并在 Server 设置 `GRPC_TLS_CERT_PATH` 与 `GRPC_TLS_KEY_PATH`。使用私有 CA 时，在 Agent 注册时传入 CA：

```bash
xlstatus-agent enroll \
  --server https://dashboard.example.com \
  --grpc-server https://grpc.dashboard.example.com:50051 \
  --grpc-tls-ca-path /etc/xlstatus-agent/tls/grpc-ca.crt \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json
```

如果 Server 配置了 `GRPC_TLS_CLIENT_CA_PATH` 启用 mTLS，Agent 还必须配置客户端证书和私钥：

```bash
xlstatus-agent enroll \
  --server https://dashboard.example.com \
  --grpc-server https://grpc.dashboard.example.com:50051 \
  --grpc-tls-ca-path /etc/xlstatus-agent/tls/grpc-ca.crt \
  --grpc-tls-client-cert-path /etc/xlstatus-agent/tls/agent.crt \
  --grpc-tls-client-key-path /etc/xlstatus-agent/tls/agent.key \
  --token xle_... \
  --name web-01 \
  --config /etc/xlstatus-agent/agent.json
```

## systemd 安装

使用当前 Release 安装脚本：

```bash
sudo SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=https://grpc.dashboard.example.com:50051 \
  GRPC_TLS_CA_PATH=/etc/xlstatus-agent/tls/grpc-ca.crt \
  GRPC_TLS_CLIENT_CERT_PATH=/etc/xlstatus-agent/tls/agent.crt \
  GRPC_TLS_CLIENT_KEY_PATH=/etc/xlstatus-agent/tls/agent.key \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash -c 'curl -fsSL https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/install-agent.sh | bash'
```

使用本地构建产物安装：

```bash
sudo BINARY_PATH=target/release/xlstatus-agent \
  SERVER_URL=http://dashboard.example.com:8080 \
  GRPC_SERVER=http://dashboard.example.com:50051 \
  ENROLLMENT_TOKEN=xle_... \
  AGENT_NAME="$(hostname)" \
  bash deploy/install-agent.sh
```

检查：

```bash
sudo systemctl status xlstatus-agent
sudo journalctl -u xlstatus-agent -n 100 --no-pager
```

## 后台一键安装

Dashboard 的“设置”页可以生成 Agent enrollment token，并给出带参数的一键安装命令。命令形态如下：

```bash
curl -fsSL 'http://dashboard.example.com:8080/api/v1/agents/install.sh?...' | sudo bash
```

带参数 bootstrap 是公开端点，`SERVER_URL` 和 `GRPC_SERVER` 只允许指向与该链接请求 `Host` 相同的主机，端口可以不同。跨主机控制面部署请直接下载 GitHub Release 的 `install-agent.sh`，并用环境变量传入 `SERVER_URL` / `GRPC_SERVER`。

这个链接由 Server 负责注入参数，真正执行的安装脚本来自 GitHub Release：

```text
https://github.com/lbyxiaolizi/XLStatus/releases/download/<VERSION>/install-agent.sh
```

后台“设置 / Agent 安装”默认会从 GitHub Releases 获取最新非草稿版本，并把该版本写入带参数安装命令。

生成链接时会包含 enrollment token；后端限制有效期为 1 到 24 小时，建议使用默认 1 小时，并只发给受信任主机。

## 地址说明

- `--server` 是 Dashboard HTTP API 地址，例如 `http://dashboard.example.com:8080`。
- `--grpc-server` 是 Agent 长连接地址，例如 `http://dashboard.example.com:50051` 或 `https://grpc.dashboard.example.com:50051`。
- `--grpc-tls-ca-path`、`--grpc-tls-domain-name`、`--grpc-tls-client-cert-path`、`--grpc-tls-client-key-path` 只用于 gRPC TLS/mTLS。配置这些字段时 `--grpc-server` 必须使用 `https://`。

两者不能混用。HTTP API 可通过 `/healthz` 检查，gRPC 端口需要确认网络可达。

## 网络探测安全策略

Agent 收到 Dashboard 下发的 HTTP/TCP/ICMP 探测任务时，会在执行端重新解析目标并拒绝私网、loopback、链路本地、组播、未指定、文档网段、CGNAT 和云元数据常见链路本地地址。HTTP 探测会把校验后的解析结果固定到请求客户端；TCP 只连接校验后的地址；ICMP 会把校验后的 IP 字符串传给 `ping`，避免系统 `ping` 再次解析原始主机名。

如果某台 Agent 必须用于内网服务探测，需要在运行环境中显式设置：

```bash
XLSTATUS_AGENT_ALLOW_PRIVATE_PROBES=1
```

该开关会放开 Agent 侧私网探测目标，建议只在受信网络内对专用 Agent 启用。兼容旧部署时，`XLSTATUS_ALLOW_PRIVATE_OUTBOUND=1` 也会放开 Agent 探测目标。

远程配置里的 `disable_send_query` 会停止 Agent 主动出口 IP 查询，并拒绝 Dashboard 下发的 HTTP/TCP/ICMP 探测任务。

## 重新注册

如果配置丢失或私钥泄漏：

1. 在 Dashboard 撤销旧 Agent 或吊销凭据。
2. 创建新的 enrollment token。
3. 重新执行 `xlstatus-agent enroll`。
4. 重启 `xlstatus-agent` 服务。

## 常见问题

- token 过期：重新生成 enrollment token。
- `--grpc-server` 不可达：检查防火墙、反代或端口映射。
- Agent 在线后没有数据：查看 `journalctl -u xlstatus-agent -f`。
- systemd 启动失败：安装脚本会打印最近日志，也可手动运行 `xlstatus-agent run --config ...`。
