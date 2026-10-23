#![allow(dead_code)]

use anyhow::Context;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpStream};
use tokio::sync::{mpsc, Mutex};
use xlstatus_proto_gen::xlstatus::v1::{io_frame, IoClose, IoData, IoError, IoFrame};
use xlstatus_shared::nat::NatTunnelControlMessage;

pub const MAX_AGENT_NAT_TUNNELS: usize = 64;
pub const MAX_NAT_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_NAT_HOST_BYTES: usize = 253;
pub const MAX_NAT_STREAM_ID_BYTES: usize = 128;
pub const NAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub type NatSessionMap = Arc<Mutex<HashMap<String, NatSocketSession>>>;

pub struct NatSocketSession {
    pub writer_tx: mpsc::Sender<Vec<u8>>,
    pub reader_task: tokio::task::JoinHandle<()>,
}

pub fn agent_max_nat_tunnels() -> usize {
    std::env::var("XLSTATUS_AGENT_MAX_NAT_TUNNELS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(256))
        .unwrap_or(MAX_AGENT_NAT_TUNNELS)
}

pub fn validate_agent_nat_stream_id(stream_id: &str) -> anyhow::Result<()> {
    let trimmed = stream_id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("NAT stream_id must not be empty");
    }
    if trimmed.len() > MAX_NAT_STREAM_ID_BYTES {
        anyhow::bail!(
            "NAT stream_id exceeds maximum length of {} bytes",
            MAX_NAT_STREAM_ID_BYTES
        );
    }
    if trimmed != stream_id || trimmed.chars().any(char::is_control) {
        anyhow::bail!("NAT stream_id contains invalid whitespace or control characters");
    }
    Ok(())
}

pub fn validate_agent_nat_frame_bytes(payload: &[u8]) -> anyhow::Result<()> {
    if payload.len() > MAX_NAT_FRAME_BYTES {
        anyhow::bail!(
            "NAT frame payload ({} bytes) exceeds maximum allowed ({} bytes)",
            payload.len(),
            MAX_NAT_FRAME_BYTES
        );
    }
    Ok(())
}

pub fn reserve_agent_nat_session<T>(
    sessions: &mut HashMap<String, T>,
    stream_id: &str,
    session: T,
    max_tunnels: usize,
) -> anyhow::Result<()> {
    validate_agent_nat_stream_id(stream_id)?;
    if sessions.contains_key(stream_id) {
        anyhow::bail!("duplicate NAT tunnel stream_id '{}'", stream_id);
    }
    if sessions.len() >= max_tunnels {
        anyhow::bail!(
            "Agent concurrent NAT tunnel limit ({}) reached",
            max_tunnels
        );
    }
    sessions.insert(stream_id.to_string(), session);
    Ok(())
}

pub async fn resolve_agent_nat_target(host: &str, port: u16) -> anyhow::Result<Vec<SocketAddr>> {
    resolve_agent_nat_target_with_policy(host, port, agent_private_nat_targets_allowed()).await
}

pub async fn resolve_agent_nat_target_with_policy(
    host: &str,
    port: u16,
    allow_private: bool,
) -> anyhow::Result<Vec<SocketAddr>> {
    if port == 0 {
        anyhow::bail!("Agent NAT target port must be greater than zero");
    }
    let host = validate_agent_nat_host_input(host)?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_agent_nat_ip_allowed(ip, allow_private)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let addrs = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve Agent NAT target host '{host}'"))?;
    validate_agent_nat_resolved_addrs(addrs, port, allow_private)
}

fn validate_agent_nat_host_input(host: &str) -> anyhow::Result<&str> {
    let host = host.trim();
    if host.is_empty() {
        anyhow::bail!("Agent NAT target host must not be empty");
    }
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        anyhow::bail!("Agent NAT target host must not be empty");
    }
    if host.len() > MAX_NAT_HOST_BYTES {
        anyhow::bail!(
            "Agent NAT target host exceeds maximum length of {} bytes",
            MAX_NAT_HOST_BYTES
        );
    }
    if host
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\' | '@' | '?'))
    {
        anyhow::bail!("Agent NAT target host contains disallowed characters");
    }
    Ok(host)
}

/// Validate every resolved `SocketAddr` against the agent's local target policy
/// before any connection attempt so DNS rebinding or multi-record responses
/// containing a forbidden IP are rejected up front.
pub fn validate_agent_nat_resolved_addrs<I>(
    addrs: I,
    expected_port: u16,
    allow_private: bool,
) -> anyhow::Result<Vec<SocketAddr>>
where
    I: IntoIterator<Item = SocketAddr>,
{
    if expected_port == 0 {
        anyhow::bail!("Agent NAT target port must be greater than zero");
    }
    let mut resolved = Vec::new();
    for addr in addrs {
        if addr.port() == 0 || addr.port() != expected_port {
            anyhow::bail!(
                "Agent NAT target resolved to unexpected port {} (expected {})",
                addr.port(),
                expected_port
            );
        }
        ensure_agent_nat_ip_allowed(addr.ip(), allow_private)?;
        if !resolved.contains(&addr) {
            resolved.push(addr);
        }
    }
    if resolved.is_empty() {
        anyhow::bail!("Agent NAT target host did not resolve to any address");
    }
    Ok(resolved)
}

pub fn ensure_agent_nat_ip_allowed(ip: IpAddr, allow_private: bool) -> anyhow::Result<()> {
    let canonical_ip = match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
    };

    if is_never_allowed_nat_target_ip(canonical_ip) {
        anyhow::bail!(
            "Agent NAT target resolves to permanently forbidden address {ip} (metadata, link-local, multicast, or unspecified)"
        );
    }

    if is_loopback_nat_target_ip(canonical_ip) {
        return Ok(());
    }

    if !allow_private {
        anyhow::bail!("Agent NAT target resolves to disallowed non-loopback address {ip}");
    }

    if is_private_lan_nat_target_ip(canonical_ip) {
        return Ok(());
    }

    anyhow::bail!("Agent NAT target resolves to disallowed non-private address {ip}")
}

fn is_loopback_nat_target_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn is_private_lan_nat_target_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            // Unique Local Addresses fc00::/7 (excluding AWS EC2 IPv6 metadata fd00:ec2::254)
            (segments[0] & 0xfe00) == 0xfc00 && !is_cloud_metadata_ipv6(v6)
        }
    }
}

fn is_never_allowed_nat_target_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_never_allowed_nat_ipv4(v4),
        IpAddr::V6(v6) => is_never_allowed_nat_ipv6(v6),
    }
}

fn is_never_allowed_nat_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 169 && o[1] == 254)
        || o[0] >= 224
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (18..=19).contains(&o[1]))
}

fn is_never_allowed_nat_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || is_cloud_metadata_ipv6(ip)
}

fn is_cloud_metadata_ipv6(ip: Ipv6Addr) -> bool {
    ip.segments() == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
}

/// Connect directly to the pre-resolved, policy-validated `SocketAddr` slice.
/// Because `TcpStream::connect` receives a concrete `SocketAddr` rather than a
/// hostname, DNS rebinding between validation and socket connect is impossible.
pub async fn connect_agent_nat_addrs(addrs: &[SocketAddr]) -> anyhow::Result<TcpStream> {
    connect_agent_nat_addrs_with_policy(addrs, agent_private_nat_targets_allowed()).await
}

pub async fn connect_agent_nat_addrs_with_policy(
    addrs: &[SocketAddr],
    allow_private: bool,
) -> anyhow::Result<TcpStream> {
    if addrs.is_empty() {
        anyhow::bail!("no Agent NAT target address resolved");
    }
    for addr in addrs {
        if addr.port() == 0 {
            anyhow::bail!("Agent NAT target port must be greater than zero");
        }
        ensure_agent_nat_ip_allowed(addr.ip(), allow_private)?;
    }

    let mut last_error = None;
    for &addr in addrs {
        match tokio::time::timeout(NAT_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => {
                last_error = Some(anyhow::anyhow!("failed to connect to {addr}: {e}"));
            }
            Err(_) => {
                last_error = Some(anyhow::anyhow!("connection to {addr} timed out"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no Agent NAT target address resolved")))
}

pub async fn dial_agent_nat_target_with_policy(
    host: &str,
    port: u16,
    allow_private: bool,
) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let addrs = resolve_agent_nat_target_with_policy(host, port, allow_private).await?;
    let mut last_error = None;
    for &addr in &addrs {
        match tokio::time::timeout(NAT_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok((stream, addr)),
            Ok(Err(e)) => {
                last_error = Some(anyhow::anyhow!("failed to connect to {addr}: {e}"));
            }
            Err(_) => {
                last_error = Some(anyhow::anyhow!("connection to {addr} timed out"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no Agent NAT target address resolved")))
}

pub fn agent_private_nat_targets_allowed() -> bool {
    [
        "XLSTATUS_AGENT_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_NAT_TARGETS",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

pub fn format_nat_target_label(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub async fn handle_nat_data_frame(
    agent_id: &str,
    stream_id: &str,
    sequence: u64,
    payload: Vec<u8>,
    disable_nat: bool,
    writer_tx: mpsc::Sender<Vec<u8>>,
    nat_sessions: &NatSessionMap,
    io_tx: &mpsc::Sender<IoFrame>,
) {
    if disable_nat {
        close_nat_session(
            agent_id,
            stream_id,
            "NAT is disabled by agent policy".to_string(),
            sequence,
            nat_sessions,
            io_tx,
            false,
        )
        .await;
        let _ = io_tx
            .send(io_error_frame(
                stream_id,
                agent_id,
                sequence,
                "nat_disabled",
                "NAT is disabled by agent policy",
            ))
            .await;
        return;
    }

    if let Err(err) = validate_agent_nat_frame_bytes(&payload) {
        close_nat_session(
            agent_id,
            stream_id,
            err.to_string(),
            sequence,
            nat_sessions,
            io_tx,
            false,
        )
        .await;
        let _ = io_tx
            .send(io_error_frame(
                stream_id,
                agent_id,
                sequence,
                "nat_frame_too_large",
                &err.to_string(),
            ))
            .await;
        return;
    }

    if writer_tx.send(payload).await.is_err() {
        close_nat_session(
            agent_id,
            stream_id,
            "NAT writer channel closed".to_string(),
            sequence,
            nat_sessions,
            io_tx,
            true,
        )
        .await;
    }
}

pub async fn handle_nat_control_message(
    agent_id: &str,
    stream_id: String,
    sequence: u64,
    message: NatTunnelControlMessage,
    disable_nat: bool,
    nat_sessions: NatSessionMap,
    io_tx: mpsc::Sender<IoFrame>,
) {
    if disable_nat {
        let _ = io_tx
            .send(io_error_frame(
                &stream_id,
                agent_id,
                sequence,
                "nat_disabled",
                "NAT is disabled by agent policy",
            ))
            .await;
        return;
    }
    if let Err(err) = validate_agent_nat_stream_id(&stream_id) {
        let _ = io_tx
            .send(io_error_frame(
                &stream_id,
                agent_id,
                sequence,
                "nat_invalid_stream",
                &err.to_string(),
            ))
            .await;
        return;
    }

    match message {
        NatTunnelControlMessage::Open {
            local_host,
            local_port,
        } => {
            let max_tunnels = agent_max_nat_tunnels();
            {
                let sessions = nat_sessions.lock().await;
                if sessions.contains_key(&stream_id) {
                    let _ = io_tx
                        .send(io_error_frame(
                            &stream_id,
                            agent_id,
                            sequence,
                            "nat_duplicate_stream",
                            "NAT tunnel stream_id is already active",
                        ))
                        .await;
                    return;
                }
                if sessions.len() >= max_tunnels {
                    let _ = io_tx
                        .send(io_error_frame(
                            &stream_id,
                            agent_id,
                            sequence,
                            "nat_tunnel_limit_reached",
                            &format!(
                                "Agent concurrent NAT tunnel limit ({}) reached",
                                max_tunnels
                            ),
                        ))
                        .await;
                    return;
                }
            }

            let local_addr_label = format_nat_target_label(&local_host, local_port);
            let connect_result = async {
                let addrs = resolve_agent_nat_target(&local_host, local_port).await?;
                connect_agent_nat_addrs(&addrs).await
            }
            .await;

            match connect_result {
                Ok(stream) => {
                    let (mut reader, mut writer) = stream.into_split();
                    let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(64);
                    let reader_stream_id = stream_id.clone();
                    let reader_agent_id = agent_id.to_string();
                    let reader_io_tx = io_tx.clone();
                    let reader_nat_sessions = nat_sessions.clone();
                    let reader_task = tokio::spawn(async move {
                        let mut read_sequence = sequence.saturating_add(2);
                        let mut buf = [0u8; 8192];
                        loop {
                            match reader.read(&mut buf).await {
                                Ok(0) => {
                                    let _ = reader_io_tx
                                        .send(io_close_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            "local NAT stream closed",
                                        ))
                                        .await;
                                    let _ =
                                        reader_nat_sessions.lock().await.remove(&reader_stream_id);
                                    break;
                                }
                                Ok(n) => {
                                    if reader_io_tx
                                        .send(io_raw_data_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            buf[..n].to_vec(),
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        let _ = reader_nat_sessions
                                            .lock()
                                            .await
                                            .remove(&reader_stream_id);
                                        break;
                                    }
                                    read_sequence = read_sequence.saturating_add(1);
                                }
                                Err(e) => {
                                    let _ = reader_io_tx
                                        .send(io_error_frame(
                                            &reader_stream_id,
                                            &reader_agent_id,
                                            read_sequence,
                                            "nat_read_failed",
                                            &e.to_string(),
                                        ))
                                        .await;
                                    let _ =
                                        reader_nat_sessions.lock().await.remove(&reader_stream_id);
                                    break;
                                }
                            }
                        }
                    });

                    let mut sessions = nat_sessions.lock().await;
                    if let Err(err) = reserve_agent_nat_session(
                        &mut sessions,
                        &stream_id,
                        NatSocketSession {
                            writer_tx,
                            reader_task,
                        },
                        max_tunnels,
                    ) {
                        drop(sessions);
                        let _ = io_tx
                            .send(io_error_frame(
                                &stream_id,
                                agent_id,
                                sequence,
                                "nat_tunnel_limit_reached",
                                &err.to_string(),
                            ))
                            .await;
                        return;
                    }
                    drop(sessions);

                    tokio::spawn({
                        let stream_id = stream_id.clone();
                        let writer_agent_id = agent_id.to_string();
                        let writer_io_tx = io_tx.clone();
                        async move {
                            while let Some(chunk) = writer_rx.recv().await {
                                if let Err(e) = writer.write_all(&chunk).await {
                                    let _ = writer_io_tx
                                        .send(io_error_frame(
                                            &stream_id,
                                            &writer_agent_id,
                                            sequence.saturating_add(1),
                                            "nat_write_failed",
                                            &e.to_string(),
                                        ))
                                        .await;
                                    break;
                                }
                            }
                        }
                    });

                    let _ = io_tx
                        .send(io_json_data_frame(
                            &stream_id,
                            agent_id,
                            sequence.saturating_add(1),
                            &NatTunnelControlMessage::Ready,
                        ))
                        .await;
                }
                Err(e) => {
                    let _ = io_tx
                        .send(io_error_frame(
                            &stream_id,
                            agent_id,
                            sequence,
                            "nat_open_failed",
                            &format!("failed to connect to {}: {}", local_addr_label, e),
                        ))
                        .await;
                }
            }
        }
        NatTunnelControlMessage::Ready => {}
    }
}

pub async fn close_nat_session(
    agent_id: &str,
    stream_id: &str,
    reason: String,
    sequence: u64,
    nat_sessions: &NatSessionMap,
    io_tx: &mpsc::Sender<IoFrame>,
    echo_close: bool,
) {
    let session = nat_sessions.lock().await.remove(stream_id);
    if let Some(session) = session {
        session.reader_task.abort();
    }
    if echo_close {
        let _ = io_tx
            .send(io_close_frame(stream_id, agent_id, sequence, &reason))
            .await;
    }
}

fn io_json_data_frame(
    stream_id: &str,
    agent_id: &str,
    sequence: u64,
    message: &NatTunnelControlMessage,
) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Data(IoData {
            data: serde_json::to_vec(message).unwrap_or_default(),
        })),
    }
}

fn io_raw_data_frame(stream_id: &str, agent_id: &str, sequence: u64, data: Vec<u8>) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Data(IoData { data })),
    }
}

fn io_close_frame(stream_id: &str, agent_id: &str, sequence: u64, reason: &str) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Close(IoClose {
            reason: reason.to_string(),
        })),
    }
}

fn io_error_frame(
    stream_id: &str,
    agent_id: &str,
    sequence: u64,
    code: &str,
    message: &str,
) -> IoFrame {
    IoFrame {
        stream_id: stream_id.to_string(),
        sequence,
        agent_id: agent_id.to_string(),
        payload: Some(io_frame::Payload::Error(IoError {
            code: code.to_string(),
            message: message.to_string(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn agent_nat_target_allows_loopback_and_rejects_private_by_default() {
        let v4 = resolve_agent_nat_target_with_policy("127.0.0.1", 8080, false)
            .await
            .unwrap();
        assert_eq!(v4, vec!["127.0.0.1:8080".parse().unwrap()]);

        let v6 = resolve_agent_nat_target_with_policy("::1", 8080, false)
            .await
            .unwrap();
        assert_eq!(v6, vec!["[::1]:8080".parse().unwrap()]);

        let mapped = resolve_agent_nat_target_with_policy("::ffff:127.0.0.1", 8080, false)
            .await
            .unwrap();
        assert_eq!(mapped.len(), 1);

        let err = resolve_agent_nat_target_with_policy("192.168.1.10", 8080, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disallowed non-loopback"));
    }

    #[tokio::test]
    async fn agent_nat_target_never_allows_metadata_multicast_unspecified_or_public_ips() {
        for forbidden in [
            "169.254.169.254",
            "169.254.1.10",
            "::ffff:169.254.169.254",
            "fd00:ec2::254",
            "fe80::1",
            "0.0.0.0",
            "::",
            "::ffff:0.0.0.0",
            "224.0.0.1",
            "ff02::1",
            "255.255.255.255",
            "1.1.1.1",
            "2606:4700:4700::1111",
        ] {
            assert!(
                resolve_agent_nat_target_with_policy(forbidden, 80, false)
                    .await
                    .is_err(),
                "target {forbidden} must be rejected when allow_private=false"
            );
            assert!(
                resolve_agent_nat_target_with_policy(forbidden, 80, true)
                    .await
                    .is_err(),
                "target {forbidden} must be rejected even when allow_private=true"
            );
        }

        // RFC1918 and IPv6 ULA are allowed only when allow_private=true
        assert!(resolve_agent_nat_target_with_policy("10.0.0.5", 8080, true)
            .await
            .is_ok());
        assert!(
            resolve_agent_nat_target_with_policy("fd12:3456:789a::1", 8080, true)
                .await
                .is_ok()
        );
    }

    #[test]
    fn dns_pinned_validation_rejects_mixed_records_containing_disallowed_ip() {
        let mixed_metadata = vec![
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap(),
            "169.254.169.254:8080".parse::<SocketAddr>().unwrap(),
        ];
        assert!(validate_agent_nat_resolved_addrs(mixed_metadata.clone(), 8080, false).is_err());
        assert!(validate_agent_nat_resolved_addrs(mixed_metadata, 8080, true).is_err());

        let mixed_private = vec![
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap(),
            "192.168.1.20:8080".parse::<SocketAddr>().unwrap(),
        ];
        assert!(validate_agent_nat_resolved_addrs(mixed_private.clone(), 8080, false).is_err());
        let allowed_when_private =
            validate_agent_nat_resolved_addrs(mixed_private, 8080, true).unwrap();
        assert_eq!(allowed_when_private.len(), 2);
    }

    #[tokio::test]
    async fn dial_agent_nat_target_connects_to_pinned_validated_socket_addr() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();

        let (stream, pinned_addr) =
            dial_agent_nat_target_with_policy("127.0.0.1", listen_addr.port(), false)
                .await
                .unwrap();
        assert_eq!(pinned_addr, listen_addr);
        assert_eq!(stream.peer_addr().unwrap(), listen_addr);
    }

    #[test]
    fn agent_nat_concurrent_tunnels_and_frame_sizes_are_bounded() {
        let mut sessions = HashMap::new();
        for idx in 0..MAX_AGENT_NAT_TUNNELS {
            reserve_agent_nat_session(
                &mut sessions,
                &format!("tunnel-{idx}"),
                idx,
                MAX_AGENT_NAT_TUNNELS,
            )
            .unwrap();
        }
        let limit_err =
            reserve_agent_nat_session(&mut sessions, "tunnel-overflow", 999, MAX_AGENT_NAT_TUNNELS)
                .unwrap_err();
        assert!(limit_err
            .to_string()
            .contains("concurrent NAT tunnel limit"));

        assert!(validate_agent_nat_frame_bytes(&vec![0u8; MAX_NAT_FRAME_BYTES]).is_ok());
        let frame_err =
            validate_agent_nat_frame_bytes(&vec![0u8; MAX_NAT_FRAME_BYTES + 1]).unwrap_err();
        assert!(frame_err.to_string().contains("exceeds maximum allowed"));
    }
}
