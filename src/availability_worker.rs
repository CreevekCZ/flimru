//! One owned worker keeps filesystem-backed previews off the terminal loop.
use flimru::{
    availability::{self, Assessment},
    config::LaunchFile,
    device::Device,
    tool::FlutterTool,
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};

struct Job {
    generation: u64,
    launch: Arc<LaunchFile>,
    tool: FlutterTool,
    targets: Vec<Device>,
    ownit: bool,
}
struct Shared {
    job: Mutex<Option<Job>>,
    ready: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
}
type Results = (u64, Vec<(usize, Assessment)>);

pub(super) struct Worker {
    shared: Arc<Shared>,
    results: mpsc::Receiver<Results>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Default for Worker {
    fn default() -> Self {
        let shared = Arc::new(Shared {
            job: Mutex::new(None),
            ready: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let (sender, results) = mpsc::sync_channel(2);
        let worker_shared = shared.clone();
        let thread = std::thread::spawn(move || run(worker_shared, sender));
        Self {
            shared,
            results,
            thread: Some(thread),
        }
    }
}
impl Worker {
    pub fn request(
        &self,
        generation: u64,
        launch: Arc<LaunchFile>,
        tool: FlutterTool,
        targets: Vec<Device>,
        ownit: bool,
    ) {
        self.shared.generation.store(generation, Ordering::Release);
        let mut job = self
            .shared
            .job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *job = Some(Job {
            generation,
            launch,
            tool,
            targets,
            ownit,
        });
        self.shared.ready.notify_one();
    }
    pub fn drain(&self, generation: u64) -> Vec<(usize, Assessment)> {
        let mut results = Vec::new();
        // A continuously producing worker must not monopolize input/PTY polling.
        for _ in 0..2 {
            let Ok((received, chunk)) = self.results.try_recv() else {
                break;
            };
            if generation == received {
                results.extend(chunk);
            }
        }
        results
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Synchronize with the wait predicate to avoid a lost shutdown wake.
        {
            let _job = self
                .shared
                .job
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.shared.stop.store(true, Ordering::Release);
            self.shared.ready.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn current(shared: &Shared, generation: u64) -> bool {
    !shared.stop.load(Ordering::Acquire) && shared.generation.load(Ordering::Acquire) == generation
}
fn send(shared: &Shared, sender: &mpsc::SyncSender<Results>, mut result: Results) -> bool {
    while current(shared, result.0) {
        match sender.try_send(result) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Full(returned)) => {
                result = returned;
                std::thread::park_timeout(std::time::Duration::from_millis(2));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        }
    }
    false
}
fn run(shared: Arc<Shared>, sender: mpsc::SyncSender<Results>) {
    loop {
        let job = {
            let mut slot = shared
                .job
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while slot.is_none() && !shared.stop.load(Ordering::Acquire) {
                slot = shared
                    .ready
                    .wait(slot)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if shared.stop.load(Ordering::Acquire) {
                return;
            }
            slot.take()
        };
        let Some(job) = job else {
            continue;
        };
        let targets: Vec<_> = job.targets.iter().collect();
        let mut chunk = Vec::with_capacity(128);
        for (index, entry) in job.launch.entries.iter().enumerate() {
            if !current(&shared, job.generation) {
                break;
            }
            let mut assessment =
                availability::assess(&job.launch, entry, &job.tool, &targets, job.ownit);
            assessment.plans.clear();
            chunk.push((index, assessment));
            if chunk.len() == 128 {
                if !send(
                    &shared,
                    &sender,
                    (job.generation, std::mem::take(&mut chunk)),
                ) {
                    break;
                }
                chunk = Vec::with_capacity(128);
            }
        }
        if !chunk.is_empty() {
            send(&shared, &sender, (job.generation, chunk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn target_change_discards_queued_stale_results_and_assesses_the_latest_snapshot() {
        let workspace = std::env::temp_dir();
        let raw = serde_json::json!({"configurations":(0..1024).map(|i|
            serde_json::json!({"name":format!("Pinned {i}"),"deviceId":"old"})).collect::<Vec<_>>()}).to_string();
        let launch =
            Arc::new(LaunchFile::parse(&raw, &workspace.join("launch.json"), &workspace).unwrap());
        let tool = FlutterTool {
            executable: "/nonexistent/tool".into(),
            prefix: Vec::new(),
            workspace,
        };
        let old = Device {
            id: "old".into(),
            name: "Old target".into(),
            platform: "ios".into(),
            available: true,
            favorite: false,
        };
        let worker = Worker::default();
        worker.request(1, launch.clone(), tool.clone(), vec![old.clone()], false);
        let deadline = Instant::now() + Duration::from_secs(5);
        // Wait until an old result exists before changing context. More old
        // results may already be queued; draining generation 2 must reject them.
        loop {
            if let Ok((generation, chunk)) = worker.results.try_recv() {
                assert_eq!(generation, 1);
                assert!(chunk.iter().all(|(_, assessment)| assessment.runnable()));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let new = Device {
            id: "new".into(),
            name: "New target".into(),
            ..old
        };
        worker.request(2, launch, tool, vec![new], false);
        let mut received = std::collections::BTreeSet::new();
        while received.len() < 1024 {
            for (index, assessment) in worker.drain(2) {
                assert!(received.insert(index), "duplicate result");
                assert!(!assessment.runnable());
                assert!(
                    assessment
                        .reasons
                        .iter()
                        .any(|reason| reason.code == "deviceId"
                            && reason.target.as_deref() == Some("New target"))
                );
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn catalog_replacement_discards_old_source_indexes() {
        let workspace = std::env::temp_dir();
        let old = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"Old"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        let new = Arc::new(
            LaunchFile::parse(
                r#"{"configurations":[{"name":"New","type":"unsupported"}]}"#,
                &workspace.join("launch.json"),
                &workspace,
            )
            .unwrap(),
        );
        let worker = Worker::default();
        let tool = FlutterTool {
            executable: "/nonexistent/tool".into(),
            prefix: Vec::new(),
            workspace,
        };
        worker.request(1, old, tool.clone(), Vec::new(), false);
        worker.request(2, new, tool, Vec::new(), false);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let results = worker.drain(2);
            if let Some((index, assessment)) = results.first() {
                assert_eq!(*index, 0);
                assert!(assessment.reasons[0].message.contains("type must be dart"));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
