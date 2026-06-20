---
title: 项目定位与上下文
status: stable
audience: [human, agent]
---

# 01. 项目定位与上下文

## 一句话定义

`XLStatus` 是一款自研的服务器/网站监控系统。功能完整对标开源项目哪吒探针（nezhahq/nezha），但所有 API、数据库 schema、安全模型、protobuf 字段名全部从零设计。

## 关键定位

- **对标而非兼容**：不复用哪吒 Go 代码、不沿用其字段名、不兼容其 gRPC 协议
- **性能优先**：批量写盘、HTTP/2 多路复用、proto 二进制、TimescaleDB 物化聚合
- **安全优先**：argon2id 密码、Ed25519 challenge-response、HTTPS 强制、限流、short-lived JWT

## 对标的产品能力（保留）

- 探针列表（多机状态总览）
- 状态监控：CPU、内存、Swap、磁盘、网络上下行与速率、负载、TCP/UDP 连接数、进程数、传感器温度、GPU
- 监控任务：HTTP、TCP、Ping、SSL 证书
- 告警规则 + 多渠道通知（Telegram / Webhook / Email / Lark / DingTalk）
- 计划任务（v1 可选）
- Web 终端（v1 不做）
- Web Dashboard：列表、详情图表、24h 趋势、实时刷新

## 自研的部分（不复用）

- Agent ↔ Server 协议、字段、消息类型
- REST API 路径、字段名、响应格式
- 数据库 schema、表结构、列名
- 认证方案、会话模型、密码学原语
- 前端实现细节

## 仓库现状

```
XLStatus/
├── .gitignore
├── CLAUDE.md                  # 已生成
├── Cargo.toml                 # 包名 XLStatus, version 0.1.0, edition 2024, 无依赖
├── src/main.rs                # println!("Hello, world!")
└── plan-mmx/                  # 本规划目录
```

- 分支：`master`，无 commits
- 无 git 历史可参考
- 无 README、无 Cursor/Copilot 规则

## 术语表

| 术语 | 含义 |
|------|------|
| **Agent** | 部署在被监控机器上的 Rust CLI 进程，负责采集并上报 |
| **Server** | 中心节点 Rust 服务，接收数据、存储、推送 dashboard |
| **Dashboard** | Next.js 14 Web 前端，浏览器访问 |
| **Sample** | 一条状态采样（CPU/内存/网络等） |
| **HostInfo** | 静态主机信息（platform/arch/cpu列表/...） |
| **MonitorTask** | 监控任务（HTTP/TCP/Ping/SSL） |
| **AlertRule** | 告警规则（metric + operator + threshold + duration） |
| **AlertEvent** | 告警触发/恢复事件 |
| **Notifier** | 通知渠道（Telegram/Webhook/...） |
| **Enrollment Token** | 一次性 token，agent 首次注册用 |
| **Session** | Agent 与 Server 之间的活跃 gRPC stream |
| **CAGG** | TimescaleDB continuous aggregate（连续聚合视图） |