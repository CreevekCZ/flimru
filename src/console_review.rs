//! One owned worker for a frozen console capture and coalesced review queries.
//!
//! The UI preflights before cloning a screen, then moves the clone into `capture`.
//! Discard old UI review data on capture/clear. Only the worker's current snapshot,
//! one pending screen, and the latest query/result are retained here; an export may
//! share the same immutable Arc. No PTY or mutable session index enters this API.
use crate::console_snapshot::{ConsoleSnapshot, MAX_QUERY_CHARS, SearchResults, SnapshotError};
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

pub const MAX_VISIBLE_ROW_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SeverityFilter {
    #[default]
    All,
    Errors,
    Warnings,
    ErrorsAndWarnings,
}
impl SeverityFilter {
    fn includes(self, severity: Option<Severity>) -> bool {
        match self {
            Self::All => true,
            Self::Errors => severity == Some(Severity::Error),
            Self::Warnings => severity == Some(Severity::Warning),
            Self::ErrorsAndWarnings => severity.is_some(),
        }
    }
    fn accepts(self, text: &str, classify: fn(&str) -> Option<Severity>) -> bool {
        self == Self::All || self.includes(classify(text))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewTicket {
    pub source: u64,
    pub snapshot: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewError {
    Snapshot(SnapshotError),
    VisibleRowsLimit,
    NoSnapshot,
    Stopped,
    GenerationLimit,
}
impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => error.fmt(f),
            Self::VisibleRowsLimit => {
                f.write_str("filtered console rows exceed the 4 MiB review index limit")
            }
            Self::NoSnapshot => f.write_str("capture a console snapshot before searching"),
            Self::Stopped => f.write_str("console review worker has stopped"),
            Self::GenerationLimit => f.write_str("console review generation limit reached"),
        }
    }
}
impl std::error::Error for ReviewError {}
impl From<SnapshotError> for ReviewError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

/// No per-row allocation for All; filtered rows use bounded four-byte indices.
#[derive(Debug)]
pub enum VisibleRows {
    All(usize),
    Filtered(Vec<u32>),
}
impl VisibleRows {
    pub fn len(&self) -> usize {
        match self {
            Self::All(count) => *count,
            Self::Filtered(rows) => rows.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn snapshot_row(&self, displayed_row: usize) -> Option<usize> {
        match self {
            Self::All(count) => (displayed_row < *count).then_some(displayed_row),
            Self::Filtered(rows) => rows.get(displayed_row).map(|row| *row as usize),
        }
    }
    pub fn displayed_row(&self, snapshot_row: usize) -> Option<usize> {
        match self {
            Self::All(count) => (snapshot_row < *count).then_some(snapshot_row),
            Self::Filtered(rows) => u32::try_from(snapshot_row)
                .ok()
                .and_then(|row| rows.binary_search(&row).ok()),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        match self {
            Self::All(_) => 0,
            Self::Filtered(rows) => rows.capacity() * size_of::<u32>(),
        }
    }
}
#[derive(Debug)]
pub struct ReviewData {
    pub snapshot: Arc<ConsoleSnapshot>,
    pub rows: VisibleRows,
    pub matches: SearchResults,
    pub query: String,
    pub filter: SeverityFilter,
}
#[derive(Debug)]
pub struct ReviewUpdate {
    pub ticket: ReviewTicket,
    pub result: Result<ReviewData, ReviewError>,
}

struct Capture {
    source: u64,
    snapshot: u64,
    screen: vt100::Screen,
    cancelled: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Desired {
    ticket: ReviewTicket,
    query: String,
    filter: SeverityFilter,
    cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
struct Mailbox {
    stopped: bool,
    clear: bool,
    capture: Option<Capture>,
    capture_cancelled: Option<Arc<AtomicBool>>,
    desired: Option<Desired>,
    search_pending: bool,
    result: Option<ReviewUpdate>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<Mailbox>,
    wake: Condvar,
}
impl Shared {
    fn lock(&self) -> MutexGuard<'_, Mailbox> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub struct ReviewWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    generation: u64,
}
impl ReviewWorker {
    /// Classifier must be quick, deterministic and free of blocking I/O. It sees
    /// one logical line, without ANSI formatting; continuation lines are separate.
    pub fn new(classify: fn(&str) -> Option<Severity>) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("console-review".into())
            .spawn(move || run(worker_shared, classify))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            generation: 0,
        })
    }

    pub fn capture(
        &mut self,
        source: u64,
        screen: vt100::Screen,
        query: String,
        filter: SeverityFilter,
    ) -> Result<ReviewTicket, ReviewError> {
        validate_query(&query)?;
        let generation = self.next_generation()?;
        let mut state = self.shared.lock();
        if state.stopped {
            return Err(ReviewError::Stopped);
        }
        cancel(&state);
        let cancelled = Arc::new(AtomicBool::new(false));
        let ticket = ReviewTicket {
            source,
            snapshot: generation,
            generation,
        };
        state.capture = Some(Capture {
            source,
            snapshot: generation,
            screen,
            cancelled: Arc::clone(&cancelled),
        });
        state.capture_cancelled = Some(cancelled);
        state.desired = Some(Desired {
            ticket,
            query,
            filter,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        state.search_pending = false;
        state.result = None;
        self.shared.wake.notify_one();
        Ok(ticket)
    }

    /// Coalesces queries during capture without restarting extraction. Replacing a
    /// search cancels its work and removes any unconsumed older result.
    pub fn search(
        &mut self,
        query: String,
        filter: SeverityFilter,
    ) -> Result<ReviewTicket, ReviewError> {
        validate_query(&query)?;
        let generation = self.next_generation()?;
        let mut state = self.shared.lock();
        if state.stopped {
            return Err(ReviewError::Stopped);
        }
        let desired = state.desired.as_mut().ok_or(ReviewError::NoSnapshot)?;
        desired.cancelled.store(true, Ordering::Relaxed);
        desired.cancelled = Arc::new(AtomicBool::new(false));
        desired.ticket.generation = generation;
        desired.query = query;
        desired.filter = filter;
        let ticket = desired.ticket;
        state.search_pending = true;
        state.result = None;
        self.shared.wake.notify_one();
        Ok(ticket)
    }

    pub fn take_update(&mut self) -> Option<ReviewUpdate> {
        let mut state = self.shared.lock();
        let result = state.result.take()?;
        state
            .desired
            .as_ref()
            .filter(|desired| desired.ticket == result.ticket)
            .map(|_| result)
    }

    /// Cancels review and drops retained worker data on its next wake. Stable
    /// source IDs must be retained when a live console transfers to saved logs.
    pub fn clear(&mut self) {
        let mut state = self.shared.lock();
        cancel(&state);
        state.capture = None;
        state.capture_cancelled = None;
        state.desired = None;
        state.search_pending = false;
        state.result = None;
        state.clear = true;
        self.shared.wake.notify_one();
    }

    /// Idempotent shutdown; wakes an idle worker, cancels active work, and joins.
    pub fn shutdown(&mut self) {
        {
            let mut state = self.shared.lock();
            cancel(&state);
            state.stopped = true;
            state.capture = None;
            state.desired = None;
            state.result = None;
            self.shared.wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn next_generation(&mut self) -> Result<u64, ReviewError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(ReviewError::GenerationLimit)?;
        Ok(self.generation)
    }
}
impl Drop for ReviewWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn validate_query(query: &str) -> Result<(), ReviewError> {
    if query.chars().take(MAX_QUERY_CHARS + 1).count() > MAX_QUERY_CHARS {
        Err(SnapshotError::QueryLimit.into())
    } else {
        Ok(())
    }
}
fn cancel(state: &Mailbox) {
    if let Some(cancelled) = &state.capture_cancelled {
        cancelled.store(true, Ordering::Relaxed);
    }
    if let Some(desired) = &state.desired {
        desired.cancelled.store(true, Ordering::Relaxed);
    }
}

enum Job {
    Clear,
    Capture(Capture),
    Search(Desired),
}
fn run(shared: Arc<Shared>, classify: fn(&str) -> Option<Severity>) {
    let mut active: Option<(u64, u64, Arc<ConsoleSnapshot>)> = None;
    loop {
        let job = {
            let mut state = shared.lock();
            while !state.stopped && !state.clear && state.capture.is_none() && !state.search_pending
            {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.stopped {
                return;
            }
            if state.clear {
                state.clear = false;
                Job::Clear
            } else if let Some(capture) = state.capture.take() {
                Job::Capture(capture)
            } else {
                state.search_pending = false;
                let Some(desired) = state.desired.clone() else {
                    continue;
                };
                Job::Search(desired)
            }
        };
        match job {
            Job::Clear => active = None,
            Job::Capture(capture) => {
                active = None;
                let result = ConsoleSnapshot::capture(capture.screen, &capture.cancelled);
                let mut state = shared.lock();
                let Some(desired) = state.desired.as_ref() else {
                    continue;
                };
                if desired.ticket.source != capture.source
                    || desired.ticket.snapshot != capture.snapshot
                    || state.stopped
                {
                    continue;
                }
                let ticket = desired.ticket;
                match result {
                    Ok(snapshot) => {
                        active = Some((capture.source, capture.snapshot, Arc::new(snapshot)));
                        state.search_pending = true;
                    }
                    Err(error) => {
                        state.search_pending = false;
                        state.result = Some(ReviewUpdate {
                            ticket,
                            result: Err(error.into()),
                        });
                    }
                }
            }
            Job::Search(desired) => {
                let result = active
                    .as_ref()
                    .filter(|(source, snapshot, _)| {
                        *source == desired.ticket.source && *snapshot == desired.ticket.snapshot
                    })
                    .ok_or(ReviewError::NoSnapshot)
                    .and_then(|(_, _, snapshot)| review(Arc::clone(snapshot), &desired, classify));
                let mut state = shared.lock();
                if !state.stopped
                    && !desired.cancelled.load(Ordering::Relaxed)
                    && state
                        .desired
                        .as_ref()
                        .is_some_and(|latest| latest.ticket == desired.ticket)
                {
                    state.result = Some(ReviewUpdate {
                        ticket: desired.ticket,
                        result,
                    });
                }
            }
        }
    }
}

fn review(
    snapshot: Arc<ConsoleSnapshot>,
    desired: &Desired,
    classify: fn(&str) -> Option<Severity>,
) -> Result<ReviewData, ReviewError> {
    let rows = if desired.filter == SeverityFilter::All {
        VisibleRows::All(snapshot.rows().len())
    } else {
        let mut rows = Vec::new();
        for line in snapshot.lines() {
            if desired.cancelled.load(Ordering::Relaxed) {
                return Err(SnapshotError::Cancelled.into());
            }
            if !desired
                .filter
                .accepts(&snapshot.plain_text()[line.bytes.clone()], classify)
            {
                continue;
            }
            for row in line.rows.clone() {
                if row % 1024 == 0 && desired.cancelled.load(Ordering::Relaxed) {
                    return Err(SnapshotError::Cancelled.into());
                }
                if rows.len() == rows.capacity() {
                    let extra = rows
                        .capacity()
                        .max(256)
                        .min(MAX_VISIBLE_ROW_BYTES / size_of::<u32>() - rows.len());
                    if extra == 0 {
                        return Err(ReviewError::VisibleRowsLimit);
                    }
                    rows.try_reserve_exact(extra)
                        .map_err(|_| ReviewError::Snapshot(SnapshotError::Allocation))?;
                    if rows.capacity() * size_of::<u32>() > MAX_VISIBLE_ROW_BYTES {
                        return Err(ReviewError::VisibleRowsLimit);
                    }
                }
                rows.push(u32::try_from(row).map_err(|_| ReviewError::VisibleRowsLimit)?);
            }
        }
        VisibleRows::Filtered(rows)
    };
    let matches = snapshot.search(&desired.query, &desired.cancelled, |line, _| {
        rows.displayed_row(snapshot.lines()[line].rows.start)
            .is_some()
    })?;
    Ok(ReviewData {
        snapshot,
        rows,
        matches,
        query: desired.query.clone(),
        filter: desired.filter,
    })
}
