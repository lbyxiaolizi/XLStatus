// Linux x86_64 collectors for M3.
// On other platforms (e.g. macOS), we degrade to whatever sysinfo offers
// for that platform; the Dashboard treats the report as best-effort.
use serde::Serialize;
use sysinfo::{Components, Disks, Networks, System};

/// HostInfo: stable per-host info, sent once on enroll and on demand.
#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub os_version: String,
    pub kernel_version: String,
    pub arch: String,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub total_memory: u64,
    pub total_swap: u64,
    pub boot_time: u64,
    pub agent_version: String,
    pub disks: Vec<HostDisk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostDisk {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub total: u64,
}

/// HostState: rolling metrics, sent every report interval.
#[derive(Debug, Clone, Serialize)]
pub struct HostState {
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub uptime_seconds: u64,
    pub tcp_connections: u64,
    pub udp_connections: u64,
    pub process_count: u64,
    /// Bytes received since boot (cumulative).
    pub network_in_total: u64,
    /// Bytes sent since boot (cumulative).
    pub network_out_total: u64,
    /// Per-NIC: (name, rx_bytes, tx_bytes).
    pub network_interfaces: Vec<NetworkInterface>,
    /// Per-mount: (mount_point, used, total).
    pub disks: Vec<DiskUsage>,
    /// Component temperatures in Celsius (empty if not available).
    pub temperatures: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskUsage {
    pub mount_point: String,
    pub used: u64,
    pub total: u64,
}

pub fn collect_host_info() -> HostInfo {
    let mut sys = System::new_all();
    sys.refresh_all();

    HostInfo {
        hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
        os: System::name().unwrap_or_else(|| "unknown".to_string()),
        os_version: System::os_version().unwrap_or_else(|| "unknown".to_string()),
        kernel_version: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
        arch: System::cpu_arch().unwrap_or_else(|| "unknown".to_string()),
        cpu_brand: sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        cpu_cores: sys.cpus().len(),
        total_memory: sys.total_memory(),
        total_swap: sys.total_swap(),
        boot_time: System::boot_time(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        disks: collect_host_disks(),
    }
}

fn collect_host_disks() -> Vec<HostDisk> {
    let disks = Disks::new_with_refreshed_list();
    let mut list: Vec<HostDisk> = disks
        .iter()
        .map(|d| HostDisk {
            device: d.name().to_string_lossy().to_string(),
            mount_point: d.mount_point().to_string_lossy().to_string(),
            fs_type: d.file_system().to_string_lossy().to_string(),
            total: d.total_space(),
        })
        .collect();
    list.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    list
}

pub fn collect_host_state() -> HostState {
    let mut sys = System::new_all();
    sys.refresh_all();

    let load = System::load_average();
    let cpu_percent = sys.global_cpu_usage();

    // Per-CPU average already encoded in global_cpu_usage on sysinfo 0.32+.

    let total_mem = sys.total_memory();
    let used_mem = total_mem.saturating_sub(sys.available_memory());

    let total_swap = sys.total_swap();
    let used_swap = sys.used_swap();

    // Per-disk usage
    let disks = Disks::new_with_refreshed_list();
    let mut disk_list: Vec<DiskUsage> = disks
        .iter()
        .map(|d| DiskUsage {
            mount_point: d.mount_point().to_string_lossy().to_string(),
            used: d.total_space().saturating_sub(d.available_space()),
            total: d.total_space(),
        })
        .collect();
    // Sort for stable output.
    disk_list.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));

    // Network: cumulative bytes per NIC.
    let networks = Networks::new_with_refreshed_list();
    let mut net_list: Vec<NetworkInterface> = networks
        .iter()
        .map(|(name, data)| NetworkInterface {
            name: name.clone(),
            rx_bytes: data.total_received(),
            tx_bytes: data.total_transmitted(),
        })
        .collect();
    net_list.sort_by(|a, b| a.name.cmp(&b.name));

    let (in_total, out_total) = net_list.iter().fold((0u64, 0u64), |(ri, ro), n| {
        (ri.saturating_add(n.rx_bytes), ro.saturating_add(n.tx_bytes))
    });

    // Temperatures (best-effort; sysinfo Components covers hwmon on Linux).
    let components = Components::new_with_refreshed_list();
    let temperatures: Vec<f32> = components.iter().map(|c| c.temperature()).collect();

    // Process count
    let process_count = sys.processes().len() as u64;

    // Connection counts. sysinfo 0.32 doesn't expose per-protocol connection
    // counters directly; report the best-known signal on each platform.
    //
    // Linux: read /proc/net/tcp and /proc/net/udp counts.
    // macOS: netstat not stable to parse; report 0 with a TODO comment.
    let (tcp_conn, udp_conn) = read_connection_counts();

    let uptime_seconds = System::uptime();

    HostState {
        cpu_percent,
        memory_used: used_mem,
        memory_total: total_mem,
        swap_used: used_swap,
        swap_total: total_swap,
        load1: load.one,
        load5: load.five,
        load15: load.fifteen,
        uptime_seconds,
        tcp_connections: tcp_conn,
        udp_connections: udp_conn,
        process_count,
        network_in_total: in_total,
        network_out_total: out_total,
        network_interfaces: net_list,
        disks: disk_list,
        temperatures,
    }
}

#[cfg(target_os = "linux")]
fn read_connection_counts() -> (u64, u64) {
    fn count_lines_skipping_header(path: &str) -> Option<u64> {
        let s = std::fs::read_to_string(path).ok()?;
        Some(s.lines().count().saturating_sub(1) as u64)
    }
    let tcp = count_lines_skipping_header("/proc/net/tcp").unwrap_or(0)
        + count_lines_skipping_header("/proc/net/tcp6").unwrap_or(0);
    let udp = count_lines_skipping_header("/proc/net/udp").unwrap_or(0)
        + count_lines_skipping_header("/proc/net/udp6").unwrap_or(0);
    (tcp, udp)
}

#[cfg(not(target_os = "linux"))]
fn read_connection_counts() -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_info_populates() {
        let info = collect_host_info();
        assert!(!info.cpu_brand.is_empty());
        assert!(info.total_memory > 0);
    }

    #[test]
    fn host_state_populates() {
        let s = collect_host_state();
        // Memory total must be > 0 on any real system; disks may be empty on
        // restricted sandboxes.
        assert!(s.memory_total > 0);
    }
}
