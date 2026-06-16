# 功能对标范围

## 总原则

XLStatus 按功能能力对标 Nezha，而不是按接口或实现复刻。所有模块都可以重新设计 API、数据结构和前端交互，但必须让最终用户完成同类监控和运维工作。

## Dashboard

目标：

- 提供管理后台、公开状态页、REST API、WebSocket 实时流、Agent RPC 入口和 MCP 入口。
- 支持多用户、角色、服务器归属、PAT 自动化访问和可审计运维操作。

主要能力：

- 首页展示服务器实时状态、在线数量、资源使用、流量、地理位置和服务可用性。
- 管理后台维护服务器、分组、服务监控、告警、任务、通知、DDNS、NAT、用户、设置。
- 公开状态页在未登录情况下展示允许公开的服务器和服务监控结果。

失败场景：

- Agent 离线时服务器状态保留最后一次快照，并标记离线。
- 数据库不可写时拒绝配置变更，但继续尽量提供只读状态。
- 后台 worker 异常必须记录日志并自动重启或进入 degraded 状态。

验收标准：

- 管理员可完成全量配置闭环。
- 成员看不到其他用户的私有服务器和资源。
- 公开页不会泄露管理员备注、密钥、私有 IP 策略和隐藏服务。

## Agent

目标：

- 运行在被监控主机上，负责指标采集、状态上报、服务探测、任务执行、终端、文件和 NAT 隧道。

主要能力：

- 采集 CPU、内存、Swap、磁盘、网络流量与速度、系统负载、TCP/UDP 连接数、进程数、温度、GPU、启动时间、系统版本、架构、虚拟化信息。
- 定期上报主机信息、运行状态和公网 IP/GeoIP。
- 执行 Dashboard 下发的 HTTP GET、ICMP Ping、TCP Ping、Shell Command、Terminal、File Manager、Config Apply、Self Update、NAT、Exec、FsList、FsRead、FsWrite、FsDelete、FsTransfer。

失败场景：

- Dashboard 不可达时指数退避重连。
- 配置损坏时拒绝启动并输出明确错误。
- 远程命令超时必须清理进程组，避免孤儿进程。

验收标准：

- Linux x86_64 Agent 可作为 systemd 服务运行并自动重启。
- 网络断开后恢复连接，Dashboard 不需要手工干预。
- 禁用命令执行后，任务、终端和文件写入相关操作全部拒绝。

## 服务监控

目标：

- 由 Agent 从不同地理和网络位置探测外部目标，形成可用性、延迟和证书状态。

主要能力：

- HTTP GET：检查状态码、延迟、TLS 证书到期、证书变化和证书过期。
- ICMP Ping：检查可达性和延迟。
- TCP Ping：检查 host:port 可连接性和延迟。
- 覆盖范围：全部服务器、仅指定服务器、排除指定服务器。
- 结果展示：当前状态、最近 30 天 Up/Down、平均延迟、按 server 的历史明细。

失败场景：

- 探测 Agent 离线时该 Agent 的结果不参与本轮成功判定。
- 证书解析失败时记录独立错误，不能影响其他服务监控。
- 目标 DNS 解析失败时记录为 Down。

验收标准：

- 支持按 30 秒以上周期调度。
- 可为失败、恢复和延迟越界触发通知和任务。
- 历史查询支持 1d、7d、30d。

## 告警

目标：

- 对服务器状态、离线、流量周期和服务监控结果进行规则判断并发送通知。

主要能力：

- 资源规则：CPU、GPU、内存、Swap、磁盘、网络速度、累计流量、负载、连接数、进程数、温度。
- 离线规则：超过指定持续时间触发。
- 周期流量规则：按小时、天、周、月、年窗口统计入站、出站或总流量。
- 触发模式：持续触发、单次触发。
- 恢复通知和失败通知可分别触发任务。

失败场景：

- 规则配置非法时拒绝保存。
- 采样不足时不误报。
- 通知渠道失败时记录失败并按策略重试。

验收标准：

- 告警触发和恢复都有去重或静默标签。
- 周期流量窗口跨时区和重启后保持一致。
- 管理员和成员的告警范围严格按服务器归属和授权过滤。

## 任务

目标：

- 支持周期任务、触发任务和批量任务执行。

主要能力：

- Cron 表达式调度。
- 手动立即执行。
- 告警或服务监控失败/恢复时触发。
- 执行范围支持全部、指定、排除、触发来源服务器。
- 保存最后执行时间、结果、输出摘要。

失败场景：

- Agent 离线时记录为 offline。
- 命令被 Agent 禁用时记录为 rejected。
- 输出过大时截断并标记。

验收标准：

- 同一 Agent 的任务结果流发送串行化，避免并发写流损坏。
- 批量执行返回 success、failure、offline 三类 ID。
- 任务执行结果可触发通知。

## 通知

目标：

- 提供可配置 Webhook 通知渠道和通知组。

主要能力：

- 支持 GET、POST。
- 支持 JSON 和 Form body。
- 支持自定义 Header。
- 支持模板占位符：事件内容、时间、服务器 ID、名称、IP、CPU、内存、磁盘、流量、连接数、负载等。
- 支持指标单位格式化。

失败场景：

- URL 指向本机、私网、链路本地、保留地址或非 HTTP/HTTPS 时拒绝。
- 非 2xx 响应记录失败。
- 重定向默认不跟随。

验收标准：

- 可以配置 Telegram、Slack、Bark、Discord 等常见 Webhook。
- 通知测试按钮能验证渠道。
- 每条通知发送都可追踪请求结果但不记录敏感 body 明文。

## DDNS

目标：

- 当 Agent 上报 IP 变化时，由 Dashboard 集中更新 DNS 记录。

主要能力：

- Provider：Cloudflare、Tencent Cloud、HE、Webhook、Dummy。
- 支持 IPv4 A 记录和 IPv6 AAAA 记录。
- 每个服务器可绑定多个 DDNS 配置，可覆盖域名。
- Webhook Provider 支持 method、headers、body、占位符。

失败场景：

- Provider 凭证错误时记录失败并按最大重试次数重试。
- IP 未变化时不重复写 DNS。
- 域名格式非法时拒绝保存。

验收标准：

- Agent IP 变化后在一个上报周期内触发 DDNS。
- 更新成功、失败、跳过都产生审计事件。

## NAT

目标：

- 允许通过 Dashboard 反向代理访问 Agent 内网服务。

主要能力：

- 按域名匹配 NAT 配置。
- 绑定目标 Agent、内部 host 和端口。
- Dashboard 与 Agent 建立流式隧道。
- 支持启用、禁用和保留域名。

失败场景：

- Agent 离线时返回明确错误页。
- 域名与 Dashboard 保留 host 冲突时拒绝保存。
- 隧道超时或断开时关闭两端流。

验收标准：

- HTTP 请求可通过 Dashboard 转发到 Agent 内网服务。
- 成员不能创建抢占 Dashboard 入口域名的 NAT。

## MCP

目标：

- 为自动化客户端和 LLM 工具客户端提供受控运维能力。

主要能力：

- Streamable HTTP JSON-RPC。
- 方法：initialize、notifications/initialized、ping、tools/list、tools/call。
- 工具：meta.whoami、server.list、server.get、server.exec、fs.list、fs.read、fs.write、fs.delete、fs.download_url、fs.upload_url。
- 临时上传下载 URL 默认 300 秒 TTL，最大 600 秒，单次使用，最大 100 MiB。

失败场景：

- MCP 默认关闭。
- PAT 吊销、scope 不足、server allowlist 不匹配时立即拒绝。
- URL 未使用到期后自动清理。

验收标准：

- MCP 只接受 PAT，不接受浏览器 Cookie session。
- tool error 不返回成功 schema。
- 限流对无效 tool 和 malformed 请求同样生效。

