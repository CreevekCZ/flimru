//! Device discovery runs independently of the terminal and of active run sessions.

use std::collections::HashMap;
use std::io::{self, Read};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::state::SavedDevice;
use crate::tool::FlutterTool;

const SCAN_TIMEOUT: Duration = Duration::from_secs(25);
const POLL_INTERVAL: Duration = Duration::from_millis(30);
const OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// Cached favorites and configuration targets remain selectable when false.
    pub available: bool,
    pub favorite: bool,
}

impl Device {
    pub fn saved(&self) -> SavedDevice {
        SavedDevice {
            id: self.id.clone(),
            name: self.name.clone(),
            platform: self.platform.clone(),
        }
    }
}

/// Combine a discovery snapshot with saved favorites and launch device targets.
///
/// IDs are exact, case-sensitive keys. Discovered metadata takes precedence over
/// cached metadata; absence from discovery never removes a saved favorite.
/// Sorting is deterministic so callers can preserve selection by device ID.
pub fn merge_devices(
    discovered: &[Device],
    favorites: &[SavedDevice],
    pinned_ids: &[String],
) -> Vec<Device> {
    let mut by_id = HashMap::<String, Device>::new();
    for device in discovered {
        if !device.id.is_empty() {
            by_id.entry(device.id.clone()).or_insert_with(|| {
                let mut device = device.clone();
                device.favorite = false;
                device
            });
        }
    }
    for saved in favorites {
        if saved.id.is_empty() {
            continue;
        }
        by_id
            .entry(saved.id.clone())
            .or_insert_with(|| Device {
                id: saved.id.clone(),
                name: saved.name.clone(),
                platform: saved.platform.clone(),
                available: false,
                favorite: true,
            })
            .favorite = true;
    }
    for id in pinned_ids {
        if id.is_empty() {
            continue;
        }
        by_id.entry(id.clone()).or_insert_with(|| {
            let (name, platform) = match id.as_str() {
                "web-server" => ("Web server", "web"),
                "chrome" => ("Chrome", "web-javascript"),
                "edge" => ("Edge", "web-javascript"),
                "macos" => ("macOS", "darwin"),
                "linux" => ("Linux", "linux"),
                "windows" => ("Windows", "windows"),
                _ => (id.as_str(), "custom"),
            };
            Device {
                id: id.clone(),
                name: name.to_owned(),
                platform: platform.to_owned(),
                available: false,
                favorite: false,
            }
        });
    }
    let mut devices: Vec<_> = by_id.into_values().collect();
    devices.sort_by(|left, right| {
        right
            .favorite
            .cmp(&left.favorite)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    devices
}

#[derive(Debug)]
pub enum DiscoveryEvent {
    Started,
    Finished(Result<Vec<Device>, String>),
}

/// One worker owns every discovery child. Refresh requests coalesce while a scan
/// is running, and dropping this owner cancels the scan and joins the worker.
pub struct Discovery {
    pub events: Receiver<DiscoveryEvent>,
    refresh: SyncSender<()>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Discovery {
    pub fn start(tool: FlutterTool, interval: Duration) -> Self {
        // One complete scan fits in this queue. If the UI stops consuming,
        // backpressure stops new scans instead of retaining old snapshots.
        let (events_tx, events) = mpsc::sync_channel(2);
        let (refresh, refresh_rx) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::spawn(move || {
            let interval = interval.max(Duration::from_millis(250));
            let mut next_scan = Instant::now();
            loop {
                if worker_cancelled.load(Ordering::Acquire) {
                    break;
                }
                let now = Instant::now();
                if now < next_scan {
                    match refresh_rx.recv_timeout(next_scan - now) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                if !send_event(&events_tx, DiscoveryEvent::Started, &worker_cancelled) {
                    break;
                }
                let result = scan(&tool, &worker_cancelled);
                if !send_event(
                    &events_tx,
                    DiscoveryEvent::Finished(result),
                    &worker_cancelled,
                ) {
                    break;
                }
                // A single queued request triggers one more scan immediately.
                // Requests during that scan are coalesced by the bounded channel.
                next_scan = if refresh_rx.try_recv().is_ok() {
                    Instant::now()
                } else {
                    Instant::now() + interval
                };
            }
        });
        Self {
            events,
            refresh,
            cancelled,
            worker: Some(worker),
        }
    }

    pub fn refresh(&self) {
        let _ = self.refresh.try_send(());
    }
}

/// Preserve scan event order without blocking cancellation on a full queue.
fn send_event(
    events: &SyncSender<DiscoveryEvent>,
    mut event: DiscoveryEvent,
    cancelled: &AtomicBool,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return false;
        }
        match events.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => {
                event = pending;
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        // Wake an idle worker even when the configured period is very long.
        let _ = self.refresh.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MachineDevice {
    id: String,
    name: String,
    #[serde(default)]
    target_platform: String,
    is_supported: Option<bool>,
}

/// Read Flutter's machine output, tolerating diagnostic banners before its
/// array. Once a potential JSON array starts, malformed data is reported rather
/// than skipping ahead to a nested or later array.
pub fn parse_devices(output: &[u8]) -> Result<Vec<Device>, String> {
    let text = std::str::from_utf8(output)
        .map_err(|error| format!("Flutter device output is not UTF-8: {error}"))?;
    let text = text.trim_start_matches('\u{feff}');
    let mut offset = 0;
    let mut payload = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        // Flutter doctor-style diagnostics can have bracketed status icons.
        let diagnostic = ["[!]", "[✓]", "[✗]", "[X]", "[x]"]
            .iter()
            .any(|marker| trimmed.starts_with(marker));
        if trimmed.starts_with('[') && !diagnostic {
            payload = Some(&text[offset + line.len() - trimmed.len()..]);
            break;
        }
        if trimmed.starts_with('{') {
            return Err("Flutter returned a JSON object; expected a device array from `flutter devices --machine`.".to_owned());
        }
        offset += line.len();
    }
    let payload = payload.ok_or_else(|| {
        "Flutter returned no device JSON array. Check that the configured Flutter executable supports `devices --machine`.".to_owned()
    })?;
    let machine: Vec<MachineDevice> = serde_json::from_str(payload)
        .map_err(|error| format!("Invalid Flutter device JSON: {error}"))?;
    let mut devices = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for device in machine {
        if device.is_supported == Some(false) {
            continue;
        }
        if device.id.trim().is_empty() {
            return Err("Flutter device JSON contains an empty device ID.".to_owned());
        }
        if ids.insert(device.id.clone()) {
            devices.push(Device {
                id: device.id,
                name: device.name,
                platform: device.target_platform,
                available: true,
                favorite: false,
            });
        }
    }
    Ok(devices)
}

/// Kill and reap a discovery child on every early return, including I/O errors.
struct OwnedChild {
    child: Child,
    finished: bool,
}

impl OwnedChild {
    #[cfg(unix)]
    fn stop_descendants(&self) {
        // Discovery starts in a private group, so no active run session is
        // affected. Clean up wrappers that exited without waiting for helpers.
        // SAFETY: kill takes an integer, with no borrowed memory. This child
        // was spawned with process_group(0), so the negative id targets its group.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
    }

    fn abort(&mut self) {
        #[cfg(unix)]
        self.stop_descendants();
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finished = true;
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.finished {
            self.abort();
        }
    }
}

fn scan(tool: &FlutterTool, cancelled: &AtomicBool) -> Result<Vec<Device>, String> {
    let mut command = tool.command();
    command
        .args(["devices", "--machine", "--device-timeout", "5"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|error| format!("Could not start Flutter device discovery: {error}"))?;
    let mut owned = OwnedChild {
        child,
        finished: false,
    };
    let mut stdout = owned
        .child
        .stdout
        .take()
        .ok_or_else(|| "Flutter discovery stdout was not piped.".to_owned())?;
    let mut stderr = owned
        .child
        .stderr
        .take()
        .ok_or_else(|| "Flutter discovery stderr was not piped.".to_owned())?;
    prepare_pipe(&stdout).map_err(|error| format!("Could not read Flutter output: {error}"))?;
    prepare_pipe(&stderr).map_err(|error| format!("Could not read Flutter errors: {error}"))?;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let deadline = Instant::now() + SCAN_TIMEOUT;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Device discovery cancelled.".to_owned());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Flutter device discovery timed out after {} seconds.{}",
                SCAN_TIMEOUT.as_secs(),
                diagnostic_suffix(&errors)
            ));
        }
        drain_pipe(&mut stdout, &mut output, errors.len())?;
        drain_pipe(&mut stderr, &mut errors, output.len())?;
        match owned.child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                owned.stop_descendants();
                owned.finished = true;
                // Collect the remaining buffered output after process exit.
                drain_pipe(&mut stdout, &mut output, errors.len())?;
                drain_pipe(&mut stderr, &mut errors, output.len())?;
                return finish_scan(status, &output, &errors);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => return Err(format!("Could not wait for Flutter discovery: {error}")),
        }
    }
}

fn finish_scan(status: ExitStatus, output: &[u8], errors: &[u8]) -> Result<Vec<Device>, String> {
    if !status.success() {
        return Err(format!(
            "Flutter device discovery exited with {status}.{}",
            diagnostic_suffix(if errors.is_empty() { output } else { errors })
        ));
    }
    parse_devices(output).map_err(|error| format!("{error}{}", diagnostic_suffix(errors)))
}

fn diagnostic_suffix(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        let excerpt: String = text.chars().take(2000).collect();
        format!("\n{excerpt}")
    }
}

#[cfg(unix)]
fn prepare_pipe(pipe: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let descriptor = pipe.as_raw_fd();
    // SAFETY: pipe owns this descriptor throughout the call; F_GETFL takes no
    // variadic argument and neither changes ownership nor accesses Rust memory.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the same live descriptor is used; F_SETFL receives its required
    // integer flags argument. The borrowed pipe retains ownership.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn drain_pipe(
    pipe: &mut (impl Read + std::os::fd::AsRawFd),
    output: &mut Vec<u8>,
    other_bytes: usize,
) -> Result<(), String> {
    let mut buffer = [0_u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.len() + other_bytes + count > OUTPUT_LIMIT {
                    return Err("Flutter device output exceeded the 1 MiB limit.".to_owned());
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Could not read Flutter device output: {error}")),
        }
    }
}

#[cfg(windows)]
fn prepare_pipe(_: &impl std::os::windows::io::AsRawHandle) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn drain_pipe(
    pipe: &mut (impl Read + std::os::windows::io::AsRawHandle),
    output: &mut Vec<u8>,
    other_bytes: usize,
) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            pipe: *mut c_void,
            buffer: *mut c_void,
            buffer_size: u32,
            bytes_read: *mut u32,
            bytes_available: *mut u32,
            bytes_left: *mut u32,
        ) -> i32;
    }

    let mut buffer = [0_u8; 8192];
    loop {
        let mut available = 0;
        // SAFETY: the pipe handle is live for this borrow; available points to
        // an initialized writable u32. Null optional outputs and a null buffer
        // with length zero are supported by PeekNamedPipe.
        let ok = unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(109 | 233)) {
                return Ok(());
            }
            return Err(format!("Could not inspect Flutter device output: {error}"));
        }
        if available == 0 {
            return Ok(());
        }
        let count = usize::min(buffer.len(), available as usize);
        let count = pipe
            .read(&mut buffer[..count])
            .map_err(|error| format!("Could not read Flutter device output: {error}"))?;
        if count == 0 {
            return Ok(());
        }
        if output.len() + other_bytes + count > OUTPUT_LIMIT {
            return Err("Flutter device output exceeded the 1 MiB limit.".to_owned());
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str) -> Device {
        Device {
            id: id.to_owned(),
            name: name.to_owned(),
            platform: "ios".to_owned(),
            available: true,
            favorite: false,
        }
    }

    #[test]
    fn parses_supported_devices_with_banners_and_deduplicates_exact_ids() {
        let devices = parse_devices(
            br#"Waiting for another flutter command to release the startup lock...
[
 {"id":"one","name":"Phone","targetPlatform":"ios","isSupported":true},
 {"id":"one","name":"Duplicate","targetPlatform":"ios"},
 {"id":"two","name":"Unsupported","targetPlatform":"unknown","isSupported":false},
 {"id":"ONE","name":"Other phone","targetPlatform":"android-arm64"}
]"#,
        )
        .unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "one");
        assert_eq!(devices[1].id, "ONE");
        assert!(devices.iter().all(|device| device.available));
    }

    #[test]
    fn status_banner_and_empty_array_are_accepted() {
        assert!(
            parse_devices(b"[!] Some diagnostic\n[]\n")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn malformed_array_is_not_hidden_by_later_valid_json() {
        let error = parse_devices(b"Banner\n[{broken\n[]").unwrap_err();
        assert!(error.contains("Invalid Flutter device JSON"));
        assert!(
            parse_devices(b"not JSON")
                .unwrap_err()
                .contains("no device JSON")
        );
        assert!(parse_devices(br#"{"devices":[]}"#).is_err());
        assert!(parse_devices(br#"[{"id":"","name":"Phone"}]"#).is_err());
        assert!(parse_devices(b"[] trailing garbage").is_err());
    }

    #[test]
    fn cached_favorites_and_pinned_targets_remain_selectable() {
        let cached = device("cached", "Zebra").saved();
        let live = device("live", "Alpha");
        let devices = merge_devices(
            &[live.clone(), live.clone(), device("LIVE", "Alpha")],
            &[cached.clone(), live.saved(), cached],
            &[
                "web-server".to_owned(),
                "web-server".to_owned(),
                "live".to_owned(),
            ],
        );
        assert_eq!(devices.len(), 4);
        assert_eq!(devices[0].id, "live");
        assert_eq!(devices[1].id, "cached");
        assert!(devices[0].favorite && devices[0].available);
        assert!(devices[1].favorite && !devices[1].available);
        assert_eq!(devices[3].id, "web-server");
        assert!(!devices[3].available);
    }

    #[test]
    fn discovered_metadata_wins_and_order_does_not_depend_on_discovery_order() {
        let first = device("b", "Same");
        let second = device("a", "Same");
        let mut saved = first.saved();
        saved.name = "Old name".to_owned();
        let forward = merge_devices(&[first.clone(), second.clone()], &[saved.clone()], &[]);
        let reverse = merge_devices(&[second, first], &[saved], &[]);
        assert_eq!(forward, reverse);
        assert_eq!(forward[0].name, "Same");
    }

    #[cfg(unix)]
    fn fake_tool(script: &str) -> (tempfile::TempDir, FlutterTool) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-flutter");
        std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tool = FlutterTool {
            executable,
            prefix: vec![],
            workspace: directory.path().to_owned(),
        };
        (directory, tool)
    }

    #[cfg(unix)]
    #[test]
    fn dropping_discovery_cancels_child_promptly() {
        let (_directory, tool) = fake_tool("sleep 30\nprintf '[]'");
        let discovery = Discovery::start(tool, Duration::from_secs(3600));
        assert!(matches!(
            discovery
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            DiscoveryEvent::Started
        ));
        thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        drop(discovery);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn manual_refresh_requests_coalesce_and_idle_worker_stops_promptly() {
        let (_directory, tool) = fake_tool("sleep 0.2\nprintf '[]'");
        let discovery = Discovery::start(tool, Duration::from_secs(3600));
        assert!(matches!(
            discovery
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            DiscoveryEvent::Started
        ));
        for _ in 0..100 {
            discovery.refresh();
        }
        assert!(matches!(
            discovery
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            DiscoveryEvent::Finished(Ok(_))
        ));
        assert!(matches!(
            discovery
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            DiscoveryEvent::Started
        ));
        assert!(matches!(
            discovery
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            DiscoveryEvent::Finished(Ok(_))
        ));
        assert!(matches!(
            discovery.events.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let start = Instant::now();
        drop(discovery);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn undrained_events_bound_scans_and_do_not_block_cancellation() {
        let (directory, tool) = fake_tool("printf 'scan\\n' >> scans.log; printf '[]'");
        let discovery = Discovery::start(tool, Duration::from_millis(250));
        for _ in 0..100 {
            discovery.refresh();
        }
        let log = directory.path().join("scans.log");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !log.exists() {
            assert!(Instant::now() < deadline, "Discovery did not start");
            thread::sleep(Duration::from_millis(10));
        }
        // Started and Finished fill the two slots. Even queued refreshes must
        // wait before starting another child while the UI is not draining.
        thread::sleep(Duration::from_millis(350));
        assert_eq!(std::fs::read_to_string(log).unwrap(), "scan\n");
        let start = Instant::now();
        drop(discovery);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[cfg(unix)]
    #[test]
    fn large_stderr_is_drained_alongside_stdout_and_bounded() {
        let (_directory, tool) = fake_tool(
            "i=0; while [ $i -lt 18000 ]; do printf 'A diagnostic line that is long enough to fill the pipe buffer!\\n' >&2; i=$((i+1)); done; printf '[]'",
        );
        let result = scan(&tool, &AtomicBool::new(false));
        assert!(result.unwrap_err().contains("1 MiB"));
    }
}
