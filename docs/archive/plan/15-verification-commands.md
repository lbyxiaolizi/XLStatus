# 命令级验收

## M0 脚手架

```bash
cd /Users/lbyxiaolizi/Documents/Project/XLStatus
cargo build --workspace
cargo run -p xlstatus-server &
sleep 3
curl -f http://localhost:8080/healthz
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext localhost:50051 describe xlstatus.v1.AgentService
cd web && pnpm install && pnpm dev &
sleep 5
curl -f http://localhost:3000
```

## M1 Web Auth + DB

```bash
XLSTATUS_SEED_ADMIN_PASSWORD=replace-with-a-strong-initial-password \
DATABASE_URL=sqlite:///tmp/xlstatus.db \
cargo run -p xlstatus-server &

curl -i -c /tmp/xlstatus.cookies \
  -X POST http://localhost:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"replace-with-a-strong-initial-password"}'

curl -b /tmp/xlstatus.cookies http://localhost:8080/api/v1/profile
curl -i -b /tmp/xlstatus.cookies -c /tmp/xlstatus.cookies \
  -X POST http://localhost:8080/api/v1/auth/refresh
curl -i -b /tmp/xlstatus.cookies \
  -X POST http://localhost:8080/api/v1/auth/logout
```

PostgreSQL 同步验收：

```bash
docker compose -f docker-compose.pg.yml up -d postgres
DATABASE_URL=postgres://xlstatus:xlstatus@localhost:5432/xlstatus \
cargo test -p xlstatus-server db_repository
```

## M2 Agent gRPC

```bash
cargo run -p xlstatus-server &

curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/servers \
  -H "Content-Type: application/json" \
  -d '{"name":"devbox"}'

curl -b /tmp/xlstatus.cookies -X POST \
  http://localhost:8080/api/v1/servers/$SERVER_ID/enrollment-token

cargo run -p xlstatus-agent -- enroll \
  --server http://localhost:8080 \
  --grpc-server http://localhost:50051 \
  --token "$TOKEN"

cargo run -p xlstatus-agent -- run --config ./agent.yaml
grpcurl -plaintext localhost:50051 list
```

## M3 状态展示

```bash
cargo run -p xlstatus-server &
cargo run -p xlstatus-agent -- run --config ./agent.yaml &
cd web && pnpm dev &

# 浏览器验收：
# 1. 登录 admin
# 2. 首页出现 server
# 3. CPU/Mem/Load 数字持续变化
# 4. 停掉 agent 后 30 秒内显示 offline
```

## M4 服务监控与告警

```bash
curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/services \
  -H "Content-Type: application/json" \
  -d '{"name":"ping-local","kind":"tcp","target":"127.0.0.1:8080","duration_seconds":30}'

curl -b /tmp/xlstatus.cookies http://localhost:8080/api/v1/services/$SERVICE_ID/history

curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/alert-rules \
  -H "Content-Type: application/json" \
  -d '{"name":"cpu-high","enabled":true,"rules":[{"type":"cpu","max":80,"duration":3}]}'
```

## M5 运维能力

```bash
curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{"name":"echo","task_type":"trigger","command":"echo ok","servers":["'$SERVER_ID'"]}'

curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/tasks/$TASK_ID/run

# 浏览器验收：
# - 打开 Web Terminal 执行 echo ok
# - 文件管理上传、读取、删除临时文件
# - 100 MiB 以下文件传输成功
```

## M6 DDNS/NAT/MCP

```bash
curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/ddns \
  -H "Content-Type: application/json" \
  -d '{"name":"dummy","provider":"dummy","enable_ipv4":true,"domains":["example.com"]}'

curl -b /tmp/xlstatus.cookies -X POST http://localhost:8080/api/v1/nat \
  -H "Content-Type: application/json" \
  -d '{"name":"local","server_id":"'$SERVER_ID'","domain":"local.example.com","target_host":"127.0.0.1:8080"}'

curl -H "Authorization: Bearer $PAT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://localhost:8080/mcp
```

## M8 性能

```bash
DATABASE_URL=postgres://xlstatus:xlstatus@localhost:5432/xlstatus \
cargo run -p xlstatus-server &

cargo run -p xlstatus-xtask -- mock-agents \
  --count 100 \
  --interval 3s \
  --duration 24h

cargo run -p xlstatus-xtask -- query-bench \
  --period 1d,7d,30d \
  --p95-target-ms 500
```
