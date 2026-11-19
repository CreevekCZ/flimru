//! Project-scoped Unix socket. Only the TUI owns this listener.

use super::{Request, WireRequest};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use socket2::{Domain, SockAddr, Socket, Type};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    net::Shutdown,
    os::fd::OwnedFd,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_CONNECTIONS: usize = 16;
pub const MAX_REQUEST: usize = 32 * 1024;
const MAX_RESPONSE: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const UI_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Serialize, Deserialize)]
enum Reply {
    Success(Value),
    Error(String),
}

struct Pending {
    request: Request,
    deadline: Instant,
    reply: mpsc::SyncSender<Reply>,
}

pub struct Server {
    listener: UnixListener,
    receiver: Option<mpsc::Receiver<Pending>>,
    sender: mpsc::SyncSender<Pending>,
    workers: Vec<Worker>,
    endpoint: Endpoint,
    _lock: File,
    pub instance: String,
}

struct Worker {
    handle: JoinHandle<()>,
    stream: UnixStream,
}

struct Endpoint {
    root: PathBuf,
    socket: PathBuf,
    lock: PathBuf,
}

impl Endpoint {
    fn new(workspace: &Path, create: bool) -> Result<Self> {
        // SAFETY: geteuid has no arguments or memory effects.
        let uid = unsafe { libc::geteuid() };
        // Keep sun_path short even when the Flutter checkout has a long path.
        let root = PathBuf::from(format!("/tmp/flimru-{uid}"));
        if create {
            match fs::DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).context("Could not create private Flimru directory");
                }
            }
        }
        let meta = fs::symlink_metadata(&root)?;
        ensure!(
            meta.is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700,
            "{} must be a real directory owned by the current user with mode 0700",
            root.display()
        );
        let key = Sha256::digest(workspace.as_os_str().as_encoded_bytes());
        let stem: String = key[..16].iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            socket: root.join(format!("{stem}.sock")),
            lock: root.join(format!("{stem}.lock")),
            root,
        })
    }

    /// Serialize lock-file creation and unlinking. The project lock itself is
    /// held for the listener's lifetime, including while the UI is idle.
    fn lock_directory(&self) -> io::Result<File> {
        let directory = File::open(&self.root)?;
        directory.lock()?;
        Ok(directory)
    }
}

impl Server {
    pub fn bind(workspace: &Path) -> Result<Self> {
        let endpoint = Endpoint::new(workspace, true)?;
        let directory = endpoint.lock_directory()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&endpoint.lock)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!("Another Flimru window for this project owns CLI and MCP access.")
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let listener = (|| -> io::Result<UnixListener> {
            remove_if_present(&endpoint.socket)?;
            let listener = UnixListener::bind(&endpoint.socket)?;
            listener.set_nonblocking(true)?;
            Ok(listener)
        })();
        let listener = match listener {
            Ok(listener) => listener,
            Err(error) => {
                let _ = remove_if_present(&endpoint.socket);
                let _ = remove_if_present(&endpoint.lock);
                return Err(error).context("Could not open the local Flimru socket");
            }
        };
        drop(directory);
        let (sender, receiver) = mpsc::sync_channel(MAX_CONNECTIONS);
        let instance = format!(
            "{:x}{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            std::process::id()
        );
        Ok(Self {
            listener,
            receiver: Some(receiver),
            sender,
            workers: Vec::new(),
            endpoint,
            instance,
            _lock: lock,
        })
    }

    pub fn accept(&mut self) {
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].handle.is_finished() {
                let _ = self.workers.swap_remove(index).handle.join();
            } else {
                index += 1;
            }
        }
        for _ in 0..4 {
            let Ok((stream, _)) = self.listener.accept() else {
                break;
            };
            if self.workers.len() >= MAX_CONNECTIONS {
                continue;
            }
            let Ok(interrupt) = stream.try_clone() else {
                continue;
            };
            let sender = self.sender.clone();
            if let Ok(handle) =
                thread::Builder::new()
                    .name("flimru-client".into())
                    .spawn(move || {
                        let _ = serve_client(&stream, &sender);
                        let _ = stream.shutdown(Shutdown::Both);
                    })
            {
                self.workers.push(Worker {
                    handle,
                    stream: interrupt,
                });
            }
        }
    }

    pub fn service(&self, mut handle: impl FnMut(Request) -> Result<Value>) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        // Bound the work accepted in a single UI frame. All reads/writes on
        // client sockets happen on workers, including slow or abandoned reads.
        for _ in 0..4 {
            let Ok(pending) = receiver.try_recv() else {
                break;
            };
            if Instant::now() >= pending.deadline {
                continue;
            }
            let reply = match handle(pending.request) {
                Ok(value) => Reply::Success(value),
                Err(error) => Reply::Error(
                    format!("{error:#}")
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(4096)
                        .collect(),
                ),
            };
            let _ = pending.reply.try_send(reply);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.receiver.take();
        for worker in &self.workers {
            let _ = worker.stream.shutdown(Shutdown::Both);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.handle.join();
        }
        if let Ok(_directory) = self.endpoint.lock_directory() {
            let _ = remove_if_present(&self.endpoint.socket);
            let _ = remove_if_present(&self.endpoint.lock);
        }
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn serve_client(stream: &UnixStream, sender: &mpsc::SyncSender<Pending>) -> Result<()> {
    let bytes = read_frame(stream, MAX_REQUEST, REQUEST_TIMEOUT)?;
    let wire = serde_json::from_slice::<WireRequest>(&bytes);
    let reply = match wire {
        Ok(wire) if wire.version == super::WIRE_VERSION => {
            let (reply, response) = mpsc::sync_channel(1);
            let pending = Pending {
                request: wire.request,
                deadline: Instant::now() + UI_TIMEOUT,
                reply,
            };
            if sender.try_send(pending).is_err() {
                Reply::Error("Flimru is busy. Wait briefly and retry.".into())
            } else {
                response.recv_timeout(UI_TIMEOUT).unwrap_or_else(|_| {
                    Reply::Error("Flimru did not respond in time. Check the session list before retrying a launch or control.".into())
                })
            }
        }
        _ => Reply::Error("Invalid request or incompatible Flimru version. Use the same binary for the TUI and clients.".into()),
    };
    let mut bytes = serde_json::to_vec(&reply)?;
    if bytes.len() > MAX_RESPONSE {
        bytes = serde_json::to_vec(&Reply::Error(
            "Flimru reply exceeded the size limit.".into(),
        ))?;
    }
    write_frame(stream, &bytes)
}

fn read_frame(mut stream: &UnixStream, limit: usize, timeout: Duration) -> Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut body = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        ensure!(!left.is_zero(), "Flimru socket read timed out");
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(body),
            Ok(count) => {
                ensure!(
                    body.len() + count <= limit,
                    "Flimru request or reply exceeded the size limit"
                );
                body.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_frame(mut stream: &UnixStream, mut body: &[u8]) -> Result<()> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    while !body.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        ensure!(!left.is_zero(), "Flimru socket write timed out");
        stream.set_write_timeout(Some(left))?;
        match stream.write(body) {
            Ok(0) => bail!("Flimru socket closed while writing"),
            Ok(count) => body = &body[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn request(workspace: &Path, request: Request) -> Result<Value> {
    let missing_ui = || {
        format!(
            "No shared Flimru instance for this project. Open Flimru in {workspace:?} and leave its terminal running."
        )
    };
    let endpoint = Endpoint::new(workspace, false).map_err(|error| {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            anyhow::anyhow!(missing_ui())
        } else {
            error.context("Could not access the private Flimru socket directory")
        }
    })?;
    let stream = connect(&endpoint.socket).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => anyhow::anyhow!(missing_ui()),
        _ => anyhow::Error::new(error).context("Could not connect to the local Flimru instance"),
    })?;
    let bytes = serde_json::to_vec(&WireRequest {
        version: super::WIRE_VERSION,
        request,
    })?;
    ensure!(
        bytes.len() <= MAX_REQUEST,
        "Flimru request exceeded the size limit"
    );
    write_frame(&stream, &bytes)?;
    stream.shutdown(Shutdown::Write)?;
    let bytes = read_frame(&stream, MAX_RESPONSE, RESPONSE_TIMEOUT)
        .context("The Flimru connection ended or timed out. Check the session list before retrying a launch or control.")?;
    match serde_json::from_slice(&bytes).context(
        "Flimru closed the connection or returned an incompatible reply. Check that it is still open and uses the same binary.",
    )? {
        Reply::Success(value) => Ok(value),
        Reply::Error(message) => bail!("{message}"),
    }
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    // A congested accept queue can block a normal UnixStream::connect.
    socket.connect_timeout(&SockAddr::unix(path)?, REQUEST_TIMEOUT)?;
    Ok(UnixStream::from(OwnedFd::from(socket)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_is_replaced_and_a_stalled_reader_does_not_block_other_clients() -> Result<()> {
        let project = tempfile::tempdir()?;
        let workspace = fs::canonicalize(project.path())?;
        let endpoint = Endpoint::new(&workspace, true)?;
        drop(UnixListener::bind(&endpoint.socket)?);
        let mut server = Server::bind(&workspace)?;

        let stalled = connect(&endpoint.socket)?;
        let request_bytes = serde_json::to_vec(&WireRequest {
            version: super::super::WIRE_VERSION,
            request: Request::Targets { config_offset: 0 },
        })?;
        write_frame(&stalled, &request_bytes)?;
        stalled.shutdown(Shutdown::Write)?;
        let mut big_reply = Some(Value::String("x".repeat(4 * 1024 * 1024)));
        let deadline = Instant::now() + Duration::from_secs(3);
        while big_reply.is_some() {
            ensure!(
                Instant::now() < deadline,
                "Stalled client request was not serviced"
            );
            server.accept();
            server.service(|_| big_reply.take().context("Duplicate request"));
            thread::sleep(Duration::from_millis(5));
        }

        let fast_workspace = workspace.clone();
        let fast = thread::spawn(move || request(&fast_workspace, Request::Sessions { id: None }));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !fast.is_finished() && Instant::now() < deadline {
            server.accept();
            server.service(|_| Ok(serde_json::json!({"sessions": []})));
            thread::sleep(Duration::from_millis(5));
        }
        let finished = fast.is_finished();
        drop(server);
        ensure!(
            fast.join()
                .map_err(|_| anyhow::anyhow!("Client panicked"))??["sessions"]
                == serde_json::json!([])
        );
        ensure!(
            finished,
            "Another client's reply was blocked by the stalled reader"
        );
        ensure!(!endpoint.socket.exists() && !endpoint.lock.exists());
        Ok(())
    }
}
