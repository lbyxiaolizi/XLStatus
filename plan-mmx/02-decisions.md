---
title: 20 项核心决策
status: stable
audience: [human, agent]
---

# 02. 核心决策汇总

20 项核心决策，每项给出**选定 + 理由**。变更需在 `99-changelog.md` 记录。

## 性能+安全优先矩阵

| ID | 决策点 | 选定 | 性能收益 | 安全收益 |
|----|--------|------|----------|----------|
| **D1** | Agent 协议 | **gRPC (tonic, HTTP/2) + Dashboard REST/WS** | HTTP/2 多路复用 1 连接跑多流；proto 二进制比 JSON 小 50–70%；序列化快 5–10× | TLS + JWT metadata 鉴权 |
| **D2** | API 风格 | REST + 统一响应信封 | HTTP/1.1 缓存友好；CDN/反代可观测 | 错误格式统一 |
| **D3** | Web 认证 | 密码 + HttpOnly Cookie + refresh rotation | short access token 减少 401 重验证 | refresh 哈希存库可吊销 |
| **D4** | Agent 认证 | Ed25519 私钥长期 + 5min JWT + challenge 续签 | 不每包签名 → 零额外 CPU | JWT 短期 → 重放窗口 5min |
| **D5** | ORM | sqlx 直 SQL | 零运行时反射；编译期 SQL 校验 | 参数化查询防 SQL 注入 |
| **D6** | Dashboard WS | 单向 push（Server → Client） | 服务端只 push 资源占用稳定 | 客户端无写入权限 |
| **D7** | 多租户 | admin/viewer 二元角色 | 权限检查简单 | RBAC 基础 |
| **D8** | 时序保留 | raw 7d + 5min 30d + 1h 1y | 物化聚合查询快 | 自动清理避免泄漏 |
| **D9** | gRPC 鉴权 | metadata `authorization: bearer <jwt>` | 一次握手搞定 | tonic 拦截器统一处理 |
| **D10** | 采样间隔 | 可配置默认 10s | 写盘压力可控 | — |
| **D11** | 写路径 | 1s 攒批 + sqlx Copy/transaction | 10× 写盘吞吐 | — |
| **D12** | 读路径 | PG 命中 CAGG；SQLite 走物化表 | 24h/7d/30d P99 < 200ms | — |
| **D13** | 加密传输 | 强制 HTTPS + Caddy ACME | 反代缓存命中 | 防 token 泄漏 |
| **D14** | 限流 | 登录 5/min/IP、API 1000/min/user、单连接 | 防 DoS | 防撞库/防多连接 |
| **D15** | 密码哈希 | argon2id 19MB/2iter/1lane | — | 防离线破解 |
| **D16** | 终端权限 | v1 不做 | 减复杂度 | 移除高危面 |
| **D17** | 前端数据流 | React Query + 单一 WS + 增量 | 一 socket 复用多 agent | 断线重连无感 |
| **D18** | 时间字段 | 服务端时间戳为准 | 防 agent 时钟漂移 | 日志对账准确 |
| **D19** | Dashboard 主题 | 暗色 zinc | 视觉对标哪吒 | — |
| **D20** | i18n | v1 仅 zh-CN | 集中精力 | — |

## 已替代的早期决策

| 原决策 | 新决策 | 变更原因 |
|--------|--------|----------|
| D1 原选 WebSocket (JSON) | D1 gRPC (tonic) | 用户反馈："gRPC 这种高效工具该用还是要用" |
| M2 原名 "Agent 注册 + 持续连接" | M2 "Agent gRPC 接入" | 与新 D1 一致 |
| Risk 原"WebSocket 大规模连接" | Risk"gRPC 大规模连接" | 传输协议变更 |
| Risk 原"WS 断线补传" | Risk "gRPC 断线补传" | 同上 |

## 决策变更流程

1. 在 `99-changelog.md` 追加条目
2. 更新对应决策行
3. 检查影响：`11-roadmap.md`、`06-grpc-protocol.md`、`07-rest-api.md` 等引用了此决策的文件
4. 通知相关里程碑负责人