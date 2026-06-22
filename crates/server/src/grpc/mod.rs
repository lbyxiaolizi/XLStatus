mod session;

pub use session::{
    base64_encoded_len, ensure_task_result_text_within, truncate_task_result_text, IoRegistry,
    SessionRegistry, TaskResponseRegistry,
};

use crate::api::v1::auth::{active_waf_ban, record_waf_event, register_agent_auth_failure};
use crate::auth::verify_agent_jwt;
use crate::db::{AgentRepository, DatabaseBackend};
use crate::realtime::{BroadcastHub, RealtimeEvent};
use chrono::Utc;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use xlstatus_proto_gen::xlstatus::v1::agent_service_server::AgentService;
use xlstatus_proto_gen::xlstatus::v1::{agent_message, AgentMessage, IoFrame, ServerMessage};
use xlstatus_shared::AgentId;

pub struct AgentServiceImpl {
    db: DatabaseBackend,
    session_registry: SessionRegistry,
    jwt_secret: String,
    /// M3: time-series store for HostState samples.
    metrics: xlstatus_tsdb::MetricStore,
    /// M3: live event hub fed to the WebSocket route.
    realtime: BroadcastHub,
    io_registry: IoRegistry,
}

impl AgentServiceImpl {
    pub fn new(
        db: DatabaseBackend,
        session_registry: SessionRegistry,
        jwt_secret: String,
        metrics: xlstatus_tsdb::MetricStore,
        realtime: BroadcastHub,
        io_registry: IoRegistry,
    ) -> Self {
        Self {
            db,
            session_registry,
            jwt_secret,
            metrics,
            realtime,
            io_registry,
        }
    }
}

#[tonic::async_trait]
impl AgentService for AgentServiceImpl {
    type SessionStream = ReceiverStream<Result<ServerMessage, Status>>;
    type IoStreamStream = ReceiverStream<Result<IoFrame, Status>>;

    async fn session(
        &self,
        request: Request<tonic::Streaming<AgentMessage>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let client_ip = client_ip_from_grpc_request(&request);
        let bearer_token = bearer_token_from_grpc_request(&request).map(ToString::to_string);
        let agent_id = authenticate_agent_request(
            &self.db,
            &self.jwt_secret,
            client_ip,
            bearer_token,
            "session",
        )
        .await?;

        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        // Register session
        self.session_registry.register(agent_id, tx.clone()).await;

        let db = self.db.clone();
        let session_registry = self.session_registry.clone();
        let metrics = self.metrics.clone();
        let realtime = self.realtime.clone();

        // Spawn task to handle incoming messages
        tokio::spawn(async move {
            while let Ok(Some(msg)) = in_stream.message().await {
                match msg.payload {
                    Some(agent_message::Payload::Heartbeat(_heartbeat)) => {
                        tracing::debug!("Heartbeat from agent {}", agent_id);

                        // Update last_seen_at
                        let agent_repo = AgentRepository::new(db.clone());
                        if let Err(e) = agent_repo.update_last_seen(agent_id).await {
                            tracing::error!("Failed to update last_seen: {}", e);
                        }
                    }
                    Some(agent_message::Payload::HostState(state)) => {
                        tracing::debug!(
                            "Host state from agent {}: CPU={:.1}%, Mem={}/{}",
                            agent_id,
                            state.cpu_percent,
                            state.memory_used,
                            state.memory_total
                        );
                        // Snapshot for the M3 persistence below.
                        let state_summary = state.clone();

                        // M3: persist the latest HostState JSON. Full TSDB
                        // writes land in M8 per plan/08-roadmap.md.
                        let state_json = state_to_json(&state_summary);
                        let agent_repo2 = AgentRepository::new(db.clone());
                        if let Err(e) = agent_repo2.update_last_state(agent_id, &state_json).await {
                            tracing::warn!("update_last_state failed: {}", e);
                        }
                        // Mirror the same JSON into the in-memory TSDB so
                        // /api/v1/servers/:id/metrics has something to
                        // return. Failures are non-fatal; the column in
                        // `agents` is the durable source of truth.
                        if let Ok(value) = serde_json::from_str::<JsonValue>(&state_json) {
                            if let Err(e) =
                                metrics.write_json(agent_id.0, Utc::now(), value.clone())
                            {
                                tracing::warn!("tsdb write failed: {}", e);
                            }
                            // M4: feed the alert engine's in-memory
                            // snapshot so ServerResource / ServerOffline
                            // conditions see the latest value on the
                            // next tick.
                            if let Some(engine) = crate::current_alert_engine() {
                                engine
                                    .observe_agent_state(&agent_id.0.to_string(), value.clone())
                                    .await;
                            }
                            // Fan out to any WebSocket subscribers.
                            realtime.publish(RealtimeEvent::new("host_state", agent_id.0, value));
                        }
                    }
                    Some(agent_message::Payload::TaskResult(result)) => {
                        tracing::debug!("Task result from agent {}: {}", agent_id, result.task_id);
                        // M5: forward the result to the waiting HTTP
                        // run_task handler (best-effort). The handler
                        // also persists the row to `task_runs`.
                        crate::current_task_response_registry()
                            .deliver(&result.task_id, result.clone())
                            .await;
                    }
                    Some(agent_message::Payload::HostInfoUpdate(info_msg)) => {
                        tracing::debug!("Host info update from agent {}", agent_id);
                        let info_json = info_to_json(&info_msg);
                        let agent_repo2 = AgentRepository::new(db.clone());
                        if let Err(e) = agent_repo2.update_last_info(agent_id, &info_json).await {
                            tracing::warn!("update_last_info failed: {}", e);
                        }
                        if let Ok(value) = serde_json::from_str::<JsonValue>(&info_json) {
                            realtime.publish(RealtimeEvent::new("host_info", agent_id.0, value));
                        }
                    }
                    Some(agent_message::Payload::GeoIpReport(report)) => {
                        tracing::debug!(
                            "GeoIP report from agent {}: ipv4={}, ipv6={}",
                            agent_id,
                            report.ipv4,
                            report.ipv6
                        );
                        if let Some(manager) = crate::current_ddns_manager() {
                            if let Err(e) = manager
                                .check_agent_ip_report(
                                    &agent_id.0.to_string(),
                                    if report.ipv4.trim().is_empty() {
                                        None
                                    } else {
                                        Some(report.ipv4.as_str())
                                    },
                                    if report.ipv6.trim().is_empty() {
                                        None
                                    } else {
                                        Some(report.ipv6.as_str())
                                    },
                                )
                                .await
                            {
                                tracing::warn!("DDNS agent IP check failed: {}", e);
                            }
                        }
                        if let Err(e) = crate::api::v1::geoip::handle_agent_ip_report(
                            &db,
                            agent_id,
                            if report.ipv4.trim().is_empty() {
                                None
                            } else {
                                Some(report.ipv4.as_str())
                            },
                            if report.ipv6.trim().is_empty() {
                                None
                            } else {
                                Some(report.ipv6.as_str())
                            },
                        )
                        .await
                        {
                            tracing::warn!("Agent IP change handling failed: {}", e);
                        }
                        realtime.publish(RealtimeEvent::new(
                            "geo_ip",
                            agent_id.0,
                            serde_json::json!({
                                "ipv4": report.ipv4,
                                "ipv6": report.ipv6,
                            }),
                        ));
                    }
                    None => {}
                }
            }

            // Unregister when stream ends
            session_registry.unregister(&agent_id).await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn io_stream(
        &self,
        request: Request<tonic::Streaming<IoFrame>>,
    ) -> Result<Response<Self::IoStreamStream>, Status> {
        let client_ip = client_ip_from_grpc_request(&request);
        let bearer_token = bearer_token_from_grpc_request(&request).map(ToString::to_string);
        let agent_id = authenticate_agent_request(
            &self.db,
            &self.jwt_secret,
            client_ip,
            bearer_token,
            "io_stream",
        )
        .await?;

        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);
        self.io_registry.register_agent(agent_id, tx.clone()).await;
        let io_registry = self.io_registry.clone();

        // Spawn task to handle IO frames
        tokio::spawn(async move {
            while let Ok(Some(frame)) = in_stream.message().await {
                tracing::debug!(
                    "IO frame stream_id={}, seq={}",
                    frame.stream_id,
                    frame.sequence
                );
                let _ = io_registry.deliver_from_agent(frame).await;
            }
            io_registry.unregister_agent(&agent_id).await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

async fn authenticate_agent_request(
    db: &DatabaseBackend,
    jwt_secret: &str,
    client_ip: String,
    bearer_token: Option<String>,
    stream_name: &str,
) -> Result<AgentId, Status> {
    if active_waf_ban(db, &client_ip)
        .await
        .ok()
        .flatten()
        .is_some()
    {
        let _ = record_waf_event(
            db,
            &client_ip,
            Some(stream_name),
            "agent_auth_blocked",
            Some("active WAF ban"),
        )
        .await;
        return Err(Status::permission_denied("IP temporarily blocked by WAF"));
    }

    let Some(token) = bearer_token else {
        let _ =
            register_agent_auth_failure(db, &client_ip, Some(stream_name), "missing bearer token")
                .await;
        return Err(Status::unauthenticated("missing bearer token"));
    };
    let claims = match verify_agent_jwt(&token, jwt_secret) {
        Ok(claims) => claims,
        Err(_) => {
            let _ = register_agent_auth_failure(
                db,
                &client_ip,
                Some(stream_name),
                "invalid bearer token",
            )
            .await;
            return Err(Status::unauthenticated("invalid bearer token"));
        }
    };
    let agent_id = match uuid::Uuid::parse_str(&claims.sub).map(AgentId) {
        Ok(agent_id) => agent_id,
        Err(_) => {
            let _ =
                register_agent_auth_failure(db, &client_ip, Some(&claims.sub), "invalid agent id")
                    .await;
            return Err(Status::unauthenticated("invalid agent id"));
        }
    };

    let agent_repo = AgentRepository::new(db.clone());
    let agent = match agent_repo.find_by_id(agent_id).await {
        Ok(Some(agent)) => agent,
        Ok(None) => {
            let agent_ref = agent_id.0.to_string();
            let _ =
                register_agent_auth_failure(db, &client_ip, Some(&agent_ref), "agent not found")
                    .await;
            return Err(Status::unauthenticated("agent not found"));
        }
        Err(err) => {
            tracing::warn!("failed to load agent during gRPC auth: {}", err);
            return Err(Status::internal("failed to load agent"));
        }
    };
    if agent.revoked_at.is_some() {
        let agent_ref = agent_id.0.to_string();
        let _ =
            register_agent_auth_failure(db, &client_ip, Some(&agent_ref), "agent revoked").await;
        return Err(Status::permission_denied("agent revoked"));
    }

    Ok(agent_id)
}

fn bearer_token_from_grpc_request<T>(request: &Request<T>) -> Option<&str> {
    request
        .metadata()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("bearer ")
                .or_else(|| value.strip_prefix("Bearer "))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn client_ip_from_grpc_request<T>(request: &Request<T>) -> String {
    crate::security::forwarded_client_ip_with_peer(
        |name| grpc_metadata_value(request, name),
        request.remote_addr().map(|addr| addr.ip().to_string()),
        request.remote_addr().map(|addr| addr.ip()),
    )
}

fn grpc_metadata_value<T>(request: &Request<T>, name: &str) -> Option<String> {
    request
        .metadata()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// M3: hand-rolled JSON serializer for the gRPC HostState message.
/// We avoid pulling `prost` types into `serde` by formatting the few
/// scalar fields we care about. The full TSDB (crates/tsdb) replaces
/// this with a typed schema in M8 per plan/08-roadmap.md.
fn state_to_json(s: &xlstatus_proto_gen::xlstatus::v1::HostState) -> String {
    use serde_json::json;
    let disks: Vec<serde_json::Value> = s
        .disks
        .iter()
        .map(|d| json!({ "mount_point": d.mount_point, "used": d.used, "total": d.total }))
        .collect();
    let net_io: Vec<serde_json::Value> = s
        .net_io
        .iter()
        .map(|n| json!({ "interface": n.interface, "bytes_sent": n.bytes_sent, "bytes_recv": n.bytes_recv }))
        .collect();
    let temperatures: Vec<serde_json::Value> = s
        .temperatures
        .iter()
        .map(|t| json!({ "label": t.label, "temperature": t.temperature }))
        .collect();
    serde_json::to_string(&json!({
        "cpu_percent": s.cpu_percent,
        "memory_used": s.memory_used,
        "memory_total": s.memory_total,
        "swap_used": s.swap_used,
        "swap_total": s.swap_total,
        "load_1": s.load_1,
        "load_5": s.load_5,
        "load_15": s.load_15,
        "tcp_connections": s.tcp_connections,
        "udp_connections": s.udp_connections,
        "process_count": s.process_count,
        "disks": disks,
        "net_io": net_io,
        "network_in_total": s.net_io.iter().map(|n| n.bytes_recv).sum::<u64>(),
        "network_out_total": s.net_io.iter().map(|n| n.bytes_sent).sum::<u64>(),
        "uptime_seconds": s.uptime_seconds,
        "temperatures": temperatures,
    }))
    .unwrap_or_else(|_| String::new())
}

fn info_to_json(info: &xlstatus_proto_gen::xlstatus::v1::HostInfoUpdate) -> String {
    use serde_json::json;
    let h = match info.host_info.as_ref() {
        Some(h) => h,
        None => return String::new(),
    };
    let disks: Vec<serde_json::Value> = h
        .disks
        .iter()
        .map(|d| {
            json!({
                "device": d.device,
                "mount_point": d.mount_point,
                "fs_type": d.fs_type,
                "total": d.total,
            })
        })
        .collect();
    serde_json::to_string(&json!({
        "hostname": h.hostname,
        "os": h.os,
        "platform": h.platform,
        "platform_version": h.platform_version,
        "kernel_version": h.kernel_version,
        "arch": h.arch,
        "cpu_cores": h.cpu_cores,
        "total_memory": h.total_memory,
        "total_swap": h.total_swap,
        "disks": disks,
    }))
    .unwrap_or_else(|_| String::new())
}
