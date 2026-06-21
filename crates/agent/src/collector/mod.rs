// Collectors are best-effort across platforms. FreeBSD uses command-based
// fallbacks to avoid sysinfo's kvm/procstat link requirements in cross builds.
use serde::Serialize;
#[cfg(target_os = "freebsd")]
use std::{
    collections::BTreeMap,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(not(target_os = "freebsd"))]
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

#[cfg(not(target_os = "freebsd"))]
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

#[cfg(not(target_os = "freebsd"))]
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

#[cfg(not(target_os = "freebsd"))]
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

#[cfg(target_os = "freebsd")]
pub fn collect_host_info() -> HostInfo {
    let (swap_total, _) = freebsd_swap_bytes();

    HostInfo {
        hostname: command_output("hostname", &[]).unwrap_or_else(|| "unknown".to_string()),
        os: "FreeBSD".to_string(),
        os_version: command_output("uname", &["-r"]).unwrap_or_else(|| "unknown".to_string()),
        kernel_version: command_output("uname", &["-v"]).unwrap_or_else(|| "unknown".to_string()),
        arch: command_output("uname", &["-m"]).unwrap_or_else(|| "unknown".to_string()),
        cpu_brand: sysctl_string("hw.model").unwrap_or_else(|| "unknown".to_string()),
        cpu_cores: sysctl_u64("hw.ncpu").unwrap_or(0) as usize,
        total_memory: sysctl_u64("hw.physmem").unwrap_or(0),
        total_swap: swap_total,
        boot_time: freebsd_boot_time(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        disks: freebsd_host_disks(),
    }
}

#[cfg(target_os = "freebsd")]
pub fn collect_host_state() -> HostState {
    let memory_total = sysctl_u64("hw.physmem").unwrap_or(0);
    let page_size = sysctl_u64("hw.pagesize").unwrap_or(4096);
    let free_pages = sysctl_u64("vm.stats.vm.v_free_count").unwrap_or(0);
    let memory_free = free_pages.saturating_mul(page_size);
    let memory_used = memory_total.saturating_sub(memory_free);
    let (swap_total, swap_used) = freebsd_swap_bytes();
    let (load1, load5, load15) = freebsd_load_average();
    let (tcp_connections, udp_connections) = freebsd_connection_counts();
    let network_interfaces = freebsd_network_interfaces();
    let (network_in_total, network_out_total) =
        network_interfaces
            .iter()
            .fold((0u64, 0u64), |(rx, tx), iface| {
                (
                    rx.saturating_add(iface.rx_bytes),
                    tx.saturating_add(iface.tx_bytes),
                )
            });

    HostState {
        cpu_percent: 0.0,
        memory_used,
        memory_total,
        swap_used,
        swap_total,
        load1,
        load5,
        load15,
        uptime_seconds: freebsd_uptime(),
        tcp_connections,
        udp_connections,
        process_count: freebsd_process_count(),
        network_in_total,
        network_out_total,
        network_interfaces,
        disks: freebsd_disk_usage(),
        temperatures: Vec::new(),
    }
}

#[cfg(target_os = "freebsd")]
fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(target_os = "freebsd")]
fn sysctl_string(name: &str) -> Option<String> {
    command_output("sysctl", &["-n", name])
}

#[cfg(target_os = "freebsd")]
fn sysctl_u64(name: &str) -> Option<u64> {
    sysctl_string(name)?.trim().parse().ok()
}

#[cfg(target_os = "freebsd")]
fn freebsd_boot_time() -> u64 {
    let output = sysctl_string("kern.boottime").unwrap_or_default();
    output
        .split("sec = ")
        .nth(1)
        .and_then(|rest| rest.split([',', '}']).next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "freebsd")]
fn freebsd_uptime() -> u64 {
    let boot_time = freebsd_boot_time();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    now.saturating_sub(boot_time)
}

#[cfg(target_os = "freebsd")]
fn freebsd_swap_bytes() -> (u64, u64) {
    let Some(output) = command_output("swapinfo", &["-k"]) else {
        return (0, 0);
    };
    let mut total = 0u64;
    let mut used = 0u64;
    for line in output.lines().skip(1) {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 3 {
            continue;
        }
        total = total.saturating_add(columns[1].parse::<u64>().unwrap_or(0).saturating_mul(1024));
        used = used.saturating_add(columns[2].parse::<u64>().unwrap_or(0).saturating_mul(1024));
    }
    (total, used)
}

#[cfg(target_os = "freebsd")]
fn freebsd_load_average() -> (f64, f64, f64) {
    let Some(output) = sysctl_string("vm.loadavg") else {
        return (0.0, 0.0, 0.0);
    };
    let values: Vec<f64> = output
        .trim_matches(|ch| ch == '{' || ch == '}')
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();
    (
        values.first().copied().unwrap_or(0.0),
        values.get(1).copied().unwrap_or(0.0),
        values.get(2).copied().unwrap_or(0.0),
    )
}

#[cfg(target_os = "freebsd")]
fn freebsd_host_disks() -> Vec<HostDisk> {
    let Some(output) = command_output("df", &["-kP"]) else {
        return Vec::new();
    };
    let mut disks: Vec<HostDisk> = output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 6 {
                return None;
            }
            Some(HostDisk {
                device: columns[0].to_string(),
                mount_point: columns[5].to_string(),
                fs_type: "unknown".to_string(),
                total: columns[1].parse::<u64>().unwrap_or(0).saturating_mul(1024),
            })
        })
        .collect();
    disks.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    disks
}

#[cfg(target_os = "freebsd")]
fn freebsd_disk_usage() -> Vec<DiskUsage> {
    let Some(output) = command_output("df", &["-kP"]) else {
        return Vec::new();
    };
    let mut disks: Vec<DiskUsage> = output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 6 {
                return None;
            }
            Some(DiskUsage {
                mount_point: columns[5].to_string(),
                used: columns[2].parse::<u64>().unwrap_or(0).saturating_mul(1024),
                total: columns[1].parse::<u64>().unwrap_or(0).saturating_mul(1024),
            })
        })
        .collect();
    disks.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    disks
}

#[cfg(target_os = "freebsd")]
fn freebsd_network_interfaces() -> Vec<NetworkInterface> {
    let Some(output) = command_output("netstat", &["-ibn"]) else {
        return Vec::new();
    };
    let mut interfaces: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for line in output.lines().skip(1) {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 10 {
            continue;
        }
        let name = columns[0].to_string();
        let rx = columns[6].parse::<u64>().unwrap_or(0);
        let tx = columns[9].parse::<u64>().unwrap_or(0);
        let entry = interfaces.entry(name).or_insert((0, 0));
        entry.0 = entry.0.max(rx);
        entry.1 = entry.1.max(tx);
    }
    interfaces
        .into_iter()
        .map(|(name, (rx_bytes, tx_bytes))| NetworkInterface {
            name,
            rx_bytes,
            tx_bytes,
        })
        .collect()
}

#[cfg(target_os = "freebsd")]
fn freebsd_process_count() -> u64 {
    command_output("ps", &["-ax", "-o", "pid="])
        .map(|output| output.lines().count() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "freebsd")]
fn freebsd_connection_counts() -> (u64, u64) {
    fn count(protocol: &str) -> u64 {
        command_output("netstat", &["-an", "-p", protocol])
            .map(|output| output.lines().skip(2).count() as u64)
            .unwrap_or(0)
    }
    (count("tcp"), count("udp"))
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
