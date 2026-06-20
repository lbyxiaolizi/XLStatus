---
title: XLStatus 项目规划索引
description: 自研服务器/网站监控系统，功能对标哪吒探针，性能+安全优先
status: active
last_updated: 2026-06-16
audience: [human, agent]
---

# XLStatus — 项目规划

自研的服务器/网站监控系统。**功能完整对标**开源项目哪吒探针（nezhahq/nezha），但**不复用原版任何代码、API、数据库 schema、protobuf 字段名**——是独立设计的产品。**性能与安全是最高优先级**。

## 阅读路线

| 你是谁 | 建议阅读顺序 |
|--------|--------------|
| **人类项目负责人**（想了解全局） | `README.md` → `01-context.md` → `02-decisions.md` → `11-roadmap.md` |
| **人类执行者**（要落地某个里程碑） | `11-roadmap.md` → 对应章节（`05`/`06`/`09` 等） |
| **Agent**（自动化实施） | 按 `11-roadmap.md` 的里程碑顺序，每个 Mx 开始前读完对应章节 |

## 目录结构

| 文件 | 内容 | 何时打开 |
|------|------|----------|
| `README.md` | 本文件（索引） | 入口 |
| `01-context.md` | 项目定位 + 对标/自研边界 + 术语表 | M0 开始前 |
| `02-decisions.md` | 20 项核心决策表（D1–D20） + 变更记录 | 设计争议时 |
| `03-architecture.md` | 通信矩阵 + 架构图 + 部署拓扑 | M0 设计阶段 |
| `04-workspace-layout.md` | 完整目录树 + 文件职责 | M0 起步 |
| `05-dependencies.md` | Cargo workspace + web 依赖 + feature flags | M0 起步 |
| `06-grpc-protocol.md` | proto 文件 + RPC 设计 + 拦截器 + 性能 | M0/M2 实施 |
| `07-rest-api.md` | REST 端点 + 响应信封 + 错误码 | M1 实施 |
| `08-websocket.md` | Dashboard WS 协议 + 消息 schema | M3 实施 |
| `09-auth-model.md` | Web + Agent 认证全流程 + JWT 格式 | M1/M2 实施 |
| `10-database.md` | 完整 SQL schema + PG 物化表 + SQLite rollup | M1/M8 实施 |
| `11-roadmap.md` | 9 个里程碑 M0–M9 + 验证标准 + 风险 | 全程 |
| `12-verification.md` | 单元 + 集成 + E2E + 性能基线 | 每个里程碑结束 |
| `13-files-checklist.md` | 高频/低频文件清单 + 复用依赖 | 协作/交接 |
| `99-changelog.md` | 规划变更记录 | 设计争议时 |

## 关键事实速览

| 维度 | 值 |
|------|----|
| 工程栈 | Rust (axum + tonic + sqlx) + Next.js 14 + shadcn/ui + Recharts |
| 通信 | gRPC :50051 (Agent↔Server) + REST/WS :8080 (Dashboard↔Server) |
| 存储 | SQLite（默认） / PostgreSQL+TimescaleDB（feature flag） |
| 总工期 | ~30 工作日（v1 不含 Web 终端） |
| 当前阶段 | M0 待启动 |

## 立即开始

```bash
# 1. 看里程碑
cat plan-mmx/11-roadmap.md

# 2. 开始 M0
cd plan-mmx && cat 04-workspace-layout.md 05-dependencies.md 06-grpc-protocol.md
```