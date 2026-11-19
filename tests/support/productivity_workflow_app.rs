#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect, style::Color};

fn fixture(area: Rect) -> App {
    let mut app = crate::tests::app();
    app.ownit = false;
    app.page = Page::Configs;
    app.show_unavailable = true;
    app.selected_device = Some(app.devices[0].clone());
    let mut blocked =
        serde_json::json!({"name":"Blocked diagnostic source", "preLaunchTask":"build"});
    for index in 0..40 {
        blocked[format!("unknown{index:02}")] = true.into();
    }
    app.launch = Arc::new(
        LaunchFile::parse(
            &serde_json::json!({"configurations":[{"name":"Ready"},blocked]}).to_string(),
            &app.launch.path,
            &app.launch.workspace,
        )
        .unwrap(),
    );
    app.settle_availability();
    app.set_config_source_selection(Some(1));
    app.diagnostics_open = true;
    for _ in 0..3 {
        assert!(
            !app.key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                area.width,
                area.height
            )
            .unwrap()
        );
    }
    assert!(app.diagnostics_offset > 0);
    app
}

fn render(app: &mut App, area: Rect) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|col| buffer[(col, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn control(app: &App, area: Rect, action: ui::MouseAction) -> MouseEvent {
    (0..area.height)
        .flat_map(|row| (0..area.width).map(move |col| click(col, row)))
        .find(|event| ui::mouse_action(app, area, *event) == Some(action))
        .unwrap()
}

fn open_quit(app: &mut App, area: Rect) {
    assert!(
        !app.key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            area.width,
            area.height
        )
        .unwrap()
    );
    assert!(app.quit_confirmation && app.diagnostics_open);
}

#[test]
fn global_quit_keys_precede_diagnostics_and_restore_its_scroll_and_source() {
    for (width, height) in [(80, 24), (34, 12), (30, 10)] {
        let area = Rect::new(0, 0, width, height);
        let mut app = fixture(area);
        let initial = render(&mut app, area);
        let offset = app.diagnostics_offset;
        let source = app.selected_config_source();
        for cancel in [
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
        ] {
            open_quit(&mut app, area);
            let quit = render(&mut app, area);
            let rendered = text(&quit);
            assert!(rendered.contains("Quit Flimru?"));
            assert!(rendered.contains("Enter/Y Quit  Esc/N Cancel"));
            for key in [KeyCode::F(1), KeyCode::Down, KeyCode::Char('i')] {
                assert!(
                    !app.key(KeyEvent::new(key, KeyModifiers::NONE), width, height)
                        .unwrap()
                );
                assert!(app.quit_confirmation && app.diagnostics_open);
                assert_eq!(app.diagnostics_offset, offset);
            }
            assert!(!app.key(cancel, width, height).unwrap());
            assert!(!app.quit_confirmation && app.diagnostics_open);
            assert_eq!(app.selected_config_source(), source);
            assert_eq!(app.diagnostics_offset, offset);
            assert_eq!(render(&mut app, area), initial);
        }
        for confirm in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('Y')] {
            open_quit(&mut app, area);
            assert!(
                app.key(KeyEvent::new(confirm, KeyModifiers::NONE), width, height)
                    .unwrap()
            );
            app.quit_confirmation = false;
        }
        assert!(
            !app.key(
                KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
                width,
                height
            )
            .unwrap()
        );
        assert!(app.diagnostics_open);
        assert_eq!(app.diagnostics_offset, offset);
    }
}

#[test]
fn global_quit_pointer_precedes_diagnostics_without_activating_hidden_parent_controls() {
    for (width, height) in [(80, 24), (34, 12), (30, 10)] {
        let area = Rect::new(0, 0, width, height);
        let mut app = fixture(area);
        app.diagnostics_open = false;
        let parent_quit = control(&app, area, ui::MouseAction::Quit);
        app.diagnostics_open = true;
        assert!(!app.mouse(parent_quit, area).unwrap());
        assert!(!app.quit_confirmation, "hidden parent Quit activated");
        app.diagnostics_open = true;
        let initial = render(&mut app, area);
        let offset = app.diagnostics_offset;
        let source = app.selected_config_source();
        open_quit(&mut app, area);
        let quit = render(&mut app, area);
        if let Some(directory) = std::env::var_os("FLIMRU_P6_APP_EVIDENCE_DIR") {
            write_evidence(
                std::path::Path::new(&directory),
                &format!("diagnostics-global-quit-{width}x{height}"),
                &quit,
            );
        }
        for action in [ui::MouseAction::ConfirmQuit, ui::MouseAction::CancelQuit] {
            let event = control(&app, area, action);
            assert_ne!(quit[(event.column, event.row)].symbol(), " ");
        }
        assert!(!app.mouse(click(0, 0), area).unwrap());
        assert!(
            !app.mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    ..click(2, 2)
                },
                area
            )
            .unwrap()
        );
        assert!(app.quit_confirmation && app.diagnostics_open);
        assert_eq!(app.diagnostics_offset, offset);
        let cancel = control(&app, area, ui::MouseAction::CancelQuit);
        assert!(!app.mouse(cancel, area).unwrap());
        assert!(!app.quit_confirmation && app.diagnostics_open);
        assert_eq!(app.selected_config_source(), source);
        assert_eq!(app.diagnostics_offset, offset);
        assert_eq!(render(&mut app, area), initial);
        open_quit(&mut app, area);
        let confirm = control(&app, area, ui::MouseAction::ConfirmQuit);
        assert!(app.mouse(confirm, area).unwrap());
    }
}

#[test]
fn global_quit_confirmation_precedes_keyboard_selection_without_copying_or_dropping_it() {
    let area = Rect::new(0, 0, 80, 24);
    let mut app = crate::tests::app();
    let mut parser = vt100::Parser::new(2, 12, 0);
    parser.process(b"selection");
    let mut selection =
        selection::Selection::capture(parser.screen(), 0, Rect::new(0, 0, 12, 2), (0, 0).into())
            .unwrap();
    selection.keyboard = true;
    selection.move_end(4, 0, true);
    let original = selection.selected_text();
    app.selection = Some(selection);
    for cancel in [KeyCode::Esc, KeyCode::Char('n')] {
        assert!(
            !app.key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                area.width,
                area.height
            )
            .unwrap()
        );
        assert!(app.quit_confirmation);
        assert!(app.selection.is_some());
        assert!(
            !app.key(
                KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT),
                area.width,
                area.height
            )
            .unwrap()
        );
        assert_eq!(app.selection.as_mut().unwrap().selected_text(), original);
        assert!(
            !app.key(
                KeyEvent::new(cancel, KeyModifiers::NONE),
                area.width,
                area.height
            )
            .unwrap()
        );
        assert!(!app.quit_confirmation);
        assert_eq!(app.selection.as_mut().unwrap().selected_text(), original);
    }
}

#[test]
fn visible_quit_toolbar_remains_clickable_from_operation_details_after_resume() {
    for (width, height) in [(80, 24), (120, 40)] {
        let area = Rect::new(0, 0, width, height);
        let mut app = crate::tests::app();
        app.page = Page::Console;
        app.runs.push(runs::Run::failed(
            app.devices[0].clone(),
            "Original console".into(),
            "ended".into(),
            height,
            width,
        ));
        app.activate(ui::MouseAction::SearchConsole, area).unwrap();
        assert!(app.reviewing());
        app.activate(ui::MouseAction::ResumeLive, area).unwrap();
        assert!(!app.reviewing());
        app.report_operation("Hot reload outcome details".into());
        app.activate(ui::MouseAction::OperationDetails, area)
            .unwrap();
        assert!(app.showing_operation_details());
        let initial = render(&mut app, area);
        let actions = control(&app, area, ui::MouseAction::Actions);
        assert!(!app.mouse(actions, area).unwrap());
        assert!(!app.actions_open && app.showing_operation_details());
        let quit = control(&app, area, ui::MouseAction::Quit);
        assert!(!ui::review_area(area).contains((quit.column, quit.row).into()));
        assert!(text(&initial).contains("Quit"));
        assert!(!app.mouse(quit, area).unwrap());
        assert!(
            app.quit_confirmation,
            "visible Quit was consumed by operation details"
        );
        assert!(app.showing_operation_details());
        render(&mut app, area);
        let cancel = control(&app, area, ui::MouseAction::CancelQuit);
        assert!(!app.mouse(cancel, area).unwrap());
        assert!(!app.quit_confirmation && app.showing_operation_details());
        assert_eq!(render(&mut app, area), initial);
    }
}

fn write_evidence(directory: &std::path::Path, stem: &str, buffer: &Buffer) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(directory.join(format!("{stem}.txt")), text(buffer) + "\n").unwrap();
    let color = |value: Color| match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Yellow => "#ffff00".into(),
        _ => "#dce3ed".into(),
    };
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><rect width="100%" height="100%" fill="#0e131b"/>"##,
        buffer.area.width * 10,
        buffer.area.height * 20
    );
    for row in 0..buffer.area.height {
        for col in 0..buffer.area.width {
            let cell = &buffer[(col, row)];
            if cell.bg != Color::Reset {
                svg.push_str(&format!(
                    r#"<rect x="{}" y="{}" width="10" height="20" fill="{}"/>"#,
                    col * 10,
                    row * 20,
                    color(cell.bg)
                ));
            }
            if cell.symbol() != " " && !cell.symbol().is_empty() {
                svg.push_str(&format!(r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15">{}</text>"#,col*10,row*20+16,color(cell.fg),escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

const NAV_AREA: Rect = Rect::new(0, 0, 80, 24);

fn settle_review(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.review.as_ref().is_some_and(|view| view.is_pending()) {
        app.tick().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.review.as_ref().unwrap().error().is_none());
}

fn navigation_fixture(reviewed: bool) -> App {
    let mut app = crate::tests::app();
    app.page = Page::Console;
    for name in ["Original", "Destination"] {
        let mut run =
            runs::Run::failed(app.devices[0].clone(), name.into(), "ended".into(), 24, 80);
        let mut parser = vt100::Parser::new(24, 80, 100);
        for index in 0..80 {
            parser.process(format!("ERROR: needle {name} {index:02}\r\n").as_bytes());
        }
        parser.screen_mut().set_scrollback(usize::MAX);
        run.scrollback_len = parser.screen().scrollback();
        parser.screen_mut().set_scrollback(0);
        run.history_capacity = 100;
        run.console = Some(parser);
        app.runs.push(run);
    }
    if reviewed {
        app.search_console();
        settle_review(&mut app);
        app.review.as_mut().unwrap().set_query("needle");
        app.review
            .as_mut()
            .unwrap()
            .set_filter(flimru::console_review::SeverityFilter::Errors);
        settle_review(&mut app);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap();
        render(&mut app, NAV_AREA);
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert!(app.review.as_ref().unwrap().offsets().0 > 0);
    }
    app.report_operation("Retained operation result".into());
    app.activate(ui::MouseAction::OperationDetails, NAV_AREA)
        .unwrap();
    assert!(app.showing_operation_details());
    app
}

fn menu_action(app: &mut App, action: ui::MouseAction) {
    assert!(!app.key(Shortcut::Actions.key(), 80, 24).unwrap());
    assert!(app.actions_open);
    app.action_index = ui::available_actions(app)
        .iter()
        .position(|item| item.action == action)
        .unwrap();
    render(app, NAV_AREA);
    assert!(
        !app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 80, 24)
            .unwrap()
    );
    assert!(!app.actions_open);
}

#[test]
fn explicit_actions_navigate_from_details_only_and_details_over_review() {
    for reviewed in [false, true] {
        for action in [
            ui::MouseAction::Devices,
            ui::MouseAction::AddSession,
            ui::MouseAction::PreviousConsole,
            ui::MouseAction::NextConsole,
            ui::MouseAction::SwitchConsole(1),
            ui::MouseAction::CloseConsole(0),
        ] {
            let mut app = navigation_fixture(reviewed);
            let initial = render(&mut app, NAV_AREA);
            let destination_id = app.runs[1].id;
            menu_action(&mut app, action);
            assert!(!app.showing_operation_details(), "{action:?}");
            assert!(
                !app.reviewing(),
                "departed source retained review: {action:?}"
            );
            match action {
                ui::MouseAction::Devices | ui::MouseAction::AddSession => {
                    assert_eq!(app.page, Page::Devices);
                    assert_eq!(app.runs.len(), 2);
                    if action == ui::MouseAction::AddSession {
                        assert!(app.status.starts_with("Add session"));
                    }
                }
                _ => {
                    assert_eq!(app.page, Page::Console);
                    assert_eq!(app.runs[app.active_run].id, destination_id);
                    assert_eq!(
                        app.runs.len(),
                        if action == ui::MouseAction::CloseConsole(0) {
                            1
                        } else {
                            2
                        }
                    );
                }
            }
            assert_ne!(render(&mut app, NAV_AREA), initial, "{action:?}");
        }
    }
}

#[test]
fn same_source_explicit_navigation_dismisses_only_details_and_preserves_frozen_review() {
    for close_other in [false, true] {
        let mut app = navigation_fixture(true);
        let view = app.review.as_ref().unwrap();
        let snapshot = view.snapshot().unwrap();
        let offsets = view.offsets();
        let current = view.current_match();
        if close_other {
            app.activate(ui::MouseAction::CloseConsole(1), NAV_AREA)
                .unwrap();
            assert_eq!(app.runs.len(), 1);
        } else {
            menu_action(&mut app, ui::MouseAction::SwitchConsole(0));
        }
        assert!(app.reviewing() && !app.showing_operation_details());
        let view = app.review.as_ref().unwrap();
        assert_eq!(view.query(), "needle");
        assert_eq!(
            view.filter(),
            flimru::console_review::SeverityFilter::Errors
        );
        assert_eq!(view.offsets(), offsets);
        assert_eq!(view.current_match(), current);
        assert!(Arc::ptr_eq(&snapshot, &view.snapshot().unwrap()));
        let screen = text(&render(&mut app, NAV_AREA));
        assert!(!screen.contains("Latest operation outcome"));
        assert!(screen.contains("needle Original"));
    }
}

#[test]
fn ordinary_navigation_keys_and_overlay_cancellation_keep_operation_details_owned() {
    for reviewed in [false, true] {
        let mut app = navigation_fixture(reviewed);
        let initial = render(&mut app, NAV_AREA);
        for shortcut in [
            Shortcut::Devices,
            Shortcut::AddSession,
            Shortcut::PreviousConsole,
            Shortcut::NextConsole,
            Shortcut::CloseSession,
        ] {
            app.key(shortcut.key(), 80, 24).unwrap();
            assert!(app.showing_operation_details());
            assert_eq!(app.page, Page::Console);
            assert_eq!(app.active_run, 0);
            assert_eq!(app.runs.len(), 2);
        }
        app.key(Shortcut::Actions.key(), 80, 24).unwrap();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert!(app.showing_operation_details() && !app.actions_open);
        menu_action(&mut app, ui::MouseAction::Help);
        assert!(app.showing_operation_details() && app.help);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert!(app.showing_operation_details() && !app.help);
        menu_action(&mut app, ui::MouseAction::Quit);
        assert!(app.showing_operation_details() && app.quit_confirmation);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 80, 24)
            .unwrap();
        assert!(app.showing_operation_details() && !app.quit_confirmation);
        assert_eq!(render(&mut app, NAV_AREA), initial);
    }
}

#[test]
fn explicit_picker_actions_leave_details_and_execute_their_visible_command() {
    for action in [
        ui::MouseAction::ToggleMulti,
        ui::MouseAction::ToggleFavorite(0),
        ui::MouseAction::NextOrRun,
        ui::MouseAction::Refresh,
    ] {
        let mut app = crate::tests::app();
        if action == ui::MouseAction::Refresh {
            while !matches!(
                app.discovery
                    .events
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
                DiscoveryEvent::Finished(_)
            ) {}
        }
        app.report_operation("Earlier operation on a different page".into());
        app.activate(ui::MouseAction::OperationDetails, NAV_AREA)
            .unwrap();
        menu_action(&mut app, action);
        assert!(!app.showing_operation_details());
        match action {
            ui::MouseAction::ToggleMulti => assert!(app.multi),
            ui::MouseAction::ToggleFavorite(_) => assert!(app.status.contains("favorites.")),
            ui::MouseAction::NextOrRun => assert_eq!(app.page, Page::Configs),
            ui::MouseAction::Refresh => {
                assert!(matches!(
                    app.discovery
                        .events
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap(),
                    DiscoveryEvent::Started
                ));
                assert!(matches!(
                    app.discovery
                        .events
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap(),
                    DiscoveryEvent::Finished(_)
                ));
            }
            _ => unreachable!(),
        }
        assert!(!text(&render(&mut app, NAV_AREA)).contains("Latest operation outcome"));
    }
    for action in [
        ui::MouseAction::ToggleUnavailable,
        ui::MouseAction::ExplainConfiguration,
        ui::MouseAction::Back,
    ] {
        let mut app = crate::tests::app();
        app.page = Page::Configs;
        app.selected_device = Some(app.devices[0].clone());
        app.settle_availability();
        app.report_operation("Earlier operation on a different page".into());
        app.activate(ui::MouseAction::OperationDetails, NAV_AREA)
            .unwrap();
        menu_action(&mut app, action);
        assert!(!app.showing_operation_details());
        match action {
            ui::MouseAction::ToggleUnavailable => assert!(app.show_unavailable),
            ui::MouseAction::ExplainConfiguration => assert!(app.diagnostics_open),
            ui::MouseAction::Back => assert_eq!(app.page, Page::Devices),
            _ => unreachable!(),
        }
        assert!(!text(&render(&mut app, NAV_AREA)).contains("Latest operation outcome"));
    }
}
