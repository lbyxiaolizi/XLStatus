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
  "agent_id": "...",
  "name": "web-01",
  "public_key": "...",
  "private_key": "..."
}
```

这个文件包含私钥，建议权限为 `0600`。

## systemd 安装

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

这个链接由 Server 负责注入参数，真正执行的安装脚本来自 GitHub Release：

```text
https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.2/install-agent.sh
```

生成链接时会包含 enrollment token；这个 token 建议设置短有效期，并只发给受信任主机。

## 地址说明

- `--server` 是 Dashboard HTTP API 地址，例如 `http://dashboard.example.com:8080`。
- `--grpc-server` 是 Agent 长连接地址，例如 `http://dashboard.example.com:50051`。

两者不能混用。HTTP API 可通过 `/healthz` 检查，gRPC 端口需要确认网络可达。

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
