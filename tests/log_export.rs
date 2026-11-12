#![allow(clippy::unwrap_used)]

use flimru::console_snapshot;
#[path = "../src/log_export.rs"]
mod log_export;

use console_snapshot::ConsoleSnapshot;
use log_export::{ExportError, ExportMetadata, ExportRequest, ExportWorker, default_destination};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn snapshot(text: &str) -> Arc<ConsoleSnapshot> {
    let mut parser = vt100::Parser::new(24, 8, 64);
    parser.process(text.as_bytes());
    Arc::new(ConsoleSnapshot::capture(parser.screen().clone(), &AtomicBool::new(false)).unwrap())
}

fn metadata() -> ExportMetadata {
    ExportMetadata::new(1_799_000_000, Some("Phone"), Some("Debug"), Some("running"))
}

fn request(workspace: &Path, destination: &Path) -> ExportRequest {
    ExportRequest::new(workspace, destination, snapshot("hello"), metadata()).unwrap()
}

fn finish(worker: &mut ExportWorker) -> Result<log_export::SavedExport, ExportError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = worker.poll() {
            return result;
        }
        assert!(Instant::now() < deadline, "export did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn exports_exact_full_frozen_utf8_text_after_filtered_search() {
    let root = tempfile::tempdir().unwrap();
    let snap = snapshot("\x1b[31mERROR ab界e\u{301}cdef\x1b[0m\r\n\r\nWARN x\r\nsecret token");
    let original = snap.plain_text().to_owned();
    let filtered = snap
        .search("ERROR", &AtomicBool::new(false), |_, line| {
            line.starts_with("ERROR")
        })
        .unwrap();
    assert_eq!(filtered.matches.len(), 1);
    let destination = default_destination(root.path(), &metadata()).unwrap();
    let request =
        ExportRequest::new(root.path(), &destination, Arc::clone(&snap), metadata()).unwrap();
    assert!(!root.path().join("flimru-logs").exists());
    let mut worker = ExportWorker::new();
    worker.start(request).unwrap();
    let saved = finish(&mut worker).unwrap();
    assert!(saved.path.is_absolute());
    assert_eq!(saved.path, fs::canonicalize(destination).unwrap());
    let bytes = fs::read(&saved.path).unwrap();
    assert_eq!(saved.bytes, bytes.len());
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(
        text,
        format!(
            "flimru retained console snapshot\nCaptured: 1799000000 UTC Unix seconds\nDevice: Phone\nConfiguration: Debug\nOutcome: running\n\n{original}"
        )
    );
    assert!(text.contains("WARN x\nsecret token"));
    assert!(!text.contains('\x1b'));
    assert_eq!(snap.plain_text(), original);
    assert!(!worker.is_running());
    assert!(worker.poll().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&saved.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn collisions_never_overwrite_or_leave_partial_files() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("existing.txt");
    fs::write(&destination, "existing").unwrap();
    let mut worker = ExportWorker::new();
    worker.start(request(root.path(), &destination)).unwrap();
    assert!(matches!(
        finish(&mut worker),
        Err(ExportError::Io {
            kind: std::io::ErrorKind::AlreadyExists,
            ..
        })
    ));
    assert_eq!(fs::read_to_string(&destination).unwrap(), "existing");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symlink_collisions_preserve_link_and_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let destination = root.path().join("log.txt");
    fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, &destination).unwrap();
    let mut worker = ExportWorker::new();
    worker.start(request(root.path(), &destination)).unwrap();
    assert!(finish(&mut worker).is_err());
    assert!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn edited_relative_paths_resolve_against_workspace_and_missing_parent_fails() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = ExportWorker::new();
    worker
        .start(request(root.path(), Path::new("relative.txt")))
        .unwrap();
    assert_eq!(
        finish(&mut worker).unwrap().path,
        fs::canonicalize(root.path().join("relative.txt")).unwrap()
    );
    worker
        .start(request(root.path(), Path::new("missing/log.txt")))
        .unwrap();
    let error = finish(&mut worker).unwrap_err();
    assert!(error.to_string().contains("destination directory"));
    assert!(!root.path().join("missing").exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn unusable_parent_fails_without_completed_or_partial_file() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("file");
    fs::write(&parent, "keep").unwrap();
    let mut worker = ExportWorker::new();
    worker
        .start(request(root.path(), &parent.join("log.txt")))
        .unwrap();
    assert!(finish(&mut worker).is_err());
    assert_eq!(fs::read_to_string(parent).unwrap(), "keep");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn unwritable_parent_fails_and_cleans_owned_partial() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("readonly");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let mut worker = ExportWorker::new();
    worker
        .start(request(root.path(), &parent.join("log.txt")))
        .unwrap();
    let result = finish(&mut worker);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn metadata_and_suggested_filename_are_bounded_and_safe_without_io() {
    let root = tempfile::tempdir().unwrap();
    let metadata = ExportMetadata::new(
        42,
        Some("../../a\n\x1b[31m/Phone\\name"),
        Some(&"界".repeat(5000)),
        None,
    );
    let first = default_destination(root.path(), &metadata).unwrap();
    let second = default_destination(root.path(), &metadata).unwrap();
    assert_ne!(first, second);
    assert_eq!(first.parent().unwrap(), root.path().join("flimru-logs"));
    let name = first.file_name().unwrap().to_str().unwrap();
    assert!(name.len() < 200);
    assert!(
        name.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))
    );
    assert!(!name.contains(".."));
    assert!(!root.path().join("flimru-logs").exists());
    let mut worker = ExportWorker::new();
    worker
        .start(ExportRequest::new(root.path(), &first, snapshot("log"), metadata).unwrap())
        .unwrap();
    let saved = finish(&mut worker).unwrap();
    let text = fs::read_to_string(saved.path).unwrap();
    let header = text.split_once("\n\n").unwrap().0;
    assert!(!header.contains('\x1b'));
    assert!(!header.contains("../"));
    assert!(!header.contains('\\'));
    assert!(header.len() < 1024);
}

#[test]
fn invalid_destinations_reject_before_any_write() {
    let root = tempfile::tempdir().unwrap();
    for path in ["", "bad\nname.txt", "bad\0name.txt"] {
        assert!(
            ExportRequest::new(root.path(), Path::new(path), snapshot("text"), metadata()).is_err()
        );
    }
    assert!(default_destination(Path::new("relative"), &metadata()).is_err());
    assert!(
        ExportRequest::new(
            root.path(),
            Path::new(&"a".repeat(4097)),
            snapshot("text"),
            metadata()
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn near_limit_snapshot_rejects_header_overflow_without_creating_destination() {
    let root = tempfile::tempdir().unwrap();
    let cell = "a\u{301}\u{302}\u{303}\u{304}\u{305}\u{306}\u{307}\u{308}\u{309}";
    let input = cell.repeat(883_011);
    let mut parser = vt100::Parser::new(884, 1000, 0);
    parser.process(input.as_bytes());
    drop(input);
    let screen = parser.screen().clone();
    drop(parser);
    let snap = Arc::new(ConsoleSnapshot::capture(screen, &AtomicBool::new(false)).unwrap());
    assert_eq!(snap.plain_text().len(), log_export::MAX_EXPORT_BYTES - 7);
    let destination = default_destination(root.path(), &metadata()).unwrap();
    assert!(matches!(
        ExportRequest::new(root.path(), &destination, snap, metadata()),
        Err(ExportError::SizeLimit)
    ));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn only_one_result_is_pending_and_cancel_or_shutdown_never_leaves_partial() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("cancel.txt");
    let mut worker = ExportWorker::new();
    worker.start(request(root.path(), &destination)).unwrap();
    assert!(worker.is_running());
    assert_eq!(
        worker.start(request(root.path(), Path::new("second.txt"))),
        Err(ExportError::Busy)
    );
    worker.cancel();
    match finish(&mut worker) {
        Ok(saved) => assert_eq!(
            fs::metadata(saved.path).unwrap().len() as usize,
            saved.bytes
        ),
        Err(error) => {
            assert_eq!(error, ExportError::Cancelled);
            assert!(!destination.exists());
        }
    }
    assert!(!root.path().join("second.txt").exists());
    let destination = root.path().join("drop.txt");
    worker.start(request(root.path(), &destination)).unwrap();
    drop(worker);
    for entry in fs::read_dir(root.path()).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".partial")
        );
    }
    if destination.exists() {
        assert!(
            fs::read_to_string(destination)
                .unwrap()
                .ends_with("\n\nhello")
        );
    }
}
