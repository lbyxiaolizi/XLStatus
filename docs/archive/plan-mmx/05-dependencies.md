---
title: 依赖选型与 Feature Flags
status: stable
audience: [human, agent]
---

# 05. 依赖选型与 Feature Flags

## 根 `Cargo.toml`

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.82"

[workspace.dependencies]
# async
tokio = { version = "1.40", features = ["full"] }
tokio-tungstenite = { version = "0.24", features = ["rustls-tls-webpki-roots"] }
async-trait = "0.1"
futures = "0.3"

# web
axum = { version = "0.7", features = ["macros", "ws"] }
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.6", features = ["trace", "cors", "request-id", "set-header", "limit"] }
tower_governor = "0.4"                         # 限流
hyper = "1"

# gRPC
tonic = "0.12"
tonic-reflection = "0.12"
tonic-build = "0.12"                           # 仅 build dep
prost = "0.13"

# db
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio-rustls-rustls", "uuid", "chrono", "json", "macros", "migrate"] }

# serde
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_with = "3"

# auth / crypto
ed25519-dalek = { version = "2", features = ["rand_core"] }
argon2 = "0.5"
rand = "0.8"
sha2 = "0.10"
hmac = "0.12"
jsonwebtoken = "9"
base64 = "0.22"
zeroize = { version = "1", features = ["derive"] }
subtle = "2"                                   # 常量时间比较

# agent 采集
sysinfo = { version = "0.32", default-features = false, features = ["cpu", "disk"] }
libc = "0.2"
nvml-wrapper = "0.10"                          # 可选特性 gpu-nvidia

# misc
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
clap = { version = "4", features = ["derive", "env"] }
config = "0.14"
dirs = "5"
url = "2"
```

## `crates/server/Cargo.toml`

```toml
[package]
name = "xlstatus-server"
version.workspace = true
edition.workspace = true

[features]
default = ["storage-sqlite"]
storage-sqlite = ["sqlx/sqlite"]
storage-postgres = ["sqlx/postgres", "sqlx/uuid"]

[dependencies]
# 工作区共享依赖
tokio = { workspace = true }
axum = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
tower_governor = { workspace = true }
hyper = { workspace = true }
tonic = { workspace = true }
tonic-reflection = { workspace = true }
prost = { workspace = true }
sqlx = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
ed25519-dalek = { workspace = true }
argon2 = { workspace = true }
rand = { workspace = true }
sha2 = { workspace = true }
hmac = { workspace = true }
jsonwebtoken = { workspace = true }
base64 = { workspace = true }
subtle = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
async-trait = { workspace = true }
futures = { workspace = true }
clap = { workspace = true }
config = { workspace = true }
url = { workspace = true }

# 本 crate 内部
xlstatus-shared = { path = "../shared" }
xlstatus-proto-gen = { path = "../proto-gen" }

[build-dependencies]
tonic-build = { workspace = true }
```

## `crates/agent/Cargo.toml`

```toml
[package]
name = "xlstatus-agent"
version.workspace = true
edition.workspace = true

[features]
default = []
gpu-nvidia = ["dep:nvml-wrapper"]

[dependencies]
tokio = { workspace = true }
tonic = { workspace = true }
prost = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
ed25519-dalek = { workspace = true }
rand = { workspace = true }
sha2 = { workspace = true }
base64 = { workspace = true }
zeroize = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
clap = { workspace = true }
config = { workspace = true }
sysinfo = { workspace = true }
libc = { workspace = true }
url = { workspace = true }
dirs = { workspace = true }
nvml-wrapper = { workspace = true, optional = true }

xlstatus-shared = { path = "../shared" }
xlstatus-proto-gen = { path = "../proto-gen" }
```

## `crates/shared/Cargo.toml`

```toml
[package]
name = "xlstatus-shared"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }
base64 = { workspace = true }
sha2 = { workspace = true }
ed25519-dalek = { workspace = true }
prost = { workspace = true }
tonic = { workspace = true }
```

## `crates/proto-gen/Cargo.toml`

```toml
[package]
name = "xlstatus-proto-gen"
version.workspace = true
edition.workspace = true

[dependencies]
prost = { workspace = true }
tonic = { workspace = true }

[build-dependencies]
tonic-build = { workspace = true }
```

## `crates/xtask/Cargo.toml`

```toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true

[[bin]]
name = "mock_agent"
path = "src/bin/mock_agent.rs"

[[bin]]
name = "seed"
path = "src/bin/seed.rs"

[dependencies]
tokio = { workspace = true }
tonic = { workspace = true }
prost = { workspace = true }
rand = { workspace = true }
clap = { workspace = true }
tracing = { workspace = true }

xlstatus-proto-gen = { path = "../proto-gen" }
xlstatus-shared = { path = "../shared" }
```

## `web/package.json`（关键依赖）

```json
{
  "name": "xlstatus-web",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "start": "next start",
    "lint": "next lint"
  },
  "dependencies": {
    "next": "^14.2",
    "react": "^18.3",
    "react-dom": "^18.3",
    "@tanstack/react-query": "^5",
    "recharts": "^2.13",
    "zod": "^3.23",
    "react-hook-form": "^7.53",
    "@hookform/resolvers": "^3.9",
    "xterm": "^5.3",
    "xterm-addon-fit": "^0.8",
    "lucide-react": "^0.451",
    "class-variance-authority": "^0.7",
    "clsx": "^2.1",
    "tailwind-merge": "^2.5"
  },
  "devDependencies": {
    "typescript": "^5.6",
    "tailwindcss": "^3.4",
    "@types/node": "^22",
    "@types/react": "^18",
    "@types/react-dom": "^18",
    "autoprefixer": "^10.4",
    "postcss": "^8.4"
  }
}
```

## 编译命令

```bash
# 默认（SQLite）
cargo build --release

# 切到 PostgreSQL + TimescaleDB
cargo build --release --no-default-features --features storage-postgres

# 启用 NVIDIA GPU 监控
cargo build --release -p xlstatus-agent --features gpu-nvidia

# 全部 workspace 检查
cargo check --all-targets --all-features

# 格式化与 lint
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# 仅 server 增量
cargo build -p xlstatus-server
```

## Feature Flag 矩阵

| 场景 | 命令 |
|------|------|
| 开发期默认（SQLite，无 GPU） | `cargo run -p xlstatus-server` |
| 生产（PG + TimescaleDB） | `cargo run -p xlstatus-server --no-default-features --features storage-postgres` |
| Agent + NVIDIA GPU 监控 | `cargo build -p xlstatus-agent --features gpu-nvidia` |
| Server + 全部特性 | `cargo build -p xlstatus-server --all-features` |