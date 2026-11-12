#![allow(clippy::unwrap_used)]
use flimru::{console_snapshot, log_export};
#[path = "../src/export_dialog.rs"]
mod export_dialog;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use export_dialog::{DialogAction, ExportDialog};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    widgets::Paragraph,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn snapshot() -> Arc<console_snapshot::ConsoleSnapshot> {
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process("ERROR 界 e\u{301}\r\nINFO hidden by filter".as_bytes());
    Arc::new(
        console_snapshot::ConsoleSnapshot::capture(
            parser.screen().clone(),
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
}
fn dialog(ready: bool) -> ExportDialog {
    ExportDialog::new(
        "Phone 界 · Debug · running",
        log_export::ExportMetadata::new(
            1799000000,
            Some("Phone 界"),
            Some("Debug"),
            Some("running"),
        ),
        Path::new("/workspace/flimru-logs/phone-debug.txt"),
        ready.then(snapshot),
    )
}
fn key(dialog: &mut ExportDialog, code: KeyCode) -> DialogAction {
    dialog.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}
fn render(dialog: &mut ExportDialog, width: u16, height: u16) -> Buffer {
    render_with_cursor(dialog, width, height).0
}
fn render_with_cursor(
    dialog: &mut ExportDialog,
    width: u16,
    height: u16,
) -> (Buffer, ratatui::layout::Position) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Paragraph::new("LIVE FLUTTER CONSOLE · other tabs keep running"),
                frame.area(),
            );
            dialog.render(frame);
        })
        .unwrap();
    (
        terminal.backend().buffer().clone(),
        terminal.get_cursor_position().unwrap(),
    )
}
fn path_dialog(path: &Path, ready: bool) -> ExportDialog {
    ExportDialog::new(
        "Frozen source",
        log_export::ExportMetadata::new(42, Some("Phone"), Some("Debug"), None),
        path,
        ready.then(snapshot),
    )
}
fn row_text(buffer: &Buffer, region: ratatui::layout::Rect) -> String {
    (region.x..region.right())
        .map(|x| buffer[(x, region.y)].symbol())
        .collect()
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
fn click(dialog: &mut ExportDialog, x: u16, y: u16) -> DialogAction {
    dialog.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }))
}

#[test]
fn requires_ready_explicit_submit_and_blocks_double_submission() {
    let mut dialog = dialog(false);
    let initial = render(&mut dialog, 34, 12);
    assert!(text(&initial).contains("Preparing snapshot"));
    assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
    let regions = dialog.regions();
    assert_eq!(
        click(&mut dialog, regions.export.x, regions.export.y),
        DialogAction::Consumed
    );
    let snap = snapshot();
    dialog.set_snapshot(Arc::clone(&snap));
    assert!(Arc::ptr_eq(dialog.snapshot().unwrap(), &snap));
    let metadata_address = std::ptr::from_ref(dialog.metadata());
    let ready = render(&mut dialog, 34, 12);
    assert!(text(&ready).contains("Ready"));
    assert!(text(&ready).contains("query/filter-hidden lines."));
    assert_eq!(
        key(&mut dialog, KeyCode::Enter),
        DialogAction::Submit("/workspace/flimru-logs/phone-debug.txt".into())
    );
    assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
    assert_eq!(
        dialog.handle_event(&Event::Paste("evil".into())),
        DialogAction::Consumed
    );
    assert!(!dialog.destination().contains("evil"));
    assert_eq!(std::ptr::from_ref(dialog.metadata()), metadata_address);
    assert_eq!(key(&mut dialog, KeyCode::Esc), DialogAction::Cancel);
}

#[test]
fn error_is_editable_retryable_and_keeps_frozen_source() {
    let mut dialog = dialog(true);
    let snap = Arc::clone(dialog.snapshot().unwrap());
    assert!(matches!(
        key(&mut dialog, KeyCode::Enter),
        DialogAction::Submit(_)
    ));
    dialog.set_error("destination already exists; choose another filename");
    assert!(text(&render(&mut dialog, 80, 24)).contains("destination already exists"));
    key(&mut dialog, KeyCode::End);
    dialog.handle_event(&Event::Paste("-new".into()));
    assert!(
        matches!(key(&mut dialog, KeyCode::Enter), DialogAction::Submit(path) if path.to_string_lossy().ends_with(".txt-new"))
    );
    assert!(Arc::ptr_eq(dialog.snapshot().unwrap(), &snap));
}

fn clear_destination(dialog: &mut ExportDialog) {
    dialog.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
}

#[test]
fn empty_destination_has_shared_disabled_status_and_submission_gate() {
    for (width, height) in [(30, 12), (34, 12), (80, 24), (120, 40)] {
        for remove in [KeyCode::Backspace, KeyCode::Delete] {
            let mut dialog = path_dialog(Path::new("界"), true);
            if remove == KeyCode::Delete {
                key(&mut dialog, KeyCode::Home);
            }
            key(&mut dialog, remove);
            let buffer = render(&mut dialog, width, height);
            assert_eq!(dialog.destination(), "");
            assert!(text(&buffer).contains("Enter a destination."));
            let region = dialog.regions().export;
            assert_eq!(buffer[(region.x, region.y)].fg, Color::Rgb(151, 163, 180));
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
        }
        for initial in ["", "/tmp/log.txt", "/tmp/bad\npath"] {
            let mut dialog = path_dialog(Path::new(initial), true);
            if !initial.is_empty() {
                clear_destination(&mut dialog);
            }
            let buffer = render(&mut dialog, width, height);
            let rendered = text(&buffer);
            assert!(rendered.contains("Enter a destination."));
            assert!(rendered.contains("Edit path · Esc Cancel"));
            assert!(!rendered.contains("Ready ·"));
            assert!(!rendered.contains("Enter Export"));
            let regions = dialog.regions();
            assert_eq!(
                buffer[(regions.export.x, regions.export.y)].fg,
                Color::Rgb(151, 163, 180)
            );
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            assert_eq!(
                click(&mut dialog, regions.export.x, regions.export.y),
                DialogAction::Consumed
            );
            key(&mut dialog, KeyCode::Tab);
            let focused = render(&mut dialog, width, height);
            assert_eq!(
                focused[(regions.export.x, regions.export.y)].fg,
                Color::Rgb(151, 163, 180)
            );
            assert!(text(&focused).contains("Edit path · Esc Cancel"));
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            key(&mut dialog, KeyCode::BackTab);
            let exact = "/tmp/chosen-界.txt";
            dialog.handle_event(&Event::Paste(exact.into()));
            let corrected = render(&mut dialog, width, height);
            assert!(text(&corrected).contains("Ready ·"));
            assert!(text(&corrected).contains("Enter Export; Esc Cancel"));
            assert_eq!(
                corrected[(regions.export.x, regions.export.y)].fg,
                Color::Rgb(96, 210, 225)
            );
            assert_eq!(
                key(&mut dialog, KeyCode::Enter),
                DialogAction::Submit(exact.into())
            );
        }
    }
}

#[test]
fn enter_guidance_matches_focus_capture_and_saving_state() {
    for (width, height) in [(30, 12), (34, 12), (80, 24), (120, 40)] {
        for state in ["ready", "preparing", "saving", "empty", "invalid", "retry"] {
            let mut dialog = dialog(state != "preparing");
            let expected = match state {
                "preparing" => "Preparing · Esc Cancel",
                "saving" => {
                    key(&mut dialog, KeyCode::Enter);
                    "Saving · Esc Cancel"
                }
                "empty" => {
                    clear_destination(&mut dialog);
                    "Edit path · Esc Cancel"
                }
                "invalid" => {
                    dialog.handle_event(&Event::Paste("bad\npath".into()));
                    "Edit path · Esc Cancel"
                }
                "retry" => {
                    dialog.set_error("Choose another filename.");
                    "Enter Export; Esc Cancel"
                }
                _ => "Enter Export; Esc Cancel",
            };
            for focus in ["destination", "export", "cancel"] {
                let buffer = render(&mut dialog, width, height);
                let rendered = text(&buffer);
                let hint = if focus == "cancel" {
                    "Enter Cancel; Esc Cancel"
                } else {
                    expected
                };
                assert!(
                    rendered.contains(hint),
                    "{state} {focus} {width}: {rendered}"
                );
                if state == "saving" {
                    assert!(!rendered.contains("Home End edit"));
                }
                if focus == "cancel" {
                    assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Cancel);
                } else if matches!(state, "preparing" | "saving" | "empty" | "invalid") {
                    assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
                }
                key(&mut dialog, KeyCode::Tab);
            }
            if state == "preparing" {
                clear_destination(&mut dialog);
                let empty = text(&render(&mut dialog, width, height));
                assert!(empty.contains("Preparing snapshot…"));
                assert!(empty.contains("Enter a destination."));
                dialog.set_snapshot(snapshot());
                let captured = text(&render(&mut dialog, width, height));
                assert!(captured.contains("Enter a destination."));
                assert!(!captured.contains("Preparing snapshot"));
                assert!(!captured.contains("Ready ·"));
            }
        }
        // Both enabled activation contexts agree with their Export hint.
        for export_focus in [false, true] {
            let mut ready = dialog(true);
            if export_focus {
                key(&mut ready, KeyCode::Tab);
            }
            assert!(text(&render(&mut ready, width, height)).contains("Enter Export; Esc Cancel"));
            assert!(matches!(
                key(&mut ready, KeyCode::Enter),
                DialogAction::Submit(_)
            ));
        }
    }
}

#[test]
fn utf8_cursor_editing_navigation_and_paste_are_bounded() {
    let mut dialog = dialog(true);
    dialog.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    dialog.handle_event(&Event::Paste("a界e\u{301}.txt".into()));
    key(&mut dialog, KeyCode::Home);
    key(&mut dialog, KeyCode::Right);
    key(&mut dialog, KeyCode::Delete);
    assert_eq!(dialog.destination(), "ae\u{301}.txt");
    key(&mut dialog, KeyCode::Right);
    key(&mut dialog, KeyCode::Backspace);
    assert_eq!(dialog.destination(), "a\u{301}.txt");
    key(&mut dialog, KeyCode::End);
    key(&mut dialog, KeyCode::Char('界'));
    assert_eq!(dialog.destination(), "a\u{301}.txt界");
    let before = dialog.destination().to_owned();
    for paste in [
        "bad\npath".to_owned(),
        "bad\x1b[31m".to_owned(),
        "x".repeat(4097),
    ] {
        assert_eq!(
            dialog.handle_event(&Event::Paste(paste)),
            DialogAction::Consumed
        );
        assert_eq!(dialog.destination(), before);
    }
    assert_eq!(key(&mut dialog, KeyCode::Esc), DialogAction::Cancel);
}

#[test]
fn preparing_capture_can_finish_after_invalid_edit_and_tiny_layout_cannot_submit() {
    let mut dialog = dialog(false);
    dialog.handle_event(&Event::Paste("bad\npath".into()));
    dialog.set_snapshot(snapshot());
    assert!(text(&render(&mut dialog, 34, 12)).contains("Error: Path has controls."));
    assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
    for (width, height) in [(1, 1), (12, 4), (29, 12), (34, 11)] {
        render(&mut dialog, width, height);
        assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
        let regions = dialog.regions();
        assert_eq!(regions.export.width, 0);
        assert_eq!(key(&mut dialog, KeyCode::Esc), DialogAction::Cancel);
    }
    render(&mut dialog, 34, 12);
    key(&mut dialog, KeyCode::Char('x'));
    assert!(matches!(
        key(&mut dialog, KeyCode::Enter),
        DialogAction::Submit(_)
    ));
}

#[test]
fn unsupported_initial_paths_never_submit_a_rewritten_filename() {
    let mut unsupported = vec![
        PathBuf::from("/tmp/name\nwith-control.txt"),
        PathBuf::from("/tmp/name\0with-control.txt"),
        PathBuf::from(format!("/tmp/{}.txt", "界".repeat(1400))),
    ];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        unsupported.push(std::ffi::OsString::from_vec(b"/tmp/invalid-\xff.txt".to_vec()).into());
    }
    for path in unsupported {
        for ready in [false, true] {
            let mut dialog = path_dialog(&path, ready);
            assert_eq!(dialog.destination(), "");
            assert!(text(&render(&mut dialog, 80, 24)).contains("Error: Initial path"));
            if !ready {
                dialog.set_snapshot(snapshot());
            }
            assert!(text(&render(&mut dialog, 80, 24)).contains("Error: Initial path"));
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            let regions = dialog.regions();
            assert_eq!(
                click(&mut dialog, regions.export.x, regions.export.y),
                DialogAction::Consumed
            );
            // Neither navigation nor an empty paste is a correction.
            key(&mut dialog, KeyCode::End);
            dialog.handle_event(&Event::Paste(String::new()));
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            let exact = "/tmp/ deliberately-chosen-👩🏽‍💻-e\u{301}.txt ";
            dialog.handle_event(&Event::Paste(exact.into()));
            assert_eq!(
                key(&mut dialog, KeyCode::Enter),
                DialogAction::Submit(exact.into())
            );
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
        }
    }
}

#[test]
fn accepted_initial_paths_submit_exactly_including_byte_limit_and_unicode() {
    for path in [
        "relative folder/./../log.txt".to_owned(),
        "/tmp/ space-界-👩🏽‍💻-e\u{301}.txt ".to_owned(),
        format!("/{}", "x".repeat(log_export::MAX_DESTINATION_BYTES - 1)),
    ] {
        let mut dialog = path_dialog(Path::new(&path), true);
        render(&mut dialog, 34, 12);
        assert_eq!(dialog.destination(), path);
        assert_eq!(
            key(&mut dialog, KeyCode::Enter),
            DialogAction::Submit(path.into())
        );
    }
}

#[test]
fn byte_limit_keeps_joined_unicode_intact_during_editing_and_rejection() {
    let path = format!("/{}abc", "👩‍💻".repeat(372));
    assert_eq!(path.len(), log_export::MAX_DESTINATION_BYTES);
    let mut dialog = path_dialog(Path::new(&path), true);
    // Exercise bounded scans with the maximum accepted multibyte destination,
    // including scalar cursor positions within joined graphemes.
    for _ in 0..64 {
        key(&mut dialog, KeyCode::Left);
        let (_, cursor) = render_with_cursor(&mut dialog, 34, 12);
        assert!(dialog.regions().destination.contains(cursor));
    }
    key(&mut dialog, KeyCode::End);
    for paste in ["界", "x", "bad\npath"] {
        dialog.handle_event(&Event::Paste(paste.into()));
        assert_eq!(dialog.destination(), path);
        assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
    }
    key(&mut dialog, KeyCode::Backspace);
    key(&mut dialog, KeyCode::Char('x'));
    let corrected = format!("{}x", &path[..path.len() - 1]);
    assert_eq!(dialog.destination(), corrected);
    assert_eq!(
        key(&mut dialog, KeyCode::Enter),
        DialogAction::Submit(corrected.into())
    );
}

#[test]
fn rejected_paste_blocks_old_destination_until_deliberate_edit() {
    for paste in [
        "bad\npath".to_owned(),
        "bad\rpath".to_owned(),
        "\x1b[31m".to_owned(),
        "界".repeat(1366),
    ] {
        let mut dialog = dialog(false);
        let original = dialog.destination().to_owned();
        let metadata = format!("{:?}", dialog.metadata());
        dialog.handle_event(&Event::Paste(paste));
        dialog.set_snapshot(snapshot());
        let snap = Arc::clone(dialog.snapshot().unwrap());
        let buffer = render(&mut dialog, 80, 24);
        assert!(text(&buffer).contains("Error: Path"));
        assert_eq!(dialog.destination(), original);
        assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
        dialog.set_error("worker error does not validate rejected input");
        for event in [Event::Resize(80, 24), Event::FocusLost, Event::FocusGained] {
            assert_eq!(dialog.handle_event(&event), DialogAction::Consumed);
        }
        assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
        key(&mut dialog, KeyCode::End);
        key(&mut dialog, KeyCode::Backspace);
        assert_eq!(
            key(&mut dialog, KeyCode::Enter),
            DialogAction::Submit(original[..original.len() - 1].into())
        );
        assert_eq!(format!("{:?}", dialog.metadata()), metadata);
        assert!(Arc::ptr_eq(dialog.snapshot().unwrap(), &snap));
    }
}

#[test]
fn grapheme_scrolling_shows_suffix_and_places_end_cursor_after_it() {
    for cluster in ["👩‍💻", "👩🏽‍💻", "👍🏽", "e\u{301}", "界"] {
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let path = format!("/{}{}/tail.txt", cluster.repeat(45), "界");
            let mut dialog = path_dialog(Path::new(&path), true);
            let (buffer, cursor) = render_with_cursor(&mut dialog, width, height);
            let region = dialog.regions().destination;
            let row = row_text(&buffer, region);
            assert!(row.contains("/tail.txt"), "{cluster} {width}: {row}");
            assert_eq!(buffer[(cursor.x - 1, cursor.y)].symbol(), "t");
            assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), " ");
            assert!(region.contains(cursor));
            // Navigation through Unicode scalars inside a grapheme still uses
            // whole rendered clusters for window start/cursor placement.
            for _ in 0..12 {
                key(&mut dialog, KeyCode::Left);
                let (buffer, cursor) = render_with_cursor(&mut dialog, width, height);
                assert!(dialog.regions().destination.contains(cursor));
                let row = row_text(&buffer, dialog.regions().destination);
                assert!(!row.starts_with("‹\u{200d}"));
                assert!(!row.starts_with("‹\u{301}"));
                assert!(!row.starts_with("‹🏽"));
            }
            key(&mut dialog, KeyCode::Home);
            key(&mut dialog, KeyCode::Right);
            let (buffer, cursor) = render_with_cursor(&mut dialog, width, height);
            assert_eq!(cursor.x, region.x + 1);
            assert_eq!(buffer[(region.x, region.y)].symbol(), "/");
            key(&mut dialog, KeyCode::End);
            assert_eq!(dialog.destination(), path);
            assert_eq!(
                key(&mut dialog, KeyCode::Enter),
                DialogAction::Submit(path.into())
            );
        }
    }
}

#[test]
fn exact_reported_zwj_path_keeps_tail_visible_at_compact_size() {
    let path = format!("/{}/tail.txt", "👩‍💻".repeat(20));
    let mut dialog = path_dialog(Path::new(&path), true);
    let (buffer, cursor) = render_with_cursor(&mut dialog, 34, 12);
    let region = dialog.regions().destination;
    assert!(row_text(&buffer, region).contains("/tail.txt"));
    assert_eq!(buffer[(cursor.x - 1, cursor.y)].symbol(), "t");
    assert!(region.contains(cursor));
}

#[test]
fn scalar_cursor_inside_grapheme_uses_its_visible_start() {
    for cluster in ["👩‍💻", "👩🏽‍💻", "👍🏽", "e\u{301}", "界"] {
        let path = format!("/{}tail.txt", cluster.repeat(40));
        let mut dialog = path_dialog(Path::new(&path), true);
        key(&mut dialog, KeyCode::Home);
        key(&mut dialog, KeyCode::Right);
        for _ in 0..cluster.chars().count() {
            let (buffer, cursor) = render_with_cursor(&mut dialog, 34, 12);
            let region = dialog.regions().destination;
            assert_eq!(cursor.x, region.x + 1, "{cluster}");
            assert_eq!(buffer[(cursor.x, cursor.y)].symbol(), cluster);
            key(&mut dialog, KeyCode::Right);
        }
        let (_, cursor) = render_with_cursor(&mut dialog, 34, 12);
        let expected = ratatui::text::Span::raw(cluster).width() as u16;
        assert_eq!(cursor.x, dialog.regions().destination.x + 1 + expected);
        assert_eq!(dialog.destination(), path);
    }
}

#[test]
fn tiny_layouts_and_every_background_mouse_coordinate_are_consumed() {
    let mut dialog = dialog(true);
    for width in 0..40 {
        for height in 0..15 {
            if width >= 30 && height >= 12 {
                continue;
            }
            render(&mut dialog, width, height);
            assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            assert_eq!(click(&mut dialog, 0, 0), DialogAction::Consumed);
            assert_eq!(dialog.regions().destination.width, 0);
            assert_eq!(dialog.regions().export.width, 0);
            assert_eq!(dialog.regions().cancel.width, 0);
        }
    }
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        render(&mut dialog, width, height);
        let regions = dialog.regions();
        for y in 0..height {
            for x in 0..width {
                if !regions.export.contains((x, y).into())
                    && !regions.cancel.contains((x, y).into())
                {
                    assert_eq!(click(&mut dialog, x, y), DialogAction::Consumed);
                }
            }
        }
        assert_eq!(
            click(&mut dialog, regions.export.x, regions.export.y),
            DialogAction::Submit(dialog.destination().into())
        );
        dialog.set_error("retry after mouse test");
    }
}

#[test]
fn rendered_button_regions_and_focus_work_at_all_requested_sizes() {
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        let mut dialog = dialog(true);
        assert!(
            dialog.covers((0, 0).into()),
            "unrendered modal blocks background actions"
        );
        let buffer = render(&mut dialog, width, height);
        let regions = dialog.regions();
        assert!(!dialog.covers((0, 0).into()));
        assert!(dialog.covers((regions.destination.x, regions.destination.y).into()));
        assert!(dialog.covers((regions.export.x, regions.export.y).into()));
        assert_eq!(buffer[(regions.export.x, regions.export.y)].symbol(), "[");
        assert_eq!(buffer[(regions.cancel.x, regions.cancel.y)].symbol(), "[");
        assert!(regions.cancel.right() <= width);
        assert_eq!(click(&mut dialog, 0, 0), DialogAction::Consumed);
        assert_eq!(
            dialog.handle_event(&Event::Resize(width, height)),
            DialogAction::Consumed
        );
        assert_eq!(
            click(&mut dialog, regions.destination.x, regions.destination.y),
            DialogAction::Consumed
        );
        key(&mut dialog, KeyCode::Tab);
        key(&mut dialog, KeyCode::Tab);
        assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Cancel);
        key(&mut dialog, KeyCode::BackTab);
        assert!(matches!(
            key(&mut dialog, KeyCode::Enter),
            DialogAction::Submit(_)
        ));
        dialog.set_error("retry");
        assert_eq!(
            click(&mut dialog, regions.cancel.x, regions.cancel.y),
            DialogAction::Cancel
        );
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
fn svg(buffer: &Buffer) -> String {
    let mut output = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><rect width="100%" height="100%" fill="#0e131b"/>"##,
        buffer.area.width * 10,
        buffer.area.height * 20
    );
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.bg != Color::Reset {
                output.push_str(&format!(
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
                let font_weight = if cell.modifier.contains(Modifier::BOLD) {
                    " font-weight=\"bold\""
                } else {
                    ""
                };
                output.push_str(&format!(
                    r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{font_weight}>{}</text>"#,
                    x * 10,
                    y * 20 + 16,
                    color(cell.fg, "#e0e8f0"),
                    escape(cell.symbol())
                ));
            }
        }
    }
    output.push_str("</svg>");
    output
}

/// Actual caller-sized TestBackend cells, optionally retained as TXT/SVG.
#[test]
fn render_evidence_all_states_and_unicode_paths() {
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        for state in [
            "ready",
            "preparing",
            "saving",
            "error",
            "long-path",
            "zwj-path",
            "modified-path",
            "invalid-initial",
            "oversize-initial",
            "rejected-paste",
            "empty-destination",
            "export-focus",
            "cancel-focus",
            "disabled-export-focus",
            "empty-export-focus",
            "saving-cancel-focus",
            "preparing-empty",
            "corrected-destination",
        ] {
            let mut dialog = dialog(!matches!(
                state,
                "preparing" | "disabled-export-focus" | "preparing-empty"
            ));
            match state {
                "saving" => {
                    key(&mut dialog, KeyCode::Enter);
                }
                "error" => dialog.set_error("Destination already exists. Choose another filename; existing files are never overwritten."),
                "long-path" => {
                    let source = "Long 界 e\u{301} source · ".repeat(100);
                    let destination = format!(
                        "/workspace/{}/終端-log.txt",
                        "long-界e\u{301}-directory/".repeat(60)
                    );
                    dialog = ExportDialog::new(
                        &source,
                        log_export::ExportMetadata::new(42, Some("Phone"), Some("Debug"), None),
                        Path::new(&destination),
                        Some(snapshot()),
                    );
                }
                "zwj-path" | "modified-path" => {
                    let cluster = if state == "zwj-path" { "👩‍💻" } else { "👩🏽‍💻界e\u{301}" };
                    let destination = format!("/{}/tail.txt", cluster.repeat(20));
                    dialog = path_dialog(Path::new(&destination), true);
                }
                "invalid-initial" => dialog = path_dialog(Path::new("/tmp/name\nwith-control.txt"), true),
                "oversize-initial" => dialog = path_dialog(Path::new(&"x".repeat(4097)), true),
                "rejected-paste" => { dialog.handle_event(&Event::Paste("bad\npath".into())); }
                "empty-destination" | "empty-export-focus" | "preparing-empty" | "corrected-destination" => {
                    clear_destination(&mut dialog);
                    if state == "empty-export-focus" { key(&mut dialog, KeyCode::Tab); }
                    if state == "corrected-destination" { dialog.handle_event(&Event::Paste("/tmp/chosen-界.txt".into())); }
                }
                "export-focus" | "disabled-export-focus" => { key(&mut dialog, KeyCode::Tab); }
                "cancel-focus" | "saving-cancel-focus" => {
                    if state == "saving-cancel-focus" { key(&mut dialog, KeyCode::Enter); }
                    key(&mut dialog, KeyCode::Tab);
                    key(&mut dialog, KeyCode::Tab);
                }
                _ => {}
            }
            let buffer = render(&mut dialog, width, height);
            let text = text(&buffer);
            assert!(
                text.contains("Export retained console log"),
                "{state} {width}x{height}\n{text}"
            );
            assert!(text.contains("query/filter-hidden lines."));
            assert!(text.contains("[ Export ]"));
            assert!(text.contains("[ Cancel ]"));
            assert!(text.contains("Destination (no overwrite)"));
            if matches!(state, "zwj-path" | "modified-path") {
                assert!(row_text(&buffer, dialog.regions().destination).contains("/tail.txt"));
            }
            if matches!(
                state,
                "invalid-initial" | "oversize-initial" | "rejected-paste"
            ) {
                assert!(text.contains("Error:"));
                assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            }
            if matches!(
                state,
                "empty-destination" | "empty-export-focus" | "preparing-empty"
            ) {
                assert!(text.contains("Enter a destination."));
                assert!(!text.contains("Ready ·"));
                assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Consumed);
            }
            if matches!(state, "cancel-focus" | "saving-cancel-focus") {
                assert!(text.contains("Enter Cancel; Esc Cancel"));
                assert_eq!(key(&mut dialog, KeyCode::Enter), DialogAction::Cancel);
            }
            if let Some(directory) = std::env::var_os("FLIMRU_P4_EVIDENCE_DIR") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let stem = format!("{state}-{width}x{height}");
                std::fs::write(directory.join(format!("{stem}.txt")), format!("{text}\n")).unwrap();
                std::fs::write(directory.join(format!("{stem}.svg")), svg(&buffer)).unwrap();
            }
        }
    }
}
