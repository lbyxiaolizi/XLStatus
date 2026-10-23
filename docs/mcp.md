# AI-Native 集群运维与 MCP `2024-11-05` 权威指南

**XLStatus** 原生内建符合 **Model Context Protocol (MCP) `2024-11-05`** 规范的 AI 集群运维引擎（`crates/server/src/mcp/`）。无需额外部署第三方抓取插件，AI 智能体（Antigravity、Cursor、Windsurf、Claude Desktop、Claude Code、Cline、OpenCode 等）即可在严格的 **RBAC 最小权限**与 **PAT 节点白名单（`server_ids`）隔离**下，直接完成全集群健康诊断、节点模糊解析、时序遥测分析、只读 SRE 剧本巡检、批量远程命令编排与只读文件审计。

---

## 1. 协议架构与三种传输模式 (Protocol & Transport Modes)

- **协议版本**：`2024-11-05`（JSON-RPC `2.0`）
- **服务端标识**：`XLStatus MCP Server`
- **声明能力 (`capabilities`)**：同时支持 `tools`（19 项）、`resources`（5 项静态资源 + 1 项动态资源模板）与 `prompts`（4 项动态注入实时数据的 SRE 提示词）。

```mermaid
flowchart LR
    subgraph Clients["AI 客户端与运维入口"]
        HTTPClient["Streamable HTTP 客户端\n(Claude Code / 自定义 Agent)"]
        StdioClient["Stdio 客户端\n(Claude Desktop / Cursor / Windsurf / Antigravity)"]
        BrowserUI["浏览器运维工作台\n(Web AI Ops Studio /mcp)"]
    end

    subgraph Bridge["本地桥接层"]
        StdioScript["scripts/xlstatus-mcp-stdio.mjs\n1MiB 行缓冲保护 + xlp_/xle_ 自动脱敏"]
    end

    subgraph Server["xlstatus-server (:8080)"]
        RpcEndpoint["POST /mcp\nJSON-RPC 2.0 (单条或 <=16 批量, <=1MiB)"]
        StudioApi["GET /api/v1/mcp/catalog\nPOST /api/v1/mcp/studio"]
        RestCompat["GET /api/v1/mcp/info\nGET /api/v1/mcp/tools\nPOST /api/v1/mcp/execute"]
        Executor["McpExecutor\nRBAC Scope + PAT server_ids 隔离 + TOTP 门禁"]
    end

    HTTPClient -->|"Authorization: Bearer xlp_..."| RpcEndpoint
    StdioClient <-->|"stdin / stdout JSON-RPC"| StdioScript
    StdioScript -->|"POST /mcp (Bearer xlp_...)"| RpcEndpoint
    BrowserUI -->|"Cookie + CSRF (+ x-totp-code)"| StudioApi
    HTTPClient -.->|"脚本轻量调用"| RestCompat

    RpcEndpoint --> Executor
    StudioApi --> Executor
    RestCompat --> Executor
```

### 模式一：Direct Streamable HTTP (`POST /mcp`)
- **适用场景**：原生支持 HTTP JSON-RPC 传输的 MCP 客户端、自动化运维流水线或 `curl` 脚本。
- **认证方式**：必须携带 `Authorization: Bearer xlp_<token>`（仅限 Personal Access Token）。
- **协议特性**：
  - 支持标准 JSON-RPC 2.0 方法：`initialize`、`notifications/initialized`、`ping`、`tools/list`、`tools/call`、`resources/list`、`resources/templates/list`、`resources/read`、`prompts/list`、`prompts/get`，以及直接将工具名作为 `method` 调用的快捷模式。
  - 支持 JSON-RPC 数组批处理（单次最多 `16` 个请求对象，空数组或超限返回 `-32600`）。
  - 请求体最大限制为 **`1 MiB` (`1,048,576` bytes)**。
- **辅助 REST 兼容端点**（同样要求 `Bearer xlp_...`）：
  - `GET /api/v1/mcp/info`：获取 MCP Server 版本与能力计数摘要。
  - `GET /api/v1/mcp/tools`：获取 19 项工具的 JSON Schema 定义列表。
  - `POST /api/v1/mcp/execute`：以 `{"tool": "...", "arguments": {...}}` 格式直接调用工具。

### 模式二：Web AI Ops Studio (`GET /api/v1/mcp/catalog` & `POST /api/v1/mcp/studio`)
- **适用场景**：Web 控制台内置的交互式 **MCP AI Ops Studio**（路径 `/mcp`，对应源码 `web/app/(dashboard)/mcp/page.tsx`）。
- **认证方式**：同时兼容浏览器 **Cookie Session（需 CSRF Header）** 与 **Bearer PAT**。
- **交互能力**：
  - `GET /api/v1/mcp/catalog`：一次性拉取全量 `tools`、`resources`、`resource_templates` 与 `prompts` 元数据。
  - `POST /api/v1/mcp/studio`：通过 `{"kind": "tool" | "resource" | "prompt", ...}` 交互式调试任意工具、读取 `xlstatus://` 资源或预览渲染后的 SRE 提示词，并返回精确的 `duration_ms` 耗时。
  - **Cookie 会话 TOTP 二次验证保护**：当通过浏览器 Cookie Session 调用高危工具 `server.exec` 或 `server.batch_exec` 且当前管理员已启用 TOTP 时，后端强制校验请求头 `x-totp-code`。

### 模式三：Stdio 桥接器 (`scripts/xlstatus-mcp-stdio.mjs`)
- **适用场景**：仅支持 `stdio` 标准输入输出传输的桌面端/IDE 客户端（Claude Desktop、Cursor、Windsurf、Antigravity 等）。
- **核心安全特性**：
  - **零外部 npm 依赖**：纯 Node.js（`>= 18`）标准库实现，直接通过内置 `fetch` 转发至 `/mcp`。
  - **内存防爆保护 (`createBoundedLineReader`)**：强制执行单行最大 **`1 MiB` (`MAX_LINE_BYTES = 1048576`)** 限制，遇到无换行符的超长流时立即丢弃并返回 JSON-RPC `-32600 Invalid Request`，防止宿主内存耗尽。
  - **敏感令牌自动脱敏 (`redactSensitiveTokens`)**：所有写往 `stderr` 或错误响应的异常文本均会自动将 `xlp_...`、`xle_...` 及当前配置的 Token 替换为 `[REDACTED]`。
  - **JSON-RPC Notification 静默处理**：对不包含 `id` 字段的通知帧（如 `notifications/initialized`）转发后不向 `stdout` 输出多余响应，严格遵循 MCP stdio 握手规范。

---

## 2. 19 项 MCP Tools 完整参考表 (Complete Reference of 19 MCP Tools)

所有工具均在 `crates/server/src/mcp/tools.rs` 中声明了精确的 JSON Schema 与 MCP 行为注解（`readOnlyHint`、`destructiveHint`、`idempotentHint`、`openWorldHint`）。

### 2.1 元信息与集群诊断 (Meta & Cluster Diagnostics)

| 工具名称 (`name`) | 所需 PAT Scope | 行为属性 | 输入参数 (`arguments`) | 功能与返回结构说明 |
|---|---|---|---|---|
| **`meta.whoami`** | 任意有效 PAT | 只读 / 幂等 | 无 (`{}`) | 返回当前认证用户 ID、用户名、角色（`admin`/`member`/`viewer`）、认证类型（`pat`/`session`）、已授权 `scopes`、`server_ids` 白名单及 MCP 协议版本。 |
| **`cluster.overview`** | `server:read` | 只读 / 幂等 | 无 (`{}`) | 返回可见集群全局汇总：`fleet`（在线/离线/已撤销节点数、可用率）、`resources`（全集群平均/峰值 CPU、总内存/磁盘使用率、总网络 RX/TX 吞吐）、`services`（健康/故障拨测数）、`alerts`（近期/触发中事件数）及按地区（`regions`）和云商（`providers`）的分布统计。 |
| **`cluster.diagnose`** | `server:read` | 只读 / 幂等 | • `cpu_threshold` (`number`, 默认 `85.0`, 范围 `10..100`)<br>• `memory_threshold` (`number`, 默认 `85.0`, 范围 `10..100`)<br>• `disk_threshold` (`number`, 默认 `85.0`, 范围 `10..100`) | **AI SRE 自动异常扫描器**。遍历所有可见节点与服务拨测，检测节点离线、CPU/内存/磁盘超阈值、7 天内到期节点及失败的服务探针；计算集群健康分（`0–100`）与状态（`healthy`/`degraded`/`critical`），按 `critical` → `warning` 排序输出异常列表，并给出每项异常的 `suggested_tool` 与 `suggested_arguments`。 |

### 2.2 服务器资产与时序遥测 (Server Inventory & Telemetry)

| 工具名称 (`name`) | 所需 PAT Scope | 行为属性 | 输入参数 (`arguments`) | 功能与返回结构说明 |
|---|---|---|---|---|
| **`server.list`** | `server:read` | 只读 / 幂等 | • `status` (`"all"` \| `"online"` \| `"offline"`, 默认 `"all"`)<br>• `search` (`string`, 可选，匹配名称/地区/国家/云商/标签)<br>• `limit` (`integer`, 默认 `50`, 范围 `1..200`)<br>• `offset` (`integer`, 默认 `0`) | 列出当前令牌可见的服务器列表，包含实时 CPU、内存、Swap、磁盘、Load (`1/5/15`)、网络速率/总流量、进程数、TCP/UDP 连接数、硬件规格与地理标签。 |
| **`server.get`** | `server:read` | 只读 / 幂等 | • `server_id` (`string`, **必填**，36 位 canonical UUID) | 获取单台可见服务器的完整硬件规格（CPU 型号/核心数、OS、内核、架构、虚拟化）、实时遥测快照、账单到期时间（`expires_at`）及最后更新时间。 |
| **`server.resolve`** | `server:read` | 只读 / 幂等 | • `query` (`string`, **必填**，支持服务器名称、UUID 前缀、地区、国家、云商或标签) | **智能模糊解析器**。对可见节点按精确匹配（`100` 分）、前缀匹配（`85` 分）、名称子串（`70` 分）、元数据匹配（`50` 分）打分排序，返回 `best_match` 与候选列表 `matches`，便于 AI 将自然语言主机名转换为标准 `server_id`。 |
| **`server.metrics`** | `server:read` | 只读 / 幂等 | • `server_id` (`string`, **必填**，canonical UUID)<br>• `range` (`"15m"` \| `"1h"` \| `"6h"` \| `"24h"`, 默认 `"1h"`) | 查询指定时间窗口内的 TSDB 时序指标（CPU、内存使用率、`load_1`），返回统计聚合值 `aggregates`（`min` / `max` / `avg` / `latest` / `samples`）以及最多 60 个均匀降采样序列点 `series`。 |

### 2.3 远程命令执行与 SRE 诊断剧本 (Remote Execution & Playbooks)

| 工具名称 (`name`) | 所需 PAT Scope | 行为属性 | 输入参数 (`arguments`) | 功能与返回结构说明 |
|---|---|---|---|---|
| **`server.inspect`** | `server:exec` | 受控执行 (`destructiveHint: false`) | • `server_id` (`string`, **必填**，canonical UUID)<br>• `playbook` (`string`, 默认 `"system_vital"`，枚举见第 3 节) | 在指定在线节点上执行预定义的**只读 SRE 诊断剧本**（固定 25 秒超时），返回剧本标题、实际执行的只读命令、`exit_code`、`stdout` 与 `stderr`。 |
| **`server.exec`** | `server:exec` | 高危执行 (`destructiveHint: true`) | • `server_id` (`string`, **必填**，canonical UUID)<br>• `command` (`string`, **必填**，`1..8192` 字节)<br>• `timeout` (`integer`, 默认 `30`，范围 `1..60` 秒) | 通过 Agent gRPC `Session` 流在单台在线节点上执行即席 Shell 命令（Unix 下运行于独立进程组，stdout/stderr 上限各 `64 KiB`），返回 `exit_code`、`stdout`、`stderr`、`status` 与 `duration_ms`。 |
| **`server.batch_exec`** | `server:exec` | 批量高危执行 (`destructiveHint: true`) | • `command` (`string`, **必填**，`1..8192` 字节)<br>• `target` (`"selected"` \| `"all_online"`, 默认 `"selected"`)<br>• `server_ids` (`string[]`, 当 `target="selected"` 时必填，最多 `10` 台 UUID)<br>• `timeout` (`integer`, 默认 `20`，范围 `1..60` 秒) | 跨多台在线节点（单次上限 **10 台**）批量分发 Shell 命令并汇总执行结果，返回 `targeted`、`succeeded`、`failed` 计数及每台节点的详细输出数组 `results`。 |

### 2.4 服务拨测 SLA、告警、自动化任务与网络拓扑 (Services, Alerts, Tasks & Network)

| 工具名称 (`name`) | 所需 PAT Scope | 行为属性 | 输入参数 (`arguments`) | 功能与返回结构说明 |
|---|---|---|---|---|
| **`service.list`** | `service:read` | 只读 / 幂等 | • `limit` (`integer`, 默认 `50`, 范围 `1..200`) | 列出 HTTP / TCP / ICMP 服务监控项及最新一次探测状态（`last_status`）、延迟（`last_delay_ms`）、HTTP 状态码与 TLS 证书过期时间（`cert_not_after`）。 |
| **`service.history`** | `service:read` | 只读 / 幂等 | • `service_id` (`string`, **必填**，服务 UUID)<br>• `limit` (`integer`, 默认 `30`, 范围 `1..200`) | 查询指定服务监控项的近期拨测历史，自动计算窗口内可用率 `uptime_percent`、延迟统计 `latency_ms`（`min`/`max`/`avg`/`latest`）及探测明细 `results`。 |
| **`alert.events`** | `alert:read` *(或 `server:read`)* | 只读 / 幂等 | • `kind` (`string`, 默认 `"all"`，可过滤 `"firing"`、`"resolved"` 等)<br>• `limit` (`integer`, 默认 `25`, 范围 `1..200`) | 列出当前可见范围内的历史告警事件流（含关联 `rule_id`、`agent_id`、`service_id`、事件载荷 `payload` 与触发时间 `fired_at`）。 |
| **`alert.rules`** | `alert:read` *(或 `server:read`)* | 只读 / 幂等 | • `limit` (`integer`, 默认 `50`, 范围 `1..200`) | 列出告警规则配置、触发模式（`once`/`always`）、条件数组 `conditions`、通知组 ID 以及绑定的故障自愈/恢复任务 ID（`failure_task_ids` / `recovery_task_ids`）。 |
| **`task.list`** | `task:read` | 只读 / 幂等 | • `limit` (`integer`, 默认 `25`, 范围 `1..100`) | 列出当前用户配置的定时 Cron 任务与自动化脚本，包含调度表达式 `schedule`、命令内容、覆盖模式及上次执行时间/结果。 |
| **`network.topology`** | `nat:read` *(或 `ddns:read` / `server:read`)* | 只读 / 幂等 | 无 (`{}`) | 汇总可见节点上的启用中 **NAT 反向隧道端口映射**（内网地址端口、公网端口、协议、允许源 IP）与 **DDNS 动态域名记录**（服务商、域名、当前 IP、最后同步时间）。 |

### 2.5 远端只读文件系统审计 (Filesystem Read-Only Tools)

| 工具名称 (`name`) | 所需 PAT Scope | 行为属性 | 输入参数 (`arguments`) | 功能与返回结构说明 |
|---|---|---|---|---|
| **`fs.list`** | `transfer:read` | 只读 / 幂等 | • `server_id` (`string`, **必填**，canonical UUID)<br>• `path` (`string`, **必填**，以 `/` 开头的绝对路径) | 列出在线服务器指定目录下的文件与子目录条目（名称、类型、字节大小、权限模式、修改时间、符号链接目标），结果上限 `1 MiB`。 |
| **`fs.read`** | `transfer:read` | 只读 / 幂等 | • `server_id` (`string`, **必填**，canonical UUID)<br>• `path` (`string`, **必填**，以 `/` 开头的绝对路径)<br>• `max_size` (`integer`, 默认 `1048576`，最大 `1 MiB`) | 读取远端服务器上的 UTF-8 文本文件内容，返回 `content`、读取字节数 `bytes` 及是否截断标志 `truncated`。 |
| **`fs.download_url`** | `transfer:read` | 只读 / 幂等 | • `server_id` (`string`, **必填**，canonical UUID)<br>• `path` (`string`, **必填**，以 `/` 开头的绝对路径)<br>• `expires_in` (`integer`, 运行时钳制为 `1..600` 秒，默认 `300` 秒) | 签发一次性临时文件下载令牌（`xlt_...`），返回可直接通过 `GET /api/v1/transfers/temp/download?token=...` 下载大文件（上限 `100 MiB`）的临时 URL。 |

---

## 3. 6 套内置 `server.inspect` SRE 只读诊断剧本 (Built-in Diagnostic Playbooks)

当 AI 需要深入排查单台节点内部状态时，优先推荐使用 `server.inspect` 而非自由拼接 `server.exec`。`server.inspect` 仅执行经过审计的固定只读 Shell 命令（超时锁定 25 秒），兼容主流 Linux 与 macOS/BSD 环境：

| Playbook 枚举值 | 诊断标题 (`title`) | 底层执行的只读命令 (`command`) | 典型适用场景 |
|---|---|---|---|
| **`system_vital`** *(默认)* | `System Vitals, Load & Memory Snapshot` | `uname -a; echo '=== UPTIME ==='; uptime; echo '=== MEMORY ==='; (free -h 2>/dev/null \|\| vm_stat 2>/dev/null \|\| true); echo '=== FILESYSTEMS ==='; df -h` | 快速摸底内核版本、系统运行时长、1/5/15 分钟负载、内存/Swap 余量与挂载点容量。 |
| **`top_processes`** | `Top CPU & Memory Processes` | `ps aux 2>/dev/null \| head -n 25 \|\| ps -ef \| head -n 25` | 排查 CPU 飙升或内存泄漏时，抓取当前资源占用最高的前 25 个进程。 |
| **`disk_and_inodes`** | `Disk Block & Inode Utilization` | `echo '=== BLOCK USAGE ==='; df -h; echo '=== INODE USAGE ==='; (df -i 2>/dev/null \|\| true)` | 排查磁盘空间告警或“有剩余空间但无法创建文件（Inode 耗尽）”故障。 |
| **`network_sockets`** | `Listening Network Sockets & Active Connections` | `(ss -tulnp 2>/dev/null \|\| netstat -an 2>/dev/null \| head -n 45 \|\| true)` | 审计节点当前监听的 TCP/UDP 端口、绑定进程及活跃外部连接。 |
| **`docker_containers`** | `Docker Containers & Runtime Status` | `(docker ps -a --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}' 2>/dev/null \|\| echo 'Docker daemon not installed or not accessible')` | 检查容器化工作负载的运行状态、重启次数与端口映射。 |
| **`system_logs`** | `Recent System & Kernel Logs` | `(journalctl -n 40 --no-pager 2>/dev/null \|\| tail -n 40 /var/log/syslog 2>/dev/null \|\| tail -n 40 /var/log/messages 2>/dev/null \|\| dmesg 2>/dev/null \| tail -n 40 \|\| echo 'No system logs accessible')` | 抓取最近 40 行 `systemd` 日志、系统日志或内核 `dmesg`（如排查 OOM Killer、硬件错误或服务崩溃）。 |

---

## 4. `xlstatus://` 上下文资源参考 (5 Static Resources + 1 Dynamic Template)

MCP 客户端可通过 `resources/list`、`resources/templates/list` 与 `resources/read` 直接订阅或挂载结构化 JSON 资源（`mimeType: "application/json"`）：

| 资源类型 | URI / URI Template | 名称 (`name`) | 所需 Scope | 返回内容说明 |
|---|---|---|---|---|
| **静态资源** | `xlstatus://cluster/overview` | `Cluster Health & Anomaly Overview` | `server:read` | 等同于 `cluster.overview`，提供集群在线率、CPU/RAM/磁盘/网络汇总、服务拨测与告警统计快照。 |
| **静态资源** | `xlstatus://servers/inventory` | `Live Server Fleet Inventory & Telemetry` | `server:read` | 返回当前可见的最多 200 台服务器的完整资产清单与实时遥测指标。 |
| **静态资源** | `xlstatus://services/health` | `Service Probes & SLA Status` | `service:read` | 返回最多 200 个 HTTP/TCP/ICMP 服务拨测监控项的最新状态、延迟与 TLS 证书有效期。 |
| **静态资源** | `xlstatus://alerts/active` | `Active Incidents, Alert Events & Rules` | `alert:read` *(或 `server:read`)* | 聚合返回最近 30 条告警事件（`events`）与最多 50 条告警规则（`rules`）。 |
| **静态资源** | `xlstatus://network/topology` | `NAT Port Mappings & DDNS Records` | `nat:read` *(或 `ddns:read` / `server:read`)* | 返回当前可见节点上启用的 NAT 端口映射与 DDNS 域名解析记录。 |
| **动态模板** | `xlstatus://servers/{server_id}/telemetry` | `Deep Server Hardware, Live State & 1h TSDB Summary` | `server:read` | 将 `{server_id}` 替换为 36 位服务器 UUID，一次性返回该节点的 `server.get` 硬件/实时状态与 `server.metrics`（`1h` 窗口）时序统计聚合。 |

---

## 5. 4 套 AI SRE 动态提示词模板 (AI SRE Prompts)

通过 `prompts/get` 获取提示词时，`McpExecutor` 会在服务端**实时查询最新集群遥测数据并内联注入到 Prompt 消息体中**，使 LLM 无需多轮工具往返即可立即给出精准诊断（调用任意 Prompt 需具备 `server:read` Scope）：

| Prompt 名称 (`name`) | 参数列表 (`arguments`) | 服务端自动预注入的实时上下文 | 引导 AI 输出的标准分析结构 |
|---|---|---|---|
| **`sre.cluster_health_audit`** | 无 | 1. `cluster.overview` 全局快照<br>2. `cluster.diagnose` 自动异常扫描结果 | 1. **Executive Fleet Summary**（健康分、在线比、整体饱和度）<br>2. **Prioritized Findings**（按 `critical` → `warning` 归类根因）<br>3. **Actionable Remediation Plan**（推荐后续调用的 MCP 工具与安全命令） |
| **`sre.troubleshoot_node`** | • `server_id` (**必填**，支持填写 UUID **或服务器模糊名称**，服务端自动调用 `server.resolve` 解析)<br>• `symptom` (可选，如 `high_cpu`、`memory_pressure`、`disk_full`、`offline`) | 1. `server.get` 节点硬件与实时状态<br>2. `server.metrics` (`range="1h"`) 时序聚合与曲线 | 1. **Telemetry Correlation**（结合症状关联分析 CPU/RAM/Swap/Disk/Load/Net）<br>2. **Diagnostic Playbook Steps**（推荐运行的 `server.inspect` 剧本及关注指标）<br>3. **Safe Remediation Commands**（非破坏性止血与修复命令） |
| **`sre.incident_triage`** | 无 | 1. `cluster.diagnose` 活跃异常<br>2. `alert.events` (最近 20 条告警事件)<br>3. `service.list` (最多 50 项服务监控) | 1. **Incident Timeline & Blast Radius**（关联告警事件、故障服务探针与受影响主机，界定爆炸半径）<br>2. **Immediate Triage Actions**（分步验证与恢复指令） |
| **`sre.capacity_and_security`** | 无 | 1. `cluster.overview` 资源汇总<br>2. `server.list` (最多 100 台节点详情)<br>3. `network.topology` (NAT 与 DDNS 拓扑) | 1. **Resource Saturation & Right-Sizing**（识别过载或闲置节点）<br>2. **Lifecycle & Billing Risks**（预警即将到期的 `expires_at` 节点）<br>3. **Network Exposure**（审计暴露的 NAT 公网端口、源 IP 限制与 DDNS 记录） |

---

## 6. 安全与多租户隔离模型 (Security & Multi-Tenant Isolation)

### 6.1 PAT Scope 最小权限矩阵
签发 Personal Access Token (`xlp_...`) 时，可按需勾选最小 Scope 集合：

| Scope 名称 | 允许调用的 MCP Tools | 允许读取的 `xlstatus://` Resources & Prompts |
|---|---|---|
| *(任意有效 PAT)* | `meta.whoami` | — |
| **`server:read`** | `cluster.overview`、`cluster.diagnose`、`server.list`、`server.get`、`server.resolve`、`server.metrics`<br>*(兼作 `alert.events`、`alert.rules`、`network.topology` 的兼容回退 Scope)* | • `xlstatus://cluster/overview`<br>• `xlstatus://servers/inventory`<br>• `xlstatus://servers/{server_id}/telemetry`<br>• 全部 4 个 `sre.*` Prompts |
| **`server:exec`** | `server.inspect`、`server.exec`、`server.batch_exec` | — |
| **`service:read`** | `service.list`、`service.history` | • `xlstatus://services/health` |
| **`alert:read`** | `alert.events`、`alert.rules` | • `xlstatus://alerts/active` |
| **`task:read`** | `task.list` | — |
| **`nat:read`** *(或 `ddns:read`)* | `network.topology` | • `xlstatus://network/topology` |
| **`transfer:read`** | `fs.list`、`fs.read`、`fs.download_url` | — |
| **`admin:*`** | 全部 19 项 MCP Tools | 全部 Resources、Resource Templates 与 Prompts |

### 6.2 `server_ids` 节点白名单强制隔离
创建 PAT 时可绑定可选的 `server_ids` 数组（节点 UUID 白名单）。一旦绑定，`McpExecutor` 会在**所有数据读取与操作路径**强制过滤：
- **服务器与集群维度**：`server.list`、`server.get`、`server.resolve`、`server.metrics`、`cluster.overview`、`cluster.diagnose` 仅统计和返回同时满足 **当前用户拥有权限** 且 **在 `server_ids` 白名单内** 的节点。
- **命令与文件维度**：对不在 `server_ids` 内的节点调用 `server.inspect`、`server.exec`、`server.batch_exec`、`fs.list`、`fs.read`、`fs.download_url` 会直接拒绝并返回 `Server not allowed`。
- **关联资源过滤**：`service.list`、`service.history`、`alert.events`、`alert.rules`、`task.list`、`network.topology` 会自动剔除绑定到非白名单节点或包含通配符全局选择器的记录，杜绝跨节点信息泄露。

### 6.3 Cookie Studio 敏感操作 TOTP 提权验证
在 Web 控制台的 **MCP AI Ops Studio**（使用浏览器 Cookie Session 认证）中：
- 只读工具、`server.inspect` 只读诊断剧本、资源读取与 Prompt 生成可直接执行。
- 执行高危工具 **`server.exec`** 或 **`server.batch_exec`** 时，若当前登录账号已开启 TOTP 二步验证，后端 `check_mcp_studio_sensitive_totp` 强制要求请求头携带有效的 6 位动态口令 `x-totp-code`，否则返回 `403 Forbidden`。

### 6.4 为什么禁用 PAT 远端文件写入工具？
在早期设计中曾评估过 `fs.write`、`fs.delete` 与 `fs.upload_url`。在生产安全审计中，XLStatus **彻底移除了所有 MCP 文件写入与删除工具**（`get_available_tools()` 仅暴露 19 个工具，且 `McpExecutor` 对任何遗留的 `fs.write` / `fs.delete` / `fs.upload_url` 调用直接拒绝并报错 `"file write operations require a cookie session"`）：
1. **防范间接提示词注入（Indirect Prompt Injection）**：AI 在读取远端日志（`system_logs` / `fs.read`）时，若日志中包含攻击者精心构造的恶意指令，开放 `fs.write` 可能导致 AI 在未经人工确认的情况下覆写 `~/.ssh/authorized_keys`、`/etc/crontab` 或系统配置。
2. **职责分离**：远端文件上传、修改与删除必须在 Web 终端文件管理器中由人工通过受 CSRF 与会话保护的浏览器完成。

---

## 7. 客户端即用配置示例 (Ready-to-Copy Client Configurations)

接入前，请先登录 XLStatus Web 控制台 → **令牌管理 (`/tokens`)**，创建一个 Personal Access Token（形如 `xlp_...`），并根据需求勾选 `server:read`、`server:exec`、`service:read`、`alert:read`、`task:read`、`nat:read`、`transfer:read` 等 Scope。

### 7.1 Antigravity / Cursor / Windsurf (`mcpServers` 配置)

#### 方式 A：使用内置 Stdio 桥接器（推荐本地或跨网络通用）
将以下内容添加到 `~/.gemini/antigravity/mcp_config.json`、`~/.cursor/mcp.json` 或 `~/.codeium/windsurf/mcp_config.json`：

```json
{
  "mcpServers": {
    "xlstatus": {
      "command": "node",
      "args": [
        "/ABSOLUTE/PATH/TO/XLStatus/scripts/xlstatus-mcp-stdio.mjs"
      ],
      "env": {
        "XLSTATUS_URL": "http://127.0.0.1:8080",
        "XLSTATUS_PAT": "xlp_your_personal_access_token_here"
      }
    }
  }
}
```

#### 方式 B：使用 Direct Streamable HTTP（适用于支持原生 HTTP Transport 的 IDE）
```json
{
  "mcpServers": {
    "xlstatus": {
      "url": "http://127.0.0.1:8080/mcp",
      "headers": {
        "Authorization": "Bearer xlp_your_personal_access_token_here"
      }
    }
  }
}
```

### 7.2 Claude Desktop (`claude_desktop_config.json`)

在 macOS (`~/Library/Application Support/Claude/claude_desktop_config.json`) 或 Windows (`%APPDATA%\Claude\claude_desktop_config.json`) 中配置：

```json
{
  "mcpServers": {
    "xlstatus": {
      "command": "node",
      "args": [
        "/ABSOLUTE/PATH/TO/XLStatus/scripts/xlstatus-mcp-stdio.mjs",
        "--url",
        "https://status-api.example.com",
        "--token",
        "xlp_your_personal_access_token_here"
      ]
    }
  }
}
```

### 7.3 Claude Code CLI

在终端中运行以下命令即可将 XLStatus 注册到 Claude Code：

```bash
# 方式 1：通过原生 HTTP 传输直接挂载
claude mcp add --transport http xlstatus http://127.0.0.1:8080/mcp \
  --header "Authorization: Bearer xlp_your_personal_access_token_here"

# 方式 2：通过本地 Stdio 桥接脚本挂载
claude mcp add xlstatus \
  -e XLSTATUS_URL="http://127.0.0.1:8080" \
  -e XLSTATUS_PAT="xlp_your_personal_access_token_here" \
  -- node /ABSOLUTE/PATH/TO/XLStatus/scripts/xlstatus-mcp-stdio.mjs
```

### 7.4 `curl` JSON-RPC 2.0 调试示例

#### 1) 初始化握手与能力发现 (`initialize`)
```bash
curl -sS -X POST "http://127.0.0.1:8080/mcp" \
  -H "Authorization: Bearer xlp_your_personal_access_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": { "name": "curl-sre-client", "version": "1.0.0" }
    }
  }' | jq .
```

#### 2) 执行 AI SRE 全集群异常扫描 (`tools/call` -> `cluster.diagnose`)
```bash
curl -sS -X POST "http://127.0.0.1:8080/mcp" \
  -H "Authorization: Bearer xlp_your_personal_access_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "cluster.diagnose",
      "arguments": {
        "cpu_threshold": 80.0,
        "memory_threshold": 85.0,
        "disk_threshold": 90.0
      }
    }
  }' | jq .
```

#### 3) 运行单节点只读 SRE 诊断剧本 (`tools/call` -> `server.inspect`)
```bash
curl -sS -X POST "http://127.0.0.1:8080/mcp" \
  -H "Authorization: Bearer xlp_your_personal_access_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "server.inspect",
      "arguments": {
        "server_id": "01960000-0000-7000-8000-000000000001",
        "playbook": "top_processes"
      }
    }
  }' | jq .
```

#### 4) 读取 `xlstatus://` 实时资源与动态 SRE Prompt (`resources/read` & `prompts/get`)
```bash
# 读取集群总览资源
curl -sS -X POST "http://127.0.0.1:8080/mcp" \
  -H "Authorization: Bearer xlp_your_personal_access_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "resources/read",
    "params": {
      "uri": "xlstatus://cluster/overview"
    }
  }' | jq .

# 获取针对特定节点（支持直接传模糊主机名）的排障 Prompt
curl -sS -X POST "http://127.0.0.1:8080/mcp" \
  -H "Authorization: Bearer xlp_your_personal_access_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "prompts/get",
    "params": {
      "name": "sre.troubleshoot_node",
      "arguments": {
        "server_id": "tokyo-edge",
        "symptom": "high_cpu"
      }
    }
  }' | jq .
```
