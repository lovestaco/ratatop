//! The process box.
//!
//! A scrolling table with a selection cursor, a sortable header, and a key hint
//! bar on the bottom border, following btop's arrangement.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    StatefulWidget, Widget,
};

use crate::app::App;
use crate::collect::proc::{Process, SortBy};
use crate::theme;

use super::mem::format_bytes;

const PID_WIDTH: u16 = 7;
const PROGRAM_WIDTH: u16 = 14;
const THREADS_WIDTH: u16 = 9;
const USER_WIDTH: u16 = 11;
const MEMORY_WIDTH: u16 = 7;
const CPU_WIDTH: u16 = 6;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIV_LINE))
        .title(Line::from(vec![
            Span::styled("p", Style::default().fg(theme::HI_FG)),
            Span::styled("roc", Style::default().fg(theme::TITLE)),
        ]))
        .title(options(app).alignment(Alignment::Right))
        .title_bottom(hints())
        .title_bottom(count(app).alignment(Alignment::Right));

    let inner = block.inner(area);
    block.render(area, buf);
    if inner.is_empty() {
        return;
    }

    let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    render_header(app, header, buf);

    // Leave the last column for the scrollbar so a long command never runs
    // underneath it.
    let [rows, scrollbar] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(body);

    let processes = app.processes.all();
    let visible = rows.height as usize;
    let offset = scroll_offset(app.processes.selected(), visible, processes.len());

    for (index, process) in processes.iter().skip(offset).take(visible).enumerate() {
        let row = Rect::new(rows.x, rows.y + index as u16, rows.width, 1);
        let selected = offset + index == app.processes.selected();
        render_row(process, row, selected, buf);
    }

    render_scrollbar(offset, visible, processes.len(), scrollbar, buf);
}

/// Keep the selected row on screen, scrolling only when it would fall off.
///
/// Returned as an offset rather than stored, so a terminal resize cannot leave a
/// stale scroll position behind.
fn scroll_offset(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    let last_offset = total - visible;
    // Centre the cursor once there is room on both sides of it.
    selected.saturating_sub(visible / 2).min(last_offset)
}

fn render_header(app: &App, area: Rect, buf: &mut Buffer) {
    let columns = [
        (SortBy::Pid, "Pid:", PID_WIDTH, Alignment::Right),
        (SortBy::Program, "Program:", PROGRAM_WIDTH, Alignment::Left),
        (SortBy::Command, "Command:", 0, Alignment::Left),
        (SortBy::Threads, "Threads:", THREADS_WIDTH, Alignment::Right),
        (SortBy::User, "User:", USER_WIDTH, Alignment::Left),
        (SortBy::Memory, "MemB", MEMORY_WIDTH, Alignment::Right),
        (SortBy::Cpu, "Cpu%", CPU_WIDTH, Alignment::Right),
    ];

    let areas = column_areas(area);
    for ((sort, label, _, alignment), column) in columns.iter().zip(areas) {
        let active = *sort == app.processes.sort;
        // The arrow marks both which column sorts and which way it runs.
        let text = if active {
            let arrow = if app.processes.reverse { "↓" } else { "↑" };
            format!("{label}{arrow}")
        } else {
            label.to_string()
        };
        let style = if active {
            Style::default()
                .fg(theme::HI_FG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TITLE)
        };
        Paragraph::new(Line::from(Span::styled(text, style)))
            .alignment(*alignment)
            .render(column, buf);
    }
}

fn render_row(process: &Process, area: Rect, selected: bool, buf: &mut Buffer) {
    if selected {
        buf.set_style(area, Style::default().bg(theme::SELECTED_BG));
    }

    let base = if selected {
        Style::default()
            .fg(theme::TITLE)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::MAIN_FG)
    };

    let [pid, program, command, threads, user, memory, cpu] = column_areas(area);

    cell(&process.pid.to_string(), base, Alignment::Right, pid, buf);
    cell(
        &process.name,
        base.fg(if selected { theme::TITLE } else { theme::HI_FG }),
        Alignment::Left,
        program,
        buf,
    );
    cell(
        &process.command,
        if selected {
            base
        } else {
            base.fg(theme::INACTIVE_FG)
        },
        Alignment::Left,
        command,
        buf,
    );
    cell(
        &process.threads.to_string(),
        base,
        Alignment::Right,
        threads,
        buf,
    );
    cell(&process.user, base, Alignment::Left, user, buf);
    cell(
        &format_memory(process.memory),
        base,
        Alignment::Right,
        memory,
        buf,
    );
    cell(
        &format!("{:.1}", process.cpu),
        base.fg(theme::CPU.sample(process.cpu / 100.0)),
        Alignment::Right,
        cpu,
        buf,
    );
}

fn cell(text: &str, style: Style, alignment: Alignment, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    Paragraph::new(Line::from(Span::styled(text.to_string(), style)))
        .alignment(alignment)
        .render(area, buf);
}

/// One layout, shared by the header and every row, so the columns line up.
fn column_areas(area: Rect) -> [Rect; 7] {
    Layout::horizontal([
        Constraint::Length(PID_WIDTH),
        Constraint::Length(PROGRAM_WIDTH),
        // Command absorbs the slack, since it is the one that benefits.
        Constraint::Min(0),
        Constraint::Length(THREADS_WIDTH),
        Constraint::Length(USER_WIDTH),
        Constraint::Length(MEMORY_WIDTH),
        Constraint::Length(CPU_WIDTH),
    ])
    .spacing(1)
    .areas(area)
}

fn render_scrollbar(offset: usize, visible: usize, total: usize, area: Rect, buf: &mut Buffer) {
    if area.is_empty() || total <= visible {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(visible)).position(offset);
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"))
        .thumb_style(Style::default().fg(theme::HI_FG))
        .track_style(Style::default().fg(theme::DIV_LINE))
        .render(area, buf, &mut state);
}

/// `reverse  cpu lazy` on the top right.
fn options(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    if app.processes.reverse {
        spans.push(Span::styled(
            "reverse ",
            Style::default().fg(theme::MAIN_FG),
        ));
    }
    spans.push(Span::styled(
        app.processes.sort.label(),
        Style::default()
            .fg(theme::HI_FG)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn hints() -> Line<'static> {
    Line::from(vec![
        Span::styled(" ↑↓ ", Style::default().fg(theme::HI_FG)),
        Span::styled("select ", Style::default().fg(theme::DIV_LINE)),
        Span::styled(" ←→ ", Style::default().fg(theme::HI_FG)),
        Span::styled("sort ", Style::default().fg(theme::DIV_LINE)),
        Span::styled(" r ", Style::default().fg(theme::HI_FG)),
        Span::styled("reverse ", Style::default().fg(theme::DIV_LINE)),
    ])
}

/// `12/399` on the bottom right: where the cursor is, and how many there are.
fn count(app: &App) -> Line<'static> {
    let total = app.processes.all().len();
    let position = if total == 0 {
        0
    } else {
        app.processes.selected() + 1
    };
    Line::from(Span::styled(
        format!(" {position}/{total} "),
        Style::default().fg(theme::MAIN_FG),
    ))
}

/// Memory in the narrow form btop uses in the table: `642M`, `6.5M`.
fn format_memory(bytes: u64) -> String {
    let text = format_bytes(bytes);
    // "642 MiB" to "642M": the column is too narrow for the full unit, and the
    // header already says the values are bytes.
    match text.split_once(' ') {
        Some((value, unit)) => format!("{value}{}", unit.chars().next().unwrap_or(' ')),
        None => text,
    }
}

/// Rows the box wants, given how much is left over.
pub fn preferred_height(available: u16) -> u16 {
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &Buffer) -> String {
        let width = buf.area.width as usize;
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw(app: &App, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(app, area, &mut buf);
        buf
    }

    #[test]
    fn renders_the_header_and_some_processes() {
        let app = App::new();
        let rendered = text(&draw(&app, 120, 20));
        for label in ["Pid:", "Program:", "Command:", "Threads:", "User:", "Cpu%"] {
            assert!(rendered.contains(label), "{label} missing");
        }
        assert!(rendered.contains("proc"), "title missing");
        assert!(rendered.contains("select"), "key hints missing");
    }

    #[test]
    fn the_sorted_column_is_marked_with_a_direction() {
        let mut app = App::new();
        let rendered = text(&draw(&app, 120, 12));
        assert!(rendered.contains("Cpu%↑"), "default sort not marked");

        app.processes.toggle_reverse();
        let rendered = text(&draw(&app, 120, 12));
        assert!(rendered.contains("Cpu%↓"), "reversed arrow not shown");
        assert!(rendered.contains("reverse"), "reverse not in the options");
    }

    #[test]
    fn changing_the_sort_moves_the_marker() {
        let mut app = App::new();
        app.processes.cycle_sort(true);
        let rendered = text(&draw(&app, 120, 12));
        assert!(!rendered.contains("Cpu%↑"), "old column still marked");
        assert!(
            rendered.contains("Pid:↑"),
            "cpu should cycle to pid: {rendered}"
        );
    }

    #[test]
    fn the_counter_shows_position_and_total() {
        let mut app = App::new();
        let total = app.processes.all().len();
        assert!(text(&draw(&app, 120, 12)).contains(&format!("1/{total}")));

        app.processes.move_selection(3);
        assert!(text(&draw(&app, 120, 12)).contains(&format!("4/{total}")));
    }

    #[test]
    fn scrolling_keeps_the_selection_on_screen() {
        // Ten rows visible, a hundred processes: the offset has to follow.
        assert_eq!(scroll_offset(0, 10, 100), 0, "top of the list");
        assert_eq!(scroll_offset(3, 10, 100), 0, "still fits without scrolling");
        assert_eq!(scroll_offset(50, 10, 100), 45, "cursor centred");
        assert_eq!(scroll_offset(99, 10, 100), 90, "clamped at the bottom");
    }

    #[test]
    fn a_short_list_never_scrolls() {
        assert_eq!(scroll_offset(2, 10, 5), 0);
        assert_eq!(scroll_offset(0, 0, 100), 0, "zero height is safe");
        assert_eq!(scroll_offset(0, 10, 0), 0, "empty list is safe");
    }

    #[test]
    fn the_selected_row_is_highlighted() {
        let app = App::new();
        let buf = draw(&app, 120, 12);
        // The first body row sits below the border and the header.
        let selected_row_bg = buf[(5, 2)].bg;
        let other_row_bg = buf[(5, 3)].bg;
        assert_eq!(selected_row_bg, theme::SELECTED_BG);
        assert_ne!(other_row_bg, theme::SELECTED_BG);
    }

    #[test]
    fn memory_is_abbreviated_to_fit_the_column() {
        assert_eq!(format_memory(642 * 1024 * 1024), "642M");
        assert_eq!(format_memory(0), "0B");
        assert!(
            format_memory(6_500_000).len() <= MEMORY_WIDTH as usize,
            "must fit the column"
        );
    }

    #[test]
    fn columns_line_up_between_header_and_rows() {
        // The header and every row must use the same layout, or the table skews.
        let area = Rect::new(0, 0, 120, 1);
        let header = column_areas(area);
        let row = column_areas(area);
        assert_eq!(header, row);
        // Command takes the slack.
        assert!(header[2].width > PROGRAM_WIDTH);
    }

    #[test]
    fn tiny_and_empty_areas_are_safe() {
        let app = App::new();
        for width in 0..20 {
            for height in 0..6 {
                draw(&app, width, height);
            }
        }
    }
}
