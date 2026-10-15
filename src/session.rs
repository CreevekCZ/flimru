//! A real PTY keeps Flutter's terminal detection and single-key controls working.
use crate::animation::AnimationControl;
use crate::tool::RunCommand;
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

enum Output {
    Bytes(Vec<u8>),
    Closed,
    Error(String),
}

pub struct Session {
    pub parser: vt100::Parser,
    pub animations: AnimationControl,
    pub scrollback_len: usize,
    pub visual_revision: u64,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Output>,
    reader: Option<JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
    pending: Vec<u8>,
    pub stop_requested: Option<Instant>,
    pub read_error: Option<String>,
    closed: bool,
    exited: Option<(portable_pty::ExitStatus, Instant)>,
    interrupt_sent: bool,
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Session {
    pub fn start(plan: &RunCommand, rows: u16, cols: u16) -> Result<Self> {
        Self::start_with_color(plan, rows, cols, None)
    }

    /// An explicit color choice applies to this child only. launch.json env
    /// values are applied last, so project-specific overrides remain honored.
    pub fn start_with_color(
        plan: &RunCommand,
        rows: u16,
        cols: u16,
        colors: Option<bool>,
    ) -> Result<Self> {
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .context("Could not open a pseudo-terminal")?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        #[cfg(unix)]
        if let Some(fd) = pair.master.as_raw_fd() {
            // The cloned reader and writer share this file description. Both are
            // nonblocking, so neither a full output channel nor pasted input can
            // hold up shutdown or the UI.
            // SAFETY: pair.master owns this live descriptor. F_GETFL takes no
            // extra argument and leaves descriptor ownership unchanged.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            // SAFETY: the master remains live; F_SETFL receives the required
            // integer flags argument and does not transfer ownership.
            if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let mut command = CommandBuilder::new(&plan.executable);
        command.args(&plan.args);
        command.cwd(&plan.cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        match colors {
            Some(true) => {
                command.env_remove("NO_COLOR");
                command.env("CLICOLOR", "1");
                command.env("CLICOLOR_FORCE", "1");
                command.env("FORCE_COLOR", "3");
            }
            Some(false) => {
                command.env("NO_COLOR", "1");
                command.env("CLICOLOR", "0");
                command.env("CLICOLOR_FORCE", "0");
                command.env("FORCE_COLOR", "0");
            }
            None => {}
        }
        for (name, value) in &plan.env {
            command.env(name, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("Could not start {}", plan.executable.display()))?;
        drop(pair.slave);
        let (tx, output) = mpsc::sync_channel(128);
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = cancelled.clone();
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0; 8192];
            while !reader_cancelled.load(Ordering::Relaxed) {
                let message = match reader.read(&mut buffer) {
                    Ok(0) => Output::Closed,
                    Ok(count) => Output::Bytes(buffer[..count].to_vec()),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Linux PTYs return EIO when their last slave closes.
                    #[cfg(unix)]
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => Output::Closed,
                    Err(error) => Output::Error(error.to_string()),
                };
                let done = !matches!(message, Output::Bytes(_));
                let mut pending = message;
                loop {
                    match tx.try_send(pending) {
                        Ok(()) => break,
                        Err(TrySendError::Disconnected(_)) => return,
                        Err(TrySendError::Full(message)) => {
                            if reader_cancelled.load(Ordering::Relaxed) {
                                return;
                            }
                            pending = message;
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
                if done {
                    break;
                }
            }
        });
        Ok(Self {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), 10_000),
            animations: AnimationControl::new(!plan.args.iter().any(|arg| arg == "--release")),
            scrollback_len: 0,
            visual_revision: 0,
            child,
            master: pair.master,
            writer,
            output,
            reader: Some(reader_thread),
            cancelled,
            pending: vec![],
            stop_requested: None,
            read_error: None,
            closed: false,
            exited: None,
            interrupt_sent: false,
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if self.pending.len() + bytes.len() > 65_536 {
            bail!("Input queue is full; wait for Flutter to read input");
        }
        self.parser.screen_mut().set_scrollback(0);
        self.pending.extend_from_slice(bytes);
        self.flush_input()
    }

    fn flush_input(&mut self) -> Result<()> {
        while !self.pending.is_empty() {
            match self.writer.write(&self.pending) {
                Ok(0) => bail!("Flutter's input stream closed"),
                Ok(count) => {
                    self.pending.drain(..count);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn request_stop(&mut self) -> Result<()> {
        if self.stop_requested.is_some() {
            self.force_stop();
            return Ok(());
        }
        self.stop_requested = Some(Instant::now());
        self.send(b"q")
    }

    pub fn force_stop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.process_id() {
            // portable-pty starts the command as a new session/group leader.
            // Signal only the owned group, including build/SDK subprocesses.
            // SAFETY: kill accesses no Rust memory. portable-pty created this
            // child's process group, and its negative id addresses that group.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
    }

    /// Drains a bounded amount of output per frame and polls for completion.
    pub fn tick(&mut self) -> Result<Option<portable_pty::ExitStatus>> {
        self.tick_with_budget(Duration::from_millis(8))
    }

    /// Smaller per-session budgets keep a batch responsive under output floods.
    pub fn tick_with_budget(
        &mut self,
        budget: Duration,
    ) -> Result<Option<portable_pty::ExitStatus>> {
        let started = Instant::now();
        let mut had_output = false;
        // Input errors must not hide a process exit that happened concurrently.
        if let Err(error) = self.flush_input() {
            self.read_error = Some(error.to_string());
            self.pending.clear();
        }
        for _ in 0..64 {
            match self.output.try_recv() {
                Ok(Output::Bytes(bytes)) => {
                    had_output = true;
                    self.animations.feed(&bytes);
                    self.parser.process(&bytes);
                    self.visual_revision = self.visual_revision.wrapping_add(1);
                }
                Ok(Output::Closed) => self.closed = true,
                Ok(Output::Error(error)) => {
                    self.read_error = Some(error);
                    self.closed = true;
                }
                Err(_) => break,
            }
            // Keep flooding child output from monopolizing the UI thread.
            if started.elapsed() >= budget {
                break;
            }
        }
        self.update_scroll_bounds();
        if self.exited.is_none()
            && let Some(status) = self.child.try_wait()?
        {
            self.exited = Some((status, Instant::now()));
        }
        if let Some(start) = self.stop_requested.filter(|_| self.exited.is_none()) {
            if start.elapsed() >= Duration::from_secs(8) {
                self.force_stop();
            } else if start.elapsed() >= Duration::from_secs(1) && !self.interrupt_sent {
                // Tests and builds don't consume `q`; interrupt their owned group
                // so Flutter has a chance to clean up before the hard deadline.
                #[cfg(unix)]
                if let Some(pid) = self.child.process_id() {
                    // SAFETY: kill accesses no Rust memory; the negative id
                    // targets the group portable-pty created for this child.
                    unsafe {
                        libc::kill(-(pid as libc::pid_t), libc::SIGINT);
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = self.send(&[3]);
                }
                self.interrupt_sent = true;
            }
        }
        // Usually EOF follows immediately. A descendant retaining the PTY must
        // not prevent returning to the picker; Drop cleans up that owned group.
        if let Some((status, when)) = &self.exited
            && (self.closed
                || (!had_output && when.elapsed() >= Duration::from_millis(300))
                || when.elapsed() >= Duration::from_secs(2))
        {
            return Ok(Some(status.clone()));
        }
        Ok(None)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.parser.screen().size() != (rows.max(1), cols.max(1)) {
            self.master.resize(size(rows, cols))?;
            self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
            self.update_scroll_bounds();
            self.visual_revision = self.visual_revision.wrapping_add(1);
        }
        Ok(())
    }

    pub fn scroll(&mut self, amount: isize) {
        let offset = self
            .parser
            .screen()
            .scrollback()
            .saturating_add_signed(amount);
        self.parser.screen_mut().set_scrollback(offset);
    }

    pub fn snapshot(&self) -> vt100::Screen {
        self.parser.screen().clone()
    }

    /// Transfer completed console history without duplicating its 10k-row heap.
    pub fn take_console(&mut self) -> vt100::Parser {
        std::mem::replace(&mut self.parser, vt100::Parser::new(1, 1, 0))
    }

    fn update_scroll_bounds(&mut self) {
        let screen = self.parser.screen_mut();
        let offset = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        self.scrollback_len = screen.scrollback();
        screen.set_scrollback(offset);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.force_stop();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            #[cfg(unix)]
            {
                let _ = reader.join();
            }
            #[cfg(not(unix))]
            {
                drop(reader);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn command(script: &str) -> RunCommand {
        RunCommand {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: Default::default(),
        }
    }

    #[test]
    fn pty_controls_resize_and_exit() {
        let mut session = Session::start(&command("stty raw -echo; printf 'ready\\r\\n'; while :; do c=$(dd bs=1 count=1 2>/dev/null); printf 'key:%s\\r\\n' \"$c\"; [ \"$c\" = q ] && exit 7; done"), 20, 90).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.parser.screen().contents().contains("ready") {
            assert!(Instant::now() < deadline);
            session.tick().unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        session.resize(15, 60).unwrap();
        assert_eq!(session.parser.screen().size(), (15, 60));
        session.send(b"h").unwrap();
        session.send(b"v").unwrap();
        session.request_stop().unwrap();
        loop {
            assert!(Instant::now() < deadline);
            if let Some(status) = session.tick().unwrap() {
                assert_eq!(status.exit_code(), 7);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(session.parser.screen().contents().contains("key:h"));
        assert!(session.parser.screen().contents().contains("key:v"));
    }

    #[test]
    fn ignored_quit_can_be_forced_and_reaped() {
        let mut session =
            Session::start(&command("trap '' HUP; while :; do sleep 1; done"), 10, 40).unwrap();
        session.request_stop().unwrap();
        session.request_stop().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while session.tick().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn child_color_policy_and_project_environment_overrides_are_explicit() {
        for (colors, override_no_color, expected) in [
            (true, None, "env:xterm-256color|truecolor|3|unset"),
            (false, None, "env:xterm-256color|truecolor|0|1"),
            (
                true,
                Some("project"),
                "env:xterm-256color|truecolor|3|project",
            ),
        ] {
            let mut plan = command(
                "printf 'env:%s|%s|%s|%s' \"$TERM\" \"$COLORTERM\" \"$FORCE_COLOR\" \"${NO_COLOR-unset}\"",
            );
            if let Some(value) = override_no_color {
                plan.env.insert("NO_COLOR".into(), value.into());
            }
            let mut session = Session::start_with_color(&plan, 5, 100, Some(colors)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while session.tick().unwrap().is_none() {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(5));
            }
            assert!(session.parser.screen().contents().contains(expected));
        }
    }

    #[test]
    fn retained_console_moves_its_cells_without_copying_history() {
        let mut session = Session::start(
            &command("printf 'saved output\\r\\n'; exec sleep 30"),
            12,
            60,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !session.parser.screen().contents().contains("saved output") {
            assert!(Instant::now() < deadline);
            session.tick().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
        let cell = session.parser.screen().cell(0, 0).unwrap() as *const vt100::Cell;
        let console = session.take_console();
        assert_eq!(
            cell,
            console.screen().cell(0, 0).unwrap() as *const vt100::Cell
        );
        assert!(console.screen().contents().contains("saved output"));
        assert_eq!(session.parser.screen().size(), (1, 1));
    }
}
