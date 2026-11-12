#![allow(clippy::unwrap_used)]
#[path = "../src/console_review.rs"]
mod console_review;
#[path = "../src/console_snapshot.rs"]
mod console_snapshot;

use console_review::{
    ReviewData, ReviewError, ReviewTicket, ReviewUpdate, ReviewWorker, Severity, SeverityFilter,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

fn screen(text: &str, columns: u16) -> vt100::Screen {
    let mut parser = vt100::Parser::new(24, columns, 100);
    parser.process(text.as_bytes());
    parser.screen().clone()
}
fn classify(line: &str) -> Option<Severity> {
    if line.starts_with("ERROR") {
        Some(Severity::Error)
    } else if line.starts_with("WARN") {
        Some(Severity::Warning)
    } else {
        None
    }
}
fn wait_update(worker: &mut ReviewWorker) -> ReviewUpdate {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(update) = worker.take_update() {
            return update;
        }
        assert!(Instant::now() < deadline, "review worker did not finish");
        thread::sleep(Duration::from_millis(1));
    }
}
fn data(worker: &mut ReviewWorker, ticket: ReviewTicket) -> ReviewData {
    let update = wait_update(worker);
    assert_eq!(update.ticket, ticket);
    update.result.unwrap()
}

#[test]
fn capture_search_and_filters_share_exact_immutable_snapshot() {
    let mut worker = ReviewWorker::new(classify).unwrap();
    let ticket = worker
        .capture(
            42,
            screen("ERROR abcdefgh\r\nWARN value\r\n  stack\r\nother", 8),
            "ABCD".into(),
            SeverityFilter::Errors,
        )
        .unwrap();
    let first = data(&mut worker, ticket);
    assert_eq!(
        first.snapshot.plain_text(),
        "ERROR abcdefgh\nWARN value\n  stack\nother"
    );
    assert_eq!(first.rows.len(), 2);
    assert_eq!(first.rows.snapshot_row(0), Some(0));
    assert_eq!(first.rows.snapshot_row(1), Some(1));
    assert_eq!(first.rows.snapshot_row(2), None);
    assert_eq!(first.rows.displayed_row(1), Some(1));
    assert_eq!(first.rows.displayed_row(2), None);
    assert_eq!(first.matches.matches.len(), 1);
    assert_eq!(first.snapshot.columns(), 8);
    assert_eq!(first.snapshot.row_text(0), Some("ERROR ab"));
    assert_eq!(first.snapshot.line_text(0), Some("ERROR abcdefgh"));
    assert_eq!(
        first
            .snapshot
            .cell_ranges(first.matches.matches[0].bytes.clone())
            .count(),
        2
    );
    assert_eq!(first.query, "ABCD");
    assert_eq!(first.filter, SeverityFilter::Errors);
    let ticket = worker
        .search(String::new(), SeverityFilter::Warnings)
        .unwrap();
    let warnings = data(&mut worker, ticket);
    assert!(Arc::ptr_eq(&first.snapshot, &warnings.snapshot));
    assert_eq!(warnings.rows.len(), 2);
    assert_eq!(warnings.rows.snapshot_row(0), Some(2));
    assert_eq!(warnings.rows.snapshot_row(1), Some(3));
    assert!(warnings.matches.matches.is_empty());
    assert!(warnings.rows.allocated_bytes() <= console_review::MAX_VISIBLE_ROW_BYTES);
    let ticket = worker
        .search("stack".into(), SeverityFilter::ErrorsAndWarnings)
        .unwrap();
    let both = data(&mut worker, ticket);
    assert_eq!(both.rows.len(), 4);
    assert!(both.matches.matches.is_empty());
    let ticket = worker.search("STACK".into(), SeverityFilter::All).unwrap();
    let all = data(&mut worker, ticket);
    assert_eq!(all.rows.len(), all.snapshot.rows().len());
    assert_eq!(all.rows.allocated_bytes(), 0);
    assert_eq!(all.matches.matches.len(), 1);
}

#[test]
fn empty_filter_is_distinct_from_no_query_matches() {
    let mut worker = ReviewWorker::new(classify).unwrap();
    let ticket = worker
        .capture(
            1,
            screen("plain text", 80),
            "none".into(),
            SeverityFilter::Errors,
        )
        .unwrap();
    let result = data(&mut worker, ticket);
    assert!(result.rows.is_empty());
    assert!(result.matches.matches.is_empty());
    let ticket = worker.search("none".into(), SeverityFilter::All).unwrap();
    let result = data(&mut worker, ticket);
    assert!(!result.rows.is_empty());
    assert!(result.matches.matches.is_empty());
}

struct Gate {
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    blocked: AtomicBool,
    calls: AtomicUsize,
}
static GATE: OnceLock<Gate> = OnceLock::new();
fn gated_classify(line: &str) -> Option<Severity> {
    let gate = GATE.get().unwrap();
    gate.calls.fetch_add(1, Ordering::Relaxed);
    if !gate.blocked.swap(true, Ordering::Relaxed) {
        gate.entered.send(()).unwrap();
        gate.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    }
    classify(line)
}

#[test]
fn rapid_queries_coalesce_and_replacement_source_cannot_publish_stale_work() {
    let (entered, entered_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    GATE.set(Gate {
        entered,
        release: Mutex::new(release_rx),
        blocked: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    })
    .ok()
    .unwrap();
    let mut worker = ReviewWorker::new(gated_classify).unwrap();
    let first = worker
        .capture(
            10,
            screen("ERROR old", 80),
            "old".into(),
            SeverityFilter::Errors,
        )
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut latest = first;
    for i in 0..100 {
        latest = worker
            .search(format!("query-{i}"), SeverityFilter::Errors)
            .unwrap();
    }
    assert!(latest.generation > first.generation);
    assert_eq!(latest.snapshot, first.snapshot);
    let replacement = worker
        .capture(
            11,
            screen("WARN new", 80),
            String::new(),
            SeverityFilter::Warnings,
        )
        .unwrap();
    let latest = worker
        .search("NEW".into(), SeverityFilter::Warnings)
        .unwrap();
    assert_eq!(latest.snapshot, replacement.snapshot);
    release.send(()).unwrap();
    let result = data(&mut worker, latest);
    assert_eq!(result.snapshot.plain_text(), "WARN new");
    assert_eq!(result.matches.matches.len(), 1);
    assert!(
        GATE.get().unwrap().calls.load(Ordering::Relaxed) <= 3,
        "intermediate queries should not be processed"
    );
    assert!(worker.take_update().is_none());
}

#[test]
fn clear_cancels_pending_capture_and_invalidates_unconsumed_results() {
    let mut worker = ReviewWorker::new(classify).unwrap();
    worker
        .capture(
            7,
            screen("ERROR old", 80),
            String::new(),
            SeverityFilter::Errors,
        )
        .unwrap();
    worker.clear();
    assert_eq!(
        worker
            .search("old".into(), SeverityFilter::All)
            .unwrap_err(),
        ReviewError::NoSnapshot
    );
    assert!(worker.take_update().is_none());
    let ticket = worker
        .capture(7, screen("new", 80), String::new(), SeverityFilter::All)
        .unwrap();
    assert_eq!(data(&mut worker, ticket).snapshot.plain_text(), "new");
    let old = worker.search("new".into(), SeverityFilter::All).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(update) = worker.take_update() {
            assert_eq!(update.ticket, old);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    worker.clear();
    assert!(worker.take_update().is_none());
}

#[test]
fn query_rejection_preserves_current_review_and_shutdown_is_idempotent() {
    let mut worker = ReviewWorker::new(classify).unwrap();
    let ticket = worker
        .capture(2, screen("hello", 80), String::new(), SeverityFilter::All)
        .unwrap();
    assert_eq!(
        worker
            .search("界".repeat(513), SeverityFilter::All)
            .unwrap_err(),
        ReviewError::Snapshot(console_snapshot::SnapshotError::QueryLimit)
    );
    assert_eq!(data(&mut worker, ticket).snapshot.plain_text(), "hello");
    let ticket = worker
        .search("界".repeat(512), SeverityFilter::All)
        .unwrap();
    assert!(data(&mut worker, ticket).matches.matches.is_empty());
    let start = Instant::now();
    worker.shutdown();
    worker.shutdown();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        worker
            .search(String::new(), SeverityFilter::All)
            .unwrap_err(),
        ReviewError::Stopped
    );
    assert_eq!(
        worker
            .capture(3, screen("bye", 80), String::new(), SeverityFilter::All)
            .unwrap_err(),
        ReviewError::Stopped
    );
}

#[test]
fn clear_releases_worker_snapshot_while_external_export_arc_can_survive() {
    let mut worker = ReviewWorker::new(classify).unwrap();
    let ticket = worker
        .capture(88, screen("frozen", 80), String::new(), SeverityFilter::All)
        .unwrap();
    let result = data(&mut worker, ticket);
    let exported = Arc::clone(&result.snapshot);
    let weak = Arc::downgrade(&result.snapshot);
    drop(result);
    worker.clear();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Arc::strong_count(&exported) != 1 {
        assert!(
            Instant::now() < deadline,
            "worker retained cleared snapshot"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(exported.plain_text(), "frozen");
    drop(exported);
    assert!(weak.upgrade().is_none());
}

#[test]
fn worker_filters_and_searches_wider_history_after_terminal_shrink() {
    let mut parser = vt100::Parser::new(2, 20, 100);
    parser.process(b"ERROR abcdefghijk\r\nnext\r\nEND");
    parser.screen_mut().set_size(2, 4);
    let mut worker = ReviewWorker::new(classify).unwrap();
    let ticket = worker
        .capture(
            90,
            parser.screen().clone(),
            "FGH".into(),
            SeverityFilter::Errors,
        )
        .unwrap();
    let result = data(&mut worker, ticket);
    assert_eq!(result.snapshot.plain_text(), "ERROR abcdefghijk\nnext\nEND");
    assert_eq!(result.snapshot.columns(), 20);
    assert_eq!(result.snapshot.rows()[0].columns, 20);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.matches.matches.len(), 1);
    assert_eq!(
        result
            .snapshot
            .cell_ranges(result.matches.matches[0].bytes.clone())
            .next()
            .unwrap()
            .columns,
        11..14
    );
}
