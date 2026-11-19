#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use flimru::console_review::SeverityFilter;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};
use std::path::Path;

const AREA: Rect = Rect::new(0, 0, 80, 24);
fn press(app: &mut App, code: KeyCode) {
    assert!(
        !app.key(KeyEvent::new(code, KeyModifiers::NONE), 80, 24)
            .unwrap()
    );
}
fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.review.as_ref().is_some_and(|view| view.is_pending()) {
        app.tick().unwrap();
        assert!(Instant::now() < deadline, "review did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        app.review.as_ref().unwrap().error().is_none(),
        "{:?}",
        app.review.as_ref().unwrap().error()
    );
}
fn ended(text: &str, rows: u16, cols: u16) -> runs::Run {
    let mut run = runs::Run::failed(
        Device {
            id: "fixture".into(),
            name: "Pixel 界".into(),
            platform: "android".into(),
            available: true,
            favorite: false,
        },
        "development".into(),
        "fixture".into(),
        rows,
        cols,
    );
    let mut parser = vt100::Parser::new(rows, cols, 100);
    parser.process(text.as_bytes());
    parser.screen_mut().set_scrollback(usize::MAX);
    run.scrollback_len = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(0);
    run.history_capacity = 100;
    run.console = Some(parser);
    run.outcome = Some("exited successfully".into());
    run
}
fn fixture(text: &str) -> App {
    let mut app = crate::tests::app();
    app.page = Page::Console;
    app.runs.push(ended(text, 24, 120));
    app
}

#[test]
fn console_review_animation_shortcut_ignores_hidden_live_sessions_in_both_pickers() {
    for page in [Page::Devices, Page::Configs] {
        let mut app = crate::tests::app();
        let session = Session::start(
            &flimru::tool::RunCommand {
                capture_vm_service: false,
                hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), "exec sleep 30".into()],
                cwd: std::env::temp_dir(),
                env: Default::default(),
            },
            5,
            70,
        )
        .unwrap();
        app.runs.push(runs::Run::running(
            app.devices[0].clone(),
            "hidden live session".into(),
            session,
            flimru::batch_commands::HotCapabilities::Supported,
        ));
        app.page = page;
        app.status = "picker status".into();
        let animation = app.runs[0].session.as_ref().unwrap().animations.state();
        app.key(Shortcut::Animations.key(), 80, 24).unwrap();
        assert_eq!(app.page, page);
        assert_eq!(
            app.status, "picker status",
            "a hidden child must not receive animation commands"
        );
        let session = app.runs[0].session.as_ref().unwrap();
        assert_eq!(session.animations.state(), animation);
        assert!(session.stop_requested.is_none());
    }
}
fn open(app: &mut App) {
    app.key(Shortcut::SearchConsole.key(), 80, 24).unwrap();
    assert!(app.reviewing());
    settle(app);
}
fn buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw(frame))
        .unwrap()
        .buffer
        .clone()
}
fn fix_render(app: &mut App, stem: &str) -> Buffer {
    let rendered = buffer(app, 34, 12);
    if let Some(output) = std::env::var_os("FLIMRU_P3_FIX_EVIDENCE_DIR") {
        evidence(Path::new(&output), stem, &rendered);
    }
    rendered
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
fn console_review_capture_recovery_and_help_describe_the_actual_mode() {
    let mut rejected = fixture("old output");
    rejected.runs[0].history_capacity = Session::SCROLLBACK_CAPACITY;
    rejected.runs[0].max_columns = 500;
    rejected.runs[0]
        .console
        .as_mut()
        .unwrap()
        .process(b"\x1b[?1049h");
    rejected.capture_review(false);
    assert!(
        rejected
            .review
            .as_ref()
            .unwrap()
            .error()
            .unwrap()
            .contains("Start a new session in a narrower terminal")
    );
    let shown = text(&fix_render(&mut rejected, "capture-recovery-34x12"));
    for instruction in ["128 MiB", "new session", "narrower", "history stays wide"] {
        assert!(shown.contains(instruction), "{instruction}: {shown}");
    }
    let mut app = fixture("ERROR needle");
    open(&mut app);
    app.review.as_mut().unwrap().set_query("needle");
    settle(&mut app);
    for mode in ["editing", "reading", "source-details"] {
        if mode == "reading" {
            press(&mut app, KeyCode::Enter);
        }
        if mode == "source-details" {
            app.activate(ui::MouseAction::SnapshotSource, AREA).unwrap();
        }
        app.help = true;
        let shown = text(&fix_render(&mut app, &format!("help-{mode}-34x12")));
        assert!(shown.contains("Process controls: Actions"), "{shown}");
        match mode {
            "editing" => {
                assert!(shown.contains("Query editing"));
                assert!(shown.contains("Esc: stop editing"));
                assert!(!shown.contains("r refresh"));
            }
            "reading" => assert!(shown.contains("r refresh · Esc resume")),
            _ => {
                assert!(shown.contains("Esc / Enter / i: back"));
                assert!(!shown.contains("r refresh"));
            }
        }
        press(&mut app, KeyCode::Esc); // Closes only Help.
        press(&mut app, KeyCode::Char('r'));
        if mode == "editing" {
            assert_eq!(app.review.as_ref().unwrap().query(), "needler");
            settle(&mut app);
        } else if mode == "reading" {
            settle(&mut app);
        } else {
            assert!(app.review.as_ref().unwrap().showing_source_details());
        }
    }
}

#[test]
fn console_review_operation_details_preserve_review_and_remain_reachable_after_resume() {
    let message = "Hot reload: Input queue is full; wait for Flutter to read input";
    let mut app = fixture("ERROR needle");
    open(&mut app);
    app.review.as_mut().unwrap().set_query("needle");
    settle(&mut app);
    press(&mut app, KeyCode::Enter);
    let source = app.reviewed_source();
    let frozen = app.review.as_ref().unwrap().snapshot().unwrap();
    app.report_operation(message.into());
    let notice = fix_render(&mut app, "operation-notice-34x12");
    assert!(text(&notice).contains("Ctrl+p details:"));
    let position = app.review.as_ref().unwrap().offsets();
    let current = app.review.as_ref().unwrap().current_match();
    // The advertised keyboard path selects the explicit action.
    app.key(Shortcut::Actions.key(), 34, 12).unwrap();
    app.action_index = ui::available_actions(&app)
        .iter()
        .position(|item| item.action == ui::MouseAction::OperationDetails)
        .unwrap();
    press(&mut app, KeyCode::Enter);
    assert!(app.showing_operation_details());
    let top = text(&fix_render(&mut app, "operation-details-34x12"));
    assert!(top.contains("Latest operation outcome"));
    press(&mut app, KeyCode::End);
    let end = text(&fix_render(&mut app, "operation-details-end-34x12"));
    assert!(end.contains("wait for Flutter to read input"), "{end}");
    for ch in ['r', 'f', 'n'] {
        press(&mut app, KeyCode::Char(ch));
    }
    app.paste("unwanted input");
    assert_eq!(app.review.as_ref().unwrap().query(), "needle");
    assert_eq!(app.review.as_ref().unwrap().filter(), SeverityFilter::All);
    assert_eq!(app.review.as_ref().unwrap().current_match(), current);
    assert_eq!(app.review.as_ref().unwrap().offsets(), position);
    assert_eq!(app.reviewed_source(), source);
    assert!(Arc::ptr_eq(
        &frozen,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    press(&mut app, KeyCode::Esc);
    assert!(app.reviewing());
    assert!(!app.review.as_ref().unwrap().editing());
    app.activate(ui::MouseAction::ResumeLive, AREA).unwrap();
    assert!(!app.reviewing());
    assert_eq!(app.last_operation.as_deref(), Some(message));
    assert!(
        ui::available_actions(&app)
            .iter()
            .any(|item| item.action == ui::MouseAction::OperationDetails)
    );
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    assert!(app.showing_operation_details());
    assert!(!app.reviewing());
    assert_eq!(app.reviewed_source(), None);
    assert!(app.review.as_ref().unwrap().snapshot().is_none());
    let actions = ui::available_actions(&app);
    assert!(
        !actions
            .iter()
            .any(|item| item.action == ui::MouseAction::SelectText)
    );
    assert!(
        actions
            .iter()
            .filter(|item| item.action != ui::MouseAction::Quit)
            .all(|item| item.shortcut.is_empty())
    );
    app.actions_open = true;
    app.action_index = actions
        .iter()
        .position(|item| item.action == ui::MouseAction::SearchConsole)
        .unwrap();
    ui::update_action_viewport(&mut app, Rect::new(0, 0, 34, 12));
    let menu = text(&fix_render(&mut app, "details-only-actions-34x12"));
    assert!(menu.contains("Search console"));
    assert!(!menu.contains("Ctrl+s"));
    press(&mut app, KeyCode::Esc);
    app.help = true;
    let help = text(&fix_render(&mut app, "help-operation-details-34x12"));
    assert!(help.contains("Ctrl+p menu"));
    assert!(!help.contains("Ctrl+g top"));
    assert!(!help.contains("Ctrl+s"));
    press(&mut app, KeyCode::Esc);
    let shown = text(&fix_render(
        &mut app,
        "operation-details-after-resume-34x12",
    ));
    assert!(shown.contains("Latest operation outcome"));
    assert!(!shown.contains("Snapshot"));
    app.key(Shortcut::SearchConsole.key(), 34, 12).unwrap();
    app.paste("unwanted input");
    assert!(
        !app.reviewing(),
        "details must consume search/input without capturing"
    );
    app.key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        34,
        12,
    )
    .unwrap();
    assert!(
        app.quit_confirmation,
        "global quit stays available over details"
    );
    press(&mut app, KeyCode::Esc);
    assert!(!app.quit_confirmation);
    assert!(app.showing_operation_details());
    press(&mut app, KeyCode::Esc);
    assert!(!app.showing_operation_details());
    assert!(!app.reviewing());
    assert!(!text(&buffer(&mut app, 34, 12)).contains("Latest operation outcome"));
    app.key(Shortcut::SearchConsole.key(), 34, 12).unwrap();
    settle(&mut app);
    assert!(app.reviewing());
    assert_eq!(app.reviewed_source(), source);
    assert!(!Arc::ptr_eq(
        &frozen,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    // A result arriving after details opened changes underlying review data,
    // never the details kind or its operation message.
    app.review.as_mut().unwrap().set_query("needle");
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    settle(&mut app);
    assert!(app.showing_operation_details());
    app.report_operation("界".repeat(console_search::MAX_OPERATION_CHARS + 20));
    let bounded = app.last_operation.as_ref().unwrap();
    assert!(bounded.chars().count() <= console_search::MAX_OPERATION_CHARS);
    assert!(bounded.ends_with("… [shortened]"));
}

#[test]
#[cfg(unix)]
fn console_review_details_only_owns_live_selection_cursor_and_input_until_closed() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input");
    let mut app = crate::tests::app();
    app.page = Page::Console;
    app.runs.push(live(&input, "stty raw -echo; printf 'ready'; while :; do IFS= read -rsn1 key || continue; printf '%s' \"$key\" >> \"$INPUT_LOG\"; done"));
    drain_until(&mut app, |app| {
        app.runs[0].screen().unwrap().contents().contains("ready")
    });
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());
    let message = "Hot reload: Input queue is full; wait for Flutter to read input";
    app.report_operation(message.into());
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    assert!(app.console_ui_owned());
    assert!(!app.reviewing());
    assert_eq!(app.reviewed_source(), None);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(
        !terminal.backend().cursor_visible(),
        "covered live PTY cursor must stay hidden"
    );
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    assert!(
        app.selection.is_none(),
        "Ctrl+e must not create a hidden selection"
    );
    app.begin_text_selection(AREA);
    app.activate(ui::MouseAction::SelectText, AREA).unwrap();
    assert!(
        app.selection.is_none(),
        "direct selection helpers must honor the same owner"
    );
    for code in [KeyCode::Down, KeyCode::Right, KeyCode::Home, KeyCode::End] {
        press(&mut app, code);
    }
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT), 80, 24)
        .unwrap();
    assert!(app.selection.is_none());
    press(&mut app, KeyCode::Enter);
    assert!(!app.showing_operation_details());
    assert!(app.selection.is_none());
    assert_eq!(
        app.status, message,
        "details navigation must not request a clipboard copy"
    );
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(
        terminal.backend().cursor_visible(),
        "live cursor must return on close"
    );
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    fix_render(&mut app, "details-only-live-34x12");
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    press(&mut app, KeyCode::Esc);
    assert!(!app.console_ui_owned());
    assert!(
        app.runs[0]
            .session
            .as_ref()
            .unwrap()
            .stop_requested
            .is_none()
    );
    app.key(Shortcut::SelectText.key(), 80, 24).unwrap();
    assert!(
        app.selection
            .as_ref()
            .is_some_and(|selection| selection.keyboard)
    );
    press(&mut app, KeyCode::Esc);
    assert!(app.selection.is_none());
    press(&mut app, KeyCode::Char('v'));
    drain_until(&mut app, |_| log(&input).ends_with('v'));
    assert_eq!(
        log(&input),
        "v",
        "modal selection/navigation must not reach the child"
    );
}

#[test]
fn console_review_unicode_filter_refresh_resume_and_complete_action_inventory() {
    let mut app = fixture(
        "\x1b[32mI/flutter: [Orders] SEVERE: 界e\u{301} needle\x1b[0m\r\nW/Tag: warning needle\r\n  stack needle",
    );
    let source = app.runs[0].id;
    open(&mut app);
    assert_eq!(app.reviewed_source(), Some(source));
    app.paste("needle");
    settle(&mut app);
    let snapshot = app.review.as_ref().unwrap().snapshot().unwrap();
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .matches
            .matches
            .len(),
        3
    );
    app.activate(ui::MouseAction::SearchFilter(SeverityFilter::Errors), AREA)
        .unwrap();
    settle(&mut app);
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .matches
            .matches
            .len(),
        1
    );
    assert!(Arc::ptr_eq(
        &snapshot,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    assert!(snapshot.plain_text().contains("stack needle"));
    app.key(Shortcut::SearchConsole.key(), 80, 24).unwrap();
    assert!(Arc::ptr_eq(
        &snapshot,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));

    assert!(app.review.as_ref().unwrap().editing());
    let actions = ui::available_actions(&app);
    for action in [
        ui::MouseAction::EditSearchQuery,
        ui::MouseAction::NextSearchMatch,
        ui::MouseAction::PreviousSearchMatch,
        ui::MouseAction::CycleSearchFilter,
        ui::MouseAction::RefreshSnapshot,
        ui::MouseAction::SnapshotSource,
        ui::MouseAction::ResumeLive,
    ] {
        assert!(actions.iter().any(|entry| entry.action == action));
    }
    assert!(
        !actions
            .iter()
            .any(|entry| entry.action == ui::MouseAction::SelectText)
    );
    assert!(
        actions
            .iter()
            .filter(|entry| matches!(
                entry.action,
                ui::MouseAction::RefreshSnapshot
                    | ui::MouseAction::NextSearchMatch
                    | ui::MouseAction::SnapshotSource
            ))
            .all(|entry| entry.shortcut.is_empty())
    );
    let query = app.review.as_ref().unwrap().query().to_owned();
    app.activate(ui::MouseAction::NextSearchMatch, AREA)
        .unwrap();
    app.activate(ui::MouseAction::PreviousSearchMatch, AREA)
        .unwrap();
    app.activate(ui::MouseAction::CycleSearchFilter, AREA)
        .unwrap();
    settle(&mut app);
    assert_eq!(app.review.as_ref().unwrap().query(), query);
    assert_eq!(
        app.review.as_ref().unwrap().filter(),
        SeverityFilter::Warnings
    );
    app.activate(ui::MouseAction::EditSearchQuery, AREA)
        .unwrap();
    app.activate(ui::MouseAction::SnapshotSource, AREA).unwrap();
    assert!(app.review.as_ref().unwrap().showing_source_details());
    press(&mut app, KeyCode::Esc);
    assert!(app.review.as_ref().unwrap().editing());
    press(&mut app, KeyCode::Esc);
    assert!(!app.review.as_ref().unwrap().editing());
    let actions = ui::available_actions(&app);
    assert_eq!(
        actions
            .iter()
            .find(|entry| entry.action == ui::MouseAction::RefreshSnapshot)
            .unwrap()
            .shortcut,
        "r"
    );
    assert_eq!(
        actions
            .iter()
            .find(|entry| entry.action == ui::MouseAction::SnapshotSource)
            .unwrap()
            .shortcut,
        "i"
    );
    app.runs[0]
        .console
        .as_mut()
        .unwrap()
        .process(b"\r\nWARN: later needle");
    app.actions_open = true;
    press(&mut app, KeyCode::Char('r'));
    assert!(!app.actions_open);
    settle(&mut app);
    assert!(!Arc::ptr_eq(
        &snapshot,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    assert_eq!(app.review.as_ref().unwrap().query(), query);
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .matches
            .matches
            .len(),
        2
    );
    press(&mut app, KeyCode::Esc);
    assert!(!app.reviewing());
    assert_eq!(app.page, Page::Console);
    assert!(app.runs[0].outcome.is_some());
}

#[test]
fn console_review_run_ids_survive_neighbor_removal_and_clear_on_actual_replacement() {
    let mut app = fixture("old first");
    app.runs.push(ended("review second", 24, 80));
    app.runs.push(ended("third", 24, 80));
    app.switch_run(1);
    open(&mut app);
    let id = app.runs[1].id;
    let snapshot = app.review.as_ref().unwrap().snapshot().unwrap();
    app.close_ended_run(0);
    assert_eq!(app.active_run, 0);
    assert_eq!(app.runs[0].id, id);
    assert_eq!(app.reviewed_source(), Some(id));
    assert!(Arc::ptr_eq(
        &snapshot,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    app.switch_run(0);
    assert!(app.reviewing(), "same source switch is not replacement");
    app.switch_run(1);
    assert!(!app.reviewing());
    open(&mut app);
    let replaced = app.runs[1].id;
    app.close_ended_run(1);
    assert!(!app.reviewing());
    app.runs.push(ended("new at recycled index", 24, 80));
    assert_ne!(app.runs[1].id, replaced);
    app.switch_run(1);
    open(&mut app);
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap()
            .plain_text(),
        "new at recycled index"
    );
}

#[test]
fn console_review_preclone_bounds_accept_wide_normal_empty_and_reject_hidden_history() {
    let mut app = crate::tests::app();
    app.page = Page::Console;
    let mut run = ended("", 24, 500);
    run.history_capacity = Session::SCROLLBACK_CAPACITY;
    app.runs.push(run);
    open(&mut app);
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap()
            .plain_text(),
        ""
    );
    app.close_review();
    app.runs[0]
        .console
        .as_mut()
        .unwrap()
        .process(b"\x1b[?1049h");
    assert!(app.runs[0].screen().unwrap().alternate_screen());
    app.capture_review(false);
    assert!(app.reviewing());
    assert!(!app.review.as_ref().unwrap().is_pending());
    assert!(app.review.as_ref().unwrap().snapshot().is_none());
    assert!(
        app.review
            .as_ref()
            .unwrap()
            .error()
            .unwrap()
            .contains("128 MiB")
    );
}

#[test]
fn console_review_retained_width_and_saved_source_metadata_remain_independent_of_launch_catalog() {
    let mut app = crate::tests::app();
    let mut run = ended("abcdefghijklmnopqrstuvwx\r\nTAIL\r\nEND", 2, 12);
    let id = run.id;
    run.console.as_mut().unwrap().screen_mut().set_size(2, 4);
    app.last_log_source = Some(App::run_source(&run));
    app.last_log_max_columns = run.historical_columns();
    app.last_log_history_capacity = run.history_capacity;
    app.last_log_scrollback = run.scrollback_len;
    app.last_log = run.console.take();
    app.active_config = "future catalog name".into();
    app.page = Page::LastLog;
    open(&mut app);
    assert_eq!(app.reviewed_source(), Some(id));
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .source()
            .unwrap()
            .configuration
            .as_deref(),
        Some("development")
    );
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .source()
            .unwrap()
            .device
            .as_deref(),
        Some("Pixel 界")
    );
    assert_eq!(
        app.review
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap()
            .plain_text(),
        "abcdefghijklmnopqrstuvwx\nTAIL\nEND"
    );
    assert_eq!(
        app.review.as_ref().unwrap().snapshot().unwrap().columns(),
        12
    );
    app.close_last_log();
    assert!(!app.reviewing());
    assert!(app.last_log_source.is_none());
}

#[test]
fn console_review_global_overlays_top_focus_and_direct_commands_do_not_edit_query() {
    let mut app = fixture("ERROR needle");
    open(&mut app);
    app.paste("needle");
    settle(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(34, 12)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());
    app.key(Shortcut::FocusActions.key(), 34, 12).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal.backend().cursor_visible());
    app.key(Shortcut::Actions.key(), 34, 12).unwrap();
    let rendered = terminal
        .draw(|frame| app.draw(frame))
        .unwrap()
        .buffer
        .clone();
    assert!(text(&rendered).contains("Actions ·"));
    assert!(!terminal.backend().cursor_visible());
    app.action_index = ui::available_actions(&app)
        .iter()
        .position(|entry| entry.action == ui::MouseAction::EditSearchQuery)
        .unwrap();
    press(&mut app, KeyCode::Enter);
    assert!(app.top_focus.is_none());
    assert_eq!(app.review.as_ref().unwrap().query(), "needle");
    app.actions_open = true;
    app.key(Shortcut::SearchConsole.key(), 34, 12).unwrap();
    assert!(!app.actions_open);
    assert!(app.review.as_ref().unwrap().editing());
    assert_eq!(app.review.as_ref().unwrap().query(), "needle");
    app.help = true;
    let rendered = terminal
        .draw(|frame| app.draw(frame))
        .unwrap()
        .buffer
        .clone();
    assert!(text(&rendered).contains("Process controls: Actions"));
    assert!(!terminal.backend().cursor_visible());
    app.paste("not a query");
    assert_eq!(app.review.as_ref().unwrap().query(), "needle");
    press(&mut app, KeyCode::Esc);
    assert!(!app.help);
    app.key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        34,
        12,
    )
    .unwrap();
    assert!(app.quit_confirmation);
    let rendered = terminal
        .draw(|frame| app.draw(frame))
        .unwrap()
        .buffer
        .clone();
    assert!(text(&rendered).contains("Quit Flimru?"));
    assert!(!terminal.backend().cursor_visible());
    press(&mut app, KeyCode::Esc);
    assert!(!app.quit_confirmation);
    assert!(app.reviewing());
    assert!(app.review.as_ref().unwrap().editing());
}

#[cfg(unix)]
fn live(log: &Path, script: &str) -> runs::Run {
    let session = Session::start(
        &flimru::tool::RunCommand {
            capture_vm_service: false,
            hot_capabilities: flimru::batch_commands::HotCapabilities::Supported,
            executable: "/bin/bash".into(),
            args: vec!["-c".into(), script.into()],
            cwd: log.parent().unwrap().into(),
            env: std::collections::BTreeMap::from([(
                "INPUT_LOG".into(),
                log.to_string_lossy().into_owned(),
            )]),
        },
        15,
        120,
    )
    .unwrap();
    runs::Run::running(
        Device {
            id: log.to_string_lossy().into_owned(),
            name: "Live pixel".into(),
            platform: "android".into(),
            available: true,
            favorite: false,
        },
        "Frozen launch-time config".into(),
        session,
        flimru::batch_commands::HotCapabilities::Supported,
    )
}
#[cfg(unix)]
fn drain_until(app: &mut App, mut ready: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.tick().unwrap();
        if ready(app) {
            return;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[cfg(unix)]
fn log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
#[cfg(unix)]
fn console_review_blocks_pty_controls_selection_and_paste_but_explicit_actions_reach_only_chosen_child()
 {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    let script = "stty raw -echo; printf 'ERROR needle ready\\r\\n'; while :; do IFS= read -rsn1 key || continue; printf '%s' \"$key\" >> \"$INPUT_LOG\"; [ \"$key\" = q ] && exit 0; done";
    let mut app = crate::tests::app();
    app.page = Page::Console;
    app.runs.push(live(&first, script));
    app.runs.push(live(&second, script));
    app.retain_consoles = true;
    drain_until(&mut app, |app| {
        app.runs
            .iter()
            .all(|run| run.screen().unwrap().contents().contains("ready"))
    });
    open(&mut app);
    let frozen = app.review.as_ref().unwrap().snapshot().unwrap();
    for key in [
        KeyCode::Char('r'),
        KeyCode::Char('R'),
        KeyCode::Char('/'),
        KeyCode::Char('q'),
    ] {
        press(&mut app, key);
    }
    app.paste("pasted qRr");
    for ch in ['a', 'e', 'x', 'z'] {
        app.key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
            80,
            24,
        )
        .unwrap();
    }
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.mouse(
            MouseEvent {
                kind,
                column: 10,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
        )
        .unwrap();
    }
    app.activate(ui::MouseAction::SelectText, AREA).unwrap();
    assert!(app.selection.is_none());
    assert!(
        app.runs
            .iter()
            .all(|run| run.session.as_ref().unwrap().stop_requested.is_none())
    );
    assert_eq!(log(&first), "");
    assert_eq!(log(&second), "");
    let query = app.review.as_ref().unwrap().query().to_owned();
    for command in ['r', 'R'] {
        app.actions_open = true;
        let actions = ui::available_actions(&app);
        assert_eq!(
            actions
                .iter()
                .find(|item| item.action == ui::MouseAction::FlutterKey(command))
                .unwrap()
                .shortcut,
            ""
        );
        app.action_index = actions
            .iter()
            .position(|item| item.action == ui::MouseAction::FlutterKey(command))
            .unwrap();
        press(&mut app, KeyCode::Enter);
    }
    drain_until(&mut app, |_| log(&first) == "rR");
    assert_eq!(log(&second), "");
    assert_eq!(app.review.as_ref().unwrap().query(), query);
    assert!(Arc::ptr_eq(
        &frozen,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
    assert_eq!(app.review_notice.as_deref(), Some("Hot restart requested"));
    assert!(text(&buffer(&mut app, 34, 12)).contains("Hot restart requested"));
    press(&mut app, KeyCode::Enter);
    let before = log(&first);
    press(&mut app, KeyCode::Char('r'));
    settle(&mut app);
    assert_eq!(
        log(&first),
        before,
        "reading r refreshes snapshot, never Flutter"
    );
    press(&mut app, KeyCode::Esc);
    assert!(!app.reviewing());
    press(&mut app, KeyCode::Char('r'));
    drain_until(&mut app, |_| log(&first) == "rRr");
    app.activate(ui::MouseAction::OperationDetails, AREA)
        .unwrap();
    assert!(app.showing_operation_details());
    assert!(!app.reviewing());
    for ch in ['r', 'R', 'q'] {
        press(&mut app, KeyCode::Char(ch));
    }
    app.paste("unwanted details paste");
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 8,
            modifiers: KeyModifiers::NONE,
        },
        AREA,
    )
    .unwrap();
    assert!(
        app.runs[0]
            .session
            .as_ref()
            .unwrap()
            .stop_requested
            .is_none()
    );
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('v'));
    drain_until(&mut app, |_| log(&first).ends_with('v'));
    assert_eq!(
        log(&first),
        "rRrv",
        "details-only input must not reach the live child"
    );
    assert_eq!(log(&second), "");
    app.switch_run(1);
    press(&mut app, KeyCode::Char('R'));
    drain_until(&mut app, |_| log(&second) == "R");
}

#[test]
#[cfg(unix)]
fn console_review_flood_drains_hidden_and_active_sessions_without_output_only_redraw() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = crate::tests::app();
    app.page = Page::Console;
    for name in ["first", "second"] {
        app.runs.push(live(
            &directory.path().join(name),
            "while :; do printf 'ERROR flood needle\\r\\n'; sleep 0.002; done",
        ));
    }
    drain_until(&mut app, |app| {
        app.runs
            .iter()
            .all(|run| run.screen().unwrap().contents().contains("needle"))
    });
    open(&mut app);
    let snapshot = app.review.as_ref().unwrap().snapshot().unwrap();
    let revisions: Vec<_> = app
        .runs
        .iter()
        .map(|run| run.session.as_ref().unwrap().visual_revision)
        .collect();
    let mut output = false;
    for _ in 0..25 {
        let update = app.tick().unwrap();
        output |= update.output_received;
        assert!(
            !update.redraw,
            "raw child output must not invalidate a frozen view"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(output);
    for (run, revision) in app.runs.iter().zip(revisions) {
        assert!(run.session.as_ref().unwrap().visual_revision > revision);
    }
    assert!(Arc::ptr_eq(
        &snapshot,
        &app.review.as_ref().unwrap().snapshot().unwrap()
    ));
}

#[test]
#[cfg(unix)]
fn console_review_completion_retains_reviewed_source_and_normal_completion_transfers_id_and_width()
{
    let directory = tempfile::tempdir().unwrap();
    for reviewed in [false, true] {
        let mut app = crate::tests::app();
        app.page = Page::Console;
        app.runs.push(live(
            &directory.path().join("child"),
            "printf 'ERROR completion ready\\r\\n'; sleep 0.15; exit 0",
        ));
        let id = app.runs[0].id;
        app.runs[0]
            .session
            .as_mut()
            .unwrap()
            .resize(15, 180)
            .unwrap();
        app.runs[0]
            .session
            .as_mut()
            .unwrap()
            .resize(15, 77)
            .unwrap();
        assert_eq!(app.runs[0].historical_columns(), 180);
        drain_until(&mut app, |app| {
            app.runs
                .first()
                .is_some_and(|run| run.screen().unwrap().contents().contains("ready"))
        });
        if reviewed {
            open(&mut app);
        }
        drain_until(&mut app, |app| app.running_count() == 0);
        if reviewed {
            assert!(app.reviewing());
            assert_eq!(app.reviewed_source(), Some(id));
            assert_eq!(app.page, Page::Console);
            assert_eq!(app.runs[0].id, id);
            assert!(app.runs[0].session.is_none());
            assert!(app.last_log.is_none());
            assert!(
                app.review_notice
                    .as_deref()
                    .unwrap()
                    .starts_with("Console ended:")
            );
            assert_eq!(app.last_operation, app.review_notice);
            let frozen = app.review.as_ref().unwrap().snapshot().unwrap();
            app.capture_review(true);
            settle(&mut app);
            assert!(!Arc::ptr_eq(
                &frozen,
                &app.review.as_ref().unwrap().snapshot().unwrap()
            ));
            assert!(
                app.review
                    .as_ref()
                    .unwrap()
                    .source()
                    .unwrap()
                    .outcome
                    .as_deref()
                    .unwrap()
                    .contains("Success")
            );
        } else {
            assert!(app.runs.is_empty());
            assert_eq!(app.last_log_source.as_ref().unwrap().id, id);
            assert_eq!(app.last_log_max_columns, 180);
            assert_eq!(
                app.last_log_source
                    .as_ref()
                    .unwrap()
                    .configuration
                    .as_deref(),
                Some("Frozen launch-time config")
            );
            app.page = Page::LastLog;
            open(&mut app);
            assert_eq!(app.reviewed_source(), Some(id));
        }
    }
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
    std::fs::write(directory.join(format!("{stem}.txt")), text(buffer) + "\n").unwrap();
    let width = buffer.area.width;
    let height = buffer.area.height;
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
                    color(cell.bg, "#101722")
                ));
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            if cell.symbol() != " " && !cell.symbol().is_empty() {
                svg.push_str(&format!(r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{}>{}</text>"#,x*10,y*20+16,color(cell.fg,"#dce3ed"),if cell.modifier.contains(Modifier::BOLD){" font-weight=\"bold\""}else{""},escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

/// Actual integrated App cells, not placeholders. Retain via FLIMRU_P3_APP_EVIDENCE_DIR.
#[test]
fn console_review_integrated_render_evidence_with_global_layers_and_source_details() {
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        for state in [
            "editing",
            "reading",
            "no-matches",
            "empty-filter",
            "empty",
            "pending",
            "error",
            "wide-history",
            "source-details",
            "source-details-scrolled",
            "actions",
            "help",
            "quit",
            "operation-notice",
            "operation-error",
        ] {
            let mut app = fixture(if state == "empty" {
                ""
            } else {
                "I/flutter: [Orders] SEVERE: request failed 界e\u{301}\r\nW/Tag: retry scheduled\r\n  stack continuation\r\nINFO connected\r\nERROR: request failed again"
            });
            app.runs[0].device.name = format!(
                "Physical Pixel 界 {} last-device-suffix",
                "long device label ".repeat(8)
            );
            app.runs[0].config = format!(
                "Development 界 {} last-config-suffix",
                "configuration label ".repeat(8)
            );
            if state == "wide-history" {
                app.runs[0] = ended(
                    &format!("ERROR: {}界e\u{301} needle\r\nlive\r\nend", "a".repeat(130)),
                    2,
                    160,
                );
                app.runs[0]
                    .console
                    .as_mut()
                    .unwrap()
                    .screen_mut()
                    .set_size(2, 4);
            }
            if state == "error" {
                app.runs[0].history_capacity = Session::SCROLLBACK_CAPACITY;
                app.runs[0].max_columns = 500;
                app.runs[0]
                    .console
                    .as_mut()
                    .unwrap()
                    .process(b"\x1b[?1049h");
            }
            app.capture_review(false);
            if !matches!(state, "pending" | "error") {
                settle(&mut app);
                if !matches!(state, "empty" | "empty-filter") {
                    app.review
                        .as_mut()
                        .unwrap()
                        .set_query(if state == "no-matches" {
                            "missing"
                        } else if state == "wide-history" {
                            "\u{301}"
                        } else {
                            "request"
                        });
                    settle(&mut app);
                }
                if state == "empty-filter" {
                    app.review
                        .as_mut()
                        .unwrap()
                        .set_filter(SeverityFilter::Errors);
                    app.runs[0]
                        .console
                        .as_mut()
                        .unwrap()
                        .process(b"\x1b[2J\x1b[HINFO only");
                    app.capture_review(true);
                    settle(&mut app);
                }
                if state != "editing" {
                    press(&mut app, KeyCode::Enter);
                }
                match state {
                    "source-details" | "source-details-scrolled" => {
                        app.activate(
                            ui::MouseAction::SnapshotSource,
                            Rect::new(0, 0, width, height),
                        )
                        .unwrap();
                    }
                    "actions" => app.actions_open = true,
                    "help" => app.help = true,
                    "quit" => app.quit_confirmation = true,
                    "operation-notice" => app.report_operation("Hot restart requested".into()),
                    "operation-error" => app.report_operation(
                        "Hot reload: Input queue is full; wait for Flutter to read input".into(),
                    ),
                    _ => {}
                }
            }
            let area = Rect::new(0, 0, width, height);
            ui::update_top_viewport(&mut app, area);
            if app.actions_open {
                ui::update_action_viewport(&mut app, area);
            }
            if state == "source-details-scrolled" {
                buffer(&mut app, width, height);
                press(&mut app, KeyCode::End);
            }
            let rendered = buffer(&mut app, width, height);
            let shown = text(&rendered);
            assert!(shown.contains("Flimru"), "{state}: {shown}");
            match state {
                "source-details" | "source-details-scrolled" => {
                    assert!(shown.contains("Frozen source details"), "{shown}")
                }
                "actions" => assert!(shown.contains("Actions ·")),
                "help" => assert!(shown.contains("Process controls: Actions")),
                "quit" => assert!(shown.contains("Quit Flimru?")),
                "operation-notice" => assert!(shown.contains("Hot restart requested")),
                "operation-error" => {
                    assert!(shown.contains("Hot reload:"));
                    if width == 34 {
                        assert!(shown.contains("Ctrl+p details:"));
                    }
                }
                "pending" => assert!(shown.contains("Capturing")),
                "error" => assert!(shown.contains("128 MiB") || shown.contains("128")),
                "empty-filter" => assert!(shown.contains("No lines in this filter"), "{shown}"),
                "empty" => assert!(shown.contains("Snapshot is empty")),
                "no-matches" => assert!(shown.contains("No matches")),
                _ => assert!(shown.contains("Search")),
            }
            assert!(!shown.contains("Drag to copy"));
            if let Some(output) = std::env::var_os("FLIMRU_P3_APP_EVIDENCE_DIR") {
                evidence(
                    Path::new(&output),
                    &format!("{state}-{width}x{height}"),
                    &rendered,
                );
            }
        }
    }
}
