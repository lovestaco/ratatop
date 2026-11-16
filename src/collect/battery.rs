//! Battery state: `/sys/class/power_supply` on Linux, IOKit's power-source API
//! on macOS.
//!
//! btop draws this on the CPU box's top border rather than in a box of its own.

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

/// Where a battery's readings come from.
///
/// Linux exposes one sysfs directory per battery. macOS has a single IOKit
/// query that covers the internal battery, so there is no path to keep.
#[cfg(target_os = "linux")]
pub type Source = PathBuf;
#[cfg(target_os = "macos")]
pub type Source = ();

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

    #[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
pub fn detect() -> Option<Source> {
    detect_in(Path::new("/sys/class/power_supply"))
}

/// Locate a battery once at startup; `None` on desktops.
#[cfg(target_os = "macos")]
pub fn detect() -> Option<Source> {
    internal_battery().map(|_| ())
}

/// Peripheral batteries (wireless mice, keyboards) also register in sysfs, so
/// we match on the `BAT` name prefix the way btop does instead of taking the
/// first supply that reports a capacity.
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
pub fn read(path: &Source) -> Option<Battery> {
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

/// Read the current charge, or `None` if the battery went away.
#[cfg(target_os = "macos")]
pub fn read(_source: &Source) -> Option<Battery> {
    internal_battery()
}

/// The internal battery's charge and state, from the power-source list.
///
/// Mirrors what `pmset -g batt` and the menu bar show: percent from current
/// over max capacity, and `Full` while on AC power and not charging, the same
/// mapping Linux's "Not charging" gets.
#[cfg(target_os = "macos")]
fn internal_battery() -> Option<Battery> {
    use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFType};
    use objc2_io_kit::{
        kIOPSACPowerValue, kIOPSBatteryPowerValue, kIOPSCurrentCapacityKey,
        kIOPSInternalBatteryType, kIOPSIsChargingKey, kIOPSMaxCapacityKey,
        kIOPSPowerSourceStateKey, kIOPSTypeKey, IOPSCopyPowerSourcesInfo, IOPSCopyPowerSourcesList,
        IOPSGetPowerSourceDescription,
    };
    use std::ffi::{c_void, CStr};

    fn lookup<'a>(dict: &'a CFDictionary, key: &CStr) -> Option<&'a CFType> {
        let key = CFString::from_str(key.to_str().ok()?);
        // SAFETY: the key is a live CFString for the duration of the call, and
        // the dictionary owns the returned value for as long as it is borrowed.
        let value = unsafe { dict.value(&*key as *const CFString as *const c_void) };
        // SAFETY: a non-null value from the dictionary is a live CF object.
        unsafe { value.cast::<CFType>().as_ref() }
    }

    fn text(dict: &CFDictionary, key: &CStr) -> Option<String> {
        Some(lookup(dict, key)?.downcast_ref::<CFString>()?.to_string())
    }

    fn number(dict: &CFDictionary, key: &CStr) -> Option<i64> {
        let value = lookup(dict, key)?.downcast_ref::<objc2_core_foundation::CFNumber>()?;
        let mut out: i64 = 0;
        // SAFETY: `out` is an i64, which is what the SInt64 type asks CF to write.
        let ok = unsafe {
            value.value(
                objc2_core_foundation::CFNumberType::SInt64Type,
                &mut out as *mut i64 as *mut c_void,
            )
        };
        ok.then_some(out)
    }

    let info = IOPSCopyPowerSourcesInfo()?;
    // SAFETY: `info` is the live blob this list is copied from.
    let list: CFRetained<CFArray> = unsafe { IOPSCopyPowerSourcesList(Some(&info)) }?;

    for index in 0..list.count() {
        // SAFETY: index is within the array's count.
        let source = unsafe { list.value_at_index(index) };
        let Some(source) = (unsafe { source.cast::<CFType>().as_ref() }) else {
            continue;
        };
        let Some(desc) = (unsafe { IOPSGetPowerSourceDescription(Some(&info), Some(source)) })
        else {
            continue;
        };

        if text(&desc, kIOPSTypeKey).as_deref() != kIOPSInternalBatteryType.to_str().ok() {
            continue;
        }

        let current = number(&desc, kIOPSCurrentCapacityKey)?;
        let max = number(&desc, kIOPSMaxCapacityKey)?;
        if max <= 0 {
            return None;
        }
        let percent = (current * 100 / max).clamp(0, 100) as u8;

        let charging = lookup(&desc, kIOPSIsChargingKey)
            .and_then(|v| v.downcast_ref::<objc2_core_foundation::CFBoolean>())
            .is_some_and(|b| b.as_bool());
        let state = text(&desc, kIOPSPowerSourceStateKey);

        let status = if charging {
            Status::Charging
        } else if state.as_deref() == kIOPSACPowerValue.to_str().ok() {
            Status::Full
        } else if state.as_deref() == kIOPSBatteryPowerValue.to_str().ok() {
            Status::Discharging
        } else {
            Status::Unknown
        };

        return Some(Battery { percent, status });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_and_charging_share_the_bolt_glyph() {
        assert_eq!(Status::Full.symbol(), "⚡");
        assert_eq!(Status::Charging.symbol(), "⚡");
        assert_ne!(Status::Discharging.symbol(), "⚡");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_kernel_status_strings() {
        assert_eq!(Status::parse("Charging\n"), Status::Charging);
        assert_eq!(Status::parse("Discharging\n"), Status::Discharging);
        assert_eq!(Status::parse("Full\n"), Status::Full);
        assert_eq!(Status::parse("Not charging\n"), Status::Full);
        assert_eq!(Status::parse("garbage"), Status::Unknown);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn absent_supply_directory_is_not_an_error() {
        assert_eq!(detect_in(Path::new("/nonexistent-ratatop-test-root")), None);
        assert_eq!(read(&PathBuf::from("/nonexistent-ratatop-test-root")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reading_the_battery_never_panics() {
        // Desktops have no battery, so both answers are fine; this checks the
        // IOKit path runs and stays in range when a battery is present.
        if let Some(battery) = detect().and_then(|source| read(&source)) {
            assert!(battery.percent <= 100);
        }
    }
}
