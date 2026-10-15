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
}

impl Terminal {
    fn start(project: &Path, state: &Path) -> Self {
        Self::start_with_color(project, state, "always")
    }

    fn start_with_color(project: &Path, state: &Path, color: &str) -> Self {
        Self::start_with_options(project, state, color, false)
    }

    fn start_with_options(project: &Path, state: &Path, color: &str, multi: bool) -> Self {
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
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_flutter-run-tui"));
        command.arg(project);
        command.args([
            "--flutter",
            project.join("fake-flutter").to_str().unwrap(),
            "--state",
            state.to_str().unwrap(),
            "--refresh-interval",
            "2",
            "--color",
            color,
        ]);
        if multi {
            command.arg("--multi");
        }
        command.env("TERM", "xterm-256color");
        // Reproduce launchers/shells which suppress color. --color always must
        // override this for both the terminal backend and its Flutter child.
        command.env("NO_COLOR", "1");
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
        }
    }

    fn read(&mut self) {
        let mut buffer = [0; 8192];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => self.screen.process(&buffer[..count]),
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
        self.until("Quit Flutter Run?");
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
        tui.until("Configurations (3)");
        tui.send(b"\r");
        tui.until("READY:alpha");
        let alpha = pid_for("alpha");
        tui.send(b"h");
        tui.until("shortcut:alpha:h");
        tui.menu_click("Add session");
        tui.until("Add session");
        tui.send(b"\x1b[B\r"); // Beta, original configuration.
        tui.until("Configurations (3)");
        tui.send(b"\r");
        tui.until("READY:beta");
        let old_beta = pid_for("beta");
        alive(alpha);
        tui.send(b"\x1b[20~"); // F9, then Gamma with a different configuration.
        tui.until("Add session");
        tui.send(b"\x1b[B\r");
        tui.until("Configurations (3)");
        tui.send(b"\x1b[B\r");
        tui.until("READY:alternate");
        let alternate = pid_for("alternate");
        alive(alpha);
        alive(old_beta);
        tui.send(b"\x1b[15~q"); // Stop only Beta.
        tui.until("2 session(s) running");
        assert_eq!(
            // SAFETY: signal zero checks the old Beta child, without signaling it.
            unsafe { libc::kill(old_beta, 0) },
            -1
        );
        tui.send(b"\x1b[20~");
        tui.until("Add session");
        tui.send(b"\x1b[H\x1b[B\r"); // Beta again, choose original config instead of the saved Alternate.
        tui.until("Configurations (3)");
        tui.send(b"\x1b[H\r");
        tui.until("READY:beta");
        let beta = pid_for("beta");
        assert_ne!(old_beta, beta);
        alive(alpha);
        alive(alternate);
        tui.send(b"\x1b[15~\x1b[15~\x1b[15~"); // Back to original Alpha, with its original history.
        tui.until("shortcut:alpha:h");
        tui.send(b"r");
        tui.until("shortcut:alpha:r");
        tui.send(b"\x1b[20~");
        tui.until("Add session");
        tui.send(b"\x1b[H\r"); // Alpha with Demo debug would be a duplicate.
        tui.until("Configurations (3)");
        tui.send(b"\x1b[H\r");
        tui.until("Already running");
        assert_eq!(pid_for("alpha"), alpha);
        tui.send(b"\x1b[F\r"); // Invalid preLaunchTask must leave all three alone.
        tui.until("Cannot start batch");
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
        tui.send(b"\x1b[21~"); // F10 focuses the bar in a live console.
        tui.until_control("›Stop");
        tui.send(b"R\x1b[200~r\x1b[201~\x1b[C\x1b[C\x1b[C");
        tui.until_control("›Color");
        tui.send(b" ");
        tui.until_control(if compact { "Color off" } else { "Color: off" });
        tui.send(b"\x1bOS"); // F4 opens the menu without losing the trigger focus.
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
        tui.until_control("›Color");
        if compact {
            tui.send(b"\x1b");
        } else {
            tui.mouse(0, 20, 8); // Clicking console content restores input focus too.
        }
        tui.until("F10 actions");
        tui.send(b"h");
        tui.until("shortcut:h");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(!log.contains("key:R\n") && !log.contains("key:r\n"));
        assert!(!log.contains("key:[\n") && !log.contains("key:\u{1b}\n"));
        tui.send(b"\x1b[21~\r"); // Stop via a keyboard-focused toolbar action.
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
    tui.click_text("F6 ›");
    tui.until("READY:beta");
    assert!(!tui.screen.screen().contents().contains("shortcut:alpha:h"));
    tui.send(b"v");
    tui.until("shortcut:beta:v");
    tui.send(b"\x1b[15~"); // F5 switches back.
    tui.until("shortcut:alpha:h");
    assert!(!tui.screen.screen().contents().contains("shortcut:beta:v"));
    tui.send(b"o");
    tui.until("alpha-history:099");
    tui.send(b"\x1b[H");
    tui.until("READY:alpha");
    tui.click_text("F6 ›");
    tui.until("shortcut:beta:v");
    tui.click_text("F5 ‹");
    tui.until("READY:alpha"); // Per-console scroll position survives switching.
    tui.send(b"q");
    tui.until("1 session(s) running");
    tui.click_text("F6 ›");
    tui.send(b"r");
    tui.until("shortcut:beta:r");
    tui.send(b"x");
    tui.until("0 session(s) running");
    tui.send(b"\x1b[15~");
    tui.until("ended");
    tui.send(b"\x1b[H");
    tui.until("READY:alpha");
    tui.send(b"\x1b[18~"); // F7 returns to selection, without losing the batch.
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("READY:alpha");
    tui.send(b"\x1b[19~"); // F8 stops every session.
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
fn multi_device_quit_and_terminal_loss_reap_every_instance() {
    for hang_up in [false, true] {
        let project = multi_project();
        let state = project.path().join("state.json");
        let mut tui = Terminal::start_with_options(project.path(), &state, "always", true);
        tui.until("Alpha");
        tui.send(b" \x1b[B ");
        tui.until("Multi 2/8");
        tui.send(b"\r");
        tui.until("Configurations (1)");
        tui.send(b"\r");
        tui.until("READY:alpha");
        tui.send(b"\x1b[17~");
        tui.until("READY:beta");
        tui.send(b"\x1b[18~");
        tui.until("Multi 2/8"); // Hidden sessions still count in quit confirmation.
        if hang_up {
            tui.hang_up();
            tui.wait_for_exit(Instant::now() + Duration::from_secs(2));
        } else {
            tui.send(&[0x11]);
            tui.until("All active Flutter sessions");
            tui.send(b"\r"); // Cancel preserves both children.
            tui.send(b"\x1b[17~h");
            tui.until("shortcut:beta:h");
            tui.quit();
        }
        assert_multi_children_reaped(project.path());
    }
}

#[test]
fn colors_survive_pty_scrollback_saved_logs_and_runtime_toggle() {
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
    tui.send(b"\r");
    tui.until_color("SESSION READY", vt100::Color::Idx(2));
    tui.send(b"c");
    tui.until("NO_COLOR=unset");
    tui.until_color("INDEXED COLOR", vt100::Color::Idx(208));
    tui.until_color("TRUE COLOR", vt100::Color::Rgb(12, 200, 99));
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.until_color("SEVERE:", vt100::Color::Rgb(255, 128, 142));
    tui.until_color("INFO:", vt100::Color::Rgb(96, 210, 225));
    tui.send(b"\x1bOR"); // F3
    tui.until_color("WARNING:", vt100::Color::Default);
    tui.send(b"\x1bOR");
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
    tui.send(b"l");
    tui.until("Last console ·");
    tui.menu_click("Oldest output");
    tui.until_color("TRUE COLOR", vt100::Color::Rgb(12, 200, 99));
    tui.until_color("WARNING:", vt100::Color::Rgb(255, 205, 112));
    tui.quit();
}

#[test]
fn auto_and_never_color_modes_respect_inherited_no_color() {
    for mode in ["auto", "never"] {
        let project = project();
        let state = project.path().join("state.json");
        std::fs::write(
            &state,
            r#"{"favorites":[{"id":"demo-device","name":"Saved phone","platform":"ios"}]}"#,
        )
        .unwrap();
        let mut tui = Terminal::start_with_color(project.path(), &state, mode);
        tui.until("Saved phone");
        tui.send(b"\r");
        tui.until("Demo debug");
        tui.send(b"\r");
        tui.until_color("SESSION READY", vt100::Color::Default);
        tui.send(b"c");
        tui.until("NO_COLOR=1");
        tui.until_color("INDEXED COLOR", vt100::Color::Default);
        tui.until_color("WARNING:", vt100::Color::Default);
        tui.send(b"q");
        tui.until("ended (");
        tui.quit();
    }
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
    tui.send(b"\x1bOS"); // Keyboard F4, End, Enter selects Quit.
    tui.until("Actions ·");
    tui.send(b"\x1b[F");
    tui.until("Quit launcher");
    tui.send(b"\r");
    tui.until("Quit Flutter Run?");
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
    tui.until("Configurations (1)");
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
    tui.until("Configurations (1)");
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
    tui.until("Quit Flutter Run?");
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
            tui.until("Configurations (1)");
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
    tui.until("Configurations (1)");
    tui.send(b"\r");
    tui.until("PROBE READY");
    tui.until("Animations: normal");
    tui.send(b"\x1b[12~");
    tui.until("PROBE SPEED:1:5.0");
    tui.until("Animations: 5× slower");
    tui.send(b"R");
    tui.until("PROBE READY 2");
    tui.until("PROBE SPEED:2:5.0");
    tui.until("Animations: 5× slower");
    tui.send(b"\x1b[12~");
    tui.until("PROBE SPEED:2:1.0");
    tui.until("Animations: normal");
    tui.send(b"R");
    tui.until("PROBE READY 3");
    tui.until("Animations: normal");
    tui.send(b"q");
    tui.until("ended (");
    tui.quit();
}
