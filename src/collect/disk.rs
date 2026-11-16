//! Disk sampling: mounts from `/proc/self/mounts`, usage from `statvfs(2)`, and
//! I/O counters from `/sys/block/*/stat`.
//!
//! Follows `btop/src/linux/btop_collect.cpp`: only physical filesystems are
//! listed, `/` sorts first and swap second, and usage is `total - available`
//! rather than `total - free`.

use std::collections::HashMap;
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A sector is 512 bytes in `/sys/block/*/stat`, regardless of the real block
/// size the device reports.
const SECTOR: u64 = 512;

/// How many I/O samples to retain per disk for the graph.
const HISTORY: usize = 512;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Disk {
    /// Display label: the mount point's last component, or `root` for `/`.
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    /// Bytes per second read since the previous sample.
    pub read_rate: f64,
    /// Bytes per second written since the previous sample.
    pub write_rate: f64,
    /// Share of wall time the device spent busy, 0..=100.
    pub io_util: f64,
    /// `io_util` history, oldest first.
    pub io_history: Vec<f64>,
    /// Swap is shown alongside real mounts but has no filesystem to stat.
    pub is_swap: bool,
}

impl Disk {
    pub fn used_percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.used as f64 / self.total as f64 * 100.0).clamp(0.0, 100.0)
    }

    /// Combined throughput, which is what the box shows beside the name.
    pub fn io_rate(&self) -> f64 {
        self.read_rate + self.write_rate
    }
}

/// Raw counters for one device, used to turn monotonic totals into rates.
// macOS does not read per-device counters yet, so nothing constructs this there.
#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct IoCounters {
    sectors_read: u64,
    sectors_written: u64,
    /// Milliseconds the device spent with I/O in flight.
    io_ticks: u64,
}

/// Holds the previous sample so rates can be derived.
#[derive(Debug, Default)]
pub struct Disks {
    disks: Vec<Disk>,
    previous: HashMap<String, IoCounters>,
    sampled_at: Option<Instant>,
}

impl Disks {
    pub fn new() -> Self {
        let mut disks = Self::default();
        // Seed the counters so the first visible tick shows a real rate rather
        // than an average since boot.
        disks.tick();
        disks
    }

    pub fn all(&self) -> &[Disk] {
        &self.disks
    }

    pub fn tick(&mut self) {
        self.tick_in(Path::new("/"), Instant::now());
    }

    fn tick_in(&mut self, root: &Path, now: Instant) {
        let elapsed = self.sampled_at.map(|then| now.duration_since(then));
        self.sampled_at = Some(now);

        let mounts = read_mounts(root);
        let mut next = Vec::with_capacity(mounts.len() + 1);

        for mount in &mounts {
            let mut disk = Disk {
                name: label_for(&mount.path),
                mount: mount.path.clone(),
                ..Disk::default()
            };
            if let Some((total, free)) = usage(&mount.path) {
                disk.total = total;
                disk.free = free;
                disk.used = total.saturating_sub(free);
            }
            self.apply_io(&mut disk, root, &mount.device, elapsed);
            next.push(disk);
        }

        if let Some(swap) = swap_disk(root) {
            next.push(swap);
        }

        // Carry each disk's graph history across the rebuild.
        for disk in &mut next {
            if let Some(old) = self.disks.iter().find(|d| d.mount == disk.mount) {
                disk.io_history = old.io_history.clone();
            }
            push_history(&mut disk.io_history, disk.io_util);
        }

        sort_disks(&mut next);
        self.disks = next;
    }

    /// Turn the device's monotonic counters into rates.
    fn apply_io(&mut self, disk: &mut Disk, root: &Path, device: &str, elapsed: Option<Duration>) {
        let Some(counters) = read_io(root, device) else {
            return;
        };
        let key = disk.mount.clone();
        let previous = self.previous.insert(key, counters);

        let (Some(previous), Some(elapsed)) = (previous, elapsed) else {
            return;
        };
        let seconds = elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return;
        }

        // saturating_sub throughout: counters reset when a device is removed and
        // re-added, and a negative rate is worse than a zero one.
        let read = counters.sectors_read.saturating_sub(previous.sectors_read) * SECTOR;
        let written = counters
            .sectors_written
            .saturating_sub(previous.sectors_written)
            * SECTOR;
        disk.read_rate = read as f64 / seconds;
        disk.write_rate = written as f64 / seconds;

        // io_ticks is in milliseconds of busy time, so its share of the elapsed
        // window is the utilisation.
        let busy_ms = counters.io_ticks.saturating_sub(previous.io_ticks) as f64;
        disk.io_util = (busy_ms / (seconds * 1000.0) * 100.0).clamp(0.0, 100.0);
    }
}

struct Mount {
    device: String,
    path: String,
}

/// Physical mounts only, in the order the kernel lists them.
#[cfg(target_os = "linux")]
fn read_mounts(root: &Path) -> Vec<Mount> {
    let virtual_fs = virtual_filesystems(root);
    let raw = fs::read_to_string(root.join("proc/self/mounts")).unwrap_or_default();

    let mut seen = Vec::new();
    let mut mounts = Vec::new();
    for line in raw.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(path), Some(fstype)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // A device with no leading slash is a pseudo-filesystem source like
        // `proc` or `gvfsd-fuse`, never a partition.
        if !device.starts_with('/') || virtual_fs.contains(&fstype.to_string()) {
            continue;
        }
        // Bind mounts repeat a device at several paths; the first wins.
        if seen.contains(&device.to_string()) {
            continue;
        }
        seen.push(device.to_string());
        mounts.push(Mount {
            device: device.to_string(),
            // The kernel escapes spaces and other characters as octal.
            path: unescape_mount(path),
        });
    }
    mounts
}

/// Mounts from `getmntinfo(3)`, keeping only ones backed by a `/dev` node.
///
/// APFS volumes under `/System/Volumes` all report the same container's space,
/// so only `/` stands in for them; listing each would repeat one number.
#[cfg(target_os = "macos")]
fn read_mounts(_root: &Path) -> Vec<Mount> {
    use std::ffi::CStr;

    let mut table: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: getmntinfo points `table` at a static libc buffer it owns, and
    // returns the number of entries in it (0 or negative on failure).
    let count = unsafe { libc::getmntinfo(&mut table, libc::MNT_NOWAIT) };
    if count <= 0 || table.is_null() {
        return Vec::new();
    }
    // SAFETY: the table holds `count` valid statfs entries, as just reported.
    let entries = unsafe { std::slice::from_raw_parts(table, count as usize) };

    let mut seen: Vec<String> = Vec::new();
    let mut mounts = Vec::new();
    for entry in entries {
        // SAFETY: the kernel NUL-terminates both name fields.
        let device = unsafe { CStr::from_ptr(entry.f_mntfromname.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let path = unsafe { CStr::from_ptr(entry.f_mntonname.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        if !device.starts_with("/dev/") || path.starts_with("/System/Volumes/") {
            continue;
        }
        if seen.contains(&device) {
            continue;
        }
        seen.push(device.clone());
        mounts.push(Mount { device, path });
    }
    mounts
}

/// Filesystem types the kernel marks `nodev`, i.e. not backed by a device.
///
/// Reading this instead of hardcoding a list means new pseudo-filesystems are
/// excluded automatically.
#[cfg(target_os = "linux")]
fn virtual_filesystems(root: &Path) -> Vec<String> {
    let raw = fs::read_to_string(root.join("proc/filesystems")).unwrap_or_default();
    let mut types: Vec<String> = raw
        .lines()
        .filter_map(|line| line.strip_prefix("nodev"))
        .map(|name| name.trim().to_string())
        .collect();
    // squashfs is device-backed but is only ever a read-only snap image, and
    // btop hides those too.
    types.push("squashfs".to_string());
    types
}

/// `/` becomes `root`; anything else uses its last path component.
fn label_for(mount: &str) -> String {
    if mount == "/" {
        return "root".to_string();
    }
    Path::new(mount)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(mount)
        .to_string()
}

/// Decode the octal escapes the kernel writes for spaces, tabs and backslashes.
#[cfg(target_os = "linux")]
fn unescape_mount(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let octal = &path[i + 1..i + 4];
            if let Ok(byte) = u8::from_str_radix(octal, 8) {
                out.push(byte as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Total and available bytes for a mount point.
///
/// Uses `f_bavail` (space an unprivileged process can actually use) rather than
/// `f_bfree`, matching btop's default and `df`'s reported capacity.
fn usage(mount: &str) -> Option<(u64, u64)> {
    let path = CString::new(mount).ok()?;
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call,
    // and `vfs` is fully initialised by statvfs before we read it. A failed call
    // is reported through the return value, which we check.
    let vfs = unsafe {
        let mut vfs: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut vfs) != 0 {
            return None;
        }
        vfs
    };

    let frsize = vfs.f_frsize as u64;
    let total = (vfs.f_blocks as u64).saturating_mul(frsize);
    let free = (vfs.f_bavail as u64).saturating_mul(frsize);
    Some((total, free.min(total)))
}

/// Swap is presented as another row in the disks box.
fn swap_disk(root: &Path) -> Option<Disk> {
    let mem = super::mem::sample_in(root);
    if mem.swap_total == 0 {
        return None;
    }
    Some(Disk {
        name: "swap".to_string(),
        mount: "swap".to_string(),
        total: mem.swap_total,
        used: mem.swap_used,
        free: mem.swap_total.saturating_sub(mem.swap_used),
        is_swap: true,
        ..Disk::default()
    })
}

/// `/` first, swap second, everything else after, matching btop's ordering.
fn sort_disks(disks: &mut [Disk]) {
    disks.sort_by_key(|disk| match disk.mount.as_str() {
        "/" => 0,
        _ if disk.is_swap => 1,
        _ => 2,
    });
}

/// Read a device's I/O counters from sysfs.
#[cfg(target_os = "linux")]
fn read_io(root: &Path, device: &str) -> Option<IoCounters> {
    let raw = fs::read_to_string(sysfs_stat(root, device)?).ok()?;
    let fields: Vec<u64> = raw
        .split_whitespace()
        .map(|v| v.parse().unwrap_or(0))
        .collect();
    // reads, reads_merged, sectors_read, ms_reading, writes, writes_merged,
    // sectors_written, ms_writing, in_flight, io_ticks, weighted_ms
    if fields.len() < 10 {
        return None;
    }
    Some(IoCounters {
        sectors_read: fields[2],
        sectors_written: fields[6],
        io_ticks: fields[9],
    })
}

/// Locate `stat` for a device node.
///
/// Whole disks live at `/sys/block/<dev>/stat`; partitions are nested one level
/// down, at `/sys/block/<parent>/<dev>/stat`.
#[cfg(target_os = "linux")]
fn sysfs_stat(root: &Path, device: &str) -> Option<PathBuf> {
    let name = Path::new(device).file_name()?.to_str()?;
    let block = root.join("sys/block");

    let direct = block.join(name).join("stat");
    if direct.exists() {
        return Some(direct);
    }
    for entry in fs::read_dir(&block).ok()?.flatten() {
        let nested = entry.path().join(name).join("stat");
        if nested.exists() {
            return Some(nested);
        }
    }
    None
}

/// macOS per-device counters live in IOKit's block-storage statistics, which
/// are not read yet, so disks report their usage with no I/O rates.
#[cfg(target_os = "macos")]
fn read_io(_root: &Path, _device: &str) -> Option<IoCounters> {
    None
}

fn push_history(history: &mut Vec<f64>, value: f64) {
    if history.len() >= HISTORY {
        history.remove(0);
    }
    history.push(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_use_the_mount_basename() {
        assert_eq!(label_for("/"), "root");
        assert_eq!(label_for("/home"), "home");
        assert_eq!(label_for("/boot/efi"), "efi");
        assert_eq!(label_for("/mnt/big disk"), "big disk");
    }

    #[test]
    fn used_percent_handles_an_unstatable_mount() {
        let disk = Disk::default();
        assert_eq!(disk.used_percent(), 0.0, "no divide by zero");
    }

    #[test]
    fn used_percent_is_a_share_of_total() {
        let disk = Disk {
            total: 1000,
            used: 980,
            ..Disk::default()
        };
        assert!((disk.used_percent() - 98.0).abs() < 1e-9);
    }

    #[test]
    fn io_rate_combines_both_directions() {
        let disk = Disk {
            read_rate: 1500.0,
            write_rate: 500.0,
            ..Disk::default()
        };
        assert_eq!(disk.io_rate(), 2000.0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn octal_escapes_in_mount_paths_are_decoded() {
        assert_eq!(unescape_mount("/mnt/my\\040disk"), "/mnt/my disk");
        assert_eq!(unescape_mount("/home"), "/home", "untouched when plain");
        // A trailing backslash must not run off the end of the string.
        assert_eq!(unescape_mount("/odd\\"), "/odd\\");
    }

    #[test]
    fn root_sorts_first_and_swap_second() {
        let mut disks = vec![
            Disk {
                mount: "/home".into(),
                ..Disk::default()
            },
            Disk {
                mount: "swap".into(),
                is_swap: true,
                ..Disk::default()
            },
            Disk {
                mount: "/".into(),
                ..Disk::default()
            },
        ];
        sort_disks(&mut disks);
        let order: Vec<&str> = disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(order, vec!["/", "swap", "/home"]);
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let mut history = Vec::new();
        for value in 0..(HISTORY + 5) {
            push_history(&mut history, value as f64);
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history.last(), Some(&((HISTORY + 4) as f64)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_root_yields_no_disks_rather_than_panicking() {
        let mut disks = Disks::default();
        disks.tick_in(Path::new("/nonexistent-ratatop-test-root"), Instant::now());
        assert!(disks.all().is_empty());
    }

    #[test]
    fn reads_the_real_system() {
        let disks = Disks::new();
        assert!(!disks.all().is_empty(), "expected at least one mount");

        let root = disks
            .all()
            .iter()
            .find(|d| d.mount == "/")
            .expect("every linux box has /");
        assert!(root.total > 0, "statvfs should report a size for /");
        assert!(root.used <= root.total);
        assert!((0.0..=100.0).contains(&root.used_percent()));
        assert_eq!(disks.all()[0].mount, "/", "root must sort first");
    }

    #[test]
    fn rates_are_zero_on_the_first_sample_and_finite_after() {
        let mut disks = Disks::default();
        let start = Instant::now();
        disks.tick_in(Path::new("/"), start);
        for disk in disks.all() {
            assert_eq!(disk.read_rate, 0.0, "no previous sample to compare against");
            assert_eq!(disk.io_util, 0.0);
        }

        disks.tick_in(Path::new("/"), start + Duration::from_secs(1));
        for disk in disks.all() {
            assert!(disk.read_rate.is_finite() && disk.read_rate >= 0.0);
            assert!((0.0..=100.0).contains(&disk.io_util));
            assert_eq!(disk.io_history.len(), 2, "one entry per tick");
        }
    }

    #[test]
    fn a_zero_length_interval_does_not_divide_by_zero() {
        let mut disks = Disks::default();
        let now = Instant::now();
        disks.tick_in(Path::new("/"), now);
        disks.tick_in(Path::new("/"), now);
        for disk in disks.all() {
            assert!(disk.read_rate.is_finite());
            assert!(disk.io_util.is_finite());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pseudo_filesystems_are_excluded() {
        let mounts = read_mounts(Path::new("/"));
        for mount in &mounts {
            assert!(
                mount.device.starts_with('/'),
                "{} is not a device",
                mount.device
            );
        }
        let paths: Vec<&str> = mounts.iter().map(|m| m.path.as_str()).collect();
        assert!(!paths.iter().any(|p| p.starts_with("/proc")));
        assert!(!paths.iter().any(|p| p.starts_with("/sys")));
    }
}
