//! Memory sampling from `/proc/meminfo`.
//!
//! Follows `btop/src/linux/btop_collect.cpp`: `cached` is the raw `Cached` field
//! (not `Cached + SReclaimable`), and `used` is derived from `MemAvailable`
//! rather than summed from parts.

use std::fs;
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

pub(super) fn sample_in(root: &Path) -> Memory {
    let raw = fs::read_to_string(root.join("proc/meminfo")).unwrap_or_default();
    parse(&raw)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "\
MemTotal:       14177796 kB
MemFree:          497404 kB
MemAvailable:    7490900 kB
Buffers:         1235656 kB
Cached:          5146132 kB
SwapCached:        81068 kB
Active:          6141092 kB
";

    #[test]
    fn parses_the_fields_the_box_shows() {
        let mem = parse(MEMINFO);
        assert_eq!(mem.total, 14_177_796 * 1024);
        assert_eq!(mem.free, 497_404 * 1024);
        assert_eq!(mem.available, 7_490_900 * 1024);
        assert_eq!(mem.cached, 5_146_132 * 1024);
    }

    #[test]
    fn used_is_total_minus_available() {
        let mem = parse(MEMINFO);
        assert_eq!(mem.used, (14_177_796 - 7_490_900) * 1024);
        assert_eq!(mem.used + mem.available, mem.total, "the two must sum");
    }

    #[test]
    fn swap_cached_does_not_get_mistaken_for_cached() {
        // "SwapCached" contains "Cached", so a sloppy match would clobber it.
        let mem = parse(MEMINFO);
        assert_eq!(mem.cached, 5_146_132 * 1024);
    }

    #[test]
    fn falls_back_when_mem_available_is_missing() {
        let mem = parse("MemTotal: 1000 kB\nMemFree: 200 kB\nCached: 300 kB\n");
        assert_eq!(mem.available, 500 * 1024, "free + cached");
        assert_eq!(mem.used, 500 * 1024);
    }

    #[test]
    fn an_impossible_available_does_not_underflow() {
        let mem = parse("MemTotal: 100 kB\nMemFree: 40 kB\nMemAvailable: 900 kB\n");
        assert_eq!(mem.used, 60 * 1024, "falls back to total minus free");
    }

    #[test]
    fn percent_is_a_share_of_total() {
        let mem = parse(MEMINFO);
        assert!((mem.percent(mem.available) - 52.8).abs() < 0.1);
        assert_eq!(Memory::default().percent(100), 0.0, "no total known");
    }

    #[test]
    fn garbage_input_yields_zeroes_not_panics() {
        assert_eq!(parse(""), Memory::default());
        assert_eq!(parse("nonsense\nMemTotal: banana kB\n"), Memory::default());
    }

    #[test]
    fn reads_the_real_system() {
        let mem = sample();
        assert!(mem.total > 0, "every machine has memory");
        assert!(mem.used <= mem.total);
        assert!(mem.free <= mem.total);
    }
}
