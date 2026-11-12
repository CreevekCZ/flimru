//! Frozen console history lets selections cross viewport edges while the PTY continues.
use ratatui::layout::{Position, Rect};
use std::time::{Duration, Instant};

const MAX_CELLS: usize = 65_536;
pub const MAX_COPY_BYTES: usize = 128 * 1024;

pub struct Selection {
    pub area: Rect,
    pub viewport: Rect,
    pub dragging: bool,
    pub keyboard: bool,
    anchor: (usize, u16),
    end: (usize, u16),
    moved: bool,
    rows: Vec<Vec<vt100::Cell>>,
    wrapped: Vec<bool>,
    history: Option<History>,
    top: usize,
    pointer: Position,
    last_scroll: Instant,
}

struct History {
    screen: vt100::Screen,
    rows: usize,
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
            .map(|row| Self::row_wraps(screen, row, area.width))
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
            history: None,
            top: 0,
            pointer: point,
            last_scroll: Instant::now(),
        };
        selection.anchor = selection.point(point);
        selection.end = selection.anchor;
        Some(selection)
    }

    /// Own the existing bounded PTY history, independent of future output/eviction.
    pub fn capture_history(
        screen: &vt100::Screen,
        start_row: u16,
        area: Rect,
        point: Position,
    ) -> Option<Self> {
        let mut selection = Self::capture(screen, start_row, area, point)?;
        let mut frozen = screen.clone();
        frozen.set_scrollback(usize::MAX);
        let rows = frozen.scrollback();
        selection.top = rows.saturating_sub(screen.scrollback()) + usize::from(start_row);
        selection.anchor.0 += selection.top;
        selection.end = selection.anchor;
        selection.history = Some(History {
            screen: frozen,
            rows,
        });
        Some(selection)
    }

    fn point(&self, point: Position) -> (usize, u16) {
        let row = point
            .y
            .saturating_sub(self.area.y)
            .min(self.area.height.saturating_sub(1));
        let mut col = point
            .x
            .saturating_sub(self.area.x)
            .min(self.area.width.saturating_sub(1));
        // Retained rows keep their original width after a terminal resize.
        // Missing historical tail cells are blank, not indexable cells.
        if self.rows[usize::from(row)]
            .get(usize::from(col))
            .is_some_and(vt100::Cell::is_wide_continuation)
        {
            col = col.saturating_sub(1);
        }
        (self.top + usize::from(row), col)
    }

    pub fn update(&mut self, point: Position) {
        self.pointer = point;
        self.end = self.point(point);
        self.moved |= self.end != self.anchor;
    }

    /// A stationary pointer at either edge keeps scrolling on the normal UI tick.
    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.dragging
            || !self.moved
            || now.duration_since(self.last_scroll) < Duration::from_millis(75)
        {
            return false;
        }
        let amount = if self.pointer.y <= self.area.y {
            -1
        } else if self.pointer.y >= self.area.bottom().saturating_sub(1) {
            1
        } else {
            return false;
        };
        self.last_scroll = now;
        self.scroll(amount)
    }

    /// Positive amounts reveal newer rows; the anchor stays in frozen history.
    pub fn scroll(&mut self, amount: isize) -> bool {
        let Some(history) = &mut self.history else {
            return false;
        };
        let total = history.rows + usize::from(history.screen.size().0);
        let top = self
            .top
            .saturating_add_signed(amount)
            .min(total.saturating_sub(usize::from(self.area.height)));
        if top == self.top {
            return false;
        }
        self.top = top;
        history
            .screen
            .set_scrollback(history.rows.saturating_sub(top));
        let start = top.saturating_sub(history.rows) as u16;
        self.rows = (start..start + self.area.height)
            .map(|row| {
                (0..self.area.width)
                    .filter_map(|col| history.screen.cell(row, col).cloned())
                    .collect()
            })
            .collect();
        self.wrapped = (start..start + self.area.height)
            .map(|row| Self::row_wraps(&history.screen, row, self.area.width))
            .collect();
        self.update(self.pointer);
        true
    }

    pub fn scroll_position(&self) -> Option<(usize, usize)> {
        self.history.as_ref().map(|history| {
            let max = (history.rows + usize::from(history.screen.size().0))
                .saturating_sub(usize::from(self.area.height));
            (self.top, max)
        })
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
            .saturating_sub(self.top)
            .saturating_add_signed(isize::from(dy))
            .min(usize::from(self.area.height - 1));
        let col = self
            .end
            .1
            .saturating_add_signed(dx)
            .min(self.area.width - 1);
        let col = if dx > 0
            && self.rows[row]
                .get(usize::from(col))
                .is_some_and(vt100::Cell::is_wide_continuation)
        {
            col.saturating_add(1).min(self.area.width - 1)
        } else {
            col
        };
        self.end = self.point((self.area.x + col, self.area.y + row as u16).into());
        if !extend {
            self.anchor = self.end;
        }
        self.moved = true;
    }

    pub fn selected_text(&mut self) -> Option<String> {
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
        let point = (self.top + usize::from(row), col);
        point >= start && point <= end
    }

    fn row_wraps(screen: &vt100::Screen, row: u16, width: u16) -> bool {
        // A historic wrap joins only if the captured width includes the whole
        // original row. Shrinking a wider retained row must not join its cut tail.
        screen.row_wrapped(row) && screen.cell(row, width).is_none()
    }

    fn text(&mut self) -> String {
        let (start, end) = (self.anchor.min(self.end), self.anchor.max(self.end));
        let mut text = String::new();
        // Only the snapshot cursor moves; rendering uses cached viewport cells.
        let history_rows = self.history.as_ref().map_or(0, |history| history.rows);
        let mut history = self.history.as_mut().map(|history| &mut history.screen);
        let mut previous_wrapped = false;
        for row in start.0..=end.0 {
            let (cells, wrapped) = if let Some(screen) = &mut history {
                screen.set_scrollback(history_rows.saturating_sub(row));
                let local = row.saturating_sub(history_rows) as u16;
                (
                    (0..self.area.width)
                        .filter_map(|col| screen.cell(local, col))
                        .collect::<Vec<_>>(),
                    Self::row_wraps(screen, local, self.area.width),
                )
            } else {
                (self.rows[row].iter().collect(), self.wrapped[row])
            };
            if row > start.0 && !previous_wrapped {
                text.push('\n');
            }
            let first = if row == start.0 { start.1 } else { 0 };
            let last = if row == end.0 {
                end.1
            } else {
                self.area.width - 1
            };
            let mut line = String::new();
            // Do not materialize virtual padding after a short historic row:
            // it would insert spaces in old soft-wrapped lines after growth.
            for cell in cells
                .iter()
                .skip(usize::from(first))
                .take(usize::from(last - first) + 1)
            {
                if cell.is_wide_continuation() {
                    continue;
                }
                line.push_str(if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                });
            }
            if row < end.0 && wrapped {
                text.push_str(&line);
            } else {
                text.push_str(line.trim_end_matches(' '));
            }
            // Clipboard rejects oversized ranges; avoid building the rest of a huge copy.
            if text.len() > MAX_COPY_BYTES {
                break;
            }
            previous_wrapped = wrapped;
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

    fn history_fixture() -> vt100::Parser {
        let mut parser = vt100::Parser::new(4, 12, 100);
        for row in 0..20 {
            parser.process(format!("row:{row:02}\r\n").as_bytes());
        }
        parser
    }

    fn resized_history(old_width: u16, new_width: u16) -> vt100::Parser {
        let mut parser = vt100::Parser::new(4, old_width, 100);
        for row in 0..20 {
            parser.process(format!("row:{row:02}\r\n").as_bytes());
        }
        parser.screen_mut().set_size(4, new_width);
        parser.screen_mut().set_scrollback(usize::MAX);
        parser
    }

    #[test]
    fn grown_history_tail_supports_mouse_keyboard_scroll_and_copy_without_padding() {
        // Exact review reproduction: retained rows have 40 cells at an 80-column size.
        let parser = resized_history(40, 80);
        assert!(parser.screen().cell(0, 60).is_none());
        let area = Rect::new(0, 0, 80, 4);
        let mut selection =
            Selection::capture_history(parser.screen(), 0, area, (60, 0).into()).unwrap();
        assert_eq!(
            selection.finish((79, 1).into()).as_deref(),
            Some("\nrow:01")
        );
        assert!(selection.contains(0, 60) && selection.contains(0, 79));
        assert!(selection.scroll(10));
        let copied = selection.selected_text().unwrap();
        assert!(copied.starts_with("\nrow:01\n"), "{copied:?}");
        assert!(copied.ends_with("row:11"), "{copied:?}");
        assert!(!copied.contains("  "));
        selection.enable_keyboard();
        // The endpoint is absolute history row 11, but keyboard movement is
        // relative to the current viewport (history rows 10..13).
        selection.move_end(-19, 0, false);
        assert_eq!(selection.end, (11, 60));
        assert!(selection.selected_text().is_none());
        selection.move_end(i16::MIN, 0, true);
        assert_eq!(selection.selected_text().as_deref(), Some("row:11"));
        selection.move_end(i16::MAX, 1, true);
        assert_eq!(selection.selected_text().as_deref(), Some("\nrow:12"));
        assert!(selection.scroll(isize::MIN));
        assert!(selection.scroll(isize::MAX));
        selection.update((79, 3).into());
        let _ = selection.selected_text();
        // Non-history capture has the same short cached rows and safe tail rules.
        let mut plain = Selection::capture(parser.screen(), 0, area, (60, 0).into()).unwrap();
        plain.enable_keyboard();
        plain.move_end(19, 0, true);
        assert!(plain.selected_text().is_none());
        assert_eq!(plain.finish((5, 1).into()).as_deref(), Some("\nrow:01"));
    }

    #[test]
    fn grown_history_preserves_old_soft_wrap_spaces_wide_and_combining_cells() {
        let original = format!("{}界e\u{301}cd  ef", "a".repeat(35));
        let mut parser = vt100::Parser::new(4, 40, 100);
        parser.process(original.as_bytes());
        for _ in 0..8 {
            parser.process(b"\r\nnext");
        }
        parser.screen_mut().set_size(4, 80);
        parser.screen_mut().set_scrollback(usize::MAX);
        assert!(parser.screen().row_wrapped(0));
        assert!(parser.screen().cell(0, 60).is_none());
        let mut selection =
            Selection::capture_history(parser.screen(), 0, Rect::new(0, 0, 80, 4), (0, 0).into())
                .unwrap();
        assert_eq!(
            selection.finish((3, 1).into()).as_deref(),
            Some(original.as_str())
        );
        // Starting on a wide continuation still includes the complete scalar
        // and the following combining mark after the historical resize.
        let mut wide =
            Selection::capture_history(parser.screen(), 0, Rect::new(0, 0, 80, 4), (36, 0).into())
                .unwrap();
        assert_eq!(wide.finish((37, 0).into()).as_deref(), Some("界e\u{301}"));
    }

    #[test]
    fn shrunk_history_clips_rows_without_joining_truncated_soft_wraps() {
        let mut parser = vt100::Parser::new(4, 80, 100);
        parser.process(format!("{}{}", "a".repeat(80), "b".repeat(80)).as_bytes());
        for _ in 0..8 {
            parser.process(b"\r\nnext");
        }
        parser.screen_mut().set_size(4, 40);
        parser.screen_mut().set_scrollback(usize::MAX);
        assert!(parser.screen().cell(0, 60).is_some());
        let mut selection =
            Selection::capture_history(parser.screen(), 0, Rect::new(0, 0, 40, 4), (0, 0).into())
                .unwrap();
        assert_eq!(
            selection.finish((39, 1).into()).unwrap(),
            format!("{}\n{}", "a".repeat(40), "b".repeat(40))
        );
        assert!(selection.scroll(3));
        selection.enable_keyboard();
        selection.move_end(i16::MAX, 0, true);
        selection.move_end(i16::MIN, -1, true);
        let _ = selection.selected_text();
        assert!(selection.rows().all(|row| row.len() <= 40));
    }

    #[test]
    fn selection_crosses_history_in_both_directions_without_losing_its_anchor() {
        let mut parser = history_fixture();
        parser.screen_mut().set_scrollback(usize::MAX);
        let mut selection =
            Selection::capture_history(parser.screen(), 0, Rect::new(2, 4, 12, 4), (2, 4).into())
                .unwrap();
        selection.update((7, 7).into());
        assert!(selection.scroll(10));
        let expected = (0..=13)
            .map(|row| format!("row:{row:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        // Original history may be evicted or overwritten while selection stays frozen.
        parser.process(b"\x1b[Hchanged");
        assert_eq!(selection.selected_text().unwrap(), expected);
        assert!(selection.scroll(isize::MAX));
        assert!(!selection.scroll(1));
        selection.update((7, 4).into());
        assert!(selection.scroll(isize::MIN));
        assert_eq!(selection.selected_text().unwrap(), "row:00");
        assert!(!selection.scroll(-1));

        parser.screen_mut().set_scrollback(0);
        let mut reverse =
            Selection::capture_history(parser.screen(), 0, Rect::new(0, 0, 12, 4), (5, 1).into())
                .unwrap();
        reverse.update((0, 0).into());
        assert!(reverse.scroll(-10));
        let text = reverse.selected_text().unwrap();
        assert!(text.starts_with("row:07\n"), "{text:?}");
        assert!(text.ends_with("row:18"), "{text:?}");
    }

    #[test]
    fn stationary_edge_scroll_is_timed_and_stops_on_release_or_leaving_edge() {
        let mut parser = history_fixture();
        parser.screen_mut().set_scrollback(usize::MAX);
        let mut selection =
            Selection::capture_history(parser.screen(), 0, Rect::new(0, 2, 12, 4), (0, 2).into())
                .unwrap();
        let now = selection.last_scroll;
        assert!(!selection.tick(now + Duration::from_millis(100))); // A click is not a drag.
        selection.update((5, 5).into());
        assert!(!selection.tick(now + Duration::from_millis(20)));
        assert!(selection.tick(now + Duration::from_millis(100)));
        assert!(selection.tick(now + Duration::from_millis(200))); // No new mouse event.
        selection.update((5, 3).into());
        assert!(!selection.tick(now + Duration::from_millis(300)));
        selection.finish((5, 5).into());
        assert!(!selection.tick(now + Duration::from_millis(400)));
    }

    #[test]
    fn saved_console_hidden_rows_and_soft_wraps_remain_selectable() {
        let mut parser = vt100::Parser::new(6, 4, 20);
        parser.process(b"ab  cd efghijklmnop");
        let mut selection =
            Selection::capture_history(parser.screen(), 2, Rect::new(0, 0, 4, 2), (0, 0).into())
                .unwrap();
        selection.update((0, 0).into());
        selection.scroll(-2);
        assert_eq!(selection.selected_text().unwrap(), "ab  cd ef");
        selection.update((3, 1).into());
        selection.scroll(3);
        assert_eq!(selection.selected_text().unwrap(), "fghijklmnop");
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
