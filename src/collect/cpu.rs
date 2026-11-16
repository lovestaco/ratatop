//! CPU sampling: `/proc/stat`, `/sys/devices/system/cpu`, and hwmon on Linux;
//! `host_processor_info(3)` and sysctl on macOS.
//!
//! Linux mirrors what `btop/src/linux/btop_collect.cpp` reads, minus the bits
//! the CPU box does not display. macOS has no sysfs equivalent, so frequency
//! and temperature stay blank there: the only public sources are sysctls that
//! Apple Silicon does not populate, and SMC access is private.

use std::fs;
use std::path::{Path, PathBuf};

/// One line of `/proc/stat`, in USER_HZ ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Times {
    pub total: u64,
    /// Everything the kernel does not count as idle or iowait.
    pub busy: u64,
}

impl Times {
    /// Busy percentage relative to a previous sample.
    ///
    /// Counters are monotonic, so a decrease means the previous sample is not
    /// comparable (CPU hotplug, suspend) and we report 0 rather than a negative.
    pub fn percent_since(self, previous: Self) -> f64 {
        let total = self.total.saturating_sub(previous.total);
        let busy = self.busy.saturating_sub(previous.busy);
        if total == 0 {
            return 0.0;
        }
        (busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    }
}

/// A CPU reading: aggregate first, then one entry per core.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sample {
    pub total: Times,
    pub cores: Vec<Times>,
}

/// Static facts that do not change between ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuInfo {
    pub model: String,
    pub core_count: usize,
    /// hwmon `tempN_input` for the CPU package, if one was found.
    temp_input: Option<PathBuf>,
    /// The sensor's critical trip point in °C, for scaling the temperature color.
    pub temp_critical: f64,
}

impl CpuInfo {
    pub fn detect() -> Self {
        Self::detect_in(Path::new("/"))
    }

    #[cfg(target_os = "linux")]
    fn detect_in(root: &Path) -> Self {
        let cpuinfo = read(&root.join("proc/cpuinfo")).unwrap_or_default();
        let (temp_input, temp_critical) = find_cpu_sensor(&root.join("sys/class/hwmon"));
        Self {
            model: parse_model(&cpuinfo),
            core_count: parse_core_count(&cpuinfo),
            temp_input,
            temp_critical,
        }
    }

    #[cfg(target_os = "macos")]
    fn detect_in(_root: &Path) -> Self {
        Self {
            model: macos::model(),
            core_count: macos::core_count(),
            temp_input: None,
            temp_critical: 80.0,
        }
    }

    /// Package temperature in °C, or `None` when no sensor was found.
    pub fn temperature(&self) -> Option<f64> {
        let raw: f64 = read(self.temp_input.as_ref()?)?.trim().parse().ok()?;
        Some(raw / 1000.0)
    }
}

/// Read the aggregate and per-core counters.
pub fn sample() -> Sample {
    sample_in(Path::new("/"))
}

#[cfg(target_os = "linux")]
fn sample_in(root: &Path) -> Sample {
    parse_stat(&read(&root.join("proc/stat")).unwrap_or_default())
}

#[cfg(target_os = "macos")]
fn sample_in(_root: &Path) -> Sample {
    macos::sample()
}

/// Mean current frequency across cores, in GHz.
///
/// `scaling_cur_freq` is the live value; `/proc/cpuinfo`'s `cpu MHz` is the
/// fallback for kernels without cpufreq.
pub fn frequency_ghz() -> Option<f64> {
    frequency_ghz_in(Path::new("/"))
}

#[cfg(target_os = "linux")]
fn frequency_ghz_in(root: &Path) -> Option<f64> {
    let cpus = root.join("sys/devices/system/cpu");
    let mut khz = Vec::new();
    if let Ok(entries) = fs::read_dir(&cpus) {
        for entry in entries.flatten() {
            let path = entry.path().join("cpufreq/scaling_cur_freq");
            if let Some(value) = read(&path).and_then(|v| v.trim().parse::<f64>().ok()) {
                khz.push(value);
            }
        }
    }
    if !khz.is_empty() {
        return Some(khz.iter().sum::<f64>() / khz.len() as f64 / 1_000_000.0);
    }

    let cpuinfo = read(&root.join("proc/cpuinfo"))?;
    let mhz: Vec<f64> = cpuinfo
        .lines()
        .filter_map(|line| line.strip_prefix("cpu MHz"))
        .filter_map(|rest| rest.split(':').nth(1)?.trim().parse().ok())
        .collect();
    (!mhz.is_empty()).then(|| mhz.iter().sum::<f64>() / mhz.len() as f64 / 1000.0)
}

/// Nominal frequency from sysctl. Intel Macs report `hw.cpufrequency`;
/// Apple Silicon does not, so this is `None` there.
#[cfg(target_os = "macos")]
fn frequency_ghz_in(_root: &Path) -> Option<f64> {
    macos::sysctl_u64(c"hw.cpufrequency").map(|hz| hz as f64 / 1_000_000_000.0)
}

/// The 1, 5, and 15 minute load averages.
pub fn load_average() -> [f64; 3] {
    load_average_in(Path::new("/"))
}

#[cfg(target_os = "linux")]
fn load_average_in(root: &Path) -> [f64; 3] {
    let Some(raw) = read(&root.join("proc/loadavg")) else {
        return [0.0; 3];
    };
    let mut values = raw.split_whitespace().filter_map(|v| v.parse().ok());
    [
        values.next().unwrap_or(0.0),
        values.next().unwrap_or(0.0),
        values.next().unwrap_or(0.0),
    ]
}

#[cfg(target_os = "macos")]
fn load_average_in(_root: &Path) -> [f64; 3] {
    let mut loads = [0.0f64; 3];
    // SAFETY: getloadavg writes at most `nelem` doubles, and the array holds 3.
    let written = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if written == 3 {
        loads
    } else {
        [0.0; 3]
    }
}

#[cfg(target_os = "linux")]
fn parse_stat(raw: &str) -> Sample {
    let mut sample = Sample::default();
    for line in raw.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            // `/proc/stat` puts every cpu line first, so the first miss ends them.
            break;
        };
        let (label, fields) = match rest.split_once(char::is_whitespace) {
            Some(parts) => parts,
            None => continue,
        };

        // Fields are: user nice system idle iowait irq softirq steal guest guest_nice.
        // guest time is already included in user, so counting it again would
        // inflate the total on virtualized hosts.
        let times: Vec<u64> = fields
            .split_whitespace()
            .take(8)
            .filter_map(|v| v.parse().ok())
            .collect();
        if times.len() < 4 {
            continue;
        }

        let total: u64 = times.iter().sum();
        let idle = times[3] + times.get(4).copied().unwrap_or(0);
        let entry = Times {
            total,
            busy: total.saturating_sub(idle),
        };

        if label.is_empty() {
            sample.total = entry;
        } else {
            sample.cores.push(entry);
        }
    }
    sample
}

#[cfg(target_os = "linux")]
fn parse_model(cpuinfo: &str) -> String {
    cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("model name")?.split(':').nth(1))
        .map(|raw| shorten_model(raw.trim()))
        // Not shortened: the noise list would eat "CPU" out of the fallback.
        .unwrap_or_else(|| "Unknown CPU".to_string())
}

/// Trim vendor boilerplate the way btop does, so the name fits the box title.
fn shorten_model(raw: &str) -> String {
    let mut name = raw.to_string();
    for noise in [
        "(R)",
        "(r)",
        "(TM)",
        "(tm)",
        "CPU",
        "Processor",
        "Dual-Core",
        "Quad-Core",
        "Six-Core",
        "Eight-Core",
        "Core",
        "with Radeon Vega Mobile Gfx",
        "with Radeon Graphics",
    ] {
        name = name.replace(noise, " ");
    }
    // Drop the "@ 3.40GHz" suffix; the box shows a live frequency instead.
    if let Some(at) = name.find('@') {
        name.truncate(at);
    }
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(target_os = "linux")]
fn parse_core_count(cpuinfo: &str) -> usize {
    cpuinfo
        .lines()
        .filter(|line| line.starts_with("processor"))
        .count()
        .max(1)
}

/// Pick a package sensor, preferring the vendor drivers btop looks for.
#[cfg(target_os = "linux")]
fn find_cpu_sensor(hwmon: &Path) -> (Option<PathBuf>, f64) {
    const PREFERRED: [&str; 5] = ["k10temp", "coretemp", "zenpower", "cpu_thermal", "acpitz"];

    let Ok(entries) = fs::read_dir(hwmon) else {
        return (None, 80.0);
    };
    let devices: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = read(&path.join("name"))?.trim().to_string();
            Some((name, path))
        })
        .collect();

    for wanted in PREFERRED {
        let Some((_, path)) = devices.iter().find(|(name, _)| name == wanted) else {
            continue;
        };
        // k10temp exposes Tctl and Tdie; both are package readings, so the first
        // temp input is the right one on every driver in this list.
        let input = path.join("temp1_input");
        if input.exists() {
            let critical = read(&path.join("temp1_crit"))
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map_or(80.0, |v| v / 1000.0);
            return (Some(input), critical);
        }
    }
    (None, 80.0)
}

/// `fs::read_to_string` that treats every failure as "not available".
///
/// Reads under `/proc` and `/sys` fail routinely — permissions, hotplug, a
/// sensor disappearing on suspend — and none of it should stop the render loop.
fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Mach and sysctl readers for macOS.
#[cfg(target_os = "macos")]
pub(super) mod macos {
    use std::ffi::{c_void, CStr};
    use std::mem::size_of;
    use std::ptr;

    use super::{Sample, Times};

    /// `CPU_STATE_MAX` entries per core: user, system, idle, nice.
    const STATES: usize = 4;
    const CPU_STATE_USER: usize = 0;
    const CPU_STATE_SYSTEM: usize = 1;
    const CPU_STATE_IDLE: usize = 2;
    const CPU_STATE_NICE: usize = 3;

    /// Per-core tick counters from `host_processor_info(3)`.
    pub fn sample() -> Sample {
        let mut count: libc::natural_t = 0;
        let mut info: libc::processor_info_array_t = ptr::null_mut();
        let mut info_count: libc::mach_msg_type_number_t = 0;

        // SAFETY: on success the call writes the core count and a vm-allocated
        // array of `info_count` integers. That array is read below and then
        // released with vm_deallocate, exactly once.
        let status = unsafe {
            libc::host_processor_info(
                mach2::mach_init::mach_host_self(),
                libc::PROCESSOR_CPU_LOAD_INFO,
                &mut count,
                &mut info,
                &mut info_count,
            )
        };
        if status != libc::KERN_SUCCESS || info.is_null() {
            return Sample::default();
        }

        let mut sample = Sample::default();
        // SAFETY: the kernel reports `info_count` valid integers at `info`.
        let ticks = unsafe { std::slice::from_raw_parts(info, info_count as usize) };
        let (cores, _) = ticks.as_chunks::<STATES>();
        for core in cores.iter().take(count as usize) {
            let user = core[CPU_STATE_USER] as u64;
            let system = core[CPU_STATE_SYSTEM] as u64;
            let idle = core[CPU_STATE_IDLE] as u64;
            let nice = core[CPU_STATE_NICE] as u64;
            let total = user + system + idle + nice;
            let entry = Times {
                total,
                busy: total - idle,
            };
            sample.total.total += entry.total;
            sample.total.busy += entry.busy;
            sample.cores.push(entry);
        }

        // SAFETY: `info` came from host_processor_info and is freed exactly once.
        unsafe {
            libc::vm_deallocate(
                mach2::traps::mach_task_self(),
                info as libc::vm_address_t,
                info_count as libc::vm_size_t * size_of::<libc::integer_t>() as libc::vm_size_t,
            );
        }
        sample
    }

    pub fn core_count() -> usize {
        // SAFETY: sysconf takes an integer and returns one.
        let online = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        usize::try_from(online).unwrap_or(1).max(1)
    }

    /// The brand string, shortened the way the Linux model names are.
    pub fn model() -> String {
        sysctl_string(c"machdep.cpu.brand_string")
            .map(|raw| super::shorten_model(&raw))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }

    pub fn sysctl_u64(name: &CStr) -> Option<u64> {
        let mut value: u64 = 0;
        let mut len = size_of::<u64>();
        // SAFETY: `value` and `len` are sized for a 64-bit sysctl, and the name is
        // NUL-terminated.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut u64).cast::<c_void>(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        (status == 0 && len == size_of::<u64>()).then_some(value)
    }

    fn sysctl_string(name: &CStr) -> Option<String> {
        let mut len = 0usize;
        // SAFETY: a null buffer asks only for the length.
        let status = unsafe {
            libc::sysctlbyname(name.as_ptr(), ptr::null_mut(), &mut len, ptr::null_mut(), 0)
        };
        if status != 0 || len == 0 {
            return None;
        }
        let mut buffer = vec![0u8; len];
        // SAFETY: `buffer` has exactly `len` writable bytes.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                buffer.as_mut_ptr().cast::<c_void>(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
        String::from_utf8(buffer[..end].to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const STAT: &str = "\
cpu  100 10 40 800 50 0 0 0 0 0
cpu0 50 5 20 400 25 0 0 0 0 0
cpu1 50 5 20 400 25 0 0 0 0 0
intr 12345 0 0
ctxt 987654
";

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_aggregate_and_per_core_lines() {
        let sample = parse_stat(STAT);
        assert_eq!(sample.cores.len(), 2);
        // total = 100+10+40+800+50 = 1000, idle = 800+50 = 850.
        assert_eq!(
            sample.total,
            Times {
                total: 1000,
                busy: 150
            }
        );
        assert_eq!(
            sample.cores[0],
            Times {
                total: 500,
                busy: 75
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stops_at_the_first_non_cpu_line() {
        let sample = parse_stat("cpu  1 1 1 1 1 0 0 0\nintr 5\ncpu9 9 9 9 9 9 0 0 0\n");
        assert!(sample.cores.is_empty(), "cpu9 is past the cpu block");
    }

    #[test]
    fn percent_is_the_busy_share_of_the_delta() {
        let before = Times {
            total: 1000,
            busy: 100,
        };
        let after = Times {
            total: 2000,
            busy: 600,
        };
        assert!((after.percent_since(before) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn percent_is_zero_when_no_time_passed() {
        let times = Times {
            total: 1000,
            busy: 100,
        };
        assert_eq!(times.percent_since(times), 0.0);
    }

    #[test]
    fn percent_survives_counters_going_backwards() {
        let before = Times {
            total: 5000,
            busy: 900,
        };
        let after = Times {
            total: 1000,
            busy: 100,
        };
        assert_eq!(after.percent_since(before), 0.0, "hotplug or suspend");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn guest_time_is_not_double_counted() {
        // guest (10) and guest_nice (10) trail the eight fields we consume; they
        // are already part of user time.
        let sample = parse_stat("cpu  100 0 0 900 0 0 0 0 10 10\n");
        assert_eq!(sample.total.total, 1000);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn model_name_drops_vendor_noise() {
        assert_eq!(
            parse_model("model name\t: AMD Ryzen 5 3550H with Radeon Vega Mobile Gfx\n"),
            "AMD Ryzen 5 3550H"
        );
        assert_eq!(
            parse_model("model name\t: Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz\n"),
            "Intel i7-8550U"
        );
    }

    #[test]
    fn shortened_model_names_are_tidy() {
        assert_eq!(shorten_model("Apple M2 Pro"), "Apple M2 Pro");
        assert_eq!(
            shorten_model("Intel(R) Core(TM) i9-9880H CPU @ 2.30GHz"),
            "Intel i9-9880H"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn model_name_falls_back_when_absent() {
        assert_eq!(parse_model(""), "Unknown CPU");
        assert_eq!(parse_core_count(""), 1, "never report zero cores");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn counts_processor_lines() {
        assert_eq!(
            parse_core_count("processor: 0\nprocessor: 1\nflags: x\n"),
            2
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_files_degrade_to_defaults() {
        let missing = Path::new("/nonexistent-ratatop-test-root");
        assert_eq!(load_average_in(missing), [0.0; 3]);
        assert_eq!(sample_in(missing), Sample::default());
        assert_eq!(frequency_ghz_in(missing), None);
    }

    #[test]
    fn reads_the_real_system() {
        // The CPU box is worthless if these do not work on the host, so assert
        // the shape of a live read rather than mocking it away.
        let info = CpuInfo::detect();
        assert!(info.core_count >= 1);
        let sample = sample();
        assert_eq!(sample.cores.len(), info.core_count);
        assert!(sample.total.total > 0);
    }
}
