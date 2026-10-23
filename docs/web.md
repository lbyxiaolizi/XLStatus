# XLStatus Web 前端工作台架构与开发指南

XLStatus 前端（`web/`）基于 **Next.js 16 (App Router)**、**React 19**、**Tailwind CSS v4** 与 **全严格模式 TypeScript** 构建，提供面向大规模集群的高密度运维工作台、AI-Native MCP 演练场与 SSR 公开状态页。

---

## 1. 核心设计特性

### 1.1 TypeScript 全严格模式零妥协
[`web/tsconfig.json`](../web/tsconfig.json) 开启了全部严格类型编译选项：
- `strict: true`、`noUncheckedIndexedAccess: true`、`exactOptionalPropertyTypes: true`
- `noPropertyAccessFromIndexSignature: true`、`noImplicitOverride: true`、`noImplicitReturns: true`
- `noFallthroughCasesInSwitch: true`、`noUnusedLocals: true`、`noUnusedParameters: true`

### 1.2 PC Priority+ 自适应溢出导航栏（`Navigation.tsx`）
- 在桌面与平板视图下，顶部导航栏通过 `ResizeObserver` 与离屏测量行实时计算可用横向宽度。
- 横向空间充足时，所有核心模块图标与名称直接平铺展示，无需点击折叠按钮。
- 当窗口收窄导致部分菜单项无法完整展示时，超出宽度的菜单项会自动收纳进右侧的 **“更多 (+N) ▾”** 溢出下拉菜单（`data-testid="nav-overflow-trigger"`），并支持键盘快捷键提示与角色权限过滤（`isAdmin(user)`）。

### 1.3 全局 `⌘K` / `Ctrl+K` 命令与搜索面板
- 在应用任意页面按下 `⌘K`（macOS）或 `Ctrl+K`（Windows/Linux），即可呼出全局命令面板。
- 支持**实时节点检索**（按名称、地域、标签或 IP 搜索集群节点并查看在线状态）与**全路由快捷跳转**（一键直达 `/dashboard`、`/servers`、`/mcp`、`/terminal`、`/settings` 等工作台）。

---

## 2. 功能工作台概览

| 路由路径 | 模块名称 | 核心交互设计 |
| :--- | :--- | :--- |
| `/dashboard` | **集群态势总览** | 顶部 4 维核心 KPI 矩阵、实时集群健康分布条、**Top 5 高负载节点排行（支持 CPU / 内存 / 磁盘一键切换）**、在线节点速览网格与快捷运维入口 |
| `/servers` | **节点控制台** | 统一工具栏整合实时搜索、状态筛选胶囊（全部/在线/离线）、分组过滤与视图切换器（卡片网格 / 高密度表格 / `WorldServerMap` 全球地图），选中节点后底部浮现 **Sticky 批量操作栏**（一键带入批量任务或批量删除） |
| `/servers/[id]` | **单节点深度工作台** | 实时硬件与系统遥测、多时间窗 TSDB 图表（`MiniLineChart` / `MiniDualAxisChart`）、安全远端文件管理器（目录浏览、在线编辑、单次临时上传/下载）、远程配置下发与强制 OTA 升级 |
| `/mcp` | **AI 运维与 MCP 中心** | 三合一 AI-Native 运维工作台：<br>1. **智能巡检与自愈台 (`diagnose`)**：实时健康评分（`0-100`）、异常雷达卡片、6 套一键跨平台 SRE 诊断 Playbook 与 4 套实时遥测增强 Prompt 生成器<br>2. **协议演练场 (`playground`)**：交互式调试全部 19 个 Tool、6 个 `xlstatus://` Resource 与 4 个 Prompt，支持 Session Studio 与原生 PAT JSON-RPC 双模式<br>3. **一键接入向导 (`connect`)**：3 档权限预设一键生成 `xlp_` 令牌并自动注入 Antigravity/Cursor、Claude Desktop (`stdio`)、Claude Code CLI 与 `curl` 配置代码 |
| `/tasks` | **批量任务与 Playbook** | 内置 5 组常用运维命令预设胶囊（系统概览、磁盘空间、内存负载、监听端口、Docker 容器）、在线节点快捷选择器与暗色终端风格执行输出检查器 |
| `/terminal` | **Web 交互式终端** | 顶部在线节点快速切换胶囊栏、实时会话维度与连接遥测状态条、快捷排障命令片段栏（一键向活跃 WebSocket 注入命令）以及基于 `xterm.js` 的可调高度终端画布 |
| `/services` | **服务监控与 SLA** | HTTP(S)/TCP/ICMP 探针管理、TLS 证书剩余天数预警、可用率热力条与在线探针即时测试 |
| `/alerts` & `/notifications` | **告警与通知工作台** | 顶部实时事件概览条 + 4 分段标签页（告警事件流 / 告警规则 / 通知渠道 / 通知分组），支持单遍 JSON 安全模板预览与测试投递 |
| `/nat` & `/ddns` | **网络穿透与动态域名** | NAT 反向隧道端口映射管理与 Cloudflare / HE.net / DNSPod / Webhook 动态解析状态监控 |
| `/settings` | **分段式系统设置中心** | 将 11 个管理模块归纳为 4 大顶层分类标签（**节点与接入**、**安全与认证**、**外观与公开页**、**系统与维护**），按需挂载对应组件以消除首屏无关请求 |
| `/status` | **公开状态页 (SSR)** | 支持首屏服务端渲染（SSR）、实时关键词检索、在线/离线状态胶囊筛选、动态国家/地区胶囊过滤及卡片/表格/全球地图三视图切换 |

---

## 3. 前端安全防御体系（`web/app/lib/security.ts`）

前端内置了专门的安全清洗与边界校验模块（[`web/app/lib/security.ts`](../web/app/lib/security.ts)），并通过 14 项独立单元测试（[`web/app/lib/security.test.mts`](../web/app/lib/security.test.mts)）持续验证：

1. **防 WHATWG URL 点段路径穿越（`encodePathSegment`）**：
   - 在 [`web/lib/api.ts`](../web/lib/api.ts) 的所有动态 REST 路径拼接点强制调用 `encodePathSegment`，显式拦截 `.`、`..`、`%2e`、`%2e%2e`、空字符串与控制字符，防止浏览器 `fetch()` 自动归一化折叠父路径。
2. **防开放重定向（`sanitizeRedirectPath` / `resolveLoginRedirectParam`）**：
   - 登录页与 OAuth 回调页仅允许以单斜杠 `/` 开头的同源相对路径，严格拦截 `//evil.com`、`/\\evil.com`、`javascript:` 与控制字符。
3. **品牌资源 URL 与主题 CSS 变量注入防护（`sanitizeAssetUrl` / `sanitizeCssVariableValue`）**：
   - 对公开状态页与设置页中的 `logo_url`、`favicon_url`、`background_url` 拦截 `javascript:`、`data:`、`vbscript:` 及 CSS `url(...)` 逃逸字符（`"`、`'`、`)`、`\\`、`;`）。
   - 对自定义主题 CSS 变量拦截 `;`、`{`、`}`、`</style>`、`@import`、`expression()` 等样式表逃逸载荷。
4. **临时传输同源校验与敏感令牌会话级隔离**：
   - 上传/下载临时文件前通过 `resolveSafeApiUrl` 强制校验 URL 必须与当前配置的 `API_BASE_URL` 同源且路径以 `/api/v1/temp-` 开头。
   - 注册令牌等短期敏感凭证仅保存在会话级 `sessionStorage` 中，并在调用 `apiClient.logout()` 时主动清空。

---

## 4. 本地开发、构建与验证命令

```bash
cd web

# 安装依赖
pnpm install --frozen-lockfile

# 启动本地开发服务器（默认监听 http://127.0.0.1:3000）
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev

# 运行全严格模式 TypeScript 类型检查
pnpm typecheck

# 运行 ESLint 静态检查
pnpm lint

# 运行前端安全与基础库单元测试（14 项）
pnpm test:unit

# 执行生产构建
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
```
