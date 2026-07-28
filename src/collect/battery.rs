//! Battery state from `/sys/class/power_supply`.
//!
//! btop draws this on the CPU box's top border rather than in a box of its own.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Charging,
    Discharging,
    Full,
    Unknown,
}

impl Status {
    /// The glyph btop puts between "BAT" and the percentage.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Charging | Self::Full => "⚡",
            Self::Discharging => "▼",
            Self::Unknown => " ",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw.trim() {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" | "Not charging" => Self::Full,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub status: Status,
}

/// Locate a battery once at startup; `None` on desktops.
///
/// Peripheral batteries (wireless mice, keyboards) also register here, so we
/// match on the `BAT` name prefix the way btop does instead of taking the first
/// supply that reports a capacity.
pub fn detect() -> Option<PathBuf> {
    detect_in(Path::new("/sys/class/power_supply"))
}

fn detect_in(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("BAT") || name == "battery")
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find(|path| path.join("capacity").exists())
}

/// Read the current charge, or `None` if the battery went away.
pub fn read(path: &Path) -> Option<Battery> {
    let percent: u8 = fs::read_to_string(path.join("capacity"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let status = fs::read_to_string(path.join("status"))
        .map(|raw| Status::parse(&raw))
        .unwrap_or(Status::Unknown);
    Some(Battery {
        percent: percent.min(100),
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_status_strings() {
        assert_eq!(Status::parse("Charging\n"), Status::Charging);
        assert_eq!(Status::parse("Discharging\n"), Status::Discharging);
        assert_eq!(Status::parse("Full\n"), Status::Full);
        assert_eq!(Status::parse("Not charging\n"), Status::Full);
        assert_eq!(Status::parse("garbage"), Status::Unknown);
    }

    #[test]
    fn full_and_charging_share_the_bolt_glyph() {
        assert_eq!(Status::Full.symbol(), "⚡");
        assert_eq!(Status::Charging.symbol(), "⚡");
        assert_ne!(Status::Discharging.symbol(), "⚡");
    }

    #[test]
    fn absent_supply_directory_is_not_an_error() {
        assert_eq!(detect_in(Path::new("/nonexistent-ratatop-test-root")), None);
        assert_eq!(read(Path::new("/nonexistent-ratatop-test-root")), None);
    }
}
