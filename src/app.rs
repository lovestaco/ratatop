//! Application state: the latest readings plus the history the graphs draw.

use std::time::Duration;

use crate::collect::battery::{self, Battery};
use crate::collect::cpu::{self, CpuInfo, Sample};
use crate::collect::disk::Disks;
use crate::collect::mem::{self, Memory};
use crate::collect::net::Net;
use crate::collect::proc::Processes;

/// How many samples to retain per graph.
///
/// One cell column holds two samples, so this covers a 256-column terminal.
/// Older samples are dropped rather than downsampled — btop does the same.
const HISTORY: usize = 512;

/// btop's default `update_ms`.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(2000);
/// btop's `update_ms` bounds.
const MIN_INTERVAL: Duration = Duration::from_millis(100);
const MAX_INTERVAL: Duration = Duration::from_millis(10_000);

pub struct App {
    pub info: CpuInfo,
    pub interval: Duration,
    /// Aggregate CPU load, oldest first.
    pub total_history: Vec<f64>,
    /// Per-core load, oldest first.
    pub core_history: Vec<Vec<f64>>,
    /// Package temperature in °C, oldest first.
    pub temp_history: Vec<f64>,
    pub frequency_ghz: Option<f64>,
    pub temperature: Option<f64>,
    pub load_average: [f64; 3],
    pub memory: Memory,
    pub disks: Disks,
    pub net: Net,
    pub processes: Processes,
    pub battery: Option<Battery>,
    battery_source: Option<battery::Source>,
    previous: Sample,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        let info = CpuInfo::detect();
        let battery_source = battery::detect();
        // Seed the counters so the first tick reports a real delta instead of
        // uptime-average load.
        let previous = cpu::sample();
        let cores = previous.cores.len();

        Self {
            info,
            interval: DEFAULT_INTERVAL,
            total_history: Vec::with_capacity(HISTORY),
            core_history: vec![Vec::with_capacity(HISTORY); cores],
            temp_history: Vec::with_capacity(HISTORY),
            frequency_ghz: None,
            temperature: None,
            load_average: [0.0; 3],
            memory: mem::sample(),
            disks: Disks::new(),
            net: Net::new(),
            processes: Processes::new(),
            battery: battery_source.as_ref().and_then(battery::read),
            battery_source,
            previous,
            should_quit: false,
        }
    }

    /// Take one reading and fold it into the history.
    pub fn tick(&mut self) {
        let sample = cpu::sample();

        push(
            &mut self.total_history,
            sample.total.percent_since(self.previous.total),
        );

        // Core count can change under hotplug, so grow the history to match
        // rather than indexing past the end.
        if self.core_history.len() < sample.cores.len() {
            self.core_history
                .resize_with(sample.cores.len(), || Vec::with_capacity(HISTORY));
        }
        for (index, core) in sample.cores.iter().enumerate() {
            let previous = self.previous.cores.get(index).copied().unwrap_or_default();
            push(&mut self.core_history[index], core.percent_since(previous));
        }

        self.previous = sample;
        self.frequency_ghz = cpu::frequency_ghz();
        self.temperature = self.info.temperature();
        if let Some(celsius) = self.temperature {
            push(&mut self.temp_history, celsius);
        }
        self.load_average = cpu::load_average();
        self.memory = mem::sample();
        self.disks.tick();
        self.net.tick();
        self.processes.tick();
        self.battery = self.battery_source.as_ref().and_then(battery::read);
    }

    /// Step the sample interval, the way btop's `+`/`-` keys do.
    ///
    /// Steps by a proportion rather than a fixed amount so the control feels the
    /// same at 100 ms as at 10 s.
    pub fn adjust_interval(&mut self, faster: bool) {
        let millis = self.interval.as_millis() as u64;
        let step = (millis / 4).max(100);
        let next = if faster {
            millis.saturating_sub(step)
        } else {
            millis.saturating_add(step)
        };
        self.interval = Duration::from_millis(next).clamp(MIN_INTERVAL, MAX_INTERVAL);
    }

    /// The newest aggregate reading.
    pub fn total_percent(&self) -> f64 {
        self.total_history.last().copied().unwrap_or(0.0)
    }

    /// The newest reading for a core.
    pub fn core_percent(&self, index: usize) -> f64 {
        self.core_history
            .get(index)
            .and_then(|history| history.last())
            .copied()
            .unwrap_or(0.0)
    }

    pub fn core_count(&self) -> usize {
        self.core_history.len()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn push(history: &mut Vec<f64>, value: f64) {
    if history.len() >= HISTORY {
        history.remove(0);
    }
    history.push(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_capped_and_keeps_the_newest_values() {
        let mut history = Vec::new();
        for value in 0..(HISTORY + 10) {
            push(&mut history, value as f64);
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history.last(), Some(&((HISTORY + 9) as f64)));
        assert_eq!(history.first(), Some(&10.0), "oldest samples are dropped");
    }

    #[test]
    fn a_fresh_app_reports_zero_rather_than_panicking() {
        let app = App::new();
        assert_eq!(app.total_percent(), 0.0, "no delta available yet");
        assert_eq!(app.core_percent(9999), 0.0, "out of range core");
    }

    #[test]
    fn ticking_records_one_sample_per_core() {
        let mut app = App::new();
        app.tick();
        assert_eq!(app.total_history.len(), 1);
        assert_eq!(app.core_history.len(), app.info.core_count);
        for history in &app.core_history {
            assert_eq!(history.len(), 1);
        }
        app.tick();
        assert_eq!(app.total_history.len(), 2);
    }

    #[test]
    fn interval_steps_proportionally_and_stays_in_bounds() {
        let mut app = App::new();
        app.adjust_interval(true);
        assert_eq!(app.interval, Duration::from_millis(1500), "2000 - 2000/4");

        for _ in 0..50 {
            app.adjust_interval(true);
        }
        assert_eq!(app.interval, MIN_INTERVAL, "clamped, not underflowed");

        for _ in 0..100 {
            app.adjust_interval(false);
        }
        assert_eq!(app.interval, MAX_INTERVAL, "clamped, not overflowed");
    }

    #[test]
    fn readings_stay_in_range() {
        let mut app = App::new();
        app.tick();
        assert!((0.0..=100.0).contains(&app.total_percent()));
        for index in 0..app.core_count() {
            assert!((0.0..=100.0).contains(&app.core_percent(index)));
        }
    }
}
