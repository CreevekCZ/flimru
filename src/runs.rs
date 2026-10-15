//! A bounded batch owns each child and console independently.
use flutter_run_tui::{device::Device, session::Session};

pub const MAX_RUNS: usize = 8;

pub struct Run {
    pub device: Device,
    pub config: String,
    pub session: Option<Session>,
    pub console: Option<vt100::Parser>,
    pub scrollback_len: usize,
    pub outcome: Option<String>,
    pub offset: usize,
    view_rows: u16,
}

impl Run {
    pub fn running(device: Device, config: String, session: Session) -> Self {
        Self {
            device,
            config,
            session: Some(session),
            console: None,
            scrollback_len: 0,
            outcome: None,
            offset: 0,
            view_rows: 1,
        }
    }

    pub fn failed(device: Device, config: String, error: String, rows: u16, cols: u16) -> Self {
        let mut console = vt100::Parser::new(rows.max(1), cols.max(1), 0);
        // Render errors as data, never as terminal escape instructions.
        let message: String = error.chars().filter(|c| !c.is_control()).collect();
        console.process(format!("Cannot start: {message}").as_bytes());
        Self {
            device,
            config,
            session: None,
            console: Some(console),
            scrollback_len: 0,
            outcome: Some(format!("failed: {message}")),
            offset: 0,
            view_rows: rows.max(1),
        }
    }

    pub fn screen(&self) -> Option<&vt100::Screen> {
        self.session
            .as_ref()
            .map(|s| s.parser.screen())
            .or_else(|| self.console.as_ref().map(vt100::Parser::screen))
    }

    pub fn scrollback(&self) -> usize {
        self.session
            .as_ref()
            .map(|s| s.scrollback_len)
            .unwrap_or(self.scrollback_len)
    }

    pub fn set_offset(&mut self, offset: usize) {
        if let Some(session) = &mut self.session {
            session.parser.screen_mut().set_scrollback(offset);
        } else if let Some(console) = &mut self.console {
            let hidden = console.screen().size().0.saturating_sub(self.view_rows) as usize;
            self.offset = offset.min(self.scrollback_len + hidden);
            console
                .screen_mut()
                .set_scrollback(self.offset.saturating_sub(hidden));
        }
    }

    pub fn max_offset(&self) -> usize {
        self.scrollback()
            + self
                .console
                .as_ref()
                .map(|p| p.screen().size().0.saturating_sub(self.view_rows) as usize)
                .unwrap_or(0)
    }

    pub fn offset(&self) -> usize {
        self.session
            .as_ref()
            .map(|s| s.parser.screen().scrollback())
            .unwrap_or(self.offset)
    }

    pub fn prepare(&mut self, rows: u16) {
        self.view_rows = rows.max(1);
        if self.session.is_none() {
            self.set_offset(self.offset);
        }
    }

    pub fn scroll(&mut self, amount: isize) {
        let offset = self.offset();
        self.set_offset(offset.saturating_add_signed(amount));
    }

    /// Poll hidden consoles too, so their queues cannot stall the children.
    /// On completion drop the PTY, workers and process group immediately, while
    /// transferring the bounded history without cloning it.
    pub fn tick(&mut self) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        let revision = session.visual_revision;
        let outcome = match session.tick_with_budget(std::time::Duration::from_millis(1)) {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(error) => Some(format!("error: {error:#}")),
        };
        let changed = revision != session.visual_revision;
        if let Some(outcome) = outcome {
            self.offset = session.parser.screen().scrollback();
            self.scrollback_len = session.scrollback_len;
            self.console = Some(session.take_console());
            self.outcome = Some(outcome);
            self.session.take();
            return true;
        }
        changed
    }
}
