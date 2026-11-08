#![cfg(unix)]

/// Exercise the real OS wrapper in a disposable child: changing the test
/// runner's hard limit would affect unrelated tests and could not be undone.
#[test]
fn startup_headroom_respects_a_child_hard_limit() {
    const CHILD: &str = "GITCOMET_RESOURCE_LIMIT_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let inherited = getrlimit(Resource::Nofile);
        let hard = inherited.maximum.map_or(512, |limit| limit.min(512));
        setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(hard.min(256)),
                maximum: Some(hard),
            },
        )
        .unwrap();
        gitcomet_base::platform::resource_limits::ensure_file_descriptor_headroom().unwrap();
        let actual = getrlimit(Resource::Nofile);
        assert_eq!(actual.current, Some(hard));
        assert_eq!(actual.maximum, Some(hard));
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "startup_headroom_respects_a_child_hard_limit",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "the child must actually execute its test"
    );
}
