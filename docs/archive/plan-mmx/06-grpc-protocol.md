---
title: gRPC 协议设计
status: stable
audience: [human, agent]
related_milestones: [M0, M2]
---

# 06. gRPC 协议设计

Agent ↔ Server 通信协议，HTTP/2 + proto binary + TLS，1 连接多流复用。

## proto 文件结构

```
proto/
└── xlstatus/v1/
    ├── common.proto         # 共享类型
    └── agent.proto          # AgentService 定义
```

## `proto/xlstatus/v1/common.proto`

```proto
syntax = "proto3";
package xlstatus.v1;

import "google/protobuf/timestamp.proto";

message HostInfo {
  string platform = 1;
  string platform_version = 2;
  string arch = 3;
  repeated string cpu = 4;
  uint64 mem_total = 5;
  uint64 disk_total = 6;
  uint64 swap_total = 7;
  string virtualization = 8;
  uint64 boot_time_unix = 9;
  string agent_version = 10;
  repeated string gpu = 11;
}

message StateSample {
  google.protobuf.Timestamp server_ts = 1;   // 服务端写入时打时间戳
  uint64 agent_ts_ms = 2;                     // agent 端原始时间戳
  double cpu = 3;                             // 0.0 ~ 1.0
  uint64 mem_used = 4;
  uint64 swap_used = 5;
  uint64 disk_used = 6;
  uint64 net_in_speed = 7;                    // bytes/s
  uint64 net_out_speed = 8;
  uint64 net_in_transfer = 9;                 // bytes total
  uint64 net_out_transfer = 10;
  uint64 uptime_s = 11;
  double load1 = 12;
  double load5 = 13;
  double load15 = 14;
  uint64 tcp_conn_count = 15;
  uint64 udp_conn_count = 16;
  uint64 process_count = 17;
  repeated SensorTemperature temperatures = 18;
  repeated double gpu = 19;
}

message SensorTemperature {
  string name = 1;
  double temperature = 2;
}

enum TaskKind {
  TASK_KIND_UNSPECIFIED = 0;
  HTTP = 1;
  TCP = 2;
  PING = 3;
  SSL = 4;
}

message TaskSpec {
  string task_id = 1;                         // UUID v7 string
  TaskKind kind = 2;
  string target = 3;
  uint32 interval_s = 4;
  uint32 timeout_s = 5;
}

message TaskResult {
  string task_id = 1;
  google.protobuf.Timestamp ts = 2;
  bool successful = 3;
  double delay_ms = 4;
  string data = 5;
}
```

## `proto/xlstatus/v1/agent.proto`

```proto
syntax = "proto3";
package xlstatus.v1;

import "google/protobuf/timestamp.proto";
import "xlstatus/v1/common.proto";

service AgentService {
  // 双向流：agent 持续推 state / host_info / task_result / ping / jwt_refresh，
  // server 注入 task / config_update / pong / force_disconnect / jwt_challenge
  rpc Session(stream ClientMessage) returns (stream ServerMessage);
}

// ============ Agent → Server ============
message ClientMessage {
  oneof body {
    Hello hello = 1;
    HostInfoUpdate host_info = 2;
    StateSample state = 3;
    BatchStateUpdate batch_state = 4;         // 离线补传
    TaskResult task_result = 5;
    Ping ping = 6;
    JwtRefreshRequest jwt_refresh = 7;
  }
}

message Hello {
  string agent_id = 1;                        // UUID
  string agent_version = 2;
  uint32 protocol_version = 3;                // 当前固定 1
  string agent_name = 4;                      // 自报名
}

message HostInfoUpdate { HostInfo info = 1; }

message BatchStateUpdate {
  repeated StateSample samples = 1;
}

message Ping { google.protobuf.Timestamp ts = 1; }

message JwtRefreshRequest {
  string nonce = 1;                            // 服务端上轮 challenge 的 nonce
  bytes signature = 2;                         // ed25519_sign(priv, sha256("xlstatus-jwt-refresh-v1:" + nonce))
}

// ============ Server → Agent ============
message ServerMessage {
  oneof body {
    HelloAck hello_ack = 1;
    TaskSpec task = 2;
    TaskCancellation task_cancel = 3;
    ConfigUpdate config = 4;
    Pong pong = 5;
    JwtChallenge jwt_challenge = 6;
    ForceDisconnect force_disconnect = 7;
  }
}

message HelloAck {
  string session_id = 1;                      // server 分配
  google.protobuf.Timestamp server_time = 2;
  uint32 sample_interval_s = 3;               // server 推荐的采样间隔
  repeated TaskSpec initial_tasks = 4;        // 握手时一次推下去
}

message TaskCancellation { string task_id = 1; }

message ConfigUpdate {
  uint32 sample_interval_s = 1;
}

message Pong { google.protobuf.Timestamp ts = 1; }

message JwtChallenge {
  string nonce = 1;
  google.protobuf.Timestamp expires_at = 2;
}

message ForceDisconnect {
  enum Reason {
    REASON_UNSPECIFIED = 0;
    AGENT_REVOKED = 1;
    SERVER_SHUTDOWN = 2;
    PROTOCOL_MISMATCH = 3;
  }
  Reason reason = 1;
  string message = 2;
}
```

## 编译生成

### `crates/proto-gen/build.rs`

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = "../../proto";
    println!("cargo:rerun-if-changed={proto_dir}");
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "xlstatus/v1/common.proto",
                "xlstatus/v1/agent.proto",
            ],
            &[proto_dir],
        )?;
    Ok(())
}
```

### `crates/proto-gen/src/lib.rs`

```rust
pub use prost;
pub use tonic;

pub mod xlstatus {
    pub mod v1 {
        tonic::include_proto!("xlstatus.v1");
    }
}
```

## 拦截器（Server 端鉴权）

### `crates/server/src/grpc_server/interceptor.rs`

```rust
use tonic::{Request, Status, service::Interceptor};
use crate::auth::agent_jwt;
use xlstatus_shared::ids::AgentId;
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentAuthInterceptor {
    pub jwt_validator: Arc<agent_jwt::Validator>,
}

impl Interceptor for AgentAuthInterceptor {
    fn call(&mut self, req: Request<()>) -> Result<Request<()>, Status> {
        let meta = req.metadata();

        // 1. 提取 JWT
        let auth = meta.get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| Status::unauthenticated("missing bearer token"))?;

        // 2. 校验（验签 + 过期 + 黑名单）
        let claims = self.jwt_validator
            .verify(auth)
            .map_err(|e| Status::unauthenticated(format!("invalid token: {e}")))?;

        // 3. 注入 agent_id 到 request extensions
        let mut req = req;
        let agent_id: AgentId = claims.sub.parse()
            .map_err(|_| Status::unauthenticated("invalid agent_id"))?;
        req.extensions_mut().insert(agent_id);

        Ok(req)
    }
}
```

## Service 实现骨架

### `crates/server/src/grpc_server/service.rs`

```rust
use tonic::{Request, Response, Status, Streaming};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use futures::StreamExt;
use xlstatus_proto_gen::xlstatus::v1::*;

type SessionStream = Pin<Box<dyn Stream<Item = Result<ServerMessage, Status>> + Send>>;

pub struct AgentServiceImpl {
    pub session_registry: Arc<SessionRegistry>,
    pub store: Arc<dyn AgentStore>,
    pub sample_buffer: Arc<SampleBatchBuffer>,
    pub agent_sessions: Arc<AgentSessionManager>,
    pub task_dispatcher: Arc<TaskDispatcher>,
    pub ws_hub: Arc<WsHub>,
}

#[tonic::async_trait]
impl agent_service_server::AgentService for AgentServiceImpl {
    async fn session(
        &self,
        request: Request<Streaming<ClientMessage>>,
    ) -> Result<Response<SessionStream>, Status> {
        let agent_id = request.extensions().get::<AgentId>().cloned()
            .ok_or_else(|| Status::unauthenticated("no agent_id"))?;
        let mut inbound = request.into_inner();
        let (tx, rx) = mpsc::channel(64);

        // 1. 校验 agent 状态
        let agent = self.store.get_agent(agent_id).await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        if agent.revoked {
            return Err(Status::permission_denied("agent revoked"));
        }

        // 2. 抢占单连接（踢旧）
        self.session_registry.register(agent_id, tx.clone()).await;

        // 3. 发送 hello_ack
        let hello_ack = ServerMessage {
            body: Some(server_message::Body::HelloAck(HelloAck {
                session_id: uuid::Uuid::now_v7().to_string(),
                server_time: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                sample_interval_s: 10,
                initial_tasks: self.task_dispatcher.initial_tasks(agent_id).await,
            })),
        };
        tx.send(Ok(hello_ack)).await
            .map_err(|e| Status::internal(e.to_string()))?;

        // 4. spawn 接收循环
        let svc = self.clone();
        tokio::spawn(async move {
            while let Some(msg) = inbound.next().await {
                match msg {
                    Ok(m) => {
                        if let Err(e) = svc.dispatch(agent_id, m, &tx).await {
                            tracing::error!(?e, "dispatch error");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?e, "stream error");
                        break;
                    }
                }
            }
            svc.session_registry.unregister(agent_id).await;
        });

        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx)
        )))
    }
}
```

## 端口与服务

```
server 启动后：
  - tonic :50051（HTTP/2，TLS 在生产通过反代终结）
  - axum  :8080（HTTP/1.1，REST + WS）
  - tonic_reflection :50051（共用 50051 端口，grpcurl 调试）
```

```rust
// crates/server/src/main.rs 片段
let grpc_addr = "[::]:50051".parse()?;
let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
health_reporter.set_serving::<AgentServiceServer<AgentServiceImpl>>().await;

let reflection = tonic_reflection::server::Builder::configure()
    .register_encoded_file_descriptor_set(xlstatus_proto_gen::FILE_DESCRIPTOR_SET)
    .build_v1()?;

Server::builder()
    .add_service(health_service)
    .add_service(reflection)
    .add_service(AgentServiceServer::with_interceptor(
        AgentServiceImpl::new(/* ... */),
        AgentAuthInterceptor { jwt_validator: Arc::new(validator) },
    ))
    .serve(grpc_addr)
    .await?;
```

## 性能要点

- **多路复用**：1 个 HTTP/2 连接跑完所有 stream（state / task / config），无需连接池
- **二进制 proto**：比 JSON 小 50–70%，序列化快 5–10×
- **背压**：tonic 自动根据下游消费速度限流，agent 端 `reporter.rs` 配合 `mpsc(64)` 容量
- **压缩**：可选 `tonic::codec::CompressionEncoding::Gzip`（v1 默认不开）
- **拦截器**只做连接级校验（JWT metadata），业务消息级校验在 service 内逐条处理

## M0 验收（gRPC 部分）

```bash
# 启动 server
cargo run -p xlstatus-server &
SERVER_PID=$!

# 验证 gRPC 反射
grpcurl -plaintext localhost:50051 list
# 期望输出：xlstatus.v1.AgentService, grpc.reflection.v1.ServerReflection, grpc.health.v1.Health

grpcurl -plaintext localhost:50051 describe xlstatus.v1.AgentService
# 期望：包含 Session 方法的描述

# 健康检查
grpcurl -plaintext localhost:50051 grpc.health.v1.Health/Check \
    '{"service": "xlstatus.v1.AgentService"}'
# 期望：{"status": "SERVING"}

# 清理
kill $SERVER_PID
```

## M2 验收（完整闭环）

```bash
# 1. 创建 enrollment token（admin 调 REST API）
curl -X POST http://localhost:8080/api/v1/agents \
    -H "Cookie: access_token=..." \
    -H "Content-Type: application/json" \
    -d '{"name":"devbox"}'
# 拿 agent_id

curl -X POST http://localhost:8080/api/v1/agents/$AGENT_ID/enrollment-token \
    -H "Cookie: access_token=..." \
    -H "Content-Type: application/json" \
    -d '{"name":"devbox-enroll"}'
# 拿 token

# 2. Agent 注册
cargo run -p xlstatus-agent -- enroll \
    --server http://localhost:8080 \
    --token $TOKEN \
    --name devbox
# 输出应显示：enrolled, agent_id stored at /var/lib/xlstatus/agent.key

# 3. Agent 启动
cargo run -p xlstatus-agent -- run \
    --server https://localhost:8443

# 4. 验证 server 端
psql -c "SELECT id, name, last_seen_at FROM agents;"
# 期望：1 行，last_seen_at 在 10s 内

grpcurl -plaintext localhost:50051 list
# 期望：仍能看到 AgentService

# 5. 用 grpcurl 注入测试 state
grpcurl -plaintext -d '{
  "agent_id": "'$AGENT_ID'",
  "agent_version": "0.1.0",
  "protocol_version": 1
}' localhost:50051 xlstatus.v1.AgentService/Session
# 期望：返回 hello_ack 后持续接收
```