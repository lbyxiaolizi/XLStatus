# 后端规划

## 模块拆分

`xlstatus-server` 按业务域拆分：

- `api`：Axum router、extractor、response envelope、OpenAPI。
- `auth`：登录、Cookie session、CSRF、PAT、RBAC、scope。
- `agents`：Agent registry、gRPC 服务、连接状态、任务流、IO 流。
- `servers`：服务器资产、分组、实时状态、配置下发。
- `monitors`：服务监控定义、调度、结果处理、历史。
- `alerts`：规则评估、状态机、静默、恢复。
- `tasks`：Cron、触发任务、手动执行、结果聚合。
- `notifications`：通知渠道、通知组、模板渲染、发送。
- `ddns`：Provider 抽象、IP 变化处理、重试。
- `nat`：域名匹配、反代隧道、保留 host。
- `mcp`：JSON-RPC、tool registry、限流、临时文件 URL。
- `tsdb`：指标写入和查询 facade。
- `audit`：审计事件和敏感字段脱敏。
- `settings`：系统配置、运行时开关。
- `storage`：数据库后端抽象、连接池、事务封装、SQLite/PostgreSQL migration 分发。

## REST API

目标：

- API 按资源设计，不兼容 Nezha，但保持易理解。

主要接口组：

- `POST /api/auth/login`
- `POST /api/auth/logout`
- `POST /api/auth/refresh`
- `GET /api/profile`
- `PATCH /api/profile`
- `GET /api/servers`
- `PATCH /api/servers/{id}`
- `POST /api/servers/{id}/config`
- `POST /api/servers/{id}/force-update`
- `GET /api/server-groups`
- `POST /api/server-groups`
- `GET /api/services`
- `POST /api/services`
- `GET /api/services/{id}/history`
- `GET /api/servers/{id}/metrics`
- `GET /api/alert-rules`
- `POST /api/alert-rules`
- `GET /api/tasks`
- `POST /api/tasks`
- `POST /api/tasks/{id}/run`
- `GET /api/notifications`
- `GET /api/notification-groups`
- `GET /api/ddns`
- `GET /api/nat`
- `GET /api/api-tokens`
- `POST /api/api-tokens`
- `GET /api/audit-logs`
- `GET /api/settings`
- `PATCH /api/settings`

数据流：

1. extractor 解析 session 或 PAT。
2. middleware 执行 RBAC 和 scope。
3. handler 调用 service 层。
4. service 层开启 transaction，写数据库和内存 registry。
5. response 使用统一 envelope：`{ ok, data, error, request_id }`。

失败场景：

- 参数错误返回 `400 validation_error`。
- 未登录返回 `401 unauthorized`。
- scope 不足返回 `403 forbidden`。
- 资源不存在返回 `404 not_found`。
- worker 队列满返回 `503 queue_full`。

验收标准：

- OpenAPI 能覆盖所有 REST endpoint。
- 每个写接口都有权限测试和审计事件。
- 同一套 service 层测试必须能在 SQLite 和 PostgreSQL 上通过。

## 数据库访问

目标：

- 在不改业务代码的前提下支持 SQLite 和 PostgreSQL。
- 大流量生产环境优先使用 PostgreSQL，避免 SQLite 单写瓶颈。

实现要求：

- 启动时根据 `DATABASE_URL` 选择 `DatabaseBackend::Sqlite` 或 `DatabaseBackend::Postgres`。
- 建立统一 `Db` facade，内部持有对应连接池。
- repository 层提供用户、服务器、监控、任务、通知、审计等接口。
- 事务通过统一 `Tx` wrapper 暴露，禁止业务层拼接后端专属 SQL。
- 后端差异只允许出现在 repository/migration 层。

高 IO 策略：

- PostgreSQL 下为 `service_results`、`task_runs`、`audit_logs` 使用按月或按日分区。
- 高频插入走批量 writer，worker 聚合后批量提交。
- 查询接口必须命中 `(tenant_id, created_at)`、`(server_id, created_at)`、`(service_id, created_at)` 组合索引。
- 长历史明细由 retention worker 清理或归档，Dashboard 默认查询聚合数据。

失败场景：

- PostgreSQL 连接池耗尽时返回 `503 db_pool_exhausted` 并记录指标。
- migration 版本不匹配时拒绝启动。
- 批量 writer flush 失败时重试有限次数，超过后进入 degraded 并保留内存告警。

验收标准：

- SQLite 和 PostgreSQL 都能完成初始化、登录、Agent 接入和核心 CRUD。
- PostgreSQL 压测下审计和任务结果写入不会阻塞 Agent 状态上报。

## WebSocket

目标：

- 提供低延迟服务器状态、终端、文件管理和传输进度。

接口：

- `/ws/servers`：推送服务器实时状态和在线统计。
- `/ws/terminal/{session_id}`：浏览器与 Agent PTY 双向流。
- `/ws/file/{session_id}`：浏览器文件管理会话。
- `/ws/transfers`：文件传输状态。

数据流：

- Server 将 Agent 状态写入 `watch` 或 `broadcast` channel。
- WebSocket connection 按用户权限过滤后发送。
- 终端和文件会话通过 session registry 绑定 Agent IO stream。

失败场景：

- Agent 离线：关闭终端和文件会话。
- 浏览器断开：取消 Agent 侧 session。
- 权限被撤销：主动关闭连接。

验收标准：

- 服务器状态流不会向用户发送无权服务器。
- 终端 resize、输入、输出都可用。

## Agent RPC

目标：

- 建立 Dashboard 和 Agent 的稳定双向控制通道。

protobuf 服务：

- `RegisterHost(HostInfo) returns (RegisterReceipt)`
- `StreamState(stream HostState) returns (stream StateReceipt)`
- `TaskStream(stream TaskResult) returns (stream Task)`
- `IoStream(stream IoFrame) returns (stream IoFrame)`
- `ReportGeoIp(GeoIpReport) returns (GeoIpReceipt)`

失败场景：

- secret 错误：拒绝连接并记录来源 IP。
- 同一 Agent 重复连接：新连接替换旧连接，旧任务流安全关闭。
- TaskResult spoof：结果必须匹配当前 Agent、task_id、inflight registry。

验收标准：

- 所有对同一 gRPC stream 的 send 都串行化。
- Agent 重连不会丢失待下发的配置任务。

## 调度器和 Worker

目标：

- 使用统一调度框架处理服务监控、任务、告警、维护。

worker：

- `service_monitor_worker`
- `alert_evaluator_worker`
- `task_scheduler_worker`
- `notification_worker`
- `ddns_worker`
- `tsdb_flush_worker`
- `maintenance_worker`

失败场景：

- 单个 job panic 不影响 worker loop。
- worker queue 满时产生 degraded 事件。
- 通知和 DDNS 使用有限重试，不无限堆积。

验收标准：

- 可以通过健康检查看到各 worker 最近成功时间和队列长度。
