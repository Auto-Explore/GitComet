use crate::test_git_env;
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};

fn git(repo: &Path, args: &[&str]) -> String {
    let mut command = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut command);
    let output = command
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run fixture git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("fixture git output")
        .trim()
        .to_owned()
}

fn repository(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).expect("create fixture repository");
    git(&path, &["init", "-b", "main"]);
    git(&path, &["config", "user.name", "Test"]);
    git(&path, &["config", "user.email", "test@example.com"]);
    git(&path, &["config", "commit.gpgsign", "false"]);
    path
}

fn commit(repo: &Path, contents: &str) -> String {
    fs::write(repo.join("file.txt"), contents).expect("write fixture file");
    git(repo, &["add", "file.txt"]);
    git(repo, &["commit", "-m", contents]);
    git(repo, &["rev-parse", "HEAD"])
}

fn remote_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        format!("file:///{normalized}")
    } else {
        normalized
    }
}

#[test]
fn background_fetch_leaves_skipped_remotes_alone() {
    let root = tempfile::tempdir().expect("fixture directory");
    let origin = repository(root.path(), "origin");
    let fork = repository(root.path(), "fork");
    commit(&origin, "origin first");
    commit(&fork, "fork first");
    let local = repository(root.path(), "local");
    git(&local, &["remote", "add", "origin", &remote_url(&origin)]);
    git(&local, &["remote", "add", "fork", &remote_url(&fork)]);
    git(&local, &["fetch", "--all"]);
    let origin_latest = commit(&origin, "origin second");
    let fork_before = git(&local, &["rev-parse", "fork/main"]);
    commit(&fork, "fork second");

    GixBackend
        .open(&local)
        .expect("open repository")
        .fetch_all_background_except(true, &["fork".to_string()], &CancellationToken::new())
        .expect("background fetch succeeds");

    assert_eq!(git(&local, &["rev-parse", "origin/main"]), origin_latest);
    assert_eq!(git(&local, &["rev-parse", "fork/main"]), fork_before);
}

#[test]
fn worktrees_share_a_fetch_signature_unless_their_own_config_changes_fetching() {
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    commit(&remote, "first");
    let local = repository(root.path(), "local");
    git(&local, &["remote", "add", "origin", &remote_url(&remote)]);
    git(&local, &["fetch", "origin"]);
    git(&local, &["reset", "--hard", "origin/main"]);
    let worktree = |name: &str| {
        let path = root.path().join(name);
        git(
            &local,
            &["worktree", "add", "-b", name, path.to_str().unwrap()],
        );
        path
    };
    let sparse = worktree("sparse");
    let narrowed = worktree("narrowed");
    git(&local, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &sparse,
        &["config", "--worktree", "core.sparseCheckout", "true"],
    );
    git(
        &narrowed,
        &[
            "config",
            "--worktree",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );
    let signature = |path: &Path| {
        GixBackend
            .open(path)
            .expect("open worktree")
            .fetch_config_signature()
            .expect("signature")
    };
    assert_eq!(
        signature(&local),
        signature(&sparse),
        "worktree config unrelated to fetching keeps one fetch"
    );
    assert_ne!(
        signature(&local),
        signature(&narrowed),
        "a worktree that fetches different refspecs fetches on its own"
    );
}

#[test]
fn worktrees_whose_relative_remote_url_names_other_repositories_fetch_apart() {
    let root = tempfile::tempdir().expect("fixture directory");
    for side in ["a", "b"] {
        let remote = repository(&root.path().join(side), "remote");
        commit(&remote, side);
    }
    let local = repository(&root.path().join("a"), "checkout");
    commit(&local, "base");
    git(&local, &["remote", "add", "origin", "../remote"]);
    let worktree = |path: PathBuf| {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        git(
            &local,
            &["worktree", "add", "-b", &name, path.to_str().unwrap()],
        );
        path
    };
    let beside = worktree(root.path().join("a").join("beside"));
    let elsewhere = worktree(root.path().join("b").join("elsewhere"));
    let signature = |path: &Path| {
        GixBackend
            .open(path)
            .expect("open worktree")
            .fetch_config_signature()
            .expect("signature")
    };
    assert_eq!(
        signature(&local),
        signature(&beside),
        "../remote is a/remote from both"
    );
    assert_ne!(
        signature(&local),
        signature(&elsewhere),
        "git fetches b/remote from b/elsewhere"
    );
}

#[test]
fn refs_fingerprint_moves_only_when_a_fetch_changes_tracking_refs_or_tags() {
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    commit(&remote, "first");
    git(&remote, &["branch", "doomed"]);
    let local = repository(root.path(), "local");
    git(&local, &["remote", "add", "origin", &remote_url(&remote)]);
    git(&local, &["fetch", "origin"]);
    let opened = GixBackend.open(&local).expect("open repository");
    let cancel = CancellationToken::new();
    let fingerprint = || opened.refs_fingerprint(&cancel).expect("fingerprint");
    let fetch = || {
        opened
            .fetch_all_background(true, &cancel)
            .expect("background fetch succeeds");
    };

    let settled = fingerprint();
    fetch();
    assert_eq!(fingerprint(), settled, "a no-op fetch moves nothing");

    commit(&remote, "second");
    fetch();
    let advanced = fingerprint();
    assert_ne!(advanced, settled, "a new remote commit");

    git(&remote, &["tag", "v1"]);
    fetch();
    let tagged = fingerprint();
    assert_ne!(tagged, advanced, "a new tag");

    git(&remote, &["branch", "-D", "doomed"]);
    fetch();
    assert_ne!(fingerprint(), tagged, "a pruned tracking branch");
}

#[test]
fn refs_fingerprint_moves_when_a_mirror_fetch_updates_its_branches() {
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    commit(&remote, "first");
    let mirror = root.path().join("mirror.git");
    git(
        root.path(),
        &[
            "clone",
            "--mirror",
            &remote_url(&remote),
            mirror.to_str().unwrap(),
        ],
    );
    let opened = GixBackend.open(&mirror).expect("open mirror");
    let cancel = CancellationToken::new();
    let fingerprint = || opened.refs_fingerprint(&cancel).expect("fingerprint");
    let settled = fingerprint();
    let latest = commit(&remote, "second");
    opened
        .fetch_all_background(true, &cancel)
        .expect("background fetch succeeds");
    // A mirror's refspec writes refs/heads, not refs/remotes.
    assert_eq!(git(&mirror, &["rev-parse", "refs/heads/main"]), latest);
    assert_ne!(fingerprint(), settled, "the fetch moved main");
}

#[test]
fn background_fetch_updates_tracking_refs_without_changing_checkout() {
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    let first = commit(&remote, "first");
    let local = repository(root.path(), "local");
    git(&local, &["remote", "add", "origin", &remote_url(&remote)]);
    git(&local, &["fetch", "origin"]);
    git(&local, &["reset", "--hard", "origin/main"]);
    fs::write(local.join("file.txt"), "staged").expect("write staged contents");
    git(&local, &["add", "file.txt"]);
    fs::write(local.join("file.txt"), "working").expect("write unstaged contents");
    let index = git(&local, &["write-tree"]);
    let latest = commit(&remote, "second");

    let opened = GixBackend.open(&local).expect("open repository");
    let output = opened
        .fetch_all_background(true, &CancellationToken::new())
        .expect("background fetch succeeds");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(git(&local, &["rev-parse", "origin/main"]), latest);
    assert_eq!(git(&local, &["rev-parse", "HEAD"]), first);
    assert_eq!(git(&local, &["write-tree"]), index);
    assert_eq!(
        fs::read_to_string(local.join("file.txt")).unwrap(),
        "working"
    );
}

#[test]
fn background_fetch_leaves_staged_manual_credentials_available() {
    use gitcomet_core::auth::{
        GitAuthKind, ScopedStagedGitAuth, StagedGitAuth, take_staged_git_auth,
    };

    let _guard = crate::auth_test_lock();
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    commit(&remote, "first");
    let local = repository(root.path(), "local");
    git(&local, &["remote", "add", "origin", &remote_url(&remote)]);
    let opened = GixBackend.open(&local).expect("open repository");
    let manual_auth = StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("fixture-user".into()),
        secret: "test-manual-fetch-secret".into(),
        challenge: None,
    };
    let _staged = ScopedStagedGitAuth::stage(manual_auth.clone());

    opened
        .fetch_all_background(false, &CancellationToken::new())
        .expect("background fetch succeeds");

    assert_eq!(take_staged_git_auth(), Some(manual_auth));
}

#[cfg(unix)]
mod transport {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    mod cancellation;

    struct HttpRemote {
        address: SocketAddr,
        stopped: Arc<AtomicBool>,
        authenticated_requests: Arc<AtomicUsize>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl HttpRemote {
        fn start(remote: &Path) -> Self {
            git(remote, &["update-server-info"]);
            let git_dir = remote.join(".git");
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture HTTP server");
            let address = listener.local_addr().expect("fixture server address");
            let stopped = Arc::new(AtomicBool::new(false));
            let stopping = Arc::clone(&stopped);
            let authenticated_requests = Arc::new(AtomicUsize::new(0));
            let requests = Arc::clone(&authenticated_requests);
            let worker = std::thread::spawn(move || {
                loop {
                    let (mut stream, _) = listener.accept().expect("accept fixture request");
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    Self::serve(&mut stream, &git_dir, &requests);
                }
            });
            Self {
                address,
                stopped,
                authenticated_requests,
                worker: Some(worker),
            }
        }

        fn serve(stream: &mut TcpStream, git_dir: &Path, requests: &AtomicUsize) {
            let mut reader = BufReader::new(&mut *stream);
            let mut request = String::new();
            reader
                .read_line(&mut request)
                .expect("read fixture request");
            let mut authenticated = false;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("read fixture header");
                if header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("authorization")
                {
                    authenticated =
                        value.trim() == "Basic Zml4dHVyZS11c2VyOnRlc3Qtc3RvcmVkLXNlY3JldA==";
                }
            }
            let (status, body) = if authenticated {
                requests.fetch_add(1, Ordering::Relaxed);
                let path = request.split_whitespace().nth(1).expect("fixture GET path");
                let path = path.split('?').next().expect("fixture URL path");
                let relative = path
                    .strip_prefix("/repo.git/")
                    .expect("fixture repository path");
                assert!(relative.split('/').all(|part| part != ".."));
                match fs::read(git_dir.join(relative)) {
                    Ok(body) => ("200 OK", body),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        ("404 Not Found", Vec::new())
                    }
                    Err(error) => panic!("read fixture response: {error}"),
                }
            } else {
                ("401 Unauthorized", Vec::new())
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nWWW-Authenticate: Basic realm=\"fixture\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            )
            .expect("write fixture response headers");
            stream.write_all(&body).expect("write fixture response");
        }

        fn url(&self) -> String {
            format!("http://{}/repo.git", self.address)
        }
    }

    impl Drop for HttpRemote {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            let _ = TcpStream::connect(self.address);
            self.worker
                .take()
                .expect("fixture worker")
                .join()
                .expect("fixture server stopped");
        }
    }

    fn shell_path(path: &Path) -> String {
        shell_quote(&path.to_string_lossy())
    }

    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    fn install_remote(local: &Path, script: &Path, contents: &str) {
        fs::write(script, contents).expect("write remote transport");
        git(
            local,
            &[
                "remote",
                "add",
                "origin",
                "ssh://fixture.invalid/repository",
            ],
        );
        git(
            local,
            &[
                "config",
                "core.sshCommand",
                &format!("sh {}", shell_path(script)),
            ],
        );
        git(local, &["config", "ssh.variant", "ssh"]);
    }

    #[test]
    fn background_fetch_uses_stored_credentials_without_interactive_helpers() {
        let _guard = crate::auth_test_lock();
        let root = tempfile::tempdir().expect("fixture directory");
        let remote = repository(root.path(), "remote");
        let latest = commit(&remote, "first");
        let local = repository(root.path(), "local");
        let server = HttpRemote::start(&remote);
        git(&local, &["remote", "add", "origin", &server.url()]);
        let popup = root.path().join("popup");
        let helper_called = root.path().join("helper-called");
        let stored = root.path().join("stored");
        let helper = format!(
            "!f() {{ if [ \"$GCM_INTERACTIVE\" != Never ] || [ \"$(git config --get credential.interactive)\" != false ]; then touch {}; exit 91; fi; touch {}; if [ -f {} ]; then printf 'username=fixture-user\\npassword=test-stored-secret\\n'; fi; }}; f",
            shell_path(&popup),
            shell_path(&helper_called),
            shell_path(&stored),
        );
        git(&local, &["config", "credential.helper", &helper]);
        git(&local, &["config", "credential.interactive", "true"]);
        let opened = GixBackend.open(&local).expect("open repository");

        let missing = opened.fetch_all_background(false, &CancellationToken::new());

        assert!(
            missing.is_err(),
            "missing credentials must refuse the fetch"
        );
        assert!(
            !popup.exists(),
            "the helper must not open an interactive flow"
        );
        assert!(
            helper_called.exists(),
            "stored credentials were queried: {missing:?}"
        );
        assert_eq!(server.authenticated_requests.load(Ordering::Acquire), 0);

        fs::write(&stored, "available").expect("install stored credential fixture");
        opened
            .fetch_all_background(false, &CancellationToken::new())
            .expect("stored credentials permit the background fetch");

        assert!(server.authenticated_requests.load(Ordering::Acquire) > 0);
        assert!(!popup.exists());
        assert_eq!(git(&local, &["rev-parse", "origin/main"]), latest);
    }

    #[test]
    fn remote_tag_lookup_never_opens_an_interactive_credential_flow() {
        let _guard = crate::auth_test_lock();
        let root = tempfile::tempdir().expect("fixture directory");
        let remote = repository(root.path(), "remote");
        commit(&remote, "first");
        let local = repository(root.path(), "local");
        let server = HttpRemote::start(&remote);
        git(&local, &["remote", "add", "origin", &server.url()]);
        let popup = root.path().join("popup");
        let helper_called = root.path().join("helper-called");
        let helper = format!(
            "!f() {{ touch {}; if [ \"$GCM_INTERACTIVE\" != Never ] || [ \"$(git config --get credential.interactive)\" != false ]; then touch {}; fi; }}; f",
            shell_path(&helper_called),
            shell_path(&popup),
        );
        git(&local, &["config", "credential.helper", &helper]);
        git(&local, &["config", "credential.interactive", "true"]);
        let opened = GixBackend.open(&local).expect("open repository");

        // Remote tags are best-effort: a refused remote leaves no tags.
        let tags = opened
            .list_remote_tags(&CancellationToken::new())
            .expect("tag lookup");

        assert!(tags.is_empty());
        assert!(helper_called.exists(), "stored credentials were queried");
        assert!(
            !popup.exists(),
            "the tag lookup must not open an interactive flow"
        );
        assert_eq!(server.authenticated_requests.load(Ordering::Acquire), 0);
    }

    #[test]
    fn background_fetch_reuses_only_verified_cached_ssh_passphrases() {
        use gitcomet_core::auth::{
            AuthChallenge, GitAuthKind, PassphraseValidation, StagedGitAuth,
            clear_session_passphrase, remember_verified_passphrase, validate_passphrase,
        };

        let _guard = crate::auth_test_lock();
        clear_session_passphrase();
        let root = tempfile::tempdir().expect("fixture directory");
        let remote = repository(root.path(), "remote");
        let latest = commit(&remote, "first");
        let local = repository(root.path(), "local");
        let key = root.path().join("identity");
        let keygen = std::process::Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-q",
                "-N",
                "test-cached-key-passphrase",
                "-f",
            ])
            .arg(&key)
            .output()
            .expect("generate fixture SSH key");
        assert!(keygen.status.success(), "fixture SSH key generation failed");
        let prompt = format!("Enter passphrase for key '{}':", key.display());
        let transferred = root.path().join("transferred");
        install_remote(
            &local,
            &root.path().join("transport.sh"),
            &format!(
                "set -eu\npassphrase=$(\"$SSH_ASKPASS\" {})\n[ \"$passphrase\" = test-cached-key-passphrase ]\ntouch {}\nexec git-upload-pack {}\n",
                shell_quote(&prompt),
                shell_path(&transferred),
                shell_path(&remote),
            ),
        );
        let opened = GixBackend.open(&local).expect("open repository");

        assert!(
            opened
                .fetch_all_background(false, &CancellationToken::new())
                .is_err()
        );
        assert!(!transferred.exists());

        let challenge = AuthChallenge::from_prompt(GitAuthKind::Passphrase, prompt);
        let auth = StagedGitAuth {
            kind: GitAuthKind::Passphrase,
            username: None,
            secret: "test-cached-key-passphrase".into(),
            challenge: Some(challenge.clone()),
        };
        assert_eq!(
            validate_passphrase(&challenge, &auth, &CancellationToken::new()),
            PassphraseValidation::Verified,
        );
        assert!(remember_verified_passphrase(&challenge, &auth));

        opened
            .fetch_all_background(false, &CancellationToken::new())
            .expect("verified cached passphrase permits the background fetch");

        assert!(transferred.exists());
        assert_eq!(git(&local, &["rev-parse", "origin/main"]), latest);
        clear_session_passphrase();
    }
}
