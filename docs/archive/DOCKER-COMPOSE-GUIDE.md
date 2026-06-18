# Docker Compose 使用指南

## 问题诊断

### 已知问题和修复

1. **Cargo.lock 版本不兼容**
   - 问题：本地使用 Rust 1.98 (nightly)，生成的 Cargo.lock version 4
   - 修复：Dockerfile 已更新为使用 `rust:latest`

2. **环境变量名称不一致**
   - 问题：docker-compose.yml 使用了 `BIND_ADDRESS` 和 `GRPC_ADDRESS`
   - 修复：已更新为 `HTTP_BIND` 和 `GRPC_BIND`

3. **缺少 SESSION_SECRET**
   - 问题：服务器启动需要 SESSION_SECRET
   - 修复：已添加默认值

## 快速开始

### 选项 1: 仅服务器 (推荐测试)

```bash
# 使用简化配置
docker-compose -f docker-compose.simple.yml up -d

# 查看日志
docker-compose -f docker-compose.simple.yml logs -f

# 测试
curl http://localhost:8080/api/info

# 停止
docker-compose -f docker-compose.simple.yml down
```

### 选项 2: 完整栈 (服务器 + Web + Agent)

```bash
# 构建所有服务
docker-compose build

# 启动
docker-compose up -d

# 查看日志
docker-compose logs -f server

# 访问
# - Server API: http://localhost:8080
# - Web UI: http://localhost:3000

# 停止
docker-compose down
```

### 选项 3: 单独构建和运行

```bash
# 只构建服务器
docker-compose build server

# 只启动服务器
docker-compose up -d server

# 查看状态
docker-compose ps

# 查看日志
docker-compose logs server
```

## 构建时间预估

首次构建（下载依赖）：
- Server: 5-10 分钟
- Agent: 3-5 分钟
- Web: 2-3 分钟

后续构建（有缓存）：
- Server: 1-2 分钟
- Agent: 1 分钟
- Web: 30 秒

## 验证步骤

### 1. 检查容器状态

```bash
docker-compose ps
# 应该显示：
# NAME                  STATUS
# xlstatus-server       Up (healthy)
```

### 2. 检查日志

```bash
docker-compose logs server | tail -20
# 应该看到：
# [INFO] Server started on 0.0.0.0:8080
# [INFO] gRPC server listening on 0.0.0.0:50051
```

### 3. 测试 API

```bash
# 健康检查
curl http://localhost:8080/api/info

# 登录测试
curl -X POST http://localhost:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"admin123"}'
```

### 4. 检查端口

```bash
lsof -i :8080  # HTTP
lsof -i :50051 # gRPC
lsof -i :3000  # Web (如果启动了)
```

## 故障排查

### 构建失败

```bash
# 清理缓存重新构建
docker-compose build --no-cache server

# 查看详细构建日志
docker-compose build server 2>&1 | tee build.log
```

### 容器无法启动

```bash
# 查看容器退出原因
docker-compose logs server

# 进入容器调试
docker-compose run --rm server /bin/bash
```

### 端口被占用

```bash
# 检查端口占用
lsof -i :8080
lsof -i :50051

# 修改 docker-compose.yml 端口映射
ports:
  - "8081:8080"  # 改为 8081
  - "50052:50051"
```

### 数据库问题

```bash
# 删除数据卷重新开始
docker-compose down -v

# 查看数据卷
docker volume ls | grep xlstatus

# 删除特定数据卷
docker volume rm xlstatus_xlstatus-data
```

## 环境变量

### 服务器必需变量

```bash
DATABASE_URL=sqlite:///data/xlstatus.db  # 数据库路径
HTTP_BIND=0.0.0.0:8080                   # HTTP 监听地址
GRPC_BIND=0.0.0.0:50051                  # gRPC 监听地址
SESSION_SECRET=your-secret-here          # 会话密钥（生产环境必须修改）
```

### 可选变量

```bash
RUST_LOG=info                            # 日志级别: error/warn/info/debug/trace
MAX_CONNECTIONS=100                       # 最大数据库连接数
```

## 生产部署建议

1. **修改默认密码和密钥**
   ```bash
   SESSION_SECRET=$(openssl rand -base64 32)
   ```

2. **使用 PostgreSQL**
   ```yaml
   # docker-compose.pg.yml
   DATABASE_URL=postgres://user:pass@postgres:5432/xlstatus
   ```

3. **启用 HTTPS**
   - 使用 Nginx 反向代理
   - 配置 SSL 证书

4. **配置持久化存储**
   ```yaml
   volumes:
     - /opt/xlstatus/data:/data
   ```

5. **资源限制**
   ```yaml
   deploy:
     resources:
       limits:
         cpus: '1'
         memory: 512M
   ```

## 网络配置

默认情况下，所有服务在同一个 Docker 网络中：

- Server: http://server:8080, grpc://server:50051
- Web: http://web:3000
- Agent: 连接到 http://server:8080

## 数据持久化

数据存储在命名卷中：

```bash
# 备份数据
docker run --rm -v xlstatus_xlstatus-data:/data -v $(pwd):/backup \
  alpine tar czf /backup/xlstatus-backup.tar.gz /data

# 恢复数据
docker run --rm -v xlstatus_xlstatus-data:/data -v $(pwd):/backup \
  alpine tar xzf /backup/xlstatus-backup.tar.gz -C /
```

## 健康检查

服务器健康检查配置：

```yaml
healthcheck:
  test: ["CMD-SHELL", "curl -f http://localhost:8080/api/info || exit 1"]
  interval: 30s      # 每 30 秒检查一次
  timeout: 10s       # 超时时间 10 秒
  retries: 3         # 失败 3 次后标记为 unhealthy
  start_period: 40s  # 启动后 40 秒才开始检查
```

## 日志管理

```bash
# 实时查看日志
docker-compose logs -f

# 查看最近 100 行
docker-compose logs --tail=100 server

# 导出日志
docker-compose logs server > server.log
```

## 更新和维护

```bash
# 更新镜像
docker-compose pull

# 重新构建
docker-compose build --pull

# 滚动更新
docker-compose up -d --force-recreate --no-deps server
```

## 性能调优

### 构建优化

```bash
# 使用 BuildKit
DOCKER_BUILDKIT=1 docker-compose build

# 并行构建
docker-compose build --parallel
```

### 运行优化

```yaml
# 限制日志大小
logging:
  driver: "json-file"
  options:
    max-size: "10m"
    max-file: "3"
```

## 当前状态

- ✅ Dockerfile.server 已修复（使用 rust:latest）
- ✅ Dockerfile.agent 已修复（使用 rust:latest）
- ✅ docker-compose.yml 已修复（环境变量）
- ✅ docker-compose.simple.yml 已创建（仅服务器）
- ⏳ 首次构建正在进行中

## 下一步

1. 等待构建完成（5-10 分钟）
2. 运行 `docker-compose -f docker-compose.simple.yml up -d`
3. 测试 API: `curl http://localhost:8080/api/info`
4. 如果成功，尝试完整栈: `docker-compose up -d`

---

**文档更新**: 2026-06-17
**测试状态**: 构建中
