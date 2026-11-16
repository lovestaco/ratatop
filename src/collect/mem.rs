//! Memory sampling: `/proc/meminfo` on Linux, `host_statistics64(3)` and
//! sysctl on macOS.
//!
//! Linux follows `btop/src/linux/btop_collect.cpp`: `cached` is the raw `Cached`
//! field (not `Cached + SReclaimable`), and `used` is derived from
//! `MemAvailable` rather than summed from parts. macOS follows Activity
//! Monitor's "Memory Used": app memory, wired, and compressed pages.

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::Path;

/// A memory reading, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub cached: u64,
    pub free: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

impl Memory {
    /// A field's share of total memory.
    pub fn percent(&self, value: u64) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (value as f64 / self.total as f64 * 100.0).clamp(0.0, 100.0)
    }
}

pub fn sample() -> Memory {
    sample_in(Path::new("/"))
}

#[cfg(target_os = "linux")]
pub(super) fn sample_in(root: &Path) -> Memory {
    let raw = fs::read_to_string(root.join("proc/meminfo")).unwrap_or_default();
    parse(&raw)
}

#[cfg(target_os = "macos")]
pub(super) fn sample_in(_root: &Path) -> Memory {
    macos::sample()
}

#[cfg(target_os = "linux")]
fn parse(raw: &str) -> Memory {
    let mut mem = Memory::default();
    let mut available = None;
    let mut swap_free = 0;

    for line in raw.lines() {
        let Some((label, rest)) = line.split_once(':') else {
            continue;
        };
        // Every value is in kB, so the useful number is the first token.
        let Some(kib) = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        let bytes = kib.saturating_mul(1024);

        match label {
            "MemTotal" => mem.total = bytes,
            "MemFree" => mem.free = bytes,
            "MemAvailable" => available = Some(bytes),
            "Cached" => mem.cached = bytes,
            "SwapTotal" => mem.swap_total = bytes,
            "SwapFree" => swap_free = bytes,
            _ => {}
        }
    }
    mem.swap_used = mem.swap_total.saturating_sub(swap_free);

    // Kernels before 3.14 have no MemAvailable; btop estimates it the same way.
    mem.available = available.unwrap_or(mem.free + mem.cached);
    // A bogus MemAvailable larger than total would underflow, so fall back to
    // free in that case rather than reporting nonsense.
    let subtract = if mem.available <= mem.total {
        mem.available
    } else {
        mem.free
    };
    mem.used = mem.total.saturating_sub(subtract);
    mem
}

#[cfg(target_os = "macos")]
mod macos {
    use std::mem::{size_of, MaybeUninit};

    use super::Memory;

    pub fn sample() -> Memory {
        let total = crate::collect::cpu::macos::sysctl_u64(c"hw.memsize").unwrap_or(0);
        // SAFETY: sysconf takes an integer and returns one.
        let page = match unsafe { libc::sysconf(libc::_SC_PAGESIZE) } {
            size if size > 0 => size as u64,
            _ => 4096,
        };

        let mut mem = Memory {
            total,
            swap_total: 0,
            ..Memory::default()
        };

        if let Some(stats) = vm_stats() {
            // Activity Monitor's "Memory Used": app memory (internal pages that
            // are not purgeable), wired memory, and compressor-held pages.
            let app = stats
                .internal_page_count
                .saturating_sub(stats.purgeable_count);
            let used_pages =
                app as u64 + stats.wire_count as u64 + stats.compressor_page_count as u64;
            mem.used = (used_pages * page).min(total);
            mem.free = stats.free_count as u64 * page;
            mem.cached = stats.external_page_count as u64 * page;
            mem.available = total.saturating_sub(mem.used);
        }

        if let Some(swap) = swap_usage() {
            mem.swap_total = swap.xsu_total;
            mem.swap_used = swap.xsu_used;
        }
        mem
    }

    fn vm_stats() -> Option<libc::vm_statistics64> {
        let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        // SAFETY: the buffer is a vm_statistics64, and `count` is in
        // integer_t units, as host_statistics64 expects.
        let status = unsafe {
            libc::host_statistics64(
                mach2::mach_init::mach_host_self(),
                libc::HOST_VM_INFO64,
                stats.as_mut_ptr().cast(),
                &mut count,
            )
        };
        // SAFETY: a successful call fills the structure.
        (status == libc::KERN_SUCCESS).then(|| unsafe { stats.assume_init() })
    }

    fn swap_usage() -> Option<libc::xsw_usage> {
        let mut usage = MaybeUninit::<libc::xsw_usage>::zeroed();
        let mut len = size_of::<libc::xsw_usage>();
        // SAFETY: the buffer is an xsw_usage, and `len` is its size.
        let status = unsafe {
            libc::sysctlbyname(
                c"vm.swapusage".as_ptr(),
                usage.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        // SAFETY: a successful call fills the structure.
        (status == 0).then(|| unsafe { usage.assume_init() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const MEMINFO: &str = "\
MemTotal:       14177796 kB
MemFree:          497404 kB
MemAvailable:    7490900 kB
Buffers:         1235656 kB
Cached:          5146132 kB
SwapCached:        81068 kB
Active:          6141092 kB
";

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_the_fields_the_box_shows() {
        let mem = parse(MEMINFO);
        assert_eq!(mem.total, 14_177_796 * 1024);
        assert_eq!(mem.free, 497_404 * 1024);
        assert_eq!(mem.available, 7_490_900 * 1024);
        assert_eq!(mem.cached, 5_146_132 * 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn used_is_total_minus_available() {
        let mem = parse(MEMINFO);
        assert_eq!(mem.used, (14_177_796 - 7_490_900) * 1024);
        assert_eq!(mem.used + mem.available, mem.total, "the two must sum");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn swap_cached_does_not_get_mistaken_for_cached() {
        // "SwapCached" contains "Cached", so a sloppy match would clobber it.
        let mem = parse(MEMINFO);
        assert_eq!(mem.cached, 5_146_132 * 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn falls_back_when_mem_available_is_missing() {
        let mem = parse("MemTotal: 1000 kB\nMemFree: 200 kB\nCached: 300 kB\n");
        assert_eq!(mem.available, 500 * 1024, "free + cached");
        assert_eq!(mem.used, 500 * 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_impossible_available_does_not_underflow() {
        let mem = parse("MemTotal: 100 kB\nMemFree: 40 kB\nMemAvailable: 900 kB\n");
        assert_eq!(mem.used, 60 * 1024, "falls back to total minus free");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn percent_is_a_share_of_total() {
        let mem = parse(MEMINFO);
        assert!((mem.percent(mem.available) - 52.8).abs() < 0.1);
        assert_eq!(Memory::default().percent(100), 0.0, "no total known");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn garbage_input_yields_zeroes_not_panics() {
        assert_eq!(parse(""), Memory::default());
        assert_eq!(parse("nonsense\nMemTotal: banana kB\n"), Memory::default());
    }

    #[test]
    fn percent_of_an_unknown_total_is_zero() {
        assert_eq!(Memory::default().percent(100), 0.0, "no total known");
    }

    #[test]
    fn reads_the_real_system() {
        let mem = sample();
        assert!(mem.total > 0, "every machine has memory");
        assert!(mem.used <= mem.total);
        assert!(mem.free <= mem.total);
    }
}
