//! A bounded viewport snapshot keeps selected text stable while the PTY continues.
use ratatui::layout::{Position, Rect};

const MAX_CELLS: usize = 65_536;
pub const MAX_COPY_BYTES: usize = 128 * 1024;

pub struct Selection {
    pub area: Rect,
    pub viewport: Rect,
    pub dragging: bool,
    pub keyboard: bool,
    anchor: (u16, u16),
    end: (u16, u16),
    moved: bool,
    rows: Vec<Vec<vt100::Cell>>,
    wrapped: Vec<bool>,
}

impl Selection {
    pub fn capture(
        screen: &vt100::Screen,
        start_row: u16,
        mut area: Rect,
        point: Position,
    ) -> Option<Self> {
        let viewport = area;
        let (rows, cols) = screen.size();
        area.width = area.width.min(cols);
        area.height = area.height.min(rows.saturating_sub(start_row));
        if !area.contains(point) || usize::from(area.width) * usize::from(area.height) > MAX_CELLS {
            return None;
        }
        let rows = (start_row..start_row + area.height)
            .map(|row| {
                (0..area.width)
                    .filter_map(|col| screen.cell(row, col).cloned())
                    .collect()
            })
            .collect();
        let wrapped = (start_row..start_row + area.height)
            .map(|row| area.width == cols && screen.row_wrapped(row))
            .collect();
        let mut selection = Self {
            area,
            viewport,
            dragging: true,
            keyboard: false,
            anchor: (0, 0),
            end: (0, 0),
            moved: false,
            rows,
            wrapped,
        };
        selection.anchor = selection.point(point);
        selection.end = selection.anchor;
        Some(selection)
    }

    fn point(&self, point: Position) -> (u16, u16) {
        let row = point
            .y
            .saturating_sub(self.area.y)
            .min(self.area.height.saturating_sub(1));
        let mut col = point
            .x
            .saturating_sub(self.area.x)
            .min(self.area.width.saturating_sub(1));
        if self.rows[usize::from(row)][usize::from(col)].is_wide_continuation() {
            col = col.saturating_sub(1);
        }
        (row, col)
    }

    pub fn update(&mut self, point: Position) {
        self.end = self.point(point);
        self.moved |= self.end != self.anchor;
    }

    pub fn finish(&mut self, point: Position) -> Option<String> {
        self.update(point);
        self.dragging = false;
        self.moved
            .then(|| self.text())
            .filter(|text| !text.is_empty())
    }

    pub fn enable_keyboard(&mut self) {
        self.keyboard = true;
        self.dragging = false;
        self.moved = true;
    }

    pub fn move_end(&mut self, dx: i16, dy: i16, extend: bool) {
        let row = self
            .end
            .0
            .saturating_add_signed(dy)
            .min(self.area.height - 1);
        let col = self
            .end
            .1
            .saturating_add_signed(dx)
            .min(self.area.width - 1);
        let col = if dx > 0 && self.rows[usize::from(row)][usize::from(col)].is_wide_continuation()
        {
            col.saturating_add(1).min(self.area.width - 1)
        } else {
            col
        };
        self.end = self.point((self.area.x + col, self.area.y + row).into());
        if !extend {
            self.anchor = self.end;
        }
        self.moved = true;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.moved
            .then(|| self.text())
            .filter(|text| !text.is_empty())
    }

    pub fn has_range(&self) -> bool {
        self.moved
    }

    pub fn rows(&self) -> impl Iterator<Item = &[vt100::Cell]> {
        self.rows.iter().map(Vec::as_slice)
    }

    pub fn contains(&self, row: u16, mut col: u16) -> bool {
        if !self.moved {
            return false;
        }
        if self
            .rows
            .get(usize::from(row))
            .and_then(|r| r.get(usize::from(col)))
            .is_some_and(vt100::Cell::is_wide_continuation)
        {
            col = col.saturating_sub(1);
        }
        let (start, end) = (self.anchor.min(self.end), self.anchor.max(self.end));
        (row, col) >= start && (row, col) <= end
    }

    fn text(&self) -> String {
        let (start, end) = (self.anchor.min(self.end), self.anchor.max(self.end));
        let mut text = String::new();
        for row in start.0..=end.0 {
            if row > start.0 && !self.wrapped[usize::from(row - 1)] {
                text.push('\n');
            }
            let first = if row == start.0 { start.1 } else { 0 };
            let last = if row == end.0 {
                end.1
            } else {
                self.area.width - 1
            };
            let mut line = String::new();
            for cell in &self.rows[usize::from(row)][usize::from(first)..=usize::from(last)] {
                if cell.is_wide_continuation() {
                    continue;
                }
                line.push_str(if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                });
            }
            if row < end.0 && self.wrapped[usize::from(row)] {
                text.push_str(&line);
            } else {
                text.push_str(line.trim_end_matches(' '));
            }
        }
        text
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn snapshot(text: &[u8], rows: u16, cols: u16, anchor: (u16, u16)) -> Selection {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(text);
        Selection::capture(
            parser.screen(),
            0,
            Rect::new(0, 0, cols, rows),
            anchor.into(),
        )
        .unwrap()
    }

    #[test]
    fn reverse_multiline_selection_copies_plain_text_without_terminal_padding() {
        let mut selection = snapshot(b"\x1b[31malpha\x1b[0m\r\n  beta\r\ngamma", 4, 12, (2, 2));
        assert_eq!(
            selection.finish((1, 0).into()).unwrap(),
            "lpha\n  beta\ngam"
        );
        assert!(selection.contains(1, 0));
        assert!(!selection.contains(0, 0));
    }

    #[test]
    fn soft_wrapped_rows_join_without_an_inserted_newline() {
        let mut selection = snapshot(b"abcdefghi", 3, 6, (4, 0));
        assert_eq!(selection.finish((2, 1).into()).unwrap(), "efghi");
    }

    #[test]
    fn soft_wrapping_preserves_spaces_at_the_wrap_boundary() {
        let mut selection = snapshot(b"ab  cd ef", 3, 4, (0, 0));
        assert_eq!(selection.finish((0, 2).into()).unwrap(), "ab  cd ef");
    }
    #[test]
    fn unicode_cells_and_combining_marks_are_not_split() {
        let mut selection = snapshot("A界e\u{301}  ".as_bytes(), 2, 12, (2, 0));
        assert_eq!(selection.finish((3, 0).into()).unwrap(), "界e\u{301}");
        assert!(selection.contains(0, 1) && selection.contains(0, 2));
    }

    #[test]
    fn clicks_do_not_copy_and_drags_clamp_to_the_text_rectangle() {
        let mut selection = snapshot(b"one\r\ntwo", 2, 5, (0, 0));
        assert!(selection.finish((0, 0).into()).is_none());
        let mut selection = snapshot(b"one\r\ntwo", 2, 5, (0, 0));
        assert_eq!(selection.finish((500, 500).into()).unwrap(), "one\ntwo");
    }

    #[test]
    fn snapshot_excludes_history_and_is_stable_when_original_output_changes() {
        let mut parser = vt100::Parser::new(3, 20, 1000);
        parser.process(b"old\r\nvisible text\r\n");
        let mut selection =
            Selection::capture(parser.screen(), 1, Rect::new(1, 4, 20, 2), (1, 4).into()).unwrap();
        parser.process(b"new output\r\n");
        assert_eq!(selection.finish((7, 4).into()).unwrap(), "visible");
        assert_eq!(selection.rows.len(), 2);
    }

    #[test]
    fn excessive_viewports_and_non_content_clicks_are_rejected() {
        let parser = vt100::Parser::new(300, 300, 0);
        assert!(
            Selection::capture(parser.screen(), 0, Rect::new(0, 0, 300, 300), (0, 0).into())
                .is_none()
        );
        assert!(
            Selection::capture(parser.screen(), 0, Rect::new(1, 4, 20, 3), (0, 4).into()).is_none()
        );
    }
    #[test]
    fn keyboard_cursor_moves_over_wide_cells_and_extends_from_its_anchor() {
        let mut selection = snapshot("A界BCD".as_bytes(), 2, 12, (0, 0));
        selection.enable_keyboard();
        selection.move_end(1, 0, false);
        assert_eq!(selection.selected_text().unwrap(), "界");
        selection.move_end(1, 0, true);
        assert_eq!(selection.selected_text().unwrap(), "界B");
        selection.move_end(1, 0, false);
        selection.move_end(1, 0, true);
        assert_eq!(selection.selected_text().unwrap(), "CD");
    }
}
