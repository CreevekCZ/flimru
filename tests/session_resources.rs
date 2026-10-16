#![cfg(unix)]
// Test fixtures intentionally fail fast on setup/protocol errors.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Lifecycle checks run in their own test process so FD counts are isolated.
use flimru::{session::Session, tool::RunCommand};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn command(directory: &Path, script: &str) -> RunCommand {
    RunCommand {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: directory.into(),
        env: Default::default(),
    }
}

fn open_descriptors() -> usize {
    (0..1024)
        .filter(|fd| {
            // SAFETY: F_GETFD accepts any integer descriptor, takes no extra
            // argument, and reports invalid descriptors without accessing memory.
            (unsafe { libc::fcntl(*fd, libc::F_GETFD) }) >= 0
        })
        .count()
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Session condition did not arrive"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_ready(directory: &Path) -> Session {
    let mut session = Session::start(
        &command(directory, "printf 'ready\\r\\n'; exec sleep 30"),
        10,
        40,
    )
    .unwrap();
    until(|| {
        assert!(session.tick().unwrap().is_none());
        session.parser.screen().contents().contains("ready")
    });
    session
}

#[test]
fn repeated_sessions_release_descriptors_and_exited_wrapper_descendants() {
    let directory = tempfile::tempdir().unwrap();
    // Warm lazy library initialization before measuring owned resources.
    drop(start_ready(directory.path()));
    let baseline = open_descriptors();
    for cycle in 0..20 {
        let session = start_ready(directory.path());
        let started = Instant::now();
        drop(session);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "Session {cycle} shutdown stalled"
        );
        assert_eq!(
            open_descriptors(),
            baseline,
            "Session {cycle} retained file descriptors"
        );
    }
    // A full multi-device batch must release every PTY/reader just as promptly.
    let sessions: Vec<_> = (0..8).map(|_| start_ready(directory.path())).collect();
    let started = Instant::now();
    drop(sessions);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        open_descriptors(),
        baseline,
        "Batch retained file descriptors"
    );

    // The wrapper exits, while one descendant floods its inherited PTY and
    // another records a heartbeat. EOF cannot arrive until the group is stopped.
    let script = "trap '' HUP; \
        (while :; do printf '%04096d\\r\\n' 1; done) & \
        (while :; do printf . >> heartbeat; sleep 0.05; done) & \
        exit 0";
    let mut session = Session::start(&command(directory.path(), script), 10, 40).unwrap();
    let heartbeat = directory.path().join("heartbeat");
    until(|| std::fs::metadata(&heartbeat).is_ok_and(|metadata| metadata.len() >= 2));
    let started = Instant::now();
    until(|| session.tick().unwrap().is_some());
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "Descendant output prevented session completion"
    );
    let dropping = Instant::now();
    drop(session);
    assert!(dropping.elapsed() < Duration::from_secs(1));
    // Allow the killed descendants to finish any already-issued filesystem I/O.
    thread::sleep(Duration::from_millis(50));
    let stopped_size = std::fs::metadata(&heartbeat).unwrap().len();
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        std::fs::metadata(&heartbeat).unwrap().len(),
        stopped_size,
        "A descendant kept running after its session was dropped"
    );
    assert_eq!(open_descriptors(), baseline);
}
