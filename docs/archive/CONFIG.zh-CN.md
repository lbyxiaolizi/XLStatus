# XLStatus 配置指南

**版本**: v1.0.0
**最后更新**: 2026-06-17

---

## 目录

1. [服务器配置](#服务器配置)
2. [Agent 配置](#agent-配置)
3. [环境变量](#环境变量)
4. [数据库设置](#数据库设置)
5. [Docker 配置](#docker-配置)
6. [生产环境部署](#生产环境部署)

---

## 服务器配置

### 配置文件

创建服务器的 `config.toml` 文件：

```toml
# XLStatus 服务器配置

[server]
# HTTP API 监听地址
http_bind = "0.0.0.0:8080"

# gRPC 服务监听地址
grpc_bind = "0.0.0.0:50051"

# 会话密钥（生产环境必须修改）
# 生成方式: openssl rand -base64 32
session_secret = "change-me-in-production-use-random-secret"

[database]
# 数据库连接 URL
# SQLite: sqlite:///path/to/xlstatus.db
# PostgreSQL: postgres://user:password@host:port/database
url = "sqlite:///data/xlstatus.db"

# 最大连接数
max_connections = 10

# 连接超时（秒）
connect_timeout = 30

[auth]
# 会话生命周期（秒）
session_lifetime = 86400  # 24 小时

# JWT 过期时间（秒）
jwt_lifetime = 3600  # 1 小时

# 密码哈希参数（Argon2）
password_memory_cost = 65536  # 64 MB
password_time_cost = 3
password_parallelism = 4

[agent]
# Agent 心跳超时（秒）
heartbeat_timeout = 60

# Agent 重连间隔（秒）
reconnect_interval = 30

[logging]
# 日志级别: error, warn, info, debug, trace
level = "info"

# 日志格式: json, pretty
format = "pretty"

[metrics]
# 指标保留期（天）
retention_days = 30

# 采样间隔（秒）
sample_interval = 60

[security]
# 启用 CORS
enable_cors = true

# 允许的来源
cors_origins = ["http://localhost:3000", "https://yourdomain.com"]

# CSRF 保护
enable_csrf = true

[features]
# 启用服务监控
enable_service_monitor = true

# 启用告警
enable_alerts = true

# 启用 NAT 穿透
enable_nat = false

# 启用 DDNS
enable_ddns = false

# 启用 MCP 协议
enable_mcp = false

[defaults]
# 默认管理员用户名
admin_username = "admin"

# 默认管理员密码（首次启动时创建）
admin_password = "admin123"
```

### 使用方法

```bash
# 使用配置文件启动
./xlstatus-server --config /path/to/config.toml

# 环境变量优先级更高
DATABASE_URL="sqlite:///data/xlstatus.db" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="your-secret" \
./xlstatus-server
```

---

## Agent 配置

### 配置文件

创建 Agent 的 `config.toml` 文件：

```toml
# XLStatus Agent 配置

[agent]
# Agent 名称（唯一标识符）
name = "agent-1"

# 服务器连接 URL
server_url = "http://localhost:8080"

# 服务器 gRPC 地址
server_grpc = "localhost:50051"

# 上报间隔（秒）
report_interval = 10

# 失败时重连间隔（秒）
reconnect_interval = 30

[auth]
# Agent ID（注册后获得）
agent_id = ""

# Agent 私钥路径（Ed25519）
private_key_path = "/etc/xlstatus/agent.key"

[collectors]
# 启用 CPU 监控
enable_cpu = true

# 启用内存监控
enable_memory = true

# 启用磁盘监控
enable_disk = true

# 启用网络监控
enable_network = true

# 启用负载监控
enable_load = true

# 启用温度监控
enable_temperature = true

# 启用 GPU 监控（如果可用）
enable_gpu = true

[logging]
# 日志级别: error, warn, info, debug, trace
level = "info"

# 日志格式: json, pretty
format = "pretty"
```

### 使用方法

```bash
# 注册 Agent（首次）
./xlstatus-agent enroll \
  --server http://localhost:8080 \
  --token <enrollment-token>

# 启动 Agent
./xlstatus-agent --config /path/to/config.toml

# 或使用环境变量
SERVER_URL="http://localhost:8080" \
AGENT_NAME="my-server" \
./xlstatus-agent
```

---

## 环境变量

### 服务器环境变量

| 变量 | 说明 | 默认值 | 必需 |
|------|------|--------|------|
| `DATABASE_URL` | 数据库连接字符串 | `sqlite:///data/xlstatus.db` | 是 |
| `HTTP_BIND` | HTTP 监听地址 | `0.0.0.0:8080` | 是 |
| `GRPC_BIND` | gRPC 监听地址 | `0.0.0.0:50051` | 是 |
| `SESSION_SECRET` | 会话加密密钥 | - | 是 |
| `RUST_LOG` | 日志级别 | `info` | 否 |
| `MAX_CONNECTIONS` | 数据库最大连接数 | `10` | 否 |

### Agent 环境变量

| 变量 | 说明 | 默认值 | 必需 |
|------|------|--------|------|
| `SERVER_URL` | 服务器 HTTP URL | `http://localhost:8080` | 是 |
| `AGENT_NAME` | Agent 标识符 | `agent-1` | 是 |
| `REPORT_INTERVAL` | 上报间隔（秒） | `10` | 否 |
| `RUST_LOG` | 日志级别 | `info` | 否 |

---

## 数据库设置

### SQLite（开发环境）

```bash
# 创建数据库目录
mkdir -p /data

# SQLite 会自动创建数据库文件
DATABASE_URL="sqlite:///data/xlstatus.db" ./xlstatus-server
```

### PostgreSQL（生产环境）

```bash
# 创建数据库
createdb xlstatus

# 运行迁移
export DATABASE_URL="postgres://user:password@localhost:5432/xlstatus"
sqlx migrate run --source crates/server/migrations

# 启动服务器
DATABASE_URL="postgres://user:password@localhost:5432/xlstatus" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="$(openssl rand -base64 32)" \
./xlstatus-server
```

---

## Docker 配置

### 简单服务器模式

创建 `docker-compose.simple.yml`:

```yaml
services:
  server:
    build:
      context: .
      dockerfile: Dockerfile.server
    container_name: xlstatus-server
    ports:
      - "8080:8080"
      - "50051:50051"
    volumes:
      - xlstatus-data:/data
    environment:
      - DATABASE_URL=sqlite:///data/xlstatus.db
      - RUST_LOG=info
      - HTTP_BIND=0.0.0.0:8080
      - GRPC_BIND=0.0.0.0:50051
      - SESSION_SECRET=change-me-in-production
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:8080/api/info || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

volumes:
  xlstatus-data:
    driver: local
```

### 完整栈（服务器 + Web + Agent）

完整配置请参见 `docker-compose.yml`。

---

## 生产环境部署

### 安全检查清单

- [ ] 修改默认管理员密码
- [ ] 生成安全的 `SESSION_SECRET`
- [ ] 使用 PostgreSQL 而不是 SQLite
- [ ] 启用 HTTPS（通过反向代理）
- [ ] 配置防火墙规则
- [ ] 设置定期备份
- [ ] 启用审计日志
- [ ] 检查 CORS 设置
- [ ] 禁用调试日志

### 生成安全密钥

```bash
# 生成会话密钥
openssl rand -base64 32

# 示例输出：
# 8X9Kp2mQ7vN4jR6sT1wY3zL5hG8bC0dE9fA2gH4iJ6k=
```

### Nginx 反向代理

```nginx
server {
    listen 80;
    server_name xlstatus.yourdomain.com;

    # 重定向到 HTTPS
    return 301 https://$server_name$request_uri;
}

server {
    listen 443 ssl http2;
    server_name xlstatus.yourdomain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # HTTP API
    location /api {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # WebSocket（实时更新）
    location /ws {
        proxy_pass http://localhost:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }

    # Web 前端
    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

### Systemd 服务

创建 `/etc/systemd/system/xlstatus-server.service`:

```ini
[Unit]
Description=XLStatus Server
After=network.target

[Service]
Type=simple
User=xlstatus
Group=xlstatus
WorkingDirectory=/opt/xlstatus
Environment="DATABASE_URL=postgres://xlstatus:password@localhost/xlstatus"
Environment="HTTP_BIND=0.0.0.0:8080"
Environment="GRPC_BIND=0.0.0.0:50051"
Environment="SESSION_SECRET=your-secret-here"
Environment="RUST_LOG=info"
ExecStart=/opt/xlstatus/xlstatus-server
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

启用和启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable xlstatus-server
sudo systemctl start xlstatus-server
sudo systemctl status xlstatus-server
```

---

## 配置示例

### 开发环境

```bash
# 快速启动开发环境
DATABASE_URL="sqlite://dev.db" \
HTTP_BIND="127.0.0.1:8080" \
GRPC_BIND="127.0.0.1:50051" \
SESSION_SECRET="dev-secret" \
RUST_LOG=debug \
./xlstatus-server
```

### 生产环境

```bash
# 使用 PostgreSQL 的生产环境
DATABASE_URL="postgres://xlstatus:secure-password@postgres.internal:5432/xlstatus" \
HTTP_BIND="0.0.0.0:8080" \
GRPC_BIND="0.0.0.0:50051" \
SESSION_SECRET="$(cat /etc/xlstatus/session.secret)" \
RUST_LOG=warn \
MAX_CONNECTIONS=50 \
./xlstatus-server
```

### Docker 环境

```bash
# 简单的 Docker 运行
docker run -d \
  --name xlstatus-server \
  -p 8080:8080 \
  -p 50051:50051 \
  -v xlstatus-data:/data \
  -e DATABASE_URL=sqlite:///data/xlstatus.db \
  -e HTTP_BIND=0.0.0.0:8080 \
  -e GRPC_BIND=0.0.0.0:50051 \
  -e SESSION_SECRET=your-secret \
  xlstatus:latest
```

---

## 故障排查

### 常见问题

1. **端口已被占用**
   ```bash
   # 检查什么在使用端口
   lsof -i :8080
   lsof -i :50051
   ```

2. **数据库连接失败**
   ```bash
   # 测试 PostgreSQL 连接
   psql $DATABASE_URL

   # 检查 SQLite 文件权限
   ls -la /data/xlstatus.db
   ```

3. **会话密钥错误**
   ```bash
   # 生成新密钥
   export SESSION_SECRET=$(openssl rand -base64 32)
   ```

4. **Agent 无法连接**
   ```bash
   # 检查服务器是否可达
   curl http://localhost:8080/api/info

   # 检查 gRPC 端口
   grpcurl -plaintext localhost:50051 list
   ```

---

## 参考文档

- [主 README](./README.md)
- [Docker Compose 指南](./DOCKER-COMPOSE-GUIDE.md)
- [项目状态](./FINAL-STATUS.md)
- [架构文档](./plan/02-architecture.md)
- [安全设计](./plan/07-security.md)

---

**最后更新**: 2026-06-17
**版本**: v1.0.0
