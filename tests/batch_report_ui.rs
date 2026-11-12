#![allow(clippy::unwrap_used)]
use flimru::batch_commands;
#[path = "../src/batch_report.rs"]
mod batch_report;

use batch_commands::{
    BatchReport, HotCommand, HotExclusion, SendFailure, SkipReason, TargetDetail, TargetOutcome,
};
use batch_report::{BatchReportView, ReportAction};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
    widgets::Paragraph,
};
use std::path::Path;

fn mixed() -> BatchReport {
    let outcomes = [
        TargetOutcome::Requested,
        TargetOutcome::Skipped(SkipReason::Ended),
        TargetOutcome::Skipped(SkipReason::Stopping),
        TargetOutcome::Skipped(SkipReason::Unsupported(HotExclusion::Profile)),
        TargetOutcome::Failed(SendFailure::InputRejected),
        TargetOutcome::Failed(SendFailure::SessionUnavailable),
        TargetOutcome::Skipped(SkipReason::Unsupported(HotExclusion::CustomRunner)),
        TargetOutcome::Requested,
    ];
    BatchReport {
        requested: 2,
        skipped: 4,
        failed: 2,
        details: outcomes
            .into_iter()
            .enumerate()
            .map(|(index, outcome)| TargetDetail {
                id: 100 + index as u64,
                device: format!("Device {} 界 e\u{301} 🧑‍💻", index + 1),
                outcome,
            })
            .collect(),
    }
}

fn all_ended() -> BatchReport {
    let mut report = mixed();
    report.requested = 0;
    report.skipped = report.details.len();
    report.failed = 0;
    for detail in &mut report.details {
        detail.outcome = TargetOutcome::Skipped(SkipReason::Ended);
    }
    report
}

fn render(view: &mut BatchReportView, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Paragraph::new("LIVE FLUTTER · active tabs keep emitting output"),
                frame.area(),
            );
            view.render(frame);
        })
        .unwrap();
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

fn key(view: &mut BatchReportView, code: KeyCode) -> ReportAction {
    view.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn mouse(view: &mut BatchReportView, kind: MouseEventKind, column: u16, row: u16) -> ReportAction {
    view.handle_event(&Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }))
}

fn pages(view: &mut BatchReportView, width: u16, height: u16) -> String {
    key(view, KeyCode::Home);
    let mut all = text(&render(view, width, height));
    let mut previous = view.offset();
    loop {
        key(view, KeyCode::PageDown);
        let buffer = render(view, width, height);
        if previous == view.offset() {
            break;
        }
        previous = view.offset();
        all.push('\n');
        all.push_str(&text(&buffer));
    }
    all
}

#[test]
fn counts_and_delivery_caveat_stay_visible_and_every_device_reason_is_reachable() {
    for command in [HotCommand::Reload, HotCommand::Restart] {
        for (width, height) in [(34, 12), (80, 24), (120, 40)] {
            let mut view = BatchReportView::new(command, mixed());
            let rendered = text(&render(&mut view, width, height));
            assert!(
                rendered.contains(&format!("{} delivery report", command.label())),
                "{rendered}"
            );
            assert!(rendered.contains("Requested/queued: 2"), "{rendered}");
            assert!(rendered.contains("Skipped: 4 · Failed: 2"), "{rendered}");
            assert!(rendered.contains("Flutter completion is"), "{rendered}");
            assert!(rendered.contains("unconfirmed."), "{rendered}");
            let all = pages(&mut view, width, height);
            for index in 1..=8 {
                assert!(all.contains(&format!("Device {index}")), "{all}");
            }
            for reason in [
                "session has ended",
                "session is stopping",
                "profile runs",
                "session could not",
                "queue the input request",
                "session is no",
                "longer available for input",
                "custom runner",
            ] {
                assert!(all.contains(reason), "{reason}: {all}");
            }
        }
    }
}

#[test]
fn zero_eligible_and_empty_reports_are_clear_without_implying_flutter_success() {
    let mut ended = BatchReportView::new(HotCommand::Reload, all_ended());
    let all = pages(&mut ended, 34, 12);
    assert!(all.contains("Requested/queued: 0"));
    assert!(all.contains("Skipped: 8 · Failed: 0"));
    assert!(all.contains("session has ended"));
    assert!(!all.contains("succeeded"));
    assert!(!all.contains("completed"));
    let mut empty = BatchReportView::new(HotCommand::Restart, BatchReport::default());
    let all = pages(&mut empty, 34, 12);
    assert!(all.contains("No session targets in this"));
    assert!(all.contains("request."));
    assert_eq!(key(&mut empty, KeyCode::End), ReportAction::Consumed);
    assert_eq!(empty.offset(), 0);
}

#[test]
fn keyboard_wheel_reverse_and_resize_clamp_to_existing_rows() {
    let mut view = BatchReportView::new(HotCommand::Restart, mixed());
    let initial = render(&mut view, 34, 12);
    assert_eq!(view.offset(), 0);
    key(&mut view, KeyCode::Up);
    assert_eq!(view.offset(), 0);
    key(&mut view, KeyCode::Down);
    assert_eq!(view.offset(), 1);
    key(&mut view, KeyCode::PageDown);
    let paged = view.offset();
    assert!(paged > 1);
    key(&mut view, KeyCode::PageUp);
    assert_eq!(view.offset(), 1);
    let close = view.close_region();
    mouse(&mut view, MouseEventKind::ScrollDown, close.x, close.y - 1);
    assert_eq!(view.offset(), 4);
    mouse(&mut view, MouseEventKind::ScrollUp, close.x, close.y - 1);
    assert_eq!(view.offset(), 1);
    key(&mut view, KeyCode::End);
    let end = view.offset();
    assert!(end > paged);
    for _ in 0..100 {
        key(&mut view, KeyCode::Down);
    }
    assert_eq!(view.offset(), end);
    view.handle_event(&Event::Resize(120, 40));
    assert_eq!(view.close_region(), Rect::default());
    render(&mut view, 120, 40);
    assert!(view.offset() < end);
    key(&mut view, KeyCode::Home);
    assert_eq!(view.offset(), 0);
    render(&mut view, 34, 12);
    assert_eq!(render(&mut view, 34, 12), initial);
}

#[test]
fn maximum_wide_labels_keep_complete_end_counter_after_resize() {
    for command in [HotCommand::Reload, HotCommand::Restart] {
        let mut report = all_ended();
        for detail in &mut report.details {
            detail.device = "界".repeat(120);
            detail.outcome = TargetOutcome::Skipped(SkipReason::Unsupported(HotExclusion::Test));
        }
        let mut view = BatchReportView::new(command, report);
        let initial = text(&render(&mut view, 34, 12));
        assert!(initial.contains("1-3/103"), "{initial}");
        key(&mut view, KeyCode::End);
        assert_eq!(view.offset(), 100);
        for (width, height) in [(34, 12), (80, 24), (34, 12)] {
            let old_close = view.close_region();
            assert_eq!(
                view.handle_event(&Event::Resize(width, height)),
                ReportAction::Consumed
            );
            assert_eq!(view.close_region(), Rect::default());
            assert_eq!(
                mouse(
                    &mut view,
                    MouseEventKind::Down(MouseButton::Left),
                    old_close.x,
                    old_close.y
                ),
                ReportAction::Consumed
            );
            render(&mut view, width, height);
            key(&mut view, KeyCode::End);
            let buffer = render(&mut view, width, height);
            let rendered = text(&buffer);
            assert!(rendered.contains("Requested/queued: 0"), "{rendered}");
            assert!(rendered.contains("Skipped: 8 · Failed: 0"), "{rendered}");
            assert!(rendered.contains("unconfirmed."), "{rendered}");
            let region = view.close_region();
            let footer: String = (0..width).map(|x| buffer[(x, region.y)].symbol()).collect();
            if width == 34 {
                assert!(footer.contains("101-103/103"), "{footer}");
            } else {
                assert!(footer.contains("32-47/47"), "{footer}");
            }
            if let Some(directory) = std::env::var_os("FLIMRU_P5_REPORT_FIX_EVIDENCE_DIR") {
                write_evidence(
                    Path::new(&directory),
                    &format!(
                        "maximum-wide-{}-end-{width}x{height}",
                        command.label().replace(' ', "-").to_lowercase()
                    ),
                    &buffer,
                );
            }
            let button: String = (region.x..region.right())
                .map(|x| buffer[(x, region.y)].symbol())
                .collect();
            assert_eq!(button, "[Close]");
            assert_eq!(
                mouse(
                    &mut view,
                    MouseEventKind::Down(MouseButton::Left),
                    region.right(),
                    region.y
                ),
                ReportAction::Consumed
            );
            assert!(view.is_open());
        }
        let close = view.close_region();
        assert_eq!(
            mouse(
                &mut view,
                MouseEventKind::Down(MouseButton::Left),
                close.right() - 1,
                close.y
            ),
            ReportAction::Closed
        );
    }
}

#[test]
fn unknown_keys_paste_background_clicks_release_and_resize_never_escape_the_modal() {
    let mut view = BatchReportView::new(HotCommand::Reload, mixed());
    let initial = render(&mut view, 80, 24);
    for event in [
        Event::Paste("rR\n\u{1b}".into()),
        Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        Event::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE)),
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )),
        Event::FocusGained,
        Event::FocusLost,
    ] {
        assert_eq!(view.handle_event(&event), ReportAction::Consumed);
        assert!(view.is_open());
    }
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::ScrollDown,
        MouseEventKind::Moved,
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        assert_eq!(mouse(&mut view, kind, 0, 0), ReportAction::Consumed);
    }
    assert_eq!(view.offset(), 0);
    assert_eq!(render(&mut view, 80, 24), initial);
}

#[test]
fn only_actual_close_cells_and_enter_escape_close_and_closed_views_do_not_render() {
    for code in [KeyCode::Enter, KeyCode::Esc] {
        let mut view = BatchReportView::new(HotCommand::Reload, mixed());
        render(&mut view, 34, 12);
        assert_eq!(key(&mut view, code), ReportAction::Closed);
        assert!(!view.is_open());
        assert_eq!(key(&mut view, code), ReportAction::Consumed);
        assert!(text(&render(&mut view, 80, 24)).contains("LIVE FLUTTER"));
    }
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        let mut view = BatchReportView::new(HotCommand::Restart, mixed());
        let buffer = render(&mut view, width, height);
        let region = view.close_region();
        assert!(view.covers((region.x, region.y).into()));
        assert!(!view.covers((0, 0).into()));
        let button: String = (region.x..region.right())
            .map(|x| buffer[(x, region.y)].symbol())
            .collect();
        assert_eq!(button, "[Close]");
        assert_eq!(
            mouse(
                &mut view,
                MouseEventKind::Down(MouseButton::Left),
                region.right(),
                region.y
            ),
            ReportAction::Consumed
        );
        assert!(view.is_open());
        assert_eq!(
            mouse(
                &mut view,
                MouseEventKind::Down(MouseButton::Left),
                region.right() - 1,
                region.y
            ),
            ReportAction::Closed
        );
    }
}

#[test]
fn tiny_layouts_reject_stale_hit_regions_and_remain_dismissible() {
    for (width, height) in [(0, 0), (1, 1), (8, 3), (24, 10), (34, 9), (33, 12)] {
        let mut view = BatchReportView::new(HotCommand::Restart, mixed());
        render(&mut view, 80, 24);
        let old_close = view.close_region();
        view.handle_event(&Event::Resize(width, height));
        assert_eq!(
            mouse(
                &mut view,
                MouseEventKind::Down(MouseButton::Left),
                old_close.x,
                old_close.y
            ),
            ReportAction::Consumed
        );
        render(&mut view, width, height);
        assert_eq!(view.close_region(), Rect::default());
        key(&mut view, KeyCode::End);
        assert_eq!(view.offset(), 0);
        assert_eq!(key(&mut view, KeyCode::Esc), ReportAction::Closed);
    }
}

#[test]
fn owned_results_stay_frozen_and_long_unicode_labels_wrap_without_broken_graphemes() {
    let mut report = mixed();
    report.details[0].device = "Long device 界 e\u{301} 🧑‍💻 ".repeat(8);
    let mut view = BatchReportView::new(HotCommand::Restart, report.clone());
    report.requested = 999;
    report.details.clear();
    let buffer = render(&mut view, 34, 12);
    assert!(text(&buffer).contains("Requested/queued: 2"));
    let mut found_combining = false;
    let mut found_joined = false;
    loop {
        let buffer = render(&mut view, 34, 12);
        for cell in &buffer.content {
            if cell.symbol().contains('\u{301}') {
                found_combining = true;
                assert!(cell.symbol().contains("e\u{301}"));
            }
            if cell.symbol().contains('\u{200d}') {
                found_joined = true;
                assert!(cell.symbol().contains("🧑‍💻"));
            }
        }
        let old = view.offset();
        key(&mut view, KeyCode::PageDown);
        if old == view.offset() {
            break;
        }
    }
    assert!(found_combining);
    assert!(found_joined);
}

#[test]
fn boundary_labels_render_whole_graphemes_or_explicit_ellipsis_in_both_paths() {
    for (state, label, expected_grapheme, truncated) in [
        ("joined-cut", format!("{}🧑‍💻", "x".repeat(118)), "", true),
        (
            "accent-cut",
            format!("{}e\u{301}", "x".repeat(119)),
            "",
            true,
        ),
        ("joined-fit", format!("{}🧑‍💻", "x".repeat(117)), "🧑‍💻", false),
        (
            "accent-fit",
            format!("{}e\u{301}", "x".repeat(118)),
            "e\u{301}",
            false,
        ),
    ] {
        let report = BatchReport {
            requested: 1,
            details: vec![TargetDetail {
                id: 900,
                device: label.clone(),
                outcome: TargetOutcome::Requested,
            }],
            ..BatchReport::default()
        };
        let snapshot = batch_commands::dispatch(
            HotCommand::Reload,
            &[batch_commands::BatchTarget {
                id: 900,
                device: label,
                capabilities: batch_commands::HotCapabilities::Supported,
                live: true,
                stopping: false,
            }],
            |_, _| Ok(()),
        );
        for (width, height) in [(34, 12), (80, 24)] {
            let mut defensive = BatchReportView::new(HotCommand::Reload, report.clone());
            let mut dispatched = BatchReportView::new(HotCommand::Reload, snapshot.clone());
            let actual = pages(&mut defensive, width, height);
            assert_eq!(actual, pages(&mut dispatched, width, height));
            if truncated {
                assert!(actual.contains('…'), "{actual}");
                assert!(!actual.contains('🧑'), "{actual}");
                assert!(!actual.contains('\u{200d}'), "{actual}");
                assert!(!actual.contains('\u{301}'), "{actual}");
            } else {
                assert!(actual.contains(expected_grapheme), "{actual}");
                assert!(!actual.contains('…'), "{actual}");
            }
            if let Some(directory) = std::env::var_os("FLIMRU_P5_REPORT_FIX_EVIDENCE_DIR") {
                for (source, view) in [("public", &mut defensive), ("dispatch", &mut dispatched)] {
                    write_evidence(
                        Path::new(&directory),
                        &format!("unicode-{state}-{source}-end-{width}x{height}"),
                        &render(view, width, height),
                    );
                }
            }
        }
    }
}

#[test]
fn publicly_constructed_reports_still_bound_and_sanitize_device_details() {
    let mut report = mixed();
    report.details[0].device = format!("SAFE\x1b[2J\n\r\t{}", "界".repeat(300));
    report.details.push(TargetDetail {
        id: 999,
        device: "NINTH_DETAIL_MUST_NOT_RENDER".into(),
        outcome: TargetOutcome::Requested,
    });
    let mut view = BatchReportView::new(HotCommand::Reload, report);
    let all = pages(&mut view, 34, 12);
    assert!(all.contains("SAFE[2J"));
    assert!(!all.contains('\u{1b}'));
    assert!(!all.contains('\t'));
    assert!(!all.contains("NINTH_DETAIL_MUST_NOT_RENDER"));
    // Even the oversize label leaves all eight bounded source entries reachable.
    assert!(all.contains("Device 8"));
}

fn color(value: Color, fallback: &str) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Yellow => "#ffff00".into(),
        _ => fallback.into(),
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn write_evidence(directory: &Path, stem: &str, buffer: &Buffer) {
    let width = buffer.area.width;
    let height = buffer.area.height;
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
                    color(cell.bg, "#101722")
                ));
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            if cell.symbol() != " " && !cell.symbol().is_empty() {
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    " font-weight=\"bold\""
                } else {
                    ""
                };
                svg.push_str(&format!(
                    r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{weight}>{}</text>"#,
                    x * 10,
                    y * 20 + 16,
                    color(cell.fg, "#dce3ed"),
                    escape(cell.symbol())
                ));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

/// Optional retained fixtures are actual TestBackend cells, never mock content.
#[test]
fn render_delivery_report_evidence() {
    let directory = std::env::var_os("FLIMRU_P5_REPORT_EVIDENCE_DIR");
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        let mut long = mixed();
        long.details[0].device = "研究用長いデバイス e\u{301} 🧑‍💻 ".repeat(10);
        let failures = BatchReport {
            requested: 0,
            skipped: 0,
            failed: 2,
            details: mixed()
                .details
                .into_iter()
                .filter(|detail| matches!(detail.outcome, TargetOutcome::Failed(_)))
                .collect(),
        };
        for (state, command, report) in [
            ("mixed-reload", HotCommand::Reload, mixed()),
            ("mixed-restart", HotCommand::Restart, mixed()),
            ("all-ended", HotCommand::Reload, all_ended()),
            ("empty", HotCommand::Restart, BatchReport::default()),
            ("long-device", HotCommand::Restart, long),
            ("failures", HotCommand::Restart, failures),
        ] {
            let mut view = BatchReportView::new(command, report);
            let buffer = render(&mut view, width, height);
            assert!(text(&buffer).contains("unconfirmed."));
            if let Some(directory) = &directory {
                write_evidence(
                    Path::new(directory),
                    &format!("{state}-{width}x{height}"),
                    &buffer,
                );
            }
            key(&mut view, KeyCode::PageDown);
            let page = render(&mut view, width, height);
            let page_offset = view.offset();
            if let Some(directory) = &directory
                && page_offset > 0
            {
                write_evidence(
                    Path::new(directory),
                    &format!("{state}-page-{width}x{height}"),
                    &page,
                );
            }
            key(&mut view, KeyCode::End);
            let end = render(&mut view, width, height);
            if let Some(directory) = &directory
                && view.offset() > page_offset
            {
                write_evidence(
                    Path::new(directory),
                    &format!("{state}-end-{width}x{height}"),
                    &end,
                );
            }
        }
    }
}
