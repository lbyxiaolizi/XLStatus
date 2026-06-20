# 前端规划

## 目标

Next.js 前端提供两类体验：

- 管理后台：面向运维人员，信息密度高、扫描效率优先。
- 用户状态页：面向访客或普通用户，展示公开服务器和服务可用性。

不做营销落地页。登录后第一屏进入 Dashboard；未登录且系统允许公开访问时第一屏进入状态页。

## 技术栈

- Next.js App Router
- TypeScript
- React Query 或 TanStack Query
- Zustand 或 React Context 保存轻量 UI 状态
- WebSocket client 封装实时状态流
- 图表使用 ECharts 或 uPlot
- 表单使用 React Hook Form + Zod
- UI 组件使用 shadcn/ui 或自建薄组件层

## 页面结构

公开区域：

- `/`：公开状态页
- `/login`：登录
- `/services`：公开服务可用性

管理后台：

- `/dashboard`：服务器总览
- `/dashboard/servers`：服务器管理
- `/dashboard/server-groups`：分组
- `/dashboard/services`：服务监控
- `/dashboard/alert-rules`：告警规则
- `/dashboard/tasks`：任务
- `/dashboard/notifications`：通知渠道和通知组
- `/dashboard/ddns`：DDNS
- `/dashboard/nat`：NAT
- `/dashboard/transfers`：文件传输
- `/dashboard/audit`：审计日志
- `/dashboard/settings`：系统设置
- `/dashboard/settings/api-tokens`：PAT
- `/dashboard/users`：用户管理
- `/dashboard/waf`：WAF 和在线用户

## 核心组件

- `ServerStatusTable`：实时服务器表格。
- `ServerMetricChart`：服务器指标曲线。
- `ServiceAvailabilityGrid`：30 天可用性。
- `TaskRunDialog`：任务执行和结果。
- `TerminalPanel`：Web Terminal。
- `FileManagerPanel`：远程文件管理。
- `PermissionGate`：按 role 和 scope 控制显示。
- `AuditLogTable`：审计查询。
- `SecretField`：密钥输入，默认隐藏。

## Nezha Dash V2 对齐项

从 `hamster1963/nezha-dash-v2` 补齐以下交互：

- 服务器详情只保留资源与网络两类图表页签，网络页签同时承载流量和服务探测。
- 服务探测延迟与丢包率使用同一张双轴图叠加展示，延迟走 ms 轴，丢包率走百分比轴。
- 探测点支持多选、清空选择；未选择时展示全部探测点。
- 探测点摘要显示最新延迟、最低延迟、最高延迟和平均丢包率。
- 网络/探测图表共用 1d、7d、30d 时间范围。
- 探测延迟图提供削峰开关，用滑动窗口降低尖刺对趋势判断的干扰。
- 图表 hover 光标必须按实际 SVG 绘图区坐标定位，不能因为边框、缩放或左右留白造成选中点偏移。
- 服务器列表支持卡片/紧凑列表视图切换，并把偏好保存在浏览器本地。
- 服务器列表支持状态、标签/分组、指标排序和升降序切换。
- 服务器列表显示总数、在线/离线、上传/下载速率和累计流量汇总。
- 服务器列表 API 从最新 HostState 暴露累计流量和运行时间；在有相邻 TSDB 样本时推导上传/下载速率，WebSocket 实时帧也按累计流量差值补速率。
- 服务器列表可展开服务状态面板，展示最近 30 天 Up/Down、平均延迟和可用率。
- 服务状态面板在没有周期流量后端数据时使用空态；后续增加 cycle transfer API 后展示周期流量卡。
- 服务器卡片显示运行时间、上传/下载速率、累计流量和自定义标签。
- 公开状态页复用 30 天服务状态条，保持访客视图和管理视图一致。
- 服务器详情资源页补齐负载、磁盘、Swap、进程、连接数、温度和 GPU 图表；缺少某项数据时显示该图表空态。
- 全局搜索/命令面板已接入导航栏，支持快捷键打开、页面跳转、服务器快速打开和浅/深主题快捷切换。
- 世界地图、语言切换、自定义背景、Logo/主题色注入属于上游主题壳能力；需要新增前端配置模型、完整多语言资源或地图坐标数据后再启用。
- 周期流量统计和世界地图属于数据源依赖项；需要后端持续存储周期配额/地理坐标后再接入。

## 交互规则

目标：

- 减少说明文字，依靠清晰控件和状态反馈完成配置。

规则：

- 表格支持搜索、过滤、排序、批量操作。
- 危险操作使用确认弹窗，展示目标数量和名称。
- 远程命令、文件删除、PAT 创建、DDNS 凭证保存都需要明确确认。
- WebSocket 断开显示重连状态，不清空已有数据。
- 表单保存失败保留用户输入。

失败场景：

- 401：跳转登录。
- 403：显示权限不足，不隐藏整个页面导致用户迷路。
- 422：在字段旁展示校验错误。
- 5xx：显示 request_id 方便查日志。

验收标准：

- 管理员能在 UI 完成所有核心配置。
- 成员只能看到自己可访问的菜单和资源。
- 移动端至少可查看状态、服务、告警和任务结果。

## 状态页

目标：

- 为访客展示允许公开的服务器和服务可用性。

展示内容：

- 系统名称。
- 在线服务器数量。
- 每台公开服务器的 CPU、内存、磁盘、网络速度、流量、在线状态。
- 公开服务监控的 30 天可用性和当前状态。

隐私规则：

- 不显示管理员备注。
- 不显示隐藏服务器和隐藏服务。
- IP 按系统设置决定是否脱敏。

验收标准：

- 未登录用户无法通过前端或 API 获取隐藏资源。

## 前后端契约

REST：

- 所有响应统一 envelope：`ok`、`data`、`error`、`request_id`。

WebSocket：

- `server.snapshot`：初始全量。
- `server.patch`：增量状态。
- `server.offline`：离线。
- `terminal.output`、`terminal.closed`。
- `transfer.progress`、`transfer.done`、`transfer.failed`。

验收标准：

- 前端 type 从 OpenAPI 生成或在 `web/lib/api/types.ts` 中集中维护。
