---
title: 验证策略
status: stable
audience: [human, agent]
related_milestones: [all]
---

# 12. 验证策略

每个里程碑结束需通过对应层级的验证。

## 验证层级

| 层级 | 工具 | 频率 | 谁负责 |
|------|------|------|--------|
| **L1 编译** | `cargo check` / `cargo build` | 每改即跑 | 实施者 |
| **L2 单元** | `cargo test -p <crate>` | 每改即跑 | 实施者 |
| **L3 集成** | `cargo test --workspace` + docker compose up | 每个 milestone 结束 | 实施者 |
| **L4 端到端** | 浏览器手动 + curl + grpcurl | 每个 milestone 结束 | 实施者 + 复核者 |
| **L5 性能** | mock_agent 压测 | M8 + M9 | 实施者 |

## L1 编译

```bash
# 每次改完代码
cargo check --all-targets --all-features
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

## L2 单元测试

### 必覆盖模块

| 模块 | 测试类型 |
|------|----------|
| `shared::ids` | UUID v7 生成 + parse 往返 |
| `shared::api_envelope` | serde 往返（成功 + 错误 + 分页） |
| `server::auth::password` | argon2 verify 正确/错误 |
| `server::auth::jwt` | 签发/校验/过期/revoke |
| `server::auth::agent_jwt` | challenge-response（伪造/过期/重放/吊销） |
| `server::domain::alert` | 规则评估纯函数（CPU>80 60s 等） |
| `server::domain::sample_batch` | 攒批逻辑（不同时间窗口） |
| `server::store::sqlite` | CRUD + 时序聚合查询 |
| `agent::collector::cpu` | sysinfo 注入 mock 断言 |
| `agent::collector::net` | net speed 计算正确性 |
| `agent::task_runner` | HTTP/TCP/Ping/SSL 各 1 个测试 |

### 跑法

```bash
cargo test --workspace
cargo test -p xlstatus-server           # 单 crate
cargo test -p xlstatus-server auth::     # 路径过滤
cargo test -- --nocapture               # 看 println
```

## L3 集成测试

### 位置：`crates/server/tests/`

```
crates/server/tests/
├── common/
│   └── mod.rs           # 共享 test helpers
├── auth.rs              # 完整 login → me → refresh
├── grpc.rs              # mock agent 跑 Session
├── samples.rs           # 写 + 查时序
└── alerts.rs            # 规则触发 + notifier 发送
```

### 模式

```rust
// crates/server/tests/auth.rs 片段
#[tokio::test]
async fn login_me_refresh_logout_flow() {
    let app = spawn_test_app().await;  // 内存 sqlite + axum
    let client = reqwest::Client::new();

    let resp = client.post(format!("{}/api/v1/auth/login", app.addr))
        .json(&json!({"username": "admin", "password": "admin123"}))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let cookies: Vec<_> = resp.headers().get_all("set-cookie").iter().collect();
    assert!(cookies.iter().any(|c| c.to_str().unwrap().contains("access_token")));

    let cookie_jar = CookieJar::from_response(&resp);

    let me = client.get(format!("{}/api/v1/auth/me", app.addr))
        .headers(cookie_jar.to_headers())
        .send().await.unwrap();
    assert_eq!(me.status(), 200);

    // ... refresh, logout, 401 验证
}
```

### 数据库

- 用 in-memory SQLite (`sqlite::memory:`) + 跑 migrations
- 每个测试独立 db 实例
- `spawn_test_app()` 在 `crates/server/tests/common/mod.rs`

## L4 端到端（手动）

### 每个 milestone 必备清单

- [ ] `curl` 验证主要 REST 端点（状态码 + 错误信封）
- [ ] `grpcurl` 验证 gRPC 反射 + 关键方法
- [ ] 浏览器登录 → 看到目标功能渲染 → 操作一遍

### 烟测脚本

`scripts/smoke.sh`（每个 milestone 写对应段）：

```bash
#!/bin/bash
set -e
trap 'pkill -P $$' EXIT

# 启动 server
cargo run -p xlstatus-server &
SERVER_PID=$!
sleep 3

# Health
echo "=== Health ==="
curl -sf http://localhost:8080/healthz | jq

# gRPC reflection
echo "=== gRPC reflection ==="
grpcurl -plaintext localhost:50051 list

# ... 后续测试

kill $SERVER_PID
```

## L5 性能基线

### 目标

| 指标 | 目标 | 测量 |
|------|------|------|
| Server CPU（1 核） | < 30% | `top -p $(pgrep xlstatus-server)` |
| Server 内存 | < 250MB | `ps -orss= -p $(pgrep xlstatus-server)` |
| WS push 延迟 | P99 < 50ms | 自定义 `tracing` span |
| REST 24h 趋势查询 | P99 < 200ms | `curl -w "%{time_total}\n"` |
| gRPC state 接收 | 1000 msg/s 零丢失 | mock_agent 压测 |

### 压测命令

```bash
# 50 个虚拟 agent × 10s × 1h
cargo run --bin mock_agent --release -- \
    --count 50 \
    --interval 10s \
    --duration 1h \
    --server localhost:50051 &

# 监控
watch -n 5 'ps -orss= -p $(pgrep xlstatus-server)'
```

## 验证失败处理

| 失败 | 处理 |
|------|------|
| L1 编译 | 立即修复，不继续 |
| L2 单元 | 修复或调整测试 |
| L3 集成 | 修复或更新 test helper |
| L4 端到端 | 记录 issue，里程碑不算完成 |
| L5 性能 | 分析瓶颈（perf / flamegraph），优化或调整目标 |