#![cfg(unix)]
// Test fixtures intentionally fail fast on setup/protocol errors.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Exercise the real binary in a terminal; never launches a real Flutter app.
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Terminal {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    screen: vt100::Parser,
    raw_output: Vec<u8>,
}

impl Terminal {
    fn start(project: &Path, state: &Path) -> Self {
        Self::start_with_options(project, state, false)
    }

    fn start_with_options(project: &Path, state: &Path, multi: bool) -> Self {
        Self::start_with_refresh_interval(project, state, multi, 2)
    }

    fn start_with_refresh_interval(
        project: &Path,
        state: &Path,
        multi: bool,
        refresh_interval: u64,
    ) -> Self {
        Self::start_configured(project, state, multi, refresh_interval, None)
    }

    fn start_configured(
        project: &Path,
        state: &Path,
        multi: bool,
        refresh_interval: u64,
        launch: Option<&Path>,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 112,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let fd = pair.master.as_raw_fd().unwrap();
        // SAFETY: pair.master owns the live PTY descriptor. Both fcntl calls
        // use the arguments required by their commands and preserve ownership.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            assert_ne!(flags, -1);
            assert_ne!(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK), -1);
        }
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_flimru"));
        command.arg(project);
        command.args([
            "--flutter",
            project.join("fake-flutter").to_str().unwrap(),
            "--state",
            state.to_str().unwrap(),
            "--refresh-interval",
            &refresh_interval.to_string(),
        ]);
        if multi {
            command.arg("--multi");
        }
        if let Some(launch) = launch {
            command.arg("--launch");
            command.arg(launch);
        }
        command.env("TERM", "xterm-256color");
        // Reproduce launchers/shells which suppress color. Always-enabled colors must
        // override this for both the terminal backend and its Flutter child.
        command.env("NO_COLOR", "1");
        // Exercise OSC52 without modifying the real test runner's clipboard.
        command.env("SSH_CONNECTION", "flimru-pty-fixture");
        command.env("FAKE_LOG", project.join("session.log"));
        command.env("FAKE_PID", project.join("child.pid"));
        command.env("FAKE_SCRIPT", project.join("probe.dart"));
        if let Some(dart) = std::env::var_os("FLUTTER_TUI_TEST_DART") {
            command.env("FAKE_DART", dart);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        Self {
            child,
            master: pair.master,
            reader,
            writer,
            screen: vt100::Parser::new(32, 112, 1000),
            raw_output: Vec::new(),
        }
    }

    fn read(&mut self) {
        let mut buffer = [0; 8192];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if self.raw_output.len() + count > 256 * 1024 {
                        self.raw_output.clear();
                    }
                    self.raw_output.extend_from_slice(&buffer[..count]);
                    self.screen.process(&buffer[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("{error}"),
            }
        }
    }

    fn until(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.read();
            if self.screen.screen().contents().contains(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Missing {expected:?}. Screen:\n{}",
                self.screen.screen().contents()
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Launcher exited early"
            );
            thread::sleep(Duration::from_millis(15));
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
    }

    fn mouse(&mut self, code: u16, column: u16, row: u16) {
        self.send(format!("\x1b[<{code};{};{}M", column + 1, row + 1).as_bytes());
    }

    fn cell_at_text(&self, needle: &str) -> Option<&vt100::Cell> {
        let (rows, cols) = self.screen.screen().size();
        for (row, text) in self
            .screen
            .screen()
            .rows(0, cols)
            .enumerate()
            .take(rows as usize)
        {
            if let Some(column) = text.find(needle) {
                // Fixture prefixes and UI borders are single-column characters.
                let column = text[..column].chars().count();
                return self.screen.screen().cell(row as u16, column as u16);
            }
        }
        None
    }

    fn until_color(&mut self, needle: &str, color: vt100::Color) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.read();
            if self
                .cell_at_text(needle)
                .is_some_and(|cell| cell.fgcolor() == color)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Missing color {color:?} on {needle:?}: {}",
                self.screen.screen().contents()
            );
            thread::sleep(Duration::from_millis(15));
        }
    }

    fn until_clipboard_request(&mut self, text: &str) {
        let mut expected = String::new();
        crossterm::Command::write_ansi(
            &crossterm::clipboard::CopyToClipboard::to_clipboard_from(text),
            &mut expected,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.read();
            if self
                .raw_output
                .windows(expected.len())
                .any(|window| window == expected.as_bytes())
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Clipboard request was not emitted"
            );
            thread::sleep(Duration::from_millis(15));
        }
    }

    fn click_text(&mut self, label: &str) {
        self.read();
        let (_, cols) = self.screen.screen().size();
        let (column, row) = self
            .screen
            .screen()
            .rows(0, cols)
            .enumerate()
            .find_map(|(row, text)| {
                text.find(label)
                    .map(|column| (text[..column].chars().count() as u16, row as u16))
            })
            .unwrap_or_else(|| {
                panic!(
                    "Cannot click {label:?}: {}",
                    self.screen.screen().contents()
                )
            });
        self.mouse(0, column, row);
    }

    fn until_control(&mut self, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.read();
            let (_, cols) = self.screen.screen().size();
            if self
                .screen
                .screen()
                .rows(0, cols)
                .nth(1)
                .is_some_and(|row| row.contains(label))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Missing toolbar {label}: {}",
                self.screen.screen().contents()
            );
            thread::sleep(Duration::from_millis(15));
        }
    }

    fn click_control(&mut self, label: &str) {
        self.until_control(label);
        let (_, cols) = self.screen.screen().size();
        let row = self.screen.screen().rows(0, cols).nth(1).unwrap();
        let byte = row.find(label).unwrap();
        self.mouse(0, row[..byte].chars().count() as u16, 1);
    }

    fn menu_click(&mut self, label: &str) {
        self.read();
        let (_, cols) = self.screen.screen().size();
        let toolbar = self.screen.screen().rows(0, cols).nth(1).unwrap();
        self.click_control(if toolbar.contains("Menu") {
            "Menu"
        } else {
            "Actions"
        });
        self.until("Actions ·");
        for _ in 0..20 {
            self.read();
            let (_, cols) = self.screen.screen().size();
            let menu_top = self
                .screen
                .screen()
                .rows(0, cols)
                .position(|row| row.contains("Actions ·"))
                .unwrap();
            let hit = self
                .screen
                .screen()
                .rows(0, cols)
                .enumerate()
                .skip(menu_top + 1)
                .take_while(|(_, text)| !text.contains("↑↓ / wheel"))
                .find_map(|(row, text)| {
                    text.find(label)
                        .map(|column| (text[..column].chars().count() as u16, row as u16))
                });
            if let Some((column, row)) = hit {
                self.mouse(0, column, row);
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    self.read();
                    if !self.screen.screen().contents().contains("Actions ·") {
                        return;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "Menu action {label} did not close"
                    );
                    thread::sleep(Duration::from_millis(15));
                }
            }
            // Acknowledge each wheel response before issuing another one. This
            // models a user clicking the row they actually see, not queuing a
            // click against an older frame while more wheel events are pending.
            let selected = self
                .screen
                .screen()
                .rows(0, cols)
                .find(|text| text.contains('›'))
                .unwrap();
            self.mouse(65, 10, 5);
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                self.read();
                if self
                    .screen
                    .screen()
                    .rows(0, cols)
                    .find(|text| text.contains('›'))
                    .is_some_and(|text| text != selected)
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Menu wheel did not update selection"
                );
                thread::sleep(Duration::from_millis(15));
            }
        }
        panic!(
            "Menu action {label:?} not visible: {}",
            self.screen.screen().contents()
        );
    }

    fn hang_up(&mut self) {
        // All three handles duplicate the same master. Closing only one does
        // not simulate the user's terminal actually disappearing.
        self.reader = Box::new(std::io::empty());
        self.writer = Box::new(std::io::sink());
        let replacement = native_pty_system().openpty(PtySize::default()).unwrap();
        self.master = replacement.master;
        drop(replacement.slave);
    }

    fn wait_for_exit(&mut self, deadline: Instant) {
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Terminal hangup left the launcher alive"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn quit(&mut self) {
        self.send(&[0x11]);
        self.until("Quit Flimru?");
        assert!(self.child.try_wait().unwrap().is_none());
        self.send(b"y");
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            self.read();
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "Launcher did not quit promptly");
            thread::sleep(Duration::from_millis(15));
        }
        let fd = self.master.as_raw_fd().unwrap();
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: master owns fd and termios provides aligned writable storage.
        // The success assertion precedes assume_init, so uninitialized memory
        // is never read when tcgetattr fails.
        unsafe {
            assert_eq!(libc::tcgetattr(fd, termios.as_mut_ptr()), 0);
            assert_ne!(
                termios.assume_init().c_lflag & libc::ICANON,
                0,
                "Terminal canonical mode was not restored"
            );
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.child.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        // Even a failed assertion must give the launcher a chance to reap its
        // independently grouped Flutter children before killing the fixture.
        let _ = self.writer.write_all(&[0x11, b'y']);
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().is_ok_and(|status| status.is_none())
            && Instant::now() < deadline
        {
            self.read();
            thread::sleep(Duration::from_millis(10));
        }
        if self.child.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        self.hang_up();
        if let Some(pid) = self.child.process_id() {
            // SAFETY: kill takes no pointers; this is the process group created
            // by portable-pty for the fixture's child, not the test runner.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.wait();
    }
}

fn project() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".vscode")).unwrap();
    std::fs::write(directory.path().join(".vscode/launch.json"), r#"{
        // JSONC; argument tokens must reach the process unchanged.
        "configurations": [{"name":"Demo debug", "type":"dart", "request":"launch", "toolArgs":["--debug","--dart-define=GREETING=hello world"],},]
    }"#).unwrap();
    std::fs::write(
        directory.path().join("fake-flutter"),
        r#"#!/bin/bash
if [ "$1" = devices ]; then
  sleep 2
  printf '[{"id":"demo-device","name":"Demo phone","targetPlatform":"ios","isSupported":true}]\n'
  exit 0
fi
printf '%s\n' "$@" >> "$FAKE_LOG"
printf '%s' "$$" > "$FAKE_PID"
stty raw -echo
printf '\033[32mSESSION READY\033[0m\r\n'
while :; do
  IFS= read -rsn1 key || continue
  printf 'shortcut:%s\r\n' "$key"
  printf 'key:%s\n' "$key" >> "$FAKE_LOG"
  if [ "$key" = o ]; then
    for ((i=0;i<100;i++)); do printf 'history-line:%03d\r\n' "$i"; done
  fi
  if [ "$key" = c ]; then
    printf '\033[38;5;208mINDEXED COLOR\033[0m\r\n'
    printf '\033[38;2;12;200;99mTRUE COLOR\033[0m\r\n'
    printf 'flutter: [Orders] WARNING: retry\r\n'
    printf 'I/flutter: [Orders] SEVERE: failed\r\n'
    printf 'flutter: [Feed] INFO: connected\r\n'
    printf 'ENV:TERM=%s COLORTERM=%s FORCE_COLOR=%s NO_COLOR=%s\r\n' "$TERM" "$COLORTERM" "$FORCE_COLOR" "${NO_COLOR-unset}"
  fi
  [ "$key" = q ] && exit 0
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(
        directory.path().join("fake-flutter"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    directory
}

fn multi_project() -> TempDir {
    let project = project();
    std::fs::write(project.path().join("fake-flutter"), r#"#!/bin/bash
if [ "$1" = devices ]; then
  printf '[{"id":"alpha","name":"Alpha","targetPlatform":"ios","isSupported":true},{"id":"beta","name":"Beta","targetPlatform":"android","isSupported":true}]\n'
  exit 0
fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --device-id ]; then device="$2"; break; fi
  shift
done
printf '%s' "$$" > "$FAKE_PID.$device"
stty raw -echo
printf 'READY:%s\r\n' "$device"
while :; do
  IFS= read -rsn1 key || continue
  printf 'shortcut:%s:%s\r\n' "$device" "$key"
  printf 'key:%s:%s\n' "$device" "$key" >> "$FAKE_LOG"
  if [ "$key" = o ]; then
    for ((i=0;i<100;i++)); do printf '%s-history:%03d\r\n' "$device" "$i"; done
  fi
  [ "$key" = q ] && exit 0
  [ "$key" = x ] && exit 11
done
"#).unwrap();
    project
}

#[test]
fn explicit_batch_hot_actions_send_once_to_each_eligible_child_and_results_consume_input() {
    let project = multi_project();
    let mut tui =
        Terminal::start_with_options(project.path(), &project.path().join("state.json"), true);
    tui.until("Alpha");
    tui.send(b" \x1b[B ");
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("READY:alpha");
    tui.send(b"rR");
    tui.until("shortcut:alpha:R");
    let path = project.path().join("session.log");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "key:alpha:r\nkey:alpha:R\n"
    );
    tui.menu_click("Reload all eligible (2)");
    tui.until("Hot reload delivery report");
    tui.until("Requested/queued: 2");
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&path).unwrap().lines().count() != 4 {
        assert!(Instant::now() < deadline);
        tui.read();
        thread::sleep(Duration::from_millis(5));
    }
    tui.send(b"rR\x05\x1b[200~should-stay-in-modal\x1b[201~");
    tui.mouse(0, 0, 0);
    tui.send(b"\r");
    tui.until("shortcut:alpha:r");
    let lines = std::fs::read_to_string(&path).unwrap();
    assert_eq!(lines.lines().count(), 4);
    assert_eq!(
        lines.lines().filter(|line| *line == "key:beta:r").count(),
        1
    );
    assert_eq!(
        lines.lines().filter(|line| *line == "key:alpha:r").count(),
        2
    );
    tui.menu_click("Restart all eligible (2)");
    tui.until("Hot restart delivery report");
    tui.until("Requested/queued: 2");
    tui.send(b"\x1b");
    tui.until("shortcut:alpha:R");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log = std::fs::read_to_string(&path).unwrap();
        if log.lines().count() == 6 {
            assert_eq!(log.lines().filter(|line| *line == "key:beta:R").count(), 1);
            assert_eq!(log.lines().filter(|line| *line == "key:alpha:R").count(), 2);
            break;
        }
        assert!(Instant::now() < deadline);
        tui.read();
        thread::sleep(Duration::from_millis(5));
    }
    tui.quit();
}

#[test]
fn animation_reconnect_requests_fresh_connection_details_without_stopping_the_app() {
    let project = project();
    let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.until("Animations: waiting");
    tui.click_control("Reconnect");
    tui.until("shortcut:h");
    tui.until("VM Service reconnect requested");
    tui.send(b"r");
    tui.until("shortcut:r");
    tui.menu_click("Reconnect animation control");
    tui.send(b"v");
    tui.until("shortcut:v");
    let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert_eq!(log.matches("key:h\n").count(), 2);
    tui.quit();
}

#[test]
fn configuration_reload_shortcut_recovers_from_invalid_and_empty_launch_files() {
    let project = project();
    let launch_path = project.path().join(".vscode/launch.json");
    let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Reload");
    tui.until("r reload");

    std::fs::write(
        &launch_path,
        r#"{"configurations":[{"name":"Reloaded config","args":["--flavor","fresh"]}]}"#,
    )
    .unwrap();
    tui.send(b"r");
    tui.until("Reloaded config");
    tui.until("Launch configurations reloaded.");

    std::fs::write(&launch_path, "{invalid json").unwrap();
    tui.send(b"r");
    tui.until("Reload failed:");
    assert!(tui.screen.screen().contents().contains("Reloaded config"));

    std::fs::write(&launch_path, r#"{"configurations":[]}"#).unwrap();
    tui.send(b"r");
    tui.until("Empty configurations array.");
    assert!(!tui.screen.screen().contents().contains("Reloaded config"));

    // The existing alias and mouse command must also work after an empty reload.
    std::fs::write(
        &launch_path,
        r#"{"configurations":[{"name":"Alias config"}]}"#,
    )
    .unwrap();
    tui.send(b"c");
    tui.until("Alias config");
    std::fs::write(
        &launch_path,
        r#"{"configurations":[{"name":"Button config","args":["--flavor","fresh"]}]}"#,
    )
    .unwrap();
    tui.click_control("Reload");
    tui.until("Button config");
    // The former selected configuration was removed, so reload requires an
    // explicit selection rather than silently activating the replacement row.
    tui.send(b"\x1b[B");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert!(log.contains("--flavor\nfresh\n"), "{log}");
    tui.send(b"r");
    tui.until("shortcut:r");
    tui.quit();
}

#[test]
fn automatic_reload_watches_explicit_path_atomic_saves_and_recovers_last_good() {
    let project = project();
    let custom = project.path().join("custom-launch.json");
    let catalog = |name| format!(r#"{{"configurations":[{{"name":"{name}"}}]}}"#);
    std::fs::write(&custom, catalog("Custom initial")).unwrap();
    let mut tui = Terminal::start_configured(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
        Some(Path::new("custom-launch.json")),
    );
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until("Custom initial");
    tui.until_control("Run");
    // Writes to the conventional path are unrelated to the explicitly selected file.
    std::fs::write(
        project.path().join(".vscode/launch.json"),
        catalog("Wrong path"),
    )
    .unwrap();
    let replacement = project.path().join("atomic-save.json");
    std::fs::write(&replacement, catalog("Atomic replacement 界")).unwrap();
    std::fs::rename(&replacement, &custom).unwrap();
    tui.until("Atomic replacement 界");
    assert!(!tui.screen.screen().contents().contains("Wrong path"));
    tui.until("Select a configuration to run");
    tui.send(b"\r");
    tui.until("Select a configuration first.");
    assert!(!project.path().join("session.log").exists());
    std::fs::write(&custom, "{PRIVATE_SECRET_INVALID_JSON").unwrap();
    tui.until("Config error");
    assert!(
        tui.screen
            .screen()
            .contents()
            .contains("Atomic replacement 界")
    );
    tui.send(b"i");
    tui.until("last good catalog");
    assert!(!tui.screen.screen().contents().contains("PRIVATE_SECRET"));
    tui.send(b"\x1b");
    // Delete/recreate with identical accepted content still clears the error.
    std::fs::remove_file(&custom).unwrap();
    tui.send(b"c");
    tui.until("not found");
    std::fs::write(&custom, catalog("Atomic replacement 界")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        tui.read();
        if !tui.screen.screen().contents().contains("Config error") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "recovery left the persistent error marker"
        );
        thread::sleep(Duration::from_millis(15));
    }
    tui.send(b"\x1b[B");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    let pid_before = std::fs::read_to_string(project.path().join("child.pid")).unwrap();
    let log_before = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    std::fs::write(&custom, catalog("Future launches only")).unwrap();
    // A failed update is visible in the console while its existing child stays alive.
    std::fs::write(&custom, "{broken").unwrap();
    tui.until("Config error");
    tui.send(b"r");
    tui.until("shortcut:r");
    assert_eq!(
        std::fs::read_to_string(project.path().join("child.pid")).unwrap(),
        pid_before
    );
    let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert!(log.starts_with(&log_before));
    assert_eq!(log.matches("--device-id\n").count(), 1);
    std::fs::write(&custom, catalog("Future launches only")).unwrap();
    tui.send(b"\x04"); // Ctrl+d: the running session survives browsing the new catalog.
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until("Future launches only");
    assert_eq!(
        std::fs::read_to_string(project.path().join("child.pid")).unwrap(),
        pid_before
    );
    tui.quit();
}

#[test]
fn manual_reload_enter_burst_never_launches_old_arguments_and_menu_uses_same_path() {
    let project = project();
    let path = project.path().join(".vscode/launch.json");
    let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Run");
    std::fs::write(
        &path,
        r#"{"configurations":[{"name":"Demo debug","args":["--flavor","fresh-after-manual"]}]}"#,
    )
    .unwrap();
    tui.send(b"r\r");
    tui.until("Launch configurations reloaded.");
    tui.until_control("Run");
    assert!(
        !project.path().join("session.log").exists(),
        "r+Enter launched a child before an explicit post-reload Run"
    );
    // Menu reload also completes immediately without the automatic debounce.
    std::fs::write(
        &path,
        r#"{"configurations":[{"name":"Demo debug","args":["--flavor","menu-version"]}]}"#,
    )
    .unwrap();
    tui.menu_click("Reload configs");
    tui.until("Launch configurations reloaded.");
    tui.until_control("Run");
    assert!(!project.path().join("session.log").exists());
    tui.send(b"\r");
    tui.until("SESSION READY");
    let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert!(log.contains("--flavor\nmenu-version\n"), "{log}");
    assert!(
        !log.contains("GREETING=hello world"),
        "old launch arguments escaped the manual guard: {log}"
    );
    tui.quit();
}

#[test]
fn manual_reload_queued_enter_crossing_event_count_budget_requires_fresh_run() {
    assert_manual_reload_burst_is_fenced(false);
}

#[test]
fn manual_reload_queued_enter_crossing_time_budget_requires_fresh_run() {
    assert_manual_reload_burst_is_fenced(true);
}

fn assert_manual_reload_burst_is_fenced(time_budget: bool) {
    for toolbar in [false, true] {
        for fail in [false, true] {
            let project = project();
            let path = project.path().join(".vscode/launch.json");
            // Keep the existing safe fake process protocol, without waiting for
            // its artificial device-discovery delay in each regression case.
            let executable = project.path().join("fake-flutter");
            let fake = std::fs::read_to_string(&executable).unwrap();
            std::fs::write(&executable, fake.replace("  sleep 2\n", "")).unwrap();
            let catalog = |fresh| {
                let mut entries = vec![serde_json::json!({
                    "name":"Demo debug", "args":["--flavor", if fresh {"fresh-burst"} else {"last-good"}]
                })];
                if time_budget {
                    // Sixty-four visibility changes on a large catalog exercise
                    // the work-time budget, with far fewer than 128 events.
                    entries.extend(
                        (0..99_999).map(|i| serde_json::json!({"name":format!("Spare {i:05}")})),
                    );
                }
                serde_json::json!({"configurations":entries}).to_string()
            };
            std::fs::write(&path, catalog(false)).unwrap();
            let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
            tui.until("Demo phone");
            tui.send(b"\r");
            tui.until_control("Run");
            let mut burst = if toolbar {
                tui.until_control("Reload");
                let (_, cols) = tui.screen.screen().size();
                let row = tui.screen.screen().rows(0, cols).nth(1).unwrap();
                let byte = row.find("Reload").unwrap();
                let column = row[..byte].chars().count();
                format!("\x1b[<0;{};2M", column + 1).into_bytes()
            } else {
                b"r".to_vec()
            };
            if fail {
                std::fs::write(&path, "{broken").unwrap();
            } else {
                std::fs::write(&path, catalog(true)).unwrap();
            }
            if time_budget {
                burst.extend_from_slice(&[b'u'; 64]);
            } else {
                for _ in 0..256 {
                    burst.extend_from_slice(b"\x1bOP"); // Inert F1 events.
                }
            }
            burst.extend_from_slice(b"\r");
            // One PTY write leaves the old Enter queued behind the work. Never
            // send a new Run until the accepted catalog's ready frame appears.
            tui.send(&burst);
            tui.until(if fail {
                "Reload failed:"
            } else {
                "Launch configurations reloaded."
            });
            tui.until_control("Run");
            assert!(
                !project.path().join("session.log").exists(),
                "queued Enter launched: toolbar={toolbar}, fail={fail}, time_budget={time_budget}"
            );
            tui.send(b"\r");
            tui.until("SESSION READY");
            let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
            assert_eq!(
                log.lines().filter(|line| *line == "run").count(),
                1,
                "{log}"
            );
            assert!(
                log.contains(if fail { "last-good" } else { "fresh-burst" }),
                "{log}"
            );
            tui.quit();
        }
    }
}

#[test]
fn initial_check_and_reload_share_the_bounded_safe_loader() {
    let project = project();
    let path = project.path().join("custom-launch.json");
    let check = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_flimru"))
            .arg(project.path())
            .args(["--check", "--launch", "custom-launch.json", "--flutter"])
            .arg(project.path().join("fake-flutter"))
            .output()
            .unwrap()
    };
    std::fs::write(
        &path,
        vec![b' '; flimru::config_reload::MAX_LAUNCH_FILE_BYTES + 1],
    )
    .unwrap();
    let oversized = check();
    assert!(!oversized.status.success());
    assert!(String::from_utf8_lossy(&oversized.stderr).contains("4 MiB input limit"));
    std::fs::write(&path, "{PRIVATE_SECRET_FROM_INVALID_SOURCE").unwrap();
    let invalid = check();
    assert!(!invalid.status.success());
    let message = String::from_utf8_lossy(&invalid.stderr);
    assert!(message.contains("invalid JSONC"));
    assert!(!message.contains("PRIVATE_SECRET"));
    std::fs::write(&path, r#"{"configurations":[],"compounds":[{}]}"#).unwrap();
    let empty = check();
    assert!(empty.status.success());
    let output = String::from_utf8_lossy(&empty.stdout);
    assert!(output.contains("0 launch configurations"));
    assert!(output.contains("Compound launch configurations"));
    assert!(!project.path().join("session.log").exists());
}

#[test]
fn quiet_picker_and_console_stop_redrawing_and_wake_for_input() {
    let project = project();
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Demo phone");
    // Drain any frame already in flight, then observe more than the old
    // one-second unconditional redraw interval.
    let assert_quiet = |tui: &mut Terminal| {
        thread::sleep(Duration::from_millis(150));
        tui.read();
        tui.raw_output.clear();
        thread::sleep(Duration::from_millis(1200));
        tui.read();
        assert!(tui.raw_output.is_empty(), "Unchanged screen was redrawn");
        assert!(tui.child.try_wait().unwrap().is_none());
    };
    assert_quiet(&mut tui);
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    assert_quiet(&mut tui);
    tui.send(b"h");
    tui.until("shortcut:h");
    tui.quit();
}

#[test]
fn hidden_console_output_is_drained_without_redrawing_the_visible_console() {
    let project = multi_project();
    let script_path = project.path().join("fake-flutter");
    let script = std::fs::read_to_string(&script_path).unwrap().replace(
        "while :; do\n  IFS= read",
        "if [ \"$device\" = alpha ]; then\n\
         (while [ ! -f \"$FAKE_SCRIPT\" ]; do sleep 0.02; done; \
         printf 'BACKGROUND:alpha\\r\\n') &\nfi\nwhile :; do\n  IFS= read",
    );
    assert!(script.contains("BACKGROUND:alpha"));
    std::fs::write(script_path, script).unwrap();
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        true,
        3600,
    );
    tui.until("Alpha");
    tui.send(b" \x1b[B ");
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("READY:alpha");
    tui.send(b"\x06");
    tui.until("READY:beta");
    thread::sleep(Duration::from_millis(150));
    tui.read();
    tui.raw_output.clear();
    std::fs::write(project.path().join("probe.dart"), "emit background output").unwrap();
    thread::sleep(Duration::from_millis(500));
    tui.read();
    assert!(
        tui.raw_output.is_empty(),
        "Hidden output redrew the visible console"
    );
    tui.send(b"\x02");
    tui.until("BACKGROUND:alpha");
    tui.quit();
    assert_multi_children_reaped(project.path());
}

fn assert_multi_children_reaped(project: &Path) {
    for device in ["alpha", "beta"] {
        let pid = std::fs::read_to_string(project.join(format!("child.pid.{device}")))
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert_eq!(
            // SAFETY: signal zero checks only the fixture's recorded child id.
            unsafe { libc::kill(pid, 0) },
            -1,
            "{device} child still exists"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}

#[test]
fn sessions_can_be_added_and_replaced_without_interrupting_existing_consoles() {
    for hang_up in [false, true] {
        let project = multi_project();
        let script = std::fs::read_to_string(project.path().join("fake-flutter")).unwrap()
            .replace(r#"{"id":"beta","name":"Beta","targetPlatform":"android","isSupported":true}"#, r#"{"id":"beta","name":"Beta","targetPlatform":"android","isSupported":true},{"id":"gamma","name":"Gamma","targetPlatform":"ios","isSupported":true}"#)
            .replace("printf '%s' \"$$\" >", "device=\"${SESSION_NAME:-$device}\"\nprintf '%s' \"$$\" >");
        std::fs::write(project.path().join("fake-flutter"), script).unwrap();
        std::fs::write(
            project.path().join(".vscode/launch.json"),
            r#"{"configurations":[
            {"name":"Demo debug"},
            {"name":"Alternate debug","env":{"SESSION_NAME":"alternate"}},
            {"name":"Blocked config","preLaunchTask":"unsupported"}
        ]}"#,
        )
        .unwrap();
        let state = project.path().join("state.json");
        let mut tui = Terminal::start(project.path(), &state);
        tui.until("Alpha");
        if !hang_up {
            tui.master
                .resize(PtySize {
                    rows: 12,
                    cols: 34,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .unwrap();
            tui.screen.screen_mut().set_size(12, 34);
            tui.until_control("Menu");
        }
        let pid_for = |name| {
            std::fs::read_to_string(project.path().join(format!("child.pid.{name}")))
                .unwrap()
                .parse::<i32>()
                .unwrap()
        };
        let alive = |pid| {
            // SAFETY: signal zero only checks a recorded fixture child id.
            assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        };
        tui.send(b"\r");
        tui.until("Configs · 2 ready / 1 blocked");
        tui.send(b"\r");
        tui.until("READY:alpha");
        let alpha = pid_for("alpha");
        tui.send(b"h");
        tui.until("shortcut:alpha:h");
        tui.menu_click("Add session");
        tui.until("Add session");
        tui.send(b"\x1b[B\r"); // Beta, original configuration.
        tui.until("Configs · 2 ready / 1 blocked");
        tui.send(b"\r");
        tui.until("READY:beta");
        let old_beta = pid_for("beta");
        alive(alpha);
        tui.send(b"\x0e"); // Ctrl+n, then Gamma with a different configuration.
        tui.until("Add session");
        tui.send(b"\x1b[B\r");
        tui.until("Configs · 2 ready / 1 blocked");
        tui.send(b"\x1b[B\r");
        tui.until("READY:alternate");
        let alternate = pid_for("alternate");
        alive(alpha);
        alive(old_beta);
        tui.send(b"\x02q"); // Stop only Beta.
        tui.until("2 session(s) running");
        assert_eq!(
            // SAFETY: signal zero checks the old Beta child, without signaling it.
            unsafe { libc::kill(old_beta, 0) },
            -1
        );
        tui.send(b"\x0e");
        tui.until("Add session");
        tui.send(b"\x1b[H\x1b[B\r"); // Beta again, choose original config instead of the saved Alternate.
        tui.until("Configs · 2 ready / 1 blocked");
        tui.send(b"\x1b[H\r");
        tui.until("READY:beta");
        let beta = pid_for("beta");
        assert_ne!(old_beta, beta);
        alive(alpha);
        alive(alternate);
        tui.send(b"\x02\x02\x02"); // Back to original Alpha, with its original history.
        tui.until("shortcut:alpha:h");
        tui.send(b"r");
        tui.until("shortcut:alpha:r");
        tui.send(b"\x0e");
        tui.until("Add session");
        tui.send(b"\x1b[H\r"); // Alpha with Demo debug would be a duplicate.
        tui.until("Configs · 2 ready / 1 blocked");
        tui.send(b"\x1b[H\r");
        tui.until("Already running");
        assert_eq!(pid_for("alpha"), alpha);
        tui.send(b"u\x1b[F\r"); // Show and inspect the blocked preLaunchTask entry.
        tui.until("Diagnostics");
        tui.send(b"\x1b");
        tui.until("Configs · 2 ready / 1 blocked");
        alive(alpha);
        alive(beta);
        alive(alternate);
        if hang_up {
            tui.hang_up();
            tui.wait_for_exit(Instant::now() + Duration::from_secs(2));
        } else {
            tui.quit();
        }
        for pid in [alpha, beta, alternate] {
            assert_eq!(
                // SAFETY: signal zero only checks these owned fixture child ids.
                unsafe { libc::kill(pid, 0) },
                -1,
                "Added session was not reaped"
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}

#[test]
fn top_actions_support_keyboard_focus_without_leaking_navigation_to_flutter() {
    for compact in [true, false] {
        let project = project();
        let state = project.path().join("state.json");
        std::fs::write(
            &state,
            r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
        )
        .unwrap();
        let mut tui = Terminal::start(project.path(), &state);
        tui.until("Saved phone");
        if compact {
            tui.master
                .resize(PtySize {
                    rows: 12,
                    cols: 34,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .unwrap();
            tui.screen.screen_mut().set_size(12, 34);
            tui.until_control("Menu");
        }
        tui.send(b"\t\x1b[C"); // Tab enters the bar; Right focuses Multi.
        tui.until_control("›Multi");
        tui.send(b"\r");
        tui.until("Multi 0/8");
        tui.send(b" "); // Space activates the same focused button.
        tui.until("Devices (");
        tui.send(b"\x1b[B\r"); // Down returns to the list; Enter advances.
        tui.until_control("Run");
        tui.send(b"\t\r"); // Primary Run is the first focused action.
        tui.until("SESSION READY");
        tui.send(b"\t"); // Live-console Tab still belongs to Flutter.
        let log_path = project.path().join("session.log");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !std::fs::read_to_string(&log_path)
            .unwrap()
            .contains("key:\t\n")
        {
            tui.read();
            assert!(
                Instant::now() < deadline,
                "Tab was not forwarded: {}",
                tui.screen.screen().contents()
            );
            thread::sleep(Duration::from_millis(15));
        }
        tui.send(b"\x07"); // Ctrl+g focuses the bar in a live console.
        tui.until_control("›Stop");
        // Add, Speed, and the recovery button precede the Actions menu.
        tui.send(b"R\x1b[200~r\x1b[201~\x1b[C\x1b[C\x1b[C\x1b[C");
        tui.until_control(if compact { "›Menu" } else { "›Actions" });
        tui.send(b"\x10"); // Ctrl+p opens the menu without losing the trigger focus.
        tui.until("Actions ·");
        tui.send(b"\x1b");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            tui.read();
            if !tui.screen.screen().contents().contains("Actions ·") {
                break;
            }
            assert!(Instant::now() < deadline, "Menu did not close");
            thread::sleep(Duration::from_millis(15));
        }
        tui.until_control(if compact { "›Menu" } else { "›Actions" });
        if compact {
            tui.send(b"\x1b");
        } else {
            tui.mouse(0, 20, 8); // Clicking console content restores input focus too.
        }
        tui.until("Ctrl+g actions");
        tui.send(b"h");
        tui.until("shortcut:h");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(!log.contains("key:R\n") && !log.contains("key:r\n"));
        assert!(!log.contains("key:[\n") && !log.contains("key:\u{1b}\n"));
        tui.send(b"\x07\r"); // Stop via a keyboard-focused toolbar action.
        tui.until("ended");
        tui.until_control("Next");
        tui.quit();
    }
}

#[test]
fn multi_device_mouse_keyboard_consoles_are_isolated_and_completed_history_is_retained() {
    let project = multi_project();
    let state = project.path().join("state.json");
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Alpha");
    tui.master
        .resize(PtySize {
            rows: 12,
            cols: 34,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(12, 34);
    tui.until_control("Menu");
    tui.menu_click("Multi-device: off");
    tui.until("Multi 0/8");
    tui.send(b" ");
    tui.until("[x]");
    tui.send(b"\x1b[B");
    tui.until("[ ] Beta");
    tui.click_text("[ ]");
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.click_control("Run");
    tui.until("READY:alpha");
    tui.send(b"h");
    tui.until("shortcut:alpha:h");
    tui.click_text("2● Beta"); // Select the second session tab.
    tui.until("READY:beta");
    assert!(!tui.screen.screen().contents().contains("shortcut:alpha:h"));
    tui.send(b"v");
    tui.until("shortcut:beta:v");
    tui.send(b"\x02"); // Ctrl+b switches back.
    tui.until("shortcut:alpha:h");
    assert!(!tui.screen.screen().contents().contains("shortcut:beta:v"));
    tui.send(b"o");
    tui.until("alpha-history:099");
    tui.send(b"\x1b[H");
    tui.until("READY:alpha");
    tui.click_text("2● Beta"); // Select the second session tab.
    tui.until("shortcut:beta:v");
    tui.click_text("1● Alpha"); // Select the first session tab.
    tui.until("READY:alpha"); // Per-console scroll position survives switching.
    tui.send(b"q");
    tui.until("1 session(s) running");
    tui.click_text("2● Beta"); // Select the second session tab.
    tui.send(b"r");
    tui.until("shortcut:beta:r");
    tui.send(b"x");
    tui.until("0 session(s) running");
    tui.send(b"\x02");
    tui.until("ended");
    tui.send(b"\x1b[H");
    tui.until("READY:alpha");
    tui.send(b"\x04"); // Ctrl+d returns to selection, without losing the batch.
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("READY:alpha");
    tui.send(b"\x18"); // Ctrl+x stops every session.
    tui.until("0 session(s) running");
    tui.quit();
    assert_multi_children_reaped(project.path());
    let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert!(
        log.contains("key:alpha:h") && log.contains("key:beta:v") && log.contains("key:beta:r")
    );
    assert!(!log.contains("key:beta:h") && !log.contains("key:alpha:v"));
}

#[test]
fn ended_session_tabs_close_with_mouse_and_control_w_without_stopping_other_runs() {
    for compact in [true, false] {
        let project = multi_project();
        let state = project.path().join("state.json");
        let mut tui = Terminal::start_with_options(project.path(), &state, true);
        tui.until("Alpha");
        if compact {
            tui.master
                .resize(PtySize {
                    rows: 12,
                    cols: 34,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .unwrap();
            tui.screen.screen_mut().set_size(12, 34);
            tui.until_control("Menu");
        }
        tui.send(b" \x1b[B \r");
        tui.until_control("Run");
        tui.send(b"\r");
        tui.until("READY:alpha");
        tui.until("2● Beta");
        tui.send(b"\x17"); // Ctrl+w must not close or send input to a running session.
        tui.until("Session is running.");
        tui.send(b"h");
        tui.until("shortcut:alpha:h");
        tui.send(b"q");
        tui.until("1○ Alpha");
        tui.click_text("2● Beta");
        tui.until("READY:beta");
        tui.send(b"h");
        tui.until("shortcut:beta:h");
        tui.click_text("×"); // Close the inactive Alpha tab and preserve active Beta.
        tui.until("Closed ended session");
        tui.until("1● Beta");
        assert!(tui.screen.screen().contents().contains("shortcut:beta:h"));
        tui.send(b"\x17");
        tui.until("Session is running.");
        tui.send(b"r");
        tui.until("shortcut:beta:r");
        tui.send(b"q");
        tui.until("1○ Beta");
        tui.send(b"\x17"); // The last ended tab closes and returns to the picker.
        tui.until("Closed ended session");
        tui.until_control("Next");
        assert!(!tui.screen.screen().contents().contains("1○"));
        tui.quit();
        assert_multi_children_reaped(project.path());
        let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
        assert!(!log.contains('\u{17}'));
    }
}

#[test]
fn multi_device_quit_and_terminal_loss_reap_every_instance() {
    for hang_up in [false, true] {
        let project = multi_project();
        let state = project.path().join("state.json");
        let mut tui = Terminal::start_with_options(project.path(), &state, true);
        tui.until("Alpha");
        tui.send(b" \x1b[B ");
        tui.until("Multi 2/8");
        tui.send(b"\r");
        tui.until("Configs · 1 ready / 0 blocked");
        tui.send(b"\r");
        tui.until("READY:alpha");
        tui.send(b"\x06");
        tui.until("READY:beta");
        tui.send(b"\x04");
        tui.until("Multi 2/8"); // Hidden sessions still count in quit confirmation.
        if hang_up {
            tui.hang_up();
            tui.wait_for_exit(Instant::now() + Duration::from_secs(2));
        } else {
            tui.send(&[0x11]);
            tui.until("All active Flutter sessions");
            tui.send(b"\r"); // Cancel preserves both children.
            tui.send(b"\x06h");
            tui.until("shortcut:beta:h");
            tui.quit();
        }
        assert_multi_children_reaped(project.path());
    }
}

#[test]
fn colors_survive_pty_scrollback_and_saved_logs_despite_inherited_no_color() {
    let project = project();
    let state = project.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
    )
    .unwrap();
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Saved phone");
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until_color("SESSION READY", vt100::Color::Idx(2));
    tui.send(b"c");
    tui.until("NO_COLOR=unset");
    tui.until_color("INDEXED COLOR", vt100::Color::Idx(208));
    tui.until_color("TRUE COLOR", vt100::Color::Rgb(12, 200, 99));
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.until_color("SEVERE:", vt100::Color::Rgb(255, 128, 142));
    tui.until_color("INFO:", vt100::Color::Rgb(96, 210, 225));
    tui.send(b"\x0ch"); // Former Ctrl+l toggle now belongs to Flutter.
    tui.until("shortcut:h");
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.send(b"o");
    tui.until("history-line:099");
    tui.menu_click("Oldest output");
    tui.until_color("TRUE COLOR", vt100::Color::Rgb(12, 200, 99));
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.menu_click("Follow output");
    tui.until("history-line:099");
    tui.send(b"q");
    tui.until("ended (");
    tui.click_text("1○ Demo debug");
    tui.until("Last console ·");
    tui.click_text("1○ Demo debug"); // Clicking the selected saved tab keeps it open.
    tui.until("Last console ·");
    tui.menu_click("Oldest output");
    tui.until_color("TRUE COLOR", vt100::Color::Rgb(12, 200, 99));
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.send(b"\x17");
    tui.until("Closed saved console.");
    assert!(!tui.screen.screen().contents().contains("1○"));
    tui.quit();
}

#[test]
fn compact_actions_support_pointer_commands_and_keyboard_navigation() {
    let project = project();
    let state = project.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
    )
    .unwrap();
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Saved phone");
    tui.master
        .resize(PtySize {
            rows: 12,
            cols: 34,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(12, 34);
    tui.until_control("Menu");
    tui.menu_click("Filter list");
    tui.until("[Apply]");
    tui.send(b"\x1b[200~Saved\x1b[201~");
    tui.until("/ Saved");
    tui.click_text("[Apply]");
    tui.click_text("[Clear]");
    tui.until("[Filter /]");
    tui.menu_click("Reload configs");
    tui.until("reloaded.");
    tui.click_control("Next");
    tui.until_control("Run");
    tui.click_control("Run");
    tui.until("SESSION READY");
    for (label, key) in [
        ("Hot reload", 'r'),
        ("Hot restart", 'R'),
        ("Flutter help", 'h'),
        ("Open DevTools", 'v'),
    ] {
        tui.menu_click(label);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !std::fs::read_to_string(project.path().join("session.log"))
            .unwrap()
            .contains(&format!("key:{key}\n"))
        {
            assert!(
                Instant::now() < deadline,
                "Mouse command {label} was not sent"
            );
            thread::sleep(Duration::from_millis(15));
        }
    }
    tui.menu_click("Stop session");
    tui.until("ended");
    tui.send(b"\x10"); // Keyboard Ctrl+p, End, Enter selects Quit.
    tui.until("Actions ·");
    tui.send(b"\x1b[F");
    tui.until("Quit launcher");
    tui.send(b"\r");
    tui.until("Quit Flimru?");
    tui.send(b"\r"); // Safe default remains Cancel.
    tui.until_control("Menu");
    tui.quit();
}

#[test]
fn cached_favorite_runs_before_discovery_then_returns_and_runs_again() {
    let project = project();
    let state = project.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
    )
    .unwrap();
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Saved phone");
    tui.until("saved · try now");
    // Discovery takes two seconds; this favorite is launchable immediately.
    let start = Instant::now();
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    assert!(start.elapsed() < Duration::from_secs(2));
    tui.send(b"h");
    tui.until("shortcut:h");
    tui.send(b"v");
    tui.until("shortcut:v");
    tui.send(b"q");
    tui.until("ended (");
    tui.until("Devices (");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(saved["last_device"], "demo-device");
    assert_eq!(saved["last_config"], "Demo debug");
    let args = std::fs::read_to_string(project.path().join("session.log")).unwrap();
    assert!(args.contains("--dart-define=GREETING=hello world\n"));
    assert!(args.contains("--device-id\ndemo-device\n"));
    tui.send(b"l");
    tui.until("Last console");
    tui.until("shortcut:v");
    tui.send(b"\x1b");
    tui.until("Devices (");
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.send(b"\x1b");
    tui.until("ended (");
    tui.quit();
}

#[test]
fn discovery_favorites_filter_and_resize_work_in_real_tui() {
    let project = project();
    let state = project.path().join("state.json");
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("No devices yet.");
    tui.until("Demo phone");
    tui.send(b"f");
    tui.until("added to favorites.");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(saved["favorites"][0]["id"], "demo-device");
    tui.send(b"/phone\r");
    tui.until("Esc clears filter");
    tui.send(b"\r");
    tui.until("Configs · 1 ready / 0 blocked");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.master
        .resize(PtySize {
            rows: 24,
            cols: 74,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(24, 74);
    tui.send(b"h");
    tui.until("shortcut:h");
    tui.send(b"q");
    tui.until("Devices (");
    tui.quit();
}

#[test]
fn mouse_controls_scrollbar_and_quit_confirmation_in_real_tui() {
    let project = project();
    let state = project.path().join("state.json");
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Demo phone");
    // The first star lives at column 4, row 4 (zero-based).
    tui.mouse(0, 4, 4);
    tui.until("added to favorites.");
    // Next then Run toolbar buttons; no keyboard needed to start.
    tui.click_control("Next");
    tui.until("Configs · 1 ready / 0 blocked");
    tui.click_control("Run");
    tui.until("SESSION READY");
    tui.send(b"o");
    tui.until("history-line:099");
    tui.mouse(64, 20, 12); // Wheel up
    tui.until("scrollback");
    // Click the top of the right scrollbar, then drag to its bottom.
    tui.mouse(0, 110, 4);
    tui.until("history-line:000");
    tui.mouse(32, 110, 26);
    tui.until("history-line:099");
    tui.send(b"\x1b[<0;111;27m"); // Release drag
    tui.send(&[0x11]);
    tui.until("Quit Flimru?");
    tui.until("active Flutter session");
    assert!(tui.child.try_wait().unwrap().is_none());
    tui.send(b"\r"); // Enter cancels by default
    tui.send(b"h");
    tui.until("shortcut:h");
    tui.quit();
}

#[test]
fn closing_terminal_exits_picker_and_active_run_without_orphans() {
    for active in [false, true] {
        let project = project();
        let state = project.path().join("state.json");
        std::fs::write(
            &state,
            r#"{"favorites":[{"id":"probe","name":"Probe","platform":"custom"}]}"#,
        )
        .unwrap();
        let mut tui = Terminal::start(project.path(), &state);
        tui.until("Probe");
        let child_pid = if active {
            tui.send(b"\r");
            tui.until("Configs · 1 ready / 0 blocked");
            tui.send(b"\r");
            tui.until("SESSION READY");
            Some(
                std::fs::read_to_string(project.path().join("child.pid"))
                    .unwrap()
                    .parse::<i32>()
                    .unwrap(),
            )
        } else {
            None
        };
        tui.hang_up();
        tui.wait_for_exit(Instant::now() + Duration::from_secs(2));
        if let Some(pid) = child_pid {
            assert_eq!(
                // SAFETY: signal 0 only checks this recorded child id; kill
                // accesses no Rust memory and sends no signal to the process.
                unsafe { libc::kill(pid, 0) },
                -1,
                "Session child was not reaped after terminal closed"
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}

#[test]
#[ignore = "Set FLUTTER_TUI_TEST_DART to a local Dart executable to validate the real VM Service"]
fn real_dart_vm_service_animation_shortcut_and_indicator() {
    assert!(
        std::path::Path::new(
            &std::env::var_os("FLUTTER_TUI_TEST_DART").expect("Set FLUTTER_TUI_TEST_DART")
        )
        .is_file()
    );
    let project = project();
    std::fs::write(
        project.path().join("probe.dart"),
        r#"import 'dart:async';
import 'dart:developer';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
Future<void> app(List<Object> arguments) async {
  final ready = arguments[0] as SendPort;
  final generation = arguments[1] as int;
  await Future<void>.delayed(const Duration(milliseconds: 350));
  var factor = 1.0;
  registerExtension('ext.flutter.timeDilation', (method, parameters) async {
    if (parameters.containsKey('timeDilation')) {
      factor = double.parse(parameters['timeDilation']!);
      print('PROBE SPEED:$generation:$factor');
    }
    return ServiceExtensionResponse.result(jsonEncode({'timeDilation': factor.toString()}));
  });
  ReceivePort().listen((_) {});
  ready.send(true);
}
Future<void> main() async {
  Isolate? worker;
  var generation = 0;
  Future<void> start() async {
    final ready = ReceivePort();
    worker = await Isolate.spawn(app, <Object>[ready.sendPort, ++generation]);
    await ready.first;
    ready.close();
    print('PROBE READY $generation');
  }
  await start();
  stdin.lineMode = false;
  stdin.echoMode = false;
  stdin.listen((data) async {
    for (final key in data) {
      if (key == 113) exit(0);
      if (key == 82) {
        print('Performing hot restart...');
        worker!.kill(priority: Isolate.immediate);
        await start();
        print('Restarted application in 350ms.');
      }
    }
  });
}
"#,
    )
    .unwrap();
    std::fs::write(
        project.path().join("fake-flutter"),
        r#"#!/bin/sh
if [ "$1" = devices ]; then printf '[]\n'; exit 0; fi
exec "$FAKE_DART" --disable-dart-dev --enable-vm-service=0 "$FAKE_SCRIPT"
"#,
    )
    .unwrap();
    let state = project.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"favorites":[{"id":"probe","name":"Probe","platform":"custom"}]}"#,
    )
    .unwrap();
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Probe");
    tui.send(b"\r");
    tui.until("Configs · 1 ready / 0 blocked");
    tui.send(b"\r");
    tui.until("PROBE READY");
    tui.until("Animations: normal");
    tui.send(b"\x01"); // Ctrl+a controls this console's animation speed.
    tui.until("PROBE SPEED:1:5.0");
    tui.until("Animations: 5× slower");
    tui.send(b"R");
    tui.until("PROBE READY 2");
    tui.until("PROBE SPEED:2:5.0");
    tui.until("Animations: 5× slower");
    tui.send(b"\x01"); // Ctrl+a controls this console's animation speed.
    tui.until("PROBE SPEED:2:1.0");
    tui.until("Animations: normal");
    tui.send(b"R");
    tui.until("PROBE READY 3");
    tui.until("Animations: normal");
    tui.send(b"q");
    tui.until("ended (");
    tui.quit();
}

#[test]
fn mouse_drag_copies_live_and_saved_console_text_without_forwarding_events() {
    let project = project();
    let state = project.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
    )
    .unwrap();
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Saved phone");
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    for saved in [false, true] {
        tui.menu_click("Oldest output");
        tui.until("SESSION READY");
        let (row, column) = tui
            .screen
            .screen()
            .rows(0, 112)
            .enumerate()
            .find_map(|(row, line)| {
                line.find("SESSION READY")
                    .map(|index| (row as u16, line[..index].chars().count() as u16))
            })
            .unwrap();
        tui.raw_output.clear();
        tui.mouse(0, column, row);
        tui.mouse(32, column + 6, row);
        tui.send(format!("\x1b[<0;{};{}m", column + 13, row + 1).as_bytes());
        tui.until("Copy requested.");
        tui.until_clipboard_request("SESSION READY");
        let cell = tui.screen.screen().cell(row, column).unwrap();
        assert_eq!(cell.bgcolor(), vt100::Color::Rgb(96, 210, 225));
        tui.send(b"\x1b"); // Clear selection; do not stop the live session.
        tui.until_color("SESSION READY", vt100::Color::Idx(2));
        if !saved {
            tui.send(b"\x05"); // Ctrl+e enters keyboard selection.
            tui.until("Shift+arrows select");
            tui.raw_output.clear();
            tui.send(b"\x1b[C\x1b[C\x1b[C\x1b[1;2C\x1b[1;2C\r");
            tui.until_clipboard_request("SIO");
            tui.send(b"\x1b");
            // The first S is outside the keyboard range (columns 3..5), so
            // its color cannot acknowledge clearing that selection. Wait for
            // the footer transition before sending h; ESC+h in one unread
            // terminal packet is an Alt+h key rather than a separate Escape.
            tui.until("r reload · R restart · Esc stop");
            tui.until_color("SESSION READY", vt100::Color::Idx(2));
            tui.send(b"h");
            tui.until("shortcut:h");
            let log = std::fs::read_to_string(project.path().join("session.log")).unwrap();
            assert!(!log.contains("key:\x1b"));
            tui.send(b"q");
            tui.until("ended (");
            tui.click_text("1○ Demo debug");
            tui.until("Last console ·");
        }
    }
    tui.send(b"\x17");
    tui.until("Closed saved console.");
    tui.quit();
}

#[test]
fn dragging_at_console_edges_scrolls_and_copies_across_viewports() {
    let project = project();
    let state = project.path().join("state.json");
    let mut tui = Terminal::start(project.path(), &state);
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until("Demo debug");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.send(b"o");
    tui.until("history-line:099");
    for saved in [false, true] {
        tui.menu_click("Oldest output");
        tui.until("history-line:000");
        let (row, column) = tui
            .screen
            .screen()
            .rows(0, 112)
            .enumerate()
            .find_map(|(row, line)| {
                line.find("history-line:000")
                    .map(|index| (row as u16, line[..index].chars().count() as u16))
            })
            .unwrap();
        tui.mouse(0, column, row);
        // A single edge event must keep scrolling after the pointer stops moving.
        tui.mouse(32, column + 15, 26);
        tui.until("history-line:030");
        // Wheel input extends the same selection instead of clearing it.
        for _ in 0..40 {
            tui.mouse(65, column + 15, 26);
        }
        tui.until("history-line:099");
        tui.raw_output.clear();
        tui.send(format!("\x1b[<0;{};27m", column + 16).as_bytes());
        tui.until("Copy requested.");
        let mut expected = (0..100)
            .map(|row| format!("history-line:{row:03}\n"))
            .collect::<String>();
        if saved {
            expected.push_str("shortcut:q\n");
        }
        tui.until_clipboard_request(&expected);
        tui.send(b"\x1b");
        if !saved {
            tui.until("r reload · R restart · Esc stop");
            tui.send(b"q");
            tui.until("ended (");
            tui.click_text("1○ Demo debug");
            tui.until("Last console ·");
        }
    }
    tui.quit();
}

#[test]
fn rejected_only_and_empty_catalogs_open_diagnostics_without_starting_children() {
    for (raw, expected) in [
        (
            r#"{"configurations":[{"name":"Rejected startup","type":"PRIVATE_TYPE"}]}"#,
            "Rejected startup",
        ),
        (r#"{"configurations":[]}"#, "Empty configurations array."),
    ] {
        let project = project();
        std::fs::write(project.path().join(".vscode/launch.json"), raw).unwrap();
        std::fs::write(project.path().join("fake-flutter"), "#!/bin/bash\nif [ \"$1\" = devices ]; then printf '[]\\n'; else printf 'UNEXPECTED RUN\\n' >> \"$FAKE_LOG\"; fi\n").unwrap();
        let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
        tui.until(expected);
        tui.send(b"\r");
        if !raw.contains("[]") {
            tui.until("Diagnostics");
            assert!(!tui.screen.screen().contents().contains("PRIVATE_TYPE"));
            tui.send(b"\x1b");
            tui.until("Why blocked?");
            tui.click_control("Why blocked?");
            tui.until("Diagnostics");
            tui.send(b"\x1b");
            tui.until("Why blocked?");
        }
        assert!(!project.path().join("session.log").exists());
        tui.quit();
    }
}

#[test]
fn device_picker_opens_config_diagnostics_with_no_targets_at_minimum_size() {
    let project = project();
    std::fs::write(project.path().join("fake-flutter"), "#!/bin/bash\nif [ \"$1\" = devices ]; then printf '[]\\n'; else printf 'UNEXPECTED RUN\\n' >> \"$FAKE_LOG\"; fi\n").unwrap();
    let mut tui = Terminal::start(project.path(), &project.path().join("state.json"));
    tui.until("No devices yet.");
    tui.master
        .resize(PtySize {
            rows: 12,
            cols: 34,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(12, 34);
    tui.send(b"d");
    tui.until("Demo debug");
    tui.send(b"i");
    tui.until("Diagnostics");
    tui.until("Select a target");
    tui.send(b"\x1b[200~DO_NOT_SEND\x1b[201~");
    tui.send(b"\r"); // dismisses the explanation; cannot launch.
    tui.until("Demo debug");
    assert!(!project.path().join("session.log").exists());
    tui.quit();
}

#[test]
fn displayed_blocked_enter_after_external_repair_explains_before_any_process_starts() {
    let project = project();
    let cwd = project.path().join("created-later");
    std::fs::write(
        project.path().join(".vscode/launch.json"),
        serde_json::json!({"configurations":[
            {"name":"External recovery","cwd":cwd}, {"name":"Ready sibling"}
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        project.path().join("fake-flutter"),
        r#"#!/bin/bash
if [ "$1" = devices ]; then
  printf '[{"id":"demo-device","name":"Demo phone","targetPlatform":"ios","isSupported":true}]\n'
  exit 0
fi
printf 'spawn\n' >> "$FAKE_LOG"
stty raw -echo
printf 'SESSION READY\r\n'
while :; do sleep 1; done
"#,
    )
    .unwrap();
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until("Ready sibling");
    tui.until_control("Run");
    tui.send(b"u");
    tui.until("blocked · cwd");
    assert!(!tui.screen.screen().contents().contains("[Run]"));
    std::fs::create_dir(&cwd).unwrap();
    tui.send(b"\r");
    tui.until("Diagnostics");
    tui.until("Ready to run");
    assert!(!project.path().join("session.log").exists());
    tui.send(b"\r"); // explicit modal dismissal cannot launch
    tui.until("External recovery");
    assert!(!project.path().join("session.log").exists());
    tui.send(b"\r"); // a later explicit Run uses fresh all-target preflight
    tui.until("SESSION READY");
    assert_eq!(
        std::fs::read_to_string(project.path().join("session.log")).unwrap(),
        "spawn\n"
    );
    tui.quit();
}

#[test]
fn diagnostics_reverse_immediately_after_keyboard_or_wheel_overscroll_and_resize() {
    let project = project();
    std::fs::write(project.path().join(".vscode/launch.json"), r#"{"configurations":[{"name":"Very long unsupported configuration 界 é","type":"unsupported"}],"compounds":[{}]}"#).unwrap();
    std::fs::write(
        project.path().join("fake-flutter"),
        "#!/bin/bash\nprintf '[]\\n'\n",
    )
    .unwrap();
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Very long unsupported");
    tui.master
        .resize(PtySize {
            rows: 12,
            cols: 34,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(12, 34);
    tui.send(b"i");
    tui.until("Diagnostics");
    let settle = |tui: &mut Terminal| {
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            tui.read();
            thread::sleep(Duration::from_millis(5));
        }
    };
    for wheel in [false, true] {
        for _ in 0..50 {
            if wheel {
                tui.mouse(65, 2, 2);
            } else {
                tui.send(b"\x1b[B");
            }
        }
        settle(&mut tui);
        let bottom = tui.screen.screen().contents();
        if wheel {
            tui.mouse(64, 2, 2);
        } else {
            tui.send(b"\x1b[A");
        }
        settle(&mut tui);
        assert_ne!(
            tui.screen.screen().contents(),
            bottom,
            "one reverse event must immediately move the overlay"
        );
    }
    tui.master
        .resize(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tui.screen.screen_mut().set_size(40, 120);
    tui.until("Very long unsupported configuration");
    settle(&mut tui);
    let fitting = tui.screen.screen().contents();
    tui.send(b"\x1b[B");
    settle(&mut tui);
    assert_eq!(
        tui.screen.screen().contents(),
        fitting,
        "fitting content cannot overscroll"
    );
    tui.send(b"\x1b");
    tui.until("Why blocked?");
    tui.quit();
}

#[test]
fn console_search_control_shortcut_is_delivered_and_review_isolates_input_from_explicit_process_actions()
 {
    let project = project();
    let log_path = project.path().join("session.log");
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.send(&[0x13]); // Ctrl+s must survive real terminal flow control/raw-mode delivery.
    tui.until("Snapshot");
    tui.until("Search");
    tui.send(b"SESSION");
    tui.until("1/1");
    tui.send(b"rR\x01\x05\x18\x1a");
    tui.send(b"\x1b[200~q\x1b[201~");
    tui.until("No matches");
    tui.mouse(0, 5, 8);
    tui.mouse(32, 30, 12);
    tui.send(b"\x1b[<0;31;13m");
    thread::sleep(Duration::from_millis(80));
    tui.read();
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log.contains("key:"), "review input reached Flutter: {log}");
    assert!(
        !tui.raw_output.windows(5).any(|bytes| bytes == b"\x1b]52;"),
        "review selection emitted a clipboard request"
    );
    tui.send(b"\x15SESSION\r");
    tui.until("n/N matches");
    tui.until("1/1");
    tui.send(b"rR\x01\x05\x18");
    thread::sleep(Duration::from_millis(80));
    assert!(!std::fs::read_to_string(&log_path).unwrap().contains("key:"));
    tui.menu_click("Hot reload");
    tui.until("Hot reload requested");
    tui.menu_click("Hot restart");
    tui.until("Hot restart requested");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let log = std::fs::read_to_string(&log_path).unwrap();
        if log.contains("key:r\n") && log.contains("key:R\n") {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    tui.read();
    assert!(tui.screen.screen().contents().contains("Snapshot"));
    assert!(
        !tui.screen.screen().contents().contains("shortcut:r"),
        "process output replaced a frozen snapshot"
    );
    tui.menu_click("Refresh snapshot");
    tui.until("1/1");
    tui.menu_click("Snapshot source details");
    tui.until("Frozen source details");
    tui.send(b"\x1b");
    tui.until("n/N matches");
    tui.send(b"\x1b");
    tui.until("Resumed live console");
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert_eq!(log.matches("key:r\n").count(), 1);
    assert_eq!(log.matches("key:R\n").count(), 1);
    assert!(!log.contains("key:q\n"));
    tui.send(b"r");
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_to_string(&log_path)
        .unwrap()
        .matches("key:r\n")
        .count()
        != 2
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    tui.send(&[0x13]);
    tui.until("Snapshot");
    tui.send(&[0x11]);
    tui.until("Quit Flimru?");
    tui.send(b"\r");
    tui.until("Snapshot");
    tui.quit();
}

#[test]
fn console_search_frozen_output_is_quiet_during_real_child_flood_and_refresh_observes_new_history()
{
    let project = project();
    std::fs::write(project.path().join("fake-flutter"),r#"#!/bin/bash
if [ "$1" = devices ]; then printf '[{"id":"demo-device","name":"Demo phone","targetPlatform":"ios","isSupported":true}]\n'; exit 0; fi
printf '%s' "$$" > "$FAKE_PID"
stty raw -echo
printf 'ERROR: FLOOD READY\r\n'
( while :; do printf 'ERROR: flood heartbeat\r\n'; printf x >> "$FAKE_LOG"; sleep 0.002; done ) &
while :; do IFS= read -rsn1 key || continue; [ "$key" = q ] && exit 0; done
"#).unwrap();
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("FLOOD READY");
    tui.send(&[0x13]);
    tui.until("Snapshot");
    tui.until("rows · col");
    tui.read();
    let screen = tui.screen.screen().contents();
    tui.raw_output.clear();
    let before = std::fs::metadata(project.path().join("session.log"))
        .unwrap()
        .len();
    thread::sleep(Duration::from_millis(220));
    tui.read();
    let after = std::fs::metadata(project.path().join("session.log"))
        .unwrap()
        .len();
    assert!(after > before, "frozen UI stopped draining the owned child");
    assert_eq!(tui.screen.screen().contents(), screen);
    assert!(
        tui.raw_output.len() < 128,
        "frozen output generated redundant terminal draws: {} bytes",
        tui.raw_output.len()
    );
    tui.send(b"\r");
    tui.send(b"r");
    tui.until("rows · col");
    tui.send(b"/heartbeat");
    tui.until("1/");
    tui.send(b"\x1b");
    tui.until("n/N matches");
    tui.send(b"\x1b");
    tui.until("Resumed live console");
    tui.quit();
}

#[test]
fn console_export_real_binary_saves_full_filtered_snapshot_and_isolates_modal_input() {
    let project = project();
    let log_path = project.path().join("session.log");
    let saved_path = project.path().join("explicit-retained-log.txt");
    let mut tui = Terminal::start_with_refresh_interval(
        project.path(),
        &project.path().join("state.json"),
        false,
        3600,
    );
    tui.until("Demo phone");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("SESSION READY");
    tui.send(b"c");
    tui.until("INFO: connected");
    tui.send(&[0x13]);
    tui.until("Search");
    tui.send(b"SESSION\r");
    tui.until("1/1");
    tui.menu_click("Console severity: Errors");
    tui.until("No matches");
    tui.menu_click("Export retained console log");
    tui.until("Export retained console log");
    tui.until("Ready");
    assert!(!project.path().join("flimru-logs").exists());
    assert!(!saved_path.exists());
    tui.send(b"rRq\x05\x10"); // Plain text plus covered selection/menu shortcuts.
    tui.mouse(0, 0, 0); // Background input belongs to the export modal.
    tui.send(&[0x15]);
    tui.send(format!("\x1b[200~{}\x1b[201~", saved_path.display()).as_bytes());
    tui.send(b"\r");
    tui.until("Saved console log");
    let exported = std::fs::read_to_string(&saved_path).unwrap();
    let (header, retained) = exported.split_once("\n\n").unwrap();
    assert!(header.contains("Device: Demo phone"));
    assert!(header.contains("Configuration: Demo debug"));
    assert!(!header.contains("--dart-define"));
    assert!(retained.contains("SESSION READY"));
    assert!(
        retained.contains("INFO: connected"),
        "query/severity-hidden lines must be exported"
    );
    assert!(retained.contains("SEVERE: failed"));
    assert!(!exported.contains('\x1b'));
    let child_input = std::fs::read_to_string(&log_path).unwrap();
    assert_eq!(child_input.matches("key:c\n").count(), 1);
    for key in ["r", "R", "q"] {
        assert!(!child_input.contains(&format!("key:{key}\n")));
    }
    tui.menu_click("Operation details");
    tui.until("Latest operation outcome");
    tui.until("explicit-retained-log");
    tui.until("bytes).");
    tui.send(b"\x1b");
    tui.until("Snapshot");
    tui.quit();
}

#[path = "support/productivity_workflow_pty.rs"]
mod productivity_workflow_pty;

#[path = "support/shared_sessions_pty.rs"]
mod shared_sessions_pty;
