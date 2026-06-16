# 规划变更日志

## 2026-06-16 合并 plan 与 plan-mmx

作者：Codex

变更原因：

- 用户要求阅读 `./plan-mmx` 并合并两份计划，打造一份最终总计划。
- 结论是 `./plan` 的功能范围更完整，`./plan-mmx` 的工程执行细节更强，因此以 `./plan` 为权威目录吸收 `plan-mmx` 的优点。

主要变更：

- 新增 [00-decisions.md](./00-decisions.md)，形成核心决策表。
- 新增 [11-workspace-layout.md](./11-workspace-layout.md)，明确 workspace 和文件职责。
- 新增 [12-dependencies.md](./12-dependencies.md)，明确 Rust/Web 依赖和 feature 策略。
- 新增 [13-protocols.md](./13-protocols.md)，合并 Agent gRPC、Dashboard WS、MCP 协议。
- 新增 [14-api-contracts.md](./14-api-contracts.md)，给出 REST 合约和端点清单。
- 新增 [15-verification-commands.md](./15-verification-commands.md)，加入命令级验收。
- 更新 [README.md](./README.md)，将 `./plan` 标记为最终权威计划。
- 更新 [07-security.md](./07-security.md)，Agent 身份升级为 enrollment token + Ed25519 + 短期 JWT。
- 更新 [08-roadmap.md](./08-roadmap.md)，从 M1-M6 扩展为 M0-M9。

保留决策：

- 完整对标范围不缩水：Web Terminal、文件管理、DDNS、NAT、MCP 都进入 v1。
- PostgreSQL 支持前置到 M1，不放到后期补丁。
- 数据库后端运行时通过 `DATABASE_URL` 切换，而不是依赖重新编译。

