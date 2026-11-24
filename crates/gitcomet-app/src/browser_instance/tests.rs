use super::*;

// Windows can reserve a whole candidate range. Find a usable range before
// testing broker behavior, and keep its listeners until the caller is ready.
fn reserve_candidate_ports(dir: &Path) -> (PathBuf, Vec<TcpListener>) {
    (0..128)
        .find_map(|index| {
            let path = dir.join(format!("instance-{index}.json"));
            let listeners = candidate_ports(&path)
                .map(|port| TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)))
                .collect::<io::Result<Vec<_>>>()
                .ok()?;
            Some((path, listeners))
        })
        .expect("find available browser instance ports")
}

// Starts the primary on a released reservation. A child spawned by a
// concurrent test copies the reservation listeners and keeps them open
// until exec, so the whole range can briefly still read as in use.
fn start_after_reservation(
    descriptor_path: &Path,
    request: BrowserOpenRequest,
) -> Result<StartResult, StartError> {
    retry_while_reserved(|| {
        start_or_forward_at(descriptor_path, request.clone(), CLAIM_WAIT_TIMEOUT)
    })
}

fn retry_while_reserved(
    mut start: impl FnMut() -> Result<StartResult, StartError>,
) -> Result<StartResult, StartError> {
    let deadline = Instant::now() + CLAIM_WAIT_TIMEOUT;
    loop {
        match start() {
            Err(StartError::Broker(error))
                if error.kind() == io::ErrorKind::AddrInUse && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

#[test]
fn an_early_probe_does_not_create_state_or_build_a_request_without_a_primary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("absent/instance.json");
    assert!(!forward_if_running_at(&path, || panic!("request built without a primary")).unwrap());
    assert!(!path.parent().unwrap().exists());

    let path = directory.path().join("stale.json");
    fs::write(&path, b"incomplete descriptor").unwrap();
    assert!(!forward_if_running_at(&path, || panic!("request built from stale data")).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"incomplete descriptor");
    assert!(!directory.path().join("stale.json.lock").exists());
}

#[test]
fn an_early_probe_ignores_incompatible_and_disconnected_descriptors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instance.json");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut descriptor = InstanceDescriptor {
        version: PROTOCOL_VERSION + 1,
        port: listener.local_addr().unwrap().port(),
        token: fixture_token(),
        pid: std::process::id(),
        product: this_product(),
    };
    for foreign_product in [false, true] {
        if foreign_product {
            descriptor.version = PROTOCOL_VERSION;
            descriptor.product = "dev.other.product".into();
        }
        write_descriptor(&path, &descriptor).unwrap();
        assert!(!forward_if_running_at(&path, || panic!("incompatible primary")).unwrap());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    descriptor.product = this_product();
    write_descriptor(&path, &descriptor).unwrap();
    drop(listener);
    assert!(!forward_if_running_at(&path, || panic!("disconnected primary")).unwrap());
    assert!(!directory.path().join("instance.json.lock").exists());
}

#[test]
fn an_early_probe_leaves_ownership_to_final_arbitration() {
    let directory = tempfile::tempdir().unwrap();
    let path = reserve_candidate_ports(directory.path()).0;
    assert!(!forward_if_running_at(&path, || panic!("no primary yet")).unwrap());
    let StartResult::Primary(mut primary) = start_after_reservation(
        &path,
        request("initial".into(), BrowserOpenTarget::ExistingWindow),
    )
    .unwrap() else {
        panic!("first launch must own the broker");
    };
    let published = fs::read(&path).unwrap();
    let receiver = primary.take_requests().unwrap();
    let fast = request("early".into(), BrowserOpenTarget::NewWindow);
    assert!(forward_if_running_at(&path, || fast.clone()).unwrap());
    assert_eq!(receiver.try_recv().unwrap(), fast);
    let raced = request("raced".into(), BrowserOpenTarget::ExistingWindow);
    assert!(matches!(
        start_or_forward_at(&path, raced.clone(), CLAIM_WAIT_TIMEOUT).unwrap(),
        StartResult::Forwarded
    ));
    assert_eq!(receiver.try_recv().unwrap(), raced);
    assert_eq!(fs::read(&path).unwrap(), published);
}

#[test]
fn a_lost_acknowledgement_never_retries_or_claims_a_new_primary() {
    for arbitrate in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("instance.json");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let descriptor = InstanceDescriptor {
            version: PROTOCOL_VERSION,
            port: listener.local_addr().unwrap().port(),
            token: fixture_token(),
            pid: std::process::id(),
            product: this_product(),
        };
        write_descriptor(&path, &descriptor).unwrap();
        let published = fs::read(&path).unwrap();
        let token = descriptor.token.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            identify_server(&mut stream, &token).unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            let wire: WireRequest = serde_json::from_str(&line).unwrap();
            assert_eq!(wire.token, token);
        });
        let request = request("accepted-once".into(), BrowserOpenTarget::NewWindow);
        let result = if arbitrate {
            start_or_forward_at(&path, request, CLAIM_WAIT_TIMEOUT)
                .map(|result| matches!(result, StartResult::Forwarded))
        } else {
            forward_if_running_at(&path, || request).map_err(StartError::Forward)
        };
        assert!(matches!(result, Err(StartError::Forward(_))), "{result:?}");
        server.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), published);
        assert!(!directory.path().join("instance.json.lock").exists());
    }
}

fn fixture_token() -> String {
    Uuid::new_v4().simple().to_string()
}

// Answers one connection per reply (None closes silently); returns the tokens.
fn fake_broker(
    token: String,
    replies: Vec<Option<&'static [u8]>>,
) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        replies
            .into_iter()
            .filter_map(|reply| {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
                identify_server(&mut stream, &token).unwrap();
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).unwrap() == 0 {
                    return None;
                }
                if let Some(reply) = reply {
                    stream.write_all(reply).unwrap();
                }
                Some(serde_json::from_str::<WireRequest>(&line).unwrap().token)
            })
            .collect()
    });
    (port, server)
}

fn fake_descriptor(port: u16, token: &str, pid: u32) -> InstanceDescriptor {
    InstanceDescriptor {
        version: PROTOCOL_VERSION,
        port,
        token: token.into(),
        pid,
        product: this_product(),
    }
}

#[test]
fn a_listener_must_identify_itself_before_receiving_a_repository_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("instance.json");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let descriptor = fake_descriptor(port, &fixture_token(), std::process::id());
    write_descriptor(&path, &descriptor).unwrap();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        stream.write_all(&[0; 32]).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut received = Vec::new();
        stream.read_to_end(&mut received).unwrap();
        received
    });
    let mut built = false;
    let delivered = forward_if_running_at(&path, || {
        built = true;
        request(
            "example-repository".into(),
            BrowserOpenTarget::ExistingWindow,
        )
    })
    .unwrap();
    let received = peer.join().unwrap();
    assert!(!delivered);
    assert!(!built);
    assert!(
        !received
            .windows(descriptor.token.len())
            .any(|w| w == descriptor.token.as_bytes())
    );
}

#[test]
fn a_shutting_down_primary_rejects_without_blocking_the_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let path = reserve_candidate_ports(dir.path()).0;
    let StartResult::Primary(mut primary) = start_after_reservation(
        &path,
        request("initial".into(), BrowserOpenTarget::ExistingWindow),
    )
    .unwrap() else {
        panic!("expected primary")
    };
    drop(primary.take_requests().unwrap());
    let next = request("next".into(), BrowserOpenTarget::ExistingWindow);
    assert!(!forward_if_running_at(&path, || next.clone()).unwrap());
    match start_or_forward_at(&path, next.clone(), Duration::from_millis(50)) {
        Err(StartError::Broker(error)) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
        Err(error) => panic!("a rejection must leave the claim to arbitration: {error:?}"),
        Ok(_) => panic!("the shutting-down primary still holds the claim"),
    }
    drop(primary);
    assert!(matches!(
        start_after_reservation(&path, next).unwrap(),
        StartResult::Primary(_)
    ));
}

#[test]
fn a_request_queued_behind_broker_shutdown_is_not_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let path = reserve_candidate_ports(dir.path()).0;
    let StartResult::Primary(mut primary) = start_after_reservation(
        &path,
        request("initial".into(), BrowserOpenTarget::ExistingWindow),
    )
    .unwrap() else {
        panic!("expected primary")
    };
    let receiver = primary.take_requests().unwrap();
    let descriptor = read_descriptor(&path).unwrap();
    // The broker serves one connection at a time and waits IO_TIMEOUT
    // on this silent one, so the next is still queued when it stops.
    let address = (Ipv4Addr::LOCALHOST, descriptor.port);
    let _stall = TcpStream::connect(address).unwrap();
    let mut queued = TcpStream::connect(address).unwrap();
    drop(primary);
    assert!(verify_server(&mut queued, &descriptor.token).is_err());
    assert!(receiver.try_recv().is_none());
}

#[test]
fn a_probed_launch_skips_the_repeat_probe_and_still_forwards() {
    let dir = tempfile::tempdir().unwrap();
    let path = reserve_candidate_ports(dir.path()).0;
    let stale = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    stale.set_nonblocking(true).unwrap();
    let stale_port = stale.local_addr().unwrap().port();
    write_descriptor(
        &path,
        &fake_descriptor(stale_port, &fixture_token(), u32::MAX),
    )
    .unwrap();
    let initial = request("initial".into(), BrowserOpenTarget::ExistingWindow);
    let StartResult::Primary(mut primary) =
        retry_while_reserved(|| claim_or_forward_at(&path, initial.clone(), CLAIM_WAIT_TIMEOUT))
            .unwrap()
    else {
        panic!("nothing holds the claim, so the launch must own the broker");
    };
    assert_eq!(
        stale.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the descriptor the caller already probed was contacted again"
    );
    // The live browser holds the claim, so the claim wait forwards to it.
    let receiver = primary.take_requests().unwrap();
    let next = request("next".into(), BrowserOpenTarget::NewWindow);
    assert!(matches!(
        claim_or_forward_at(&path, next.clone(), CLAIM_WAIT_TIMEOUT).unwrap(),
        StartResult::Forwarded
    ));
    assert_eq!(receiver.try_recv().unwrap(), next);
}

#[test]
fn a_stale_descriptor_on_a_foreign_listener_does_not_block_launch() {
    // A foreign reply proves the port is not ours; a silent close is
    // only trusted as undelivered once the descriptor's process is gone.
    for (reply, pid) in [
        (
            Some(&b"HTTP/1.1 400 Bad Request\r\n"[..]),
            std::process::id(),
        ),
        (None, u32::MAX),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = reserve_candidate_ports(dir.path()).0;
        let (port, server) = fake_broker(fixture_token(), vec![reply]);
        write_descriptor(&path, &fake_descriptor(port, &fixture_token(), pid)).unwrap();
        let launch = request("repo".into(), BrowserOpenTarget::ExistingWindow);
        let result = start_after_reservation(&path, launch);
        assert!(
            matches!(result, Ok(StartResult::Primary(_))),
            "{reply:?}: {:?}",
            result.err()
        );
        assert!(server.join().unwrap().is_empty());
    }
}

#[test]
fn a_claim_waiter_keeps_polling_after_a_rejection_until_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = reserve_candidate_ports(dir.path()).0;
    let launch = request("repo".into(), BrowserOpenTarget::NewWindow);
    let BrowserInstanceClaim::Acquired(claim) =
        acquire_browser_instance_claim(&path, &launch, CLAIM_WAIT_TIMEOUT).unwrap()
    else {
        panic!("claim unexpectedly forwarded");
    };
    // The new primary rejects the stale token until it publishes its own.
    let stale_token = fixture_token();
    let (stale_port, stale) = fake_broker(stale_token.clone(), vec![Some(b"rejected\n")]);
    write_descriptor(&path, &fake_descriptor(stale_port, &stale_token, u32::MAX)).unwrap();
    let waiter_path = path.clone();
    let waiter = thread::spawn(move || {
        start_or_forward_at(&waiter_path, launch, CLAIM_WAIT_TIMEOUT)
            .map(|result| matches!(result, StartResult::Forwarded))
    });
    assert_eq!(stale.join().unwrap(), [stale_token]);
    let fresh_token = fixture_token();
    let (port, published) = fake_broker(fresh_token.clone(), vec![Some(b"ok\n")]);
    write_descriptor(
        &path,
        &fake_descriptor(port, &fresh_token, std::process::id()),
    )
    .unwrap();
    assert!(matches!(waiter.join().unwrap(), Ok(true)));
    assert_eq!(published.join().unwrap(), [fresh_token]);
    drop(claim);
}

#[test]
fn pr530_broker_rejects_requests_after_ui_receiver_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let descriptor = reserve_candidate_ports(dir.path()).0;
    let StartResult::Primary(mut primary) = start_after_reservation(
        &descriptor,
        request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
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
    let descriptor = reserve_candidate_ports(dir.path()).0;
    let primary = start_after_reservation(
        &descriptor,
        request(PathBuf::from("initial"), BrowserOpenTarget::ExistingWindow),
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
    let descriptor = reserve_candidate_ports(dir.path()).0;
    let path = dir.path().join("repo");
    let mut primary = match start_after_reservation(
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
    let descriptor = reserve_candidate_ports(dir.path()).0;
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"repo-\xff".to_vec()));
    let mut primary = match start_after_reservation(
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
    let descriptor = reserve_candidate_ports(dir.path()).0;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[
        b'r' as u16,
        b'e' as u16,
        b'p' as u16,
        b'o' as u16,
        0xd800,
    ]));
    let mut primary = match start_after_reservation(
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
    let token = fixture_token();
    let foreign = InstanceDescriptor {
        version: PROTOCOL_VERSION,
        port,
        token: token.clone(),
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
        token,
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
    let descriptor_path = reserve_candidate_ports(dir.path()).0;
    let stale_token = fixture_token();
    write_descriptor(
        &descriptor_path,
        &InstanceDescriptor {
            version: PROTOCOL_VERSION,
            port: 1,
            token: stale_token.clone(),
            pid: u32::MAX,
            product: this_product(),
        },
    )
    .expect("stale descriptor");

    let primary = match start_after_reservation(
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
    assert_ne!(current.token, stale_token);
    drop(primary);
    assert!(!descriptor_path.exists());
}

#[test]
fn review_regression_followup_broker_claim_serializes_descriptor_publication() {
    let dir = tempfile::tempdir().expect("tempdir");
    let descriptor_path = reserve_candidate_ports(dir.path()).0;
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
    let descriptor_path = reserve_candidate_ports(dir.path()).0;
    let request = request(PathBuf::from("repo"), BrowserOpenTarget::NewWindow);
    let BrowserInstanceClaim::Acquired(claim) =
        acquire_browser_instance_claim(&descriptor_path, &request, CLAIM_WAIT_TIMEOUT)
            .expect("first broker claim")
    else {
        panic!("first broker unexpectedly forwarded");
    };

    // Separate file handles contend here: EWOULDBLOCK on Unix,
    // ERROR_LOCK_VIOLATION on Windows, both TryLockError::WouldBlock.
    let wait = Duration::from_millis(50);
    let started = Instant::now();
    let result = start_or_forward_at(&descriptor_path, request.clone(), wait);
    match result {
        Err(StartError::Broker(error)) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
        Err(error) => panic!("unexpected forwarding error: {error:?}"),
        Ok(_) => panic!("a held claim must not allow another primary"),
    }
    assert!(
        started.elapsed() >= wait,
        "contention must retry until the deadline"
    );
    assert!(!descriptor_path.exists());

    drop(claim);
    // The reclaim waits CLAIM_WAIT_TIMEOUT, not ZERO: a child spawned by a
    // concurrent test inherits the lock fd until exec, so the released
    // flock can briefly still read contended.
    let StartResult::Primary(primary) =
        start_after_reservation(&descriptor_path, request).expect("reclaim released lock")
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
    let descriptor_path = reserve_candidate_ports(dir.path()).0;
    let primary = match start_after_reservation(
        &descriptor_path,
        request(PathBuf::from("first"), BrowserOpenTarget::ExistingWindow),
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
        Err(StartError::Broker(error)) => {
            assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{case}")
        }
        Err(error) => panic!("{case}: unexpected forwarding error: {error:?}"),
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
        token: fixture_token(),
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
    let token = descriptor.token.clone();
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("connect");
        verify_server(&mut stream, &token).expect("verify the listening broker");
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
    let (descriptor_path, mut listeners) = reserve_candidate_ports(dir.path());
    listeners.truncate(4);
    let occupied: Vec<u16> = candidate_ports(&descriptor_path).take(4).collect();

    let started = Instant::now();
    let primary = match start_after_reservation(
        &descriptor_path,
        BrowserOpenRequest {
            path: None,
            target: BrowserOpenTarget::ExistingWindow,
        },
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
