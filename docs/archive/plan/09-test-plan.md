# 测试计划

## 单元测试

权限：

- Admin 可访问全部资源。
- Member 只能访问自己的资源。
- PAT scope 缺失时拒绝。
- PAT server allowlist 对任务、服务、告警 fanout 生效。
- PAT 不能访问 profile、password、api token 自管理接口。

配置：

- Dashboard 默认值。
- Agent 默认值。
- 无效配置拒绝。
- secret 不写入日志。

告警：

- CPU、内存、磁盘、流量、离线、温度规则。
- duration 采样不足不误报。
- 单次触发不重复通知。
- 恢复状态发送恢复通知。
- 周期流量窗口计算。

通知：

- GET、POST JSON、POST Form。
- Header 渲染。
- 占位符替换。
- 指标单位格式化。
- 非 2xx 记录失败。

DDNS：

- A 和 AAAA 记录。
- Provider 参数校验。
- Webhook 占位符。
- 最大重试。

TSDB：

- 写 buffer flush。
- 1d、7d、30d 降采样。
- retention 清理。
- 查询空数据。

数据库后端：

- SQLite migration 从空库初始化成功。
- PostgreSQL migration 从空库初始化成功。
- repository 测试在 SQLite 和 PostgreSQL 上返回一致结果。
- PostgreSQL 分区表自动创建和 retention 清理。
- 连接池耗尽时返回可观测错误。

Agent：

- 采集器部分失败不影响上报。
- 网络速度差值。
- exec 超时清理。
- 文件路径校验。
- 根目录删除拒绝。

## 集成测试

Dashboard + Agent：

- Agent 注册成功。
- secret 错误拒绝。
- Agent 重连替换旧连接。
- 状态上报写内存和 TSDB。
- WebSocket 收到实时更新。

任务：

- 手动任务下发到在线 Agent。
- 离线 Agent 返回 offline。
- 多任务并发结果不串流。
- 触发任务由告警触发。

服务监控：

- HTTP 成功和失败。
- TCP 成功和失败。
- ICMP 成功和失败。
- HTTPS 证书到期模拟。

文件和终端：

- 终端打开、输入、resize、关闭。
- 文件 list/read/write/delete。
- 大文件 upload/download。
- 传输中断清理。

MCP：

- initialize。
- tools/list。
- tools/call 成功。
- tool error。
- scope denied。
- rate limit。
- 临时 URL 单次使用。

NAT：

- 域名匹配。
- Agent 内网 HTTP 代理成功。
- Agent 离线返回错误。
- 禁用配置后拒绝。

## E2E 测试

使用 Playwright：

- 初始化管理员登录。
- 创建成员。
- 添加服务器。
- Agent 接入后首页出现实时状态。
- 创建服务监控并查看历史。
- 创建通知渠道并发送测试。
- 创建告警规则并触发。
- 创建任务并手动执行。
- 打开终端并执行 `echo ok`。
- 文件管理上传、读取、删除。
- 创建 PAT 并用 MCP 列出服务器。
- 成员登录看不到管理员私有服务器。

验收标准：

- E2E 不依赖外部公网服务，HTTP/TCP/ICMP 目标由测试容器提供。

## 安全测试

SSRF：

- 通知 URL 指向 localhost 被拒绝。
- DDNS webhook 指向私网被拒绝。
- HTTP monitor 指向 metadata IP 被拒绝。

CSRF：

- Cookie session 写请求无 token 被拒绝。
- PAT Bearer 写请求不需要 CSRF。

路径：

- 相对路径拒绝。
- 根目录删除拒绝。
- symlink 行为有明确测试。

远程执行：

- 环境变量 allowlist。
- 超时 kill process group。
- 输出截断。

MCP：

- Cookie session 调 MCP 被拒绝。
- revoked PAT 调 MCP 被拒绝。
- 无效 tool 消耗限流额度。

## 性能测试

场景：

- 100 Agent，每 3 秒上报一次，持续 24 小时。
- 1000 个服务监控任务，按 30 秒周期调度。
- 30 天 TSDB 查询 P95 小于 500 ms。
- WebSocket 100 客户端同时订阅。
- PostgreSQL 后端下持续写入 service_results、task_runs、audit_logs，验证批量写入和分区索引。

指标：

- Dashboard RSS。
- CPU 使用率。
- SQL 数据库写延迟。
- PostgreSQL 连接池等待时间。
- TSDB flush 延迟。
- WebSocket 广播延迟。
- Agent CPU 和内存占用。

## 长稳测试

目标：

- 暴露重连、内存泄漏、任务堆积和文件句柄泄漏。

场景：

- Dashboard 重启后 Agent 自动重连。
- Agent 重启后状态恢复。
- 网络抖动 10 分钟后恢复。
- 持续终端会话打开关闭。
- 持续文件传输取消和重试。

验收标准：

- 24 小时无 panic。
- 无明显内存持续增长。
- worker 队列不持续堆积。
