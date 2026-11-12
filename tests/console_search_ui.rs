#![allow(clippy::unwrap_used)]
#[path = "../src/console_search.rs"]
mod console_search;

use console_search::{SearchEffect, SearchSource, SearchView};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use flimru::console_review::{Severity, SeverityFilter};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
    widgets::Paragraph,
};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const ACCENT: Color = Color::Rgb(96, 210, 225);
fn classify(line: &str) -> Option<Severity> {
    if line.starts_with("ERROR") {
        Some(Severity::Error)
    } else if line.starts_with("WARN") {
        Some(Severity::Warning)
    } else {
        None
    }
}
fn source(id: u64) -> SearchSource {
    SearchSource {
        device: Some("Pixel 界".into()),
        configuration: Some("development".into()),
        outcome: Some("running".into()),
        ..SearchSource::new(id, "Pixel 界 · development".into(), SystemTime::UNIX_EPOCH)
    }
}
fn screen(text: &str, rows: u16, columns: u16) -> vt100::Screen {
    let mut parser = vt100::Parser::new(rows, columns, 100);
    parser.process(text.as_bytes());
    parser.screen().clone()
}
fn wait(view: &mut SearchView) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while view.is_pending() {
        view.poll();
        assert!(Instant::now() < deadline, "search view did not finish");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(view.error().is_none(), "{:?}", view.error());
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn view(text: &str) -> SearchView {
    let mut view = SearchView::new(classify).unwrap();
    view.capture(source(4), screen(text, 24, 120)).unwrap();
    wait(&mut view);
    view
}
fn draw(view: &mut SearchView, width: u16, height: u16, area: Rect) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view.render(frame, area))
        .unwrap()
        .buffer
        .clone()
}
fn text(buffer: &Buffer) -> String {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn review_owns_keyboard_paste_mouse_and_two_stage_escape() {
    let mut view = SearchView::new(classify).unwrap();
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('q'))),
        SearchEffect::Inactive
    );
    view.capture(source(1), screen("hello", 24, 80)).unwrap();
    assert!(view.editing());
    for ch in ['r', 'R'] {
        assert_eq!(
            view.handle_event(&key(KeyCode::Char(ch))),
            SearchEffect::Consumed
        );
    }
    assert_eq!(view.query(), "rR");
    for ch in ['a', 'e'] {
        assert_eq!(
            view.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::CONTROL
            ))),
            SearchEffect::Consumed
        );
        assert_eq!(view.query(), "rR");
    }
    view.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('/'))),
        SearchEffect::Consumed
    );
    assert_eq!(view.query(), "/");
    let control_f = Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert_eq!(view.handle_event(&control_f), SearchEffect::Consumed);
    assert_eq!(view.query(), "/");
    let release = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));
    assert_eq!(view.handle_event(&release), SearchEffect::Consumed);
    assert!(view.editing());
    assert_eq!(
        view.handle_event(&key(KeyCode::Esc)),
        SearchEffect::Consumed
    );
    assert!(!view.editing());
    assert_eq!(
        view.handle_event(&Event::Paste("rRq/stop".into())),
        SearchEffect::Consumed
    );
    assert_eq!(view.query(), "/");
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('R'))),
        SearchEffect::Consumed
    );
    for ch in ['a', 'e'] {
        assert_eq!(
            view.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::CONTROL
            ))),
            SearchEffect::Consumed
        );
    }
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('r'))),
        SearchEffect::RefreshSnapshot
    );
    let mouse = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 2,
        row: 4,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(view.handle_event(&mouse), SearchEffect::Consumed);
    assert_eq!(
        view.handle_event(&key(KeyCode::Esc)),
        SearchEffect::ResumeLive
    );
    assert!(!view.is_active());
    assert!(view.snapshot().is_none());
}

#[test]
fn query_paste_is_bounded_unicode_and_enter_stops_editing() {
    let mut view = view("hello");
    assert_eq!(
        view.handle_event(&Event::Paste("界".repeat(600))),
        SearchEffect::Consumed
    );
    assert_eq!(view.query().chars().count(), 512);
    assert_eq!(
        view.handle_event(&key(KeyCode::Backspace)),
        SearchEffect::Consumed
    );
    assert_eq!(view.query().chars().count(), 511);
    assert_eq!(
        view.handle_event(&key(KeyCode::Enter)),
        SearchEffect::Consumed
    );
    assert!(!view.editing());
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('q'))),
        SearchEffect::Consumed
    );
    assert_eq!(view.query().chars().count(), 511);
    view.handle_event(&key(KeyCode::Char('/')));
    let clear = Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    view.handle_event(&clear);
    assert!(view.query().is_empty());
    view.handle_event(&Event::Paste("hello\n\r\u{1b}".into()));
    assert_eq!(view.query(), "hello");
    wait(&mut view);
    assert_eq!(view.data().unwrap().matches.matches.len(), 1);
}

#[test]
fn filter_navigation_and_refresh_keep_frozen_source_metadata_and_export_scope() {
    let mut view = view("ERROR first\r\nWARN second\r\n  stack\r\nERROR third");
    let frozen = view.snapshot().unwrap();
    view.set_query("r");
    wait(&mut view);
    assert_eq!(view.current_match(), Some(0));
    let count = view.data().unwrap().matches.matches.len();
    view.next_match(true);
    assert_eq!(view.current_match(), Some(count - 1));
    view.next_match(false);
    assert_eq!(view.current_match(), Some(0));
    view.set_filter(SeverityFilter::Errors);
    wait(&mut view);
    assert_eq!(view.filter(), SeverityFilter::Errors);
    assert_eq!(view.data().unwrap().rows.len(), 2);
    assert!(Arc::ptr_eq(&frozen, &view.snapshot().unwrap()));
    assert!(
        view.snapshot()
            .unwrap()
            .plain_text()
            .contains("WARN second\n  stack")
    );
    view.handle_event(&key(KeyCode::Enter));
    view.handle_event(&key(KeyCode::Char('f')));
    wait(&mut view);
    assert_eq!(view.filter(), SeverityFilter::Warnings);
    assert_eq!(view.data().unwrap().rows.len(), 1);
    let mut refreshed = source(4);
    refreshed.captured_at = SystemTime::UNIX_EPOCH + Duration::from_secs(42);
    refreshed.outcome = Some("exited successfully".into());
    view.refresh(refreshed, screen("WARN refreshed", 24, 80))
        .unwrap();
    assert!(
        view.snapshot().is_none(),
        "refresh releases the prior UI Arc before capture"
    );
    wait(&mut view);
    assert_eq!(view.query(), "r");
    assert_eq!(
        view.source().unwrap().captured_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(42)
    );
    assert_eq!(
        view.source().unwrap().configuration.as_deref(),
        Some("development")
    );
    assert_eq!(
        view.source().unwrap().outcome.as_deref(),
        Some("exited successfully")
    );
    assert!(view.refresh(source(999), screen("wrong", 1, 8)).is_err());
}

fn wide_history() -> vt100::Screen {
    let row = format!("ERROR {}界e\u{301} needle", "a".repeat(94));
    let mut parser = vt100::Parser::new(2, 120, 100);
    parser.process(format!("{row}\r\nlive\r\nend").as_bytes());
    parser.screen_mut().set_size(2, 4);
    parser.screen().clone()
}

#[test]
fn combining_only_match_highlights_owning_cell_and_horizontal_reveal_survives_resize() {
    let mut view = SearchView::new(classify).unwrap();
    view.capture(source(8), wide_history()).unwrap();
    view.set_query("\u{301}");
    wait(&mut view);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    let left = view.offsets().1;
    assert!(left > 4);
    let x = 1 + 102 - left;
    assert_eq!(buffer[(x, 3)].symbol(), "e\u{301}");
    assert_eq!(buffer[(x, 3)].bg, ACCENT);
    assert!(!text(&buffer).contains("copy"));
    view.handle_event(&key(KeyCode::Enter));
    view.handle_event(&key(KeyCode::Left));
    assert_eq!(view.offsets().1, left - 1);
    let buffer = draw(&mut view, 80, 24, Rect::new(0, 0, 80, 24));
    let x = 1 + 102 - view.offsets().1;
    assert_eq!(buffer[(x, 3)].symbol(), "e\u{301}");
    assert_eq!(buffer[(x, 3)].bg, ACCENT);
    assert!(view.offsets().1 <= 42);
    assert_eq!(
        view.snapshot()
            .unwrap()
            .plain_text()
            .matches("needle")
            .count(),
        1
    );
}

#[test]
fn scrolling_is_bounded_and_match_navigation_reveals_vertically() {
    let log = (0..70)
        .map(|i| format!("line {i:02} needle"))
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut view = view(&log);
    view.set_query("needle");
    wait(&mut view);
    draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    view.handle_event(&key(KeyCode::Enter));
    view.next_match(true);
    assert_eq!(view.current_match(), Some(69));
    assert!(view.offsets().0 > 50);
    view.handle_event(&key(KeyCode::Home));
    assert_eq!(view.offsets(), (0, 0));
    view.handle_event(&key(KeyCode::End));
    assert!(view.offsets().0 > 50);
    view.handle_event(&key(KeyCode::PageUp));
    view.handle_event(&key(KeyCode::Up));
    view.handle_event(&key(KeyCode::PageDown));
    view.handle_event(&key(KeyCode::Down));
    view.handle_event(&key(KeyCode::Right));
    view.handle_event(&key(KeyCode::Left));
    view.next_match(false);
    assert_eq!(view.current_match(), Some(0));
    assert_eq!(view.offsets().0, 0);
}

#[test]
fn height_only_viewport_changes_preserve_manual_scroll_but_navigation_reveals() {
    let log = std::iter::once("needle at beginning".to_owned())
        .chain((1..100).map(|row| format!("line {row:03}")))
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut view = view(&log);
    view.set_query("needle");
    wait(&mut view);
    draw(&mut view, 80, 24, Rect::new(0, 0, 80, 24));
    assert_eq!(view.current_match(), Some(0));
    view.handle_event(&key(KeyCode::Enter));
    view.handle_event(&key(KeyCode::End));
    let manual = view.offsets();
    assert!(manual.0 > 70);
    for height in [23, 24, 23] {
        draw(&mut view, 80, 24, Rect::new(0, 0, 80, height));
        assert_eq!(view.offsets(), manual);
    }
    view.next_match(false);
    assert_eq!(view.offsets().0, 0);
    view.handle_event(&key(KeyCode::End));
    view.set_query("line 001");
    wait(&mut view);
    draw(&mut view, 80, 24, Rect::new(0, 0, 80, 23));
    assert!(view.offsets().0 < 2);
}

#[test]
fn source_replacement_and_close_reject_queued_old_results_and_sanitize_labels() {
    let mut view = view("old");
    view.set_query("old");
    let mut next = source(99);
    next.label = format!("\u{1b}{}\n", "界".repeat(200));
    view.capture(next, screen("new source", 24, 80)).unwrap();
    assert!(view.query().is_empty());
    assert_eq!(view.filter(), SeverityFilter::All);
    assert!(view.editing());
    wait(&mut view);
    assert_eq!(view.source().unwrap().id, 99);
    assert_eq!(view.source().unwrap().label.chars().count(), 96);
    assert!(!view.source().unwrap().label.contains('\u{1b}'));
    assert!(view.source().unwrap().label.ends_with('…'));
    assert_eq!(view.snapshot().unwrap().plain_text(), "new source");
    view.close();
    assert!(!view.poll());
    assert!(view.source().is_none());
    view.show_error(
        source(100),
        "Capture exceeds 128 MiB; narrow retained history.".into(),
    );
    assert!(view.is_active());
    assert!(view.error().unwrap().contains("128 MiB"));
    assert_eq!(
        view.handle_event(&key(KeyCode::Esc)),
        SearchEffect::ResumeLive
    );
}

#[test]
fn frozen_source_details_are_scrollable_and_preserve_review_mode_and_position() {
    let label = "Physical Pixel 界 · configuration with a long identity suffix · ended success";
    let mut frozen = source(91);
    frozen.label = label.into();
    frozen.device = Some("Physical device 界 e\u{301} 01".into());
    frozen.configuration = Some("Debug configuration 界 e\u{301} profile tail".into());
    frozen.outcome = Some("ended successfully".into());
    frozen.captured_at = SystemTime::UNIX_EPOCH + Duration::from_secs(42);
    let log = (0..80)
        .map(|i| format!("row {i} needle"))
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut view = SearchView::new(classify).unwrap();
    view.capture(frozen, screen(&log, 24, 120)).unwrap();
    view.set_query("needle");
    wait(&mut view);
    let area = Rect::new(0, 0, 34, 9);
    draw(&mut view, 34, 9, area);
    view.handle_event(&key(KeyCode::Enter));
    view.next_match(false);
    view.handle_event(&key(KeyCode::Down));
    view.handle_event(&key(KeyCode::Right));
    let position = view.offsets();
    let current = view.current_match();
    let snapshot = view.snapshot().unwrap();
    let before = draw(&mut view, 34, 9, area);
    let title = text(&before).lines().next().unwrap().to_owned();
    assert!(title.contains('…'), "{title}");
    assert!(text(&before).contains("i source"));
    assert_eq!(
        view.source().unwrap().label,
        label,
        "title clipping must not mutate identity"
    );
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('i'))),
        SearchEffect::Consumed
    );
    assert!(view.showing_source_details());
    for event in [
        key(KeyCode::Char('r')),
        key(KeyCode::Char('n')),
        key(KeyCode::Char('f')),
        Event::Paste("unexpected input".into()),
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )),
    ] {
        assert_eq!(view.handle_event(&event), SearchEffect::Consumed);
    }
    assert!(view.showing_source_details());
    let mut buffer = draw(&mut view, 34, 9, area);
    let mut contents = String::new();
    for _ in 0..100 {
        for x in 1..33 {
            contents.push_str(buffer[(x, 1)].symbol());
        }
        let previous = buffer.clone();
        view.handle_event(&key(KeyCode::Down));
        buffer = draw(&mut view, 34, 9, area);
        if buffer == previous {
            for y in 2..7 {
                for x in 1..33 {
                    contents.push_str(buffer[(x, y)].symbol());
                }
            }
            break;
        }
    }
    let compact = |value: &str| {
        value
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
    };
    for field in [
        "Frozen source ID: 91",
        label,
        "Physical device 界 e\u{301} 01",
        "Debug configuration 界 e\u{301} profile tail",
        "ended successfully",
        "Captured: Unix UTC 42 s",
    ] {
        assert!(
            compact(&contents).contains(&compact(field)),
            "missing {field}: {contents}"
        );
    }
    assert_eq!(
        view.handle_event(&key(KeyCode::Esc)),
        SearchEffect::Consumed
    );
    assert!(!view.showing_source_details());
    assert!(view.is_active());
    assert!(!view.editing());
    assert_eq!(view.query(), "needle");
    assert_eq!(view.filter(), SeverityFilter::All);
    assert_eq!(view.current_match(), current);
    assert_eq!(view.offsets(), position);
    assert!(Arc::ptr_eq(&view.snapshot().unwrap(), &snapshot));
    assert_eq!(
        view.source().unwrap().captured_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(42)
    );
    assert_eq!(draw(&mut view, 34, 9, area), before);
}

#[test]
fn explicit_review_actions_do_not_type_shortcuts_and_extreme_metadata_is_marked() {
    let mut frozen = source(92);
    frozen.label = "\u{1b}".to_owned() + &"界".repeat(600);
    frozen.device = Some("e\u{301}".repeat(300));
    frozen.configuration = Some("界".repeat(300));
    frozen.outcome = Some("success".repeat(100));
    let mut view = SearchView::new(classify).unwrap();
    view.capture(
        frozen,
        screen("ERROR needle needle\r\nWARN needle", 24, 120),
    )
    .unwrap();
    wait(&mut view);
    view.handle_event(&key(KeyCode::Char('i')));
    assert_eq!(view.query(), "i");
    assert!(!view.showing_source_details());
    view.set_query("needle");
    wait(&mut view);
    view.next_match(false);
    assert_eq!(view.current_match(), Some(1));
    view.previous_match();
    assert_eq!(view.current_match(), Some(0));
    view.cycle_filter();
    wait(&mut view);
    assert_eq!(view.filter(), SeverityFilter::Errors);
    assert_eq!(view.query(), "needle");
    assert!(view.editing());
    view.show_source_details();
    assert!(view.showing_source_details());
    let buffer = draw(&mut view, 34, 9, Rect::new(0, 0, 34, 9));
    let rendered = text(&buffer);
    assert!(rendered.contains("Shortened: label 96"), "{rendered}");
    assert!(rendered.contains("fields 128."), "{rendered}");
    let source = view.source().unwrap();
    for (value, bound) in [
        (&source.label, 96),
        (source.device.as_ref().unwrap(), 128),
        (source.configuration.as_ref().unwrap(), 128),
        (source.outcome.as_ref().unwrap(), 128),
    ] {
        assert!(value.chars().count() <= bound);
        assert!(value.ends_with('…'));
        assert!(!value.chars().any(char::is_control));
    }
    for width in 1..12 {
        for height in 1..10 {
            draw(&mut view, width, height, Rect::new(0, 0, width, height));
        }
    }
    assert_eq!(
        view.handle_event(&key(KeyCode::Enter)),
        SearchEffect::Consumed
    );
    assert!(view.editing());
    assert_eq!(view.query(), "needle");
    view.handle_event(&key(KeyCode::Enter));
    assert!(!view.editing());
    view.edit_query();
    assert!(view.editing());
    view.close();
    view.show_source_details();
    view.edit_query();
    assert!(!view.showing_source_details());
    assert!(!view.editing());
}

#[test]
fn bounded_source_and_error_labels_preserve_boundary_graphemes_and_exact_fits() {
    let mut view = SearchView::new(classify).unwrap();
    let mut normalize = |value: &str, budget| {
        let mut frozen = source(95);
        if budget == 96 {
            frozen.label = value.into();
        }
        if budget == 128 {
            frozen.device = Some(value.into());
        }
        view.show_error(
            frozen,
            if budget == 512 {
                value.into()
            } else {
                "fixture".into()
            },
        );
        match budget {
            96 => view.source().unwrap().label.clone(),
            128 => view.source().unwrap().device.clone().unwrap(),
            _ => view.error().unwrap().to_owned(),
        }
    };
    for budget in [96, 128, 512] {
        let prefix = "a".repeat(budget - 2);
        for cluster in ["👩\u{200d}🚀", "e\u{301}\u{302}"] {
            let value = format!("{prefix}{cluster}tail");
            assert_eq!(normalize(&value, budget), format!("{prefix}…"));
            // The same boundary grapheme is retained intact when it fits.
            let exact = format!("{}{cluster}", "a".repeat(budget - 3));
            assert_eq!(normalize(&exact, budget), exact);
        }
        assert_eq!(
            normalize(&format!("{prefix}界e\u{301}tail"), budget),
            format!("{prefix}界…")
        );
        let exact = format!("{}界", "a".repeat(budget - 1));
        assert_eq!(normalize(&exact, budget), exact);
    }
}

#[test]
fn operation_details_survive_pending_search_and_close_without_claiming_a_snapshot() {
    let mut view = view("needle needle");
    draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    let frozen = view.snapshot().unwrap();
    view.set_query("needle");
    view.show_operation_details("Hot reload: Input queue is full; wait for Flutter to read input");
    wait(&mut view);
    assert!(view.showing_operation_details());
    assert!(!view.showing_source_details());
    assert_eq!(view.query(), "needle");
    assert!(Arc::ptr_eq(&frozen, &view.snapshot().unwrap()));
    assert!(
        text(&draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12)))
            .contains("Latest operation outcome")
    );
    view.handle_event(&key(KeyCode::Esc));
    assert!(view.is_active());
    assert!(view.editing());
    view.close();
    view.show_operation_details("Hot reload: Input queue is full; wait for Flutter to read input");
    assert!(!view.is_active());
    assert!(view.source().is_none());
    assert!(view.snapshot().is_none());
    let rendered = text(&draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12)));
    assert!(rendered.contains("Latest operation outcome"));
    assert!(!rendered.contains("Snapshot"));
    assert_eq!(
        view.handle_event(&Event::Paste("r".into())),
        SearchEffect::Consumed
    );
    assert_eq!(
        view.handle_event(&key(KeyCode::Esc)),
        SearchEffect::Consumed
    );
    assert!(!view.showing_operation_details());
    assert_eq!(
        view.handle_event(&key(KeyCode::Char('r'))),
        SearchEffect::Inactive
    );
}

fn color(value: Color, fallback: &str) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
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
fn write_evidence(directory: &std::path::Path, stem: &str, buffer: &Buffer) {
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
                svg.push_str(&format!(r#"<text x="{}" y="{}" fill="{}" font-family="Menlo,monospace" font-size="15"{}>{}</text>"#,x*10,y*20+16,color(cell.fg,"#dce3ed"),if cell.modifier.contains(Modifier::BOLD) { " font-weight=\"bold\"" } else { "" },escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{stem}.svg")), svg).unwrap();
}

/// Set FLIMRU_P3_EVIDENCE_DIR to retain actual TestBackend cells as TXT/SVG.
#[test]
fn render_evidence_for_review_states_and_minimum_integration_rect() {
    for (width, height) in [(34, 12), (80, 24), (120, 40)] {
        for state in [
            "editing",
            "reading",
            "no-matches",
            "empty-filter",
            "empty-snapshot",
            "pending",
            "error",
            "wide-highlight",
            "capped",
        ] {
            let mut view = SearchView::new(classify).unwrap();
            match state {
                "error" => view.show_error(source(1), "Retained screen exceeds the 128 MiB capture estimate. Refresh after reducing retained history.".into()),
                "pending" => { view.capture(source(1), screen("ERROR waiting",24,120)).unwrap(); }
                "wide-highlight" => { view.capture(source(1), wide_history()).unwrap(); view.set_query("\u{301}"); wait(&mut view); view.handle_event(&key(KeyCode::Enter)); }
                "capped" => { view.capture(source(1), screen(&"a".repeat(10_001),1,10_002)).unwrap(); view.set_query("A"); wait(&mut view); view.handle_event(&key(KeyCode::Enter)); }
                _ => {
                    let log = if state == "empty-snapshot" { "" } else { "ERROR request failed 界 e\u{301}\r\nWARN retry scheduled\r\n  stack continuation\r\nINFO connected\r\nERROR request failed again" };
                    view.capture(source(1),screen(log,24,120)).unwrap();
                    if state == "empty-filter" { view.set_query("missing"); view.set_filter(SeverityFilter::Errors); }
                    else if state == "no-matches" { view.set_query("missing"); }
                    else if state != "empty-snapshot" { view.set_query("request"); }
                    wait(&mut view);
                    if state == "empty-filter" { view.set_filter(SeverityFilter::Warnings); wait(&mut view); /* clear separately below */ }
                    if state != "editing" { view.handle_event(&key(KeyCode::Enter)); }
                }
            }
            if state == "empty-filter" {
                view.capture(source(1), screen("INFO connected", 24, 120))
                    .unwrap();
                view.set_filter(SeverityFilter::Errors);
                wait(&mut view);
            }
            let buffer = draw(&mut view, width, height, Rect::new(0, 0, width, height));
            let rendered = text(&buffer);
            assert!(rendered.contains("Snapshot"), "{state}: {rendered}");
            assert!(rendered.contains("Search"), "{state}: {rendered}");
            match state {
                "pending" => assert!(rendered.contains("Capturing")),
                "error" => assert!(
                    rendered
                        .replace('│', " ")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains("128 MiB"),
                    "{rendered}"
                ),
                "no-matches" => assert!(rendered.contains("No matches")),
                "empty-filter" => assert!(rendered.contains("No lines in this filter")),
                "empty-snapshot" => {
                    assert!(rendered.contains("Snapshot is empty"));
                    assert!(rendered.contains("1 row ·"));
                    assert!(!rendered.contains("1 rows"));
                }
                "capped" => assert!(rendered.contains("1/10000+")),
                "reading" => assert!(rendered.contains("1/2")),
                _ => {}
            }
            if let Some(directory) = std::env::var_os("FLIMRU_P3_EVIDENCE_DIR") {
                write_evidence(
                    std::path::Path::new(&directory),
                    &format!("{state}-{width}x{height}"),
                    &buffer,
                );
            }
            if width == 34 {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let completed = terminal
                    .draw(|frame| {
                        frame.render_widget(
                            Paragraph::new("Toolbar · sessions continue"),
                            Rect::new(0, 0, 34, 1),
                        );
                        frame.render_widget(
                            Paragraph::new("[Pixel] [Other live]"),
                            Rect::new(0, 2, 34, 1),
                        );
                        view.render(frame, Rect::new(0, 3, 34, 9));
                    })
                    .unwrap();
                let buffer = completed.buffer;
                let rendered = text(buffer);
                assert!(rendered.contains("Search"));
                assert!(rendered.contains(if view.editing() {
                    "Enter read"
                } else {
                    "Esc resume live"
                }));
                if let Some(directory) = std::env::var_os("FLIMRU_P3_EVIDENCE_DIR") {
                    write_evidence(
                        std::path::Path::new(&directory),
                        &format!("{state}-34x9-integration"),
                        buffer,
                    );
                }
            }
        }
    }
}

#[test]
fn render_long_frozen_source_and_details_at_all_review_sizes() {
    for (width, height) in [(34, 12), (80, 24), (120, 40), (34, 9)] {
        let mut frozen = source(93);
        frozen.label = format!(
            "Physical Pixel 界 · {} · ended success",
            "long configuration segment ".repeat(4)
        );
        frozen.device =
            Some("Physical Pixel 界 e\u{301} · development device identity suffix".into());
        frozen.configuration =
            Some("Development profile 界 · long configuration identity suffix".into());
        frozen.outcome = Some("ended successfully".into());
        frozen.captured_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut view = SearchView::new(classify).unwrap();
        view.capture(
            frozen,
            screen(
                "ERROR first request\r\nWARN second request\r\nINFO third request",
                24,
                120,
            ),
        )
        .unwrap();
        view.set_query("request");
        wait(&mut view);
        let frame_height = if height == 9 { 12 } else { height };
        let area = Rect::new(0, frame_height - height, width, height);
        for state in [
            "long-source-editing",
            "long-source-reading",
            "source-details-top",
            "source-details-page",
            "source-details-end",
        ] {
            match state {
                "long-source-reading" => {
                    view.handle_event(&key(KeyCode::Enter));
                }
                "source-details-top" => view.show_source_details(),
                "source-details-page" => {
                    view.handle_event(&key(KeyCode::PageDown));
                }
                "source-details-end" => {
                    view.handle_event(&key(KeyCode::End));
                }
                _ => {}
            }
            let mut terminal = Terminal::new(TestBackend::new(width, frame_height)).unwrap();
            let completed = terminal
                .draw(|frame| {
                    if height == 9 {
                        frame.render_widget(
                            Paragraph::new("Toolbar · sessions continue"),
                            Rect::new(0, 0, 34, 1),
                        );
                        frame.render_widget(
                            Paragraph::new("[Pixel] [Other live]"),
                            Rect::new(0, 2, 34, 1),
                        );
                    }
                    view.render(frame, area);
                })
                .unwrap();
            let rendered = text(completed.buffer);
            if state.starts_with("long-source") {
                assert!(
                    rendered
                        .lines()
                        .nth(usize::from(area.y))
                        .unwrap()
                        .contains('…'),
                    "{rendered}"
                );
            } else {
                assert!(rendered.contains("Frozen source details"), "{rendered}");
                assert!(rendered.contains("Esc back"), "{rendered}");
                if state == "source-details-top" {
                    assert!(rendered.contains("Shortened:"), "{rendered}");
                }
                if state == "source-details-end" {
                    assert!(rendered.contains("1700000000 s"), "{rendered}");
                }
            }
            if let Some(directory) = std::env::var_os("FLIMRU_P3_EVIDENCE_DIR") {
                let suffix = if height == 9 { "-integration" } else { "" };
                write_evidence(
                    std::path::Path::new(&directory),
                    &format!("{state}-{width}x{height}{suffix}"),
                    completed.buffer,
                );
            }
        }
    }
}

#[test]
fn lowercase_expansion_and_other_matches_color_complete_original_cells() {
    let mut view = view("İ i");
    view.set_query("i");
    wait(&mut view);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert_eq!(buffer[(1, 3)].symbol(), "İ");
    assert_eq!(buffer[(1, 3)].bg, ACCENT);
    assert_eq!(buffer[(3, 3)].bg, Color::Rgb(34, 64, 78));
    view.next_match(false);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert_eq!(buffer[(1, 3)].bg, Color::Rgb(34, 64, 78));
    assert_eq!(buffer[(3, 3)].bg, ACCENT);
}

#[test]
fn horizontal_clip_drops_partial_wide_cell_and_preserves_following_combining_cell() {
    let row = format!("{}界e\u{301}{}", "a".repeat(20), "z".repeat(90));
    let mut view = view(&row);
    view.set_query("\u{301}");
    wait(&mut view);
    draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    view.handle_event(&key(KeyCode::Enter));
    view.handle_event(&key(KeyCode::Home));
    for _ in 0..21 {
        view.handle_event(&key(KeyCode::Right));
    }
    assert_eq!(view.offsets().1, 21);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert_eq!(buffer[(1, 3)].symbol(), " ");
    assert_eq!(buffer[(2, 3)].symbol(), "e\u{301}");
    assert_eq!(buffer[(2, 3)].bg, ACCENT);
    assert!(text(&buffer).contains("◌\u{301}"));
}

#[test]
fn highlighted_wide_glyph_styles_both_cells_and_tiny_areas_are_safe() {
    let mut view = view("界");
    view.set_query("界");
    wait(&mut view);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert_eq!(buffer[(1, 3)].symbol(), "界");
    assert_eq!(buffer[(1, 3)].bg, ACCENT);
    assert_eq!(buffer[(2, 3)].bg, ACCENT);
    for width in 1..12 {
        for height in 1..10 {
            draw(&mut view, width, height, Rect::new(0, 0, width, height));
        }
    }
}

#[test]
fn editing_keeps_the_end_of_a_long_query_visible() {
    let mut view = view("log");
    view.set_query(&("a".repeat(506) + "needle"));
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert!(text(&buffer).lines().nth(1).unwrap().contains("needle"));
    assert_eq!(view.query().chars().count(), 512);
}

fn long_overlapping_query_render(columns: u16) -> Duration {
    let mut parser = vt100::Parser::new(24, columns, 20_000);
    parser.process("a".repeat(12_000).as_bytes());
    let mut view = SearchView::new(classify).unwrap();
    view.capture(source(43), parser.screen().clone()).unwrap();
    view.set_query(&"a".repeat(512));
    wait(&mut view);
    assert_eq!(view.data().unwrap().matches.matches.len(), 10_000);
    assert!(view.data().unwrap().matches.capped);
    let area = Rect::new(0, 0, 120, 40);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| view.render(frame, area)).unwrap();
    view.handle_event(&key(KeyCode::Enter));
    let target_row = 5_000 / usize::from(columns);
    while view.offsets().0 < target_row {
        view.handle_event(&key(KeyCode::PageDown));
    }
    while view.offsets().0 > target_row {
        view.handle_event(&key(KeyCode::Up));
    }
    let top = view.offsets().0;
    let started = Instant::now();
    for _ in 0..10 {
        terminal.draw(|frame| view.render(frame, area)).unwrap();
    }
    let elapsed = started.elapsed();
    eprintln!(
        "overlapping viewport: captured_columns={columns} top={top} matches=10000 query_chars=512 rendered=120x40 frames=10 elapsed={elapsed:?}"
    );
    // The current match is offscreen, but secondary matches cover every a.
    for y in 3..37 {
        for x in 1..1 + columns.min(118) {
            assert_eq!(terminal.backend().buffer()[(x, y)].symbol(), "a");
            assert_eq!(
                terminal.backend().buffer()[(x, y)].bg,
                Color::Rgb(34, 64, 78)
            );
        }
    }
    // Navigate far beyond the skipped candidates. Its original index must
    // retain current styling while the preceding overlapping matches stay dim.
    for _ in 0..5_000 {
        view.next_match(false);
    }
    assert_eq!(view.current_match(), Some(5_000));
    assert_eq!(
        view.data().unwrap().matches.matches[5_000].bytes.start,
        5_000
    );
    let buffer = draw(&mut view, 120, 40, area);
    let snapshot = view.snapshot().unwrap();
    let (top, left) = view.offsets();
    let mut active_cells = 0;
    let mut other_cells = 0;
    for y in 3..37 {
        let row = &snapshot.rows()[top + usize::from(y - 3)];
        for column in 0..columns.saturating_sub(left).min(118) {
            let cell = &buffer[(1 + column, y)];
            let byte = row.bytes.start + usize::from(left + column);
            let active = (5_000..5_512).contains(&byte);
            assert_eq!(cell.symbol(), "a");
            assert_eq!(
                cell.bg,
                if active {
                    ACCENT
                } else {
                    Color::Rgb(34, 64, 78)
                }
            );
            assert_eq!(cell.modifier.contains(Modifier::BOLD), active);
            if active {
                active_cells += 1;
            } else {
                other_cells += 1;
            }
        }
    }
    assert!(active_cells > 0);
    assert!(other_cells > 0);
    elapsed
}

#[test]
fn long_overlapping_queries_highlight_visible_rows_with_bounded_debug_work() {
    for columns in [1, 32, 120] {
        // Deliberately loose for loaded debug builds; the former width-one
        // traversal independently measured about 410 ms for a single frame.
        assert!(
            long_overlapping_query_render(columns) < Duration::from_secs(2),
            "ten viewport frames exceeded the loaded-debug rendering budget at width {columns}"
        );
    }
}

#[test]
fn wrapped_long_combining_query_keeps_complete_active_and_secondary_wide_cells() {
    let mut parser = vt100::Parser::new(24, 3, 1_000);
    parser.process("e\u{301}界".repeat(500).as_bytes());
    let mut view = SearchView::new(classify).unwrap();
    view.capture(source(44), parser.screen().clone()).unwrap();
    // Begins inside the first cell and ends at a combining mark after more
    // than 100 wrapped rows. Both boundaries must retain their owning cells.
    view.set_query(&("\u{301}界".to_owned() + &"e\u{301}界".repeat(100) + "e\u{301}"));
    wait(&mut view);
    assert!(view.data().unwrap().matches.matches.len() > 300);
    draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    view.next_match(false);
    assert_eq!(view.current_match(), Some(1));
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    for y in 3..9 {
        assert_eq!(buffer[(1, y)].symbol(), "e\u{301}");
        assert_eq!(buffer[(2, y)].symbol(), "界");
        let active = y > 3;
        for x in 1..4 {
            assert_eq!(
                buffer[(x, y)].bg,
                if active {
                    ACCENT
                } else {
                    Color::Rgb(34, 64, 78)
                }
            );
            assert_eq!(buffer[(x, y)].modifier.contains(Modifier::BOLD), active);
        }
    }
    view.handle_event(&key(KeyCode::Enter));
    for _ in 0..97 {
        view.handle_event(&key(KeyCode::Down));
    }
    assert_eq!(view.offsets().0, 97);
    let buffer = draw(&mut view, 34, 12, Rect::new(0, 0, 34, 12));
    assert_eq!(buffer[(1, 8)].symbol(), "e\u{301}");
    assert_eq!(buffer[(1, 8)].bg, ACCENT);
    assert_eq!(buffer[(2, 8)].symbol(), "界");
    for x in 2..4 {
        assert_eq!(buffer[(x, 8)].bg, Color::Rgb(34, 64, 78));
        assert!(!buffer[(x, 8)].modifier.contains(Modifier::BOLD));
    }
}

/// Explicitly measure viewport rendering with wide retained physical rows.
#[test]
#[ignore]
fn viewport_render_work_measurement() {
    let mut view = SearchView::new(classify).unwrap();
    view.capture(source(42), screen(&"a".repeat(1_200_000), 24, 60_000))
        .unwrap();
    wait(&mut view);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let start = Instant::now();
    for _ in 0..10 {
        terminal
            .draw(|frame| view.render(frame, Rect::new(0, 0, 120, 40)))
            .unwrap();
    }
    eprintln!(
        "viewport rendering: text={} captured_columns={} rendered=120x40 frames=10 elapsed={:?}",
        view.snapshot().unwrap().plain_text().len(),
        view.snapshot().unwrap().columns(),
        start.elapsed()
    );
    for columns in [1, 32, 120] {
        long_overlapping_query_render(columns);
    }
}

#[test]
fn parent_focus_suppresses_native_query_cursor_without_changing_editing_state() {
    let mut view = view("log");
    let mut terminal = Terminal::new(TestBackend::new(34, 12)).unwrap();
    terminal
        .draw(|frame| view.render_with_focus(frame, Rect::new(0, 0, 34, 12), false))
        .unwrap();
    assert!(!terminal.backend().cursor_visible());
    assert!(view.editing());
    terminal
        .draw(|frame| view.render(frame, Rect::new(0, 0, 34, 12)))
        .unwrap();
    assert!(terminal.backend().cursor_visible());
}

#[test]
fn dismissing_operation_details_preserves_review_and_leaves_source_details_alone() {
    let lines = (0..80)
        .map(|index| format!("ERROR needle {index:02}\r\n"))
        .collect::<String>();
    let mut view = view(&lines);
    view.set_query("needle");
    view.set_filter(SeverityFilter::Errors);
    wait(&mut view);
    view.handle_event(&key(KeyCode::Enter));
    draw(&mut view, 80, 24, Rect::new(0, 3, 80, 21));
    view.handle_event(&key(KeyCode::End));
    let snapshot = view.snapshot().unwrap();
    let offsets = view.offsets();
    let current = view.current_match();
    view.show_operation_details("retained operation outcome");
    view.dismiss_operation_details();
    assert!(!view.showing_operation_details());
    assert!(view.is_active());
    assert_eq!(view.query(), "needle");
    assert_eq!(view.filter(), SeverityFilter::Errors);
    assert_eq!(view.offsets(), offsets);
    assert_eq!(view.current_match(), current);
    assert!(Arc::ptr_eq(&snapshot, &view.snapshot().unwrap()));
    view.show_source_details();
    view.dismiss_operation_details();
    assert!(view.showing_source_details());
    view.close();
    view.show_operation_details("after resume");
    view.dismiss_operation_details();
    assert!(!view.showing_operation_details() && !view.is_active());
    assert!(view.source().is_none());
}
