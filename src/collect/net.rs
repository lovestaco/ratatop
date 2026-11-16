//! Network sampling: `/proc/net/dev` and `/proc/net/route` on Linux,
//! `sysctl(NET_RT_IFLIST2)` on macOS, plus interface addresses via
//! `getifaddrs(3)` on both.
//!
//! Follows `btop/src/linux/btop_collect.cpp`: rates are deltas between samples,
//! `top` is the highest rate seen, and `total` is the interface's counter since
//! boot.

use std::collections::{BTreeMap, HashMap};
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// How many rate samples to retain per direction.
const HISTORY: usize = 512;

/// One direction of traffic on an interface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Direction {
    /// Bytes per second over the last interval.
    pub rate: f64,
    /// Highest rate seen since ratatop started.
    pub top: f64,
    /// Bytes transferred, as the kernel counts them since boot.
    pub total: u64,
    /// `rate` history, oldest first.
    pub history: Vec<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interface {
    pub name: String,
    /// First IPv4 address bound to the interface, if any.
    pub ip: Option<String>,
    pub download: Direction,
    pub upload: Direction,
}

impl Interface {
    /// Full-scale value for the graphs.
    ///
    /// btop scales to the highest rate seen rather than the current one, so the
    /// graph keeps its shape instead of rescaling every frame. The 10 KiB floor
    /// stops an idle link from magnifying noise into mountains.
    pub fn graph_max(&self) -> f64 {
        self.download.top.max(self.upload.top).max(10.0 * 1024.0)
    }
}

/// Raw counters, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counters {
    received: u64,
    transmitted: u64,
}

#[derive(Debug, Default)]
pub struct Net {
    interfaces: BTreeMap<String, Interface>,
    previous: HashMap<String, Counters>,
    sampled_at: Option<Instant>,
    selected: Option<String>,
}

impl Net {
    pub fn new() -> Self {
        let mut net = Self::default();
        // Seed the counters so the first visible tick is a real rate rather than
        // an average since boot.
        net.tick();
        net
    }

    pub fn tick(&mut self) {
        self.tick_in(Path::new("/"), Instant::now());
    }

    fn tick_in(&mut self, root: &Path, now: Instant) {
        let elapsed = self.sampled_at.map(|then| now.duration_since(then));
        self.sampled_at = Some(now);

        let addresses = ip_addresses();

        for (name, counters) in read_counters(root) {
            let previous = self.previous.insert(name.clone(), counters);
            let entry = self
                .interfaces
                .entry(name.clone())
                .or_insert_with(|| Interface {
                    name: name.clone(),
                    ..Interface::default()
                });
            entry.ip = addresses.get(&name).cloned();
            entry.download.total = counters.received;
            entry.upload.total = counters.transmitted;

            let rates = rates(previous, counters, elapsed);
            update(&mut entry.download, rates.map(|(down, _)| down));
            update(&mut entry.upload, rates.map(|(_, up)| up));
        }

        if self.selected.is_none() {
            self.selected = default_interface(root, &self.interfaces);
        }
    }

    /// Interfaces worth showing, in name order.
    ///
    /// Loopback is excluded: it is always present, always busy, and never what
    /// you opened a monitor to look at.
    pub fn interfaces(&self) -> Vec<&Interface> {
        self.interfaces
            .values()
            .filter(|iface| !is_loopback(&iface.name))
            .collect()
    }

    pub fn selected(&self) -> Option<&Interface> {
        let name = self.selected.as_ref()?;
        self.interfaces.get(name)
    }

    /// Step to the next or previous interface, wrapping at both ends.
    pub fn cycle(&mut self, forward: bool) {
        let names: Vec<String> = self.interfaces().iter().map(|i| i.name.clone()).collect();
        if names.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|name| names.iter().position(|n| n == name))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % names.len()
        } else {
            (current + names.len() - 1) % names.len()
        };
        self.selected = Some(names[next].clone());
    }
}

/// `lo` on Linux, `lo0` on macOS.
fn is_loopback(name: &str) -> bool {
    name == "lo" || name == "lo0"
}

/// Bytes per second in each direction, or `None` without a comparable sample.
fn rates(
    previous: Option<Counters>,
    current: Counters,
    elapsed: Option<Duration>,
) -> Option<(f64, f64)> {
    let (previous, elapsed) = (previous?, elapsed?);
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return None;
    }
    // saturating_sub covers the counter wrapping or the interface being reset.
    let down = current.received.saturating_sub(previous.received) as f64 / seconds;
    let up = current.transmitted.saturating_sub(previous.transmitted) as f64 / seconds;
    Some((down, up))
}

fn update(direction: &mut Direction, rate: Option<f64>) {
    let rate = rate.unwrap_or(0.0);
    direction.rate = rate;
    if rate > direction.top {
        direction.top = rate;
    }
    if direction.history.len() >= HISTORY {
        direction.history.remove(0);
    }
    direction.history.push(rate);
}

/// Interface name to received/transmitted byte counts.
#[cfg(target_os = "linux")]
fn read_counters(root: &Path) -> Vec<(String, Counters)> {
    let raw = fs::read_to_string(root.join("proc/net/dev")).unwrap_or_default();
    parse_dev(&raw)
}

#[cfg(target_os = "macos")]
fn read_counters(_root: &Path) -> Vec<(String, Counters)> {
    macos::counters()
}

#[cfg(target_os = "linux")]
fn parse_dev(raw: &str) -> Vec<(String, Counters)> {
    raw.lines()
        .filter_map(|line| {
            // Only data lines carry a colon separating the interface name from
            // its counters, so this alone drops the two header lines.
            let (name, rest) = line.split_once(':')?;
            let fields: Vec<u64> = rest
                .split_whitespace()
                .map(|v| v.parse().unwrap_or(0))
                .collect();
            // receive: bytes packets errs drop fifo frame compressed multicast
            // transmit: bytes ...
            if fields.len() < 9 {
                return None;
            }
            Some((
                name.trim().to_string(),
                Counters {
                    received: fields[0],
                    transmitted: fields[8],
                },
            ))
        })
        .collect()
}

/// The interface carrying the default route, falling back to the busiest one.
#[cfg(target_os = "linux")]
fn default_interface(root: &Path, interfaces: &BTreeMap<String, Interface>) -> Option<String> {
    let routes = fs::read_to_string(root.join("proc/net/route")).unwrap_or_default();
    let default = routes.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace();
        let name = fields.next()?;
        // A destination of all zeroes is the default route.
        (fields.next()? == "00000000").then(|| name.to_string())
    });
    if let Some(name) = default {
        if interfaces.contains_key(&name) {
            return Some(name);
        }
    }
    busiest(interfaces)
}

/// macOS has no `/proc/net/route`, so the busiest interface stands in for the
/// default route.
#[cfg(target_os = "macos")]
fn default_interface(_root: &Path, interfaces: &BTreeMap<String, Interface>) -> Option<String> {
    busiest(interfaces)
}

fn busiest(interfaces: &BTreeMap<String, Interface>) -> Option<String> {
    interfaces
        .values()
        .filter(|iface| !is_loopback(&iface.name))
        .max_by_key(|iface| iface.download.total + iface.upload.total)
        .map(|iface| iface.name.clone())
}

/// IPv4 address per interface, from `getifaddrs(3)`.
///
/// There is no file that lists these. `/proc/net/fib_trie` comes closest but is
/// a serialised routing structure, not an interface listing.
fn ip_addresses() -> HashMap<String, String> {
    let mut addresses = HashMap::new();

    // SAFETY: getifaddrs allocates the list and reports failure through its
    // return value, which we check before dereferencing. We only read fields of
    // nodes still in the list, stop at the null terminator, and free the list
    // exactly once through the same pointer getifaddrs handed us.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return addresses;
        }

        let mut node = list;
        while !node.is_null() {
            let entry = &*node;
            node = entry.ifa_next;

            if entry.ifa_addr.is_null() {
                continue;
            }

            // Interfaces carry several addresses; only IPv4 fits the box.
            if u32::from((*entry.ifa_addr).sa_family) != libc::AF_INET as u32 {
                continue;
            }

            let Ok(name) = CStr::from_ptr(entry.ifa_name).to_str() else {
                continue;
            };
            let socket = entry.ifa_addr.cast::<libc::sockaddr_in>();
            let octets = u32::from_be((*socket).sin_addr.s_addr).to_be_bytes();
            // First address wins, matching what `ip addr` shows first.
            addresses
                .entry(name.to_string())
                .or_insert_with(|| octets.map(|o| o.to_string()).join("."));
        }

        libc::freeifaddrs(list);
    }
    addresses
}

/// Per-interface byte counters from `sysctl(NET_RT_IFLIST2)`.
///
/// These are the 64-bit `if_data64` counters. `getifaddrs` would hand back the
/// 32-bit `if_data`, which wraps every few minutes on a busy link.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_int, c_void};
    use std::mem::size_of;
    use std::ptr;

    use super::Counters;

    unsafe extern "C" {
        fn if_indextoname(ifindex: u32, ifname: *mut c_char) -> *mut c_char;
    }

    pub fn counters() -> Vec<(String, Counters)> {
        let mib: [c_int; 6] = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];

        let mut len = 0usize;
        // SAFETY: a null buffer asks only for the size of the dump.
        let status = unsafe {
            libc::sysctl(
                mib.as_ptr() as *mut c_int,
                mib.len() as libc::c_uint,
                ptr::null_mut(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        if status != 0 || len == 0 {
            return Vec::new();
        }

        let mut buffer = vec![0u8; len];
        // SAFETY: `buffer` has `len` writable bytes; the kernel reports how many
        // it actually wrote back through `len`.
        let status = unsafe {
            libc::sysctl(
                mib.as_ptr() as *mut c_int,
                mib.len() as libc::c_uint,
                buffer.as_mut_ptr().cast::<c_void>(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return Vec::new();
        }
        buffer.truncate(len);

        let mut out = Vec::new();
        let mut offset = 0usize;
        // Each message starts with its own length, so walk them by that.
        while offset + size_of::<libc::if_msghdr2>() <= buffer.len() {
            // SAFETY: the bounds check above keeps the header inside the buffer.
            // read_unaligned because the kernel does not guarantee alignment.
            let header: libc::if_msghdr2 =
                unsafe { ptr::read_unaligned(buffer.as_ptr().add(offset).cast()) };
            let message_len = header.ifm_msglen as usize;
            if message_len == 0 || offset + message_len > buffer.len() {
                break;
            }

            if header.ifm_type as c_int == libc::RTM_IFINFO2 {
                let mut name = [0 as c_char; libc::IF_NAMESIZE];
                // SAFETY: the name buffer is IF_NAMESIZE bytes, as if_indextoname expects.
                let ok = unsafe { if_indextoname(header.ifm_index as u32, name.as_mut_ptr()) };
                if !ok.is_null() {
                    // SAFETY: if_indextoname wrote a NUL-terminated name into `name`.
                    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
                    if let Ok(name) = name.to_str() {
                        out.push((
                            name.to_string(),
                            Counters {
                                received: header.ifm_data.ifi_ibytes,
                                transmitted: header.ifm_data.ifi_obytes,
                            },
                        ));
                    }
                }
            }
            offset += message_len;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 2011390   15626    0    0    0     0          0         0  2011390   15626    0    0    0     0       0          0
  eno1:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0
  wlo1: 319815680  240000    0    0    0     0          0         0 137363456  180000    0    0    0     0       0          0
";

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_receive_and_transmit_bytes() {
        let parsed = parse_dev(DEV);
        assert_eq!(parsed.len(), 3, "headers are not data lines");

        let wlo1 = parsed.iter().find(|(name, _)| name == "wlo1").unwrap();
        assert_eq!(wlo1.1.received, 319_815_680);
        // Transmit bytes is the ninth counter, not the second.
        assert_eq!(wlo1.1.transmitted, 137_363_456);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn header_lines_are_skipped() {
        // Neither header line contains a colon, which is what excludes them.
        let parsed = parse_dev(DEV);
        assert!(parsed.iter().all(|(name, _)| !name.contains("face")));
        assert!(parsed.iter().all(|(name, _)| !name.contains("Inter")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn garbage_input_yields_nothing() {
        assert!(parse_dev("").is_empty());
        assert!(parse_dev("no colon here\n").is_empty());
        assert!(parse_dev("eth0: 1 2 3\n").is_empty(), "too few fields");
    }

    #[test]
    fn rates_need_a_previous_sample_and_elapsed_time() {
        let current = Counters {
            received: 2000,
            transmitted: 1000,
        };
        let previous = Counters {
            received: 1000,
            transmitted: 500,
        };
        assert_eq!(rates(None, current, Some(Duration::from_secs(1))), None);
        assert_eq!(rates(Some(previous), current, None), None);
        assert_eq!(
            rates(Some(previous), current, Some(Duration::ZERO)),
            None,
            "no divide by zero"
        );

        let (down, up) = rates(Some(previous), current, Some(Duration::from_secs(2))).unwrap();
        assert_eq!((down, up), (500.0, 250.0));
    }

    #[test]
    fn rates_survive_a_counter_reset() {
        let previous = Counters {
            received: 9000,
            transmitted: 9000,
        };
        let current = Counters {
            received: 10,
            transmitted: 10,
        };
        let (down, up) = rates(Some(previous), current, Some(Duration::from_secs(1))).unwrap();
        assert_eq!((down, up), (0.0, 0.0), "interface was reset, not negative");
    }

    #[test]
    fn top_is_the_highest_rate_seen() {
        let mut direction = Direction::default();
        update(&mut direction, Some(100.0));
        update(&mut direction, Some(900.0));
        update(&mut direction, Some(50.0));
        assert_eq!(direction.rate, 50.0, "rate is the latest");
        assert_eq!(direction.top, 900.0, "top is the peak");
        assert_eq!(direction.history, vec![100.0, 900.0, 50.0]);
    }

    #[test]
    fn history_is_capped() {
        let mut direction = Direction::default();
        for value in 0..(HISTORY + 5) {
            update(&mut direction, Some(value as f64));
        }
        assert_eq!(direction.history.len(), HISTORY);
        assert_eq!(direction.history.last(), Some(&((HISTORY + 4) as f64)));
    }

    #[test]
    fn graph_scale_has_a_floor() {
        let idle = Interface::default();
        assert_eq!(
            idle.graph_max(),
            10.0 * 1024.0,
            "idle link is not magnified"
        );

        let busy = Interface {
            download: Direction {
                top: 5_000_000.0,
                ..Direction::default()
            },
            ..Interface::default()
        };
        assert_eq!(busy.graph_max(), 5_000_000.0);
    }

    #[test]
    fn loopback_is_hidden() {
        let net = Net::new();
        assert!(net
            .interfaces()
            .iter()
            .all(|iface| !is_loopback(&iface.name)));
    }

    #[test]
    fn both_platforms_loopback_names_are_recognised() {
        assert!(is_loopback("lo"));
        assert!(is_loopback("lo0"));
        assert!(!is_loopback("en0"));
    }

    #[test]
    fn cycling_wraps_in_both_directions() {
        let mut net = Net::new();
        let names: Vec<String> = net.interfaces().iter().map(|i| i.name.clone()).collect();
        if names.len() < 2 {
            return; // single-interface machine
        }
        let start = net.selected().map(|i| i.name.clone());

        for _ in 0..names.len() {
            net.cycle(true);
        }
        assert_eq!(
            net.selected().map(|i| i.name.clone()),
            start,
            "wrapped back"
        );

        net.cycle(false);
        net.cycle(true);
        assert_eq!(net.selected().map(|i| i.name.clone()), start, "reversible");
    }

    #[test]
    fn cycling_with_no_interfaces_does_nothing() {
        let mut net = Net::default();
        net.cycle(true);
        assert!(net.selected().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_root_yields_no_interfaces() {
        let mut net = Net::default();
        net.tick_in(Path::new("/nonexistent-ratatop-test-root"), Instant::now());
        assert!(net.interfaces().is_empty());
    }

    #[test]
    fn reads_the_real_system() {
        let mut net = Net::new();
        net.tick();
        let selected = net.selected().expect("a default interface");
        assert!(!selected.name.is_empty());
        assert!(selected.download.rate >= 0.0 && selected.download.rate.is_finite());
        assert!(selected.download.total > 0 || selected.upload.total > 0);
    }

    #[test]
    fn addresses_look_like_ipv4() {
        for (name, ip) in ip_addresses() {
            assert!(!name.is_empty());
            let octets: Vec<&str> = ip.split('.').collect();
            assert_eq!(octets.len(), 4, "{ip} is not dotted quad");
            assert!(octets.iter().all(|o| o.parse::<u8>().is_ok()), "{ip}");
        }
    }
}
