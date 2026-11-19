#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use flimru::batch_commands::{HotCapabilities, HotExclusion, TargetOutcome};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::Path};

const AREA: Rect = Rect::new(0, 0, 80, 24);

fn until(app: &mut App, mut ready: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(app) {
        app.tick().unwrap();
        assert!(
            Instant::now() < deadline,
            "batch fixture did not settle: {}",
            app.status
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
fn live(directory: &Path, name: &str, capabilities: HotCapabilities, reading: bool) -> runs::Run {
    let script = if reading {
        "stty raw -echo; printf 'ready'; while :; do IFS= read -rsn1 key || continue; printf '%s' \"$key\" >> \"$INPUT_LOG\"; done"
    } else {
        "stty raw -echo; printf 'ready'; exec sleep 30"
    };
    let session = Session::start(
        &flimru::tool::RunCommand {
            capture_vm_service: false,
            hot_capabilities: capabilities,
            executable: "/bin/bash".into(),
            args: vec!["-c".into(), script.into()],
            cwd: directory.into(),
            env: BTreeMap::from([(
                "INPUT_LOG".into(),
                directory.join(name).display().to_string(),
            )]),
        },
        24,
        80,
    )
    .unwrap();
    runs::Run::running(
        Device {
            id: name.into(),
            name: name.into(),
            platform: "android".into(),
            available: true,
            favorite: false,
        },
        "Launch-time debug".into(),
        session,
        capabilities,
    )
}
fn ended(app: &App) -> runs::Run {
    let mut run = runs::Run::failed(
        app.devices[0].clone(),
        "Frozen original".into(),
        "ended".into(),
        24,
        80,
    );
    let mut parser = vt100::Parser::new(24, 80, 150);
    for index in 0..100 {
        parser.process(format!("ERROR: needle {index:03} 界e\u{301}\r\n").as_bytes());
    }
    run.console = Some(parser);
    run.history_capacity = 150;
    run.scrollback_len = 77;
    run
}
fn fixture(directory: &Path) -> App {
    let mut app = crate::tests::app();
    app.page = Page::Console;
    app.retain_consoles = true;
    app.ownit = false;
    app.runs
        .push(live(directory, "first", HotCapabilities::Supported, true));
    until(&mut app, |app| {
        app.runs.iter().all(|run| {
            run.screen()
                .is_some_and(|screen| screen.contents().contains("ready"))
        })
    });
    app
}
fn press(app: &mut App, code: KeyCode) {
    assert!(
        !app.key(KeyEvent::new(code, KeyModifiers::NONE), 80, 24)
            .unwrap()
    );
}
fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn batch_hot_launch_capabilities_remain_stored_across_catalog_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let executable = root.join("compatible-wrapper");
    std::fs::write(&executable, "#!/bin/sh\nprintf 'ready\\n'\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = root.join("launch.json");
    std::fs::write(
        &path,
        r#"{"configurations":[{"name":"Debug"},{"name":"Profile","flutterMode":"profile"}]}"#,
    )
    .unwrap();
    let mut app = crate::tests::app();
    app.ownit = false;
    app.tool = FlutterTool {
        executable,
        prefix: vec![],
        workspace: root.into(),
    };
    app.launch = Arc::new(LaunchFile::load(&path, root).unwrap());
    app.multi = true;
    app.marked_devices = vec![app.devices[0].clone()];
    app.settle_availability();
    app.set_config_selection(0);
    app.start_batch(80, 24);
    assert_eq!(app.runs.len(), 1);
    let mut second = app.devices[0].clone();
    second.id = "second-target".into();
    app.marked_devices = vec![second];
    app.settle_availability();
    app.set_config_selection(1);
    app.start_batch(80, 24);
    assert_eq!(app.runs.len(), 2, "{}", app.status);
    let ids: Vec<_> = app.runs.iter().map(|run| run.id).collect();
    std::fs::write(
        &path,
        r#"{"configurations":[{"name":"Debug","flutterMode":"release"},{"name":"Profile"}]}"#,
    )
    .unwrap();
    assert!(app.apply_config_reload(ReloadResult {
        generation: 1,
        source: ReloadSource::Automatic,
        result: Ok(LaunchFile::load(&path, root).unwrap())
    }));
    assert_eq!(app.runs.iter().map(|run| run.id).collect::<Vec<_>>(), ids);
    assert_eq!(app.runs[0].hot_capabilities, HotCapabilities::Supported);
    assert_eq!(
        app.runs[1].hot_capabilities,
        HotCapabilities::Unsupported(HotExclusion::Profile)
    );
    assert_eq!(app.hot_eligible_count(), 1);
}

#[test]
fn batch_hot_mixed_targets_skip_and_continue_after_a_real_full_input_queue() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.runs.push(ended(&app));
    for (name, caps, reading) in [
        ("stopping", HotCapabilities::Supported, true),
        (
            "profile",
            HotCapabilities::Unsupported(HotExclusion::Profile),
            true,
        ),
        (
            "test",
            HotCapabilities::Unsupported(HotExclusion::Test),
            true,
        ),
        ("full", HotCapabilities::Supported, false),
        ("last", HotCapabilities::Supported, true),
        (
            "custom",
            HotCapabilities::Unsupported(HotExclusion::CustomRunner),
            true,
        ),
    ] {
        app.runs.push(live(directory.path(), name, caps, reading));
    }
    until(&mut app, |app| {
        app.runs
            .iter()
            .filter(|run| run.session.is_some())
            .all(|run| run.screen().unwrap().contents().contains("ready"))
    });
    app.runs[2].session.as_mut().unwrap().stop_requested = Some(Instant::now());
    let full = app.runs[5].session.as_mut().unwrap();
    for _ in 0..32 {
        if full.send(&[b'x'; 4096]).is_err() {
            break;
        }
    }
    let mut rejected = false;
    for _ in 0..4096 {
        if full.send(b"x").is_err() {
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "fixture must fill the real nonblocking input queue"
    );
    let targets = app.batch_targets();
    assert_eq!(app.hot_eligible_count(), 3);
    let report = app.dispatch_hot_command(HotCommand::Restart, &targets);
    assert_eq!((report.requested, report.skipped, report.failed), (2, 5, 1));
    assert_eq!(
        report.details[5].outcome,
        TargetOutcome::Failed(SendFailure::InputRejected)
    );
    until(&mut app, |_| {
        log(&directory.path().join("first")) == "R" && log(&directory.path().join("last")) == "R"
    });
    for name in ["stopping", "profile", "test", "custom"] {
        assert_eq!(log(&directory.path().join(name)), "");
    }
    assert_eq!(app.runs.len(), 8);
}

#[test]
fn batch_hot_snapshots_resolve_stable_ids_after_reorder_and_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.runs.push(live(
        directory.path(),
        "second",
        HotCapabilities::Supported,
        true,
    ));
    until(&mut app, |app| {
        app.runs[1].screen().unwrap().contents().contains("ready")
    });
    let targets = app.batch_targets();
    app.runs.swap(0, 1);
    drop(app.runs.remove(0));
    app.runs.insert(
        0,
        live(
            directory.path(),
            "replacement",
            HotCapabilities::Supported,
            true,
        ),
    );
    until(&mut app, |app| {
        app.runs[0].screen().unwrap().contents().contains("ready")
    });
    let report = app.dispatch_hot_command(HotCommand::Reload, &targets);
    assert_eq!((report.requested, report.skipped, report.failed), (1, 0, 1));
    until(&mut app, |_| log(&directory.path().join("first")) == "r");
    assert_eq!(log(&directory.path().join("replacement")), "");
    assert_eq!(log(&directory.path().join("second")), "");
}

#[test]
fn batch_hot_actions_are_explicit_while_r_and_upper_r_remain_selected_only() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.runs.push(live(
        directory.path(),
        "second",
        HotCapabilities::Supported,
        true,
    ));
    app.active_run = 1;
    until(&mut app, |app| {
        app.runs[1].screen().unwrap().contents().contains("ready")
    });
    press(&mut app, KeyCode::Char('r'));
    press(&mut app, KeyCode::Char('R'));
    until(&mut app, |_| log(&directory.path().join("second")) == "rR");
    assert_eq!(log(&directory.path().join("first")), "");
    let actions = ui::available_actions(&app);
    for item in actions
        .iter()
        .filter(|item| matches!(item.action, ui::MouseAction::BatchHotCommand(_)))
    {
        assert_eq!(item.shortcut, "");
    }
    app.activate(ui::MouseAction::BatchHotCommand(HotCommand::Reload), AREA)
        .unwrap();
    until(&mut app, |_| {
        log(&directory.path().join("first")) == "r"
            && log(&directory.path().join("second")) == "rRr"
    });
    assert_eq!(app.active_run, 1);
    assert!(app.batch_report.is_some());
    app.request_hot_command(HotCommand::Restart); // No retry/reentrant dispatch while delivery results own input.
    press(&mut app, KeyCode::Enter);
    assert!(app.batch_report.is_none());
    assert_eq!(log(&directory.path().join("second")), "rRr");
}

#[test]
fn batch_hot_report_preserves_frozen_review_query_filter_manual_offsets_and_active_tab() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.runs.insert(0, ended(&app));
    app.active_run = 0;
    app.capture_review(false);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    app.review.as_mut().unwrap().set_query("needle");
    app.review
        .as_mut()
        .unwrap()
        .set_filter(SeverityFilter::Errors);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    press(&mut app, KeyCode::Enter);
    render(&mut app, 80, 24);
    press(&mut app, KeyCode::PageDown);
    render(&mut app, 80, 24);
    let view = app.review.as_ref().unwrap();
    let snapshot = view.snapshot().unwrap();
    let offsets = view.offsets();
    let source = view.source().unwrap().id;
    let query = view.query().to_owned();
    assert!(
        offsets.0 > 0,
        "fixture must preserve manual review away from its current match"
    );
    app.request_hot_command(HotCommand::Reload);
    render(&mut app, 80, 24);
    app.paste("must not edit frozen query");
    press(&mut app, KeyCode::Char('r'));
    press(&mut app, KeyCode::Down);
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
        AREA,
    )
    .unwrap();
    press(&mut app, KeyCode::Esc);
    render(&mut app, 80, 24);
    let view = app.review.as_ref().unwrap();
    assert_eq!(app.active_run, 0);
    assert_eq!(view.source().unwrap().id, source);
    assert_eq!(view.query(), query);
    assert_eq!(view.filter(), SeverityFilter::Errors);
    assert_eq!(view.offsets(), offsets);
    assert!(Arc::ptr_eq(&snapshot, &view.snapshot().unwrap()));
    assert!(app.review_notice.is_none());
    until(&mut app, |_| log(&directory.path().join("first")) == "r");
}

#[test]
fn batch_hot_modal_owns_live_and_query_cursor_input_and_keeps_global_quit_above_it() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());
    app.request_hot_command(HotCommand::Reload);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let view = app.batch_report.as_ref().unwrap();
    assert!(view.is_open());
    assert_eq!(view.offset(), 0);
    assert_eq!(view.close_region().width, 7);
    assert!(!terminal.backend().cursor_visible());
    assert!(app.console_ui_owned());
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    app.begin_text_selection(AREA);
    assert!(app.selection.is_none());
    app.key(Shortcut::Actions.key(), 80, 24).unwrap();
    assert!(!app.actions_open);
    for code in [KeyCode::Char('r'), KeyCode::Char('R'), KeyCode::Char('q')] {
        press(&mut app, code);
    }
    app.paste("modal paste");
    for (width, height, covered) in [(80, 24, true), (120, 40, false)] {
        render(&mut app, width, height);
        let area = Rect::new(0, 0, width, height);
        let event = (0..width)
            .find_map(|column| {
                let event = MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row: 1,
                    modifiers: KeyModifiers::NONE,
                };
                (ui::mouse_action(&app, area, event) == Some(ui::MouseAction::Quit))
                    .then_some(event)
            })
            .unwrap();
        assert_eq!(
            app.batch_report
                .as_ref()
                .unwrap()
                .covers((event.column, event.row).into()),
            covered
        );
        app.mouse(event, area).unwrap();
        assert_eq!(app.quit_confirmation, !covered);
        if app.quit_confirmation {
            press(&mut app, KeyCode::Esc);
        }
    }
    app.batch_report_event(&Event::Resize(80, 24));
    let event = (0..80)
        .find_map(|column| {
            let event = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: 1,
                modifiers: KeyModifiers::NONE,
            };
            (ui::mouse_action(&app, AREA, event) == Some(ui::MouseAction::Quit)).then_some(event)
        })
        .unwrap();
    app.mouse(event, AREA).unwrap();
    assert!(
        !app.quit_confirmation,
        "pending resize must not expose a covered Quit cell"
    );
    app.key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        80,
        24,
    )
    .unwrap();
    assert!(app.quit_confirmation);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal.backend().cursor_visible());
    press(&mut app, KeyCode::Esc);
    assert!(app.batch_report.is_some());
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('z'));
    until(&mut app, |_| log(&directory.path().join("first")) == "rz");
    app.capture_review(false);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.review.as_ref().unwrap().editing());
    assert!(terminal.backend().cursor_visible());
    app.request_hot_command(HotCommand::Restart);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal.backend().cursor_visible());
    app.paste("query must stay empty");
    press(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.review.as_ref().unwrap().editing());
    assert_eq!(app.review.as_ref().unwrap().query(), "");
    assert!(terminal.backend().cursor_visible());
}

#[test]
fn batch_hot_tiny_global_quit_is_visible_above_batch_export_and_operation_details() {
    let directory = tempfile::tempdir().unwrap();
    for modal in ["batch", "export", "details"] {
        for (width, height) in [(33, 12), (34, 11), (24, 10)] {
            let area = Rect::new(0, 0, width, height);
            let mut app = crate::tests::app();
            app.ownit = false;
            app.page = Page::Console;
            app.retain_consoles = true;
            Arc::make_mut(&mut app.launch).workspace = directory.path().to_owned();
            app.runs.push(ended(&app));
            match modal {
                "batch" => app.request_hot_command(HotCommand::Reload),
                "export" => {
                    app.open_export();
                    until(&mut app, |app| {
                        app.pending_export
                            .as_ref()
                            .and_then(|pending| pending.dialog.as_ref())
                            .is_some_and(|dialog| dialog.snapshot().is_some())
                    });
                }
                _ => {
                    app.last_operation = Some("Retained operation details".into());
                    app.activate(ui::MouseAction::OperationDetails, area)
                        .unwrap();
                }
            }
            let initial = render(&mut app, width, height);
            assert!(!text(&initial).contains("Quit Flimru?"));
            let click = |column, row| MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            };
            let hidden_quit = click(1, height - 1);
            assert_eq!(
                ui::mouse_action(&app, area, hidden_quit),
                Some(ui::MouseAction::Quit)
            );
            assert!(!app.mouse(hidden_quit, area).unwrap());
            assert!(!app.quit_confirmation, "{modal} must consume obscured Quit");
            for cancel in [KeyCode::Char('n'), KeyCode::Esc] {
                let (open_width, open_height) = if cancel == KeyCode::Esc {
                    render(&mut app, 80, 24);
                    (80, 24)
                } else {
                    (width, height)
                };
                assert!(
                    !app.key(
                        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                        open_width,
                        open_height
                    )
                    .unwrap()
                );
                if cancel == KeyCode::Esc {
                    assert!(text(&render(&mut app, 80, 24)).contains("Quit Flimru?"));
                    app.batch_report_event(&Event::Resize(width, height));
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                assert!(!terminal.backend().cursor_visible());
                let buffer = terminal.backend().buffer();
                let rendered = text(buffer);
                assert!(rendered.contains("Quit Flimru?"), "{modal}: {rendered}");
                assert!(rendered.contains("Enter/Y Quit"), "{modal}: {rendered}");
                assert!(rendered.contains("Esc/N Cancel"), "{modal}: {rendered}");
                // Every actionable confirmation/cancellation cell must display
                // the exact corresponding button, never a blank modal layer.
                for (action, expected) in [
                    (ui::MouseAction::ConfirmQuit, "[Quit]"),
                    (ui::MouseAction::CancelQuit, "[Cancel]"),
                ] {
                    let mut cells = String::new();
                    for row in 0..height {
                        for column in 0..width {
                            if ui::mouse_action(&app, area, click(column, row)) == Some(action) {
                                cells.push_str(buffer[(column, row)].symbol());
                            }
                        }
                    }
                    assert_eq!(cells, expected, "{modal} {width}x{height}");
                }
                if let Some(destination) = std::env::var_os("FLIMRU_P5_APP_FIX_EVIDENCE_DIR") {
                    write_evidence(
                        Path::new(&destination),
                        &format!("tiny-quit-{modal}-{width}x{height}"),
                        buffer,
                    );
                }
                assert!(
                    !app.key(KeyEvent::new(cancel, KeyModifiers::NONE), width, height)
                        .unwrap()
                );
                assert!(!app.quit_confirmation);
                assert_eq!(render(&mut app, width, height), initial);
            }
            app.key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                width,
                height,
            )
            .unwrap();
            render(&mut app, width, height);
            assert!(!app.mouse(click(9, height - 1), area).unwrap());
            assert!(!app.quit_confirmation);
            assert_eq!(render(&mut app, width, height), initial);
            app.key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                width,
                height,
            )
            .unwrap();
            render(&mut app, width, height);
            assert!(
                app.key(
                    KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                    width,
                    height
                )
                .unwrap()
            );
            assert!(app.mouse(hidden_quit, area).unwrap());
        }
    }
    assert!(!directory.path().join("flimru-logs").exists());
}

fn write_evidence(directory: &Path, stem: &str, buffer: &Buffer) {
    let color = |value: Color| match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Yellow => "#ffff00".into(),
        Color::White => "#ffffff".into(),
        _ => "#dce3ed".into(),
    };
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let (width, height) = (buffer.area.width, buffer.area.height);
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(directory.join(format!("{stem}.txt")), text(buffer) + "\n").unwrap();
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><rect width="100%" height="100%" fill="#101722"/>"##,
        width * 10,
        height * 20,
        width * 10,
        height * 20
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            if cell.bg != Color::Reset {
                svg.push_str(&format!(
                    r#"<rect x="{}" y="{}" width="10" height="20" fill="{}"/>"#,
                    x * 10,
                    y * 20,
                    color(cell.bg)
                ));
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            if cell.symbol() != " " && !cell.symbol().is_empty() {
                svg.push_str(&format!(r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{}>{}</text>"#,x*10,y*20+16,color(cell.fg),if cell.modifier.contains(Modifier::BOLD){" font-weight=\"bold\""}else{""},escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

#[test]
fn batch_hot_actual_app_render_evidence_preserves_zero_and_mixed_eligible_counts() {
    let directory = tempfile::tempdir().unwrap();
    let destination = std::env::var_os("FLIMRU_P5_APP_EVIDENCE_DIR");
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        let mut app = fixture(directory.path());
        app.runs.push(ended(&app));
        let mut profile = ended(&app);
        profile.hot_capabilities = HotCapabilities::Unsupported(HotExclusion::Profile);
        app.runs.push(profile);
        for (state, zero) in [("actions-mixed", false), ("actions-zero", true)] {
            if zero {
                drop(app.runs.remove(0));
                app.active_run = 0;
            }
            app.actions_open = true;
            app.action_index = ui::available_actions(&app)
                .iter()
                .position(|item| {
                    item.action == ui::MouseAction::BatchHotCommand(HotCommand::Restart)
                })
                .unwrap();
            ui::update_action_viewport(&mut app, Rect::new(0, 0, width, height));
            let buffer = render(&mut app, width, height);
            let rendered = text(&buffer);
            assert!(
                rendered.contains(&format!("Restart all eligible ({})", usize::from(!zero))),
                "{rendered}"
            );
            assert!(rendered.contains("Hot restart"), "{rendered}");
            if let Some(destination) = &destination {
                write_evidence(
                    Path::new(destination),
                    &format!("{state}-{width}x{height}"),
                    &buffer,
                );
            }
        }
        app.actions_open = false;
        app.runs.push(live(
            directory.path(),
            &format!("report-{width}"),
            HotCapabilities::Supported,
            true,
        ));
        until(&mut app, |app| {
            app.runs
                .last()
                .unwrap()
                .screen()
                .unwrap()
                .contents()
                .contains("ready")
        });
        app.capture_review(false);
        until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
        app.review.as_mut().unwrap().set_query("needle");
        until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
        let frozen = app.review.as_ref().unwrap().snapshot().unwrap();
        app.request_hot_command(HotCommand::Reload);
        let buffer = render(&mut app, width, height);
        assert!(text(&buffer).contains("Hot reload delivery report"));
        assert!(Arc::ptr_eq(
            &frozen,
            &app.review.as_ref().unwrap().snapshot().unwrap()
        ));
        if let Some(destination) = &destination {
            write_evidence(
                Path::new(destination),
                &format!("report-over-review-{width}x{height}"),
                &buffer,
            );
        }
    }
}
