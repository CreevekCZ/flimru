//! Real Flimru TUI, CLI and MCP subprocesses with two fake Flutter devices.

use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    process::{self, ChildStdin, ChildStdout, Command, Output, Stdio},
    sync::mpsc,
};

fn flimru() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flimru"))
}

fn shell(project: &Path, args: &[&str]) -> Output {
    flimru()
        .current_dir(project)
        .arg("session")
        .args(args)
        .output()
        .unwrap()
}

fn succeeded(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn cli(tui: &mut Terminal, project: &Path, args: &[&str]) -> Value {
    let output = while_draining(tui, || shell(project, args));
    serde_json::from_str(&succeeded(output)).unwrap()
}

fn while_draining<T: Send>(tui: &mut Terminal, request: impl FnOnce() -> T + Send) -> T {
    thread::scope(|scope| {
        let worker = scope.spawn(request);
        while !worker.is_finished() {
            tui.read();
            thread::sleep(Duration::from_millis(10));
        }
        worker.join().unwrap()
    })
}

fn eventually(tui: &mut Terminal, mut ready: impl FnMut(&mut Terminal) -> bool) {
    let end = Instant::now() + Duration::from_secs(6);
    loop {
        tui.read();
        if ready(tui) {
            break;
        }
        assert!(
            Instant::now() < end,
            "Condition timed out:\n{}",
            tui.screen.screen().contents()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

struct Mcp {
    child: process::Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
}

impl Mcp {
    fn connect(project: &Path) -> Self {
        let mut child = flimru()
            .args(["mcp", "--project"])
            .arg(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            input,
            output,
            id: 0,
        };
        let initialize = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1.0"}
            }),
        );
        assert_eq!(initialize["protocolVersion"], "2025-11-25");
        writeln!(
            client.input,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        let list = client.request("tools/list", json!({}));
        let tools = list["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);
        assert!(tools.iter().any(|tool| tool["name"] == "flimru_logs"));
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(
            self.input,
            "{}",
            json!({
                "jsonrpc": "2.0", "id": self.id, "method": method, "params": params
            })
        )
        .unwrap();
        let mut line = String::new();
        assert_ne!(self.output.read_line(&mut line).unwrap(), 0);
        let message: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(message["id"], self.id);
        assert_eq!(message["jsonrpc"], "2.0");
        assert!(message["error"].is_null(), "{message}");
        message["result"].clone()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    fn call(&mut self, tui: &mut Terminal, name: &str, arguments: Value) -> Value {
        let result = while_draining(tui, || self.tool(name, arguments));
        assert_ne!(result["isError"], true, "{result}");
        assert!(result["structuredContent"].is_object(), "{result}");
        result["structuredContent"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Follower {
    process: process::Child,
    lines: mpsc::Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
}

impl Follower {
    fn start(project: &Path, id: &str) -> Self {
        let mut process = flimru()
            .current_dir(project)
            .args(["session", "logs", id, "--follow"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = process.stdout.take().unwrap();
        let (sender, lines) = mpsc::sync_channel(16);
        let reader = Some(thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.try_send(line).is_err() {
                    break;
                }
            }
        }));
        Self {
            process,
            lines,
            reader,
        }
    }

    fn next(&mut self, tui: &mut Terminal, expected: &str) {
        let lines = &mut self.lines;
        assert_eq!(
            while_draining(tui, move || lines
                .recv_timeout(Duration::from_secs(6))
                .unwrap()),
            expected
        );
    }

    fn wait_for_end(&mut self) {
        let end = Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(status) = self.process.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < end,
                "Follow did not exit after the session ended"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

#[test]
fn shared_mcp_and_cli_observe_and_control_both_launch_directions() {
    let project = multi_project();
    let workspace = project.path();
    let mut mcp = Mcp::connect(workspace);
    let closed = mcp.tool("flimru_targets", json!({}));
    assert_eq!(closed["isError"], true);
    assert!(
        closed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Open Flimru")
    );
    let mut tui = Terminal::start(workspace, &workspace.join("state.json"));
    tui.until("Alpha");

    let targets = mcp.call(&mut tui, "flimru_targets", json!({}));
    assert_eq!(targets["devices"][0]["id"], "alpha");
    assert_eq!(targets["devices"][1]["id"], "beta");
    assert_eq!(targets["configurations"][0]["name"], "Demo debug");
    tui.send(b"\r");
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("READY:alpha");

    let subdirectory = workspace.join("lib");
    std::fs::create_dir(&subdirectory).unwrap();
    let sessions = cli(&mut tui, &subdirectory, &["list"]);
    let alpha = sessions["sessions"][0]["id"].as_str().unwrap();
    let alpha_status = mcp.call(&mut tui, "flimru_sessions", json!({"id": alpha}));
    assert_eq!(alpha_status["device"]["id"], "alpha");
    assert!(alpha_status["controls"]["reload"].is_null());
    let alpha_logs = mcp.call(&mut tui, "flimru_logs", json!({"id": alpha}));
    assert_eq!(alpha_logs["text"], "READY:alpha\n");
    let alpha_cursor = alpha_logs["cursor"].as_u64().unwrap();

    let mut follow_alpha = Follower::start(workspace, alpha);
    follow_alpha.next(&mut tui, "READY:alpha");
    tui.send(b"r");
    tui.until("shortcut:alpha:r");
    follow_alpha.next(&mut tui, "shortcut:alpha:r");
    drop(follow_alpha);
    let incremental = mcp.call(
        &mut tui,
        "flimru_logs",
        json!({"id": alpha, "cursor": alpha_cursor}),
    );
    assert_eq!(incremental["text"], "shortcut:alpha:r\n");
    assert_eq!(incremental["state"], "running");

    // Start while the human browses devices. The first run remains owned.
    tui.send(b"\x04");
    tui.until("Devices (2)");
    let beta_run = mcp.call(
        &mut tui,
        "flimru_start",
        json!({"device": "beta", "config": "Demo debug"}),
    );
    let beta = beta_run["id"].as_str().unwrap();
    assert_ne!(beta, alpha);
    tui.until("READY:beta");
    assert_eq!(
        cli(&mut tui, workspace, &["list"])["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(mcp);

    // An independent agent reconnects and can read the earlier output.
    let mut mcp = Mcp::connect(workspace);
    let mut follow_beta = Follower::start(workspace, beta);
    follow_beta.next(&mut tui, "READY:beta");
    cli(&mut tui, workspace, &["reload", beta]);
    tui.until("shortcut:beta:r");
    follow_beta.next(&mut tui, "shortcut:beta:r");
    let result = mcp.call(
        &mut tui,
        "flimru_control",
        json!({"id": beta, "control": "restart"}),
    );
    assert_eq!(result["message"], "Hot restart requested");
    tui.until("shortcut:beta:R");
    follow_beta.next(&mut tui, "shortcut:beta:R");
    cli(&mut tui, workspace, &["control", beta, "help"]);
    follow_beta.next(&mut tui, "shortcut:beta:h");

    mcp.call(
        &mut tui,
        "flimru_control",
        json!({"id": beta, "control": "stop"}),
    );
    follow_beta.next(&mut tui, "shortcut:beta:q");
    while_draining(&mut tui, || follow_beta.wait_for_end());
    assert_eq!(
        cli(&mut tui, workspace, &["inspect", beta])["state"],
        "ended"
    );
    assert_eq!(
        cli(&mut tui, workspace, &["inspect", alpha])["state"],
        "running"
    );
    tui.send(b"\x02");
    tui.until("READY:alpha");
    tui.send(b"q");
    eventually(&mut tui, |tui| {
        cli(tui, workspace, &["inspect", alpha])["state"] == "ended"
    });
    let retained = succeeded(while_draining(&mut tui, || {
        shell(workspace, &["logs", alpha])
    }));
    assert_eq!(
        retained,
        "READY:alpha\nshortcut:alpha:r\nshortcut:alpha:q\n"
    );
    let routed = std::fs::read_to_string(workspace.join("session.log")).unwrap();
    assert_eq!(
        routed,
        "key:alpha:r\nkey:beta:r\nkey:beta:R\nkey:beta:h\nkey:beta:q\nkey:alpha:q\n"
    );

    let launched = cli(
        &mut tui,
        workspace,
        &["start", "--device", "beta", "--config-index", "1"],
    );
    let new_beta = launched["id"].as_str().unwrap();
    assert_ne!(new_beta, beta);
    tui.until("READY:beta");
    cli(&mut tui, workspace, &["stop", new_beta]);
    eventually(&mut tui, |tui| {
        cli(tui, workspace, &["inspect", new_beta])["state"] == "ended"
    });
    assert_eq!(
        succeeded(while_draining(&mut tui, || shell(
            workspace,
            &["logs", new_beta]
        ))),
        "READY:beta\nshortcut:beta:q\n"
    );
    tui.quit();
    assert_multi_children_reaped(workspace);
    let closed = mcp.tool("flimru_sessions", json!({}));
    assert_eq!(closed["isError"], true);
}

#[test]
fn shared_ownership_is_project_scoped_and_start_errors_are_actionable() {
    let project = multi_project();
    let workspace = project.path();
    let other = multi_project();
    let config = workspace.join(".vscode/launch.json");
    std::fs::write(
        &config,
        r#"{"configurations":[
        {"name":"Demo debug"}, {"name":"Demo debug"},
        {"name":"Blocked","preLaunchTask":"unsupported"},
        {"name":"Release","flutterMode":"release"}
    ]}"#,
    )
    .unwrap();
    let mut primary = Terminal::start(workspace, &workspace.join("primary-state.json"));
    primary.until("Alpha");
    let mut secondary = Terminal::start(workspace, &workspace.join("secondary-state.json"));
    secondary.until("Another Flimru window");
    assert_eq!(
        cli(&mut primary, workspace, &["list"])["sessions"],
        json!([])
    );
    let wrong_project = shell(other.path(), &["list"]);
    assert!(!wrong_project.status.success());
    assert!(String::from_utf8_lossy(&wrong_project.stderr).contains("Open Flimru"));

    for (device, config, expected) in [
        ("alpha", "Demo debug", "ambiguous"),
        ("alpha", "Blocked", "preLaunchTask"),
        ("missing", "Release", "Unknown device ID"),
    ] {
        let output = while_draining(&mut primary, || {
            shell(
                workspace,
                &["start", "--device", device, "--config", config],
            )
        });
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{output:?}"
        );
    }
    let run = cli(
        &mut primary,
        workspace,
        &["start", "--device", "alpha", "--config", "Release"],
    );
    let id = run["id"].as_str().unwrap();
    primary.until("READY:alpha");
    assert!(
        run["controls"]["reload"]
            .as_str()
            .unwrap()
            .contains("release")
    );
    let reload = while_draining(&mut primary, || shell(workspace, &["reload", id]));
    assert!(!reload.status.success());
    assert!(String::from_utf8_lossy(&reload.stderr).contains("release"));
    assert!(!workspace.join("session.log").exists());

    primary.quit();
    let closed = shell(workspace, &["list"]);
    assert!(
        !closed.status.success(),
        "The second TUI unexpectedly became the owner"
    );
    secondary.quit();
    let mut reopened = Terminal::start(workspace, &workspace.join("reopened-state.json"));
    reopened.until("Alpha");
    let stale = while_draining(&mut reopened, || shell(workspace, &["stop", id]));
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("earlier or different"));
    reopened.quit();
}
