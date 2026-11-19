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

// Some device log transports render ESC as \^[ (sometimes with an extra
// backslash). Decode only SGR sequences, leaving other escaped text untouched.
#[derive(Default)]
struct EscapedSgr {
    pending: Vec<u8>,
}

impl EscapedSgr {
    fn feed(&mut self, bytes: &[u8], end: bool) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::with_capacity(self.pending.len());
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i] != b'\\' {
                output.push(self.pending[i]);
                i += 1;
                continue;
            }
            let start = i;
            let mut j = i + 1;
            if self.pending.get(j) == Some(&b'\\') {
                j += 1;
            }
            let prefix = b"^[[";
            let available = &self.pending[j..];
            if available.len() < prefix.len() && prefix.starts_with(available) && !end {
                break;
            }
            if available.starts_with(prefix) {
                j += prefix.len();
                let parameters = j;
                while j < self.pending.len()
                    && (self.pending[j].is_ascii_digit() || self.pending[j] == b';')
                    && j - start < 64
                {
                    j += 1;
                }
                if j == self.pending.len() && j - start < 64 && !end {
                    break;
                }
                if self.pending.get(j) == Some(&b'm') {
                    output.extend_from_slice(b"\x1b[");
                    output.extend_from_slice(&self.pending[parameters..=j]);
                    i = j + 1;
                    continue;
                }
            }
            output.push(self.pending[i]);
            i += 1;
        }
        self.pending.drain(..i);
        output
    }
}

pub struct Session {
    pub parser: vt100::Parser,
    #[cfg(unix)]
    pub log: crate::session_log::SessionLog,
    #[cfg(unix)]
    pub interactive_flutter: bool,
    pub animations: AnimationControl,
    pub scrollback_len: usize,
    /// Lifetime width bound: vt100 retains old scrollback rows across resize.
    pub max_columns: u16,
    pub visual_revision: u64,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Output>,
    reader: Option<JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
    pending: Vec<u8>,
    escaped_sgr: EscapedSgr,
    pub stop_requested: Option<Instant>,
    pub read_error: Option<String>,
    closed: bool,
    exited: Option<(portable_pty::ExitStatus, Instant)>,
    interrupt_sent: bool,
    // Dropped after the animation worker and after the child has exited.
    _service_directory: Option<tempfile::TempDir>,
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(unix)]
fn wait_for_output(fd: Option<std::os::fd::RawFd>) -> std::io::Result<()> {
    let Some(fd) = fd else {
        thread::sleep(Duration::from_millis(5));
        return Ok(());
    };
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: descriptor is writable for one pollfd. The caller keeps master
    // alive until the reader is joined, so fd is valid throughout this call.
    // The deadline bounds cancellation when a descendant retains an idle slave.
    let ready = unsafe { libc::poll(&mut descriptor, 1, 100) };
    if ready < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    if descriptor.revents & libc::POLLNVAL != 0 {
        return Err(std::io::Error::from_raw_os_error(libc::EBADF));
    }
    Ok(())
}

impl Session {
    pub const SCROLLBACK_CAPACITY: usize = 10_000;
    /// Request colored child output while honoring explicit launch.json env overrides.
    pub fn start(plan: &RunCommand, rows: u16, cols: u16) -> Result<Self> {
        let mut args = plan.args.clone();
        // Preserve an explicitly configured output file. Custom runners, tests,
        // and release builds continue to use console detection only.
        let service_directory = if plan.capture_vm_service
            && !args.iter().any(|arg| {
                arg == "--vmservice-out-file" || arg.starts_with("--vmservice-out-file=")
            }) {
            let mut directory = tempfile::Builder::new();
            directory.prefix("flimru-vm-service-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                directory.permissions(std::fs::Permissions::from_mode(0o700));
            }
            let directory = directory.tempdir()?;
            let position = args
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or(args.len());
            args.insert(
                position,
                format!(
                    "--vmservice-out-file={}",
                    directory.path().join("service-uri").display()
                ),
            );
            Some(directory)
        } else {
            None
        };
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .context("Could not open a pseudo-terminal")?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        #[cfg(unix)]
        let reader_fd = pair.master.as_raw_fd();
        #[cfg(unix)]
        if let Some(fd) = reader_fd {
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
        command.args(&args);
        command.cwd(&plan.cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env_remove("NO_COLOR");
        command.env("CLICOLOR", "1");
        command.env("CLICOLOR_FORCE", "1");
        command.env("FORCE_COLOR", "3");
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
                        #[cfg(unix)]
                        match wait_for_output(reader_fd) {
                            Ok(()) => continue,
                            Err(error) => Output::Error(error.to_string()),
                        }
                        #[cfg(not(unix))]
                        {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
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
            parser: vt100::Parser::new(rows.max(1), cols.max(1), Self::SCROLLBACK_CAPACITY),
            #[cfg(unix)]
            log: Default::default(),
            #[cfg(unix)]
            interactive_flutter: !matches!(
                plan.hot_capabilities,
                crate::batch_commands::HotCapabilities::Unsupported(
                    crate::batch_commands::HotExclusion::Test
                        | crate::batch_commands::HotExclusion::CustomRunner
                )
            ) && !plan.args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "--machine" | "--machine=true" | "--no-resident" | "--resident=false"
                )
            }),
            animations: match &service_directory {
                Some(directory) => {
                    AnimationControl::with_service_file(directory.path().join("service-uri"))
                }
                None => AnimationControl::new(
                    !plan
                        .args
                        .iter()
                        .any(|arg| matches!(arg.as_str(), "--release" | "--release=true")),
                ),
            },
            scrollback_len: 0,
            max_columns: cols.max(1),
            visual_revision: 0,
            child,
            master: pair.master,
            writer,
            output,
            reader: Some(reader_thread),
            cancelled,
            pending: vec![],
            escaped_sgr: EscapedSgr::default(),
            stop_requested: None,
            read_error: None,
            closed: false,
            exited: None,
            interrupt_sent: false,
            _service_directory: service_directory,
        })
    }

    pub fn reconnect_animations(&mut self) -> Result<()> {
        self.animations.reconnect()?;
        // Flutter's help command reprints debugger addresses without restarting
        // the app. This also recovers custom runners without a service file.
        self.send(b"h")
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if self.pending.len() + bytes.len() > 65_536 {
            bail!("Input queue is full; wait for Flutter to read input");
        }
        self.parser.screen_mut().set_scrollback(0);
        self.pending.extend_from_slice(bytes);
        if bytes == b"R" {
            self.animations.restarting();
        }
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
                    self.process_output(&bytes, false);
                    self.visual_revision = self.visual_revision.wrapping_add(1);
                }
                Ok(Output::Closed) => {
                    self.process_output(&[], true);
                    self.closed = true;
                }
                Ok(Output::Error(error)) => {
                    self.read_error = Some(error);
                    self.process_output(&[], true);
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

    fn process_output(&mut self, bytes: &[u8], end: bool) {
        let normalized = self.escaped_sgr.feed(bytes, end);
        self.parser.process(&normalized);
        #[cfg(unix)]
        self.log.feed(&normalized);
    }

    #[cfg(unix)]
    pub fn take_log(&mut self) -> crate::session_log::SessionLog {
        self.process_output(&[], true);
        self.log.finish();
        std::mem::take(&mut self.log)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.parser.screen().size() != (rows.max(1), cols.max(1)) {
            self.master.resize(size(rows, cols))?;
            self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
            self.max_columns = self.max_columns.max(cols.max(1));
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

    #[test]
    fn escaped_device_colors_survive_chunk_boundaries_and_preserve_other_text() {
        let input = br"flutter: \^[[2m[AutoRoute]\^[[0m \\^[[90mFINEST:\\^[[0m Popped \^[[2J";
        for split in 0..=input.len() {
            let mut decoder = super::EscapedSgr::default();
            let mut output = decoder.feed(&input[..split], false);
            output.extend(decoder.feed(&input[split..], false));
            output.extend(decoder.feed(&[], true));
            assert_eq!(
                output,
                b"flutter: \x1b[2m[AutoRoute]\x1b[0m \x1b[90mFINEST:\x1b[0m Popped \\^[[2J"
            );
            let mut parser = vt100::Parser::new(2, 100, 0);
            parser.process(&output);
            assert!(
                parser
                    .screen()
                    .contents()
                    .contains("[AutoRoute] FINEST: Popped")
            );
            assert_eq!(
                parser.screen().cell(0, 21).unwrap().fgcolor(),
                vt100::Color::Idx(8)
            );
        }
    }
    use super::*;
    fn command(script: &str) -> RunCommand {
        RunCommand {
            capture_vm_service: false,
            hot_capabilities: crate::batch_commands::HotCapabilities::Supported,
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: Default::default(),
        }
    }

    #[test]
    fn service_files_are_private_per_session_and_cleaned_after_children_exit() {
        let directory = tempfile::tempdir().unwrap();
        let mut plan = command(
            "printf '%s' \"$1\" > \"$URI_LOG\"; printf 'CAPTURE READY\\r\\n'; exec sleep 30",
        );
        plan.capture_vm_service = true;
        plan.args.push("fixture".into());
        // Capture must be a Flutter option, before the application separator.
        plan.args
            .extend(["--".into(), "application argument".into()]);
        plan.env.insert(
            "URI_LOG".into(),
            directory
                .path()
                .join("uri.log")
                .to_string_lossy()
                .into_owned(),
        );
        let mut first = Session::start(&plan, 10, 80).unwrap();
        let first_path = first
            ._service_directory
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&first_path).unwrap().permissions().mode() & 0o077,
            0
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !first.parser.screen().contents().contains("CAPTURE READY") {
            assert!(Instant::now() < deadline);
            first.tick().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(directory.path().join("uri.log")).unwrap(),
            format!(
                "--vmservice-out-file={}",
                first_path.join("service-uri").display()
            )
        );
        let second = Session::start(&plan, 10, 80).unwrap();
        let second_path = second
            ._service_directory
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        assert_ne!(first_path, second_path);
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        drop(second);
        assert!(!second_path.exists());
    }

    #[test]
    fn explicit_service_output_is_not_overridden_or_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("user-service-uri");
        std::fs::write(&path, "user data").unwrap();
        let mut plan = command("exec sleep 30");
        plan.capture_vm_service = true;
        plan.args.extend([
            "fixture".into(),
            format!("--vmservice-out-file={}", path.display()),
        ]);
        let session = Session::start(&plan, 10, 80).unwrap();
        assert!(session._service_directory.is_none());
        drop(session);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "user data");
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
        assert_eq!(session.max_columns, 90);
        session.resize(15, 120).unwrap();
        session.resize(15, 60).unwrap();
        assert_eq!(session.parser.screen().size(), (15, 60));
        assert_eq!(session.max_columns, 120);
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
        for (override_no_color, expected) in [
            (None, "env:xterm-256color|truecolor|3|unset"),
            (Some("project"), "env:xterm-256color|truecolor|3|project"),
        ] {
            let mut plan = command(
                "printf 'env:%s|%s|%s|%s' \"$TERM\" \"$COLORTERM\" \"$FORCE_COLOR\" \"${NO_COLOR-unset}\"",
            );
            if let Some(value) = override_no_color {
                plan.env.insert("NO_COLOR".into(), value.into());
            }
            let mut session = Session::start(&plan, 5, 100).unwrap();
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
