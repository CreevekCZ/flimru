// Test fixtures intentionally fail fast on setup/protocol errors.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Exercise the VM Service client against an isolated local HTTP service.
use flimru::animation::{AnimationControl, AnimationState};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use url::Url;

const EXTENSION: &str = "ext.flutter.timeDilation";
const AUTH_PATH: &str = "/authCode=/";

#[derive(Clone)]
struct ServiceState {
    factor: f64,
    isolate: String,
    extension_available: bool,
    reject_writes: bool,
    fail_all: bool,
    stall: bool,
}

#[derive(Debug, Clone)]
struct Request {
    path: String,
    params: Vec<(String, String)>,
}

impl Request {
    fn parameter(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

struct Service {
    endpoint: String,
    state: Arc<Mutex<ServiceState>>,
    requests: Arc<Mutex<Vec<Request>>>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Service {
    fn new(extension_available: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}{AUTH_PATH}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ServiceState {
            factor: 1.0,
            isolate: "isolates/first Flutter app".into(),
            extension_available,
            reject_writes: false,
            fail_all: false,
            stall: false,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let worker = {
            let state = state.clone();
            let requests = requests.clone();
            let stopped = stopped.clone();
            thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => handle(stream, &state, &requests, &stopped),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("Fake VM Service could not accept: {error}"),
                    }
                }
            })
        };
        Self {
            endpoint,
            state,
            requests,
            stopped,
            worker: Some(worker),
        }
    }

    fn connect(&self, control: &mut AnimationControl) {
        // Flutter's output may be chunked, colored, and use a websocket URL.
        let endpoint = self.endpoint.replacen("http://", "ws://", 1);
        control.feed(b"A Dart VM Ser");
        control.feed(
            format!("vice on Test device is available at: \x1b[32m{endpoint}ws\x1b[").as_bytes(),
        );
        control.feed(b"0m\r\n");
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn handle(
    mut stream: TcpStream,
    state: &Mutex<ServiceState>,
    requests: &Mutex<Vec<Request>>,
    stopped: &AtomicBool,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        if stopped.load(Ordering::Relaxed) || Instant::now() >= deadline || bytes.len() > 8192 {
            return;
        }
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
    let text = String::from_utf8(bytes).unwrap();
    let target = text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = Url::parse(&format!("http://127.0.0.1{target}")).unwrap();
    let request = Request {
        path: url.path().into(),
        params: url.query_pairs().into_owned().collect(),
    };
    requests.lock().unwrap().push(request.clone());
    let stalled = state.lock().unwrap().stall;
    if stalled {
        while !stopped.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    let mut state = state.lock().unwrap();
    let result = response(&request, &mut state);
    drop(state);
    let body = result.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn response(request: &Request, state: &mut ServiceState) -> Value {
    let error = || json!({"jsonrpc":"2.0", "error":{"code":-32000,"message":"Rejected"}});
    if state.fail_all {
        return error();
    }
    let Some(method) = request.path.strip_prefix(AUTH_PATH) else {
        return error();
    };
    let result = match method {
        "getVM" => json!({"isolates":[{"id":state.isolate}]}),
        "getIsolate" if request.parameter("isolateId") == Some(&state.isolate) => {
            let extensions = if state.extension_available {
                vec![EXTENSION]
            } else {
                vec!["ext.dart.io.getOpenFiles"]
            };
            json!({"extensionRPCs":extensions})
        }
        EXTENSION if request.parameter("isolateId") == Some(&state.isolate) => {
            if let Some(value) = request.parameter("timeDilation") {
                if state.reject_writes {
                    return error();
                }
                state.factor = value.parse().unwrap();
            }
            // Flutter's service extension returns the factor as a string.
            json!({"timeDilation":state.factor.to_string()})
        }
        _ => return error(),
    };
    json!({"jsonrpc":"2.0", "result":result})
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "VM Service condition did not arrive"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn confirmed(control: &AnimationControl, expected: f64) {
    until(|| control.state() == AnimationState::Confirmed(expected));
}

#[test]
fn toggles_use_authenticated_endpoint_and_confirm_actual_factors() {
    let service = Service::new(true);
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    confirmed(&control, 1.0);
    assert_eq!(control.label(), "Animations: normal");

    control.toggle().unwrap();
    confirmed(&control, 5.0);
    assert_eq!(control.label(), "Animations: 5× slower");
    assert_eq!(control.short_label(), "5×");

    control.toggle().unwrap();
    confirmed(&control, 1.0);
    let requests = service.requests();
    assert!(
        requests
            .iter()
            .all(|request| request.path.starts_with(AUTH_PATH))
    );
    let extension_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.path.ends_with(EXTENSION))
        .collect();
    assert!(
        extension_requests.iter().all(|request| {
            request.parameter("isolateId") == Some("isolates/first Flutter app")
        })
    );
    let writes: Vec<_> = extension_requests
        .iter()
        .filter_map(|request| request.parameter("timeDilation"))
        .collect();
    assert_eq!(writes, ["5.0", "1.0"]);
}

#[test]
fn polling_observes_devtools_changes_and_new_isolate_after_hot_restart() {
    let service = Service::new(true);
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    confirmed(&control, 1.0);
    service.state.lock().unwrap().factor = 3.0;
    confirmed(&control, 3.0);
    assert_eq!(control.label(), "Animations: 3× slower");

    {
        let mut state = service.state.lock().unwrap();
        state.isolate = "isolates/restarted?app=2".into();
        state.factor = 1.0;
    }
    until(|| {
        service.requests().iter().any(|request| {
            request.path.ends_with(EXTENSION)
                && request.parameter("isolateId") == Some("isolates/restarted?app=2")
                && request.parameter("timeDilation") == Some("3.0")
        })
    });
    confirmed(&control, 3.0);
    control.toggle().unwrap();
    confirmed(&control, 1.0);
    control.toggle().unwrap();
    confirmed(&control, 5.0);
    assert!(service.requests().iter().any(|request| {
        request.path.ends_with(EXTENSION)
            && request.parameter("isolateId") == Some("isolates/restarted?app=2")
            && request.parameter("timeDilation") == Some("5.0")
    }));
}

#[test]
fn selected_speed_survives_extension_registration_delay_and_transient_restart_failures() {
    let service = Service::new(true);
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    confirmed(&control, 1.0);
    control.toggle().unwrap();
    confirmed(&control, 5.0);
    control.restarting();
    {
        let mut state = service.state.lock().unwrap();
        state.isolate = "isolates/restarting".into();
        state.factor = 1.0;
        state.extension_available = false;
    }
    until(|| control.state() == AnimationState::Restoring(5.0));
    assert_eq!(control.label(), "Animations: restoring 5×…");
    assert_eq!(control.short_label(), "5×…");
    assert!(control.toggle().is_err());
    {
        let mut state = service.state.lock().unwrap();
        state.extension_available = true;
        state.reject_writes = true;
    }
    until(|| control.state() == AnimationState::Disconnected);
    assert_eq!(service.state.lock().unwrap().factor, 1.0);
    service.state.lock().unwrap().reject_writes = false;
    let ready = Instant::now();
    confirmed(&control, 5.0);
    assert!(
        ready.elapsed() < Duration::from_millis(700),
        "Restart restoration used the idle polling interval"
    );

    control.toggle().unwrap();
    confirmed(&control, 1.0);
    {
        let mut state = service.state.lock().unwrap();
        state.isolate = "isolates/normal-after-restart".into();
        state.factor = 1.0;
    }
    control.feed(b"Performing hot restart...\nRestarted application in 30ms.\n");
    until(|| {
        service.requests().iter().any(|request| {
            request.parameter("isolateId") == Some("isolates/normal-after-restart")
                && request.path.ends_with(EXTENSION)
        })
    });
    confirmed(&control, 1.0);
    assert!(
        !service
            .requests()
            .iter()
            .any(
                |request| request.parameter("isolateId") == Some("isolates/normal-after-restart")
                    && request.parameter("timeDilation").is_some()
            )
    );
}

#[test]
fn reconnection_and_reused_isolate_ids_restore_the_preference_without_crossing_consoles() {
    let first = Service::new(true);
    let second = Service::new(true);
    let mut slow = AnimationControl::new(true);
    let mut normal = AnimationControl::new(true);
    first.connect(&mut slow);
    second.connect(&mut normal);
    confirmed(&slow, 1.0);
    confirmed(&normal, 1.0);
    slow.toggle().unwrap();
    confirmed(&slow, 5.0);
    // A tool completion marker also covers services which reuse isolate IDs.
    first.state.lock().unwrap().factor = 1.0;
    slow.feed(b"Restarted application in 50ms.\n");
    until(|| {
        first
            .requests()
            .iter()
            .filter(|request| request.parameter("timeDilation") == Some("5.0"))
            .count()
            == 2
    });
    confirmed(&slow, 5.0);
    confirmed(&normal, 1.0);
    assert_eq!(second.state.lock().unwrap().factor, 1.0);

    // The VM URL/auth token may change as well as the isolate identity.
    let replacement = Service::new(true);
    replacement.connect(&mut slow);
    until(|| replacement.state.lock().unwrap().factor == 5.0);
    confirmed(&slow, 5.0);
    confirmed(&normal, 1.0);
}

#[test]
fn missing_flutter_extension_is_unavailable_without_sending_toggle() {
    let service = Service::new(false);
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    until(|| control.state() == AnimationState::Unavailable);
    assert_eq!(control.label(), "Animations: unavailable");
    assert!(control.toggle().is_err());
    assert!(
        service
            .requests()
            .iter()
            .all(|request| !request.path.ends_with(EXTENSION))
    );
}

#[test]
fn failed_toggle_never_claims_success_and_service_errors_disconnect() {
    let service = Service::new(true);
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    confirmed(&control, 1.0);
    service.state.lock().unwrap().reject_writes = true;
    control.toggle().unwrap();
    until(|| control.state() == AnimationState::Disconnected);
    assert_eq!(control.label(), "Animations: disconnected");
    assert_eq!(service.state.lock().unwrap().factor, 1.0);
    assert!(control.toggle().is_err());
    service.state.lock().unwrap().reject_writes = false;
    confirmed(&control, 5.0);
    service.state.lock().unwrap().fail_all = true;
    until(|| control.state() == AnimationState::Disconnected);
    assert_ne!(control.state(), AnimationState::Confirmed(5.0));
}

#[test]
fn cancelling_an_unresponsive_request_finishes_promptly() {
    let service = Service::new(true);
    service.state.lock().unwrap().stall = true;
    let mut control = AnimationControl::new(true);
    service.connect(&mut control);
    until(|| !service.requests().is_empty());
    let started = Instant::now();
    drop(control);
    assert!(started.elapsed() < Duration::from_secs(2));
}
