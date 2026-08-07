//! Process sampling from `/proc/[pid]/`.
//!
//! Follows `btop/src/linux/btop_collect.cpp`: CPU is the process's share of all
//! cores between two samples, memory is resident set size, and the table is
//! sorted in the collector so the UI only draws.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::cpu;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Process {
    pub pid: u32,
    /// The `comm` field: the executable name, truncated to 15 bytes by the kernel.
    pub name: String,
    /// Full command line, or the kernel thread name in brackets.
    pub command: String,
    pub threads: u32,
    pub user: String,
    /// Resident set size in bytes.
    pub memory: u64,
    /// Share of total CPU capacity, so a fully busy core on an 8 core box is 12.5.
    pub cpu: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Pid,
    Program,
    Command,
    Threads,
    User,
    Memory,
    Cpu,
}

impl SortBy {
    pub const ALL: [SortBy; 7] = [
        SortBy::Pid,
        SortBy::Program,
        SortBy::Command,
        SortBy::Threads,
        SortBy::User,
        SortBy::Memory,
        SortBy::Cpu,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortBy::Pid => "pid",
            SortBy::Program => "program",
            SortBy::Command => "command",
            SortBy::Threads => "threads",
            SortBy::User => "user",
            SortBy::Memory => "mem",
            SortBy::Cpu => "cpu lazy",
        }
    }

    fn next(self, forward: bool) -> SortBy {
        let index = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        let len = Self::ALL.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        Self::ALL[next]
    }
}

/// Per-process CPU counters from the previous sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Ticks {
    busy: u64,
}

pub struct Processes {
    list: Vec<Process>,
    previous: HashMap<u32, Ticks>,
    previous_total: u64,
    users: HashMap<u32, String>,
    page_size: u64,
    pub sort: SortBy,
    pub reverse: bool,
    selected: usize,
}

impl Default for Processes {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            previous: HashMap::new(),
            previous_total: 0,
            users: HashMap::new(),
            page_size: page_size(),
            // btop opens sorted by CPU, which is what you almost always want.
            sort: SortBy::Cpu,
            reverse: false,
            selected: 0,
        }
    }
}

impl Processes {
    pub fn new() -> Self {
        let mut processes = Self {
            users: read_users(Path::new("/")),
            ..Self::default()
        };
        // Seed the counters so the first visible tick is a real delta rather
        // than an average over the process's whole lifetime.
        processes.tick();
        processes
    }

    pub fn all(&self) -> &[Process] {
        &self.list
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn tick(&mut self) {
        self.tick_in(Path::new("/"));
    }

    fn tick_in(&mut self, root: &Path) {
        let total = cpu::sample().total.total;
        // The whole machine's tick delta is the denominator, so a process pinning
        // one core of eight reads as 12.5% rather than 100%.
        let elapsed = total.saturating_sub(self.previous_total);
        self.previous_total = total;

        let proc_dir = root.join("proc");
        let Ok(entries) = fs::read_dir(&proc_dir) else {
            self.list.clear();
            return;
        };

        let mut next = Vec::with_capacity(self.list.len().max(64));
        let mut seen = HashMap::with_capacity(self.list.len().max(64));

        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            // A process can exit between the readdir and the read, so every
            // failure here just means "gone" and skips the entry.
            let Some(mut process) = self.read_process(&entry.path(), pid) else {
                continue;
            };

            let busy = process.cpu as u64;
            seen.insert(pid, Ticks { busy });
            process.cpu = self.cpu_percent(pid, busy, elapsed);
            next.push(process);
        }

        self.previous = seen;
        self.list = next;
        self.sort_list();
        self.clamp_selection();
    }

    /// Parse one process, carrying raw busy ticks in `cpu` for the caller to
    /// convert once the total delta is known.
    fn read_process(&self, dir: &Path, pid: u32) -> Option<Process> {
        let stat = fs::read_to_string(dir.join("stat")).ok()?;
        let parsed = parse_stat(&stat)?;

        let user = fs::metadata(dir)
            .ok()
            .and_then(|meta| self.users.get(&meta.uid()).cloned())
            .unwrap_or_else(|| "?".to_string());

        Some(Process {
            pid,
            command: read_command(dir).unwrap_or_else(|| format!("[{}]", parsed.name)),
            name: parsed.name,
            threads: parsed.threads,
            user,
            memory: parsed.rss_pages.saturating_mul(self.page_size),
            cpu: parsed.busy_ticks as f64,
        })
    }

    fn cpu_percent(&self, pid: u32, busy: u64, elapsed: u64) -> f64 {
        if elapsed == 0 {
            return 0.0;
        }
        let previous = self.previous.get(&pid).copied().unwrap_or_default();
        // A process that appeared since the last sample has no baseline, and its
        // lifetime total would read as a huge spike.
        if !self.previous.contains_key(&pid) {
            return 0.0;
        }
        let delta = busy.saturating_sub(previous.busy);
        (delta as f64 / elapsed as f64 * 100.0).clamp(0.0, 100.0)
    }

    fn sort_list(&mut self) {
        match self.sort {
            SortBy::Pid => self.list.sort_by_key(|p| p.pid),
            SortBy::Program => self.list.sort_by(|a, b| a.name.cmp(&b.name)),
            SortBy::Command => self.list.sort_by(|a, b| a.command.cmp(&b.command)),
            SortBy::Threads => self.list.sort_by_key(|p| p.threads),
            SortBy::User => self.list.sort_by(|a, b| a.user.cmp(&b.user)),
            SortBy::Memory => self.list.sort_by_key(|p| p.memory),
            // Floats have no total order, so this cannot be sort_by_key.
            SortBy::Cpu => self.list.sort_by(|a, b| {
                a.cpu
                    .partial_cmp(&b.cpu)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        // Numeric columns are most useful largest-first, which is the opposite of
        // what a plain sort gives, so descending is the default.
        if !self.reverse {
            self.list.reverse();
        }
    }

    pub fn cycle_sort(&mut self, forward: bool) {
        self.sort = self.sort.next(forward);
        self.sort_list();
        self.selected = 0;
    }

    pub fn toggle_reverse(&mut self) {
        self.reverse = !self.reverse;
        self.sort_list();
        self.selected = 0;
    }

    /// Move the selection, saturating at both ends rather than wrapping.
    pub fn move_selection(&mut self, delta: isize) {
        if self.list.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.list.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.list.len().saturating_sub(1);
    }

    fn clamp_selection(&mut self) {
        // Processes exit constantly, so the list shrinks under the cursor.
        self.selected = self.selected.min(self.list.len().saturating_sub(1));
    }
}

struct Stat {
    name: String,
    threads: u32,
    rss_pages: u64,
    busy_ticks: u64,
}

/// Parse `/proc/[pid]/stat`.
///
/// The `comm` field is wrapped in parentheses and may itself contain spaces and
/// parentheses, so the only safe split is at the *last* `)`. Splitting on
/// whitespace, or on the first `)`, misreads every field for a process named
/// something like `foo (bar)`.
fn parse_stat(raw: &str) -> Option<Stat> {
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    let name = raw.get(open + 1..close)?.to_string();

    // Fields resume at `state`, which is field 3 in the man page's numbering.
    let fields: Vec<&str> = raw.get(close + 1..)?.split_whitespace().collect();
    let field = |number: usize| -> u64 {
        fields
            .get(number - 3)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };

    Some(Stat {
        name,
        threads: field(20) as u32,
        rss_pages: field(24),
        // utime + stime: user and system time, both in clock ticks.
        busy_ticks: field(14) + field(15),
    })
}

/// The full command line, with NUL separators turned back into spaces.
///
/// Kernel threads have an empty `cmdline`, which is how you tell them apart.
fn read_command(dir: &Path) -> Option<String> {
    let raw = fs::read(dir.join("cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    let text = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

/// uid to username, from `/etc/passwd`.
///
/// Read once at startup: it is a file lookup per process otherwise, and the
/// mapping does not change while a monitor is open.
fn read_users(root: &Path) -> HashMap<u32, String> {
    let raw = fs::read_to_string(root.join("etc/passwd")).unwrap_or_default();
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

fn page_size() -> u64 {
    // SAFETY: sysconf takes an integer and returns one, touching no memory we
    // own. A negative result means the name is unsupported, which cannot happen
    // for _SC_PAGESIZE, but is handled anyway.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as u64
    } else {
        4096
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_normal_stat_line() {
        let raw = "125045 (cat) R 125025 125045 125025 0 -1 4194304 92 0 0 0 \
            11 22 0 0 20 0 7 0 965989 8720384 480 18446744073709551615";
        let stat = parse_stat(raw).unwrap();
        assert_eq!(stat.name, "cat");
        assert_eq!(stat.busy_ticks, 33, "utime 11 + stime 22");
        assert_eq!(stat.threads, 7);
        assert_eq!(stat.rss_pages, 480);
    }

    #[test]
    fn comm_may_contain_spaces_and_parentheses() {
        // This is the classic /proc/pid/stat trap. Splitting on whitespace or on
        // the first ')' shifts every subsequent field.
        let raw = "42 (my (weird) app) S 1 42 1 0 -1 0 0 0 0 0 \
            5 5 0 0 20 0 3 0 100 200 64 0";
        let stat = parse_stat(raw).unwrap();
        assert_eq!(stat.name, "my (weird) app");
        assert_eq!(stat.busy_ticks, 10);
        assert_eq!(stat.threads, 3);
        assert_eq!(stat.rss_pages, 64);
    }

    #[test]
    fn malformed_stat_lines_are_rejected_not_panicked_on() {
        assert!(parse_stat("").is_none());
        assert!(parse_stat("123 no parens here").is_none());
        // Truncated after comm: fields default to zero rather than panicking.
        let stat = parse_stat("1 (init) S").unwrap();
        assert_eq!(stat.threads, 0);
        assert_eq!(stat.busy_ticks, 0);
    }

    #[test]
    fn cmdline_nuls_become_spaces() {
        let dir = tempdir();
        fs::write(dir.join("cmdline"), b"/bin/ls\0-la\0/tmp\0").unwrap();
        assert_eq!(read_command(&dir).as_deref(), Some("/bin/ls -la /tmp"));
    }

    #[test]
    fn empty_cmdline_means_kernel_thread() {
        let dir = tempdir();
        fs::write(dir.join("cmdline"), b"").unwrap();
        assert_eq!(read_command(&dir), None);
        fs::write(dir.join("cmdline"), b"\0\0\0").unwrap();
        assert_eq!(read_command(&dir), None, "only separators is still empty");
    }

    #[test]
    fn passwd_maps_uid_to_name() {
        let dir = tempdir();
        fs::create_dir_all(dir.join("etc")).unwrap();
        fs::write(
            dir.join("etc/passwd"),
            "root:x:0:0:root:/root:/bin/bash\nlovestaco:x:1000:1000::/home/l:/bin/bash\n",
        )
        .unwrap();
        let users = read_users(&dir);
        assert_eq!(users.get(&0).map(String::as_str), Some("root"));
        assert_eq!(users.get(&1000).map(String::as_str), Some("lovestaco"));
    }

    #[test]
    fn missing_passwd_is_not_an_error() {
        assert!(read_users(Path::new("/nonexistent-ratatop-test-root")).is_empty());
    }

    #[test]
    fn sort_cycles_through_every_column_and_wraps() {
        let mut seen = vec![SortBy::Cpu];
        let mut sort = SortBy::Cpu;
        for _ in 0..SortBy::ALL.len() - 1 {
            sort = sort.next(true);
            assert!(!seen.contains(&sort), "{sort:?} repeated early");
            seen.push(sort);
        }
        assert_eq!(sort.next(true), SortBy::Cpu, "wraps back to the start");
        assert_eq!(
            SortBy::Cpu.next(false).next(true),
            SortBy::Cpu,
            "reversible"
        );
    }

    #[test]
    fn sorting_orders_descending_by_default() {
        let mut processes = fixture();
        processes.sort = SortBy::Memory;
        processes.sort_list();
        let memories: Vec<u64> = processes.list.iter().map(|p| p.memory).collect();
        assert_eq!(memories, vec![300, 200, 100], "largest first");

        processes.toggle_reverse();
        let memories: Vec<u64> = processes.list.iter().map(|p| p.memory).collect();
        assert_eq!(memories, vec![100, 200, 300]);
    }

    #[test]
    fn sorting_by_cpu_handles_floats() {
        let mut processes = fixture();
        processes.sort = SortBy::Cpu;
        processes.sort_list();
        let cpus: Vec<f64> = processes.list.iter().map(|p| p.cpu).collect();
        assert_eq!(cpus, vec![9.5, 2.5, 0.0]);
    }

    #[test]
    fn selection_saturates_at_both_ends() {
        let mut processes = fixture();
        processes.move_selection(-5);
        assert_eq!(processes.selected(), 0, "cannot go above the first row");

        processes.move_selection(100);
        assert_eq!(processes.selected(), 2, "cannot go past the last row");

        processes.select_first();
        assert_eq!(processes.selected(), 0);
        processes.select_last();
        assert_eq!(processes.selected(), 2);
    }

    #[test]
    fn selection_survives_the_list_shrinking() {
        let mut processes = fixture();
        processes.select_last();
        assert_eq!(processes.selected(), 2);

        // Processes exit constantly; the cursor must not dangle past the end.
        processes.list.truncate(1);
        processes.clamp_selection();
        assert_eq!(processes.selected(), 0);

        processes.list.clear();
        processes.clamp_selection();
        assert_eq!(processes.selected(), 0, "empty list is still index 0");
    }

    #[test]
    fn an_empty_list_ignores_movement() {
        let mut processes = Processes::default();
        processes.move_selection(3);
        assert_eq!(processes.selected(), 0);
    }

    #[test]
    fn a_new_process_reports_zero_rather_than_its_lifetime() {
        let processes = Processes::default();
        // pid 999 has no previous sample, so its total ticks must not be read as
        // a delta.
        assert_eq!(processes.cpu_percent(999, 50_000, 100), 0.0);
    }

    #[test]
    fn cpu_percent_is_a_share_of_the_whole_machine() {
        let mut processes = Processes::default();
        processes.previous.insert(7, Ticks { busy: 100 });
        // 25 of 200 total ticks is 12.5%, which is one busy core out of eight.
        assert!((processes.cpu_percent(7, 125, 200) - 12.5).abs() < 1e-9);
        assert_eq!(processes.cpu_percent(7, 125, 0), 0.0, "no time elapsed");
    }

    #[test]
    fn reads_the_real_system() {
        let mut processes = Processes::new();
        processes.tick();
        assert!(processes.all().len() > 10, "a live machine has processes");

        let us = processes
            .all()
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("our own process must be listed");
        assert!(!us.name.is_empty());
        assert!(us.memory > 0, "we are resident");
        assert!(us.threads >= 1);
        assert!((0.0..=100.0).contains(&us.cpu));
    }

    #[test]
    fn kernel_threads_get_a_bracketed_command() {
        let processes = Processes::new();
        // kthreadd is pid 2 on every Linux system and has no cmdline.
        if let Some(kthreadd) = processes.all().iter().find(|p| p.pid == 2) {
            assert!(
                kthreadd.command.starts_with('['),
                "expected bracketed name, got {:?}",
                kthreadd.command
            );
        }
    }

    #[test]
    fn a_missing_proc_yields_an_empty_list() {
        let mut processes = Processes::default();
        processes.tick_in(Path::new("/nonexistent-ratatop-test-root"));
        assert!(processes.all().is_empty());
    }

    fn fixture() -> Processes {
        let list = vec![
            Process {
                pid: 1,
                memory: 100,
                cpu: 2.5,
                ..Process::default()
            },
            Process {
                pid: 2,
                memory: 300,
                cpu: 0.0,
                ..Process::default()
            },
            Process {
                pid: 3,
                memory: 200,
                cpu: 9.5,
                ..Process::default()
            },
        ];
        Processes {
            list,
            ..Processes::default()
        }
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ratatop-proc-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }
}
