#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};
use std::path::Path;

const AREA: Rect = Rect::new(0, 0, 80, 24);

fn fixture(workspace: &Path, text: &str) -> App {
    let mut app = crate::tests::app();
    Arc::make_mut(&mut app.launch).workspace = workspace.to_path_buf();
    app.page = Page::Console;
    app.retain_consoles = true;
    let mut run = runs::Run::failed(
        app.devices[0].clone(),
        "frozen config".into(),
        "fixture".into(),
        24,
        80,
    );
    let mut parser = vt100::Parser::new(24, 80, 100);
    parser.process(text.as_bytes());
    run.console = Some(parser);
    run.outcome = Some("exited successfully".into());
    run.history_capacity = 100;
    app.runs.push(run);
    app
}
fn press(app: &mut App, key: KeyCode) {
    assert!(
        !app.key(KeyEvent::new(key, KeyModifiers::NONE), 80, 24)
            .unwrap()
    );
}
fn until(app: &mut App, mut ready: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(app) {
        app.tick().unwrap();
        assert!(
            Instant::now() < deadline,
            "App export did not settle: {}",
            app.status
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn settle_capture(app: &mut App) {
    until(app, |app| {
        app.pending_export.as_ref().unwrap().capture.is_none()
    });
    assert!(
        app.pending_export
            .as_ref()
            .unwrap()
            .dialog
            .as_ref()
            .unwrap()
            .snapshot()
            .is_some()
    );
}
fn destination(app: &App) -> PathBuf {
    app.pending_export
        .as_ref()
        .unwrap()
        .dialog
        .as_ref()
        .unwrap()
        .destination()
        .into()
}
fn buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame))
        .unwrap()
        .buffer
        .clone()
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
fn log_export_review_reuses_exact_arc_full_filter_scope_and_preserves_query_viewport() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(
        root.path(),
        "ERROR needle 界e\u{301}\r\nINFO hidden\r\nWARN retained",
    );
    app.capture_review(false);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    let view = app.review.as_mut().unwrap();
    view.set_query("needle");
    view.set_filter(SeverityFilter::Errors);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    let view = app.review.as_ref().unwrap();
    let frozen = view.snapshot().unwrap();
    let offsets = view.offsets();
    let source = view.source().unwrap().clone();
    app.activate(ui::MouseAction::ExportLog, AREA).unwrap();
    let pending = app.pending_export.as_ref().unwrap();
    assert_eq!(pending.source_id, source.id);
    assert!(pending.capture.is_none());
    assert!(Arc::ptr_eq(
        pending.dialog.as_ref().unwrap().snapshot().unwrap(),
        &frozen
    ));
    assert!(!root.path().join("flimru-logs").exists());
    let path = destination(&app);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.pending_export.is_none());
    let exported = std::fs::read_to_string(&path).unwrap();
    assert_eq!(exported.split_once("\n\n").unwrap().1, frozen.plain_text());
    assert!(exported.contains("INFO hidden\nWARN retained"));
    assert!(exported.contains("Configuration: frozen config"));
    assert!(
        exported.contains(
            &source
                .captured_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string()
        )
    );
    assert!(
        app.last_operation
            .as_ref()
            .unwrap()
            .contains(&std::fs::canonicalize(path).unwrap().display().to_string())
    );
    let view = app.review.as_ref().unwrap();
    assert_eq!(view.query(), "needle");
    assert_eq!(view.filter(), SeverityFilter::Errors);
    assert_eq!(view.offsets(), offsets);
    assert!(Arc::ptr_eq(&view.snapshot().unwrap(), &frozen));
    // Pending source review never triggers a replacement capture for export.
    app.close_review();
    app.capture_review(false);
    let captured_at = app.review.as_ref().unwrap().source().unwrap().captured_at;
    app.open_export();
    assert!(app.pending_export.is_none());
    assert!(app.status.contains("not ready"));
    assert_eq!(
        app.review.as_ref().unwrap().source().unwrap().captured_at,
        captured_at
    );
    app.close_review();
    app.runs[0].history_capacity = Session::SCROLLBACK_CAPACITY;
    app.runs[0].max_columns = 500;
    app.runs[0]
        .console
        .as_mut()
        .unwrap()
        .process(b"\x1b[?1049h");
    app.capture_review(false);
    assert!(app.review.as_ref().unwrap().error().is_some());
    app.open_export();
    assert!(app.pending_export.is_none());
    assert!(
        app.status
            .contains("Start a new session in a narrower terminal")
    );
    assert!(!app.status.contains("Wait for capture"));
    app.close_review();
    app.open_export();
    assert!(app.pending_export.as_ref().unwrap().capture.is_none());
    assert!(
        app.last_operation
            .as_ref()
            .unwrap()
            .contains("Start a new session in a narrower terminal")
    );
}

#[test]
fn log_export_outside_review_freezes_invocation_source_without_writes_or_double_submit() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path(), "old retained text");
    let source_id = app.runs[0].id;
    app.runs[0].offset = 3;
    app.open_export();
    assert!(!root.path().join("flimru-logs").exists());
    app.runs[0]
        .console
        .as_mut()
        .unwrap()
        .process(b"\r\nnew output");
    app.runs[0].config = "later renamed config".into();
    app.runs.push(runs::Run::failed(
        app.devices[1].clone(),
        "Other".into(),
        "unrelated".into(),
        24,
        80,
    ));
    app.switch_run(1);
    settle_capture(&mut app);
    assert_eq!(app.pending_export.as_ref().unwrap().source_id, source_id);
    assert_eq!(app.runs[0].offset, 3);
    assert!(!root.path().join("flimru-logs").exists());
    let path = destination(&app);
    press(&mut app, KeyCode::Enter);
    let pending = app.pending_export.as_ref().unwrap();
    assert!(pending.writer.is_running());
    assert!(pending.capture.is_none());
    press(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.pending_export.is_none());
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(saved.ends_with("\n\nold retained text"));
    assert!(saved.contains("Configuration: frozen config"));
    assert!(!saved.contains("new output"));
    assert!(!saved.contains("Other"));
    assert_eq!(
        std::fs::read_dir(root.path().join("flimru-logs"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(app.active_run, 1);
}

#[test]
fn log_export_error_retry_retains_source_and_reports_complete_outcome_details() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path(), "exact retained output");
    app.open_export();
    settle_capture(&mut app);
    let path = destination(&app);
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "existing").unwrap();
    press(&mut app, KeyCode::Enter);
    until(&mut app, |app| {
        !app.pending_export.as_ref().unwrap().writer.is_running()
    });
    assert!(app.status.contains("destination must not exist"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "existing");
    app.paste("-retry");
    let retry = destination(&app);
    press(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.pending_export.is_none());
    assert!(
        std::fs::read_to_string(&retry)
            .unwrap()
            .ends_with("\n\nexact retained output")
    );
    let message = app.last_operation.clone().unwrap();
    assert!(message.contains(&std::fs::canonicalize(retry).unwrap().display().to_string()));
    assert!(message.contains("bytes"));
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    assert!(app.showing_operation_details());
    assert!(text(&buffer(&mut app, 120, 40)).contains("Saved console log"));
}

#[test]
fn log_export_cancel_and_shutdown_acknowledge_publication_wins_and_close_source() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path(), "retained");
    app.open_export();
    press(&mut app, KeyCode::Esc);
    assert!(app.pending_export.is_none());
    assert!(!root.path().join("flimru-logs").exists());
    app.open_export();
    settle_capture(&mut app);
    let path = destination(&app);
    press(&mut app, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    press(&mut app, KeyCode::Esc);
    assert!(!app.export_dialog_open());
    assert!(
        app.pending_export.is_some(),
        "retain writer until its result is acknowledged"
    );
    until(&mut app, |app| app.pending_export.is_none());
    assert!(app.status.starts_with("Saved console log"));
    assert!(app.status.contains("bytes"));
    app.open_export();
    app.close_ended_run(0);
    assert!(app.pending_export.is_none());
    let mut app = fixture(root.path(), "shutdown");
    app.open_export();
    settle_capture(&mut app);
    let path = destination(&app);
    press(&mut app, KeyCode::Enter);
    let message = app.shutdown_export().unwrap();
    assert!(app.pending_export.is_none());
    if path.exists() {
        assert!(message.starts_with("Saved console log"));
    } else {
        assert!(message.contains("cancelled"));
    }
    assert!(
        !std::fs::read_dir(root.path().join("flimru-logs"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".partial"))
    );
}

#[test]
#[cfg(unix)]
fn log_export_live_modal_owns_input_selection_and_cursor_while_global_quit_remains_available() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("child-input");
    let mut app = fixture(root.path(), "unused");
    app.runs.clear();
    let session = Session::start(&flimru::tool::RunCommand {
        capture_vm_service: false,
            hot_capabilities: flimru::batch_commands::HotCapabilities::Supported, executable: "/bin/bash".into(),
        args: vec!["-c".into(), "stty raw -echo; printf 'ready'; while :; do IFS= read -rsn1 key || continue; printf '%s' \"$key\" >> \"$INPUT_LOG\"; done".into()],
        cwd: root.path().into(), env: std::collections::BTreeMap::from([("INPUT_LOG".into(), input.display().to_string())]),
    },24,80).unwrap();
    app.runs.push(runs::Run::running(
        app.devices[0].clone(),
        "live source".into(),
        session,
        flimru::batch_commands::HotCapabilities::Supported,
    ));
    until(&mut app, |app| {
        app.runs[0].screen().unwrap().contents().contains("ready")
    });
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());
    app.open_export();
    settle_capture(&mut app);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let destination_region = app
        .pending_export
        .as_ref()
        .unwrap()
        .dialog
        .as_ref()
        .unwrap()
        .regions()
        .destination;
    assert_eq!(
        terminal.get_cursor_position().unwrap().y,
        destination_region.y
    );
    assert!(app.console_ui_owned());
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    app.begin_text_selection(AREA);
    assert!(app.selection.is_none());
    app.key(Shortcut::Actions.key(), 80, 24).unwrap();
    assert!(!app.actions_open);
    for code in [
        KeyCode::Char('r'),
        KeyCode::Char('R'),
        KeyCode::Char('q'),
        KeyCode::Home,
        KeyCode::End,
    ] {
        press(&mut app, code);
    }
    app.paste("modal-only");
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
    app.key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        80,
        24,
    )
    .unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.quit_confirmation);
    assert!(!terminal.backend().cursor_visible());
    app.paste("quit overlay input");
    press(&mut app, KeyCode::Enter);
    assert!(!app.quit_confirmation);
    press(&mut app, KeyCode::Esc);
    assert!(!app.console_ui_owned());
    press(&mut app, KeyCode::Char('z'));
    until(&mut app, |_| {
        std::fs::read_to_string(&input).is_ok_and(|text| text.contains('z'))
    });
    assert_eq!(std::fs::read_to_string(input).unwrap(), "z");
    assert!(
        app.runs[0]
            .session
            .as_ref()
            .unwrap()
            .stop_requested
            .is_none()
    );
}

fn left_click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn action_cell(app: &App, area: Rect, action: ui::MouseAction) -> Option<MouseEvent> {
    (area.y..area.bottom()).find_map(|y| {
        (area.x..area.right())
            .map(|x| left_click(x, y))
            .find(|event| ui::mouse_action(app, area, *event) == Some(action))
    })
}

#[test]
fn log_export_only_visible_quit_opens_confirmation_and_cancel_retains_the_modal() {
    let root = tempfile::tempdir().unwrap();
    for (width, height, visible) in [
        (34, 12, false),
        (80, 24, true),
        (120, 40, true),
        (120, 12, true),
        (29, 11, false),
    ] {
        let area = Rect::new(0, 0, width, height);
        let mut app = fixture(root.path(), "frozen output");
        app.open_export();
        settle_capture(&mut app);
        let rendered = buffer(&mut app, width, height);
        let pending = app.pending_export.as_ref().unwrap();
        let dialog = pending.dialog.as_ref().unwrap();
        let frozen = Arc::clone(dialog.snapshot().unwrap());
        let original_path = destination(&app);
        let visible_quit = (area.y..area.bottom()).find_map(|y| {
            (area.x..area.right()).find_map(|x| {
                (rendered[(x, y)].symbol() == "Q"
                    && ui::mouse_action(&app, area, left_click(x, y))
                        == Some(ui::MouseAction::Quit)
                    && !dialog.covers((x, y).into()))
                .then_some(left_click(x, y))
            })
        });
        assert_eq!(visible_quit.is_some(), visible);
        let quit =
            visible_quit.unwrap_or_else(|| action_cell(&app, area, ui::MouseAction::Quit).unwrap());
        assert_eq!(!dialog.covers((quit.column, quit.row).into()), visible);
        // A non-Quit toolbar action stays blocked, even where it is visible.
        if let Some(help) = action_cell(&app, area, ui::MouseAction::Help) {
            assert!(!app.mouse(help, area).unwrap());
            assert!(!app.help);
            assert!(!app.actions_open);
        }
        assert!(!app.mouse(left_click(0, 0), area).unwrap());
        assert!(!app.mouse(quit, area).unwrap());
        assert_eq!(app.quit_confirmation, visible);
        assert!(app.export_dialog_open());
        assert_eq!(destination(&app), original_path);
        assert!(!root.path().join("flimru-logs").exists());
        if visible {
            let confirmation = buffer(&mut app, width, height);
            assert!(text(&confirmation).contains("Quit Flimru?"));
            app.paste("confirmation must not edit destination");
            let cancel = action_cell(&app, area, ui::MouseAction::CancelQuit).unwrap();
            assert!(!app.mouse(cancel, area).unwrap());
            assert!(!app.quit_confirmation);
            assert!(app.export_dialog_open());
            assert_eq!(destination(&app), original_path);
            assert!(Arc::ptr_eq(
                app.pending_export
                    .as_ref()
                    .unwrap()
                    .dialog
                    .as_ref()
                    .unwrap()
                    .snapshot()
                    .unwrap(),
                &frozen
            ));
            buffer(&mut app, width, height);
            assert!(!app.mouse(quit, area).unwrap());
            assert!(app.quit_confirmation);
            // Existing keyboard cancellation and explicit mouse confirmation
            // retain the normal global confirmation semantics.
            assert!(
                !app.key(
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    width,
                    height
                )
                .unwrap()
            );
            assert!(!app.quit_confirmation);
            assert!(!app.mouse(quit, area).unwrap());
            let confirm = action_cell(&app, area, ui::MouseAction::ConfirmQuit).unwrap();
            if let Some(directory) = std::env::var_os("FLIMRU_P4_APP_EVIDENCE_DIR") {
                evidence(
                    Path::new(&directory),
                    &format!("quit-confirmation-{width}x{height}"),
                    &buffer(&mut app, width, height),
                );
            }
            assert!(app.mouse(confirm, area).unwrap());
        }
    }
}

#[test]
fn log_export_cancel_notice_preserves_manual_review_away_from_current_match() {
    let root = tempfile::tempdir().unwrap();
    let log = std::iter::once("needle at beginning".to_owned())
        .chain((1..100).map(|row| format!("line {row:03}")))
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut app = fixture(root.path(), &log);
    app.capture_review(false);
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    app.review.as_mut().unwrap().set_query("needle");
    until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
    buffer(&mut app, 80, 24);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::End);
    buffer(&mut app, 80, 24);
    assert!(app.review_notice.is_none());
    let view = app.review.as_ref().unwrap();
    assert_eq!(view.current_match(), Some(0));
    let manual = view.offsets();
    assert!(manual.0 > 70);
    let frozen = view.snapshot().unwrap();
    app.open_export();
    buffer(&mut app, 80, 24);
    assert_eq!(app.review.as_ref().unwrap().offsets(), manual);
    press(&mut app, KeyCode::Esc);
    assert!(app.review_notice.is_some());
    let resumed = buffer(&mut app, 80, 24);
    let view = app.review.as_ref().unwrap();
    assert_eq!(view.offsets(), manual);
    assert_eq!(view.query(), "needle");
    assert!(Arc::ptr_eq(&view.snapshot().unwrap(), &frozen));
    if let Some(directory) = std::env::var_os("FLIMRU_P4_APP_EVIDENCE_DIR") {
        evidence(
            Path::new(&directory),
            "cancel-notice-manual-review-80x24",
            &resumed,
        );
    }
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.review.as_ref().unwrap().offsets().0, 0);
}

#[test]
fn log_export_retained_outcome_preserves_maximum_path_and_byte_count() {
    let mut app = crate::tests::app();
    let directories = vec!["x".repeat(128); 31].join("/");
    let path = format!("/{directories}/{}-tail.txt", "y".repeat(87));
    assert_eq!(path.len(), log_export::MAX_DESTINATION_BYTES);
    assert!(
        Path::new(&path).components().all(|component| component
            .as_os_str()
            .as_encoded_bytes()
            .len()
            <= 128)
    );
    // This is the real result formatter boundary, without relying on a host
    // filesystem supporting a 4096-byte path or component.
    let message = App::export_result_message(&Ok(SavedExport {
        path: path.clone().into(),
        bytes: usize::MAX,
    }));
    app.report_operation(message.clone());
    assert_eq!(app.last_operation.as_deref(), Some(message.as_str()));
    assert_eq!(app.status, message);
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    let view = app.review.as_mut().unwrap();
    let mut retained = String::new();
    for _ in 0..4 {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let rendered = terminal.backend().buffer();
        // Join the actual details viewport rows without border/padding cells;
        // normal terminal wrapping may split the filename or numeric count.
        for y in 1..rendered.area.height - 2 {
            let row: String = (1..rendered.area.width - 1)
                .map(|x| rendered[(x, y)].symbol())
                .collect();
            retained.push_str(row.trim_end());
        }
        view.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
    }
    assert!(retained.contains("tail.txt"));
    assert!(retained.contains(&format!("{} bytes", usize::MAX)));
    assert!(!retained.contains("[shortened]"));
    app.report_operation("x".repeat(console_search::MAX_OPERATION_CHARS + 1));
    let bounded = app.last_operation.as_ref().unwrap();
    assert!(bounded.chars().count() <= console_search::MAX_OPERATION_CHARS);
    assert!(bounded.ends_with("… [shortened]"));
}

#[test]
#[cfg(unix)]
fn log_export_shutdown_sanitizes_canonical_path_reached_through_clean_symlink() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("canonical-\x1b[31m-parent");
    std::fs::create_dir(&actual).unwrap();
    let clean = root.path().join("clean-parent");
    std::os::unix::fs::symlink(&actual, &clean).unwrap();
    let destination = clean.join("saved.txt");
    let mut app = fixture(root.path(), "exact retained output");
    app.open_export();
    settle_capture(&mut app);
    app.key(
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        80,
        24,
    )
    .unwrap();
    app.paste(destination.to_str().unwrap());
    press(&mut app, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !destination.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let message = app.shutdown_export().unwrap();
    assert!(message.starts_with("Saved console log"));
    assert!(message.contains("canonical-[31m-parent"));
    assert!(message.contains("saved.txt"));
    assert!(message.ends_with(" bytes)."));
    assert!(!message.chars().any(char::is_control));
    assert_eq!(app.last_operation.as_deref(), Some(message.as_str()));
    assert_eq!(app.status, message);
    assert!(
        std::fs::read_to_string(destination)
            .unwrap()
            .ends_with("\n\nexact retained output")
    );
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(value: Color, fallback: &str) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Yellow => "#ffff00".into(),
        _ => fallback.into(),
    }
}
fn evidence(directory: &Path, stem: &str, buffer: &Buffer) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(
        directory.join(format!("{stem}.txt")),
        format!("{}\n", text(buffer)),
    )
    .unwrap();
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><rect width="100%" height="100%" fill="#0e131b"/>"##,
        buffer.area.width * 10,
        buffer.area.height * 20
    );
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.bg != Color::Reset {
                svg.push_str(&format!(
                    r#"<rect x="{}" y="{}" width="10" height="20" fill="{}"/>"#,
                    x * 10,
                    y * 20,
                    color(cell.bg, "#0e131b")
                ));
            }
        }
    }
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol() != " " && !cell.symbol().is_empty() {
                svg.push_str(&format!(r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{}>{}</text>"#,x*10,y*20+16,color(cell.fg,"#e0e8f0"),if cell.modifier.contains(Modifier::BOLD) {" font-weight=\"bold\""}else{""},escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

#[test]
fn log_export_actual_app_render_evidence_ready_error_and_review() {
    let root = tempfile::tempdir().unwrap();
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        for state in ["ready", "error", "review"] {
            let mut app = fixture(
                root.path(),
                "ERROR needle 界e\u{301}\r\nINFO retained despite filter",
            );
            if state == "review" {
                app.capture_review(false);
                until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
                app.review
                    .as_mut()
                    .unwrap()
                    .set_filter(SeverityFilter::Errors);
                until(&mut app, |app| !app.review.as_ref().unwrap().is_pending());
            }
            app.activate(ui::MouseAction::ExportLog, AREA).unwrap();
            settle_capture(&mut app);
            if state == "error" {
                app.pending_export.as_mut().unwrap().dialog.as_mut().unwrap().set_error("Destination exists. Choose another filename; existing logs are never overwritten.");
            }
            let rendered = buffer(&mut app, width, height);
            let text = text(&rendered);
            assert!(text.contains("Export retained console log"));
            assert!(text.contains("query/filter-hidden lines."));
            assert!(text.contains("[ Export ]"));
            if let Some(directory) = std::env::var_os("FLIMRU_P4_APP_EVIDENCE_DIR") {
                evidence(
                    Path::new(&directory),
                    &format!("{state}-{width}x{height}"),
                    &rendered,
                );
            }
        }
    }
}
