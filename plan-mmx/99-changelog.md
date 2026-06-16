---
title: 规划变更日志
status: active
audience: [human, agent]
---

# 99. 规划变更日志

每次修改本规划目录下的任何文件，请追加一条记录。

## 格式

```markdown
## YYYY-MM-DD — <一句话标题>

**作者**：人类 / Agent
**影响文件**：plan-mmx/XX-foo.md, plan-mmx/YY-bar.md
**影响决策**：D1, D2, ...（如适用）
**影响里程碑**：M0–M9（如适用）
**变更原因**：...

**改动**：
- 旧：...
- 新：...
```

## 记录

## 2026-06-16 — 规划从单文件拆为多文件

**作者**：Agent
**影响文件**：plan-mmx/（全部）
**变更原因**：用户要求把 `./plan-mmx` 改成文件夹，文档按章节拆分，便于人类按需打开、Agent 结构化处理。

**改动**：
- 旧：`./plan-mmx`（59KB，1389 行单文件）
- 新：`./plan-mmx/` 目录，11 个 markdown + 1 个 README + 1 个 changelog

**新文件清单**：
- `README.md` — 索引
- `01-context.md` — 定位
- `02-decisions.md` — 20 项决策
- `03-architecture.md` — 架构 + 通信矩阵
- `04-workspace-layout.md` — 目录结构
- `05-dependencies.md` — 依赖 + features
- `06-grpc-protocol.md` — gRPC 协议（与 `common.proto`/`agent.proto` 完整定义）
- `07-rest-api.md` — REST API
- `08-websocket.md` — Dashboard WS
- `09-auth-model.md` — 鉴权（A/B/C/D/E 节）
- `10-database.md` — schema
- `11-roadmap.md` — 9 个里程碑 + 验收命令
- `12-verification.md` — 验证策略
- `13-files-checklist.md` — 文件清单 + 复用依赖
- `99-changelog.md` — 本文件

---

## 2026-06-16 — D1 决策变更：WebSocket → gRPC

**作者**：Agent（按用户指令）
**影响文件**：plan-mmx/02-decisions.md, 06-grpc-protocol.md, 11-roadmap.md, 13-files-checklist.md
**影响决策**：D1
**影响里程碑**：M0, M2, M8
**变更原因**：用户反馈 "gRPC 这种高效工具该用还是要用，不要因噎废食"。

**改动**：
- 旧：D1 = WebSocket (JSON)
- 新：D1 = gRPC (tonic, HTTP/2) + Dashboard REST/WS
- 新增 `crates/proto-gen/` crate
- 新增 `proto/xlstatus/v1/{common,agent}.proto`
- M2 由 "Agent 注册 + 持续连接" 改为 "Agent gRPC 接入"
- 风险章节：移除 "WebSocket 大规模连接"、"WS 断线补传"，加入 "gRPC 大规模连接"、"tonic 拦截器限制"、"proto schema 演进"
- 依赖：`tonic = "0.12"` + `tonic-reflection = "0.12"` + `tonic-build = "0.12"` + `prost = "0.13"`

---

## 2026-06-16 — 初始规划：项目目标与范围

**作者**：Agent（与用户协作）
**影响文件**：plan-mmx/（初始）
**变更原因**：用户决定：
- 范围：Agent + Server + Dashboard 全栈
- 形态：Web 应用
- 存储：SQLite（默认）+ PostgreSQL+TimescaleDB（feature flag）
- 协议：自研（不复用哪吒）
- 安全：自研（Ed25519 + JWT）
- 前端：Next.js 14 + shadcn/ui + Recharts
- 设计原则：性能 + 安全优先

**改动**：
- 仓库从 `cargo new` 空脚手架开始
- 目标：30 工作日完成 v1（不含 Web 终端）
- 9 个里程碑 M0–M9

---

## 2026-06-16 — 规划生成

**作者**：Agent
**变更原因**：用户输入 "我准备用 rust 写一个功能 1:1 对照哪吒探针的探针面板，进行完整的项目规划"。

**关键决策序列**：
1. 初版：1:1 复刻哪吒（含复用其 proto 字段名）
2. 调整：功能对标 + 自研 API/数据库/安全模型
3. 调整：性能+安全优先（argon2 / Ed25519 / 限流 / HTTPS / 短期 JWT / gRPC）
4. 调整：D1 从 WebSocket 改 gRPC
5. 调整：单文件 → 多文件目录

---

## 待办

- [ ] 添加 `docs/api.md`（OpenAPI 3.1）— M3 前
- [ ] 添加 `docs/ws-protocol.md`（Dashboard WS 详细）— M3 前
- [ ] 添加 `docs/architecture.md`（C4 架构图）— M3 前
- [ ] proto schema 演进策略文档 — M8 前后
- [ ] gRPC 性能调优笔记（h2 settings、quinn）— M8+ 性能不达标时
