//! Host-level checks and housekeeping: RAM/swap/disk preflight, a disk "doctor",
//! swap creation, and port-conflict detection.

use anyhow::{Context, Result, bail};
use hostable_core::{PortSpec, Protocol, Requirements};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize)]
pub struct Preflight {
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub swap_total_mb: u64,
    pub disk_free_mb: u64,
    /// Things worth knowing, shown but not blocking.
    pub warnings: Vec<String>,
    /// Things that would make the install fail; creation is refused.
    pub blockers: Vec<String>,
}

pub struct RunningServer {
    pub name: String,
    pub reserved_mb: u64,
}

pub fn preflight(
    requirements: &Requirements,
    wanted_memory_mb: Option<u32>,
    disk_path: &Path,
    running: &[RunningServer],
) -> Preflight {
    let mem = read_meminfo();
    let disk_free_mb = disk_free_mb(disk_path).unwrap_or(u64::MAX);
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    if disk_free_mb < u64::from(requirements.min_disk_mb) {
        blockers.push(format!(
            "Only {disk_free_mb} MB of disk is free but this game needs about {} MB.",
            requirements.min_disk_mb
        ));
    }

    let wanted = u64::from(wanted_memory_mb.unwrap_or(requirements.recommended_memory_mb));
    let reserved: u64 = running.iter().map(|server| server.reserved_mb).sum();
    let usable = mem.total_mb + mem.swap_total_mb;

    if mem.total_mb + mem.swap_total_mb < u64::from(requirements.min_memory_mb) {
        warnings.push(format!(
            "This game wants at least {} MB of memory; the machine has {} MB RAM + {} MB swap. It will probably be killed by the kernel. Add swap (Settings, or `POST /api/system/swap`) or use a bigger VPS.",
            requirements.min_memory_mb, mem.total_mb, mem.swap_total_mb
        ));
    } else if mem.total_mb < u64::from(requirements.recommended_memory_mb) {
        warnings.push(format!(
            "{} MB of RAM is below the recommended {} MB. It will work for a few players but may lag.",
            mem.total_mb, requirements.recommended_memory_mb
        ));
    }

    if !running.is_empty() {
        let names = running
            .iter()
            .map(|server| server.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if reserved + wanted > mem.total_mb {
            warnings.push(format!(
                "Already running: {names} (about {reserved} MB). Together with this server (about {wanted} MB) that is more than the {} MB of RAM, so running both at once will swap or get killed. Stop the others first.",
                mem.total_mb
            ));
        } else {
            warnings.push(format!("Already running: {names} (about {reserved} MB)."));
        }
    }

    if mem.swap_total_mb == 0 && usable < u64::from(requirements.recommended_memory_mb) * 3 / 2 {
        warnings.push(
            "No swap is configured. A few GB of swap protects against sudden out-of-memory kills."
                .to_string(),
        );
    }

    Preflight {
        total_memory_mb: mem.total_mb,
        available_memory_mb: mem.available_mb,
        swap_total_mb: mem.swap_total_mb,
        disk_free_mb: if disk_free_mb == u64::MAX {
            0
        } else {
            disk_free_mb
        },
        warnings,
        blockers,
    }
}

struct MemInfo {
    total_mb: u64,
    available_mb: u64,
    swap_total_mb: u64,
}

fn read_meminfo() -> MemInfo {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| {
        text.lines()
            .find_map(|line| {
                let rest = line.strip_prefix(key)?.strip_prefix(':')?;
                rest.split_whitespace().next()?.parse::<u64>().ok()
            })
            .unwrap_or(0)
    };
    MemInfo {
        total_mb: kb("MemTotal") / 1024,
        available_mb: kb("MemAvailable") / 1024,
        swap_total_mb: kb("SwapTotal") / 1024,
    }
}

fn disk_free_mb(path: &Path) -> Option<u64> {
    // `df` needs an existing path; walk up to the first one that exists.
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let output = Command::new("df").args(["-Pm"]).arg(probe).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()
}

/// Create (or grow into) a swapfile. Idempotent: does nothing if one is active.
pub fn ensure_swap(size_mb: u32) -> Result<String> {
    if !(512..=65_536).contains(&size_mb) {
        bail!("swap size must be between 512 and 65536 MB");
    }
    if read_meminfo().swap_total_mb > 0 {
        return Ok("swap is already enabled".to_string());
    }
    let path = "/swapfile";
    if Path::new(path).exists() {
        bail!("{path} already exists but is not active; remove or enable it manually");
    }
    run("fallocate", &["-l", &format!("{size_mb}M"), path])?;
    run("chmod", &["600", path])?;
    run("mkswap", &[path])?;
    run("swapon", &[path])?;
    let fstab = fs::read_to_string("/etc/fstab").unwrap_or_default();
    if !fstab.lines().any(|line| line.starts_with(path)) {
        let mut updated = fstab;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str("/swapfile none swap sw 0 0\n");
        fs::write("/etc/fstab", updated).context("failed to update /etc/fstab")?;
    }
    Ok(format!("created and enabled a {size_mb} MB swapfile"))
}

// ---- ports ------------------------------------------------------------------------

/// Ports something is already listening on, as `(port, protocol)`.
pub fn listening_ports() -> Vec<(u16, Protocol)> {
    let Ok(output) = Command::new("ss").args(["-H", "-ltun"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let protocol = match columns.next()? {
                "tcp" => Protocol::Tcp,
                "udp" => Protocol::Udp,
                _ => return None,
            };
            // columns: State Recv-Q Send-Q Local Peer
            let local = columns.nth(3)?;
            let port = local.rsplit(':').next()?.parse::<u16>().ok()?;
            Some((port, protocol))
        })
        .collect()
}

pub fn conflicting_ports(wanted: &[PortSpec]) -> Vec<u16> {
    let in_use = listening_ports();
    wanted
        .iter()
        .filter(|port| {
            in_use
                .iter()
                .any(|(used, protocol)| *used == port.port && *protocol == port.protocol)
        })
        .map(|port| port.port)
        .collect()
}

/// First port at or above `preferred` that is neither listening nor claimed.
pub fn next_free_port(preferred: u16, protocol: Protocol, claimed: &[u16]) -> u16 {
    let in_use = listening_ports();
    (preferred..=u16::MAX)
        .find(|candidate| {
            !claimed.contains(candidate)
                && !in_use
                    .iter()
                    .any(|(used, proto)| used == candidate && *proto == protocol)
        })
        .unwrap_or(preferred)
}

// ---- disk doctor -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DiskItem {
    pub label: String,
    pub path: Option<PathBuf>,
    pub bytes: u64,
    /// Identifier accepted by the clean endpoint, if Hostable can free this safely.
    pub clean_action: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskReport {
    pub total_mb: u64,
    pub free_mb: u64,
    pub used_percent: u64,
    pub items: Vec<DiskItem>,
}

pub struct DiskScanInput<'a> {
    pub servers_root: &'a Path,
    pub backups_root: &'a Path,
    pub registered_paths: &'a [PathBuf],
    pub extra_roots: &'a [PathBuf],
}

pub fn disk_report(input: &DiskScanInput<'_>) -> DiskReport {
    let (total_mb, free_mb) = df_total_free(Path::new("/")).unwrap_or((0, 0));
    let used_percent = (total_mb.saturating_sub(free_mb) * 100)
        .checked_div(total_mb)
        .unwrap_or(0);
    let mut items = Vec::new();

    let journal = dir_size(Path::new("/var/log/journal")) + dir_size(Path::new("/run/log/journal"));
    if journal > 0 {
        items.push(DiskItem {
            label: "System journal (logs)".to_string(),
            path: Some(PathBuf::from("/var/log/journal")),
            bytes: journal,
            clean_action: Some("journal".to_string()),
            note: "Safe to trim; keeps the most recent 200 MB.".to_string(),
        });
    }
    let apt = dir_size(Path::new("/var/cache/apt"));
    if apt > 50 * 1024 * 1024 {
        items.push(DiskItem {
            label: "Downloaded package files".to_string(),
            path: Some(PathBuf::from("/var/cache/apt")),
            bytes: apt,
            clean_action: Some("apt".to_string()),
            note: "Safe to clear; apt re-downloads when needed.".to_string(),
        });
    }

    if let Ok(entries) = fs::read_dir(input.backups_root) {
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let bytes = dir_size(&entry.path());
            if bytes > 0 {
                items.push(DiskItem {
                    label: format!("Backups of {}", entry.file_name().to_string_lossy()),
                    path: Some(entry.path()),
                    bytes,
                    clean_action: None,
                    note: "Lower the backup retention in Settings to keep fewer.".to_string(),
                });
            }
        }
    }

    // Folders in the servers root that no registered server uses.
    if let Ok(entries) = fs::read_dir(input.servers_root) {
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let path = entry.path();
            if !input.registered_paths.iter().any(|known| known == &path) {
                items.push(DiskItem {
                    label: format!(
                        "Unregistered folder {}",
                        entry.file_name().to_string_lossy()
                    ),
                    bytes: dir_size(&path),
                    path: Some(path),
                    clean_action: None,
                    note: "Not managed by Hostable. Delete by hand if you no longer need it."
                        .to_string(),
                });
            }
        }
    }

    // Big folders elsewhere (old server copies, world backups). Reported, never deleted.
    for root in input.extra_roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let path = entry.path();
            if input.registered_paths.iter().any(|known| known == &path) {
                continue;
            }
            let bytes = dir_size(&path);
            if bytes >= 500 * 1024 * 1024 {
                items.push(DiskItem {
                    label: format!("Large folder {}", path.display()),
                    path: Some(path),
                    bytes,
                    clean_action: None,
                    note: "Outside Hostable. Check for old server copies or world backups."
                        .to_string(),
                });
            }
        }
    }

    items.sort_by_key(|item| std::cmp::Reverse(item.bytes));
    DiskReport {
        total_mb,
        free_mb,
        used_percent,
        items,
    }
}

pub fn disk_clean(action: &str) -> Result<String> {
    match action {
        "journal" => {
            run("journalctl", &["--vacuum-size=200M"])?;
            Ok("trimmed the system journal to 200 MB".to_string())
        }
        "apt" => {
            run("apt-get", &["clean"])?;
            Ok("cleared downloaded package files".to_string())
        }
        other => bail!("unknown clean action: {other}"),
    }
}

fn df_total_free(path: &Path) -> Option<(u64, u64)> {
    let output = Command::new("df").args(["-Pm"]).arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut columns = text.lines().nth(1)?.split_whitespace();
    let total = columns.nth(1)?.parse::<u64>().ok()?;
    let free = columns.nth(1)?.parse::<u64>().ok()?;
    Some((total, free))
}

// ---- storage summary for the admin UI --------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ServerStorage {
    pub id: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageReport {
    /// Whole filesystem the servers live on.
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub servers: Vec<ServerStorage>,
}

static STORAGE_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<String>, StorageReport)>> =
    std::sync::Mutex::new(None);
const STORAGE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Per-server folder sizes plus the filesystem totals. Folder sizes need a full
/// walk (slow for a big world), so the answer is reused for a minute.
pub fn storage_report(servers: &[(String, PathBuf)], fs_path: &Path) -> StorageReport {
    let key: Vec<String> = servers
        .iter()
        .map(|(id, path)| format!("{id}={}", path.display()))
        .collect();
    if let Ok(cache) = STORAGE_CACHE.lock()
        && let Some((at, cached_key, report)) = cache.as_ref()
        && at.elapsed() < STORAGE_TTL
        && *cached_key == key
    {
        return report.clone();
    }
    let (total_bytes, used_bytes, free_bytes) = df_bytes(fs_path).unwrap_or((0, 0, 0));
    let report = StorageReport {
        total_bytes,
        used_bytes,
        free_bytes,
        servers: servers
            .iter()
            .map(|(id, path)| ServerStorage {
                id: id.clone(),
                bytes: dir_size(path),
            })
            .collect(),
    };
    if let Ok(mut cache) = STORAGE_CACHE.lock() {
        *cache = Some((std::time::Instant::now(), key, report.clone()));
    }
    report
}

/// (total, used, available) in bytes for the filesystem holding `path`.
fn df_bytes(path: &Path) -> Option<(u64, u64, u64)> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let output = Command::new("df").args(["-Pk"]).arg(probe).output().ok()?;
    parse_df_kb(&String::from_utf8_lossy(&output.stdout))
}

fn parse_df_kb(text: &str) -> Option<(u64, u64, u64)> {
    let mut columns = text.lines().nth(1)?.split_whitespace();
    let total = columns.nth(1)?.parse::<u64>().ok()?;
    let used = columns.next()?.parse::<u64>().ok()?;
    let free = columns.next()?.parse::<u64>().ok()?;
    Some((total * 1024, used * 1024, free * 1024))
}

/// Size in bytes of a directory tree. Stays on one filesystem and does not follow symlinks.
fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    Command::new("du")
        .args(["-sbx"])
        .arg(path)
        .output()
        .ok()
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn df_output_is_read_in_bytes() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1000 400 600 40% /\n";
        assert_eq!(
            super::parse_df_kb(text),
            Some((1000 * 1024, 400 * 1024, 600 * 1024))
        );
        assert_eq!(super::parse_df_kb("nope"), None);
    }

    use super::*;

    fn requirements(min: u32, recommended: u32, disk: u32) -> Requirements {
        Requirements {
            packages: Vec::new(),
            architectures: Vec::new(),
            apt_components: Vec::new(),
            min_memory_mb: min,
            recommended_memory_mb: recommended,
            min_disk_mb: disk,
        }
    }

    #[test]
    fn blocks_when_disk_is_too_small() {
        let report = preflight(&requirements(1, 1, u32::MAX), None, Path::new("/"), &[]);
        assert!(!report.blockers.is_empty());
    }

    #[test]
    fn warns_when_memory_is_far_below_the_minimum() {
        let report = preflight(
            &requirements(u32::MAX, u32::MAX, 0),
            None,
            Path::new("/"),
            &[],
        );
        assert!(report.warnings.iter().any(|w| w.contains("at least")));
    }

    #[test]
    fn warns_about_servers_already_running() {
        let running = [RunningServer {
            name: "Cobbleverse".to_string(),
            reserved_mb: u64::MAX / 4,
        }];
        let report = preflight(&requirements(0, 0, 0), Some(1024), Path::new("/"), &running);
        assert!(report.warnings.iter().any(|w| w.contains("Cobbleverse")));
    }

    #[test]
    fn finds_a_free_port_past_claimed_ones() {
        let port = next_free_port(25565, Protocol::Tcp, &[25565, 25566]);
        assert!(port >= 25567);
    }
}
