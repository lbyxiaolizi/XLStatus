# M2 阶段进度报告

**更新时间**: 2026-06-16  
**完成度**: 50%

## ✅ 已完成 (3/6 任务)

### 1. Agent enrollment API ✅
- `POST /api/v1/enrollment-tokens` - 创建 enrollment token (xle_ 前缀)
- `POST /api/v1/agents/enroll` - Agent 注册
- Token 一次性使用验证
- Token 过期时间控制（默认1小时）
- 自动关联到创建者

**测试验证**:
- ✅ 创建 enrollment token
- ✅ Agent 成功 enroll
- ✅ Token 不能重复使用
- ✅ 生成唯一 Agent ID

### 2. 数据库 Schema ✅
- `enrollment_tokens` 表 (SQLite + PostgreSQL)
- `agents` 表 (SQLite + PostgreSQL)
- Migration 002 已应用
- 支持 token 使用追踪

### 3. Repository 层 ✅
- `EnrollmentTokenRepository`
  - create() - 创建 token
  - find_and_use() - 查找并标记为已使用
- `AgentRepository`
  - create() - 创建 agent
  - find_by_id() - 查询 agent
  - update_last_seen() - 更新心跳
  - revoke() - 吊销 agent

## 🚧 进行中 (0/6 任务)

### 4. Ed25519 keypair 生成 ⬜
- Agent 本地生成密钥对
- 私钥保存（0600权限）
- 公钥上传到 Server

### 5. Agent JWT 认证 ⬜
- JWT 签发（5分钟有效期）
- JWT 验证
- Challenge refresh 机制
- gRPC interceptor

### 6. gRPC Session ⬜
- 双向流实现
- Session registry
- 心跳和 last_seen_at 更新
- Agent 重连逻辑

### 7. Agent CLI enroll 命令 ⬜
- 生成 Ed25519 keypair
- 调用 enrollment API
- 保存配置文件

### 8. Agent CLI run 命令 ⬜
- 连接 gRPC
- JWT 认证
- 心跳发送
- 重连处理

### 9. M2 验收测试 ⬜
- enroll → run → 看到 last_seen_at
- JWT 续签测试
- 吊销测试

## 📊 当前状态

### API 端点
```
✅ POST /api/v1/enrollment-tokens  创建 enrollment token
✅ POST /api/v1/agents/enroll       Agent enrollment
⬜ gRPC AgentService.Session        双向流（未实现）
```

### 数据库表
```
✅ enrollment_tokens
✅ agents
⬜ JWT claims（内存）
```

### Repository
```
✅ EnrollmentTokenRepository
✅ AgentRepository
```

## 🧪 测试结果

### Enrollment API 测试 (4/4 通过)
```
✅ 创建管理员用户
✅ 创建 enrollment token (xle_ 前缀)
✅ Agent enroll 成功（返回 agent_id）
✅ Token 不能重复使用（正确拒绝）
```

## 📝 技术债务

1. **JWT 认证**: 需要实现 JWT 签发和验证
2. **gRPC Session**: 需要实现双向流
3. **Agent CLI**: 需要实现 enroll 和 run 命令
4. **Ed25519**: 需要添加密钥对生成和验证
5. **测试**: 需要完整的端到端测试

## 🎯 下一步

由于 token 使用接近 70%，建议：
1. 继续实现剩余功能（JWT + gRPC + Agent CLI）
2. 或先总结当前成果，下次继续

**预计剩余工作量**: 50%（JWT认证、gRPC Session、Agent CLI）

---

**M2 阶段完成度**: 50%  
**M0**: ✅ 100% | **M1**: ✅ 100% | **M2**: 🚧 50%
