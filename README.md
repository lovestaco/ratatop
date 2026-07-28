# ratatop

A reimplementation of [btop](https://github.com/aristocratos/btop) in Rust, built on
[ratatui](https://ratatui.rs).

The goal is a resource monitor that feels like btop — the four-box layout, braille
graphs with gradient fills, mouse-driven navigation, and a themable process
manager — but written as a modern Rust TUI.

## Status

Not started. This directory currently holds the upstream references used to guide
the port:

| Path       | What it is                                | Upstream                                  |
| ---------- | ----------------------------------------- | ----------------------------------------- |
| `btop/`    | The btop C++ source — the model to follow | https://github.com/aristocratos/btop      |
| `ratatui/` | The ratatui TUI library                   | https://github.com/ratatui/ratatui        |
| `htop/`    | htop C source — secondary reference        | https://github.com/htop-dev/htop          |

None of these are dependencies to be edited — they are read-only references.
ratatui will be pulled in from crates.io when the app crate is created.

## Planned scope

**v1 — the four boxes**

- CPU box: per-core load, frequency, temperature, and a scrolling history graph
- Memory box: RAM/swap gauges plus disk usage and I/O per mount
- Network box: up/down graphs with auto-scaling units and per-interface totals
- Process box: PID, program, command, threads, user, MEM%, CPU%, sortable

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

## Layout (planned)

```
ratatop/
├── src/
│   ├── main.rs          # terminal setup, event loop
│   ├── app.rs           # application state, input handling
│   ├── collect/         # /proc + /sys readers, delta tracking
│   ├── model/           # Cpu, Mem, Net, Process types
│   ├── theme/           # theme parsing, gradients
│   └── ui/              # cpu/mem/net/proc boxes, graph widget, menu
├── btop/                # reference only
├── htop/                # reference only
└── ratatui/             # reference only
```

## Getting started

Nothing to run yet. Once the crate exists:

```sh
cargo run --release
```

## Why

btop looks great but is ~20k lines of C++ with hand-rolled ANSI drawing. ratatui
provides layout, styling, and event handling, so the interesting work becomes the
collectors and the graph rendering rather than terminal plumbing. It's also a good
excuse to read a mature systems codebase closely.
