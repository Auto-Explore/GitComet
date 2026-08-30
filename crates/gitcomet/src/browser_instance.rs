use gitcomet_ui_gpui::{BrowserOpenRequest, BrowserOpenTarget};
use serde::{Deserialize, Serialize};
use smol::channel::{Receiver, Sender};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROTOCOL_VERSION: u32 = 1;
const PORT_BASE: u16 = 42_000;
const PORT_SPAN: u16 = 20_000;
const PORT_CANDIDATES: u16 = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const DESCRIPTOR_RETRY_WINDOW: Duration = Duration::from_millis(250);
const MAX_WIRE_BYTES: usize = 1024 * 1024;
const INSTANCE_FILE_ENV: &str = "GITCOMET_BROWSER_INSTANCE_FILE";

#[cfg(test)]
struct DescriptorPublishGate {
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
}

#[cfg(test)]
impl DescriptorPublishGate {
    fn new() -> Self {
        Self {
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    fn wait_until_entered(&self) {
        let (entered, wake) = &self.entered;
        let entered = entered.lock().unwrap_or_else(|error| error.into_inner());
        let _entered = wake
            .wait_while(entered, |entered| !*entered)
            .unwrap_or_else(|error| error.into_inner());
    }

    fn enter_and_wait(&self) {
        let (entered, wake) = &self.entered;
        *entered.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_all();

        let (released, wake) = &self.released;
        let released = released.lock().unwrap_or_else(|error| error.into_inner());
        let _released = wake
            .wait_while(released, |released| !*released)
            .unwrap_or_else(|error| error.into_inner());
    }

    fn release(&self) {
        let (released, wake) = &self.released;
        *released.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_all();
    }
}

#[cfg(test)]
fn descriptor_publish_gate_slot()
-> &'static Mutex<Option<(PathBuf, u16, Arc<DescriptorPublishGate>)>> {
    static GATE: OnceLock<Mutex<Option<(PathBuf, u16, Arc<DescriptorPublishGate>)>>> =
        OnceLock::new();
    GATE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn wait_before_descriptor_publish_for_test(path: &Path, port: u16) {
    let gate = descriptor_publish_gate_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .filter(|(gate_path, gate_port, _)| gate_path == path && *gate_port == port)
        .map(|(_, _, gate)| Arc::clone(gate));
    if let Some(gate) = gate {
        gate.enter_and_wait();
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InstanceDescriptor {
    version: u32,
    port: u16,
    token: String,
    pid: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireTarget {
    ExistingWindow,
    NewWindow,
}

impl From<BrowserOpenTarget> for WireTarget {
    fn from(value: BrowserOpenTarget) -> Self {
        match value {
            BrowserOpenTarget::ExistingWindow => Self::ExistingWindow,
            BrowserOpenTarget::NewWindow => Self::NewWindow,
        }
    }
}

impl From<WireTarget> for BrowserOpenTarget {
    fn from(value: WireTarget) -> Self {
        match value {
            WireTarget::ExistingWindow => Self::ExistingWindow,
            WireTarget::NewWindow => Self::NewWindow,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
enum WirePath {
    UnixBytes(Vec<u8>),
    WindowsWide(Vec<u16>),
    Utf8(String),
}

impl WirePath {
    fn from_path(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self::UnixBytes(path.as_os_str().as_bytes().to_vec())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self::WindowsWide(path.as_os_str().encode_wide().collect())
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self::Utf8(path.to_string_lossy().into_owned())
        }
    }

    fn into_path(self) -> Option<PathBuf> {
        match self {
            Self::UnixBytes(bytes) => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
                }
                #[cfg(not(unix))]
                {
                    let _ = bytes;
                    None
                }
            }
            Self::WindowsWide(wide) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStringExt;
                    Some(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
                }
                #[cfg(not(windows))]
                {
                    let _ = wide;
                    None
                }
            }
            Self::Utf8(path) => Some(PathBuf::from(path)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WireRequest {
    version: u32,
    token: String,
    path: Option<WirePath>,
    target: WireTarget,
}

pub(crate) enum StartResult {
    Forwarded,
    Primary(PrimaryBrowserInstance),
}

pub(crate) struct PrimaryBrowserInstance {
    requests: Receiver<BrowserOpenRequest>,
    _server: BrowserInstanceServer,
}

impl PrimaryBrowserInstance {
    pub(crate) fn requests(&self) -> Receiver<BrowserOpenRequest> {
        self.requests.clone()
    }
}

struct BrowserInstanceServer {
    descriptor_path: PathBuf,
    descriptor: InstanceDescriptor,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    // Keep the interprocess ownership claim until the server and descriptor
    // have both been torn down. A missing or temporarily unreadable
    // descriptor must never let another process establish a second primary.
    _claim: fs::File,
}

enum BrowserInstanceClaim {
    Acquired(fs::File),
    Forwarded,
}

impl Drop for BrowserInstanceServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }

        let owns_descriptor = read_descriptor(&self.descriptor_path).is_some_and(|current| {
            current.port == self.descriptor.port && current.token == self.descriptor.token
        });
        if owns_descriptor {
            let _ = fs::remove_file(&self.descriptor_path);
        }
    }
}

pub(crate) fn normalize_browser_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.map(|path| {
        let absolute = if path.is_relative() {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        } else {
            path
        };
        absolute.canonicalize().unwrap_or(absolute)
    })
}

pub(crate) fn start_or_forward(request: BrowserOpenRequest) -> io::Result<StartResult> {
    let descriptor_path = std::env::var_os(INSTANCE_FILE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(gitcomet_state::session::browser_instance_file_path)
        .ok_or_else(|| io::Error::other("no per-user state directory is available"))?;
    start_or_forward_at(&descriptor_path, request)
}

fn start_or_forward_at(
    descriptor_path: &Path,
    request: BrowserOpenRequest,
) -> io::Result<StartResult> {
    if let Some(descriptor) = read_descriptor(descriptor_path)
        && forward_request(&descriptor, &request).is_ok()
    {
        return Ok(StartResult::Forwarded);
    }

    // Binding a candidate and publishing its descriptor are separate
    // operations. Serialize that interval across processes so a contender
    // cannot time out on the bound port, choose another one, and become a
    // second primary while the first process is merely preempted in between.
    let claim = match acquire_browser_instance_claim(descriptor_path, &request)? {
        BrowserInstanceClaim::Acquired(claim) => claim,
        BrowserInstanceClaim::Forwarded => return Ok(StartResult::Forwarded),
    };

    let first_port = first_candidate_port(descriptor_path);
    let mut last_error = None;
    for offset in 0..PORT_CANDIDATES {
        let port = PORT_BASE + (first_port - PORT_BASE + offset) % PORT_SPAN;
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        match TcpListener::bind(address) {
            Ok(listener) => return start_primary(descriptor_path, listener, port, claim),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                last_error = Some(err);
                let deadline = Instant::now() + DESCRIPTOR_RETRY_WINDOW;
                loop {
                    if let Some(descriptor) = read_descriptor(descriptor_path)
                        && descriptor.port == port
                        && forward_request(&descriptor, &request).is_ok()
                    {
                        return Ok(StartResult::Forwarded);
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::other("no browser broker port is available")))
}

fn acquire_browser_instance_claim(
    descriptor_path: &Path,
    request: &BrowserOpenRequest,
) -> io::Result<BrowserInstanceClaim> {
    let Some(parent) = descriptor_path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "browser instance path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    let mut lock_name = descriptor_path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_path = PathBuf::from(lock_name);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(lock_path)?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(BrowserInstanceClaim::Acquired(file)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                // The primary retains the claim for its lifetime. Keep looking
                // for its descriptor while publication is in progress instead
                // of waiting on the lock and missing the moment forwarding
                // becomes possible.
                if let Some(descriptor) = read_descriptor(descriptor_path)
                    && forward_request(&descriptor, request).is_ok()
                {
                    return Ok(BrowserInstanceClaim::Forwarded);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn start_primary(
    descriptor_path: &Path,
    listener: TcpListener,
    port: u16,
    claim: fs::File,
) -> io::Result<StartResult> {
    listener.set_nonblocking(true)?;
    let descriptor = InstanceDescriptor {
        version: PROTOCOL_VERSION,
        port,
        token: Uuid::new_v4().simple().to_string(),
        pid: std::process::id(),
    };
    #[cfg(test)]
    wait_before_descriptor_publish_for_test(descriptor_path, port);
    write_descriptor(descriptor_path, &descriptor)?;

    let (requests_tx, requests_rx) = smol::channel::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server_descriptor = descriptor.clone();
    let server_thread = thread::Builder::new()
        .name("gitcomet-browser-instance".to_string())
        .spawn(move || server_loop(listener, server_descriptor, requests_tx, server_stop))?;

    Ok(StartResult::Primary(PrimaryBrowserInstance {
        requests: requests_rx,
        _server: BrowserInstanceServer {
            descriptor_path: descriptor_path.to_path_buf(),
            descriptor,
            stop,
            thread: Some(server_thread),
            _claim: claim,
        },
    }))
}

fn server_loop(
    listener: TcpListener,
    descriptor: InstanceDescriptor,
    requests: Sender<BrowserOpenRequest>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _address)) => {
                let _ = handle_connection(stream, &descriptor, &requests);
            }
            Err(err) if accept_error_is_retryable(&err) => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn accept_error_is_retryable(_error: &io::Error) -> bool {
    // `accept` failures describe one attempt, not the viability of the bound
    // listener. This includes connection aborts and temporary process/system
    // descriptor exhaustion. Retrying with the loop's backoff keeps the live
    // broker claim paired with a live accept loop.
    true
}

fn handle_connection(
    mut stream: TcpStream,
    descriptor: &InstanceDescriptor,
    requests: &Sender<BrowserOpenRequest>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut stream).take((MAX_WIRE_BYTES + 1) as u64);
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 || bytes > MAX_WIRE_BYTES || !line.ends_with('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid browser broker request length",
            ));
        }
    }

    let request: WireRequest = serde_json::from_str(&line)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if request.version != PROTOCOL_VERSION
        || descriptor.version != PROTOCOL_VERSION
        || request.token != descriptor.token
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "browser broker authentication failed",
        ));
    }
    let path = match request.path {
        Some(path) => Some(path.into_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "repository path encoding does not match this platform",
            )
        })?),
        None => None,
    };
    if requests
        .try_send(BrowserOpenRequest {
            path,
            target: request.target.into(),
        })
        .is_err()
    {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "browser request receiver is closed",
        ));
    }

    stream.write_all(b"ok\n")?;
    stream.flush()
}

fn forward_request(
    descriptor: &InstanceDescriptor,
    request: &BrowserOpenRequest,
) -> io::Result<()> {
    if descriptor.version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported browser broker protocol",
        ));
    }
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, descriptor.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let wire = WireRequest {
        version: PROTOCOL_VERSION,
        token: descriptor.token.clone(),
        path: request.path.as_deref().map(WirePath::from_path),
        target: request.target.into(),
    };
    serde_json::to_writer(&mut stream, &wire).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    if response == "ok\n" {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "browser broker rejected the request",
        ))
    }
}

fn read_descriptor(path: &Path) -> Option<InstanceDescriptor> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn write_descriptor(path: &Path, descriptor: &InstanceDescriptor) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "browser instance path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".browser-instance-{}-{}.tmp",
        std::process::id(),
        Uuid::new_v4().simple()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    serde_json::to_writer(&mut file, descriptor).map_err(io::Error::other)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(first_err) => {
            // Windows cannot atomically replace an existing file. The short
            // gap is tolerated because clients retry descriptor reads.
            let _ = fs::remove_file(path);
            fs::rename(&temporary, path).map_err(|_| first_err)
        }
    }
}

fn first_candidate_port(path: &Path) -> u16 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    PORT_BASE + (hash % u64::from(PORT_SPAN)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: PathBuf, target: BrowserOpenTarget) -> BrowserOpenRequest {
        BrowserOpenRequest {
            path: Some(path),
            target,
        }
    }

    #[test]
    fn review_regression_confirmed_broker_retries_transient_accept_errors() {
        assert!(accept_error_is_retryable(&io::Error::from(
            io::ErrorKind::WouldBlock
        )));
        assert!(
            accept_error_is_retryable(&io::Error::from(io::ErrorKind::ConnectionAborted)),
            "an aborted connection is local to one accept attempt and must not kill the broker"
        );
    }

    #[test]
    fn second_browser_process_forwards_to_the_primary_instance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor = dir.path().join("instance.json");
        let path = dir.path().join("repo");
        let primary = match start_or_forward_at(
            &descriptor,
            request(path.clone(), BrowserOpenTarget::ExistingWindow),
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::NewWindow)
            )
            .expect("forward request"),
            StartResult::Forwarded
        ));
        let received = smol::block_on(primary.requests().recv()).expect("forwarded request");
        assert_eq!(received.path.as_deref(), Some(path.as_path()));
        assert_eq!(received.target, BrowserOpenTarget::NewWindow);
    }

    #[cfg(unix)]
    #[test]
    fn forwarded_repository_paths_preserve_non_utf8_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor = dir.path().join("instance.json");
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"repo-\xff".to_vec()));
        let primary = match start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::ExistingWindow)
            )
            .expect("forward non-UTF-8 path"),
            StartResult::Forwarded
        ));
        let received = smol::block_on(primary.requests().recv()).expect("forwarded request");
        assert_eq!(
            received.path.expect("path").as_os_str().as_bytes(),
            path.as_os_str().as_bytes()
        );
    }

    #[cfg(windows)]
    #[test]
    fn forwarded_repository_paths_preserve_windows_wide_units() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor = dir.path().join("instance.json");
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'r' as u16,
            b'e' as u16,
            b'p' as u16,
            b'o' as u16,
            0xd800,
        ]));
        let primary = match start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::ExistingWindow)
            )
            .expect("forward Windows path"),
            StartResult::Forwarded
        ));
        let received = smol::block_on(primary.requests().recv()).expect("forwarded request");
        assert_eq!(
            received
                .path
                .expect("path")
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>(),
            path.as_os_str().encode_wide().collect::<Vec<_>>()
        );
    }

    #[test]
    fn stale_descriptor_is_replaced_by_a_new_primary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor_path = dir.path().join("instance.json");
        write_descriptor(
            &descriptor_path,
            &InstanceDescriptor {
                version: PROTOCOL_VERSION,
                port: 1,
                token: "stale".to_string(),
                pid: u32::MAX,
            },
        )
        .expect("stale descriptor");

        let primary = match start_or_forward_at(
            &descriptor_path,
            BrowserOpenRequest {
                path: None,
                target: BrowserOpenTarget::ExistingWindow,
            },
        )
        .expect("replace stale descriptor")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("stale descriptor unexpectedly forwarded"),
        };
        let current = read_descriptor(&descriptor_path).expect("current descriptor");
        assert_ne!(current.token, "stale");
        drop(primary);
        assert!(!descriptor_path.exists());
    }

    #[test]
    fn review_regression_followup_broker_claim_serializes_descriptor_publication() {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor_path = dir.path().join("instance.json");
        let first_port = first_candidate_port(&descriptor_path);
        let gate = Arc::new(DescriptorPublishGate::new());
        *descriptor_publish_gate_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some((descriptor_path.clone(), first_port, Arc::clone(&gate)));

        let (first_ready_tx, first_ready_rx) = std::sync::mpsc::channel();
        let (first_stop_tx, first_stop_rx) = std::sync::mpsc::channel();
        let first_descriptor = descriptor_path.clone();
        let first = thread::spawn(move || {
            let result = start_or_forward_at(
                &first_descriptor,
                request(PathBuf::from("first"), BrowserOpenTarget::ExistingWindow),
            )
            .expect("first broker start");
            let primary = match result {
                StartResult::Primary(primary) => primary,
                StartResult::Forwarded => panic!("first broker unexpectedly forwarded"),
            };
            first_ready_tx.send(()).expect("publish first readiness");
            let _ = first_stop_rx.recv();
            drop(primary);
        });
        gate.wait_until_entered();

        let (second_result_tx, second_result_rx) = std::sync::mpsc::channel();
        let (second_stop_tx, second_stop_rx) = std::sync::mpsc::channel();
        let second_descriptor = descriptor_path.clone();
        let second = thread::spawn(move || {
            let result = start_or_forward_at(
                &second_descriptor,
                request(PathBuf::from("second"), BrowserOpenTarget::ExistingWindow),
            )
            .expect("second broker start");
            match result {
                StartResult::Forwarded => {
                    second_result_tx.send(false).expect("publish forwarding");
                }
                StartResult::Primary(primary) => {
                    second_result_tx
                        .send(true)
                        .expect("publish duplicate ownership");
                    let _ = second_stop_rx.recv();
                    drop(primary);
                }
            }
        });

        // Give the second process enough time to exhaust the descriptor retry
        // window while the first owns the candidate but is paused before its
        // atomic descriptor rename.
        thread::sleep(DESCRIPTOR_RETRY_WINDOW + Duration::from_millis(100));
        gate.release();
        first_ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first primary should finish publishing");
        let second_became_primary = second_result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second broker should resolve after publication");

        let _ = second_stop_tx.send(());
        let _ = first_stop_tx.send(());
        second.join().expect("join second broker thread");
        first.join().expect("join first broker thread");
        *descriptor_publish_gate_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;

        assert!(
            !second_became_primary,
            "a delayed descriptor must not allow two browser primaries"
        );
    }
}
