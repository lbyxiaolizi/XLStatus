---
title: 实施里程碑 M0–M9
status: active
audience: [human, agent]
---

# 11. 实施里程碑

9 个里程碑，总计 ~30 工作日（v1 不含 Web 终端 M7）。

## 总览

| 里程碑 | 工期 | 主题 | 关键产物 | 验证命令 |
|--------|------|------|----------|----------|
| **M0** | 2 天 | 脚手架 | workspace + proto + axum/tonic hello | `cargo build` + `grpcurl list` + `pnpm dev` |
| **M1** | 4 天 | DB + Web Auth | migration + store + login/refresh | curl 全流程 + 迁移成功 |
| **M2** | 4 天 | Agent gRPC | enroll + Session + JWT refresh | mock agent 连上 + DB 行 |
| **M3** | 5 天 | 状态采集展示 | 全部 collector + dashboard 实时 | 浏览器看到数字跳 |
| **M4** | 4 天 | 监控任务 | HTTP/TCP/Ping/SSL + 结果回传 | 加 ping 任务看到 latency |
| **M5** | 3 天 | 告警 | 规则引擎 + Webhook/Telegram | 配规则触发后 Webhook 收到 |
| **M6** | 2 天 | 计划任务 | cron 解析 + Agent 执行 | 配每日 03:00 跑命令 |
| **M7** | — | （v1 不做）Web 终端 | — | — |
| **M8** | 3 天 | PG + TimescaleDB | postgres.rs + CAGG + 压测 | PG 模式 P99 < 200ms |
| **M9** | 3 天 | 收尾 | docker + Caddy + README + 压测脚本 | `docker compose up` 一次起全栈 |

---

## M0 — 脚手架（2 天）

详见 `04-workspace-layout.md`、`05-dependencies.md`、`06-grpc-protocol.md`。

### 步骤

1. **根 `Cargo.toml` 改 workspace**（保留原 `package = XLStatus`，加 `[workspace]` 块引用 `crates/*`）
2. 创建 `proto/xlstatus/v1/{common,agent}.proto`（helloworld 占位即可）
3. `crates/shared/`：写 `ids.rs`、`sample.rs`、`error.rs`、`api_envelope.rs`、`time.rs`、`crypto.rs`、`lib.rs`
4. `crates/proto-gen/`：`build.rs` + `lib.rs`
5. `crates/server/`：
   - `Cargo.toml`（features + 依赖）
   - `migrations/20260101000001_init.sql`（全部表）
   - `src/main.rs`：最小双服务（axum :8080 `/healthz` + tonic :50051 helloworld.Greeter + reflection）
   - `src/config.rs`：toml + env 加载
   - `src/store/`：trait 定义 + sqlite 占位
6. `crates/agent/`：
   - `Cargo.toml`
   - `src/main.rs`：clap CLI 骨架（enroll / run / version 子命令）
7. `web/`：
   - `pnpm create next-app@14 .`（初始化在 `web/` 目录）
   - `pnpm dlx shadcn@latest init`
   - 删示例页面，加 `app/page.tsx` 欢迎页

### 验收

```bash
# 1. workspace 编译
cd /Users/lbyxiaolizi/Documents/Project/XLStatus
cargo build --workspace
# 期望：编译通过，警告可接受

# 2. server 启动
cargo run -p xlstatus-server &
sleep 3

# 3. axum 健康检查
curl http://localhost:8080/healthz
# 期望：200 OK

# 4. gRPC 反射
grpcurl -plaintext localhost:50051 list
# 期望：输出 xlstatus.v1.AgentService + reflection + health

grpcurl -plaintext localhost:50051 describe xlstatus.v1.AgentService
# 期望：包含 Session 方法的描述

# 5. gRPC 健康
grpcurl -plaintext -d '{"service": "xlstatus.v1.AgentService"}' \
    localhost:50051 grpc.health.v1.Health/Check
# 期望：{"status": "SERVING"}

# 6. 前端
cd web
pnpm dev &
sleep 5
curl http://localhost:3000
# 期望：HTML 含 "XLStatus" 字样

# 7. 清理
pkill -f "cargo run -p xlstatus-server"
pkill -f "next dev"
```

### 退出标准

- [ ] 全部 cargo build 通过
- [ ] axum :8080 + tonic :50051 + Next.js :3000 三服务同时可起
- [ ] `grpcurl` 反射能看到 AgentService
- [ ] `web/app/page.tsx` 渲染成功

---

## M1 — 数据库 + Web Auth（4 天）

详见 `07-rest-api.md`、`09-auth-model.md` A 节、`10-database.md`。

### 步骤

1. 完成 `migrations/20260101000002_indexes.sql`、`20260101000004_seed_admin.sql`
2. `crates/server/src/store/sqlite.rs`：实现 `AgentStore`/`UserStore`/`EnrollmentStore` trait
3. `crates/server/src/auth/password.rs`：argon2 包装
4. `crates/server/src/auth/jwt.rs`：access JWT 签发/校验
5. `crates/server/src/auth/session_cookie.rs`：cookie 读写中间件
6. `crates/server/src/api/auth.rs`：`/auth/login`、`/refresh`、`/logout`、`/me` 4 个端点
7. `crates/server/src/main.rs`：挂载路由 + `RequireAuth` 中间件 + 限流
8. `crates/server/src/ratelimit.rs`：`tower_governor` 集成
9. 集成测试：`tests/auth.rs` 跑完整 login → me → refresh 流程

### 验收

```bash
# 启动 server
XLSTATUS_SEED_ADMIN_PASSWORD=admin123 cargo run -p xlstatus-server &

# 1. 登录
curl -i -c /tmp/cookies.txt -X POST http://localhost:8080/api/v1/auth/login \
    -H "Content-Type: application/json" \
    -d '{"username":"admin","password":"admin123"}'
# 期望：200 + Set-Cookie access_token + refresh_token

# 2. 查 me
curl -b /tmp/cookies.txt http://localhost:8080/api/v1/auth/me
# 期望：200 + {"data": {"user": {"username":"admin", "is_admin": true}}}

# 3. 错误密码
curl -X POST http://localhost:8080/api/v1/auth/login \
    -H "Content-Type: application/json" \
    -d '{"username":"admin","password":"wrong"}'
# 期望：401 + {"error": {"code": "unauthenticated", ...}}

# 4. 错误密码连试 6 次
for i in {1..6}; do
    curl -X POST http://localhost:8080/api/v1/auth/login \
        -H "Content-Type: application/json" \
        -d '{"username":"admin","password":"wrong"}' -w "\n%{http_code}\n"
done
# 期望：第 6 次返回 429 + rate_limited

# 5. Refresh
curl -i -b /tmp/cookies.txt -c /tmp/cookies.txt -X POST \
    http://localhost:8080/api/v1/auth/refresh
# 期望：200 + 新 Set-Cookie（rotation 验证：旧 cookie 再用会 401）

# 6. 登出
curl -b /tmp/cookies.txt -X POST http://localhost:8080/api/v1/auth/logout
# 期望：204，refresh token 撤销

# 7. DB 验证
sqlite3 data/xlstatus.db "SELECT id, username, is_admin FROM users;"
# 期望：1 行，admin/1
```

### 退出标准

- [ ] login/refresh/logout/me 4 端点全通
- [ ] cookie HttpOnly + SameSite=Lax
- [ ] 限流 5/min/IP 在第 6 次触发
- [ ] refresh rotation 工作（旧 token 第二次用 401）
- [ ] `tests/auth.rs` 集成测试通过

---

## M2 — Agent gRPC 接入（4 天）

详见 `06-grpc-protocol.md`、`09-auth-model.md` B 节。

### 步骤

1. 完善 `proto/xlstatus/v1/agent.proto`（用 plan 定义的完整版替换 helloworld）
2. `crates/server/src/grpc_server/`：
   - `interceptor.rs`：JWT 校验
   - `auth.rs`：agent JWT 签发
   - `proto_to_domain.rs`：转换函数
   - `service.rs`：`Session` RPC 实现
3. `crates/server/src/api/agent_enroll.rs`：`POST /agent/enroll` 端点
4. `crates/server/src/api/agents.rs`：`/agents` CRUD + enrollment token 颁发 + sessions 踢出
5. `crates/server/src/auth/agent_jwt.rs`：5min JWT
6. `crates/server/src/store/sqlite.rs`：补充 `AgentSessionStore`、`EnrollmentStore`
7. `crates/agent/src/keys.rs`：私钥持久化（0600, zeroize）
8. `crates/agent/src/auth.rs`：challenge 签名
9. `crates/agent/src/grpc_client.rs`：tonic client + reconnect + JWT 缓存
10. `crates/agent/src/enroll.rs`：首次注册流程
11. `crates/agent/src/main.rs`：接入 enroll/run 子命令

### 验收

```bash
# 启动 server
cargo run -p xlstatus-server &

# 1. 创建 agent
curl -b /tmp/cookies.txt -X POST http://localhost:8080/api/v1/agents \
    -H "Content-Type: application/json" \
    -d '{"name":"devbox"}'
# 期望：201 + {"data": {"id": "..."}}

AGENT_ID="..."

# 2. 颁发 enrollment token
curl -b /tmp/cookies.txt -X POST \
    http://localhost:8080/api/v1/agents/$AGENT_ID/enrollment-token \
    -H "Content-Type: application/json" \
    -d '{"name":"devbox-enroll"}'
# 期望：200 + {"data": {"token": "et_..."}}

TOKEN="et_..."

# 3. Agent 注册
cd /Users/lbyxiaolizi/Documents/Project/XLStatus
cargo run -p xlstatus-agent -- enroll \
    --server http://localhost:8080 \
    --token $TOKEN \
    --name devbox
# 期望：输出 "enrolled, agent_id stored at /var/lib/xlstatus/agent.key"

# 4. Agent 启动
cargo run -p xlstatus-agent -- run \
    --server http://localhost:8080 \
    --grpc-server localhost:50051
# 期望：开始周期性 state 上报

# 5. DB 验证
sqlite3 data/xlstatus.db "SELECT id, name, last_seen_at FROM agents;"
# 期望：last_seen_at 在 10s 内

sqlite3 data/xlstatus.db "SELECT COUNT(*) FROM state_samples WHERE agent_id = '$AGENT_ID';"
# 期望：> 0

# 6. gRPC 反射
grpcurl -plaintext localhost:50051 list
# 期望：能看到 AgentService

# 7. 踢出
curl -b /tmp/cookies.txt -X DELETE \
    http://localhost:8080/api/v1/agents/$AGENT_ID/sessions
# 期望：agent 端日志显示 disconnected, reason: AGENT_REVOKED
```

### 退出标准

- [ ] Agent enroll → run 全流程通
- [ ] Server DB 有 agent + state_samples 行
- [ ] JWT 续签工作（5min 边界 agent 自动续）
- [ ] 踢出端点断开 agent
- [ ] `tests/grpc.rs` 集成测试通过

---

## M3 — 状态采集与展示（5 天）

详见 `06-grpc-protocol.md` 状态字段、`08-websocket.md`、`04-workspace-layout.md` web 部分。

### 步骤

1. `crates/agent/src/collector/`：全部 9 个子模块
2. `crates/agent/src/collector/host.rs`：启动时一次性采集
3. `crates/agent/src/reporter.rs`：周期性 state 上报 + 攒批 + backpressure
4. `crates/server/src/domain/sample_batch.rs`：服务端 1s 攒批 + 写盘
5. `crates/server/src/domain/session.rs`：在线 agent 缓存
6. `crates/server/src/ws/{hub,client,messages}.rs`：Dashboard WS 服务
7. `crates/server/src/api/samples.rs`：时序查询
8. `crates/server/src/api/agents.rs`：补充 list 详情
9. `web/app/(authed)/dashboard/page.tsx`：服务器列表
10. `web/app/(authed)/dashboard/[id]/page.tsx`：单机详情
11. `web/components/server/`：`ServerCard` / `StatusBadge` / `HostInfoPanel`
12. `web/components/charts/`：`MetricLineChart` / `Sparkline` / `NetworkAreaChart`
13. `web/lib/ws.ts` + `useWebSocket` hook

### 验收

```bash
# 启动 server
cargo run -p xlstatus-server &

# 启动 2 个 agent
cargo run -p xlstatus-agent -- run &
cargo run -p xlstatus-agent -- run --name box2 &

# 启动前端
cd web && pnpm dev &

# 浏览器打开 http://localhost:3000
# - 登录 admin
# - 看到 2 个 agent 卡片
# - 点击进入详情，看到 CPU/Mem/Load 折线图
# - 数字应每 10s 跳动
# - 关闭一个 agent，对应卡片应 30s 内变灰（offline）

# 截图存到 docs/screenshots/dashboard.png
```

### 退出标准

- [ ] 列表页能看到所有 agent
- [ ] 详情页图表 X 轴时间正确
- [ ] WS 实时数字跳（延迟 < 2s）
- [ ] agent 离线 30s 内变灰
- [ ] 24h 趋势查询 < 200ms（SQLite 单机）

---

## M4 — 监控任务（4 天）

### 步骤

1. `crates/server/src/api/monitor.rs`：监控任务 CRUD + results
2. `crates/server/src/domain/task_dispatch.rs`：分配任务给 agent
3. `crates/server/src/grpc_server/service.rs`：收到 ClientMessage.task_result → 写 DB
4. `crates/agent/src/task_runner.rs`：HTTP/TCP/Ping/SSL 执行器
5. `web/app/(authed)/services/page.tsx`：监控任务列表
6. 集成测试

### 验收

```bash
# 1. 创建 ping 任务
curl -b /tmp/cookies.txt -X POST http://localhost:8080/api/v1/monitor-tasks \
    -H "Content-Type: application/json" \
    -d '{"name":"ping-google","kind":"ping","target":"google.com","interval_s":60,"timeout_s":3}'

# 2. 60s 后查结果
sqlite3 data/xlstatus.db "SELECT task_id, ts, successful, delay_ms FROM monitor_results ORDER BY ts DESC LIMIT 5;"

# 3. Dashboard 看到 latency 时序图
```

---

## M5 — 告警（3 天）

### 步骤

1. `crates/server/src/domain/alert.rs`：规则引擎（每 5s 评估）
2. `crates/server/src/domain/notifier.rs`：trait + Webhook/Telegram/Email 实现
3. `crates/server/src/api/alerts.rs`：`/alert-rules` CRUD + `/alert-events`
4. `crates/server/src/api/notifiers.rs`：`/notifiers` CRUD + `/test`
5. `web/app/(authed)/alerts/page.tsx`：告警规则 + 事件历史

### 验收

```bash
# 1. 创建 Webhook notifier
curl -b /tmp/cookies.txt -X POST http://localhost:8080/api/v1/notifiers \
    -H "Content-Type: application/json" \
    -d '{"name":"test","kind":"webhook","config":{"url":"http://localhost:9999/hook"}}'

# 2. 创建 CPU>80% 规则
curl -b /tmp/cookies.txt -X POST http://localhost:8080/api/v1/alert-rules \
    -H "Content-Type: application/json" \
    -d '{"name":"high-cpu","metric":"cpu","operator":"gt","threshold":0.8,"duration_s":30,"notifier_ids":["..."]}'

# 3. 启动接收 webhook 的服务（python -m http.server 9999）
# 4. 触发高 CPU（yes > /dev/null &）
# 5. 30s 后查 alert_events 并验证 webhook 收到 POST
```

---

## M6 — 计划任务（2 天，可选）

### 步骤

1. 引入 `cron` crate
2. `crates/server/src/domain/scheduler.rs`：调度器
3. `crates/agent/src/task_runner.rs`：扩展支持 shell 命令
4. `web/app/(authed)/tasks/page.tsx`：UI

---

## M8 — PostgreSQL + TimescaleDB（3 天）

### 步骤

1. `crates/server/src/store/postgres.rs`：实现 store trait（PG 方言）
2. `crates/server/src/store/timescaledb.rs`：hypertable + CAGG migration 逻辑
3. `crates/server/src/main.rs`：根据 feature flag 选 sqlite/postgres
4. 切换编译：`cargo build --no-default-features --features storage-postgres`
5. docker-compose 启动 PG + TimescaleDB
6. 跑 mock_agent 50 个 × 10s × 1h 压测
7. 验收查询 P99

### 验收

```bash
# 启动 PG+TS
docker compose up -d timescaledb

# 编译
cargo run -p xlstatus-server --no-default-features --features storage-postgres

# 压测
cargo run --bin mock_agent -- --count 50 --interval 10s --duration 1h &

# 1h 后查响应时间
psql -c "SELECT count(*) FROM state_samples;"
psql -c "SELECT pg_size_pretty(pg_total_relation_size('state_samples'));"

# 查询 P99
psql -c "\timing on
SELECT * FROM state_samples_5min
WHERE agent_id = '...' AND bucket > NOW() - INTERVAL '1 hour'
ORDER BY bucket DESC LIMIT 1000;"
```