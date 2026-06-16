# 数据模型

## 原则

- SQL 元数据层支持 SQLite 和 PostgreSQL，TSDB 存时间序列指标。
- SQLite 用于开发、小规模单机和演示部署；PostgreSQL 是大流量生产推荐后端。
- 所有表使用显式 migration。
- SQLite 和 PostgreSQL 分别维护 migration，但业务模型保持一致。
- 不兼容 Nezha 数据库结构。
- 软删除只用于需要审计恢复的资源；普通配置删除直接硬删并保留 audit log。
- Agent 高频指标不得直接写入 SQL 元数据层，必须进入 TSDB 或外部指标后端。

## 数据库后端

支持后端：

- `sqlite`：默认本地开发、小规模部署、低运维成本。
- `postgres`：生产推荐，面向多 Agent、大量任务、审计和服务探测写入压力。

连接配置：

- `DATABASE_URL=sqlite:///data/xlstatus.db`
- `DATABASE_URL=postgres://xlstatus:password@postgres:5432/xlstatus`

实现要求：

- 使用 SQLx 编译期或离线校验 SQL。
- repository 层隐藏 `SqlitePool` 和 `PgPool` 差异。
- 所有查询显式分页，禁止无限制列表。
- 所有时间字段统一存 UTC。
- 所有多租户查询必须带 `owner_user_id` 或权限过滤条件。

PostgreSQL 高 IO 要求：

- `service_results`、`task_runs`、`audit_logs`、`transfers` 支持按时间分区。
- 高频插入使用批量写入，默认 100 条或 1 秒 flush。
- 关键查询索引覆盖最近时间窗口。
- 保留策略按表配置，历史数据可归档到对象存储或冷表。
- 连接池默认按 CPU 和部署规模配置，必须暴露 pool wait、active、idle 指标。

可扩展后端：

- 关系型元数据层后续可评估 MySQL/MariaDB，但第一版只承诺 SQLite 和 PostgreSQL。
- 指标层预留外部后端接口，可接入 VictoriaMetrics、ClickHouse 或 TimescaleDB。

## 核心表

`users`：

- `id`
- `username`
- `password_hash`
- `role`
- `agent_secret_hash`
- `reject_password`
- `token_version`
- `created_at`
- `updated_at`

`sessions`：

- `id`
- `user_id`
- `token_hash`
- `ip`
- `user_agent`
- `expires_at`
- `created_at`

`api_tokens`：

- `id`
- `user_id`
- `name`
- `token_hash`
- `scopes_json`
- `server_ids_json`
- `expires_at`
- `last_used_at`
- `last_used_ip`
- `created_at`
- `revoked_at`

`servers`：

- `id`
- `owner_user_id`
- `name`
- `agent_uuid`
- `agent_secret_hash`
- `note`
- `public_note`
- `display_index`
- `hide_for_guest`
- `enable_ddns`
- `ddns_profile_ids_json`
- `override_ddns_domains_json`
- `created_at`
- `updated_at`

`server_groups`：

- `id`
- `owner_user_id`
- `name`
- `created_at`
- `updated_at`

`server_group_members`：

- `group_id`
- `server_id`

`services`：

- `id`
- `owner_user_id`
- `name`
- `kind`
- `target`
- `duration_seconds`
- `display_index`
- `notify`
- `notification_group_id`
- `cover_mode`
- `server_selector_json`
- `hide_for_guest`
- `latency_min_ms`
- `latency_max_ms`
- `latency_notify`
- `enable_trigger_task`
- `fail_task_ids_json`
- `recover_task_ids_json`

`service_results`：

- `id`
- `service_id`
- `server_id`
- `status`
- `delay_ms`
- `error`
- `cert_fingerprint`
- `cert_not_after`
- `created_at`

PostgreSQL 分区：

- 按 `created_at` 月分区，超大规模部署可切换日分区。
- 索引：`(service_id, created_at desc)`、`(server_id, created_at desc)`、`(status, created_at desc)`。

`alert_rules`：

- `id`
- `owner_user_id`
- `name`
- `enabled`
- `trigger_mode`
- `notification_group_id`
- `rules_json`
- `fail_task_ids_json`
- `recover_task_ids_json`

`tasks`：

- `id`
- `owner_user_id`
- `name`
- `task_type`
- `schedule`
- `command`
- `cover_mode`
- `server_selector_json`
- `push_successful`
- `notification_group_id`
- `last_executed_at`
- `last_result`

`task_runs`：

- `id`
- `task_id`
- `server_id`
- `status`
- `delay_ms`
- `output`
- `output_truncated`
- `created_at`

PostgreSQL 分区：

- 按 `created_at` 月分区。
- 索引：`(task_id, created_at desc)`、`(server_id, created_at desc)`、`(status, created_at desc)`。

`notifications`：

- `id`
- `owner_user_id`
- `name`
- `url`
- `request_method`
- `request_type`
- `headers_json`
- `body_template`
- `verify_tls`
- `format_metric_units`

`notification_groups`：

- `id`
- `owner_user_id`
- `name`

`notification_group_members`：

- `group_id`
- `notification_id`

`ddns_profiles`：

- `id`
- `owner_user_id`
- `name`
- `provider`
- `enable_ipv4`
- `enable_ipv6`
- `max_retries`
- `domains_json`
- `credentials_encrypted_json`
- `webhook_config_json`

`nat_tunnels`：

- `id`
- `owner_user_id`
- `enabled`
- `name`
- `server_id`
- `domain`
- `target_host`

`transfers`：

- `id`
- `owner_user_id`
- `server_id`
- `op`
- `path`
- `size`
- `status`
- `error`
- `created_at`
- `completed_at`

PostgreSQL 分区：

- 按 `created_at` 月分区。
- 索引：`(server_id, created_at desc)`、`(status, created_at desc)`。

`audit_logs`：

- `id`
- `user_id`
- `api_token_id`
- `action`
- `resource_type`
- `resource_id`
- `server_id`
- `ip`
- `outcome`
- `metadata_json`
- `sensitive_hash`
- `created_at`

PostgreSQL 分区：

- 按 `created_at` 月分区。
- 索引：`(user_id, created_at desc)`、`(api_token_id, created_at desc)`、`(resource_type, resource_id, created_at desc)`、`(server_id, created_at desc)`。

`settings`：

- `key`
- `value_json`
- `updated_at`

## 内存态模型

`AgentRegistry`：

- `server_id -> AgentConnection`
- 保存 task stream、io stream registry、last_active、host_info、state_snapshot。

`ServerSnapshot`：

- 不直接每次从 DB 读取。
- Agent 状态上报后写入内存、广播、批量写 TSDB。

`InflightTaskRegistry`：

- `task_run_id -> oneshot sender`
- 用于 MCP exec、fs 操作和普通任务等待回包。

## TSDB 指标

服务器指标：

- `server_cpu`
- `server_memory_used`
- `server_swap_used`
- `server_disk_used`
- `server_net_in_speed`
- `server_net_out_speed`
- `server_net_in_transfer`
- `server_net_out_transfer`
- `server_load1`
- `server_load5`
- `server_load15`
- `server_tcp_conn`
- `server_udp_conn`
- `server_process_count`
- `server_temperature_max`
- `server_uptime`
- `server_gpu_max`

服务指标：

- `service_delay`
- `service_status`

查询周期：

- `1d`：30 秒降采样。
- `7d`：30 分钟降采样。
- `30d`：2 小时降采样。

外部指标后端预留：

- 定义 `MetricStore` trait：`write_server_metrics`、`write_service_metrics`、`query_server_metric`、`query_service_history`、`compact`、`health`.
- 默认实现为本地嵌入式 TSDB。
- 可选实现允许接入 VictoriaMetrics、ClickHouse 或 TimescaleDB。
- 外部后端只影响指标历史，不影响 SQL 元数据结构。

验收标准：

- 服务历史可以按 service_id 查询每个 server 的可用性。
- 服务器指标可以按 server_id、metric、period 查询。

## 密钥存储

- 密码：Argon2id。
- PAT：只存 SHA-256 或更强哈希，明文只在创建时展示一次。
- Agent secret：存 hash；配置下发时生成新 secret。
- DDNS 和通知凭证：使用 master key 加密后存储。
- master key：优先环境变量，缺省初始化生成本地 sealed 文件。
