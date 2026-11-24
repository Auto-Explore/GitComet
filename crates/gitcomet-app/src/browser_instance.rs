use gitcomet_ui_gpui::{BrowserOpenRequest, BrowserOpenTarget};
use gitcomet_ui_gpui::{BrowserRequestReceiver, BrowserRequestSender, browser_request_channel};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROTOCOL_VERSION: u32 = 2;
const PORT_BASE: u16 = 42_000;
const PORT_SPAN: u16 = 20_000;
const PORT_CANDIDATES: u16 = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const CLAIM_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_WIRE_BYTES: usize = 1024 * 1024;
const INSTANCE_FILE_ENV: &str = "GITCOMET_BROWSER_INSTANCE_FILE";
const OK_REPLY: &[u8] = b"ok\n";
const REJECTED_REPLY: &[u8] = b"rejected\n";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InstanceDescriptor {
    version: u32,
    port: u16,
    token: String,
    pid: u32,
    /// App id of the product that owns the broker. Descriptors written before
    /// the field existed belong to GitComet.
    #[serde(default = "upstream_product_id")]
    product: String,
}

fn upstream_product_id() -> String {
    gitcomet_core::identity::ProductIdentity::gitcomet()
        .app_id()
        .to_string()
}

fn this_product() -> String {
    gitcomet_core::identity::current().app_id().to_string()
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
    #[serde(default = "upstream_product_id")]
    product: String,
    path: Option<WirePath>,
    target: WireTarget,
}

pub(crate) enum StartResult {
    Forwarded,
    Primary(PrimaryBrowserInstance),
}

#[derive(Debug)]
pub(crate) enum StartError {
    Broker(io::Error),
    Forward(io::Error),
}

impl From<io::Error> for StartError {
    fn from(error: io::Error) -> Self {
        Self::Broker(error)
    }
}

pub(crate) struct PrimaryBrowserInstance {
    requests: Option<BrowserRequestReceiver>,
    _server: BrowserInstanceServer,
}

impl PrimaryBrowserInstance {
    pub(crate) fn take_requests(&mut self) -> Option<BrowserRequestReceiver> {
        self.requests.take()
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
        // Wake the blocking accept without periodic idle polling. The server
        // checks `stop` before reading this connection.
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.descriptor.port);
        let _ = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT);
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
        gitcomet_core::path_utils::canonicalize_or_original(absolute)
    })
}

fn descriptor_path() -> io::Result<PathBuf> {
    std::env::var_os(INSTANCE_FILE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(gitcomet_state::session::browser_instance_file_path)
        .ok_or_else(|| io::Error::other("no per-user state directory is available"))
}

pub(crate) fn forward_if_running(request: impl FnOnce() -> BrowserOpenRequest) -> io::Result<bool> {
    let Ok(path) = descriptor_path() else {
        return Ok(false);
    };
    forward_if_running_at(&path, request)
}

// Ok(false): nothing was delivered. Err: the request may have been delivered.
fn forward_if_running_at(
    descriptor_path: &Path,
    request: impl FnOnce() -> BrowserOpenRequest,
) -> io::Result<bool> {
    let Some(descriptor) = read_descriptor(descriptor_path) else {
        return Ok(false);
    };
    let Ok(stream) = connect_to_descriptor(&descriptor) else {
        return Ok(false);
    };
    match send_request(stream, &descriptor, &request()) {
        Ok(delivered) => Ok(delivered),
        // No reply may mean a lost acknowledgement, so never resend unless
        // the descriptor's process is gone and the descriptor is stale.
        Err(_) if !crate::process_liveness::process_is_running(descriptor.pid) => Ok(false),
        Err(error) => Err(error),
    }
}

/// `probed`: this process has just found no browser to forward to with
/// [`forward_if_running`], so arbitration starts at the claim instead of
/// connecting to the same, possibly stale, descriptor again. A live browser
/// holds the claim for its whole life and a claim waiter forwards to it.
pub(crate) fn start_or_forward(
    request: BrowserOpenRequest,
    probed: bool,
) -> Result<StartResult, StartError> {
    let descriptor_path = descriptor_path()?;
    if probed {
        claim_or_forward_at(&descriptor_path, request, CLAIM_WAIT_TIMEOUT)
    } else {
        start_or_forward_at(&descriptor_path, request, CLAIM_WAIT_TIMEOUT)
    }
}

fn start_or_forward_at(
    descriptor_path: &Path,
    request: BrowserOpenRequest,
    claim_wait: Duration,
) -> Result<StartResult, StartError> {
    if forward_if_running_at(descriptor_path, || request.clone()).map_err(StartError::Forward)? {
        return Ok(StartResult::Forwarded);
    }
    claim_or_forward_at(descriptor_path, request, claim_wait)
}

fn claim_or_forward_at(
    descriptor_path: &Path,
    request: BrowserOpenRequest,
    claim_wait: Duration,
) -> Result<StartResult, StartError> {
    // Binding a candidate and publishing its descriptor are separate
    // operations. Serialize that interval across processes so a contender
    // cannot time out on the bound port, choose another one, and become a
    // second primary while the first process is merely preempted in between.
    let claim = match acquire_browser_instance_claim(descriptor_path, &request, claim_wait)? {
        BrowserInstanceClaim::Acquired(claim) => claim,
        BrowserInstanceClaim::Forwarded => return Ok(StartResult::Forwarded),
    };

    // With the claim held, no other primary can own a candidate port, so an
    // occupied one belongs to an unrelated service: move on without waiting.
    let mut last_error = None;
    for port in candidate_ports(descriptor_path) {
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        match TcpListener::bind(address) {
            Ok(listener) => {
                return start_primary(descriptor_path, listener, port, claim)
                    .map_err(StartError::Broker);
            }
            Err(err) => last_error = Some(err),
        }
    }

    Err(StartError::Broker(last_error.unwrap_or_else(|| {
        io::Error::other("no browser broker port is available")
    })))
}

fn acquire_browser_instance_claim(
    descriptor_path: &Path,
    request: &BrowserOpenRequest,
    claim_wait: Duration,
) -> Result<BrowserInstanceClaim, StartError> {
    let Some(parent) = descriptor_path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "browser instance path has no parent",
        )
        .into());
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
    let deadline = Instant::now() + claim_wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(BrowserInstanceClaim::Acquired(file)),
            // std maps Windows' ERROR_LOCK_VIOLATION to WouldBlock as well, so
            // simultaneous launches retry on every platform.
            Err(TryLockError::WouldBlock) => {
                // The primary retains the claim for its lifetime. Keep looking
                // for its descriptor while publication is in progress instead
                // of waiting on the lock and missing the moment forwarding
                // becomes possible.
                if forward_if_running_at(descriptor_path, || request.clone())
                    .map_err(StartError::Forward)?
                {
                    return Ok(BrowserInstanceClaim::Forwarded);
                }
                // A live primary may never accept (protocol mismatch, deleted
                // descriptor); give up so the caller launches independently.
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the running instance did not accept the request",
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
    }
}

fn start_primary(
    descriptor_path: &Path,
    listener: TcpListener,
    port: u16,
    claim: fs::File,
) -> io::Result<StartResult> {
    let descriptor = InstanceDescriptor {
        version: PROTOCOL_VERSION,
        port,
        token: Uuid::new_v4().simple().to_string(),
        pid: std::process::id(),
        product: this_product(),
    };
    write_descriptor(descriptor_path, &descriptor)?;

    let (requests_tx, requests_rx) = browser_request_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server_descriptor = descriptor.clone();
    let server_thread = thread::Builder::new()
        .name("browser-instance-broker".to_string())
        .spawn(move || server_loop(listener, server_descriptor, requests_tx, server_stop))?;

    Ok(StartResult::Primary(PrimaryBrowserInstance {
        requests: Some(requests_rx),
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
    requests: BrowserRequestSender,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _address)) => {
                if stop.load(Ordering::Acquire) {
                    // A launch racing shutdown must not mistake the closed
                    // connection for a lost acknowledgement.
                    let _ = stream.write_all(REJECTED_REPLY);
                    break;
                }
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
    requests: &BrowserRequestSender,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    identify_server(&mut stream, &descriptor.token)?;
    if let Err(error) = enqueue_request(&mut stream, descriptor, requests) {
        // Nothing was queued: say so, so the client may start a primary
        // instead of guessing whether the request was delivered.
        let _ = stream.write_all(REJECTED_REPLY);
        return Err(error);
    }
    stream.write_all(OK_REPLY)?;
    stream.flush()
}

fn enqueue_request(
    stream: &mut TcpStream,
    descriptor: &InstanceDescriptor,
    requests: &BrowserRequestSender,
) -> io::Result<()> {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut *stream).take((MAX_WIRE_BYTES + 1) as u64);
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
        || request.product != descriptor.product
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
    Ok(())
}

fn server_proof(token: &str, challenge: &[u8; 16]) -> Hmac<Sha256> {
    let mut mac = Hmac::new_from_slice(token.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(b"browser-instance-server-v2");
    mac.update(challenge);
    mac
}

fn identify_server(stream: &mut TcpStream, token: &str) -> io::Result<()> {
    let mut challenge = [0; 16];
    stream.read_exact(&mut challenge)?;
    stream.write_all(&server_proof(token, &challenge).finalize().into_bytes())
}

fn verify_server(stream: &mut TcpStream, token: &str) -> io::Result<()> {
    let challenge = Uuid::new_v4();
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.write_all(challenge.as_bytes())?;
    let mut proof = [0; 32];
    stream.read_exact(&mut proof)?;
    server_proof(token, challenge.as_bytes())
        .verify_slice(&proof)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unrecognized browser broker",
            )
        })
}

fn connect_to_descriptor(descriptor: &InstanceDescriptor) -> io::Result<TcpStream> {
    if descriptor.version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported browser broker protocol",
        ));
    }
    // Another product's broker at this path must never receive our requests.
    if descriptor.product != this_product() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the browser broker belongs to another product",
        ));
    }
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, descriptor.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT)?;
    verify_server(&mut stream, &descriptor.token)?;
    Ok(stream)
}

#[cfg(test)]
fn forward_request(
    descriptor: &InstanceDescriptor,
    request: &BrowserOpenRequest,
) -> io::Result<()> {
    if send_request(connect_to_descriptor(descriptor)?, descriptor, request)? {
        Ok(())
    } else {
        Err(io::Error::other("browser broker rejected the request"))
    }
}

fn send_request(
    mut stream: TcpStream,
    descriptor: &InstanceDescriptor,
    request: &BrowserOpenRequest,
) -> io::Result<bool> {
    let wire = WireRequest {
        version: PROTOCOL_VERSION,
        token: descriptor.token.clone(),
        product: descriptor.product.clone(),
        path: request.path.as_deref().map(WirePath::from_path),
        target: request.target.into(),
    };
    let mut line = serde_json::to_vec(&wire)?;
    line.push(b'\n');
    // The broker queues a request only once it has read the whole line, so
    // a failure before the line is out (a broker closing its listener as it
    // shuts down, say) delivered nothing.
    let sent = stream.write_all(&line).and_then(|()| stream.flush());
    if sent.is_err() {
        return Ok(false);
    }

    let mut response = Vec::new();
    let reply_limit = OK_REPLY.len().max(REJECTED_REPLY.len()) as u64;
    match BufReader::new(stream)
        .take(reply_limit)
        .read_until(b'\n', &mut response)
    {
        Ok(_) => {}
        // A reset comes from a peer that closed with the request unread, or
        // whose process ended: nothing will act on the request either way.
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => return Ok(false),
        Err(error) => return Err(error),
    }
    if response == OK_REPLY {
        Ok(true)
    } else if OK_REPLY.starts_with(&response) {
        // Silence or a cut-off "ok": the request may have been queued.
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "browser broker did not acknowledge the request",
        ))
    } else {
        // A rejection, or a reply our broker never sends (another service).
        Ok(false)
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

fn candidate_ports(path: &Path) -> impl Iterator<Item = u16> {
    let first_port = first_candidate_port(path);
    (0..PORT_CANDIDATES)
        .map(move |offset| PORT_BASE + (first_port - PORT_BASE + offset) % PORT_SPAN)
}

#[cfg(test)]
mod tests;
