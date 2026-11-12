#![allow(clippy::unwrap_used)]

#[path = "../src/console_snapshot.rs"]
mod console_snapshot;

use console_snapshot::{ConsoleSnapshot, MAX_MATCHES, SnapshotError, SnapshotLimits};
use std::sync::atomic::{AtomicBool, Ordering};

fn cancelled() -> AtomicBool {
    AtomicBool::new(false)
}
fn parser(text: &str, rows: u16, columns: u16, history: usize) -> vt100::Parser {
    let mut parser = vt100::Parser::new(rows, columns, history);
    parser.process(text.as_bytes());
    parser
}
fn snapshot(text: &str, rows: u16, columns: u16) -> ConsoleSnapshot {
    ConsoleSnapshot::capture(
        parser(text, rows, columns, 0).screen().clone(),
        &cancelled(),
    )
    .unwrap()
}

#[test]
fn history_is_chronological_and_capture_leaves_source_viewport_unchanged() {
    let mut parser = parser("00\r\n01\r\n02\r\n03\r\n04", 2, 8, 10);
    parser.screen_mut().set_scrollback(1);
    let source_contents = parser.screen().contents();
    let snap = ConsoleSnapshot::capture(parser.screen().clone(), &cancelled()).unwrap();
    assert_eq!(snap.plain_text(), "00\n01\n02\n03\n04");
    assert_eq!(snap.rows().len(), 5);
    assert_eq!(snap.lines().len(), 5);
    assert_eq!(parser.screen().scrollback(), 1);
    assert_eq!(parser.screen().contents(), source_contents);
    parser.process(b"changed");
    assert_eq!(snap.plain_text(), "00\n01\n02\n03\n04");
}

#[test]
fn soft_wraps_join_but_explicit_blank_rows_and_padding_survive() {
    let snap = snapshot("ab  cdef\r\n\r\nlast  ", 5, 4);
    assert_eq!(snap.plain_text(), "ab  cdef\n\nlast");
    assert_eq!(snap.columns(), 4);
    assert!(snap.rows()[0].wrapped);
    assert_eq!(snap.line_text(0), Some("ab  cdef"));
    assert_eq!(snap.row_text(0), Some("ab  "));
    assert_eq!(snap.lines()[0].rows, 0..2);
    let matches = snap.search(" cD", &cancelled(), |_, _| true).unwrap();
    assert_eq!(matches.matches.len(), 1);
    let cells: Vec<_> = snap.cell_ranges(matches.matches[0].bytes.clone()).collect();
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].row, 0);
    assert_eq!(cells[0].columns, 3..4);
    assert_eq!(cells[1].row, 1);
    assert_eq!(cells[1].columns, 0..2);
}

#[test]
fn wide_and_combining_cells_map_original_utf8_even_after_case_expansion() {
    let snap = snapshot("A界e\u{301} İẞ", 1, 20);
    assert_eq!(snap.plain_text(), "A界e\u{301} İẞ");
    let found = snap
        .search("界E\u{301}", &cancelled(), |_, _| true)
        .unwrap();
    let cells: Vec<_> = snap.cell_ranges(found.matches[0].bytes.clone()).collect();
    assert_eq!(cells[0].columns, 1..4);
    let found = snap.search("\u{301}", &cancelled(), |_, _| true).unwrap();
    assert_eq!(
        snap.cell_ranges(found.matches[0].bytes.clone())
            .next()
            .unwrap()
            .columns,
        3..4
    );
    let found = snap.search("i\u{307}", &cancelled(), |_, _| true).unwrap();
    assert_eq!(&snap.plain_text()[found.matches[0].bytes.clone()], "İ");
    assert_eq!(
        snap.cell_ranges(found.matches[0].bytes.clone())
            .next()
            .unwrap()
            .columns,
        5..6
    );
    let found = snap.search("ß", &cancelled(), |_, _| true).unwrap();
    assert_eq!(&snap.plain_text()[found.matches[0].bytes.clone()], "ẞ");
}

#[test]
fn ansi_is_removed_and_filters_apply_to_whole_logical_lines() {
    let snap = snapshot("\x1b[31mERROR abcdef\x1b[0m\r\nWARN no\r\nnext", 4, 8);
    assert_eq!(snap.plain_text(), "ERROR abcdef\nWARN no\nnext");
    let found = snap
        .search("ABCD", &cancelled(), |_, line| line.starts_with("ERROR"))
        .unwrap();
    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].logical_line, 0);
    assert!(
        snap.search("no", &cancelled(), |_, line| line.starts_with("ERROR"))
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(
        snap.search("", &cancelled(), |_, _| true)
            .unwrap()
            .matches
            .is_empty()
    );
    assert!(
        snap.search("abc\ndef", &cancelled(), |_, _| true)
            .unwrap()
            .matches
            .is_empty()
    );
}

#[test]
fn literal_search_is_overlapping_and_capped_only_when_more_matches_exist() {
    let snap = snapshot("a.a aaA", 1, 16);
    assert_eq!(
        snap.search("a.a", &cancelled(), |_, _| true)
            .unwrap()
            .matches
            .len(),
        1
    );
    assert_eq!(
        snap.search("aa", &cancelled(), |_, _| true)
            .unwrap()
            .matches
            .len(),
        2
    );
    let snap = snapshot(&"a".repeat(MAX_MATCHES + 1), 1, (MAX_MATCHES + 2) as u16);
    let found = snap.search("A", &cancelled(), |_, _| true).unwrap();
    assert_eq!(found.matches.len(), MAX_MATCHES);
    assert!(found.capped);
    let snap = snapshot(&"a".repeat(MAX_MATCHES), 1, (MAX_MATCHES + 2) as u16);
    assert!(!snap.search("a", &cancelled(), |_, _| true).unwrap().capped);
}

#[test]
fn query_limit_counts_unicode_characters_and_operations_cancel() {
    let snap = snapshot("abc", 1, 8);
    assert!(
        snap.search(&"界".repeat(512), &cancelled(), |_, _| true)
            .is_ok()
    );
    assert_eq!(
        snap.search(&"界".repeat(513), &cancelled(), |_, _| true)
            .unwrap_err(),
        SnapshotError::QueryLimit
    );
    let cancel = AtomicBool::new(true);
    assert_eq!(
        snap.search("a", &cancel, |_, _| true).unwrap_err(),
        SnapshotError::Cancelled
    );
    assert_eq!(
        ConsoleSnapshot::capture(parser("a", 1, 8, 0).screen().clone(), &cancel).unwrap_err(),
        SnapshotError::Cancelled
    );
    let cancel = cancelled();
    assert_eq!(
        snap.search("a", &cancel, |_, _| {
            cancel.store(true, Ordering::Relaxed);
            true
        })
        .unwrap_err(),
        SnapshotError::Cancelled
    );
}

#[test]
fn text_and_coordinate_budgets_reject_instead_of_truncating() {
    let screen = parser("abcdef", 1, 8, 0).screen().clone();
    let limits = SnapshotLimits {
        text_bytes: 5,
        coordinate_bytes: 64 * 1024,
    };
    assert_eq!(
        ConsoleSnapshot::capture_with_limits(screen, &cancelled(), limits).unwrap_err(),
        SnapshotError::TextLimit
    );
    let screen = parser("\r\n\r\n", 3, 8, 0).screen().clone();
    let limits = SnapshotLimits {
        text_bytes: 100,
        coordinate_bytes: 1,
    };
    assert_eq!(
        ConsoleSnapshot::capture_with_limits(screen, &cancelled(), limits).unwrap_err(),
        SnapshotError::CoordinateLimit
    );
    let screen = parser("abcdef", 1, 8, 0).screen().clone();
    let limits = SnapshotLimits {
        text_bytes: 6,
        coordinate_bytes: 64 * 1024,
    };
    let snap = ConsoleSnapshot::capture_with_limits(screen, &cancelled(), limits).unwrap();
    assert_eq!(snap.plain_text(), "abcdef");
    assert!(snap.coordinate_bytes() <= limits.coordinate_bytes);
}

#[test]
fn blank_and_trailing_wide_rows_and_empty_ranges_are_safe() {
    let snap = snapshot("界\r\n", 2, 8);
    assert_eq!(snap.plain_text(), "界\n");
    assert_eq!(snap.cell_ranges(0..3).next().unwrap().columns, 0..2);
    assert_eq!(snap.cell_ranges(0..0).count(), 0);
    assert_eq!(snap.cell_ranges(3..4).count(), 0);
    assert_eq!(snap.cell_ranges(100..101).count(), 0);
    assert_eq!(snap.row_text(50), None);
    assert_eq!(snap.line_text(50), None);
}

#[test]
fn snapshot_is_send_sync_and_ascii_coordinates_are_compressed() {
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<ConsoleSnapshot>();
    let snap = snapshot(&"x".repeat(60_000), 1, 60_001);
    assert!(snap.coordinate_bytes() < 64 * 1024);
    let found = snap.search("XXX", &cancelled(), |_, _| true).unwrap();
    assert_eq!(found.matches[5].bytes, 5..8);
    assert_eq!(
        snap.cell_ranges(found.matches[5].bytes.clone())
            .next()
            .unwrap()
            .columns,
        5..8
    );
}

/// Explicit near-limit measurement; kept out of routine correctness runs.
#[test]
#[ignore]
fn near_limit_capture_and_search_measurement() {
    use std::time::Instant;
    let cell = "a\u{301}\u{302}\u{303}\u{304}\u{305}\u{306}\u{307}\u{308}";
    let input = cell.repeat(980_000);
    let parsed = parser(&input, 980, 1000, 0);
    let screen = parsed.screen().clone();
    drop(parsed);
    drop(input);
    let start = Instant::now();
    let snap = ConsoleSnapshot::capture(screen, &cancelled()).unwrap();
    let capture = start.elapsed();
    let start = Instant::now();
    let found = snap
        .search(&"a".repeat(512), &cancelled(), |_, _| true)
        .unwrap();
    let search = start.elapsed();
    assert!(found.matches.is_empty());
    assert!(snap.plain_text().len() > 15 * 1024 * 1024);
    assert!(snap.coordinate_bytes() <= console_snapshot::MAX_COORDINATE_BYTES);
    eprintln!(
        "near-limit: text={} coordinate_capacity={} capture={capture:?} search={search:?}",
        snap.plain_text().len(),
        snap.coordinate_bytes()
    );
    let input = cell.repeat(1_000_000);
    let parsed = parser(&input, 1000, 1000, 0);
    let screen = parsed.screen().clone();
    drop(parsed);
    drop(input);
    let start = Instant::now();
    assert_eq!(
        ConsoleSnapshot::capture(screen, &cancelled()).unwrap_err(),
        SnapshotError::TextLimit
    );
    eprintln!("oversize rejection: {:?}", start.elapsed());
}

#[test]
fn unused_screen_rows_are_omitted_but_explicit_blanks_and_below_cursor_text_remain() {
    assert_eq!(snapshot("hello", 24, 80).plain_text(), "hello");
    assert_eq!(snapshot("hello\r\n\r\n", 24, 80).plain_text(), "hello\n\n");
    assert_eq!(
        snapshot("top\r\n\r\nbottom\x1b[H", 24, 80).plain_text(),
        "top\n\nbottom"
    );
    assert_eq!(snapshot("", 24, 80).plain_text(), "");
    let snap = snapshot("", 24, 80);
    assert_eq!(snap.rows().len(), 1);
    assert_eq!(snap.lines().len(), 1);
}

#[test]
fn dense_unicode_mapping_has_its_own_budget_even_below_text_limit() {
    let input = "界".repeat(4096);
    let screen = parser(&input, 8, 1024, 0).screen().clone();
    let limits = SnapshotLimits {
        text_bytes: 1024 * 1024,
        coordinate_bytes: 16 * 1024,
    };
    assert_eq!(
        ConsoleSnapshot::capture_with_limits(screen, &cancelled(), limits).unwrap_err(),
        SnapshotError::CoordinateLimit
    );
}

#[test]
fn clone_preflight_uses_configured_history_bound_without_mutating_viewport() {
    use console_snapshot::estimate_screen_capture;
    let mut parsed = parser("hello\r\nworld", 24, 120, 10_000);
    parsed.screen_mut().set_scrollback(1);
    let before = parsed.screen().scrollback();
    let estimate = estimate_screen_capture(parsed.screen(), 10_000).unwrap();
    assert!(estimate > 38_000_000 && estimate < 40_000_000);
    assert_eq!(parsed.screen().scrollback(), before);
    let parsed = parser("", 24, 500, 10_000);
    assert_eq!(
        estimate_screen_capture(parsed.screen(), 10_000).unwrap_err(),
        SnapshotError::CaptureLimit
    );
    assert_eq!(
        estimate_screen_capture(parsed.screen(), usize::MAX).unwrap_err(),
        SnapshotError::CaptureLimit
    );
}

#[test]
#[ignore]
fn realistic_history_clone_measurement() {
    use std::time::Instant;
    let row = "x".repeat(118) + "\r\n";
    let parsed = parser(&row.repeat(10_024), 24, 120, 10_000);
    let estimate = console_snapshot::estimate_screen_capture(parsed.screen(), 10_000).unwrap();
    let start = Instant::now();
    let screen = parsed.screen().clone();
    let clone = start.elapsed();
    let start = Instant::now();
    let snap = ConsoleSnapshot::capture(screen, &cancelled()).unwrap();
    let capture = start.elapsed();
    assert_eq!(snap.rows().len(), 10_024);
    eprintln!(
        "realistic history: estimate={estimate} clone={clone:?} extraction={capture:?} text={} coordinate_capacity={}",
        snap.plain_text().len(),
        snap.coordinate_bytes()
    );
}

#[test]
fn mapping_after_a_resize_stays_inside_captured_column_bounds() {
    let mut parsed = parser("ab界", 1, 4, 0);
    parsed.screen_mut().set_size(1, 3);
    let snap = ConsoleSnapshot::capture(parsed.screen().clone(), &cancelled()).unwrap();
    for range in snap.cell_ranges(0..snap.plain_text().len()) {
        assert!(range.columns.end <= snap.columns());
    }
}

#[test]
fn wider_scrollback_survives_shrink_with_original_soft_wrap_and_cell_coordinates() {
    let mut parsed = parser("abcdefghijklmnopqrstuvwx\r\nTAIL\r\nEND", 2, 12, 100);
    parsed.screen_mut().set_size(2, 4);
    parsed.screen_mut().set_scrollback(usize::MAX);
    assert!(
        parsed.screen().cell(0, 11).is_some(),
        "vt100 retains original history width"
    );
    let offset = parsed.screen().scrollback();
    let snap = ConsoleSnapshot::capture(parsed.screen().clone(), &cancelled()).unwrap();
    assert_eq!(snap.plain_text(), "abcdefghijklmnopqrstuvwx\nTAIL\nEND");
    assert_eq!(snap.columns(), 12);
    assert_eq!(
        snap.rows()
            .iter()
            .map(|row| row.columns)
            .collect::<Vec<_>>(),
        vec![12, 12, 4, 4]
    );
    assert!(snap.rows()[0].wrapped);
    assert_eq!(snap.lines()[0].rows, 0..2);
    let found = snap.search("jklMNO", &cancelled(), |_, _| true).unwrap();
    let cells: Vec<_> = snap.cell_ranges(found.matches[0].bytes.clone()).collect();
    assert_eq!(cells[0].columns, 9..12);
    assert_eq!(cells[1].columns, 0..3);
    assert_eq!(parsed.screen().scrollback(), offset);
}

#[test]
fn narrow_scrollback_preserves_its_original_padding_after_expansion() {
    let mut parsed = parser("abcdefgh\r\nnext\r\nlast", 2, 4, 100);
    parsed.screen_mut().set_size(2, 12);
    let snap = ConsoleSnapshot::capture(parsed.screen().clone(), &cancelled()).unwrap();
    assert_eq!(snap.plain_text(), "abcdefgh\nnext\nlast");
    assert_eq!(snap.rows()[0].columns, 4);
    assert_eq!(snap.rows()[1].columns, 4);
    assert_eq!(snap.columns(), 12);
}

#[test]
fn resized_history_keeps_wide_combining_and_trailing_cells() {
    let mut parsed = parser("ab界e\u{301} XYZ\r\nnewrow\r\nEND", 2, 12, 100);
    parsed.screen_mut().set_size(2, 4);
    let snap = ConsoleSnapshot::capture(parsed.screen().clone(), &cancelled()).unwrap();
    assert_eq!(snap.plain_text(), "ab界e\u{301} XYZ\nnewr\nEND");
    let found = snap
        .search("E\u{301} XYZ", &cancelled(), |_, _| true)
        .unwrap();
    assert_eq!(
        snap.cell_ranges(found.matches[0].bytes.clone())
            .next()
            .unwrap()
            .columns,
        4..9
    );
}

#[test]
fn historical_width_preflight_prevents_clone_underestimate_after_shrink() {
    use console_snapshot::{estimate_screen_capture, estimate_screen_capture_with_max_columns};
    let mut parsed = parser("", 24, 500, 10_000);
    parsed.screen_mut().set_size(24, 4);
    assert!(
        estimate_screen_capture(parsed.screen(), 10_000).is_ok(),
        "current width alone is not a historical bound"
    );
    assert_eq!(
        estimate_screen_capture_with_max_columns(parsed.screen(), 10_000, 500).unwrap_err(),
        SnapshotError::CaptureLimit
    );
    let parsed = parser("", 24, 120, 10_000);
    assert_eq!(
        estimate_screen_capture_with_max_columns(parsed.screen(), 10_000, 1).unwrap(),
        estimate_screen_capture(parsed.screen(), 10_000).unwrap()
    );
}
