//! A bounded batch owns each child and console independently.
use flimru::{
    batch_commands::{BatchTarget, HotCapabilities, HotExclusion},
    device::Device,
    session::Session,
};

pub const MAX_RUNS: usize = 8;
pub type RunId = u64;
static NEXT_RUN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Console identities survive vector removal and live-to-ended transfer. At a
/// million launches per second the u64 space lasts more than half a million years.
pub fn next_id() -> RunId {
    NEXT_RUN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Run {
    pub id: RunId,
    pub max_columns: u16,
    pub history_capacity: usize,
    pub device: Device,
    pub config: String,
    pub hot_capabilities: HotCapabilities,
    pub session: Option<Session>,
    pub console: Option<vt100::Parser>,
    #[cfg(unix)]
    pub ended_log: Option<flimru::session_log::SessionLog>,
    pub scrollback_len: usize,
    pub outcome: Option<String>,
    pub offset: usize,
    view_rows: u16,
}

impl Run {
    pub fn running(
        device: Device,
        config: String,
        session: Session,
        hot_capabilities: HotCapabilities,
    ) -> Self {
        Self {
            id: next_id(),
            max_columns: session.max_columns,
            history_capacity: Session::SCROLLBACK_CAPACITY,
            device,
            config,
            hot_capabilities,
            session: Some(session),
            console: None,
            #[cfg(unix)]
            ended_log: None,
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
        let diagnostic = format!("Cannot start: {message}");
        console.process(diagnostic.as_bytes());
        #[cfg(unix)]
        let ended_log = {
            let mut log = flimru::session_log::SessionLog::default();
            log.feed(diagnostic.as_bytes());
            log.feed(b"\n");
            Some(log)
        };
        Self {
            id: next_id(),
            max_columns: cols.max(1),
            history_capacity: 0,
            device,
            config,
            hot_capabilities: HotCapabilities::Unsupported(HotExclusion::CustomRunner),
            session: None,
            console: Some(console),
            #[cfg(unix)]
            ended_log,
            scrollback_len: 0,
            outcome: Some(format!("failed: {message}")),
            offset: 0,
            view_rows: rows.max(1),
        }
    }

    pub fn batch_target(&self) -> BatchTarget {
        BatchTarget {
            id: self.id,
            device: flimru::batch_commands::bounded_device_label(&self.device.name),
            capabilities: self.hot_capabilities,
            live: self.session.is_some(),
            stopping: self
                .session
                .as_ref()
                .is_some_and(|session| session.stop_requested.is_some()),
        }
    }

    #[cfg(unix)]
    pub fn log(&self) -> Option<&flimru::session_log::SessionLog> {
        self.session
            .as_ref()
            .map(|session| &session.log)
            .or(self.ended_log.as_ref())
    }

    pub fn screen(&self) -> Option<&vt100::Screen> {
        self.session
            .as_ref()
            .map(|s| s.parser.screen())
            .or_else(|| self.console.as_ref().map(vt100::Parser::screen))
    }

    pub fn historical_columns(&self) -> u16 {
        self.max_columns.max(
            self.session
                .as_ref()
                .map_or(0, |session| session.max_columns),
        )
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
            #[cfg(unix)]
            {
                self.ended_log = Some(session.take_log());
            }
            self.offset = session.parser.screen().scrollback();
            self.scrollback_len = session.scrollback_len;
            self.max_columns = self.max_columns.max(session.max_columns);
            self.console = Some(session.take_console());
            self.outcome = Some(outcome);
            self.session.take();
            return true;
        }
        changed
    }
}
