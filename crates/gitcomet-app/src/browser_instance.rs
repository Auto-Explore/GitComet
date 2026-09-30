use gitcomet_ui_gpui::{BrowserOpenRequest, BrowserOpenTarget};
use gitcomet_ui_gpui::{BrowserRequestReceiver, BrowserRequestSender, browser_request_channel};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROTOCOL_VERSION: u32 = 1;
const PORT_BASE: u16 = 42_000;
const PORT_SPAN: u16 = 20_000;
const PORT_CANDIDATES: u16 = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const CLAIM_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_WIRE_BYTES: usize = 1024 * 1024;
const INSTANCE_FILE_ENV: &str = "GITCOMET_BROWSER_INSTANCE_FILE";

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

pub(crate) fn start_or_forward(request: BrowserOpenRequest) -> io::Result<StartResult> {
    let descriptor_path = std::env::var_os(INSTANCE_FILE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(gitcomet_state::session::browser_instance_file_path)
        .ok_or_else(|| io::Error::other("no per-user state directory is available"))?;
    start_or_forward_at(&descriptor_path, request, CLAIM_WAIT_TIMEOUT)
}

fn start_or_forward_at(
    descriptor_path: &Path,
    request: BrowserOpenRequest,
    claim_wait: Duration,
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
            Ok(listener) => return start_primary(descriptor_path, listener, port, claim),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::other("no browser broker port is available")))
}

fn acquire_browser_instance_claim(
    descriptor_path: &Path,
    request: &BrowserOpenRequest,
    claim_wait: Duration,
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
    let deadline = Instant::now() + claim_wait;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(BrowserInstanceClaim::Acquired(file)),
            // Windows reports ERROR_LOCK_VIOLATION, not WouldBlock. Use fs2's
            // platform-specific error so simultaneous launches retry there too.
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                // The primary retains the claim for its lifetime. Keep looking
                // for its descriptor while publication is in progress instead
                // of waiting on the lock and missing the moment forwarding
                // becomes possible.
                if let Some(descriptor) = read_descriptor(descriptor_path)
                    && forward_request(&descriptor, request).is_ok()
                {
                    return Ok(BrowserInstanceClaim::Forwarded);
                }
                // A live primary may never accept (protocol mismatch, deleted
                // descriptor); give up so the caller launches independently.
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the running instance did not accept the request",
                    ));
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
            Ok((stream, _address)) => {
                if stop.load(Ordering::Acquire) {
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
    // Another product's broker at this path must never receive our requests.
    if descriptor.product != this_product() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the browser broker belongs to another product",
        ));
    }
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, descriptor.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let wire = WireRequest {
        version: PROTOCOL_VERSION,
        token: descriptor.token.clone(),
        product: descriptor.product.clone(),
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

fn candidate_ports(path: &Path) -> impl Iterator<Item = u16> {
    let first_port = first_candidate_port(path);
    (0..PORT_CANDIDATES)
        .map(move |offset| PORT_BASE + (first_port - PORT_BASE + offset) % PORT_SPAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr530_broker_rejects_requests_after_ui_receiver_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let descriptor = dir.path().join("instance.json");
        let StartResult::Primary(mut primary) = start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .unwrap() else {
            panic!("expected primary")
        };
        drop(primary.take_requests().unwrap());
        let descriptor = read_descriptor(&descriptor).unwrap();
        assert!(
            forward_request(
                &descriptor,
                &request(
                    PathBuf::from("after-quit"),
                    BrowserOpenTarget::ExistingWindow,
                )
            )
            .is_err(),
            "a request with no UI consumer must not be acknowledged"
        );
    }

    #[test]
    fn pr530_idle_broker_shuts_down_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let descriptor = dir.path().join("instance.json");
        let primary = start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .unwrap();
        let started = Instant::now();
        drop(primary);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!descriptor.exists());
    }

    #[test]
    fn pr530_browser_path_uses_shared_normalization() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            normalize_browser_path(Some(dir.path().to_path_buf())),
            Some(gitcomet_core::path_utils::canonicalize_or_original(
                dir.path().to_path_buf()
            ))
        );
        let relative = PathBuf::from("missing-browser-repository");
        assert_eq!(
            normalize_browser_path(Some(relative.clone())),
            Some(std::env::current_dir().unwrap().join(relative))
        );
    }

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
        let mut primary = match start_or_forward_at(
            &descriptor,
            request(path.clone(), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::NewWindow),
                CLAIM_WAIT_TIMEOUT,
            )
            .expect("forward request"),
            StartResult::Forwarded
        ));
        let received =
            smol::block_on(primary.take_requests().unwrap().recv()).expect("forwarded request");
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
        let mut primary = match start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::ExistingWindow),
                CLAIM_WAIT_TIMEOUT,
            )
            .expect("forward non-UTF-8 path"),
            StartResult::Forwarded
        ));
        let received =
            smol::block_on(primary.take_requests().unwrap().recv()).expect("forwarded request");
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
        let mut primary = match start_or_forward_at(
            &descriptor,
            request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };

        assert!(matches!(
            start_or_forward_at(
                &descriptor,
                request(path.clone(), BrowserOpenTarget::ExistingWindow),
                CLAIM_WAIT_TIMEOUT,
            )
            .expect("forward Windows path"),
            StartResult::Forwarded
        ));
        let received =
            smol::block_on(primary.take_requests().unwrap().recv()).expect("forwarded request");
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
    fn a_descriptor_of_another_product_is_never_forwarded_to() {
        let (requests, _receiver) = browser_request_channel();
        let listener =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind listener");
        let port = listener.local_addr().expect("address").port();
        let foreign = InstanceDescriptor {
            version: PROTOCOL_VERSION,
            port,
            token: "token".to_string(),
            pid: std::process::id(),
            product: "comet-pro".to_string(),
        };
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            handle_connection(stream, &foreign, &requests)
        });
        let ours = InstanceDescriptor {
            version: PROTOCOL_VERSION,
            port,
            token: "token".to_string(),
            pid: std::process::id(),
            product: this_product(),
        };
        // The client refuses a foreign descriptor before connecting...
        let foreign_view = InstanceDescriptor {
            product: "comet-pro".to_string(),
            ..ours.clone()
        };
        let request = BrowserOpenRequest {
            path: None,
            target: BrowserOpenTarget::NewWindow,
        };
        assert!(forward_request(&foreign_view, &request).is_err());
        // ...and the foreign server rejects a request that names our product.
        assert!(forward_request(&ours, &request).is_err());
        assert!(server.join().expect("server thread").is_err());
    }

    #[test]
    fn descriptors_without_a_product_belong_to_gitcomet() {
        let descriptor: InstanceDescriptor =
            serde_json::from_str(r#"{"version":1,"port":1,"token":"t","pid":1}"#).unwrap();
        assert_eq!(descriptor.product, upstream_product_id());
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
                product: this_product(),
            },
        )
        .expect("stale descriptor");

        let primary = match start_or_forward_at(
            &descriptor_path,
            BrowserOpenRequest {
                path: None,
                target: BrowserOpenTarget::ExistingWindow,
            },
            CLAIM_WAIT_TIMEOUT,
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
        let BrowserInstanceClaim::Acquired(claim) = acquire_browser_instance_claim(
            &descriptor_path,
            &request(PathBuf::from("first"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("first broker claim") else {
            panic!("first broker unexpectedly forwarded");
        };
        // Hold the actual lock and bound socket before publishing, just as a
        // primary preempted during startup would. No global test gate is needed.
        let listener =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind primary");
        let port = listener.local_addr().unwrap().port();

        let (second_result_tx, second_result_rx) = std::sync::mpsc::channel();
        let second_descriptor = descriptor_path.clone();
        let second = thread::spawn(move || {
            let _ = second_result_tx.send(start_or_forward_at(
                &second_descriptor,
                request(PathBuf::from("second"), BrowserOpenTarget::ExistingWindow),
                CLAIM_WAIT_TIMEOUT,
            ));
        });

        assert!(
            matches!(
                second_result_rx.recv_timeout(Duration::from_millis(100)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ),
            "lock contention must wait for publication, including on Windows"
        );
        assert!(!descriptor_path.exists());

        let StartResult::Primary(mut primary) =
            start_primary(&descriptor_path, listener, port, claim).expect("publish primary")
        else {
            panic!("publishing unexpectedly forwarded");
        };
        assert!(matches!(
            second_result_rx
                .recv_timeout(CLAIM_WAIT_TIMEOUT)
                .expect("second broker should resolve after publication")
                .expect("second broker should forward"),
            StartResult::Forwarded
        ));
        second.join().expect("join second broker thread");
        let received = primary
            .take_requests()
            .unwrap()
            .try_recv()
            .expect("forwarded request");
        assert_eq!(received.path, Some(PathBuf::from("second")));
        assert_eq!(received.target, BrowserOpenTarget::ExistingWindow);
    }

    #[test]
    fn contended_claim_waits_until_timeout_then_can_be_reclaimed_after_release() {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor_path = dir.path().join("instance.json");
        let request = request(PathBuf::from("repo"), BrowserOpenTarget::NewWindow);
        let BrowserInstanceClaim::Acquired(claim) =
            acquire_browser_instance_claim(&descriptor_path, &request, CLAIM_WAIT_TIMEOUT)
                .expect("first broker claim")
        else {
            panic!("first broker unexpectedly forwarded");
        };

        // Separate file handles produce the native fs2 contention error here:
        // EWOULDBLOCK on Unix, ERROR_LOCK_VIOLATION on Windows.
        let wait = Duration::from_millis(50);
        let started = Instant::now();
        let result = start_or_forward_at(&descriptor_path, request.clone(), wait);
        match result {
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
            Ok(_) => panic!("a held claim must not allow another primary"),
        }
        assert!(
            started.elapsed() >= wait,
            "contention must retry until the deadline"
        );
        assert!(!descriptor_path.exists());

        drop(claim);
        // Not ZERO: a child spawned by a concurrent test inherits the lock fd
        // until exec, so the released flock can briefly still read contended.
        let StartResult::Primary(primary) =
            start_or_forward_at(&descriptor_path, request, CLAIM_WAIT_TIMEOUT)
                .expect("reclaim released lock")
        else {
            panic!("no primary exists to forward to");
        };
        assert!(descriptor_path.exists());
        drop(primary);
        assert!(!descriptor_path.exists());
    }

    // Guards against a launch spinning forever when a live primary holds the
    // claim but cannot be forwarded to; it must fall back to launching alone.
    #[test]
    fn unreachable_claim_holder_falls_back_to_an_independent_launch() {
        assert_launch_falls_back("primary speaks another protocol version", |path| {
            let mut descriptor = read_descriptor(path).expect("descriptor");
            descriptor.version = PROTOCOL_VERSION + 1;
            write_descriptor(path, &descriptor).expect("rewrite descriptor");
        });
        assert_launch_falls_back("descriptor was cleaned up", |path| {
            fs::remove_file(path).expect("remove descriptor");
        });
    }

    fn assert_launch_falls_back(case: &str, break_forwarding: impl FnOnce(&Path)) {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor_path = dir.path().join("instance.json");
        let primary = match start_or_forward_at(
            &descriptor_path,
            request(PathBuf::from("first"), BrowserOpenTarget::ExistingWindow),
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("first process unexpectedly forwarded"),
        };
        break_forwarding(&descriptor_path);
        let published = read_descriptor(&descriptor_path).map(|current| current.token);

        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let second_descriptor = descriptor_path.clone();
        thread::spawn(move || {
            let _ = result_tx.send(start_or_forward_at(
                &second_descriptor,
                request(PathBuf::from("second"), BrowserOpenTarget::ExistingWindow),
                Duration::from_millis(100),
            ));
        });
        let result = result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("{case}: second launch is still waiting"));

        match result {
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{case}"),
            Ok(_) => panic!("{case}: second launch should fall back to running alone"),
        }
        assert_eq!(
            read_descriptor(&descriptor_path).map(|current| current.token),
            published,
            "{case}: the fallback must not publish a second primary"
        );
        drop(primary);
    }

    // Guards against WouldBlock when accepted sockets are non-blocking and
    // read timeouts would otherwise be ignored.
    #[test]
    fn connection_handler_waits_for_a_request_split_across_writes() {
        let listener =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let descriptor = InstanceDescriptor {
            version: PROTOCOL_VERSION,
            port: address.port(),
            token: "token".to_string(),
            pid: std::process::id(),
            product: this_product(),
        };
        let mut wire = serde_json::to_vec(&WireRequest {
            version: PROTOCOL_VERSION,
            token: descriptor.token.clone(),
            product: descriptor.product.clone(),
            path: None,
            target: WireTarget::NewWindow,
        })
        .expect("serialize request");
        wire.push(b'\n');
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            let (head, tail) = wire.split_at(wire.len() / 2);
            stream.write_all(head).expect("write first half");
            thread::sleep(Duration::from_millis(200));
            let _ = stream.write_all(tail);
            let mut response = String::new();
            let _ = BufReader::new(stream).read_line(&mut response);
            response
        });

        let (stream, _address) = listener.accept().expect("accept");
        // Read timeouts must also work if a caller supplies a non-blocking socket.
        stream.set_nonblocking(true).expect("non-blocking stream");
        let (requests_tx, requests_rx) = browser_request_channel();
        handle_connection(stream, &descriptor, &requests_tx).expect("serve split request");

        assert_eq!(client.join().expect("join client"), "ok\n");
        let received = requests_rx.try_recv().expect("forwarded request");
        assert_eq!(received.target, BrowserOpenTarget::NewWindow);
    }

    // Guards startup latency: holding the claim rules out another primary on
    // a candidate port, so occupied ports are skipped without polling.
    #[test]
    fn occupied_candidate_ports_are_skipped_without_waiting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let descriptor_path = dir.path().join("instance.json");
        let occupied: Vec<u16> = candidate_ports(&descriptor_path).take(4).collect();
        // A port something else already holds is occupied just the same.
        let _listeners: Vec<TcpListener> = occupied
            .iter()
            .filter_map(|&port| {
                TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).ok()
            })
            .collect();

        let started = Instant::now();
        let primary = match start_or_forward_at(
            &descriptor_path,
            BrowserOpenRequest {
                path: None,
                target: BrowserOpenTarget::ExistingWindow,
            },
            CLAIM_WAIT_TIMEOUT,
        )
        .expect("start primary")
        {
            StartResult::Primary(primary) => primary,
            StartResult::Forwarded => panic!("no primary exists to forward to"),
        };
        let elapsed = started.elapsed();

        let published = read_descriptor(&descriptor_path).expect("published descriptor");
        assert!(!occupied.contains(&published.port));
        assert!(
            elapsed < Duration::from_millis(500),
            "skipping {} occupied ports took {elapsed:?}",
            occupied.len()
        );
        drop(primary);
    }
}
