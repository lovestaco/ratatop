//! Braille history graph.
//!
//! btop's graphs are not bar charts — each terminal cell carries a 2×4 grid of
//! braille dots, so one cell row resolves four levels and one cell column holds
//! two samples. `btop/src/btop_draw.cpp` precomputes the glyphs; we assemble the
//! bit pattern per cell instead.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::theme::Gradient;

/// Base codepoint of the braille block; dot bits are added to it.
const BRAILLE_BLANK: u32 = 0x2800;

/// Bit for each dot, indexed `[column][row]` over the 2×4 cell grid.
///
/// The braille block numbers dots 1-6 down the two columns and appends 7 and 8
/// as a fourth row, so the fourth entry breaks the otherwise regular pattern.
const DOTS: [[u8; 4]; 2] = [
    [0x01, 0x02, 0x04, 0x40], // left column, top to bottom
    [0x08, 0x10, 0x20, 0x80], // right column, top to bottom
];

/// A history graph filled from the bottom up, colored by value.
pub struct BrailleGraph<'a> {
    /// Percentages, oldest first. Only the newest samples that fit are drawn.
    data: &'a [f64],
    gradient: Gradient,
    /// Fill downward from the top instead of upward from the bottom.
    inverted: bool,
}

impl<'a> BrailleGraph<'a> {
    pub fn new(data: &'a [f64], gradient: Gradient) -> Self {
        Self {
            data,
            gradient,
            inverted: false,
        }
    }

    /// Hang the graph from the top edge, growing downward.
    ///
    /// btop mirrors the upload graph against the download one so the pair reads
    /// as a single figure with traffic flowing out from the middle.
    pub fn inverted(mut self) -> Self {
        self.inverted = true;
        self
    }

    /// How many samples an area of this width can show.
    pub const fn capacity(width: u16) -> usize {
        width as usize * 2
    }
}

impl Widget for BrailleGraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        let columns = Self::capacity(area.width);
        let dot_rows = area.height as usize * 4;

        // Right-align: the newest sample sits at the right edge, and a partly
        // filled history is padded on the left rather than stretched.
        let visible = &self.data[self.data.len().saturating_sub(columns)..];
        let offset = columns - visible.len();

        for (index, &value) in visible.iter().enumerate() {
            let column = offset + index;
            let cell_x = area.x + (column / 2) as u16;
            let dot_column = column % 2;

            // Low readings round down to nothing, which is what gives btop's
            // graphs their dotted texture. Forcing a minimum of one dot instead
            // turns any steady load into a flat unbroken line.
            let filled = ((value.clamp(0.0, 100.0) / 100.0) * dot_rows as f64).round() as usize;
            if filled == 0 {
                continue;
            }

            let style = Style::default().fg(self.gradient.sample(value / 100.0));

            for dot in 0..filled.min(dot_rows) {
                // Dots stack from the graph's baseline: the bottom edge normally,
                // the top edge when inverted.
                let offset = if self.inverted {
                    dot
                } else {
                    dot_rows - 1 - dot
                };
                let cell_y = area.y + (offset / 4) as u16;
                let dot_row = offset % 4;

                let cell = &mut buf[(cell_x, cell_y)];
                let existing = cell
                    .symbol()
                    .chars()
                    .next()
                    .map_or(0, |c| (c as u32).saturating_sub(BRAILLE_BLANK));
                // Keep the sibling column's dots when the second sample lands in
                // the same cell.
                let bits = if existing < 0x100 { existing as u8 } else { 0 };
                let merged = bits | DOTS[dot_column][dot_row];

                if let Some(glyph) = char::from_u32(BRAILLE_BLANK + u32::from(merged)) {
                    cell.set_char(glyph);
                }
                // The newest sample owns the color of a shared cell.
                cell.set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    fn render(data: &[f64], width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(data, theme::CPU).render(area, &mut buf);
        buf
    }

    fn symbols(buf: &Buffer) -> String {
        buf.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn two_samples_share_one_cell() {
        assert_eq!(BrailleGraph::capacity(1), 2);
        assert_eq!(BrailleGraph::capacity(10), 20);
    }

    #[test]
    fn full_value_lights_every_dot_in_the_column() {
        let buf = render(&[100.0, 100.0], 1, 1);
        assert_eq!(symbols(&buf), "⣿");
    }

    #[test]
    fn a_single_sample_fills_only_its_own_column() {
        // Right column full, left column empty — the newest sample is right-aligned.
        let buf = render(&[100.0], 1, 1);
        assert_eq!(symbols(&buf), "⢸", "expected the right dot column only");
    }

    #[test]
    fn low_readings_round_down_to_blank() {
        assert_eq!(symbols(&render(&[0.0, 0.0], 1, 1)), " ");
        // A single cell row resolves four levels, so anything under ~12% is
        // blank. The gaps this leaves are what make the graph look dotted
        // instead of like a solid line.
        assert_eq!(symbols(&render(&[5.0, 5.0], 1, 1)), " ");
        assert_ne!(symbols(&render(&[20.0, 20.0], 1, 1)), " ");
    }

    #[test]
    fn fills_upward_from_the_bottom() {
        // Half of a two-cell-high graph is four dot rows: the lower cell only.
        let buf = render(&[50.0, 50.0], 1, 2);
        let cells = symbols(&buf);
        assert_eq!(cells.chars().next(), Some(' '), "top cell empty");
        assert_eq!(cells.chars().nth(1), Some('⣿'), "bottom cell full");
    }

    #[test]
    fn only_the_newest_samples_are_drawn() {
        // Capacity is two; the leading 100s must be dropped, not averaged in.
        let buf = render(&[100.0, 100.0, 100.0, 0.0, 0.0], 1, 1);
        assert_eq!(symbols(&buf), " ");
    }

    #[test]
    fn history_shorter_than_the_area_is_left_padded() {
        let buf = render(&[100.0, 100.0], 2, 1);
        assert_eq!(symbols(&buf), " ⣿", "newest sample hugs the right edge");
    }

    #[test]
    fn values_are_clamped_into_range() {
        assert_eq!(symbols(&render(&[500.0, 500.0], 1, 1)), "⣿");
        assert_eq!(symbols(&render(&[-20.0, -20.0], 1, 1)), " ");
    }

    #[test]
    fn color_tracks_the_value() {
        // Both values have to be high enough to light a dot, or there is no cell
        // to carry a color.
        let light = render(&[30.0, 30.0], 1, 1);
        let busy = render(&[100.0, 100.0], 1, 1);
        assert_eq!(light[(0, 0)].fg, theme::CPU.sample(0.30));
        assert_eq!(busy[(0, 0)].fg, theme::CPU.sample(1.0));
        assert_ne!(light[(0, 0)].fg, busy[(0, 0)].fg);
    }

    #[test]
    fn inverted_graphs_hang_from_the_top() {
        let area = Rect::new(0, 0, 1, 2);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[50.0, 50.0], theme::CPU)
            .inverted()
            .render(area, &mut buf);
        let cells = symbols(&buf);
        assert_eq!(cells.chars().next(), Some('\u{28ff}'), "top cell full");
        assert_eq!(cells.chars().nth(1), Some(' '), "bottom cell empty");
    }

    #[test]
    fn inverting_mirrors_the_normal_graph() {
        let normal = symbols(&render(&[25.0, 25.0], 1, 2));
        let area = Rect::new(0, 0, 1, 2);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[25.0, 25.0], theme::CPU)
            .inverted()
            .render(area, &mut buf);
        let flipped: String = symbols(&buf).chars().rev().collect();
        assert_eq!(normal.chars().filter(|c| *c != ' ').count(), 1);
        assert_eq!(
            flipped.chars().filter(|c| *c != ' ').count(),
            1,
            "same ink, opposite end: {normal:?} vs {:?}",
            symbols(&buf)
        );
    }

    #[test]
    fn empty_input_and_empty_area_are_safe() {
        assert_eq!(symbols(&render(&[], 4, 1)), "    ");
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        BrailleGraph::new(&[50.0], theme::CPU).render(Rect::new(0, 0, 0, 0), &mut buf);
    }
}
