//! Owned, bounded background polling for the selected launch configuration file.
//!
//! Contents (not metadata) identify versions. Automatic changes need two matching
//! observations, 500 ms apart; manual reloads bypass that debounce. The mailbox
//! holds at most one result and manual requests immediately invalidate older work.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::LaunchFile;

pub const MAX_LAUNCH_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const RELOAD_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadSource {
    Automatic,
    Manual,
}

/// Error text deliberately contains no parser excerpts, source JSON, expanded
/// environment values, or OS error strings that might include file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadError {
    Missing,
    PermissionDenied,
    NotRegularFile,
    TooLarge,
    InvalidUtf8,
    ReadFailed,
    InvalidConfiguration,
}

impl fmt::Display for ReloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Launch configuration file was not found.",
            Self::PermissionDenied => "Permission denied reading the launch configuration file.",
            Self::NotRegularFile => "Launch configuration path must be a regular file.",
            Self::TooLarge => "Launch configuration file exceeds the 4 MiB input limit.",
            Self::InvalidUtf8 => "Launch configuration file must contain UTF-8 text.",
            Self::ReadFailed => "Could not read the launch configuration file.",
            Self::InvalidConfiguration => {
                "Launch configuration file contains invalid JSONC or an invalid catalog structure."
            }
        })
    }
}

impl std::error::Error for ReloadError {}

#[derive(Debug)]
pub struct ReloadResult {
    pub generation: u64,
    pub source: ReloadSource,
    pub result: Result<LaunchFile, ReloadError>,
}

/// Startup and `--check` use the same bounded reader and safe parser errors as
/// background reloads. Call before entering the interactive UI.
pub fn load_bounded(path: &Path, workspace: &Path) -> Result<LaunchFile, ReloadError> {
    let observation = read_observation(path, || false).ok_or(ReloadError::ReadFailed)?;
    parse_observation(observation, path, workspace)
}

/// Owns one worker. Dropping this handle wakes and joins it; regular-file reads
/// are limited to 4 MiB + one detection byte and check cancellation in chunks.
/// As with other synchronous filesystem APIs, a stalled kernel/filesystem read
/// itself cannot be interrupted portably.
pub struct ConfigReloader {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl ConfigReloader {
    /// The initial file version is observed and published, even if startup has
    /// already loaded a catalog. This closes the startup-load/worker-start race.
    pub fn new(path: PathBuf, workspace: PathBuf) -> io::Result<Self> {
        Self::with_interval(path, workspace, RELOAD_POLL_INTERVAL)
    }

    fn with_interval(path: PathBuf, workspace: PathBuf, interval: Duration) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("flimru-config-reload".into())
            .spawn(move || run_worker(worker_shared, path, workspace, interval))?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    /// Coalesces requests into one immediate read. The returned generation can
    /// be used by the UI to identify completion of its most recent request.
    /// Explicit manual requests return a result even for unchanged contents;
    /// automatic observations of unchanged contents/errors publish nothing.
    pub fn request_reload(&self) -> u64 {
        let mut control = self.shared.lock();
        control.generation = control.generation.wrapping_add(1);
        control.manual_pending = true;
        control.mailbox = None;
        let generation = control.generation;
        self.shared.wake.notify_one();
        generation
    }

    /// Takes the newest completed result, discarding anything invalidated by a
    /// newer request. Consumers should apply it directly on their UI thread.
    pub fn poll(&mut self) -> Option<ReloadResult> {
        let mut control = self.shared.lock();
        let result = control.mailbox.take()?;
        (result.generation == control.generation).then_some(result)
    }
}

impl Drop for ConfigReloader {
    fn drop(&mut self) {
        {
            let mut control = self.shared.lock();
            control.shutdown = true;
            control.mailbox = None;
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            // No worker code panics intentionally; a panic must still be joined.
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Control> {
        self.control
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn cancelled(&self) -> bool {
        self.lock().shutdown
    }

    fn publish(&self, result: ReloadResult) {
        let mut control = self.lock();
        if !control.shutdown && control.generation == result.generation {
            control.mailbox = Some(result);
        }
    }
}

#[derive(Default)]
struct Control {
    shutdown: bool,
    generation: u64,
    manual_pending: bool,
    mailbox: Option<ReloadResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Observation {
    Content(Vec<u8>),
    Error(ReloadError),
}

/// Pure observation state, independent of filesystem access and wall clocks.
#[derive(Default)]
struct Debounce {
    accepted: Option<Observation>,
    candidate: Option<Observation>,
}

impl Debounce {
    fn automatic(&mut self, observation: Observation) -> Option<Observation> {
        if self.accepted.as_ref() == Some(&observation) {
            self.candidate = None;
            None
        } else if self.candidate.as_ref() == Some(&observation) {
            Some(self.accept(observation))
        } else {
            self.candidate = Some(observation);
            None
        }
    }

    fn accept(&mut self, observation: Observation) -> Observation {
        self.candidate = None;
        self.accepted = Some(observation.clone());
        observation
    }
}

fn run_worker(shared: Arc<Shared>, path: PathBuf, workspace: PathBuf, interval: Duration) {
    let mut debounce = Debounce::default();
    let mut next_poll = Instant::now();
    loop {
        let (source, read_generation) = {
            let mut control = shared.lock();
            while !control.shutdown && !control.manual_pending && Instant::now() < next_poll {
                let remaining = next_poll.saturating_duration_since(Instant::now());
                (control, _) = shared
                    .wake
                    .wait_timeout(control, remaining)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if control.shutdown {
                return;
            }
            let source = if control.manual_pending {
                control.manual_pending = false;
                ReloadSource::Manual
            } else {
                ReloadSource::Automatic
            };
            (source, control.generation)
        };
        next_poll = Instant::now() + interval;
        let Some(observation) = read_observation(&path, || shared.cancelled()) else {
            return;
        };
        let observation = match source {
            ReloadSource::Automatic => debounce.automatic(observation),
            ReloadSource::Manual => Some(debounce.accept(observation)),
        };
        let Some(observation) = observation else {
            continue;
        };
        let generation = {
            let mut control = shared.lock();
            if control.shutdown {
                return;
            }
            // A manual request during an automatic read or earlier manual job
            // wins; do not assign the stale observation a newer generation.
            if control.generation != read_generation || control.manual_pending {
                continue;
            }
            if source == ReloadSource::Automatic {
                control.generation = control.generation.wrapping_add(1);
            }
            control.generation
        };
        let result = parse_observation(observation, &path, &workspace);
        shared.publish(ReloadResult {
            generation,
            source,
            result,
        });
    }
}

fn read_observation(path: &Path, cancelled: impl Fn() -> bool) -> Option<Observation> {
    if cancelled() {
        return None;
    }
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Avoid blocking on a FIFO before the regular-file check below.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) => return Some(Observation::Error(read_error(&error))),
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => return Some(Observation::Error(read_error(&error))),
    };
    if !metadata.is_file() {
        return Some(Observation::Error(ReloadError::NotRegularFile));
    }
    if metadata.len() > MAX_LAUNCH_FILE_BYTES as u64 {
        return Some(Observation::Error(ReloadError::TooLarge));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return None;
        }
        let remaining = MAX_LAUNCH_FILE_BYTES + 1 - contents.len();
        let read_limit = remaining.min(buffer.len());
        match file.read(&mut buffer[..read_limit]) {
            Ok(0) => return Some(Observation::Content(contents)),
            Ok(count) => {
                contents.extend_from_slice(&buffer[..count]);
                if contents.len() > MAX_LAUNCH_FILE_BYTES {
                    return Some(Observation::Error(ReloadError::TooLarge));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Some(Observation::Error(read_error(&error))),
        }
    }
}

fn read_error(error: &io::Error) -> ReloadError {
    match error.kind() {
        io::ErrorKind::NotFound => ReloadError::Missing,
        io::ErrorKind::PermissionDenied => ReloadError::PermissionDenied,
        _ => ReloadError::ReadFailed,
    }
}

fn parse_observation(
    observation: Observation,
    path: &Path,
    workspace: &Path,
) -> Result<LaunchFile, ReloadError> {
    match observation {
        Observation::Error(error) => Err(error),
        Observation::Content(contents) => {
            let raw = std::str::from_utf8(&contents).map_err(|_| ReloadError::InvalidUtf8)?;
            LaunchFile::parse(raw, path, workspace).map_err(|_| ReloadError::InvalidConfiguration)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::process::{Child, Command, ExitStatus, Stdio};

    fn content(text: &str) -> Observation {
        Observation::Content(text.as_bytes().to_vec())
    }

    #[test]
    fn automatic_observations_require_two_identical_changes() {
        let mut debounce = Debounce::default();
        assert_eq!(debounce.automatic(content("first")), None);
        assert_eq!(debounce.automatic(content("intermediate")), None);
        assert_eq!(debounce.automatic(content("final")), None);
        assert_eq!(debounce.automatic(content("final")), Some(content("final")));
        assert_eq!(debounce.automatic(content("final")), None);
        assert_eq!(debounce.automatic(content("other")), None);
        assert_eq!(debounce.automatic(content("final")), None);
        assert_eq!(debounce.automatic(content("other")), None);
        assert_eq!(debounce.automatic(content("other")), Some(content("other")));
    }

    #[test]
    fn manual_bypasses_debounce_and_resets_pending_automatic_change() {
        let mut debounce = Debounce::default();
        assert_eq!(debounce.automatic(content("old")), None);
        assert_eq!(debounce.accept(content("manual")), content("manual"));
        assert_eq!(debounce.automatic(content("manual")), None);
        assert_eq!(debounce.automatic(content("old")), None);
        assert_eq!(debounce.automatic(content("old")), Some(content("old")));
    }

    #[test]
    fn identical_errors_are_suppressed_and_delete_recreate_is_a_change() {
        let mut debounce = Debounce::default();
        debounce.accept(content("valid"));
        let missing = Observation::Error(ReloadError::Missing);
        assert_eq!(debounce.automatic(missing.clone()), None);
        assert_eq!(debounce.automatic(missing.clone()), Some(missing.clone()));
        assert_eq!(debounce.automatic(missing), None);
        assert_eq!(debounce.automatic(content("valid")), None);
        assert_eq!(debounce.automatic(content("valid")), Some(content("valid")));
    }

    #[test]
    fn mailbox_is_coalesced_and_stale_generations_cannot_publish() {
        let shared = Shared::default();
        shared.lock().generation = 3;
        for generation in [3, 2, 1] {
            shared.publish(ReloadResult {
                generation,
                source: ReloadSource::Automatic,
                result: Err(ReloadError::Missing),
            });
        }
        assert_eq!(
            shared.lock().mailbox.as_ref().map(|r| r.generation),
            Some(3)
        );
        shared.publish(ReloadResult {
            generation: 3,
            source: ReloadSource::Manual,
            result: Err(ReloadError::TooLarge),
        });
        assert!(matches!(
            shared.lock().mailbox.as_ref().map(|r| &r.result),
            Some(Err(ReloadError::TooLarge))
        ));
        shared.lock().shutdown = true;
        shared.publish(ReloadResult {
            generation: 3,
            source: ReloadSource::Manual,
            result: Err(ReloadError::ReadFailed),
        });
        assert!(matches!(
            shared.lock().mailbox.as_ref().map(|r| &r.result),
            Some(Err(ReloadError::TooLarge))
        ));
    }

    #[test]
    fn manual_request_invalidates_mailbox_and_poll_rejects_stale_results() {
        let shared = Arc::new(Shared::default());
        let mut reloader = ConfigReloader {
            shared,
            worker: None,
        };
        reloader.shared.publish(ReloadResult {
            generation: 0,
            source: ReloadSource::Automatic,
            result: Err(ReloadError::Missing),
        });
        let generation = reloader.request_reload();
        assert_eq!(generation, 1);
        assert!(reloader.poll().is_none());
        assert_eq!(reloader.request_reload(), 2);
        assert!(reloader.shared.lock().manual_pending);
        reloader.shared.lock().mailbox = Some(ReloadResult {
            generation: 0,
            source: ReloadSource::Automatic,
            result: Err(ReloadError::Missing),
        });
        assert!(reloader.poll().is_none());
    }

    #[test]
    fn bounded_reader_detects_limits_utf8_and_nonregular_paths() -> Result<(), Box<dyn Error>> {
        let fixture = tempfile::tempdir()?;
        let path = fixture.path().join("launch.json");
        std::fs::write(&path, vec![b' '; MAX_LAUNCH_FILE_BYTES])?;
        let observation = read_observation(&path, || false);
        assert!(
            matches!(observation, Some(Observation::Content(bytes)) if bytes.len() == MAX_LAUNCH_FILE_BYTES)
        );
        std::fs::write(&path, vec![b' '; MAX_LAUNCH_FILE_BYTES + 1])?;
        assert_eq!(
            read_observation(&path, || false),
            Some(Observation::Error(ReloadError::TooLarge))
        );
        std::fs::write(&path, [0xff])?;
        let invalid = parse_observation(Observation::Content(vec![0xff]), &path, fixture.path());
        assert!(matches!(invalid, Err(ReloadError::InvalidUtf8)));
        assert_eq!(
            read_observation(fixture.path(), || false),
            Some(Observation::Error(ReloadError::NotRegularFile))
        );
        assert_eq!(read_observation(&path, || true), None);
        Ok(())
    }

    #[test]
    fn parser_errors_do_not_expose_source_secrets() {
        let secret = "SECRET-TOKEN-DO-NOT-PRINT";
        let result = parse_observation(content(secret), Path::new("launch.json"), Path::new("."));
        assert!(matches!(result, Err(ReloadError::InvalidConfiguration)));
        if let Err(error) = result {
            assert!(!error.to_string().contains(secret));
        }
    }

    fn wait_result(reloader: &mut ConfigReloader) -> io::Result<ReloadResult> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = reloader.poll() {
                return Ok(result);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "reload result not received",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    const CHILD_TEST: &str = "config_reload::tests::worker_subprocess_child";
    const CHILD_CASE: &str = "FLIMRU_CONFIG_RELOAD_TEST_CASE";
    const CHILD_FIXTURE: &str = "FLIMRU_CONFIG_RELOAD_TEST_FIXTURE";

    struct OwnedChild {
        child: Child,
        reaped: bool,
    }

    impl OwnedChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            let status = self.child.try_wait()?;
            self.reaped = status.is_some();
            Ok(status)
        }

        fn kill_and_wait(&mut self) -> io::Result<ExitStatus> {
            // Always reap, including the race where the process exited just
            // before kill. A wait error leaves the guard responsible for cleanup.
            let killed = self.child.kill();
            let status = self.child.wait()?;
            self.reaped = true;
            killed?;
            Ok(status)
        }
    }

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !self.reaped {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    struct WatchdogResult {
        status: ExitStatus,
        timed_out: bool,
        drop_started: bool,
        output: String,
    }

    fn run_worker_case(case: &str) -> Result<WatchdogResult, Box<dyn Error>> {
        // The parent owns the fixture and output even if the child must be
        // killed while a worker is stuck. Reap before removing any of them.
        let fixture = tempfile::tempdir()?;
        let output_path = fixture.path().join("child-output.log");
        let output = File::create(&output_path)?;
        let mut child = OwnedChild {
            child: Command::new(std::env::current_exe()?)
                .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
                .env(CHILD_CASE, case)
                .env(CHILD_FIXTURE, fixture.path())
                .stdout(Stdio::from(output.try_clone()?))
                .stderr(Stdio::from(output))
                .spawn()?,
            reaped: false,
        };
        let marker = fixture.path().join("dropping");
        // Preserve the two-second drop bound independently of the existing
        // three-second result waits, plus one second to start the child.
        let result_waits = match case {
            "updates" => 6,
            "fifo" => 2,
            "deadline" => 1,
            _ => 0,
        };
        let setup_deadline = Instant::now() + Duration::from_secs(1 + 3 * result_waits);
        let mut drop_deadline = None;
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status, false);
            }
            if drop_deadline.is_none() && marker.try_exists()? {
                drop_deadline = Some(Instant::now() + Duration::from_secs(2));
            }
            if Instant::now() >= drop_deadline.unwrap_or(setup_deadline) {
                break (child.kill_and_wait()?, true);
            }
            thread::sleep(Duration::from_millis(1));
        };
        let result = WatchdogResult {
            status,
            timed_out,
            drop_started: marker.try_exists()?,
            output: std::fs::read_to_string(output_path)?,
        };
        // Explicit cleanup makes fixture teardown failure visible to the test.
        drop(child);
        fixture.close()?;
        Ok(result)
    }

    fn assert_worker_case_finishes(case: &str) -> Result<(), Box<dyn Error>> {
        let result = run_worker_case(case)?;
        assert!(
            !result.timed_out,
            "worker case {case} timed out: {}",
            result.output
        );
        assert!(
            result.status.success(),
            "worker case {case} failed: {}",
            result.output
        );
        assert!(result.drop_started, "worker case {case} did not reach drop");
        Ok(())
    }

    fn assert_drop_finishes(reloader: ConfigReloader, fixture: &Path) -> io::Result<()> {
        let shared = Arc::clone(&reloader.shared);
        std::fs::write(fixture.join("dropping"), [])?;
        drop(reloader);
        assert!(shared.lock().shutdown);
        assert!(shared.lock().mailbox.is_none());
        // Both the handle and its joined worker must have released ownership.
        assert_eq!(Arc::strong_count(&shared), 1);
        Ok(())
    }

    #[test]
    fn worker_handles_initial_same_size_atomic_delete_recreate_and_manual_updates()
    -> Result<(), Box<dyn Error>> {
        assert_worker_case_finishes("updates")
    }

    fn worker_update_case(fixture: &Path) -> Result<(), Box<dyn Error>> {
        let path = fixture.join("launch.json");
        let catalog = |name| format!(r#"{{"configurations":[{{"name":"{name}"}}]}}"#);
        std::fs::write(&path, catalog("one"))?;
        let mut reloader = ConfigReloader::with_interval(
            path.clone(),
            fixture.to_owned(),
            Duration::from_millis(5),
        )?;
        let first = wait_result(&mut reloader)?;
        assert_eq!(first.source, ReloadSource::Automatic);
        assert_eq!(first.result?.configurations[0].name, "one");
        let modified = std::fs::metadata(&path)?.modified()?;
        std::fs::write(&path, catalog("two"))?;
        File::options()
            .write(true)
            .open(&path)?
            .set_times(std::fs::FileTimes::new().set_modified(modified))?;
        let second = wait_result(&mut reloader)?;
        assert!(second.generation > first.generation);
        assert_eq!(second.result?.configurations[0].name, "two");
        let replacement = fixture.join("atomic.json");
        std::fs::write(&replacement, catalog("new"))?;
        std::fs::rename(replacement, &path)?;
        assert_eq!(
            wait_result(&mut reloader)?.result?.configurations[0].name,
            "new"
        );
        std::fs::remove_file(&path)?;
        assert!(matches!(
            wait_result(&mut reloader)?.result,
            Err(ReloadError::Missing)
        ));
        std::fs::write(&path, "{broken")?;
        assert!(matches!(
            wait_result(&mut reloader)?.result,
            Err(ReloadError::InvalidConfiguration)
        ));
        std::fs::write(&path, r#"{"configurations":[]}"#)?;
        let manual_generation = reloader.request_reload();
        let manual = wait_result(&mut reloader)?;
        assert_eq!(manual.generation, manual_generation);
        assert_eq!(manual.source, ReloadSource::Manual);
        assert!(manual.result?.configurations.is_empty());
        assert_drop_finishes(reloader, fixture)?;
        Ok(())
    }

    #[test]
    fn drop_wakes_worker_waiting_for_poll_deadline() -> Result<(), Box<dyn Error>> {
        assert_worker_case_finishes("deadline")
    }

    fn deadline_drop_case(fixture: &Path) -> Result<(), Box<dyn Error>> {
        let path = fixture.join("launch.json");
        std::fs::write(&path, r#"{"configurations":[]}"#)?;
        let mut reloader =
            ConfigReloader::with_interval(path, fixture.to_owned(), Duration::from_secs(60))?;
        reloader.request_reload();
        wait_result(&mut reloader)?;
        assert_drop_finishes(reloader, fixture)?;
        Ok(())
    }

    #[test]
    fn public_constructor_uses_default_poll_interval() -> Result<(), Box<dyn Error>> {
        assert_eq!(RELOAD_POLL_INTERVAL, Duration::from_millis(500));
        assert_worker_case_finishes("constructor")
    }

    fn constructor_case(fixture: &Path) -> Result<(), Box<dyn Error>> {
        let reloader = ConfigReloader::new(fixture.join("missing.json"), fixture.to_owned())?;
        assert_drop_finishes(reloader, fixture)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fifo_replacement_is_rejected_without_blocking_worker_or_drop() -> Result<(), Box<dyn Error>>
    {
        assert_worker_case_finishes("fifo")
    }

    #[cfg(unix)]
    fn fifo_case(fixture: &Path) -> Result<(), Box<dyn Error>> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = fixture.join("launch.json");
        std::fs::write(&path, r#"{"configurations":[]}"#)?;
        let mut reloader = ConfigReloader::with_interval(
            path.clone(),
            fixture.to_owned(),
            Duration::from_secs(60),
        )?;
        reloader.request_reload();
        assert!(wait_result(&mut reloader)?.result.is_ok());
        std::fs::remove_file(&path)?;
        let c_path = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: c_path owns a valid NUL-terminated path for this call. The
        // task-owned fixture directory remains alive until the worker is joined.
        if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        reloader.request_reload();
        assert!(matches!(
            wait_result(&mut reloader)?.result,
            Err(ReloadError::NotRegularFile)
        ));
        assert_drop_finishes(reloader, fixture)?;
        Ok(())
    }

    #[test]
    fn watchdog_kills_and_reaps_a_stuck_drop() -> Result<(), Box<dyn Error>> {
        let result = run_worker_case("stuck_drop")?;
        assert!(
            result.drop_started,
            "child did not reach the deliberately stuck drop: {}",
            result.output
        );
        assert!(result.timed_out);
        assert!(!result.status.success());
        Ok(())
    }

    #[test]
    #[ignore = "invoked only by the parent-owned subprocess watchdog"]
    fn worker_subprocess_child() -> Result<(), Box<dyn Error>> {
        let case = std::env::var(CHILD_CASE)?;
        let fixture =
            PathBuf::from(std::env::var_os(CHILD_FIXTURE).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "missing child fixture")
            })?);
        match case.as_str() {
            "updates" => worker_update_case(&fixture),
            "deadline" => deadline_drop_case(&fixture),
            "constructor" => constructor_case(&fixture),
            #[cfg(unix)]
            "fifo" => fifo_case(&fixture),
            "stuck_drop" => {
                let reloader = ConfigReloader {
                    shared: Arc::new(Shared::default()),
                    // Only the owned child contains this deliberately stuck
                    // thread; timeout kills and reaps the entire process.
                    worker: Some(thread::spawn(|| {
                        loop {
                            thread::park();
                        }
                    })),
                };
                assert_drop_finishes(reloader, &fixture)?;
                Err(io::Error::other("deliberately stuck drop unexpectedly returned").into())
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown child case").into()),
        }
    }
}
