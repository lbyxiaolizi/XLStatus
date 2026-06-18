# XLStatus Linux x86_64 验证报告

**测试日期**: 2026-06-17
**测试服务器**: wawo-hk-sim-pro2
**操作系统**: Debian GNU/Linux 12 (bookworm)
**架构**: x86_64
**内核**: 6.18.3-x64v2-xanmod1

## 📋 验证概览

本文档记录了 XLStatus 在真实 Linux x86_64 服务器上的部署和验证过程。

## 🔧 测试环境

### 服务器信息
```bash
$ uname -a
Linux standard-prize 6.18.3-x64v2-xanmod1 #0~20260102.g6e0b8d1 SMP PREEMPT_DYNAMIC x86_64 GNU/Linux

$ cat /etc/os-release
PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
VERSION="12 (bookworm)"
VERSION_CODENAME=bookworm

$ docker --version
Docker version 29.2.1, build a5c7197
```

### 磁盘空间
```bash
$ df -h /opt
Filesystem      Size  Used Avail Use% Mounted on
/dev/vda3        20G   10G  8.8G  54% /
```

## 🚀 部署方法

由于本地编译的二进制文件是 macOS 格式，无法在 Linux 上运行，我们采用 **Docker 构建** 方法：

### Dockerfile
```dockerfile
FROM rust:1.75-bookworm as builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin xlstatus-server --bin xlstatus-agent

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/xlstatus-server /usr/local/bin/
COPY --from=builder /build/target/release/xlstatus-agent /usr/local/bin/
RUN chmod +x /usr/local/bin/xlstatus-*
WORKDIR /data
ENV DATABASE_URL=sqlite:///data/xlstatus.db
ENV HTTP_BIND=0.0.0.0:8080
ENV GRPC_BIND=0.0.0.0:50051
EXPOSE 8080 50051
CMD ["/usr/local/bin/xlstatus-server"]
```

### 构建命令
```bash
cd /opt/xlstatus
docker build -t xlstatus:test .
```

## 📝 部署步骤

### 1. 准备工作
```bash
# 创建目录
ssh root@wawo-hk-sim-pro2 "mkdir -p /opt/xlstatus"

# 上传源代码
tar czf xlstatus-src.tar.gz \
  --exclude target --exclude node_modules --exclude .git \
  Cargo.toml Cargo.lock crates/ proto/ README.md

scp xlstatus-src.tar.gz root@wawo-hk-sim-pro2:/opt/xlstatus/

# 上传 Dockerfile
scp Dockerfile root@wawo-hk-sim-pro2:/opt/xlstatus/
```

### 2. 构建镜像
```bash
ssh root@wawo-hk-sim-pro2 "cd /opt/xlstatus && docker build -t xlstatus:test ."
```

**预计时间**: 5-10 分钟（首次构建需要下载依赖）

### 3. 运行测试
```bash
# 使用测试脚本
ssh root@wawo-hk-sim-pro2 "bash /opt/xlstatus/test-xlstatus.sh"

# 或手动运行
ssh root@wawo-hk-sim-pro2 "docker run -d \
  --name xlstatus-test \
  -p 8080:8080 \
  -p 50051:50051 \
  -e SESSION_SECRET='test-secret' \
  xlstatus:test"
```

## 🧪 验证测试

### 测试项目

1. ✅ **Docker 镜像构建**
   - 源代码上传成功
   - Dockerfile 配置正确
   - 构建过程启动

2. ⏳ **容器运行测试**（构建完成后）
   - 容器启动
   - 端口监听（8080, 50051）
   - 服务响应

3. ⏳ **功能验证**（构建完成后）
   - HTTP API 可访问
   - gRPC 服务可用
   - 数据库初始化
   - 日志输出正常

### 测试脚本

已上传到服务器 `/opt/xlstatus/test-xlstatus.sh`：

```bash
#!/bin/bash
# 自动化测试脚本
# 验证：镜像存在、容器运行、端口监听、API响应
```

运行测试：
```bash
ssh root@wawo-hk-sim-pro2 "bash /opt/xlstatus/test-xlstatus.sh"
```

## 📊 当前状态

### 已完成
- ✅ 服务器连接验证
- ✅ Docker 环境确认
- ✅ 源代码上传
- ✅ Dockerfile 创建
- ✅ 构建过程启动
- ✅ 测试脚本准备

### 进行中
- ⏳ Docker 镜像构建（预计 5-10 分钟）

### 待完成
- ⏳ 容器运行测试
- ⏳ 功能验证
- ⏳ 性能测试

## 🔍 故障排查

### 如果构建失败

```bash
# 查看构建日志
ssh root@wawo-hk-sim-pro2 "docker logs $(docker ps -lq)"

# 查看失败的构建步骤
ssh root@wawo-hk-sim-pro2 "docker build -t xlstatus:test /opt/xlstatus 2>&1 | tail -50"
```

### 如果容器无法启动

```bash
# 查看容器日志
ssh root@wawo-hk-sim-pro2 "docker logs xlstatus-test"

# 检查端口占用
ssh root@wawo-hk-sim-pro2 "ss -tlnp | grep -E '8080|50051'"
```

### 如果 API 无响应

```bash
# 进入容器检查
ssh root@wawo-hk-sim-pro2 "docker exec -it xlstatus-test bash"

# 容器内测试
curl http://localhost:8080/api/info
```

## 📈 性能预期

基于本地测试结果：

| 指标 | 预期值 |
|------|--------|
| 二进制大小 | Server: ~8.6 MB, Agent: ~971 KB |
| 内存占用 | Server: ~15-30 MB, Agent: ~5-10 MB |
| 启动时间 | Server: ~200ms, Agent: ~100ms |
| HTTP 响应 | < 50ms |
| gRPC 延迟 | < 10ms |

## ✅ 验证清单

- [x] 服务器访问
- [x] Docker 环境
- [x] 源代码传输
- [x] Dockerfile 准备
- [x] 构建启动
- [x] 测试脚本
- [ ] 构建完成
- [ ] 容器运行
- [ ] HTTP 测试
- [ ] gRPC 测试
- [ ] 数据库测试
- [ ] Agent 连接测试

## 🚀 后续步骤

构建完成后：

1. **运行测试脚本**
   ```bash
   ssh root@wawo-hk-sim-pro2 "bash /opt/xlstatus/test-xlstatus.sh"
   ```

2. **访问服务**
   ```bash
   # 获取服务器 IP
   ssh root@wawo-hk-sim-pro2 "hostname -I"

   # 访问（替换为实际 IP）
   curl http://SERVER_IP:8080/api/info
   ```

3. **测试 Agent 连接**
   ```bash
   # 在另一台服务器或容器中运行 agent
   docker run --rm xlstatus:test /usr/local/bin/xlstatus-agent --help
   ```

4. **性能测试**
   ```bash
   # 压力测试（可选）
   ab -n 1000 -c 10 http://SERVER_IP:8080/api/info
   ```

## 📝 验证报告

**最终状态**: ⏳ 构建中

构建完成后，本报告将更新最终验证结果。

---

**创建时间**: 2026-06-17 02:50 UTC
**更新时间**: 待构建完成后更新
**测试人员**: Claude Code (Opus 4.8)
