use super::*;

fn failure(message: &str) -> Option<Failure> {
    classify(&Error::new(ErrorKind::Backend(message.into())))
}

#[test]
fn failures_that_need_the_user_pause_and_host_failures_block_the_host() {
    for (message, expected) in [
        (
            "fatal: Authentication failed for 'https://github.com/o/r.git/'",
            Failure::SignIn,
        ),
        (
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            Failure::SignIn,
        ),
        (
            "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
            Failure::SignIn,
        ),
        (
            "remote: Repository not found.\nfatal: repository 'https://github.com/o/r.git/' not found",
            Failure::Missing,
        ),
        (
            "fatal: unable to access 'https://gitlab.com/o/r.git/': The requested URL returned error: 403",
            Failure::Missing,
        ),
        (
            "fatal: '/nowhere' does not appear to be a git repository",
            Failure::Missing,
        ),
        (
            "fatal: unable to access 'https://github.com/o/r.git/': The requested URL returned error: 429",
            Failure::Busy,
        ),
        (
            "error: RPC failed; HTTP 503 curl 22 The requested URL returned error: 503",
            Failure::Busy,
        ),
        (
            "fatal: unable to access 'https://github.com/o/r.git/': Could not resolve host: github.com",
            Failure::Network,
        ),
        (
            "ssh: connect to host github.com port 22: Connection timed out\nfatal: Could not read from remote repository.",
            Failure::Network,
        ),
        (
            "error: cannot lock ref 'refs/remotes/origin/main'",
            Failure::Other,
        ),
    ] {
        assert_eq!(failure(message), Some(expected), "{message}");
    }
    assert_eq!(classify(&Error::new(ErrorKind::Cancelled)), None);
    assert!(Failure::SignIn.pauses() && Failure::Missing.pauses());
    assert!(Failure::Network.blocks_host() && Failure::Busy.blocks_host());
    assert!(Failure::SignIn.blocks_host() && !Failure::Missing.blocks_host());
    assert!(!Failure::Other.pauses() && !Failure::Other.blocks_host());
}

#[test]
fn multi_remote_output_classifies_each_failed_remote_by_its_own_lines() {
    // `fetch.parallel` 1 and above: each remote's errors precede its name.
    for (sign_in, offline) in [
        (
            "error: could not fetch private",
            "error: Could not fetch offline",
        ),
        (
            "could not fetch 'private' (exit code: 128)",
            "could not fetch 'offline' (exit code: 128)",
        ),
    ] {
        let error = Error::new(ErrorKind::Backend(format!(
            "fatal: unable to get password from user\n{sign_in}\n\
             fatal: unable to access 'http://offline.example/r.git/': Could not resolve host: offline.example\n\
             {offline}\nerror: could not fetch 'private'"
        )));
        assert_eq!(
            remote_failures(&error),
            [
                ("private".to_owned(), Failure::SignIn),
                ("offline".to_owned(), Failure::Network),
            ]
        );
    }
    assert!(remote_failures(&Error::new(ErrorKind::Backend("fatal: offline".into()))).is_empty());
}

#[test]
fn remote_hosts_ignore_users_and_paths_and_local_repositories_have_none() {
    for (url, host) in [
        ("https://github.com/o/r.git", Some("github.com")),
        ("https://user:token@GitHub.com/o/r.git", Some("github.com")),
        (
            "ssh://git@gitlab.example:2222/o/r.git",
            Some("gitlab.example:2222"),
        ),
        ("git@github.com:o/r.git", Some("github.com")),
        ("github.com:o/r.git", Some("github.com")),
        ("file:///srv/git/r.git", None),
        ("/srv/git/r.git", None),
        ("../sibling", None),
        ("C:\\repos\\r.git", None),
        ("C:/repos/r.git", None),
        ("./a:b", None),
    ] {
        assert_eq!(remote_host(url).as_deref(), host, "{url}");
    }
}

#[test]
fn backoff_doubles_to_a_cap() {
    let minutes = |minutes: u64| Duration::from_secs(minutes * 60);
    let half_hour = AutoFetchInterval::new(30).unwrap();
    for (failures, delay) in [(0, 30), (1, 60), (2, 120), (3, 240), (9, 240)] {
        assert_eq!(retry_delay(half_hour, failures), minutes(delay));
    }
    let daily = AutoFetchInterval::new(1440).unwrap();
    assert_eq!(
        retry_delay(daily, 3),
        minutes(1440),
        "never shorter than the interval"
    );
    for (strikes, delay) in [(1, 5), (2, 10), (3, 20), (4, 40), (5, 60), (12, 60)] {
        assert_eq!(host_block(Failure::Network, strikes), Some(minutes(delay)));
    }
    for (strikes, delay) in [(1, 15), (2, 30), (3, 60), (4, 60)] {
        assert_eq!(host_block(Failure::Busy, strikes), Some(minutes(delay)));
    }
    assert_eq!(
        host_block(Failure::SignIn, 1),
        None,
        "sign-in waits for a manual fetch"
    );
}

#[test]
fn quiet_repositories_wait_up_to_four_intervals() {
    let half_hour = AutoFetchInterval::new(30).unwrap();
    for (quiet, minutes) in [(0, 30), (1, 60), (2, 120), (3, 120), (40, 120)] {
        assert_eq!(
            quiet_delay(half_hour, quiet),
            Duration::from_secs(minutes * 60)
        );
    }
}
