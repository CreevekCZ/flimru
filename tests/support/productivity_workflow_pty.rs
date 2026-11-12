//! Final cross-feature flow, registered by the coordinator only after P5.
use super::*;

fn wait_for(tui: &mut Terminal, description: &str, mut ready: impl FnMut(&Terminal) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        tui.read();
        if ready(tui) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for {description}:\n{}",
            tui.screen.screen().contents()
        );
        assert!(tui.child.try_wait().unwrap().is_none(), "Launcher exited");
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_original_review(tui: &mut Terminal) {
    tui.until("Snapshot · Alpha · Demo debug");
    tui.until("Search: ORIGINAL");
    tui.until("Errors · 1/1");
    let screen = tui.screen.screen().contents();
    assert!(screen.contains("ORIGINAL error alpha"), "{screen}");
    assert!(!screen.contains("ORIGINAL info alpha"), "{screen}");
    assert!(!screen.contains("shortcut:alpha:"), "{screen}");
}

fn read_log(project: &Path) -> String {
    std::fs::read_to_string(project.join("session.log")).unwrap_or_default()
}

fn key_count(log: &str, device: &str, key: char) -> usize {
    let expected = format!("key:{device}:{key}");
    log.lines().filter(|line| *line == expected).count()
}

#[test]
fn automatic_catalog_reload_filtered_export_and_batch_commands_preserve_frozen_review() {
    let project = multi_project();
    let script_path = project.path().join("fake-flutter");
    let script = std::fs::read_to_string(&script_path).unwrap();
    let ready = "printf 'READY:%s\\r\\n' \"$device\"";
    assert!(script.contains(ready), "multi-device fixture changed");
    std::fs::write(
        script_path,
        script.replace(
            ready,
            "printf 'start:%s\\n' \"$device\" >> \"$FAKE_LOG\"\n\
             printf 'READY:%s\\r\\n' \"$device\"\n\
             printf 'flutter: ERROR: ORIGINAL error %s\\r\\n' \"$device\"\n\
             printf 'flutter: INFO: ORIGINAL info %s\\r\\n' \"$device\"",
        ),
    )
    .unwrap();
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
    tui.until_control("Run");
    tui.send(b"\r");
    tui.until("ORIGINAL info alpha");
    // Visit Beta once to observe that both actual children have started.
    tui.send(b"\x06");
    tui.until("ORIGINAL info beta");
    tui.send(b"\x02");
    tui.until("ORIGINAL info alpha");
    let child_pids = ["alpha", "beta"].map(|device| {
        std::fs::read_to_string(project.path().join(format!("child.pid.{device}"))).unwrap()
    });

    tui.send(b"\x13"); // Ctrl+s: freeze Alpha before any hot command.
    tui.until("Snapshot");
    tui.send(b"ORIGINAL\r");
    tui.until("All · 1/2");
    tui.menu_click("Console severity: Errors");
    assert_original_review(&mut tui);

    // Observe the automatic watcher's failed version and successful recovery
    // while review stays open; no timing assumption about its debounce is needed.
    let launch_path = project.path().join(".vscode/launch.json");
    std::fs::write(&launch_path, "{temporary incomplete save").unwrap();
    tui.until("Config error");
    assert_original_review(&mut tui);
    std::fs::write(
        &launch_path,
        r#"{"configurations":[
            {"name":"Future debug"},
            {"name":"Future blocked","preLaunchTask":"unsupported"}
        ]}"#,
    )
    .unwrap();
    wait_for(
        &mut tui,
        "automatic catalog recovery during review",
        |tui| !tui.screen.screen().contents().contains("Config error"),
    );
    assert_original_review(&mut tui);

    // Explicit single-session action produces later live text without replacing
    // the frozen snapshot. File evidence proves the child received the byte.
    tui.menu_click("Hot reload");
    wait_for(&mut tui, "Alpha single-session reload", |_| {
        key_count(&read_log(project.path()), "alpha", 'r') == 1
    });
    assert_original_review(&mut tui);

    let destination = project.path().join("chosen-frozen-log.txt");
    tui.menu_click("Export retained console log");
    tui.until("Destination (no overwrite)");
    tui.until("Enter Export; Esc Cancel");
    tui.send(b"\x15"); // Edit the actual destination, rather than using its default.
    tui.send(b"\x1b[200~");
    tui.send(destination.to_str().unwrap().as_bytes());
    tui.send(b"\x1b[201~\r");
    wait_for(&mut tui, "completed frozen export", |tui| {
        destination.is_file()
            && !tui
                .screen
                .screen()
                .contents()
                .contains("Destination (no overwrite)")
    });
    let exported = std::fs::read_to_string(&destination).unwrap();
    assert!(exported.contains("Device: Alpha\n"), "{exported}");
    assert!(
        exported.contains("Configuration: Demo debug\n"),
        "{exported}"
    );
    let (_, body) = exported.split_once("\n\n").unwrap();
    assert_eq!(
        body,
        "READY:alpha\nflutter: ERROR: ORIGINAL error alpha\nflutter: INFO: ORIGINAL info alpha\n"
    );
    assert!(!exported.contains("shortcut:"), "{exported}");
    assert!(!exported.contains('\x1b'), "export retained ANSI controls");
    assert_original_review(&mut tui);

    for (action, title, key, alpha_count) in [
        (
            "Reload all eligible (2)",
            "Hot reload delivery report",
            'r',
            2,
        ),
        (
            "Restart all eligible (2)",
            "Hot restart delivery report",
            'R',
            1,
        ),
    ] {
        tui.menu_click(action);
        tui.until(title);
        tui.until("Requested/queued: 2");
        tui.until("Skipped: 0 · Failed: 0");
        tui.until("Flutter completion is unconfirmed.");
        wait_for(&mut tui, "one batch byte per eligible target", |_| {
            let log = read_log(project.path());
            key_count(&log, "alpha", key) == alpha_count && key_count(&log, "beta", key) == 1
        });
        tui.send(b"\x1b");
        assert_original_review(&mut tui);
    }

    tui.menu_click("Resume live console");
    tui.until("shortcut:alpha:R");
    tui.send(b"\x04"); // Browse accepted config changes without replacing either child.
    tui.until("Multi 2/8");
    tui.send(b"\r");
    tui.until("Future debug");
    tui.menu_click("Show unavailable");
    tui.until("Future blocked");
    tui.send(b"\x1b[F\r"); // Unavailable selection opens explanation, never Run.
    tui.until("Diagnostics");
    tui.until("preLaunchTask");
    let log = read_log(project.path());
    for (device, pid) in ["alpha", "beta"].into_iter().zip(child_pids) {
        assert_eq!(
            std::fs::read_to_string(project.path().join(format!("child.pid.{device}"))).unwrap(),
            pid,
            "catalog browsing or hot command respawned {device}"
        );
        assert_eq!(
            log.lines()
                .filter(|line| *line == format!("start:{device}"))
                .count(),
            1
        );
    }
    assert_eq!(key_count(&log, "alpha", 'r'), 2);
    assert_eq!(key_count(&log, "beta", 'r'), 1);
    assert_eq!(key_count(&log, "alpha", 'R'), 1);
    assert_eq!(key_count(&log, "beta", 'R'), 1);
    // Quit is global even while the configuration explanation owns input.
    tui.quit();
    assert_multi_children_reaped(project.path());
}
