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
        ]);
        command.env("TERM", "xterm-256color");
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
    tui.mouse(0, 3, 1);
    tui.until("Configurations (1)");
    tui.mouse(0, 12, 1);
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
        r#"import 'dart:developer';
import 'dart:convert';
import 'dart:io';
void main() {
  var factor = 1.0;
  registerExtension('ext.flutter.timeDilation', (method, parameters) async {
    if (parameters.containsKey('timeDilation')) factor = double.parse(parameters['timeDilation']!);
    return ServiceExtensionResponse.result(jsonEncode({'timeDilation': factor.toString()}));
  });
  stdin.lineMode = false;
  stdin.echoMode = false;
  print('PROBE READY');
  stdin.listen((data) { for (final key in data) { if (key == 113) exit(0); } });
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
    tui.until("Animations: 5× slower");
    tui.send(b"\x1b[12~");
    tui.until("Animations: normal");
    tui.send(b"q");
    tui.until("ended (");
    tui.quit();
}
