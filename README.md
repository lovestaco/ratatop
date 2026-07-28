# ratatop

A reimplementation of [btop](https://github.com/aristocratos/btop) in Rust, built on
[ratatui](https://ratatui.rs).

The goal is a resource monitor that feels like btop — the four-box layout, braille
graphs with gradient fills, mouse-driven navigation, and a themable process
manager — but written as a modern Rust TUI.

## Status

**The CPU box works.** It samples real data and renders btop's layout:

```
┌BAT⚡100% ▰▰▰▰▰▰▰▰▰▰──────────────────────────────2000ms┐
│                   AMD Ryzen 5 3550H            1.4 GHz│
│CPU          ⣀⣀⣀⢀⣀⡀⢀⣀⢀⢀⡀⡀⣀⣀⢀⣀⣀⣀⣀⢀⡀⣀⣀⡀⢀⣀⣀⣀⢀⢀    4%  75°C│
│C0             ⢀⢠ ⡄⢀⡄⡀  ⡀   ⣀⣀     ⢀⡀   ⡀            0%│
│C1            ⢀   ⡀⢀  ⢀⡀   ⡀⡀ ⢀ ⢀ ⢀⡀  ⢀ ⢀            0%│
│…                                                      │
│Load AVG:   0.41   0.93   1.28                         │
└───────────────────────────────────────────────────────┘
```

Everything else — the memory, network, and process boxes — is still to come.

This directory also holds the upstream references used to guide the port:

| Path       | What it is                                | Upstream                                  |
| ---------- | ----------------------------------------- | ----------------------------------------- |
| `btop/`    | The btop C++ source — the model to follow | https://github.com/aristocratos/btop      |
| `ratatui/` | The ratatui TUI library                   | https://github.com/ratatui/ratatui        |
| `htop/`    | htop C source — secondary reference        | https://github.com/htop-dev/htop          |

None of these are dependencies to be edited — they are read-only references.
ratatui will be pulled in from crates.io when the app crate is created.

## Planned scope

**v1 — the four boxes**

- [x] CPU box: per-core load, frequency, temperature, load averages, battery
- [ ] Memory box: RAM/swap gauges plus disk usage and I/O per mount
- [ ] Network box: up/down graphs with auto-scaling units and per-interface totals
- [ ] Process box: PID, program, command, threads, user, MEM%, CPU%, sortable

**v2 — interaction**

- Mouse support throughout: click to sort, click a box to focus, scroll the list
- Process detail view — expanded stats and a per-process CPU graph
- Filter (`f`), tree view (`e`), signal send / kill
- Box toggling (`1`–`4`) and preset layouts

**v3 — polish**

- Menu (`Esc`) for options, matching btop's config surface
- Theme files compatible with btop's `themes/*.theme` format
- Config persistence in `~/.config/ratatop/`
- Graph symbol modes: braille, block, TTY

## Design notes

- **Data source:** `/proc` and `/sys` on Linux first, read directly rather than
  shelling out. btop's `src/linux/btop_collect.cpp` is the reference for what to
  read and how often; the BSD/macOS collectors are out of scope for v1.
- **Graphs are the hard part.** btop's look comes from braille sub-cell plotting
  with a color gradient applied along the value axis, not from a stock chart
  widget. Expect a custom ratatui widget rather than `Sparkline`/`Chart`.
- **CPU and network values are deltas** between samples, so the collector retains
  previous counters per core, per interface, and per process — same as btop.
- **Rendering:** ratatui's immediate-mode draw loop. Keep collection off the
  render path so a slow `/proc` sweep never stalls input.
- **Layout:** four resizable boxes. btop lets the user toggle boxes on and off and
  reflows the rest, so the layout should be computed from an enabled-box set
  rather than hardcoded constraints.

## Layout

```
ratatop/
├── src/
│   ├── main.rs          # terminal setup, event loop, box placement
│   ├── app.rs           # state and per-tick history
│   ├── collect/         # /proc + /sys readers, delta tracking
│   │   ├── cpu.rs       # /proc/stat, cpufreq, hwmon, loadavg
│   │   └── battery.rs   # /sys/class/power_supply
│   ├── theme.rs         # btop's default colors and gradients
│   ├── widgets/         # braille graph, gradient meter
│   └── ui.rs            # the CPU box
├── examples/snapshot.rs # render one frame as plain text
├── btop/                # reference only
├── htop/                # reference only
└── ratatui/             # reference only
```

## Getting started

```sh
make run        # launch it (q or Esc quits)
make test       # 46 tests
make ci         # fmt-check + clippy -D warnings + test
make snapshot   # print one frame as text, no terminal needed
make help       # every target
```

`make snapshot` is the quickest way to check layout changes — it dumps the
rendered buffer to stdout, so it works over a pipe and in CI.

Run a single test with `make test-one T=<name>`; `T` also accepts a module path,
so `make test-one T=widgets::braille` runs that whole module.

## Why

btop looks great but is ~20k lines of C++ with hand-rolled ANSI drawing. ratatui
provides layout, styling, and event handling, so the interesting work becomes the
collectors and the graph rendering rather than terminal plumbing. It's also a good
excuse to read a mature systems codebase closely.
