# 依赖与 Feature Flags

## Rust workspace 依赖

```toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws", "macros"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "compression-br", "compression-gzip", "set-header"] }
tonic = { version = "0.12", features = ["transport", "tls"] }
tonic-build = "0.12"
tonic-reflection = "0.12"
prost = "0.13"
prost-types = "0.13"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite", "postgres", "uuid", "chrono", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde", "clock"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror = "2"
anyhow = "1"
clap = { version = "4", features = ["derive", "env"] }
argon2 = "0.5"
jsonwebtoken = "9"
ed25519-dalek = { version = "2", features = ["rand_core", "zeroize"] }
sha2 = "0.10"
subtle = "2"
zeroize = "1"
rand = "0.8"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }
utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }
cron = "0.12"
dashmap = "6"
parking_lot = "0.12"
```

## Server crate

默认启用：

- `storage-sqlite`
- `storage-postgres`
- `embedded-tsdb`
- `mcp`
- `nat`
- `terminal`

可选：

- `external-metrics`
- `otel`
- `pprof`

原则：

- SQLite/PostgreSQL 不通过互斥 feature 决定生产能力；二者默认编译进 server，通过 `DATABASE_URL` 运行时选择。
- TimescaleDB、ClickHouse、VictoriaMetrics 等外部指标后端通过 `MetricStore` 实现扩展。

## Agent crate

默认启用：

- `linux`
- `terminal`
- `file-transfer`
- `nat`

可选：

- `gpu-nvidia`
- `systemd`
- `self-update`

后续：

- `windows-service`
- `macos-launchd`

## Web 依赖

```json
{
  "dependencies": {
    "next": "^14",
    "react": "^18",
    "react-dom": "^18",
    "@tanstack/react-query": "^5",
    "zod": "^3",
    "react-hook-form": "^7",
    "lucide-react": "^0.468.0",
    "recharts": "^2",
    "xterm": "^5",
    "xterm-addon-fit": "^0.10.0"
  },
  "devDependencies": {
    "typescript": "^5",
    "eslint": "^8",
    "prettier": "^3",
    "@playwright/test": "^1"
  }
}
```

## 常用命令

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

cargo run -p xlstatus-server
cargo run -p xlstatus-agent -- run --config ./agent.yaml

cd web
pnpm install
pnpm dev
pnpm typecheck
pnpm test:e2e
```

## 版本策略

- M0 锁定第一版依赖。
- 每个里程碑结束只允许 patch/minor 安全更新。
- M6 前做一次依赖审计和许可证检查。

