//! Frozen console review UI. Route application-wide actions/modal overlays first;
//! while active, every remaining event belongs here and must never reach the PTY.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use flimru::console_review::{
    ReviewData, ReviewError, ReviewTicket, ReviewWorker, Severity, SeverityFilter,
};
use flimru::console_snapshot::{ConsoleSnapshot, MAX_QUERY_CHARS};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::borrow::Cow;
use std::io;
use std::ops::Range;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const ACCENT: Color = Color::Rgb(96, 210, 225);
const MUTED: Color = Color::Rgb(151, 163, 180);
const PANEL: Color = Color::Rgb(22, 28, 39);
const TEXT: Color = Color::Rgb(224, 232, 240);
const MATCH: Color = Color::Rgb(34, 64, 78);
const ERROR: Color = Color::Rgb(255, 128, 142);
// Accommodate a full 4096-byte export destination plus its outcome/byte count.
pub const MAX_OPERATION_CHARS: usize = 8192;

enum Details {
    Source,
    Operation(String),
}

/// Capture-time metadata, independent of later tab selection or session lifecycle.
/// Callers supply display labels only, never launch arguments or environment.
#[derive(Clone, Debug)]
pub struct SearchSource {
    pub id: u64,
    pub label: String,
    pub device: Option<String>,
    pub configuration: Option<String>,
    pub outcome: Option<String>,
    pub captured_at: SystemTime,
}
impl SearchSource {
    pub fn new(id: u64, label: String, captured_at: SystemTime) -> Self {
        Self {
            id,
            label,
            device: None,
            configuration: None,
            outcome: None,
            captured_at,
        }
    }
    fn normalized(mut self) -> (Self, bool) {
        let mut shortened = false;
        let mut normalize = |value: &str, limit| {
            let (value, clipped) = bounded_label(value, limit);
            shortened |= clipped;
            value
        };
        self.label = normalize(&self.label, 96);
        self.device = self.device.map(|value| normalize(&value, 128));
        self.configuration = self.configuration.map(|value| normalize(&value, 128));
        self.outcome = self.outcome.map(|value| normalize(&value, 128));
        (self, shortened)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchEffect {
    Inactive,
    Consumed,
    ResumeLive,
    RefreshSnapshot,
}

pub struct SearchView {
    worker: ReviewWorker,
    source: Option<SearchSource>,
    source_shortened: bool,
    details: Option<Details>,
    source_scroll: usize,
    source_viewport: Rect,
    expected: Option<ReviewTicket>,
    data: Option<ReviewData>,
    query: String,
    filter: SeverityFilter,
    editing: bool,
    pending: bool,
    capturing: bool,
    error: Option<String>,
    notice: Option<&'static str>,
    current: Option<usize>,
    top: usize,
    left: u16,
    viewport: Rect,
    reveal: bool,
}
impl SearchView {
    pub fn new(classify: fn(&str) -> Option<Severity>) -> io::Result<Self> {
        Ok(Self {
            worker: ReviewWorker::new(classify)?,
            source: None,
            source_shortened: false,
            details: None,
            source_scroll: 0,
            source_viewport: Rect::default(),
            expected: None,
            data: None,
            query: String::new(),
            filter: SeverityFilter::All,
            editing: false,
            pending: false,
            capturing: false,
            error: None,
            notice: None,
            current: None,
            top: 0,
            left: 0,
            viewport: Rect::default(),
            reveal: false,
        })
    }
    pub fn is_active(&self) -> bool {
        self.source.is_some()
    }
    pub fn is_pending(&self) -> bool {
        self.pending
    }
    pub fn editing(&self) -> bool {
        self.editing
    }
    pub fn source(&self) -> Option<&SearchSource> {
        self.source.as_ref()
    }
    pub fn showing_source_details(&self) -> bool {
        matches!(self.details, Some(Details::Source))
    }
    pub fn showing_operation_details(&self) -> bool {
        matches!(self.details, Some(Details::Operation(_)))
    }
    /// An operation outcome is live App information, independent of a snapshot.
    /// It can be inspected after resuming without making review active again.
    pub fn show_operation_details(&mut self, message: &str) {
        self.details = Some(Details::Operation(display_label(
            message,
            MAX_OPERATION_CHARS,
        )));
        self.source_scroll = 0;
    }
    /// Deliberate page/tab actions leave the outcome without discarding a
    /// same-source frozen review, query, filter or reading position.
    pub fn dismiss_operation_details(&mut self) {
        if self.showing_operation_details() {
            self.details = None;
            self.source_scroll = 0;
        }
    }
    /// Explicit Actions entry points work in either editing or reading mode.
    pub fn edit_query(&mut self) {
        if self.is_active() {
            self.details = None;
            self.editing = true;
        }
    }
    pub fn show_source_details(&mut self) {
        if self.is_active() {
            self.details = Some(Details::Source);
            self.source_scroll = 0;
        }
    }
    pub fn data(&self) -> Option<&ReviewData> {
        self.data.as_ref()
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn filter(&self) -> SeverityFilter {
        self.filter
    }
    pub fn current_match(&self) -> Option<usize> {
        self.current
    }
    pub fn offsets(&self) -> (usize, u16) {
        (self.top, self.left)
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Exact full snapshot, including lines hidden by query/severity filtering.
    pub fn snapshot(&self) -> Option<Arc<ConsoleSnapshot>> {
        self.data.as_ref().map(|data| Arc::clone(&data.snapshot))
    }

    /// Caller must run the historical-width-aware preflight before making this
    /// screen clone. New sources start an empty query; refreshing preserves it.
    pub fn capture(
        &mut self,
        source: SearchSource,
        screen: vt100::Screen,
    ) -> Result<(), ReviewError> {
        let source_id = source.id;
        let same_source = self.source.as_ref().is_some_and(|old| old.id == source.id);
        if !same_source {
            self.query.clear();
            self.filter = SeverityFilter::All;
            self.editing = true;
        }
        // Release UI-owned output before handing another capture to the worker.
        self.data = None;
        let (source, shortened) = source.normalized();
        self.source = Some(source);
        self.source_shortened = shortened;
        self.details = None;
        self.source_scroll = 0;
        self.current = None;
        self.top = 0;
        self.left = 0;
        self.notice = None;
        self.error = None;
        self.pending = false;
        self.expected = None;
        match self
            .worker
            .capture(source_id, screen, self.query.clone(), self.filter)
        {
            Ok(ticket) => {
                self.expected = Some(ticket);
                self.pending = true;
                self.capturing = true;
                self.reveal = true;
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Supply refreshed capture-time outcome/time alongside the same stable ID.
    pub fn refresh(
        &mut self,
        source: SearchSource,
        screen: vt100::Screen,
    ) -> Result<(), ReviewError> {
        if self.source.as_ref().is_none_or(|old| old.id != source.id) {
            return Err(ReviewError::NoSnapshot);
        }
        self.capture(source, screen)
    }
    /// Keep a failed preflight visible and input-isolated without cloning a screen.
    pub fn show_error(&mut self, source: SearchSource, message: String) {
        self.close();
        let (source, shortened) = source.normalized();
        self.source = Some(source);
        self.source_shortened = shortened;
        self.error = Some(display_label(&message, 512));
    }
    pub fn close(&mut self) {
        self.data = None;
        self.source = None;
        self.source_shortened = false;
        self.details = None;
        self.source_scroll = 0;
        self.source_viewport = Rect::default();
        self.expected = None;
        self.pending = false;
        self.capturing = false;
        self.error = None;
        self.notice = None;
        self.current = None;
        self.query.clear();
        self.filter = SeverityFilter::All;
        self.editing = false;
        self.top = 0;
        self.left = 0;
        self.worker.clear();
    }
    /// Poll on the ordinary UI tick; a stale source/capture/query cannot replace
    /// the current view, even if the caller closed and reopened the same source.
    pub fn poll(&mut self) -> bool {
        let Some(update) = self.worker.take_update() else {
            return false;
        };
        if self.expected != Some(update.ticket)
            || self
                .source
                .as_ref()
                .is_none_or(|source| source.id != update.ticket.source)
        {
            return false;
        }
        self.pending = false;
        self.capturing = false;
        match update.result {
            Ok(data) => {
                self.current = (!data.matches.matches.is_empty()).then_some(0);
                self.data = Some(data);
                self.error = None;
                self.top = 0;
                self.left = 0;
                self.reveal = true;
                self.reveal_current();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        true
    }
    pub fn set_query(&mut self, query: &str) {
        if !self.is_active() {
            return;
        }
        let mut chars = query.chars().take(MAX_QUERY_CHARS + 1);
        self.query = chars
            .by_ref()
            .take(MAX_QUERY_CHARS)
            .filter(|ch| !ch.is_control())
            .collect();
        self.notice = chars
            .next()
            .is_some()
            .then_some("Query limited to 512 characters");
        self.submit();
    }
    pub fn set_filter(&mut self, filter: SeverityFilter) {
        if self.is_active() && self.filter != filter {
            self.filter = filter;
            self.submit();
        }
    }
    pub fn next_match(&mut self, backwards: bool) {
        if self.pending || self.error.is_some() {
            return;
        }
        let count = self
            .data
            .as_ref()
            .map_or(0, |data| data.matches.matches.len());
        if count == 0 {
            return;
        }
        self.current = Some(match (self.current, backwards) {
            (None, false) => 0,
            (None, true) => count - 1,
            (Some(index), false) => (index + 1) % count,
            (Some(index), true) => (index + count - 1) % count,
        });
        self.reveal = true;
        self.reveal_current();
    }
    pub fn previous_match(&mut self) {
        self.next_match(true);
    }
    fn submit(&mut self) {
        self.error = None;
        self.current = None;
        match self.worker.search(self.query.clone(), self.filter) {
            Ok(ticket) => {
                self.expected = Some(ticket);
                self.pending = true;
            }
            Err(error) => {
                self.pending = false;
                self.error = Some(error.to_string());
            }
        }
    }
    pub fn cycle_filter(&mut self) {
        self.set_filter(match self.filter {
            SeverityFilter::All => SeverityFilter::Errors,
            SeverityFilter::Errors => SeverityFilter::Warnings,
            SeverityFilter::Warnings => SeverityFilter::ErrorsAndWarnings,
            SeverityFilter::ErrorsAndWarnings => SeverityFilter::All,
        });
    }
    pub fn handle_event(&mut self, event: &Event) -> SearchEffect {
        if self.details.is_some() {
            self.handle_source_event(event);
            return SearchEffect::Consumed;
        }
        if !self.is_active() {
            return SearchEffect::Inactive;
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.key(*key),
            Event::Paste(text) if self.editing => {
                let available = MAX_QUERY_CHARS.saturating_sub(self.query.chars().count());
                let mut chars = text.chars().take(MAX_QUERY_CHARS + 1);
                let addition: String = chars
                    .by_ref()
                    .take(available)
                    .filter(|ch| !ch.is_control())
                    .collect();
                let capped = chars.next().is_some();
                if !addition.is_empty() {
                    self.query.push_str(&addition);
                    self.submit();
                }
                self.notice = capped.then_some("Query limited to 512 characters");
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll(-3, 0),
                MouseEventKind::ScrollDown => self.scroll(3, 0),
                MouseEventKind::ScrollLeft => self.scroll(0, -3),
                MouseEventKind::ScrollRight => self.scroll(0, 3),
                _ => {}
            },
            Event::Resize(_, _) => self.reveal = self.current.is_some(),
            _ => {}
        }
        SearchEffect::Consumed
    }
    fn key(&mut self, key: KeyEvent) -> SearchEffect {
        let has_command_modifier = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if key.code == KeyCode::Esc {
            if self.editing {
                self.editing = false;
            } else {
                self.close();
                return SearchEffect::ResumeLive;
            }
            return SearchEffect::Consumed;
        }
        if self.editing {
            match key.code {
                KeyCode::Enter => self.editing = false,
                KeyCode::Backspace if !has_command_modifier => {
                    if self.query.pop().is_some() {
                        self.notice = None;
                        self.submit();
                    }
                }
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                    self.query.clear();
                    self.notice = None;
                    self.submit();
                }
                KeyCode::Char(ch) if !ch.is_control() && !has_command_modifier => {
                    if self.query.chars().count() < MAX_QUERY_CHARS {
                        self.query.push(ch);
                        self.notice = None;
                        self.submit();
                    } else {
                        self.notice = Some("Query limited to 512 characters");
                    }
                }
                _ => {}
            }
        } else if !has_command_modifier {
            match key.code {
                KeyCode::Char('n') => self.next_match(false),
                KeyCode::Char('N') => self.previous_match(),
                KeyCode::Char('/') => self.edit_query(),
                KeyCode::Char('f') => self.cycle_filter(),
                KeyCode::Char('i') => self.show_source_details(),
                KeyCode::Char('r') => return SearchEffect::RefreshSnapshot,
                KeyCode::Up => self.scroll(-1, 0),
                KeyCode::Down => self.scroll(1, 0),
                KeyCode::Left => self.scroll(0, -1),
                KeyCode::Right => self.scroll(0, 1),
                KeyCode::PageUp => self.scroll(-(self.viewport.height.max(1) as isize), 0),
                KeyCode::PageDown => self.scroll(self.viewport.height.max(1) as isize, 0),
                KeyCode::Home => {
                    self.top = 0;
                    self.left = 0;
                    self.reveal = false;
                }
                KeyCode::End => {
                    self.top = usize::MAX;
                    self.reveal = false;
                    self.clamp();
                }
                _ => {}
            }
        }
        SearchEffect::Consumed
    }
    fn handle_source_event(&mut self, event: &Event) {
        let page = usize::from(self.source_viewport.height.max(1));
        let max = self
            .details_lines(self.source_viewport.width)
            .len()
            .saturating_sub(page);
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc {
                    self.details = None;
                } else if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    match key.code {
                        KeyCode::Enter | KeyCode::Char('i') => self.details = None,
                        KeyCode::Up => self.source_scroll = self.source_scroll.saturating_sub(1),
                        KeyCode::Down => self.source_scroll = self.source_scroll.saturating_add(1),
                        KeyCode::PageUp => {
                            self.source_scroll = self.source_scroll.saturating_sub(page)
                        }
                        KeyCode::PageDown => {
                            self.source_scroll = self.source_scroll.saturating_add(page)
                        }
                        KeyCode::Home => self.source_scroll = 0,
                        KeyCode::End => self.source_scroll = max,
                        _ => {}
                    }
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.source_scroll = self.source_scroll.saturating_sub(3)
                }
                MouseEventKind::ScrollDown => {
                    self.source_scroll = self.source_scroll.saturating_add(3)
                }
                _ => {}
            },
            _ => {}
        }
        self.source_scroll = self.source_scroll.min(max);
    }
    fn scroll(&mut self, vertical: isize, horizontal: i16) {
        self.top = self.top.saturating_add_signed(vertical);
        self.left = self.left.saturating_add_signed(horizontal);
        self.reveal = false;
        self.clamp();
    }
    fn clamp(&mut self) {
        let rows = self.data.as_ref().map_or(0, |data| data.rows.len());
        let columns = self.data.as_ref().map_or(0, |data| data.snapshot.columns());
        self.top = self
            .top
            .min(rows.saturating_sub(usize::from(self.viewport.height.max(1))));
        self.left = self
            .left
            .min(columns.saturating_sub(self.viewport.width.max(1)));
    }
    fn reveal_current(&mut self) {
        if !self.reveal || self.viewport.is_empty() {
            return;
        }
        let Some(data) = &self.data else { return };
        let Some(found) = self
            .current
            .and_then(|index| data.matches.matches.get(index))
        else {
            self.reveal = false;
            self.clamp();
            return;
        };
        if let Some(cells) = data.snapshot.cell_ranges(found.bytes.clone()).next()
            && let Some(row) = data.rows.displayed_row(cells.row)
        {
            let height = usize::from(self.viewport.height);
            if row < self.top {
                self.top = row;
            } else if row >= self.top + height {
                self.top = row + 1 - height;
            }
            let width = self.viewport.width;
            if cells.columns.start < self.left {
                self.left = cells.columns.start;
            } else if cells.columns.end > self.left.saturating_add(width) {
                self.left = if cells.columns.end - cells.columns.start > width {
                    cells.columns.start
                } else {
                    cells.columns.end.saturating_sub(width)
                };
            }
        }
        self.reveal = false;
        self.clamp();
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        self.render_with_focus(frame, area, true);
    }

    /// Parent overlays/top focus can suppress the native query cursor without
    /// changing the user's editing state or frozen review data.
    pub fn render_with_focus(&mut self, frame: &mut Frame<'_>, area: Rect, focused: bool) {
        if area.is_empty() {
            return;
        }
        if self.details.is_some() {
            self.render_details(frame, area);
            return;
        }
        if !self.is_active() {
            return;
        }
        frame.render_widget(Clear, area);
        let label = self
            .source
            .as_ref()
            .map_or("", |source| source.label.as_str());
        let block = Block::default()
            .borders(Borders::ALL)
            .title(clipped_title(
                &format!(" Snapshot · {label} "),
                area.width.saturating_sub(2),
            ))
            .title_bottom(if self.editing { "" } else { " i source " })
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL).fg(TEXT));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        let header_height = inner.height.min(2);
        let footer_height = inner.height.saturating_sub(header_height).min(2);
        let body = Rect::new(
            inner.x,
            inner.y + header_height,
            inner.width,
            inner.height - header_height - footer_height,
        );
        // A notice can change only the height. Preserve manual vertical scroll
        // rather than revealing an old match merely because a notice appeared.
        // Width changes still reveal matches whose cell span may be off-screen.
        if body.width != self.viewport.width {
            self.reveal = self.current.is_some();
        }
        self.viewport = body;
        self.clamp();
        self.reveal_current();
        let query_text = if self.query.is_empty() {
            if self.editing {
                "type to search"
            } else {
                "(empty)"
            }
        } else {
            &self.query
        };
        let query_display = if query_text
            .chars()
            .next()
            .is_some_and(|ch| Span::raw(ch.to_string()).width() == 0)
        {
            Cow::Owned(format!("◌{query_text}"))
        } else {
            Cow::Borrowed(query_text)
        };
        let query_text = query_display.as_ref();
        let prefix = if self.editing {
            "Search ▸ "
        } else {
            "Search: "
        };
        let prefix_width = (prefix.chars().count() as u16).min(inner.width);
        frame.render_widget(
            Paragraph::new(prefix).style(Style::default().fg(if self.editing {
                ACCENT
            } else {
                MUTED
            })),
            Rect::new(inner.x, inner.y, prefix_width, 1),
        );
        let query_area = Rect::new(
            inner.x + prefix_width,
            inner.y,
            inner.width - prefix_width,
            1,
        );
        if query_area.width > 0 {
            let query_width = Span::raw(query_text).width().min(usize::from(u16::MAX)) as u16;
            let offset = if self.editing {
                query_width.saturating_sub(query_area.width.saturating_sub(1))
            } else {
                0
            };
            frame.render_widget(Paragraph::new(query_text).scroll((0, offset)), query_area);
            if self.editing && focused {
                let caret = if self.query.is_empty() {
                    0
                } else {
                    query_width.saturating_sub(offset)
                }
                .min(query_area.width - 1);
                frame.set_cursor_position((query_area.x + caret, query_area.y));
            }
        }
        if header_height > 1 {
            let status = if self.pending {
                if self.capturing {
                    "Capturing…".into()
                } else {
                    "Searching…".into()
                }
            } else if let Some(notice) = self.notice {
                notice.into()
            } else if self.error.is_some() {
                "Review unavailable".into()
            } else if let Some(data) = &self.data {
                let count = data.matches.matches.len();
                if self.query.is_empty() {
                    format!(
                        "{} {} · col {} ↔",
                        data.rows.len(),
                        if data.rows.len() == 1 { "row" } else { "rows" },
                        usize::from(self.left) + 1
                    )
                } else if count == 0 {
                    "No matches".into()
                } else {
                    format!(
                        "{}/{}{} · col {} ↔",
                        self.current.map_or(0, |index| index + 1),
                        count,
                        if data.matches.capped { "+ capped" } else { "" },
                        usize::from(self.left) + 1
                    )
                }
            } else {
                String::new()
            };
            let text = Line::from(vec![
                Span::styled(filter_label(self.filter), Style::default().fg(ACCENT)),
                Span::styled(format!(" · {status}"), Style::default().fg(MUTED)),
            ]);
            frame.render_widget(
                Paragraph::new(text),
                Rect::new(inner.x, inner.y + 1, inner.width, 1),
            );
        }
        if !body.is_empty() {
            if let Some(error) = &self.error {
                frame.render_widget(
                    Paragraph::new(error.as_str())
                        .style(Style::default().fg(ERROR))
                        .wrap(Wrap { trim: false }),
                    body,
                );
            } else if self.pending {
                frame.render_widget(
                    Paragraph::new(if self.capturing {
                        "Capturing retained console…"
                    } else {
                        "Updating snapshot review…"
                    })
                    .style(Style::default().fg(MUTED)),
                    body,
                );
            } else if let Some(data) = &self.data {
                if data.rows.is_empty() {
                    frame.render_widget(
                        Paragraph::new("No lines in this filter.")
                            .style(Style::default().fg(MUTED)),
                        body,
                    );
                } else if data.snapshot.plain_text().is_empty() {
                    frame.render_widget(
                        Paragraph::new("Snapshot is empty.").style(Style::default().fg(MUTED)),
                        body,
                    );
                } else {
                    self.render_rows(frame, body, data);
                }
            }
        }
        if footer_height > 0 {
            let hint = if self.editing {
                "Enter read · Esc stop editing"
            } else {
                "n/N matches · f filter · / edit"
            };
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(MUTED)),
                Rect::new(inner.x, inner.bottom() - footer_height, inner.width, 1),
            );
        }
        if footer_height > 1 {
            let hint = if self.editing {
                "Snapshot · live output continues"
            } else {
                "r refresh · Esc resume live"
            };
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(MUTED)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    }

    fn details_lines(&self, width: u16) -> Vec<Line<'static>> {
        let fields = if let Some(Details::Operation(message)) = &self.details {
            vec![
                "Latest operation outcome (live; not frozen metadata).".into(),
                message.clone(),
            ]
        } else {
            let Some(source) = &self.source else {
                return Vec::new();
            };
            let captured = match source.captured_at.duration_since(UNIX_EPOCH) {
                Ok(time) => format!("{} s", time.as_secs()),
                Err(error) => format!("-{} s", error.duration().as_secs()),
            };
            let mut fields = vec![format!("Frozen source ID: {}", source.id)];
            if self.source_shortened {
                fields.push("Shortened: label 96; fields 128.".into());
                fields.push("… marks omitted characters.".into());
            }
            fields.extend([
                format!("Label: {}", source.label),
                format!(
                    "Device: {}",
                    source.device.as_deref().unwrap_or("(not supplied)")
                ),
                format!(
                    "Configuration: {}",
                    source.configuration.as_deref().unwrap_or("(not supplied)")
                ),
                format!(
                    "Outcome: {}",
                    source.outcome.as_deref().unwrap_or("(not supplied)")
                ),
                format!("Captured: Unix UTC {captured}"),
            ]);
            fields
        };
        // Wrap this small, bounded metadata explicitly so scroll bounds and
        // rendering agree, including long words and wide/combining graphemes.
        let mut lines = Vec::new();
        if width == 0 {
            return lines;
        }
        for field in fields {
            let span = Span::raw(field);
            let mut line = String::new();
            let mut columns = 0;
            for grapheme in span.styled_graphemes(Style::default()) {
                let size = Span::raw(grapheme.symbol).width();
                if columns + size > usize::from(width) && !line.is_empty() {
                    lines.push(Line::raw(std::mem::take(&mut line)));
                    columns = 0;
                }
                if size > usize::from(width) {
                    line.push('…');
                    columns += 1;
                } else {
                    line.push_str(grapheme.symbol);
                    columns += size;
                }
            }
            lines.push(Line::raw(line));
        }
        lines
    }

    fn render_details(&mut self, frame: &mut Frame<'_>, area: Rect) {
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(if self.showing_operation_details() {
                " Latest operation outcome "
            } else {
                " Frozen source details "
            })
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().fg(TEXT).bg(PANEL));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.source_viewport = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let lines = self.details_lines(inner.width);
        let max = lines
            .len()
            .saturating_sub(usize::from(self.source_viewport.height));
        self.source_scroll = self.source_scroll.min(max);
        if !self.source_viewport.is_empty() {
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((self.source_scroll.min(usize::from(u16::MAX)) as u16, 0)),
                self.source_viewport,
            );
        }
        if !inner.is_empty() {
            frame.render_widget(
                Paragraph::new("↑↓ PgUp/PgDn · Esc back").style(Style::default().fg(MUTED)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    }

    fn render_rows(&self, frame: &mut Frame<'_>, body: Rect, data: &ReviewData) {
        let snapshot = &data.snapshot;
        for y in 0..body.height {
            let Some(row_index) = data.rows.snapshot_row(self.top + usize::from(y)) else {
                break;
            };
            let row = &snapshot.rows()[row_index];
            let right = self.left.saturating_add(body.width);
            // Find whole-cell byte boundaries before touching text. This bounds
            // drawing to the viewport even for a 65k-column historical row.
            let start = byte_at_column(snapshot, row.bytes.clone(), self.left, false);
            let end = byte_at_column(snapshot, row.bytes.clone(), right, true);
            if start >= end {
                continue;
            }
            let matches = &data.matches.matches;
            let first = matches.partition_point(|found| found.bytes.end <= start);
            let mut highlighted: Vec<Range<u16>> = Vec::new();
            let mut current = None;
            for (index, found) in matches
                .iter()
                .enumerate()
                .skip(first)
                .take_while(|(_, found)| found.bytes.start < end)
            {
                // A long match can cross hundreds of narrow wrapped rows.
                // Map only this row, retaining the original match index/style.
                let bytes =
                    found.bytes.start.max(row.bytes.start)..found.bytes.end.min(row.bytes.end);
                for cells in snapshot.cell_ranges(bytes) {
                    if self.current == Some(index) {
                        current = Some(cells.columns.clone());
                    }
                    if let Some(previous) = highlighted.last_mut()
                        && previous.end >= cells.columns.start
                    {
                        previous.end = previous.end.max(cells.columns.end);
                    } else {
                        highlighted.push(cells.columns);
                    }
                }
            }
            let mut byte = start;
            let mut highlight = 0;
            while byte < end {
                let Some(cells) = snapshot.cell_ranges(byte..byte + 1).next() else {
                    break;
                };
                let next = if snapshot.plain_text().as_bytes()[byte].is_ascii()
                    && (byte + 1 == end || snapshot.plain_text().as_bytes()[byte + 1].is_ascii())
                {
                    byte + 1
                } else {
                    byte_at_column(snapshot, byte..end, cells.columns.end, false)
                };
                if next <= byte {
                    break;
                }
                while highlighted
                    .get(highlight)
                    .is_some_and(|range| range.end <= cells.columns.start)
                {
                    highlight += 1;
                }
                let active = current.as_ref().is_some_and(|range| {
                    range.start < cells.columns.end && range.end > cells.columns.start
                });
                let marked = highlighted
                    .get(highlight)
                    .is_some_and(|range| range.start < cells.columns.end);
                let style = if active {
                    Style::default()
                        .fg(PANEL)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD)
                } else if marked {
                    Style::default().fg(TEXT).bg(MATCH)
                } else {
                    Style::default().fg(TEXT).bg(PANEL)
                };
                let x = body.x + cells.columns.start.saturating_sub(self.left);
                frame.buffer_mut().set_stringn(
                    x,
                    body.y + y,
                    &snapshot.plain_text()[byte..next],
                    usize::from(cells.columns.end - cells.columns.start),
                    style,
                );
                // Ratatui resets wide continuation cells while writing a
                // grapheme. Keep their background/style aligned with its head.
                for continuation in 1..cells.columns.end - cells.columns.start {
                    frame.buffer_mut()[(x + continuation, body.y + y)].set_style(style);
                }
                byte = next;
            }
        }
    }
}

/// Lower bound by original cell coordinates; returned bytes are complete cell
/// boundaries, so matching only a combining mark still colors its base cell.
fn byte_at_column(
    snapshot: &ConsoleSnapshot,
    bytes: Range<usize>,
    column: u16,
    by_end: bool,
) -> usize {
    let mut low = bytes.start;
    let mut high = bytes.end;
    while low < high {
        let middle = low + (high - low) / 2;
        let before = snapshot
            .cell_ranges(middle..middle + 1)
            .next()
            .is_some_and(|cells| {
                if by_end {
                    cells.columns.end <= column
                } else {
                    cells.columns.start < column
                }
            });
        if before {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}
fn filter_label(filter: SeverityFilter) -> &'static str {
    match filter {
        SeverityFilter::All => "All",
        SeverityFilter::Errors => "Errors",
        SeverityFilter::Warnings => "Warnings",
        SeverityFilter::ErrorsAndWarnings => "Err+Warn",
    }
}
fn display_label(value: &str, limit: usize) -> String {
    bounded_label(value, limit).0
}
fn bounded_label(value: &str, limit: usize) -> (String, bool) {
    // One lookahead scalar detects omitted text. Its boundary grapheme is
    // discarded as a whole, including a partial ZWJ/combining sequence.
    let sanitized: String = value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit + 1)
        .collect();
    if sanitized.chars().count() <= limit {
        return (sanitized, false);
    }
    let mut label = String::new();
    let mut scalars = 0;
    let span = Span::raw(&sanitized);
    for grapheme in span.styled_graphemes(Style::default()) {
        let size = grapheme.symbol.chars().count();
        if scalars + size >= limit {
            break;
        }
        label.push_str(grapheme.symbol);
        scalars += size;
    }
    if limit > 0 {
        label.push('…');
    }
    (label, true)
}
fn clipped_title(text: &str, width: u16) -> String {
    let span = Span::raw(text);
    if span.width() <= usize::from(width) {
        return text.to_owned();
    }
    let mut title = String::new();
    if width == 0 {
        return title;
    }
    let mut columns = 0;
    for grapheme in span.styled_graphemes(Style::default()) {
        let size = Span::raw(grapheme.symbol).width();
        if columns + size > usize::from(width - 1) {
            break;
        }
        title.push_str(grapheme.symbol);
        columns += size;
    }
    title.push('…');
    title
}
