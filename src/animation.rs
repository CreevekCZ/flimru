//! Poll Flutter's reported VM Service, so the badge reflects confirmed state,
//! including changes made in DevTools. Restore the selected speed when a hot
//! restart replaces the app isolate, without claiming success before confirmation.
use anyhow::{Result, bail};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use url::Url;

const EXTENSION: &str = "ext.flutter.timeDilation";

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationState {
    Waiting,
    Checking,
    Confirmed(f64),
    Restoring(f64),
    Unavailable,
    Disconnected,
}

fn lock_state(state: &Mutex<AnimationState>) -> MutexGuard<'_, AnimationState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // This mutex contains a single enum, so resetting it restores all
            // invariants. Do not retain an untrusted confirmed value or panic
            // the UI if its worker failed while holding the lock.
            let mut guard = poisoned.into_inner();
            *guard = AnimationState::Disconnected;
            state.clear_poison();
            guard
        }
    }
}

enum Command {
    Connect(Url),
    Toggle,
    Restarting,
    Restarted,
    Reconnect,
    Shutdown,
}

#[derive(Default)]
struct Preference {
    factor: Option<f64>,
    isolate: Option<String>,
    restore: bool,
    restarting: bool,
}
#[derive(Clone, Copy)]
enum Escape {
    Text,
    Start,
    Csi,
    Osc,
    OscEnd,
}

pub struct AnimationControl {
    state: Arc<Mutex<AnimationState>>,
    connection_error: Arc<Mutex<Option<String>>>,
    commands: SyncSender<Command>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    enabled: bool,
    line: Vec<u8>,
    escape: Escape,
    endpoint: Option<Url>,
    pending_toggle: Arc<AtomicBool>,
}

impl AnimationControl {
    pub fn new(enabled: bool) -> Self {
        Self::start(enabled, None)
    }

    pub fn with_service_file(path: PathBuf) -> Self {
        Self::start(true, Some(path))
    }

    fn start(enabled: bool, service_file: Option<PathBuf>) -> Self {
        let state = Arc::new(Mutex::new(if enabled {
            AnimationState::Waiting
        } else {
            AnimationState::Unavailable
        }));
        let connection_error = Arc::new(Mutex::new(None));
        let (commands, rx) = mpsc::sync_channel(16);
        let cancelled = Arc::new(AtomicBool::new(false));
        let pending_toggle = Arc::new(AtomicBool::new(false));
        let worker = if enabled {
            let state = state.clone();
            let connection_error = connection_error.clone();
            let cancelled = cancelled.clone();
            let pending_toggle = pending_toggle.clone();
            Some(thread::spawn(move || {
                let agent = ureq::Agent::new_with_config(
                    ureq::Agent::config_builder()
                        .timeout_global(Some(Duration::from_millis(800)))
                        .max_redirects(0)
                        .proxy(None)
                        .build(),
                );
                let mut endpoint = None;
                let mut last_file_endpoint = None;
                let mut preference = Preference::default();
                let mut fast_until = None;
                while !cancelled.load(Ordering::Relaxed) {
                    let fast = fast_until.is_some_and(|until| Instant::now() < until);
                    if !fast {
                        preference.restarting = false;
                    }
                    let command = rx.recv_timeout(if fast {
                        Duration::from_millis(100)
                    } else if endpoint.is_some() {
                        Duration::from_secs(1)
                    } else if service_file.is_some() {
                        Duration::from_millis(250)
                    } else {
                        Duration::from_secs(3600)
                    });
                    let toggle = match command {
                        Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(Command::Connect(url)) => {
                            endpoint = Some(url);
                            preference.isolate = None;
                            preference.restore = preference.factor.is_some();
                            *lock_state(&state) = AnimationState::Checking;
                            false
                        }
                        Ok(Command::Toggle) => {
                            *lock_state(&state) = AnimationState::Checking;
                            true
                        }
                        Ok(Command::Restarting) => {
                            preference.restarting = true;
                            fast_until = Some(Instant::now() + Duration::from_secs(20));
                            false
                        }
                        Ok(Command::Restarted) => {
                            preference.restarting = false;
                            preference.restore = preference.factor.is_some();
                            fast_until = Some(Instant::now() + Duration::from_secs(2));
                            false
                        }
                        Ok(Command::Reconnect) => {
                            fast_until = Some(Instant::now() + Duration::from_secs(5));
                            *lock_state(&state) = if endpoint.is_some() {
                                AnimationState::Checking
                            } else {
                                AnimationState::Waiting
                            };
                            false
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                    };
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(url) = service_file.as_deref().and_then(read_service_file)
                        && last_file_endpoint.as_ref() != Some(&url)
                    {
                        last_file_endpoint = Some(url.clone());
                        if endpoint.as_ref() != Some(&url) {
                            endpoint = Some(url);
                            preference.isolate = None;
                            preference.restore = preference.factor.is_some();
                        }
                    }
                    if let Some(endpoint) = &endpoint {
                        let result =
                            read_factor(&agent, endpoint, toggle, &mut preference, &cancelled);
                        if let Ok(mut detail) = connection_error.lock() {
                            *detail = result.as_ref().err().map(ToString::to_string);
                        }
                        let status = match result {
                            Ok(Some(factor)) => AnimationState::Confirmed(factor),
                            Ok(None) if preference.restore || preference.restarting => preference
                                .factor
                                .map(AnimationState::Restoring)
                                .unwrap_or(AnimationState::Unavailable),
                            Ok(None) => AnimationState::Unavailable,
                            Err(_) => AnimationState::Disconnected,
                        };
                        if !cancelled.load(Ordering::Relaxed) {
                            *lock_state(&state) = status;
                        }
                    }
                    if toggle {
                        pending_toggle.store(false, Ordering::Release);
                    }
                }
            }))
        } else {
            None
        };
        Self {
            state,
            connection_error,
            commands,
            cancelled,
            worker,
            enabled,
            line: vec![],
            escape: Escape::Text,
            endpoint: None,
            pending_toggle,
        }
    }

    pub fn state(&self) -> AnimationState {
        if self.pending_toggle.load(Ordering::Acquire) {
            AnimationState::Checking
        } else {
            lock_state(&self.state).clone()
        }
    }

    #[cfg(unix)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Transport diagnostics never include the authenticated service URL.
    pub fn connection_error(&self) -> Option<String> {
        if self.state() != AnimationState::Disconnected {
            return None;
        }
        self.connection_error.lock().ok()?.clone()
    }

    pub fn label(&self) -> String {
        match self.state() {
            AnimationState::Confirmed(factor) if (factor - 1.0).abs() < 0.001 => {
                "Animations: normal".into()
            }
            AnimationState::Confirmed(factor) if factor < 1.0 => {
                format!("Animations: {}× faster", 1.0 / factor)
            }
            AnimationState::Confirmed(factor) => format!("Animations: {factor}× slower"),
            AnimationState::Restoring(factor) => format!("Animations: restoring {factor}×…"),
            AnimationState::Waiting => "Animations: waiting".into(),
            AnimationState::Checking => "Animations: checking…".into(),
            AnimationState::Unavailable => "Animations: unavailable".into(),
            AnimationState::Disconnected => "Animations: disconnected".into(),
        }
    }

    pub fn short_label(&self) -> String {
        match self.state() {
            AnimationState::Confirmed(factor) => format!("{factor}×"),
            AnimationState::Restoring(factor) => format!("{factor}×…"),
            AnimationState::Waiting | AnimationState::Checking => "…".into(),
            _ => "—".into(),
        }
    }

    pub fn toggle(&self) -> Result<()> {
        let state = lock_state(&self.state);
        if self.pending_toggle.load(Ordering::Acquire) {
            bail!("Animation control is busy; try again shortly.");
        }
        match &*state {
            AnimationState::Confirmed(_) => {}
            AnimationState::Waiting | AnimationState::Checking | AnimationState::Restoring(_) => {
                bail!(
                    "Animation control is waiting for Flutter's VM Service. Try Actions → Reconnect animation control."
                )
            }
            AnimationState::Unavailable => bail!(
                "Animation control needs a Flutter debug/profile app with the timeDilation service extension."
            ),
            AnimationState::Disconnected => {
                bail!(
                    "Cannot reach Flutter's VM Service. Try Actions → Reconnect animation control."
                )
            }
        }
        self.pending_toggle.store(true, Ordering::Release);
        if self.commands.try_send(Command::Toggle).is_err() {
            self.pending_toggle.store(false, Ordering::Release);
            bail!("Animation control is busy; try again shortly.");
        }
        Ok(())
    }

    pub fn reconnect(&self) -> Result<()> {
        if !self.enabled {
            bail!("Animation control needs a Flutter debug/profile app.");
        }
        self.commands
            .try_send(Command::Reconnect)
            .map_err(|_| anyhow::anyhow!("Animation control is busy; try again shortly."))
    }

    /// Wake the worker before forwarding `R`; output markers also cover
    /// restarts initiated through other Flutter terminal input paths.
    pub fn restarting(&self) {
        if self.enabled {
            let _ = self.commands.try_send(Command::Restarting);
        }
    }

    /// Capture complete tool lines, stripping ANSI codes even across chunks.
    pub fn feed(&mut self, bytes: &[u8]) {
        if !self.enabled {
            return;
        }
        for byte in bytes.iter().copied() {
            match self.escape {
                Escape::Start => {
                    self.escape = match byte {
                        b'[' => Escape::Csi,
                        b']' => Escape::Osc,
                        _ => Escape::Text,
                    }
                }
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = Escape::Text;
                    }
                }
                Escape::Osc => match byte {
                    7 => self.escape = Escape::Text,
                    0x1b => self.escape = Escape::OscEnd,
                    _ => {}
                },
                Escape::OscEnd => {
                    self.escape = if byte == b'\\' {
                        Escape::Text
                    } else {
                        Escape::Osc
                    }
                }
                Escape::Text => match byte {
                    0x1b => self.escape = Escape::Start,
                    b'\n' | b'\r' => {
                        self.finish_line();
                        self.line.clear();
                    }
                    byte if byte >= 0x20 || byte == b'\t' => {
                        if self.line.len() >= 16_384 {
                            self.line.clear();
                        }
                        self.line.push(byte);
                    }
                    _ => {}
                },
            }
        }
    }

    fn finish_line(&mut self) {
        let line = String::from_utf8_lossy(&self.line);
        if line.contains("Performing hot restart") {
            self.restarting();
        } else if line.contains("Restarted application in ") {
            let _ = self.commands.try_send(Command::Restarted);
        }
        let candidate = if line.contains("A Dart VM Service on ") {
            line.split_once("is available at:").map(|(_, url)| url)
        } else {
            [
                "Debug service listening on ",
                "The Dart VM service is listening on ",
                "Connecting to VM Service at ",
            ]
            .iter()
            .find_map(|marker| line.split_once(marker).map(|(_, url)| url))
        };
        if let Some(candidate) = candidate.and_then(|value| value.split_whitespace().next())
            && let Some(url) = service_url(candidate)
            && self.endpoint.as_ref() != Some(&url)
            && self
                .commands
                .try_send(Command::Connect(url.clone()))
                .is_ok()
        {
            self.endpoint = Some(url);
        }
    }
}

impl Drop for AnimationControl {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn service_url(candidate: &str) -> Option<Url> {
    let mut url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "ws")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    match url.host()? {
        url::Host::Ipv4(ip) if ip.is_loopback() => {}
        url::Host::Ipv6(ip) if ip.is_loopback() => {}
        url::Host::Domain("localhost") => {}
        _ => return None,
    }
    if url.scheme() == "ws" {
        url.set_scheme("http").ok()?;
    }
    let path = url
        .path()
        .trim_end_matches('/')
        .strip_suffix("/ws")
        .unwrap_or(url.path().trim_end_matches('/'))
        .to_owned();
    url.set_path(&format!("{path}/"));
    Some(url)
}

fn read_service_file(path: &Path) -> Option<Url> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 16_384 {
        return None;
    }
    let mut value = String::new();
    file.take(16_385).read_to_string(&mut value).ok()?;
    if value.len() > 16_384 {
        return None;
    }
    service_url(value.trim())
}

fn rpc(
    agent: &ureq::Agent,
    endpoint: &Url,
    method: &str,
    params: &[(&str, &str)],
    cancelled: &AtomicBool,
) -> Result<Value> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    let mut url = endpoint.join(method)?;
    url.query_pairs_mut().extend_pairs(params.iter().copied());
    let response: Value = agent
        .get(url.as_str())
        .call()
        .map_err(|error| match error {
            ureq::Error::Timeout(_) => anyhow::anyhow!("VM Service request timed out"),
            ureq::Error::StatusCode(code) => anyhow::anyhow!("VM Service returned HTTP {code}"),
            _ => anyhow::anyhow!("Could not connect to VM Service"),
        })?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_json()
        .map_err(|_| anyhow::anyhow!("Invalid VM Service response"))?;
    if response.get("error").is_some() {
        bail!("VM Service rejected the request");
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Invalid VM Service response"))
}

fn factor(result: &Value) -> Result<f64> {
    let value = result
        .get("timeDilation")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        })
        .filter(|v| v.is_finite() && *v > 0.0);
    value.ok_or_else(|| anyhow::anyhow!("Invalid timeDilation response"))
}

fn read_factor(
    agent: &ureq::Agent,
    endpoint: &Url,
    toggle: bool,
    preference: &mut Preference,
    cancelled: &AtomicBool,
) -> Result<Option<f64>> {
    let vm = rpc(agent, endpoint, "getVM", &[], cancelled)?;
    let isolates = vm
        .get("isolates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Invalid isolate list"))?;
    if preference.isolate.as_deref().is_some_and(|previous| {
        !isolates
            .iter()
            .any(|isolate| isolate.get("id").and_then(Value::as_str) == Some(previous))
    }) {
        preference.restore = preference.factor.is_some();
    }
    for isolate in isolates.iter().take(32) {
        let Some(id) = isolate.get("id").and_then(Value::as_str) else {
            continue;
        };
        let isolate = rpc(
            agent,
            endpoint,
            "getIsolate",
            &[("isolateId", id)],
            cancelled,
        )?;
        if !isolate
            .get("extensionRPCs")
            .and_then(Value::as_array)
            .is_some_and(|extensions| {
                extensions
                    .iter()
                    .any(|value| value.as_str() == Some(EXTENSION))
            })
        {
            continue;
        }
        let current = factor(&rpc(
            agent,
            endpoint,
            EXTENSION,
            &[("isolateId", id)],
            cancelled,
        )?)?;
        let replaced = preference
            .isolate
            .as_deref()
            .is_some_and(|previous| previous != id);
        preference.restore |= replaced || preference.restarting;
        preference.isolate = Some(id.into());
        if toggle {
            preference.factor = Some(if (current - 1.0).abs() < 0.001 {
                5.0
            } else {
                1.0
            });
            preference.restore = true;
        } else if preference.factor.is_none() || !preference.restore {
            // A DevTools change on the same running isolate becomes the new
            // preference. A fresh isolate's default must not erase that choice.
            preference.factor = Some(current);
        }
        let target = preference.factor.unwrap_or(current);
        let confirmed = if preference.restore && (current - target).abs() >= 0.001 {
            // Keep the target after transient failures, and retry on a later
            // poll instead of replacing it with Flutter's reset value.
            let target_value = if target.fract() == 0.0 {
                format!("{target:.1}")
            } else {
                target.to_string()
            };
            let actual = factor(&rpc(
                agent,
                endpoint,
                EXTENSION,
                &[("isolateId", id), ("timeDilation", &target_value)],
                cancelled,
            )?)?;
            if (actual - target).abs() >= 0.001 {
                bail!("Flutter did not confirm the selected animation speed");
            }
            actual
        } else {
            current
        };
        preference.restore = false;
        if replaced {
            preference.restarting = false;
        }
        return Ok(Some(confirmed));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poisoned_state_recovers_without_claiming_an_animation_factor() {
        let control = AnimationControl::new(false);
        let state = Arc::clone(&control.state);
        assert!(
            thread::spawn(move || {
                let mut guard = state.lock().unwrap();
                *guard = AnimationState::Confirmed(5.0);
                panic!("Simulated worker failure");
            })
            .join()
            .is_err()
        );
        assert_eq!(control.state(), AnimationState::Disconnected);
        assert!(!control.state.is_poisoned());
        assert!(control.toggle().is_err());
    }

    #[test]
    fn service_endpoint_preserves_auth_and_normalizes_websockets() {
        assert_eq!(
            service_url("ws://127.0.0.1:8181/aCode=/ws")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8181/aCode=/"
        );
        assert!(service_url("http://example.com:80/auth/").is_none());
        assert!(service_url("http://127.0.0.1:8181/auth/?uri=secret").is_none());
        assert_eq!(
            service_url("http://[::1]:8181/auth/").unwrap().path(),
            "/auth/"
        );
    }
    #[test]
    fn line_detection_handles_chunks_and_ansi_and_ignores_devtools_urls() {
        let mut control = AnimationControl::new(true);
        control.feed(b"DevTools at http://127.0.0.1:8181/?uri=secret\nA Dart VM Ser");
        assert!(control.endpoint.is_none());
        control.feed(b"vice on Test is available at: \x1b[32mhttp://127.0.0.1:8181/code=/\x1b[");
        assert!(control.endpoint.is_none());
        control.feed(b"0m\r\n");
        assert_eq!(control.endpoint.as_ref().unwrap().path(), "/code=/");
    }
    #[test]
    fn service_file_rejects_oversized_non_local_and_invalid_addresses() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("service-uri");
        assert!(read_service_file(&path).is_none());
        for value in [
            "http://example.com:8181/token/".into(),
            "x".repeat(16_385),
            "ws://".into(),
        ] {
            std::fs::write(&path, value).unwrap();
            assert!(read_service_file(&path).is_none());
        }
        std::fs::write(&path, "ws://127.0.0.1:8181/token=/ws\r\n").unwrap();
        assert_eq!(
            read_service_file(&path).unwrap().as_str(),
            "http://127.0.0.1:8181/token=/"
        );
    }

    #[test]
    fn carriage_return_terminated_service_line_is_detected() {
        let mut control = AnimationControl::new(true);
        control.feed(b"Connecting to VM Service at ws://127.0.0.1:8181/token=/ws\r");
        assert_eq!(control.endpoint.as_ref().unwrap().path(), "/token=/");
    }

    #[test]
    fn invalid_factors_and_release_control_never_claim_to_be_enabled() {
        for value in [
            serde_json::json!({"timeDilation":"NaN"}),
            serde_json::json!({"timeDilation":0}),
            serde_json::json!({}),
        ] {
            assert!(factor(&value).is_err());
        }
        let control = AnimationControl::new(false);
        assert_eq!(control.label(), "Animations: unavailable");
        assert!(control.toggle().is_err());
        assert!(control.reconnect().is_err());
    }
}
