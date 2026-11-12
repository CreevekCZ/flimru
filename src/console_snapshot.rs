//! Immutable, bounded console text shared by review, search and export workers.
//!
//! Capture consumes a worker-owned screen. It never changes a live screen's viewport.
//! Unused bottom grid rows are omitted; the cursor row and any populated rows
//! below it remain, preserving explicit cursor-advanced blank lines.
//! Physical rows retain their captured width/wrapping; callers may clip or scroll them
//! after a resize. Logical lines join soft wraps and preserve explicit blank rows.
use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_CAPTURE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_COORDINATE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_QUERY_CHARS: usize = 512;
pub const MAX_MATCHES: usize = 10_000;

/// Smaller limits are useful for constrained callers. Larger values are clamped
/// to the hard budgets above. Coordinate limits include allocated vector capacity.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotLimits {
    pub text_bytes: usize,
    pub coordinate_bytes: usize,
}
impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            text_bytes: MAX_TEXT_BYTES,
            coordinate_bytes: MAX_COORDINATE_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Cancelled,
    CaptureLimit,
    TextLimit,
    CoordinateLimit,
    QueryLimit,
    Allocation,
}
impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "console snapshot operation cancelled",
            Self::CaptureLimit => {
                "retained console screen exceeds the 128 MiB capture estimate limit"
            }
            Self::TextLimit => {
                "retained console text exceeds the snapshot text limit (at most 16 MiB)"
            }
            Self::CoordinateLimit => {
                "retained console coordinates exceed the snapshot memory limit (at most 32 MiB)"
            }
            Self::QueryLimit => "console search query exceeds 512 characters",
            Self::Allocation => "could not allocate console snapshot memory",
        })
    }
}
impl std::error::Error for SnapshotError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotRow {
    pub bytes: Range<usize>,
    pub logical_line: usize,
    /// Original physical width, which may exceed the terminal's current width.
    pub columns: u16,
    pub wrapped: bool,
    runs: Range<usize>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalLine {
    pub bytes: Range<usize>,
    pub rows: Range<usize>,
}

// ASCII runs map bytes one-to-one to columns. Other runs describe one cell,
// including its combining marks; wide continuation cells never get a run.
#[derive(Clone, Copy, Debug)]
struct CellRun {
    start: u32,
    end: u32,
    column: u16,
    width: u16, // zero means an ASCII run
}

#[derive(Debug)]
pub struct ConsoleSnapshot {
    text: String,
    rows: Vec<SnapshotRow>,
    lines: Vec<LogicalLine>,
    runs: Vec<CellRun>,
    columns: u16,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellRange {
    pub row: usize,
    /// End-exclusive terminal columns, including both halves of wide cells.
    pub columns: Range<u16>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub logical_line: usize,
    /// Original UTF-8 bytes, never offsets into case-converted text.
    pub bytes: Range<usize>,
}
#[derive(Debug, Default)]
pub struct SearchResults {
    pub matches: Vec<SearchMatch>,
    /// At least one additional match exists; the stored count is a lower bound.
    pub capped: bool,
}

/// Preflight for sources known never to have had wider historical rows.
///
/// **After a resize this is not a safe clone bound by itself:** vt100 retains
/// scrollback at its original widths. Use `estimate_screen_capture_with_max_columns`
/// with a caller-maintained maximum width instead. History must be its configured
/// capacity or another upper bound, never the current viewport scrollback offset.
pub fn estimate_screen_capture(
    screen: &vt100::Screen,
    history_rows_upper_bound: usize,
) -> Result<usize, SnapshotError> {
    estimate_screen_capture_with_max_columns(screen, history_rows_upper_bound, screen.size().1)
}

/// Conservative preflight before cloning a screen, without moving its viewport.
/// Track `max_historical_columns = max(initial_columns, every_resize_columns)` for
/// the source's lifetime and retain it when the source becomes a saved console.
/// Both visible grids, retained history and row/container overhead are included.
/// The estimate excludes the original live screen and separately bounded output.
/// Passing a value below the current width is safe; the larger width is used.
/// Passing a value below an earlier historical width invalidates the bound.
pub fn estimate_screen_capture_with_max_columns(
    screen: &vt100::Screen,
    history_rows_upper_bound: usize,
    max_historical_columns: u16,
) -> Result<usize, SnapshotError> {
    let (rows, columns) = screen.size();
    let retained_rows = history_rows_upper_bound.saturating_add(usize::from(rows) * 2);
    let row_bytes = estimated_row_bytes(columns.max(max_historical_columns));
    let estimate = retained_rows.saturating_mul(row_bytes).saturating_add(4096);
    check_capture_estimate(estimate)?;
    Ok(estimate)
}

fn estimated_row_bytes(columns: u16) -> usize {
    usize::from(columns)
        .saturating_mul(size_of::<vt100::Cell>())
        .saturating_add(64)
}
fn check_capture_estimate(estimate: usize) -> Result<(), SnapshotError> {
    if estimate > MAX_CAPTURE_BYTES {
        Err(SnapshotError::CaptureLimit)
    } else {
        Ok(())
    }
}

// Row::get is exposed by Screen::cell and checks the retained row's actual length,
// rather than current screen columns. Widths fit u16; a width of 65535 has valid
// indices 0..65535, so this lower-bound search also handles that maximum.
fn physical_row_columns(screen: &vt100::Screen, row: u16) -> u16 {
    let mut low = 0;
    let mut high = u16::MAX;
    while low < high {
        let middle = low + (high - low) / 2;
        if screen.cell(row, middle).is_some() {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

impl ConsoleSnapshot {
    pub fn capture(screen: vt100::Screen, cancelled: &AtomicBool) -> Result<Self, SnapshotError> {
        Self::capture_with_limits(screen, cancelled, SnapshotLimits::default())
    }

    pub fn capture_with_limits(
        mut screen: vt100::Screen,
        cancelled: &AtomicBool,
        limits: SnapshotLimits,
    ) -> Result<Self, SnapshotError> {
        check_cancelled(cancelled)?;
        let limits = SnapshotLimits {
            text_bytes: limits.text_bytes.min(MAX_TEXT_BYTES),
            coordinate_bytes: limits.coordinate_bytes.min(MAX_COORDINATE_BYTES),
        };
        let (height, columns) = screen.size();
        screen.set_scrollback(0);
        let mut last_row = screen.cursor_position().0;
        'content: for row in (last_row.saturating_add(1)..height).rev() {
            check_cancelled(cancelled)?;
            for col in 0..columns {
                if col % 1024 == 0 {
                    check_cancelled(cancelled)?;
                }
                if screen.cell(row, col).is_some_and(vt100::Cell::has_contents) {
                    last_row = row;
                    break 'content;
                }
            }
        }
        let visible_rows = usize::from(last_row)
            .saturating_add(1)
            .min(usize::from(height));
        screen.set_scrollback(usize::MAX);
        let history_rows = screen.scrollback();
        // Actual historical widths are discovered below. The live/alternate
        // drawing grids have both been resized to current columns by vt100.
        let mut capture_estimate = estimate_screen_capture(&screen, 0)?;
        let total_rows = history_rows.saturating_add(visible_rows);
        let mut out = Self {
            text: String::new(),
            rows: Vec::new(),
            lines: Vec::new(),
            runs: Vec::new(),
            columns,
        };
        let mut line_start = 0;
        let mut line_row = 0;
        for row in 0..total_rows {
            check_cancelled(cancelled)?;
            screen.set_scrollback(history_rows.saturating_sub(row));
            let local_row = row.saturating_sub(history_rows) as u16;
            let row_columns = if row < history_rows {
                physical_row_columns(&screen, local_row)
            } else {
                columns
            };
            if row < history_rows {
                capture_estimate =
                    capture_estimate.saturating_add(estimated_row_bytes(row_columns));
                check_capture_estimate(capture_estimate)?;
            }
            out.columns = out.columns.max(row_columns);
            let wrapped = screen.row_wrapped(local_row) && row + 1 < total_rows;
            let start = out.text.len();
            let run_start = out.runs.len();
            // Match selection's rule: retain padding on soft wraps, trim only
            // ASCII trailing spaces on the last physical row of a logical line.
            let mut end_column = row_columns;
            if !wrapped {
                while end_column > 0 {
                    if end_column % 1024 == 0 {
                        check_cancelled(cancelled)?;
                    }
                    let Some(cell) = screen.cell(local_row, end_column - 1) else {
                        break;
                    };
                    if cell.is_wide_continuation()
                        || (cell.has_contents() && cell.contents() != " ")
                    {
                        break;
                    }
                    end_column -= 1;
                }
            }
            for col in 0..end_column {
                if col % 1024 == 0 {
                    check_cancelled(cancelled)?;
                }
                let Some(cell) = screen.cell(local_row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let contents = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };
                let byte_start = out.text.len();
                append_text(&mut out.text, contents, limits.text_bytes)?;
                let ascii = contents.len() == 1 && contents.is_ascii() && !cell.is_wide();
                if ascii && out.runs.len() > run_start {
                    let last_index = out.runs.len() - 1;
                    let last = &mut out.runs[last_index];
                    if last.width == 0
                        && usize::from(last.column) + (last.end - last.start) as usize
                            == usize::from(col)
                    {
                        last.end = out.text.len() as u32;
                        continue;
                    }
                }
                let remaining = limits
                    .coordinate_bytes
                    .saturating_sub(out.coordinate_bytes());
                reserve_coordinate(&mut out.runs, remaining)?;
                out.runs.push(CellRun {
                    start: byte_start as u32,
                    end: out.text.len() as u32,
                    column: col,
                    width: if ascii {
                        0
                    } else if cell.is_wide() {
                        2
                    } else {
                        1
                    },
                });
            }
            let remaining = limits
                .coordinate_bytes
                .saturating_sub(out.coordinate_bytes());
            reserve_coordinate(&mut out.rows, remaining)?;
            out.rows.push(SnapshotRow {
                bytes: start..out.text.len(),
                logical_line: out.lines.len(),
                columns: row_columns,
                wrapped,
                runs: run_start..out.runs.len(),
            });
            if !wrapped {
                let remaining = limits
                    .coordinate_bytes
                    .saturating_sub(out.coordinate_bytes());
                reserve_coordinate(&mut out.lines, remaining)?;
                out.lines.push(LogicalLine {
                    bytes: line_start..out.text.len(),
                    rows: line_row..row + 1,
                });
                if row + 1 < total_rows {
                    append_text(&mut out.text, "\n", limits.text_bytes)?;
                }
                line_start = out.text.len();
                line_row = row + 1;
            }
        }
        Ok(out)
    }

    pub fn plain_text(&self) -> &str {
        &self.text
    }
    pub fn rows(&self) -> &[SnapshotRow] {
        &self.rows
    }
    pub fn lines(&self) -> &[LogicalLine] {
        &self.lines
    }
    /// Maximum captured physical width, including wider pre-resize history.
    pub fn columns(&self) -> u16 {
        self.columns
    }
    pub fn row_text(&self, row: usize) -> Option<&str> {
        self.rows.get(row).map(|row| &self.text[row.bytes.clone()])
    }
    pub fn line_text(&self, line: usize) -> Option<&str> {
        self.lines
            .get(line)
            .map(|line| &self.text[line.bytes.clone()])
    }
    pub fn coordinate_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<SnapshotRow>()
            + self.lines.capacity() * size_of::<LogicalLine>()
            + self.runs.capacity() * size_of::<CellRun>()
    }

    /// Maps text to captured physical cells without allocating a highlight array.
    /// Combining-character substrings highlight their owning cell in full.
    pub fn cell_ranges(&self, bytes: Range<usize>) -> impl Iterator<Item = CellRange> + '_ {
        let start_row = self
            .rows
            .partition_point(|row| row.bytes.end <= bytes.start);
        self.rows
            .iter()
            .enumerate()
            .skip(start_row)
            .take_while(move |(_, row)| row.bytes.start < bytes.end)
            .filter_map(move |(index, row)| {
                let start = row.bytes.start.max(bytes.start);
                let end = row.bytes.end.min(bytes.end);
                if start >= end {
                    return None;
                }
                let runs = &self.runs[row.runs.clone()];
                let first = runs.get(runs.partition_point(|run| run.end as usize <= start))?;
                let last = runs.get(
                    runs.partition_point(|run| (run.start as usize) < end)
                        .checked_sub(1)?,
                )?;
                let left = first.column
                    + if first.width == 0 {
                        (start - first.start as usize) as u16
                    } else {
                        0
                    };
                // A resize may leave a wide head at the captured right edge.
                let right = (usize::from(last.column)
                    + if last.width == 0 {
                        end - last.start as usize
                    } else {
                        usize::from(last.width)
                    })
                .min(usize::from(row.columns)) as u16;
                Some(CellRange {
                    row: index,
                    columns: left..right,
                })
            })
    }

    /// Literal Unicode lowercase matching, with no regex or normalization.
    /// Lowercase expansions (for example İ -> i + combining dot) preserve original
    /// byte offsets. The streaming KMP matcher uses O(query length) scratch space
    /// and supports overlapping matches. Eligibility is assessed per logical line.
    pub fn search(
        &self,
        query: &str,
        cancelled: &AtomicBool,
        mut eligible: impl FnMut(usize, &str) -> bool,
    ) -> Result<SearchResults, SnapshotError> {
        check_cancelled(cancelled)?;
        if query.chars().take(MAX_QUERY_CHARS + 1).count() > MAX_QUERY_CHARS {
            return Err(SnapshotError::QueryLimit);
        }
        let pattern: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        if pattern.is_empty() {
            return Ok(SearchResults::default());
        }
        let mut prefix = vec![0; pattern.len()];
        for i in 1..pattern.len() {
            let mut j = prefix[i - 1];
            while j > 0 && pattern[i] != pattern[j] {
                j = prefix[j - 1];
            }
            if pattern[i] == pattern[j] {
                j += 1;
            }
            prefix[i] = j;
        }
        let mut found = SearchResults::default();
        let mut positions = VecDeque::with_capacity(pattern.len());
        for (line_index, line) in self.lines.iter().enumerate() {
            check_cancelled(cancelled)?;
            let text = &self.text[line.bytes.clone()];
            if !eligible(line_index, text) {
                continue;
            }
            let mut matched = 0;
            positions.clear();
            for (offset, ch) in text.char_indices() {
                if offset % 1024 < 4 {
                    check_cancelled(cancelled)?;
                }
                let original =
                    (line.bytes.start + offset)..(line.bytes.start + offset + ch.len_utf8());
                for folded in ch.to_lowercase() {
                    if positions.len() == pattern.len() {
                        positions.pop_front();
                    }
                    positions.push_back(original.clone());
                    while matched > 0 && folded != pattern[matched] {
                        matched = prefix[matched - 1];
                    }
                    if folded == pattern[matched] {
                        matched += 1;
                    }
                    if matched == pattern.len() {
                        if let Some(first) = positions.front() {
                            let bytes = first.start..original.end;
                            if found.matches.last().is_none_or(|last| last.bytes != bytes) {
                                if found.matches.len() == MAX_MATCHES {
                                    found.capped = true;
                                    return Ok(found);
                                }
                                if found.matches.len() == found.matches.capacity() {
                                    let extra = found
                                        .matches
                                        .capacity()
                                        .max(64)
                                        .min(MAX_MATCHES - found.matches.len());
                                    found
                                        .matches
                                        .try_reserve_exact(extra)
                                        .map_err(|_| SnapshotError::Allocation)?;
                                }
                                found.matches.push(SearchMatch {
                                    logical_line: line_index,
                                    bytes,
                                });
                            }
                        }
                        matched = prefix[matched - 1];
                    }
                }
            }
        }
        Ok(found)
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), SnapshotError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(SnapshotError::Cancelled)
    } else {
        Ok(())
    }
}
fn append_text(text: &mut String, addition: &str, limit: usize) -> Result<(), SnapshotError> {
    let needed = text
        .len()
        .checked_add(addition.len())
        .ok_or(SnapshotError::TextLimit)?;
    if needed > limit {
        return Err(SnapshotError::TextLimit);
    }
    if needed > text.capacity() {
        let target = needed
            .max(text.capacity().max(4096).saturating_mul(2))
            .min(limit);
        text.try_reserve_exact(target - text.len())
            .map_err(|_| SnapshotError::Allocation)?;
        if text.capacity() > limit {
            return Err(SnapshotError::TextLimit);
        }
    }
    text.push_str(addition);
    Ok(())
}
fn reserve_coordinate<T>(items: &mut Vec<T>, remaining: usize) -> Result<(), SnapshotError> {
    if items.len() < items.capacity() {
        return Ok(());
    }
    let count = items.capacity().max(256).min(remaining / size_of::<T>());
    if count == 0 {
        return Err(SnapshotError::CoordinateLimit);
    }
    let old_capacity = items.capacity();
    items
        .try_reserve_exact(count)
        .map_err(|_| SnapshotError::Allocation)?;
    if (items.capacity() - old_capacity) * size_of::<T>() > remaining {
        return Err(SnapshotError::CoordinateLimit);
    }
    Ok(())
}
