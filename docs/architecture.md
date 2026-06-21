# 架构说明

本文描述当前代码和发布形态，历史设计推演已归档到 `docs/archive/`。

## 组件

```text
Browser
  |
  | HTTP / WebSocket
  v
web/ Next.js UI  --->  xlstatus-server HTTP API
                         |
                         | gRPC bidirectional stream
                         v
                     xlstatus-agent
```

- `xlstatus-server`：控制面服务，负责 HTTP API、认证授权、WebSocket、Agent gRPC、数据库访问、后台服务探测、告警、DDNS、NAT、MCP 和维护能力。
- `xlstatus-agent`：被监控主机上的 Agent，负责注册、主机状态采集、gRPC 会话、任务执行、文件操作、终端、NAT IO 和配置应用。
- `web/`：Next.js Dashboard 与公开状态页。生产构建时会把 `NEXT_PUBLIC_API_URL` 写入浏览器 bundle。
- 数据库：SQLite 或 PostgreSQL。Server 内置迁移，启动时创建/迁移应用表。
- Release 资产：多平台 server/agent 二进制和 `install-server.sh`、`install-agent.sh`。

## 请求路径

### Dashboard

1. Browser 打开 Web UI。
2. Web UI 通过 `NEXT_PUBLIC_API_URL` 请求 Server HTTP API。
3. 登录后使用 Cookie session 和 CSRF header。
4. 实时服务器状态通过 `/ws/servers` 推送。

### Agent

1. Dashboard 创建 enrollment token。
2. Agent 调用 HTTP enrollment 接口换取身份和密钥。
3. Agent 建立 gRPC session。
4. Server 下发任务、终端 IO、配置更新、NAT IO 或强制更新消息。
5. Agent 上报 HostState、HostInfo、GeoIP、任务结果和 IO frame。

### Public Status

公开页面访问 `/api/v1/public/status`，不需要 session。它只返回公开可见的服务器、服务和主题/站点信息。

## 数据边界

- 用户认证、RBAC、PAT、CSRF 和 session 在 Server 侧处理。
- Agent 配置包含私钥，安装后应保持 `0600` 权限。
- Enrollment token 会出现在一键安装链接中，应设置短有效期，并只发给受信任主机。
- Web UI 的 CORS 来源必须精确配置，不能使用 `*`。

## 发布拓扑

### Docker Compose

`docker-compose.yml` 启动 Server 和 Web。SQLite 数据默认放在 `./data`。远端部署时需要设置：

```env
XLSTATUS_PUBLIC_API_URL=https://api.example.com
XLSTATUS_CORS_ALLOWED_ORIGINS=https://status.example.com
```

修改这些值后必须重新构建 Web 镜像。

### systemd

`deploy/install.sh` 安装 Server，`deploy/install-agent.sh` 安装 Agent。默认 Release 版本为 `v0.1.0-alpha.3`。

### GitHub Release

推送 `v*` tag 后，GitHub Actions 构建并发布：

```text
xlstatus-server-linux-x86_64
xlstatus-agent-linux-x86_64
xlstatus-server-linux-arm64
xlstatus-agent-linux-arm64
xlstatus-server-linux-i386
xlstatus-agent-linux-i386
xlstatus-server-windows-x86_64.exe
xlstatus-agent-windows-x86_64.exe
xlstatus-server-windows-arm64.exe
xlstatus-agent-windows-arm64.exe
xlstatus-server-windows-i386.exe
xlstatus-agent-windows-i386.exe
xlstatus-server-darwin-x86_64
xlstatus-agent-darwin-x86_64
xlstatus-server-darwin-arm64
xlstatus-agent-darwin-arm64
xlstatus-server-freebsd-x86_64
xlstatus-agent-freebsd-x86_64
xlstatus-server-freebsd-i386
xlstatus-agent-freebsd-i386
install-server.sh
install-agent.sh
```

带参数 Agent bootstrap 会从该 Release 下载 `install-agent.sh`。Linux 安装脚本会按当前机器架构选择 `linux-x86_64`、`linux-arm64` 或 `linux-i386` 资产。
