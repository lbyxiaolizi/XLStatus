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

const AGENT_TELEMETRY_MAX_ITEMS: usize = 64;
const AGENT_TELEMETRY_MAX_STRING_BYTES: usize = 128;
const AGENT_TELEMETRY_MAX_JSON_BYTES: usize = 256 * 1024;

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
    let mut telemetry_truncated = false;
    telemetry_truncated |= s.disks.len() > AGENT_TELEMETRY_MAX_ITEMS;
    telemetry_truncated |= s.net_io.len() > AGENT_TELEMETRY_MAX_ITEMS;
    telemetry_truncated |= s.temperatures.len() > AGENT_TELEMETRY_MAX_ITEMS;
    telemetry_truncated |= !s.gpus.is_empty();

    let mut disks = Vec::with_capacity(s.disks.len().min(AGENT_TELEMETRY_MAX_ITEMS));
    for d in s.disks.iter().take(AGENT_TELEMETRY_MAX_ITEMS) {
        disks.push(json!({
            "mount_point": bounded_telemetry_string(&d.mount_point, &mut telemetry_truncated),
            "used": d.used,
            "total": d.total
        }));
    }

    let mut net_io = Vec::with_capacity(s.net_io.len().min(AGENT_TELEMETRY_MAX_ITEMS));
    let mut network_in_total = 0_u64;
    let mut network_out_total = 0_u64;
    for n in s.net_io.iter().take(AGENT_TELEMETRY_MAX_ITEMS) {
        network_in_total = network_in_total.saturating_add(n.bytes_recv);
        network_out_total = network_out_total.saturating_add(n.bytes_sent);
        net_io.push(json!({
            "interface": bounded_telemetry_string(&n.interface, &mut telemetry_truncated),
            "bytes_sent": n.bytes_sent,
            "bytes_recv": n.bytes_recv
        }));
    }

    let mut temperatures = Vec::with_capacity(s.temperatures.len().min(AGENT_TELEMETRY_MAX_ITEMS));
    for t in s.temperatures.iter().take(AGENT_TELEMETRY_MAX_ITEMS) {
        temperatures.push(json!({
            "label": bounded_telemetry_string(&t.label, &mut telemetry_truncated),
            "temperature": t.temperature
        }));
    }

    let value = json!({
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
        "network_in_total": network_in_total,
        "network_out_total": network_out_total,
        "uptime_seconds": s.uptime_seconds,
        "temperatures": temperatures,
        "telemetry_truncated": telemetry_truncated,
    });
    serialize_telemetry_json(value, || {
        json!({
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
            "network_in_total": network_in_total,
            "network_out_total": network_out_total,
            "uptime_seconds": s.uptime_seconds,
            "disks": [],
            "net_io": [],
            "temperatures": [],
            "telemetry_truncated": true,
        })
    })
}

fn info_to_json(info: &xlstatus_proto_gen::xlstatus::v1::HostInfoUpdate) -> String {
    use serde_json::json;
    let h = match info.host_info.as_ref() {
        Some(h) => h,
        None => return String::new(),
    };
    let mut telemetry_truncated = h.disks.len() > AGENT_TELEMETRY_MAX_ITEMS;
    let mut disks = Vec::with_capacity(h.disks.len().min(AGENT_TELEMETRY_MAX_ITEMS));
    for d in h.disks.iter().take(AGENT_TELEMETRY_MAX_ITEMS) {
        disks.push(json!({
            "device": bounded_telemetry_string(&d.device, &mut telemetry_truncated),
            "mount_point": bounded_telemetry_string(&d.mount_point, &mut telemetry_truncated),
            "fs_type": bounded_telemetry_string(&d.fs_type, &mut telemetry_truncated),
            "total": d.total,
        }));
    }
    let value = json!({
        "hostname": bounded_telemetry_string(&h.hostname, &mut telemetry_truncated),
        "os": bounded_telemetry_string(&h.os, &mut telemetry_truncated),
        "platform": bounded_telemetry_string(&h.platform, &mut telemetry_truncated),
        "platform_version": bounded_telemetry_string(&h.platform_version, &mut telemetry_truncated),
        "kernel_version": bounded_telemetry_string(&h.kernel_version, &mut telemetry_truncated),
        "arch": bounded_telemetry_string(&h.arch, &mut telemetry_truncated),
        "cpu_cores": h.cpu_cores,
        "total_memory": h.total_memory,
        "total_swap": h.total_swap,
        "disks": disks,
        "telemetry_truncated": telemetry_truncated,
    });
    serialize_telemetry_json(value, || {
        json!({
            "hostname": forced_bounded_telemetry_string(&h.hostname),
            "os": forced_bounded_telemetry_string(&h.os),
            "platform": forced_bounded_telemetry_string(&h.platform),
            "platform_version": forced_bounded_telemetry_string(&h.platform_version),
            "kernel_version": forced_bounded_telemetry_string(&h.kernel_version),
            "arch": forced_bounded_telemetry_string(&h.arch),
            "cpu_cores": h.cpu_cores,
            "total_memory": h.total_memory,
            "total_swap": h.total_swap,
            "disks": [],
            "telemetry_truncated": true,
        })
    })
}

fn bounded_telemetry_string(value: &str, telemetry_truncated: &mut bool) -> String {
    if value.len() <= AGENT_TELEMETRY_MAX_STRING_BYTES {
        return value.to_string();
    }
    *telemetry_truncated = true;
    let mut end = AGENT_TELEMETRY_MAX_STRING_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn forced_bounded_telemetry_string(value: &str) -> String {
    let mut telemetry_truncated = true;
    bounded_telemetry_string(value, &mut telemetry_truncated)
}

fn serialize_telemetry_json<F>(value: serde_json::Value, fallback: F) -> String
where
    F: FnOnce() -> serde_json::Value,
{
    let serialized = serde_json::to_string(&value).unwrap_or_else(|_| String::new());
    if serialized.len() <= AGENT_TELEMETRY_MAX_JSON_BYTES {
        return serialized;
    }

    let fallback = serde_json::to_string(&fallback()).unwrap_or_else(|_| String::new());
    if fallback.len() <= AGENT_TELEMETRY_MAX_JSON_BYTES {
        fallback
    } else {
        "{\"telemetry_truncated\":true}".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_proto_gen::xlstatus::v1::{
        DiskInfo, DiskState, GpuState, HostInfo, HostInfoUpdate, HostState, NetStat, TempSensor,
    };

    fn parse_json(value: &str) -> serde_json::Value {
        serde_json::from_str(value).expect("telemetry JSON should parse")
    }

    #[test]
    fn host_state_json_bounds_repeated_and_string_fields() {
        let long_ascii = "x".repeat(AGENT_TELEMETRY_MAX_STRING_BYTES + 64);
        let mut state = HostState {
            cpu_percent: 42.0,
            memory_used: 1024,
            memory_total: 2048,
            swap_used: 1,
            swap_total: 2,
            load_1: 0.1,
            load_5: 0.2,
            load_15: 0.3,
            tcp_connections: 4,
            udp_connections: 5,
            process_count: 6,
            uptime_seconds: 7,
            ..Default::default()
        };
        state.disks = (0..AGENT_TELEMETRY_MAX_ITEMS + 8)
            .map(|index| DiskState {
                mount_point: format!("{long_ascii}-{index}"),
                used: index as u64,
                total: 10_000 + index as u64,
            })
            .collect();
        state.net_io = (0..AGENT_TELEMETRY_MAX_ITEMS + 8)
            .map(|index| NetStat {
                interface: format!("{long_ascii}-{index}"),
                bytes_sent: 100 + index as u64,
                bytes_recv: 200 + index as u64,
            })
            .collect();
        state.temperatures = (0..AGENT_TELEMETRY_MAX_ITEMS + 8)
            .map(|index| TempSensor {
                label: format!("{long_ascii}-{index}"),
                temperature: 40.0 + index as f64,
            })
            .collect();
        state.gpus = vec![GpuState {
            index: 0,
            name: long_ascii.clone(),
            utilization: 99.0,
            memory_used: 1,
            memory_total: 2,
            temperature: 70.0,
        }];

        let json = state_to_json(&state);
        assert!(json.len() <= AGENT_TELEMETRY_MAX_JSON_BYTES);
        let parsed = parse_json(&json);
        assert_eq!(parsed["telemetry_truncated"], true);
        assert_eq!(
            parsed["disks"].as_array().unwrap().len(),
            AGENT_TELEMETRY_MAX_ITEMS
        );
        assert_eq!(
            parsed["net_io"].as_array().unwrap().len(),
            AGENT_TELEMETRY_MAX_ITEMS
        );
        assert_eq!(
            parsed["temperatures"].as_array().unwrap().len(),
            AGENT_TELEMETRY_MAX_ITEMS
        );
        assert_eq!(
            parsed["disks"][0]["mount_point"].as_str().unwrap().len(),
            AGENT_TELEMETRY_MAX_STRING_BYTES
        );
        assert_eq!(
            parsed["net_io"][0]["interface"].as_str().unwrap().len(),
            AGENT_TELEMETRY_MAX_STRING_BYTES
        );
        assert_eq!(
            parsed["temperatures"][0]["label"].as_str().unwrap().len(),
            AGENT_TELEMETRY_MAX_STRING_BYTES
        );
        assert!(parsed.get("gpus").is_none());
    }

    #[test]
    fn host_info_json_bounds_repeated_and_utf8_strings() {
        let long_utf8 = "界".repeat(AGENT_TELEMETRY_MAX_STRING_BYTES);
        let info = HostInfoUpdate {
            host_info: Some(HostInfo {
                hostname: long_utf8.clone(),
                os: long_utf8.clone(),
                platform: long_utf8.clone(),
                platform_version: long_utf8.clone(),
                kernel_version: long_utf8.clone(),
                arch: long_utf8.clone(),
                cpu_cores: 8,
                total_memory: 16,
                total_swap: 32,
                disks: (0..AGENT_TELEMETRY_MAX_ITEMS + 8)
                    .map(|index| DiskInfo {
                        device: format!("{long_utf8}-{index}"),
                        mount_point: format!("{long_utf8}-{index}"),
                        fs_type: format!("{long_utf8}-{index}"),
                        total: 1_000 + index as u64,
                    })
                    .collect(),
            }),
        };

        let json = info_to_json(&info);
        assert!(json.len() <= AGENT_TELEMETRY_MAX_JSON_BYTES);
        let parsed = parse_json(&json);
        assert_eq!(parsed["telemetry_truncated"], true);
        assert_eq!(
            parsed["disks"].as_array().unwrap().len(),
            AGENT_TELEMETRY_MAX_ITEMS
        );
        assert!(parsed["hostname"]
            .as_str()
            .unwrap()
            .is_char_boundary(parsed["hostname"].as_str().unwrap().len()));
        assert!(parsed["hostname"].as_str().unwrap().len() <= AGENT_TELEMETRY_MAX_STRING_BYTES);
        assert!(
            parsed["disks"][0]["device"].as_str().unwrap().len()
                <= AGENT_TELEMETRY_MAX_STRING_BYTES
        );
    }

    #[test]
    fn host_info_without_payload_remains_empty() {
        assert_eq!(info_to_json(&HostInfoUpdate { host_info: None }), "");
    }
}
