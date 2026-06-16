# 快速开始指南

5 分钟内启动并运行 XLStatus。

## 前置要求

- Linux x86_64 系统
- Docker 和 Docker Compose（推荐）或
- Rust 1.75+ 用于源码构建

## 方式 1：Docker Compose（推荐）

### 1. 克隆仓库

```bash
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus
```

### 2. 启动服务

**使用 SQLite（更简单，单文件数据库）：**

```bash
docker compose up -d
```

**使用 PostgreSQL（更适合生产环境）：**

```bash
docker compose -f docker-compose.pg.yml up -d
```

### 3. 访问控制面板

在浏览器中打开：

```
http://localhost:8080
```

**默认账号：**
- 用户名：`admin`
- 密码：`admin123`

⚠️ **重要：** 请立即修改默认密码！

### 4. 检查状态

```bash
# 查看日志
docker compose logs -f

# 检查容器状态
docker compose ps

# 停止服务
docker compose down
```

## 方式 2：安装脚本

### 服务器安装

```bash
curl -fsSL https://install.xlstatus.io | bash
```

这将：
- 将 XLStatus 服务器安装到 `/opt/xlstatus`
- 创建 systemd 服务
- 在 8080 端口启动服务器

### Agent 安装

在每台需要监控的服务器上：

```bash
curl -fsSL https://install.xlstatus.io/agent | bash
```

你需要：
- 服务器 URL（例如：`http://your-server:8080`）
- 注册令牌（从控制面板 → 设置 → Agents 获取）

## 方式 3：从源码构建

### 1. 安装依赖

```bash
# Ubuntu/Debian
sudo apt-get install build-essential libssl-dev pkg-config

# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 构建服务器

```bash
git clone https://github.com/yourusername/xlstatus.git
cd xlstatus

cargo build --release --bin xlstatus-server
```

### 3. 运行服务器

```bash
export DATABASE_URL=sqlite://./dev.db
export BIND_ADDRESS=0.0.0.0:8080
export GRPC_ADDRESS=0.0.0.0:50051

./target/release/xlstatus-server
```

### 4. 构建 Web 界面

```bash
cd web
npm install
npm run build
npm start
```

访问 http://localhost:3000

## 下一步

1. **修改默认密码**
   - 进入 设置 → 用户
   - 修改 admin 密码

2. **添加第一个 Agent**
   - 进入 设置 → Agents
   - 生成注册令牌
   - 在目标服务器上安装 Agent

3. **配置服务监控**
   - 进入 服务监控
   - 点击"添加服务"
   - 配置 HTTP/TCP/ICMP 检查

4. **设置告警**
   - 进入 告警规则
   - 创建告警规则
   - 配置通知渠道

5. **探索功能**
   - 查看服务器指标
   - 调度任务
   - 配置 NAT 端口转发
   - 设置 DDNS

## 故障排除

### 服务器无法启动

```bash
# 查看日志
docker compose logs server

# 或使用 systemd
journalctl -u xlstatus -f
```

### Agent 无法连接

1. 验证服务器 URL 正确
2. 检查注册令牌
3. 验证端口 50051 可访问
4. 查看 Agent 日志：`journalctl -u xlstatus-agent -f`

### 数据库问题

```bash
# 重置 SQLite 数据库
rm ./data/xlstatus.db
docker compose restart server

# 重置 PostgreSQL
docker compose -f docker-compose.pg.yml down -v
docker compose -f docker-compose.pg.yml up -d
```

## 获取帮助

- 📚 [完整文档](./README.zh-CN.md)
- 🐛 [问题反馈](https://github.com/yourusername/xlstatus/issues)
- 💬 [Discord 社区](https://discord.gg/xlstatus)

## 下一步阅读

- [配置指南](./configuration.md)
- [Agent 设置](./agent-setup.md)
- [API 文档](./api.md)
- [安全最佳实践](./security.md)
