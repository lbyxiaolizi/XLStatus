---
title: 数据库 Schema
status: stable
audience: [human, agent]
related_milestones: [M1, M8]
---

# 10. 数据库 Schema

`sqlx` 双适配：SQLite（默认）+ PostgreSQL（feature flag）。本文件给出双方言通用 schema + PG 专属脚本 + SQLite 物化表策略。

## 通用 schema（双方言都执行）

`crates/server/migrations/20260101000001_init.sql`：

```sql
-- ========== 用户 ==========
CREATE TABLE users (
  id              UUID PRIMARY KEY,
  username        TEXT NOT NULL UNIQUE,
  email           TEXT,
  password_hash   TEXT NOT NULL,                 -- argon2id PHC string
  is_admin        BOOLEAN NOT NULL DEFAULT 0,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE user_sessions (
  id                 UUID PRIMARY KEY,
  user_id            UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  refresh_token_hash TEXT NOT NULL,              -- sha256 hex
  user_agent         TEXT,
  ip                 TEXT,
  created_at         TIMESTAMPTZ NOT NULL,
  last_used_at       TIMESTAMPTZ NOT NULL,
  expires_at         TIMESTAMPTZ NOT NULL,
  revoked_at         TIMESTAMPTZ
);
CREATE INDEX user_sessions_user_id_idx ON user_sessions(user_id);
CREATE INDEX user_sessions_expires_at_idx ON user_sessions(expires_at) WHERE revoked_at IS NULL;

-- ========== Agent ==========
CREATE TABLE agents (
  id              UUID PRIMARY KEY,
  name            TEXT NOT NULL,
  public_key      BLOB NOT NULL,                 -- 32 字节 Ed25519
  agent_version   TEXT,
  tags            JSONB NOT NULL DEFAULT '[]',
  created_at      TIMESTAMPTZ NOT NULL,
  updated_at      TIMESTAMPTZ NOT NULL,
  last_seen_at    TIMESTAMPTZ,
  revoked         BOOLEAN NOT NULL DEFAULT 0
);
CREATE INDEX agents_last_seen_at_idx ON agents(last_seen_at);

CREATE TABLE agent_sessions (
  id              UUID PRIMARY KEY,
  agent_id        UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  jwt_jti         TEXT NOT NULL UNIQUE,
  issued_at       TIMESTAMPTZ NOT NULL,
  expires_at      TIMESTAMPTZ NOT NULL,
  revoked_at      TIMESTAMPTZ,
  last_active_at  TIMESTAMPTZ
);
CREATE INDEX agent_sessions_agent_id_idx ON agent_sessions(agent_id);

CREATE TABLE enrollment_tokens (
  id          UUID PRIMARY KEY,
  token_hash  TEXT NOT NULL,                     -- sha256(token)
  name        TEXT,
  created_by  UUID REFERENCES users(id),
  created_at  TIMESTAMPTZ NOT NULL,
  expires_at  TIMESTAMPTZ NOT NULL,
  used_at     TIMESTAMPTZ,
  used_by     UUID REFERENCES agents(id)
);
CREATE INDEX enrollment_tokens_hash_idx ON enrollment_tokens(token_hash);

-- 静态主机信息
CREATE TABLE host_info (
  agent_id            UUID PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
  platform            TEXT,
  platform_version    TEXT,
  arch                TEXT,
  cpu                 JSONB,
  mem_total           BIGINT,
  disk_total          BIGINT,
  swap_total          BIGINT,
  virtualization      TEXT,
  boot_time           BIGINT,
  gpu                 JSONB,
  updated_at          TIMESTAMPTZ NOT NULL
);

-- ========== 状态采样（时序） ==========
CREATE TABLE state_samples (
  agent_id         UUID NOT NULL,
  ts               TIMESTAMPTZ NOT NULL,
  cpu              REAL,
  mem_used         BIGINT,
  swap_used        BIGINT,
  disk_used        BIGINT,
  net_in_speed     BIGINT,
  net_out_speed    BIGINT,
  net_in_transfer  BIGINT,
  net_out_transfer BIGINT,
  load1            REAL, load5 REAL, load15 REAL,
  uptime           BIGINT,
  tcp_conn_count   BIGINT,
  udp_conn_count   BIGINT,
  process_count    BIGINT,
  temperatures     JSONB,
  gpu              JSONB
);
CREATE INDEX state_samples_agent_ts_idx ON state_samples(agent_id, ts DESC);

-- ========== 监控任务 ==========
CREATE TABLE monitor_tasks (
  id           UUID PRIMARY KEY,
  name         TEXT NOT NULL,
  kind         TEXT NOT NULL,                    -- http/tcp/ping/ssl
  target       TEXT NOT NULL,
  interval_s   INTEGER NOT NULL,
  timeout_s    INTEGER NOT NULL DEFAULT 5,
  enabled      BOOLEAN NOT NULL DEFAULT 1,
  agent_id     UUID REFERENCES agents(id) ON DELETE SET NULL,
  created_at   TIMESTAMPTZ NOT NULL,
  updated_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX monitor_tasks_enabled_idx ON monitor_tasks(enabled);

CREATE TABLE monitor_results (
  id          BIGSERIAL PRIMARY KEY,
  task_id     UUID NOT NULL REFERENCES monitor_tasks(id) ON DELETE CASCADE,
  agent_id    UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  ts          TIMESTAMPTZ NOT NULL,
  successful  BOOLEAN NOT NULL,
  delay_ms    REAL,
  data        TEXT
);
CREATE INDEX monitor_results_task_ts_idx ON monitor_results(task_id, ts DESC);
CREATE INDEX monitor_results_agent_ts_idx ON monitor_results(agent_id, ts DESC);

-- ========== 告警 ==========
CREATE TABLE alert_rules (
  id           UUID PRIMARY KEY,
  name         TEXT NOT NULL,
  agent_id     UUID REFERENCES agents(id) ON DELETE CASCADE,  -- NULL = all
  metric       TEXT NOT NULL,
  operator     TEXT NOT NULL,
  threshold    REAL,
  duration_s   INTEGER NOT NULL DEFAULT 60,
  enabled      BOOLEAN NOT NULL DEFAULT 1,
  notifier_ids JSONB NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL,
  updated_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX alert_rules_enabled_idx ON alert_rules(enabled);

CREATE TABLE notifiers (
  id         UUID PRIMARY KEY,
  name       TEXT NOT NULL,
  kind       TEXT NOT NULL,                      -- telegram/webhook/email/lark/dingtalk
  config     JSONB NOT NULL,
  enabled    BOOLEAN NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE alert_events (
  id            UUID PRIMARY KEY,
  rule_id       UUID NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
  agent_id      UUID REFERENCES agents(id) ON DELETE SET NULL,
  status        TEXT NOT NULL,                    -- firing/resolved
  triggered_at  TIMESTAMPTZ NOT NULL,
  resolved_at   TIMESTAMPTZ,
  trigger_value REAL,
  message       TEXT
);
CREATE INDEX alert_events_triggered_at_idx ON alert_events(triggered_at DESC);
CREATE INDEX alert_events_rule_agent_idx ON alert_events(rule_id, agent_id, triggered_at DESC);
```

## PG 专属：hypertable + CAGG

`crates/server/migrations/20260101000003_pg_hypertable.sql`（仅 `storage-postgres` 模式执行）：

```sql
SELECT create_hypertable('state_samples', 'ts', chunk_time_interval => INTERVAL '1 day');
SELECT create_hypertable('monitor_results', 'ts', chunk_time_interval => INTERVAL '7 days');

CREATE MATERIALIZED VIEW state_samples_5min
WITH (timescaledb.continuous) AS
SELECT agent_id,
       time_bucket(INTERVAL '5 minutes', ts) AS bucket,
       avg(cpu) AS cpu,
       avg(mem_used) AS mem_used,
       avg(net_in_speed) AS net_in_speed,
       avg(net_out_speed) AS net_out_speed,
       avg(load1) AS load1, avg(load5) AS load5, avg(load15) AS load15
FROM state_samples GROUP BY agent_id, bucket;

CREATE MATERIALIZED VIEW state_samples_1h
WITH (timescaledb.continuous) AS
SELECT agent_id, time_bucket(INTERVAL '1 hour', ts) AS bucket,
       avg(cpu) AS cpu, avg(mem_used) AS mem_used
FROM state_samples GROUP BY agent_id, bucket;

-- 自动保留策略
SELECT add_retention_policy('state_samples', INTERVAL '7 days');
SELECT add_retention_policy('state_samples_5min', INTERVAL '30 days');
SELECT add_retention_policy('state_samples_1h', INTERVAL '365 days');
```

## SQLite 物化表

`crates/server/migrations/20260101000005_sqlite_rollup.sql`（仅 `storage-sqlite` 模式）：

```sql
CREATE TABLE state_samples_5min (
  agent_id     UUID NOT NULL,
  bucket       TIMESTAMPTZ NOT NULL,
  cpu          REAL,
  mem_used     BIGINT,
  net_in_speed BIGINT,
  net_out_speed BIGINT,
  load1        REAL, load5 REAL, load15 REAL,
  PRIMARY KEY (agent_id, bucket)
);
CREATE INDEX state_samples_5min_bucket_idx ON state_samples_5min(bucket);

CREATE TABLE state_samples_1h (
  agent_id UUID NOT NULL,
  bucket   TIMESTAMPTZ NOT NULL,
  cpu      REAL,
  mem_used BIGINT,
  PRIMARY KEY (agent_id, bucket)
);
CREATE INDEX state_samples_1h_bucket_idx ON state_samples_1h(bucket);
```

### SQLite Rollup 任务

`crates/server/src/domain/sample_batch.rs` 启动时 spawn：

- 每 60s 一次 5min rollup：把超 7 天的 raw 数据按 5min 聚合到 `state_samples_5min`（用 `INSERT OR REPLACE`）
- 每小时一次 1h rollup：把超 30 天的 5min 数据按 1h 聚合到 `state_samples_1h`
- 每天一次清理：`DELETE FROM state_samples_1h WHERE bucket < NOW() - INTERVAL '1 year'`

## Seed admin

`crates/server/migrations/20260101000004_seed_admin.sql`：

```sql
-- 默认 admin 账号，密码从环境变量 XLSTATUS_SEED_ADMIN_PASSWORD 读
-- 启动时若 password_hash 为占位符 '<argon2-hash>'，则用 env 变量算 hash 替换
INSERT INTO users (id, username, password_hash, is_admin, created_at, updated_at)
VALUES (
  '00000000-0000-0000-0000-000000000001',
  'admin',
  '<argon2-hash>',
  1,
  CURRENT_TIMESTAMP,
  CURRENT_TIMESTAMP
)
ON CONFLICT (id) DO NOTHING;
```

启动时：
```rust
// crates/server/src/main.rs
if let Some(pw) = std::env::var("XLSTATUS_SEED_ADMIN_PASSWORD").ok() {
    let hash = argon2_hash(&pw);
    sqlx::query!("UPDATE users SET password_hash = ? WHERE id = ?", hash, ADMIN_ID)
        .execute(&pool).await?;
}
```

首次启动会在日志打印临时密码（如果 env 未设）：

```
WARN: using default admin password "xlstatus-change-me-now", please change after first login
```

## 迁移命令

```bash
# 安装 sqlx-cli
cargo install sqlx-cli --no-default-features --features sqlite,rustls
cargo install sqlx-cli --no-default-features --features postgres,rustls

# 创建新 migration
sqlx migrate add -s crates/server/migrations <name>

# 跑 migration
sqlx migrate run --source crates/server/migrations --database-url $XLSTATUS_DB_URL

# 准备查询（生成 sqlx-data.json 给编译期用）
cargo sqlx prepare --workspace
```

## 索引策略

| 表 | 索引 | 用途 |
|----|------|------|
| agents | last_seen_at | 列表按最近活跃排序 |
| state_samples | (agent_id, ts DESC) | 单机时间范围查询 |
| monitor_results | (task_id, ts DESC) | 单任务结果 |
| monitor_results | (agent_id, ts DESC) | 单 agent 结果 |
| alert_events | triggered_at DESC | 事件流 |
| alert_events | (rule_id, agent_id, triggered_at DESC) | 规则+agent 组合查 |
| user_sessions | user_id | 用户 session 列表 |
| user_sessions | expires_at (WHERE revoked_at IS NULL) | 清理过期未撤销 |
| agent_sessions | agent_id | agent session 列表 |
| enrollment_tokens | token_hash | 注册时查找 |