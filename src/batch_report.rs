//! Frozen delivery results only: this view never sends or retries a command.
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use flimru::batch_commands::{BatchReport, HotCommand, TargetOutcome, bounded_device_label};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

const ACCENT: Color = Color::Rgb(96, 210, 225);
const MUTED: Color = Color::Rgb(151, 163, 180);
const PANEL: Color = Color::Rgb(22, 28, 39);
const TEXT: Color = Color::Rgb(224, 232, 240);
const GREEN: Color = Color::Rgb(128, 214, 159);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportAction {
    Consumed,
    Closed,
}

/// The report is owned and immutable. Only the viewport and dismissal change;
/// no current tab, run-vector position, or live session is consulted.
pub struct BatchReportView {
    command: HotCommand,
    report: BatchReport,
    open: bool,
    offset: usize,
    max_offset: usize,
    viewport: Rect,
    close_region: Rect,
    area: Rect,
}

impl BatchReportView {
    pub fn new(command: HotCommand, report: BatchReport) -> Self {
        Self {
            command,
            report,
            open: true,
            offset: 0,
            max_offset: 0,
            viewport: Rect::default(),
            close_region: Rect::default(),
            area: Rect::default(),
        }
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Hit testing always uses the most recently rendered button cells.
    #[cfg(test)]
    pub fn close_region(&self) -> Rect {
        self.close_region
    }

    pub fn covers(&self, position: ratatui::layout::Position) -> bool {
        // Before first render or after resize, no underlying pointer target is
        // proven visible. Keyboard global quit remains available to the caller.
        self.area.is_empty() || self.area.contains(position)
    }

    /// Global quit and modal precedence belong to the caller. Every other event
    /// is consumed, including paste, unknown keys, and background pointer input.
    pub fn handle_event(&mut self, event: &Event) -> ReportAction {
        if !self.open {
            return ReportAction::Consumed;
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter | KeyCode::Esc => return self.close(),
                KeyCode::Up => self.scroll(-1),
                KeyCode::Down => self.scroll(1),
                KeyCode::PageUp => self.scroll(-(self.viewport.height.max(1) as isize)),
                KeyCode::PageDown => self.scroll(self.viewport.height.max(1) as isize),
                KeyCode::Home => self.offset = 0,
                KeyCode::End => self.offset = self.max_offset,
                _ => {}
            },
            Event::Mouse(mouse) => {
                let position = (mouse.column, mouse.row).into();
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && self.close_region.contains(position)
                {
                    return self.close();
                }
                if self.viewport.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => self.scroll(-3),
                        MouseEventKind::ScrollDown => self.scroll(3),
                        _ => {}
                    }
                }
            }
            Event::Resize(_, _) => {
                // Reject stale pointer regions until the caller renders again.
                self.close_region = Rect::default();
                self.viewport = Rect::default();
                self.area = Rect::default();
            }
            _ => {}
        }
        ReportAction::Consumed
    }

    fn close(&mut self) -> ReportAction {
        self.open = false;
        self.close_region = Rect::default();
        self.viewport = Rect::default();
        self.area = Rect::default();
        ReportAction::Closed
    }

    fn scroll(&mut self, rows: isize) {
        self.offset = self.offset.saturating_add_signed(rows).min(self.max_offset);
    }

    pub fn render(&mut self, frame: &mut Frame) {
        self.close_region = Rect::default();
        self.viewport = Rect::default();
        self.area = Rect::default();
        if !self.open {
            return;
        }
        let screen = frame.area();
        if screen.width < 34 || screen.height < 12 {
            self.area = screen;
            self.offset = 0;
            self.max_offset = 0;
            frame.render_widget(Clear, screen);
            frame.render_widget(
                Paragraph::new(format!(
                    "{} delivery report\nEnlarge to inspect results.\nEnter/Esc closes.",
                    self.command.label()
                ))
                .style(Style::default().fg(TEXT).bg(PANEL))
                .wrap(Wrap { trim: false }),
                screen,
            );
            return;
        }
        let width = screen.width.saturating_sub(2).min(88);
        let height = screen.height.saturating_sub(2).min(24);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        self.area = area;
        let block = Block::default()
            .title(Line::styled(
                format!(" {} delivery report ", self.command.label()),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().fg(TEXT).bg(PANEL));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let content = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        let mut header = wrap(
            &format!("Requested/queued: {}", self.report.requested),
            content.width,
            Style::default().fg(ACCENT),
        );
        header.extend(wrap(
            &format!(
                "Skipped: {} · Failed: {}",
                self.report.skipped, self.report.failed
            ),
            content.width,
            Style::default().fg(TEXT),
        ));
        header.extend(wrap(
            "Flutter completion is unconfirmed.",
            content.width,
            Style::default().fg(MUTED),
        ));
        let header_height = (header.len() as u16).min(content.height.saturating_sub(2));
        frame.render_widget(
            Paragraph::new(header),
            Rect::new(content.x, content.y, content.width, header_height),
        );
        self.viewport = Rect::new(
            content.x,
            content.y + header_height,
            content.width,
            content.height.saturating_sub(header_height + 1),
        );
        let lines = self.detail_lines(content.width);
        self.max_offset = lines
            .len()
            .saturating_sub(usize::from(self.viewport.height));
        self.offset = self.offset.min(self.max_offset);
        frame.render_widget(Paragraph::new(lines[self.offset..].to_vec()), self.viewport);
        let footer = Rect::new(content.x, content.bottom() - 1, content.width, 1);
        self.close_region = Rect::new(footer.x, footer.y, 7.min(footer.width), 1);
        frame.render_widget(
            Paragraph::new("[Close]")
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            self.close_region,
        );
        let end = (self.offset + usize::from(self.viewport.height)).min(lines.len());
        let position = format!("{}-{end}/{}", self.offset + 1, lines.len());
        let hint_width = footer.width.saturating_sub(7);
        // Keep the full row range/total; shorten optional help to fit the
        // actual cells left after Close, including wider three-digit totals.
        let help = [
            " Enter/Esc · ↑↓ PgUp/PgDn · ",
            " Esc ↑↓ Pg ",
            " Esc ↑↓ ",
            " ↑↓ ",
            " ",
        ]
        .into_iter()
        .find(|help| Span::raw(*help).width() + position.len() <= usize::from(hint_width))
        .unwrap_or("");
        frame.render_widget(
            Paragraph::new(format!("{help}{position}")).style(Style::default().fg(MUTED)),
            Rect::new(footer.x + 7, footer.y, hint_width, 1),
        );
    }

    fn detail_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.report.details.is_empty() {
            return wrap(
                "No session targets in this request.",
                width,
                Style::default().fg(MUTED),
            );
        }
        let mut lines = Vec::new();
        for (index, detail) in self.report.details.iter().take(8).enumerate() {
            if index > 0 {
                lines.push(Line::raw(""));
            }
            // The dispatch helper already bounds/sanitizes labels. Bound again
            // because BatchReport is also publicly constructible by the caller.
            let device = bounded_device_label(&detail.device);
            lines.extend(wrap(
                &device,
                width,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ));
            let (label, color) = match detail.outcome {
                TargetOutcome::Requested => ("Requested/queued".to_owned(), GREEN),
                TargetOutcome::Skipped(reason) => (format!("Skipped: {}", reason.reason()), MUTED),
                TargetOutcome::Failed(reason) => {
                    (format!("Failed: {}", reason.reason()), Color::Yellow)
                }
            };
            lines.extend(wrap(&label, width, Style::default().fg(color)));
        }
        lines
    }
}

/// Explicit grapheme wrapping makes scroll bounds agree with rendered rows,
/// including wide, combining, and joined device-label characters.
fn wrap(value: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let width = usize::from(width);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut columns = 0;
    let span = Span::raw(value);
    for grapheme in span.styled_graphemes(Style::default()) {
        let size = Span::raw(grapheme.symbol).width();
        if columns + size > width && !line.is_empty() {
            // Prefer a word boundary, while long device words still wrap by
            // grapheme without losing or splitting a joined character.
            if let Some(space) = line.rfind(' ').filter(|index| *index > 0) {
                let tail = line.split_off(space + 1);
                line.pop();
                lines.push(Line::styled(std::mem::replace(&mut line, tail), style));
                columns = Span::raw(&line).width();
            }
            if columns + size > width {
                lines.push(Line::styled(std::mem::take(&mut line), style));
                columns = 0;
            }
        }
        if size > width {
            line.push('…');
            columns += 1;
        } else {
            line.push_str(grapheme.symbol);
            columns += size;
        }
    }
    lines.push(Line::styled(line, style));
    lines
}
