//! Explicit export of a frozen retained-terminal snapshot, never a raw byte stream.
//!
//! Preparing a request and its default destination performs no filesystem writes.
//! Submission creates only the workspace's immediate `flimru-logs` directory;
//! edited destinations otherwise require an existing parent. Relative destinations
//! resolve against the absolute workspace supplied at capture time. Log contents
//! are exported in full and are **not** secret-redacted.
use crate::console_snapshot::ConsoleSnapshot;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

pub const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DESTINATION_BYTES: usize = 4096;
const MAX_LABEL_BYTES: usize = 256;
const WRITE_CHUNK_BYTES: usize = 64 * 1024;
static NEXT_FILENAME: AtomicU64 = AtomicU64::new(0);

/// Only bounded display labels are retained: no environment or launch arguments.
#[derive(Clone, Debug)]
pub struct ExportMetadata {
    captured_unix_seconds: u64,
    device: Option<String>,
    configuration: Option<String>,
    outcome: Option<String>,
}

impl ExportMetadata {
    /// Capture time is explicitly UTC Unix seconds, avoiding local timezone state.
    pub fn new(
        captured_unix_seconds: u64,
        device: Option<&str>,
        configuration: Option<&str>,
        outcome: Option<&str>,
    ) -> Self {
        Self {
            captured_unix_seconds,
            device: device.and_then(sanitize_label),
            configuration: configuration.and_then(sanitize_label),
            outcome: outcome.and_then(sanitize_label),
        }
    }

    fn header(&self) -> String {
        let mut header = format!(
            "flimru retained console snapshot\nCaptured: {} UTC Unix seconds\n",
            self.captured_unix_seconds
        );
        for (name, value) in [
            ("Device", &self.device),
            ("Configuration", &self.configuration),
            ("Outcome", &self.outcome),
        ] {
            if let Some(value) = value {
                header.push_str(name);
                header.push_str(": ");
                header.push_str(value);
                header.push('\n');
            }
        }
        header.push('\n');
        header
    }
}

/// Generate a unique, safe suggestion without creating directories or files.
/// Publication still uses create-new semantics if this name already exists.
pub fn default_destination(
    workspace: &Path,
    metadata: &ExportMetadata,
) -> Result<PathBuf, ExportError> {
    validate_workspace(workspace)?;
    let device = filename_label(metadata.device.as_deref().unwrap_or("console"));
    let configuration = filename_label(metadata.configuration.as_deref().unwrap_or("run"));
    let sequence = NEXT_FILENAME.fetch_add(1, Ordering::Relaxed);
    let filename = format!(
        "flimru-{}-{device}-{configuration}-{}-{sequence}.txt",
        metadata.captured_unix_seconds,
        std::process::id()
    );
    let destination = workspace.join("flimru-logs").join(filename);
    validate_path(&destination)?;
    Ok(destination)
}

#[derive(Debug)]
pub struct ExportRequest {
    snapshot: Arc<ConsoleSnapshot>,
    header: String,
    destination: PathBuf,
    create_default_parent: bool,
    bytes: usize,
}

impl ExportRequest {
    /// Freezes source, metadata and destination. No query/filter is accepted:
    /// exports always include the entire captured snapshot's plain UTF-8 text.
    pub fn new(
        workspace: &Path,
        destination: &Path,
        snapshot: Arc<ConsoleSnapshot>,
        metadata: ExportMetadata,
    ) -> Result<Self, ExportError> {
        validate_workspace(workspace)?;
        validate_path(destination)?;
        let destination = if destination.is_absolute() {
            destination.to_path_buf()
        } else {
            workspace.join(destination)
        };
        validate_path(&destination)?;
        if destination.file_name().is_none() {
            return Err(ExportError::InvalidInput("destination needs a filename"));
        }
        let header = metadata.header();
        let bytes = validate_size(header.len(), snapshot.plain_text().len())?;
        let create_default_parent = destination.parent() == Some(&workspace.join("flimru-logs"));
        Ok(Self {
            snapshot,
            header,
            destination,
            create_default_parent,
            bytes,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SavedExport {
    pub path: PathBuf,
    pub bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ExportError {
    Busy,
    Cancelled,
    SizeLimit,
    InvalidInput(&'static str),
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
        os_code: Option<i32>,
    },
    WorkerStopped,
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("an export is already pending"),
            Self::Cancelled => f.write_str("console log export cancelled"),
            Self::SizeLimit => f.write_str("console export including its header exceeds 16 MiB"),
            Self::InvalidInput(reason) => write!(f, "invalid export destination: {reason}"),
            Self::Io {
                operation,
                kind,
                os_code,
            } => {
                write!(f, "could not {operation}: {kind}")?;
                if let Some(code) = os_code {
                    write!(f, " (OS error {code})")?;
                }
                Ok(())
            }
            Self::WorkerStopped => f.write_str("console export worker stopped unexpectedly"),
        }
    }
}
impl std::error::Error for ExportError {}

struct ActiveExport {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<SavedExport, ExportError>>,
    thread: JoinHandle<()>,
}

/// Owns at most one job and one bounded completion result. Polling never waits for
/// filesystem work. Cancellation is cooperative; if publication wins the race,
/// the result truthfully reports success. Drop cancels and joins the owned job.
#[derive(Default)]
pub struct ExportWorker {
    active: Option<ActiveExport>,
}

impl ExportWorker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, request: ExportRequest) -> Result<(), ExportError> {
        if self.active.is_some() {
            return Err(ExportError::Busy);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("console-log-export".into())
            .spawn(move || {
                let result = write_export(request, &worker_cancelled);
                let _ = sender.send(result);
            })
            .map_err(|error| io_error("start export worker", error))?;
        self.active = Some(ActiveExport {
            cancelled,
            receiver,
            thread,
        });
        Ok(())
    }

    /// Includes completed work until its single result is consumed by `poll`.
    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    pub fn poll(&mut self) -> Option<Result<SavedExport, ExportError>> {
        let active = self.active.as_ref()?;
        let result = match active.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(ExportError::WorkerStopped),
        };
        // The worker sends after all writing and cleanup. It has no remaining
        // filesystem work; joining only reaps its owned thread.
        if let Some(active) = self.active.take() {
            let _ = active.thread.join();
        }
        Some(result)
    }

    pub fn cancel(&mut self) {
        if let Some(active) = &self.active {
            active.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Final owner shutdown only: cancel, join and acknowledge the actual
    /// publication result. Normal UI cancellation should use `cancel` + `poll`.
    pub fn shutdown(&mut self) -> Option<Result<SavedExport, ExportError>> {
        self.cancel();
        let active = self.active.take()?;
        let _ = active.thread.join();
        Some(
            active
                .receiver
                .try_recv()
                .unwrap_or(Err(ExportError::WorkerStopped)),
        )
    }
}

impl Drop for ExportWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn write_export(
    request: ExportRequest,
    cancelled: &AtomicBool,
) -> Result<SavedExport, ExportError> {
    check_cancelled(cancelled)?;
    let parent = request
        .destination
        .parent()
        .ok_or(ExportError::InvalidInput(
            "destination needs a parent directory",
        ))?;
    if request.create_default_parent {
        match fs::create_dir(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("create flimru-logs directory", error)),
        }
    }
    check_cancelled(cancelled)?;
    let parent =
        fs::canonicalize(parent).map_err(|error| io_error("open destination directory", error))?;
    let filename = request
        .destination
        .file_name()
        .ok_or(ExportError::InvalidInput("destination needs a filename"))?;
    let destination = parent.join(filename);
    // tempfile's create-new random file is owner-readable/writable (0600) on
    // Unix. It lives beside the destination so publication stays on one volume.
    let mut temporary = tempfile::Builder::new()
        .prefix(".flimru-export-")
        .suffix(".partial")
        .tempfile_in(&parent)
        .map_err(|error| io_error("create temporary export", error))?;
    write_chunks(
        temporary.as_file_mut(),
        request.header.as_bytes(),
        cancelled,
    )?;
    write_chunks(
        temporary.as_file_mut(),
        request.snapshot.plain_text().as_bytes(),
        cancelled,
    )?;
    temporary
        .as_file_mut()
        .flush()
        .map_err(|error| io_error("flush export", error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| io_error("sync export", error))?;
    check_cancelled(cancelled)?;
    // A hard link atomically publishes the complete file and fails if *any*
    // destination entry exists, including a dangling symlink. Retain the owned
    // NamedTempFile guard so its temporary name is removed on success as well
    // as every error/cancellation path. Filesystems without hard-link support
    // return a visible error rather than falling back to an overwriting rename.
    fs::hard_link(temporary.path(), &destination)
        .map_err(|error| io_error("publish export (destination must not exist)", error))?;
    drop(temporary);
    Ok(SavedExport {
        path: destination,
        bytes: request.bytes,
    })
}

fn write_chunks(
    writer: &mut impl Write,
    bytes: &[u8],
    cancelled: &AtomicBool,
) -> Result<(), ExportError> {
    for chunk in bytes.chunks(WRITE_CHUNK_BYTES) {
        check_cancelled(cancelled)?;
        writer
            .write_all(chunk)
            .map_err(|error| io_error("write export", error))?;
    }
    check_cancelled(cancelled)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ExportError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(ExportError::Cancelled)
    } else {
        Ok(())
    }
}

fn io_error(operation: &'static str, error: io::Error) -> ExportError {
    // Do not expose arbitrary OS/custom error strings, source text or path data.
    ExportError::Io {
        operation,
        kind: error.kind(),
        os_code: error.raw_os_error(),
    }
}

fn validate_workspace(workspace: &Path) -> Result<(), ExportError> {
    if !workspace.is_absolute() {
        return Err(ExportError::InvalidInput("workspace must be absolute"));
    }
    validate_path(workspace)
}

fn validate_path(path: &Path) -> Result<(), ExportError> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_encoded_bytes().len() > MAX_DESTINATION_BYTES
    {
        return Err(ExportError::InvalidInput("path must contain 1–4096 bytes"));
    }
    if path.to_string_lossy().chars().any(char::is_control) {
        return Err(ExportError::InvalidInput(
            "path contains control characters",
        ));
    }
    Ok(())
}

fn validate_size(header: usize, text: usize) -> Result<usize, ExportError> {
    header
        .checked_add(text)
        .filter(|bytes| *bytes <= MAX_EXPORT_BYTES)
        .ok_or(ExportError::SizeLimit)
}

fn sanitize_label(label: &str) -> Option<String> {
    let mut out = String::new();
    // Bound inspection as well as the final allocation for hostile input.
    for ch in label.chars().take(MAX_LABEL_BYTES) {
        if ch.is_control()
            || matches!(
                ch,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '.'
            )
        {
            continue;
        }
        if out.len() + ch.len_utf8() > MAX_LABEL_BYTES {
            break;
        }
        out.push(ch);
    }
    let out = out.trim().to_owned();
    if out.is_empty() { None } else { Some(out) }
}

fn filename_label(label: &str) -> String {
    let mut out = String::new();
    for ch in label.chars().take(48) {
        let ch = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch
        } else {
            '-'
        };
        if ch != '-' || !out.ends_with('-') {
            out.push(ch);
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "console".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_counts_toward_limit_and_overflow_is_rejected() {
        assert_eq!(
            validate_size(20, MAX_EXPORT_BYTES - 20),
            Ok(MAX_EXPORT_BYTES)
        );
        assert_eq!(
            validate_size(20, MAX_EXPORT_BYTES - 19),
            Err(ExportError::SizeLimit)
        );
        assert_eq!(validate_size(1, usize::MAX), Err(ExportError::SizeLimit));
    }

    #[test]
    fn chunked_writer_observes_cancellation_between_writes() {
        struct CancellingWriter<'a>(&'a AtomicBool, usize);
        impl Write for CancellingWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.1 += bytes.len();
                self.0.store(true, Ordering::Relaxed);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cancelled = AtomicBool::new(false);
        let mut writer = CancellingWriter(&cancelled, 0);
        assert_eq!(
            write_chunks(&mut writer, &vec![0; WRITE_CHUNK_BYTES * 3], &cancelled),
            Err(ExportError::Cancelled)
        );
        assert_eq!(writer.1, WRITE_CHUNK_BYTES);
    }

    #[test]
    fn error_display_excludes_arbitrary_os_payloads() {
        let error = io_error("write export", io::Error::other("secret source payload"));
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn cancelled_before_writing_creates_neither_directory_nor_partial() {
        let workspace = tempfile::tempdir().unwrap();
        let parser = vt100::Parser::new(1, 8, 0);
        let snapshot = Arc::new(
            ConsoleSnapshot::capture(parser.screen().clone(), &AtomicBool::new(false)).unwrap(),
        );
        let metadata = ExportMetadata::new(42, None, None, None);
        let destination = default_destination(workspace.path(), &metadata).unwrap();
        let request =
            ExportRequest::new(workspace.path(), &destination, snapshot, metadata).unwrap();
        assert_eq!(
            write_export(request, &AtomicBool::new(true)),
            Err(ExportError::Cancelled)
        );
        assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 0);
    }
}
