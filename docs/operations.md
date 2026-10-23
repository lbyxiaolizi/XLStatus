# 日常运维手册 (Day-2 Operations)

本文档面向生产环境管理员，涵盖 **XLStatus `v0.2`** 的健康检查、服务生命周期管理、SQLite 在线热备份与加固恢复、SQLite 到 PostgreSQL 原子迁移、以及 Agent 节点集群与安全运营（远程配置下发、强制升级、会话/PAT 管控、WAF 封禁与审计日志）。

---

## 1. 健康检查、服务生命周期与日志巡检

### 1.1 健康检查 (`/healthz` 与维护状态)

- **探针级存活检查（无需认证，适用于负载均衡器 / Kubernetes / Docker Healthcheck）**：
  ```bash
  curl -fsS http://127.0.0.1:8080/healthz
  # 正常输出: OK (HTTP 200)
  ```
- **管理级维护状态检查（需管理员 Cookie 会话）**：
  ```bash
  curl -fsS -b cookies.txt http://127.0.0.1:8080/api/v1/maintenance/status
  ```
  返回当前数据库后端（`sqlite` / `postgres`）、在线备份/恢复支持状态、时序库（TSDB）健康度、当前内存样本数 `tsdb_samples` 及保留天数 `tsdb_retention_days`。

### 1.2 systemd 服务生命周期管理 (`systemctl`)

```bash
# Server 控制面服务
sudo systemctl status xlstatus --no-pager
sudo systemctl restart xlstatus
sudo systemctl stop xlstatus
sudo systemctl start xlstatus

# Agent 节点服务
sudo systemctl status xlstatus-agent --no-pager
sudo systemctl restart xlstatus-agent
```

### 1.3 日志查看与动态调试 (`journalctl`)

```bash
# 查看 Server 最近 100 行日志并持续追踪
sudo journalctl -u xlstatus -n 100 -f

# 查看 Agent 最近 100 行日志
sudo journalctl -u xlstatus-agent -n 100 --no-pager

# 临时以 debug 级别在前台复现并排查问题
sudo -u xlstatus RUST_LOG=debug CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
```

### 1.4 端口监听检查与前台冒烟测试

默认端口：HTTP API `8080`、Agent gRPC `50051`、Web UI `3000`。

```bash
sudo ss -tlnp | grep -E ':(8080|50051|3000)\b' || true
```

在远端 Linux 主机执行 8 秒前台冒烟验证：

```bash
timeout 8s sudo -u xlstatus CONFIG_FILE=/etc/xlstatus/server.toml /usr/local/bin/xlstatus-server
echo $?
```

- 退出码 `124`：服务持续稳定运行至 `timeout` 结束（冒烟通过）。
- 退出码 `1`：启动失败，检查终端打印的 `Error:`（如端口被占用或密钥熵不足）。

---

## 2. SQLite 在线热备份与加固恢复

XLStatus 在 **系统设置 (`/settings`) -> 维护与备份** 页面及 `/api/v1/maintenance/*` 接口中内置了零停机 SQLite 在线备份、全量归档导出、空间回收（`VACUUM`）、TSDB 压缩与安全沙箱恢复能力。

> **认证与二次验证要求**：
> 所有 `/api/v1/maintenance/*` 接口均**强制要求管理员 Cookie 会话**（拒绝 PAT 令牌访问，防止 API 令牌泄露导致整库被拖库或覆盖）。若当前管理员账号已启用 TOTP 2FA，执行备份下载、归档导出、实际恢复（`dry_run=false`）、`VACUUM` 与 TSDB 变更时必须在请求头携带 `X-TOTP-Code: <6位动态码>`。

### 2.1 在线热备份 (`/api/v1/maintenance/backup` & `/archive`)

无需停止 `xlstatus-server`，服务端通过私有 `0700` 临时目录执行 SQLite `VACUUM INTO` 生成一致性快照，所有下载响应均附带 `Cache-Control: no-store`：

1. **仅导出 SQLite 数据库快照 (`GET /api/v1/maintenance/backup`)**：
   ```bash
   curl -fsSL -b cookies.txt \
     -H "X-TOTP-Code: 123456" \
     http://127.0.0.1:8080/api/v1/maintenance/backup \
     -o "xlstatus-backup-$(date -u +%Y%m%d%H%M%S).sqlite3"
   ```
2. **导出全量 ZIP 归档 (`GET /api/v1/maintenance/archive`)**：
   包含 `manifest.json`（生成时间、数据库/TSDB 状态）、`database.sqlite3`（完整 SQLite 快照）以及 `tsdb_samples.json`（内存时序样本导出）：
   ```bash
   curl -fsSL -b cookies.txt \
     -H "X-TOTP-Code: 123456" \
     http://127.0.0.1:8080/api/v1/maintenance/archive \
     -o "xlstatus-archive-$(date -u +%Y%m%d%H%M%S).zip"
   ```
3. **命令行离线/热备（宿主机直接操作）**：
   ```bash
   sudo sqlite3 /var/lib/xlstatus/xlstatus.db ".backup '/var/backups/xlstatus-$(date -u +%Y%m%d%H%M%S).sqlite3'"
   ```

> **重要提醒**：备份数据库的同时，务必离线备份 `/etc/xlstatus/server.toml`（或 `.env`）中的 `SECRET_ENCRYPTION_KEY` 与 `SESSION_SECRET`。若丢失匹配的 `SECRET_ENCRYPTION_KEY`，备份库中的加密凭据（DDNS Token、Cloudflared Token、GeoIP Token、TOTP 密钥）将无法解密。

### 2.2 加固版 SQLite 在线恢复 (`/api/v1/maintenance/restore`)

针对上传的 SQLite 备份文件，`POST /api/v1/maintenance/restore` 实施了多层防御校验，严防恶意构造的 SQLite 文件触发任意代码执行或破坏Schema：

1. **文件头与体积校验**：
   - 请求体上限为 `512 MiB` (`RESTORE_MAX_BYTES`)。
   - 强制校验文件长度 `>= 100` 字节，且前 16 字节必须精确匹配 SQLite 3 魔数头 `SQLite format 3\0`。
2. **只读沙箱与 `PRAGMA trusted_schema = OFF` 预检**：
   - 备份文件先写入权限为 `0700` 的私有临时目录，并以 `read_only = true` + `PRAGMA trusted_schema = OFF` 打开。
   - 执行 `PRAGMA integrity_check`，任何非 `"ok"` 结果立即拒绝。
3. **恶意数据库对象拦截 (`ensure_no_untrusted_sqlite_objects`)**：
   - 扫描 `sqlite_master`，**仅允许普通 `table` 与 `index` 对象**。
   - 任何包含 `trigger`（触发器）、`view`（视图）或 `CREATE VIRTUAL TABLE`（虚拟表/FTS/自定义扩展）的备份文件都会被立即拦截并报错，杜绝在查询或恢复期间触发恶意 SQL 副作用。
4. **Schema 精确兼容性核验**：
   - 逐表比对当前运行数据库与备份数据库的表名及列顺序（`PRAGMA table_info`），缺失表或列定义不一致均拒绝恢复。
5. **事务级原子数据替换与外键核验**：
   - 预检通过后，若 `dry_run=true` 则仅返回兼容性报告（`user_version`、`table_count`、`row_count`）而不修改数据。
   - 若 `dry_run=false`（实际恢复），服务端在 `PRAGMA trusted_schema = OFF` 下开启 `BEGIN IMMEDIATE` 独占事务，按逆序清空当前表并从 `ATTACH` 的备份库逐表复制列数据，最后执行 `PRAGMA foreign_key_check` 确认无任何外键违规后才执行 `COMMIT`；任意步骤失败立即 `ROLLBACK`。

**执行预检 (`dry_run=true`) 与实际恢复示例**：

```bash
# 1. 先执行 dry_run 预检（不修改线上库）
curl -fsS -b cookies.txt \
  -H "X-CSRF-Token: $CSRF_TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @xlstatus-backup.sqlite3 \
  "http://127.0.0.1:8080/api/v1/maintenance/restore?dry_run=true"

# 2. 预检返回 compatible=true 后，带上 TOTP 验证码执行正式恢复
curl -fsS -b cookies.txt \
  -H "X-CSRF-Token: $CSRF_TOKEN" \
  -H "X-TOTP-Code: 123456" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @xlstatus-backup.sqlite3 \
  "http://127.0.0.1:8080/api/v1/maintenance/restore?dry_run=false"
```

### 2.3 SQLite `VACUUM` 与 TSDB 在线维护

```bash
# 执行 SQLite VACUUM 回收碎片空间
curl -fsS -X POST -b cookies.txt \
  -H "X-CSRF-Token: $CSRF_TOKEN" -H "X-TOTP-Code: 123456" \
  http://127.0.0.1:8080/api/v1/maintenance/sqlite/vacuum

# 手动触发 TSDB 过期样本压缩
curl -fsS -X POST -b cookies.txt \
  -H "X-CSRF-Token: $CSRF_TOKEN" -H "X-TOTP-Code: 123456" \
  http://127.0.0.1:8080/api/v1/maintenance/tsdb/compact

# 在线调整 TSDB 样本保留天数（1..=3650 天）
curl -fsS -X POST -b cookies.txt \
  -H "X-CSRF-Token: $CSRF_TOKEN" -H "X-TOTP-Code: 123456" \
  -H "Content-Type: application/json" \
  -d '{"retention_days": 30}' \
  http://127.0.0.1:8080/api/v1/maintenance/tsdb/retention
```

---

## SQLite 到 PostgreSQL 迁移

当节点规模或高频探测/任务日志增长需要从 SQLite 平滑升级至 PostgreSQL 时，使用仓库内置的 [`scripts/import_sqlite_to_postgres.py`](../scripts/import_sqlite_to_postgres.py) 工具完成原子迁移。

### 迁移安全特性与设计保障

1. **严格的表名与标识符白名单 (`XLSTATUS_TABLES`)**：
   - 脚本内置 34 张 XLStatus 官方表白名单（如 `users`、`agents`、`servers`、`services`、`tasks`、`nat_mappings`、`audit_logs` 等），并对所有表名与列名强制执行正则校验 (`^[A-Za-z_][A-Za-z0-9_]{0,62}$`)。
   - 目标 PostgreSQL `public` schema 中任何不属于白名单的第三方表或扩展表**绝不会被加锁、检查、读取或清空**。
2. **外键拓扑排序与类型自动转换**：
   - 自动查询 PostgreSQL `information_schema` 中的外键约束图，按父表优先的拓扑顺序生成导入序列。
   - 自动将 SQLite 的 `0/1` 转换为 PostgreSQL `boolean`，将可空 UUID / 时间戳列中的空字符串 `""` 转为 `NULL` (`\N`)。
3. **单一事务独占锁、安全 `--truncate` 与逐表行数核验**：
   - 整个导入过程在单个 `psql` 事务 (`BEGIN; ... COMMIT;`) 中完成：首先对白名单表获取 `LOCK TABLE ... IN ACCESS EXCLUSIVE MODE`。
   - 默认要求目标表为空；传入 `--truncate` 时执行 `TRUNCATE ... RESTART IDENTITY`（**故意省略 `CASCADE`**，若存在未知外部表通过外键引用 XLStatus 表，PostgreSQL 会直接拒绝并回滚，绝不级联误删未知表数据）。
   - 以 `COPY_BATCH_SIZE = 1000` 流式批次写入 `\copy ... FROM STDIN`，不在内存中缓存整表，并在 `COMMIT` 前对每张导入表执行 `DO $xlstatus_count$ ... SELECT COUNT(*)` 行数核对。任何一步失败或管道中断都会自动触发整库 `ROLLBACK`（包括回滚 `TRUNCATE`）。
4. **错误输出自动脱敏 (`redact_sensitive_text`)**：
   - 子进程异常退出时，自动擦除 stderr/stdout 与命令参数中的数据库密码、`PGPASSWORD`、`sslpassword` 等凭据。

### 标准迁移步骤

```bash
# 1. 停止当前 SQLite 实例写入并创建一致性备份
docker compose stop web server
mkdir -p ./backups/pre-pg
sqlite3 ./data/xlstatus.db ".backup './backups/pre-pg/xlstatus.sqlite3'"

# 2. 启动 PostgreSQL 容器（确保 .env 中 SESSION_SECRET 和 SECRET_ENCRYPTION_KEY 与原实例完全一致！）
docker compose -f docker-compose.pg.yml up -d postgres

# 3. 临时启动一次 Server，让内置迁移在 PostgreSQL 中创建最新表结构，随后停止 Server
docker compose -f docker-compose.pg.yml up -d server
curl -fsS http://127.0.0.1:8080/healthz
docker compose -f docker-compose.pg.yml stop server

# 4. 执行原子导入（因为第 3 步可能创建了初始 seed admin，使用 --truncate 替换为 SQLite 原有数据）
python3 scripts/import_sqlite_to_postgres.py \
  ./backups/pre-pg/xlstatus.sqlite3 \
  --container xlstatus-postgres \
  --user xlstatus \
  --database xlstatus \
  --truncate

# 5. 重新启动 PostgreSQL 编排下的 Server 与 Web 并验证
docker compose -f docker-compose.pg.yml up -d server web
curl -fsS http://127.0.0.1:8080/healthz
```

---

## 3. Agent 集群日常运营与安全管控

### 3.1 远程下发 Agent 配置 (`/api/v1/servers/:id/config`)

无需 SSH 登录受管节点，管理员可在 **服务器详情页 (`/servers/[id]`) -> 运维配置** 或通过 API 实时向在线 Agent 推送配置更新（通过 gRPC `ServerMessage::ConfigUpdate` 下发并持久化到节点的 `agent.json`）：

- 可动态调整项：指标上报间隔 (`report_interval_secs`)、是否禁用远程命令执行 (`disable_command_execute`)、允许文件管理访问的根目录白名单 (`file_allowed_roots`) 等。
- **高危安全锁 (`disable_command_execute`)**：若在节点本地 `agent.json` 或通过远程配置开启了禁用命令执行，Agent 会拒绝后续 Shell 任务与终端请求。

### 3.2 远程强制升级 Agent (`force-update`)

在服务器详情页点击 **强制升级 Agent**（或调用 `POST /api/v1/servers/:id/force-update`）：

- 默认安全策略：**仅允许下载官方 GitHub Release 地址**（匹配当前架构与版本标签），拒绝任意第三方 URL，防止供应链投毒。
- 若在完全隔离的内网自建制品库中需要指定自定义 HTTPS 下载地址，需在 Server 端显式设置环境变量 `XLSTATUS_ALLOW_CUSTOM_FORCE_UPDATE_URL=1`。

### 3.3 注册令牌轮换与吊销失陷 Agent (`/api/v1/agents/:id/revoke`)

- **注册令牌 (`xle_...`) 管理**：在 **系统设置 -> Agent 接入** 中可设置过期时间与最大使用次数，随时删除未使用的令牌。
- **立即吊销 Agent**：当某台受管节点退役或疑似失陷时，在控制台点击 **吊销 Agent**（或调用 `POST /api/v1/agents/:id/revoke`）。
  - Server 立即在数据库中标记该 Agent 为已吊销，拒绝其后续 Ed25519 挑战签名与 JWT 刷新。
  - 同时通过当前活跃的 gRPC 双向流立即推送 `ServerMessage::ForceDisconnect`，令 Agent 进程主动断开并退出重连循环（由 `test-run/verify-m2-revoke.sh` 验证）。

### 3.4 会话管理与个人访问令牌 (PAT) 精细化授权

1. **在线会话管理 (`/api/v1/auth/sessions`)**：
   - 在 **系统设置 -> 账户与安全** 中可查看当前账号的所有活跃登录会话（含客户端 IP、User-Agent、创建时间与过期时间），支持一键吊销异常或旧设备的会话。
2. **个人访问令牌 (`xlp_...` PAT) 最小权限管控 (`/api/v1/tokens`)**：
   - 每个 PAT 均可绑定**细粒度权限范围 (`scopes`)**、**指定服务器白名单 (`server_ids`)** 与**过期时间**。
   - **MCP 读写隔离安全边界**：即使 PAT 拥有 `server:exec` 或 `transfer:write` 权限，通过 `/mcp` 或 `/api/v1/mcp/execute` 调用时，高危文件改动工具（`fs.write`、`fs.delete`、`fs.upload_url`）也会被服务端强制拦截（返回 `403 Forbidden`），仅允许在控制台管理员 Cookie 会话下操作。

### 3.5 WAF IP 封禁管理与安全审计日志

1. **WAF 自动与手动 IP 封禁 (`/api/v1/auth/waf/bans`)**：
   - 当某个客户端 IP 高频触发登录失败、TOTP 验证失败或恶意探测时，内置 WAF 会自动记录 `waf_events` 并封禁该 IP。
   - 管理员可在 **系统设置 -> 安全与 WAF** 中查看当前封禁列表、手动添加恶意 IP/网段封禁或提前解封误触的办公网 IP。
2. **全链路审计日志 (`/api/v1/audit-logs`)**：
   - 所有登录/登出、TOTP 启停、用户创建/删除、Agent 注册/吊销、远程命令执行、文件读写/传输、NAT 映射变更、数据库备份与恢复均会写入 `audit_logs` 表，支持在控制台按操作人、动作类型与时间范围检索。
