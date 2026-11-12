//! Modal presentation and bounded path editing only. Capture and filesystem work
//! belong to the caller. Every event is consumed while this modal is open.
use crate::console_snapshot::ConsoleSnapshot;
use crate::log_export::{ExportMetadata, MAX_DESTINATION_BYTES};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const ACCENT: Color = Color::Rgb(96, 210, 225);
const MUTED: Color = Color::Rgb(151, 163, 180);
const PANEL: Color = Color::Rgb(22, 28, 39);
const TEXT: Color = Color::Rgb(224, 232, 240);
const INPUT: Color = Color::Rgb(14, 19, 27);

#[derive(Debug, Eq, PartialEq)]
pub enum DialogAction {
    Submit(PathBuf),
    Cancel,
    Consumed,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct DialogRegions {
    pub destination: Rect,
    pub export: Rect,
    pub cancel: Rect,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Focus {
    Destination,
    Export,
    Cancel,
}

enum Status {
    Preparing,
    Ready,
    Saving,
    Error(String),
}

/// The source label, metadata and snapshot remain tied to the captured console,
/// regardless of which live tab the caller subsequently displays.
pub struct ExportDialog {
    source: String,
    metadata: ExportMetadata,
    snapshot: Option<Arc<ConsoleSnapshot>>,
    destination: String,
    input_error: Option<&'static str>,
    cursor: usize,
    focus: Focus,
    status: Status,
    regions: DialogRegions,
    covered_area: Option<Rect>,
    usable_layout: bool,
}

impl ExportDialog {
    pub fn new(
        source: &str,
        metadata: ExportMetadata,
        destination: &Path,
        snapshot: Option<Arc<ConsoleSnapshot>>,
    ) -> Self {
        // Paths are data, unlike display labels: never strip, replace or truncate
        // bytes into a different filename. Unsupported initial paths require an
        // explicit replacement before submission.
        let (destination, input_error) = match destination.to_str() {
            None => (
                String::new(),
                Some("Initial path is not UTF-8. Enter a new path."),
            ),
            Some(path) if path.len() > MAX_DESTINATION_BYTES => (
                String::new(),
                Some("Initial path exceeds 4096 B. Enter a new path."),
            ),
            Some(path) if path.chars().any(char::is_control) => (
                String::new(),
                Some("Initial path has controls. Enter a new path."),
            ),
            Some(path) => (path.to_owned(), None),
        };
        let cursor = destination.len();
        let status = if snapshot.is_some() {
            Status::Ready
        } else {
            Status::Preparing
        };
        Self {
            source: bounded_text(source, 512),
            metadata,
            snapshot,
            destination,
            input_error,
            cursor,
            focus: Focus::Destination,
            status,
            regions: DialogRegions::default(),
            covered_area: None,
            usable_layout: true,
        }
    }

    pub fn metadata(&self) -> &ExportMetadata {
        &self.metadata
    }
    pub fn snapshot(&self) -> Option<&Arc<ConsoleSnapshot>> {
        self.snapshot.as_ref()
    }
    pub fn destination(&self) -> &str {
        &self.destination
    }
    pub fn regions(&self) -> DialogRegions {
        self.regions
    }

    /// Use the last rendered coverage when routing global pointer actions.
    /// Until the first draw, treat the background as fully covered.
    pub fn covers(&self, position: Position) -> bool {
        self.covered_area.is_none_or(|area| area.contains(position))
    }

    /// Supply the capture for this dialog's original source only. The caller
    /// must discard stale capture results after cancelling/replacing the dialog.
    pub fn set_snapshot(&mut self, snapshot: Arc<ConsoleSnapshot>) {
        if self.snapshot.is_none() && !matches!(self.status, Status::Saving) {
            self.snapshot = Some(snapshot);
            self.status = Status::Ready;
        }
    }

    /// The caller supplies a sanitized, useful worker/validation error.
    /// Errors retain the destination and captured source for editing/retry.
    pub fn set_error(&mut self, message: &str) {
        self.status = Status::Error(bounded_text(message, 512));
        self.focus = Focus::Destination;
    }

    fn destination_error(&self) -> Option<&'static str> {
        self.input_error.or_else(|| {
            self.destination
                .is_empty()
                .then_some("Enter a destination.")
        })
    }

    fn ready(&self) -> bool {
        self.snapshot.is_some()
            && self.destination_error().is_none()
            && !matches!(self.status, Status::Preparing | Status::Saving)
    }

    fn submit(&mut self) -> DialogAction {
        if !self.ready() || !self.usable_layout {
            return DialogAction::Consumed;
        }
        self.status = Status::Saving;
        DialogAction::Submit(PathBuf::from(&self.destination))
    }

    fn edit(&mut self, text: &str) {
        if matches!(self.status, Status::Saving) {
            return;
        }
        if self.destination.len().saturating_add(text.len()) > MAX_DESTINATION_BYTES {
            self.reject_input("Path exceeds 4096 bytes. Edit before export.");
            return;
        }
        if text.chars().any(char::is_control) {
            self.reject_input("Path has controls. Edit before export.");
            return;
        }
        if text.is_empty() {
            return;
        }
        self.destination.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.input_error = None;
    }

    fn reject_input(&mut self, message: &'static str) {
        self.input_error = Some(message);
        self.focus = Focus::Destination;
    }

    fn enter_hint(&self) -> &'static str {
        if self.focus == Focus::Cancel {
            "Enter Cancel; Esc Cancel"
        } else if self.ready() {
            "Enter Export; Esc Cancel"
        } else if matches!(self.status, Status::Saving) {
            "Saving · Esc Cancel"
        } else if matches!(self.status, Status::Preparing) {
            "Preparing · Esc Cancel"
        } else {
            "Edit path · Esc Cancel"
        }
    }

    /// Caller handles its global quit/menu policy first. Nothing from this
    /// handler is forwarded to Flutter, including background clicks or paste.
    pub fn handle_event(&mut self, event: &Event) -> DialogAction {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc {
                    return DialogAction::Cancel;
                }
                if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
                    self.focus = match (self.focus, key.code == KeyCode::BackTab) {
                        (Focus::Destination, false) | (Focus::Cancel, true) => Focus::Export,
                        (Focus::Export, false) | (Focus::Destination, true) => Focus::Cancel,
                        _ => Focus::Destination,
                    };
                    return DialogAction::Consumed;
                }
                if key.code == KeyCode::Enter {
                    return if self.focus == Focus::Cancel {
                        DialogAction::Cancel
                    } else {
                        self.submit()
                    };
                }
                if self.focus != Focus::Destination || matches!(self.status, Status::Saving) {
                    return DialogAction::Consumed;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
                    self.destination.clear();
                    self.cursor = 0;
                    self.input_error = None;
                    return DialogAction::Consumed;
                }
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                {
                    return DialogAction::Consumed;
                }
                match key.code {
                    KeyCode::Home => self.cursor = 0,
                    KeyCode::End => self.cursor = self.destination.len(),
                    KeyCode::Left => {
                        self.cursor = self.destination[..self.cursor]
                            .char_indices()
                            .next_back()
                            .map_or(0, |(i, _)| i)
                    }
                    KeyCode::Right => {
                        self.cursor += self.destination[self.cursor..]
                            .chars()
                            .next()
                            .map_or(0, char::len_utf8)
                    }
                    KeyCode::Backspace => {
                        if let Some((previous, _)) =
                            self.destination[..self.cursor].char_indices().next_back()
                        {
                            self.destination.drain(previous..self.cursor);
                            self.cursor = previous;
                            self.input_error = None;
                        }
                    }
                    KeyCode::Delete => {
                        if let Some(ch) = self.destination[self.cursor..].chars().next() {
                            self.destination
                                .drain(self.cursor..self.cursor + ch.len_utf8());
                            self.input_error = None;
                        }
                    }
                    KeyCode::Char(ch) => self.edit(&ch.to_string()),
                    _ => {}
                }
            }
            Event::Paste(text) if self.focus == Focus::Destination => self.edit(text),
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = (mouse.column, mouse.row).into();
                if self.regions.cancel.contains(position) {
                    return DialogAction::Cancel;
                }
                if self.regions.export.contains(position) {
                    return self.submit();
                }
                if self.regions.destination.contains(position) {
                    self.focus = Focus::Destination;
                    self.cursor = self.destination.len();
                }
            }
            _ => {}
        }
        DialogAction::Consumed
    }

    pub fn render(&mut self, frame: &mut Frame) {
        self.render_with_focus(frame, true);
    }

    /// Covered dialogs keep their state but must not claim the native cursor
    /// while an application-wide quit overlay owns keyboard input.
    pub fn render_with_focus(&mut self, frame: &mut Frame, focused: bool) {
        self.regions = DialogRegions::default();
        let screen = frame.area();
        self.covered_area = Some(screen);
        self.usable_layout = screen.width >= 30 && screen.height >= 12;
        if !self.usable_layout {
            frame.render_widget(Clear, screen);
            frame.render_widget(
                Paragraph::new("Export console log\nEnlarge terminal to edit.\nEsc cancels.")
                    .style(Style::default().fg(TEXT).bg(PANEL))
                    .wrap(Wrap { trim: false }),
                screen,
            );
            return;
        }
        let width = screen.width.saturating_sub(2).min(80);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - 12) / 2,
            width,
            12,
        );
        self.covered_area = Some(area);
        let block = Block::default()
            .title(" Export retained console log ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL).fg(TEXT));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let row = |offset, height| {
            Rect::new(
                inner.x + 1,
                inner.y + offset,
                inner.width.saturating_sub(2),
                height,
            )
        };
        let label_style = Style::default().fg(MUTED);
        frame.render_widget(
            Paragraph::new(clipped_text(
                &format!("Source: {}", self.source),
                usize::from(row(0, 1).width),
            )),
            row(0, 1),
        );
        frame.render_widget(
            Paragraph::new("Full snapshot, including\nquery/filter-hidden lines.")
                .style(label_style),
            row(1, 2),
        );
        frame.render_widget(
            Paragraph::new("Destination (no overwrite)").style(label_style),
            row(3, 1),
        );
        self.regions.destination = row(4, 1);
        let (visible, cursor_column) = input_window(
            &self.destination,
            self.cursor,
            usize::from(self.regions.destination.width),
        );
        frame.render_widget(
            Paragraph::new(visible).style(Style::default().bg(INPUT).fg(
                if self.focus == Focus::Destination {
                    ACCENT
                } else {
                    TEXT
                },
            )),
            self.regions.destination,
        );
        let destination_error = self.destination_error();
        let status = match (&self.status, destination_error) {
            (Status::Saving, _) => "Saving frozen snapshot…\nEsc cancels pending work.".to_owned(),
            (_, Some(message)) if self.input_error.is_some() => format!("Error: {message}"),
            (Status::Preparing, Some(message)) => format!("Preparing snapshot…\n{message}"),
            (Status::Preparing, None) => "Preparing snapshot…\nWaiting for capture.".to_owned(),
            (_, Some(message)) => message.to_owned(),
            (Status::Error(message), None) => format!("Error: {message}"),
            (Status::Ready, None) => format!(
                "Ready · {} B retained text\nMay contain secrets. UTF-8.",
                self.snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.plain_text().len())
            ),
        };
        let status_color = if destination_error.is_some() || matches!(self.status, Status::Error(_))
        {
            Color::Yellow
        } else {
            MUTED
        };
        frame.render_widget(
            Paragraph::new(status)
                .style(Style::default().fg(status_color))
                .wrap(Wrap { trim: false }),
            row(5, 2),
        );
        let buttons = row(7, 1);
        self.regions.export = Rect::new(buttons.x, buttons.y, 10, 1);
        self.regions.cancel = Rect::new(buttons.x + 12, buttons.y, 10, 1);
        for (text, region, focus, enabled) in [
            (
                "[ Export ]",
                self.regions.export,
                Focus::Export,
                self.ready(),
            ),
            ("[ Cancel ]", self.regions.cancel, Focus::Cancel, true),
        ] {
            let mut style = Style::default().fg(if enabled { ACCENT } else { MUTED });
            if self.focus == focus {
                style = style
                    .bg(Color::Rgb(34, 64, 78))
                    .add_modifier(Modifier::BOLD);
            }
            frame.render_widget(Paragraph::new(text).style(style), region);
        }
        frame.render_widget(
            Paragraph::new(self.enter_hint()).style(label_style),
            row(8, 1),
        );
        frame.render_widget(
            Paragraph::new(if matches!(self.status, Status::Saving) {
                "Tab focus"
            } else {
                "←→ Home End edit · Tab focus"
            })
            .style(label_style),
            row(9, 1),
        );
        if focused && self.focus == Focus::Destination && !matches!(self.status, Status::Saving) {
            frame.set_cursor_position((
                self.regions.destination.x + cursor_column as u16,
                self.regions.destination.y,
            ));
        }
    }
}

fn bounded_text(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_bytes) {
        if ch.is_control() {
            continue;
        }
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out
}

fn input_window(text: &str, cursor: usize, width: usize) -> (String, usize) {
    if width < 2 {
        return (String::new(), 0);
    }
    let span = Span::raw(text);
    let mut end = 0;
    let mut column = 0;
    // A scalar-edit cursor can sit inside a joined/combining grapheme. Display
    // it at that grapheme's beginning, using the same boundaries and widths as
    // Ratatui's renderer for both measurement and scrolling.
    for grapheme in span.styled_graphemes(Style::default()) {
        end += grapheme.symbol.len();
        if end > cursor {
            break;
        }
        column += Span::raw(grapheme.symbol).width();
    }
    let mut start = 0;
    if column >= width {
        for grapheme in span.styled_graphemes(Style::default()) {
            if column < width - 1 {
                break;
            }
            column = column.saturating_sub(Span::raw(grapheme.symbol).width());
            start += grapheme.symbol.len();
        }
    }
    if start > 0 {
        (
            format!("‹{}", &text[start..]),
            (column + 1).min(width.saturating_sub(1)),
        )
    } else {
        (text.to_owned(), column.min(width.saturating_sub(1)))
    }
}

fn clipped_text(text: &str, width: usize) -> String {
    if Line::from(text).width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut columns = 0;
    for ch in text.chars() {
        let cell_width = Span::raw(ch.to_string()).width();
        if columns + cell_width >= width {
            break;
        }
        out.push(ch);
        columns += cell_width;
    }
    if width > 0 {
        out.push('…');
    }
    out
}
