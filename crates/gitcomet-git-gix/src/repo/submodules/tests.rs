use gitcomet_core::remote_url::{RemoteProtocol, RemoteUrlPolicy, validate_remote_url_with_policy};

use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::CancellationToken;
use std::cell::Cell;
use std::error::Error as _;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

#[test]
fn submodule_cleanup_failure_keeps_original_git_details_and_both_causes() {
    let failure = GitFailure::new(
        "git submodule add",
        GitFailureId::CommandFailed,
        Some(128),
        vec![0xff, b'\n'],
        vec![0xfe, b'\n'],
        Some("clone failed".into()),
    );
    for kind in [
        ErrorKind::Backend("clone failed".into()),
        ErrorKind::Git(failure.clone()),
    ] {
        let original = Error::new(kind).with_source(std::io::Error::from_raw_os_error(5));
        let cleanup = Error::new(ErrorKind::Backend("cleanup failed".into()))
            .with_source(std::io::Error::from_raw_os_error(28));
        let workdir = Path::new("submodule-workdir");
        let note = "Cleanup after failed submodule add also failed: cleanup failed";
        let error = append_failed_submodule_add_note(original, note, cleanup, workdir);
        match error.kind() {
            ErrorKind::Backend(message) => {
                assert_eq!(message, &format!("clone failed\n\n{note}"));
            }
            ErrorKind::Git(actual) => {
                assert_eq!(actual.command(), "git submodule add");
                assert_eq!(actual.id(), GitFailureId::CommandFailed);
                assert_eq!(actual.exit_code(), Some(128));
                assert_eq!(actual.stdout(), &[0xff, b'\n']);
                assert_eq!(actual.stderr(), &[0xfe, b'\n']);
                assert_eq!(
                    actual.detail(),
                    Some(format!("clone failed\n\n{note}").as_str())
                );
            }
            other => panic!("classification changed: {other:?}"),
        }
        assert_eq!(error.context().unwrap().path(), Some(workdir));
        let cause = error
            .source()
            .unwrap()
            .downcast_ref::<SubmoduleCleanupError>()
            .unwrap();
        for (error, expected) in [(&cause.original, 5), (&cause.cleanup, 28)] {
            assert_eq!(
                error
                    .source()
                    .unwrap()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .raw_os_error(),
                Some(expected)
            );
        }
        assert!(std::ptr::addr_eq(cause.source().unwrap(), &cause.original));
    }
}

/// One unreadable submodule costs its own subtree and nothing else.
#[test]
fn only_cancellation_stops_a_nested_listing_instead_of_pruning_it() {
    for kind in [
        ErrorKind::Backend("gix index: decode failed".to_string()),
        ErrorKind::Unsupported("path is not valid UTF-8"),
        ErrorKind::NotARepository,
        ErrorKind::Io(std::io::ErrorKind::PermissionDenied),
        ErrorKind::Git(GitFailure::new(
            "git submodule status",
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            Vec::new(),
            None,
        )),
    ] {
        assert!(
            nested_failure_prunes_subtree(&kind),
            "{kind:?} must cost only this subtree, not the whole listing"
        );
    }
    assert!(!nested_failure_prunes_subtree(&ErrorKind::Cancelled));
}

#[test]
fn configured_submodule_urls_survive_validation() {
    // The URL is validated after gix re-serializes it, so every shape a
    // real .gitmodules carries has to come back through unchanged enough.
    let policy = RemoteUrlPolicy::default().with_allowed(RemoteProtocol::Http, true);
    for source in [
        "../sibling.git",
        "./nested.git",
        "sub/other.git",
        "/srv/git/repo.git",
        "https://example.com/org/repo.git",
        "http://git.internal/org/repo.git",
        "ssh://git@example.com/org/repo.git",
        "git@example.com:org/repo.git",
        "file:///srv/git/repo.git",
    ] {
        let url = gix::url::parse(source.as_bytes().as_bstr()).expect(source);
        let rendered = bytes_to_text_preserving_utf8(url.to_bstring().as_ref());
        assert!(
            validate_remote_url_with_policy(&rendered, policy).is_ok(),
            "{source} rendered as {rendered}"
        );
    }
}

/// A submodule whose own nested submodule pointer `vendor/x` moves in
/// two commits and once more in the index. Returns the two commits.
#[test]
fn submodule_add_args_separate_positionals_from_options() {
    let mut cmd = Command::new("git");
    let label = push_submodule_add_args(
        &mut cmd,
        "--reference=/tmp/evil",
        Path::new("sub"),
        Some("main"),
        Some("lib/sub"),
        true,
    );
    let args: Vec<_> = cmd.get_args().collect();
    assert_eq!(
        cmd.get_envs().collect::<Vec<_>>(),
        [(OsStr::new("GIT_LITERAL_PATHSPECS"), Some(OsStr::new("1")))]
    );
    assert_eq!(
        args,
        [
            OsStr::new("submodule"),
            OsStr::new("add"),
            OsStr::new("--branch"),
            OsStr::new("main"),
            OsStr::new("--force"),
            OsStr::new("--name"),
            OsStr::new("lib/sub"),
            OsStr::new("--"),
            OsStr::new("--reference=/tmp/evil"),
            OsStr::new("sub"),
        ]
    );
    assert_eq!(
        label,
        "git submodule add --branch main --force --name lib/sub --reference=/tmp/evil sub"
    );
}

#[test]
fn submodule_add_label_masks_credentials_but_argv_keeps_the_url() {
    let mut cmd = Command::new("git");
    let url = "https://user:s3cret@example.com/lib.git";
    let label = push_submodule_add_args(&mut cmd, url, Path::new("lib"), None, None, false);
    assert_eq!(
        label,
        "git submodule add https://user:***@example.com/lib.git lib"
    );
    assert!(cmd.get_args().any(|arg| arg == OsStr::new(url)));
}

#[test]
fn submodule_git_dir_name_validation_accepts_nested_relative_names() {
    for name in ["sub", "group/sub", "./sub", "a.b/c-d"] {
        validate_submodule_git_dir_name(Path::new(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn submodule_git_dir_name_validation_rejects_names_that_escape_modules_dir() {
    for name in ["", ".", "..", "../victim", "sub/../../victim", "sub/.."] {
        let err = validate_submodule_git_dir_name(Path::new(name))
            .expect_err(&format!("{name:?} must be rejected"));
        assert!(
            matches!(err.kind(), ErrorKind::Backend(_)),
            "{name:?}: {err}"
        );
    }
    let absolute = std::env::temp_dir().join("gitcomet-victim");
    validate_submodule_git_dir_name(&absolute).expect_err("absolute names must be rejected");
}

#[test]
fn remove_submodule_git_dir_refuses_parent_dir_names() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let git_dir = tmp.path().join(".git");
    let victim = tmp.path().join("victim");
    std::fs::create_dir_all(git_dir.join("modules")).expect("modules dir");
    std::fs::create_dir_all(&victim).expect("victim dir");
    std::fs::write(victim.join("keep"), b"keep").expect("victim file");

    let err = remove_submodule_git_dir(&git_dir, Path::new("../../victim"))
        .expect_err("a name with '..' must not be joined onto .git/modules");
    assert!(err.to_string().contains("../../victim"), "{err}");
    assert!(victim.join("keep").exists(), "victim directory was deleted");
}

#[test]
fn remove_submodule_git_dir_refuses_absolute_names() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let git_dir = tmp.path().join(".git");
    std::fs::create_dir_all(git_dir.join("modules")).expect("modules dir");
    let victim = tempfile::tempdir().expect("victim tempdir");
    std::fs::write(victim.path().join("keep"), b"keep").expect("victim file");

    let err = remove_submodule_git_dir(&git_dir, victim.path())
        .expect_err("an absolute name must not replace the .git/modules root");
    assert!(matches!(err.kind(), ErrorKind::Backend(_)), "{err}");
    assert!(
        victim.path().join("keep").exists(),
        "victim directory was deleted"
    );
}

#[test]
fn remove_submodule_git_dir_removes_nested_module_and_prunes_empty_parents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let git_dir = tmp.path().join(".git");
    let modules = git_dir.join("modules");
    let module_dir = modules.join("group").join("sub");
    std::fs::create_dir_all(&module_dir).expect("module dir");
    std::fs::write(module_dir.join("HEAD"), b"ref: refs/heads/main\n").expect("module file");

    remove_submodule_git_dir(&git_dir, Path::new("group/sub")).expect("remove module dir");

    assert!(!module_dir.exists());
    assert!(
        !modules.join("group").exists(),
        "empty parent should be pruned"
    );
    assert!(modules.exists(), "modules root must survive");
}

#[test]
fn resolve_submodule_logical_name_rejects_names_that_escape_modules_dir() {
    for name in ["../../escape", "/tmp/gitcomet-victim"] {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_test_repo(tmp.path());
        std::fs::write(
            tmp.path().join(".gitmodules"),
            format!(
                "[submodule \"{name}\"]\n\tpath = sub\n\turl = https://example.invalid/sub.git\n"
            ),
        )
        .expect("write .gitmodules");

        let repo = open_repo(tmp.path()).repo();
        let err = resolve_submodule_logical_name(&repo, Path::new("sub"))
            .expect_err(&format!("name {name:?} must be rejected"));
        assert!(err.to_string().contains(name), "{name:?}: {err}");
    }
}

#[test]
fn resolve_submodule_logical_name_returns_safe_names() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    std::fs::write(
        tmp.path().join(".gitmodules"),
        "[submodule \"group/sub\"]\n\tpath = sub\n\turl = https://example.invalid/sub.git\n",
    )
    .expect("write .gitmodules");

    let repo = open_repo(tmp.path()).repo();
    let name = resolve_submodule_logical_name(&repo, Path::new("sub")).expect("resolve name");
    assert_eq!(name.as_deref(), Some(Path::new("group/sub")));
}

/// A repository at `path` with one commit, its Git directory among the
/// parent's modules as `git submodule` leaves it, or inside the checkout.
fn commit_child(parent: &Path, path: &str, absorbed: bool) -> String {
    let child = parent.join(path);
    std::fs::create_dir_all(&child).unwrap();
    if absorbed {
        let git_dir = gix::open(parent).unwrap().git_dir().to_owned();
        let modules = git_dir.join("modules").join(path);
        std::fs::create_dir_all(modules.parent().unwrap()).unwrap();
        let separate = format!("--separate-git-dir={}", modules.display());
        git_success(&child, &["init", "-q", &separate]);
        for (key, value) in [
            ("user.name", "t"),
            ("user.email", "t@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            git_success(&child, &["config", key, value]);
        }
    } else {
        init_test_repo(&child);
    }
    git_success(&child, &["commit", "-q", "--allow-empty", "-m", "child"]);
    gix::open(&child).unwrap().head_id().unwrap().to_string()
}

fn add_gitlink(repo: &Path, path: &str, id: &str) {
    let entry = format!("160000,{id},{path}");
    git_success(repo, &["update-index", "--add", "--cacheinfo", &entry]);
}

/// Gitlinks at `paths`, in one Git call.
fn add_gitlinks(repo: &Path, paths: impl Iterator<Item = String>, id: &str) {
    use std::io::Write as _;
    let mut child = crate::test_support::git_command()
        .arg("-C")
        .arg(repo)
        .args(["update-index", "--index-info"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for path in paths {
        writeln!(stdin, "160000 {id}\t{path}").unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

/// Threads that read `.gitmodules` after it changed list nothing of
/// theirs: positions taken from the earlier read would name other
/// submodules, or none.
#[test]
fn mappings_changed_under_the_listing_threads_are_listed_on_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    let path = |number: usize| format!("mods/m{number:02}");
    let modules = |numbers: &mut dyn Iterator<Item = usize>| -> String {
        numbers
            .map(|number| {
                let path = path(number);
                format!("[submodule \"{path}\"]\n\tpath = {path}\n\turl = ./{path}\n")
            })
            .collect()
    };
    for number in 0..40 {
        std::fs::create_dir_all(root.join(path(number))).unwrap();
    }
    add_gitlinks(root, (0..40).map(path), &"1".repeat(40));
    std::fs::write(root.join(".gitmodules"), modules(&mut (0..40))).unwrap();

    let repo = gix::open(root).unwrap();
    let cancellation = CancellationToken::new();
    let list = |workers: usize| {
        let gitlinks = collect_gitlinks(&repo, &cancellation).unwrap();
        list_submodules_in(&repo, gitlinks, workers, &cancellation).unwrap()
    };
    // Every other mapping goes, and the file looks newer, as a checkout
    // that removes submodules leaves it.
    let gitmodules = root.join(".gitmodules");
    let rewrite = {
        let gitmodules = gitmodules.clone();
        let fewer = modules(&mut (0..40).step_by(2));
        move || {
            let modified = std::fs::metadata(&gitmodules).unwrap().modified().unwrap();
            std::fs::write(&gitmodules, fewer).unwrap();
            let later = modified + std::time::Duration::from_secs(5);
            std::fs::File::options()
                .write(true)
                .open(&gitmodules)
                .unwrap()
                .set_modified(later)
                .unwrap();
        }
    };
    BEFORE_LISTERS.set(Some(Box::new(rewrite)));
    let listed = list(2);
    assert!(BEFORE_LISTERS.take().is_none(), "the threads started");
    assert_eq!(listed, list(1));
    let mapped = listed
        .iter()
        .filter(|submodule| submodule.status != SubmoduleStatus::MissingMapping)
        .count();
    assert_eq!(mapped, 20);
}

/// Recorded commits come from the index as it is on disk, also after Git
/// rewrote it within one timestamp tick.
#[test]
fn listed_submodules_follow_an_index_rewritten_without_a_newer_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let modules = "[submodule \"sub\"]\n\tpath = sub\n\turl = ./sub\n";
    std::fs::write(root.join(".gitmodules"), modules).unwrap();
    git_success(root, &["add", ".gitmodules"]);
    add_gitlink(root, "sub", &"1".repeat(40));
    let repo = open_repo(root);
    let recorded = || {
        repo.list_submodules_impl()
            .unwrap()
            .into_iter()
            .map(|submodule| submodule.recorded_head.as_ref().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(recorded(), ["1".repeat(40)]);

    let index = root.join(".git/index");
    let modified = std::fs::metadata(&index).unwrap().modified().unwrap();
    add_gitlink(root, "sub", &"2".repeat(40));
    std::fs::File::options()
        .write(true)
        .open(&index)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(recorded(), ["2".repeat(40)]);
}

/// Enough submodules to share out, one of each kind in every four.
#[test]
fn many_submodules_listed_on_several_threads_come_out_as_on_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    let mut modules = String::new();
    let mut moved = Vec::new();
    for number in 0..40 {
        let path = format!("mods/m{number:02}");
        let configure = |modules: &mut String| {
            modules.push_str(&format!(
                "[submodule \"{path}\"]\n\tpath = {path}\n\turl = ./{path}\n"
            ));
        };
        match number % 4 {
            // A gitlink `.gitmodules` does not mention, with a
            // repository checked out and a gitlink of its own.
            0 => {
                let id = commit_child(root, &path, false);
                add_gitlink(root, &path, &id);
                add_gitlink(&root.join(&path), "inner", &id);
            }
            1 => {
                let id = commit_child(root, &path, true);
                add_gitlink(root, &path, &id);
                configure(&mut modules);
            }
            // Checked out at another commit than the recorded one, and
            // every other one with a submodule of its own.
            2 => {
                let id = commit_child(root, &path, true);
                add_gitlink(root, &path, &id);
                configure(&mut modules);
                let child = root.join(&path);
                if number % 8 == 2 {
                    let inner = commit_child(&child, "inner", true);
                    add_gitlink(&child, "inner", &inner);
                    let inner = "[submodule \"inner\"]\n\tpath = inner\n\turl = ./inner\n";
                    std::fs::write(child.join(".gitmodules"), inner).unwrap();
                    git_success(&child, &["add", ".gitmodules"]);
                }
                git_success(&child, &["commit", "-q", "--allow-empty", "-m", "moved"]);
                moved.push(path.clone());
            }
            _ => {
                let id = "1".repeat(40);
                add_gitlink(root, &path, &id);
                std::fs::create_dir_all(root.join(&path)).unwrap();
                configure(&mut modules);
            }
        }
    }
    std::fs::write(root.join(".gitmodules"), modules).unwrap();
    git_success(root, &["add", ".gitmodules"]);
    // Enough plain unmapped gitlinks besides for eight threads.
    let plain = (0..100).map(|number| format!("plain/p{number:03}"));
    add_gitlinks(root, plain, &"2".repeat(40));
    git_success(root, &["commit", "-q", "-m", "submodules"]);

    let repo = gix::open(root).unwrap();
    let cancellation = CancellationToken::new();
    let list = |workers: usize| {
        let gitlinks = collect_gitlinks(&repo, &cancellation).unwrap();
        assert!(gitlinks.len() >= workers * SUBMODULE_LISTERS_MIN_SUBMODULES);
        list_submodules_in(&repo, gitlinks, workers, &cancellation).unwrap()
    };
    let alone = list(1);
    let count = |status: SubmoduleStatus| {
        alone
            .iter()
            .filter(|submodule| submodule.status == status)
            .count()
    };
    // Ten of each kind at the top and the plain ones; below them, a
    // gitlink in each unmapped one and a submodule in five moved ones.
    assert_eq!(count(SubmoduleStatus::MissingMapping), 120);
    assert_eq!(count(SubmoduleStatus::UpToDate), 15);
    assert_eq!(count(SubmoduleStatus::HeadMismatch), 10);
    assert_eq!(count(SubmoduleStatus::NotInitialized), 10);
    assert_eq!(alone.len(), 155);
    assert!(alone.is_sorted_by(|a, b| a.path < b.path));
    for workers in [2, 8] {
        assert_eq!(list(workers), alone, "{workers} workers");
    }
}

#[test]
fn cancelled_recursive_submodule_listing_stops_early() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let repo = open_repo(tmp.path());
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = repo
        .list_submodules_cancellable_impl(&cancellation)
        .expect_err("cancelled submodule listing should fail");
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
}

#[test]
fn allow_file_submodule_transport_uses_git_config_not_protocol_allowlist() {
    let mut cmd = Command::new("git");

    allow_file_submodule_transport(&mut cmd);

    let args = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(args, ["-c", "protocol.file.allow=always"]);
    assert!(
        !cmd.get_envs()
            .any(|(key, _)| key == OsStr::new("GIT_ALLOW_PROTOCOL"))
    );
}

fn git_config_failure(stderr: &str) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        "git config --global gitcomet.submodule.allowfiletransport-example true",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        stderr.as_bytes().to_vec(),
        None,
    )))
}

#[test]
fn git_config_contention_detection_handles_windows_access_denied() {
    assert!(is_git_config_contention_error(&git_config_failure(
        "error: could not lock config file .gitconfig: File exists"
    )));
    let windows_access_denied = "warning: unable to access 'C:\\Temp\\global.gitconfig': Permission denied\n\
             fatal: unknown error occurred while reading the configuration files";
    assert!(is_git_config_contention_error(&git_config_failure(
        windows_access_denied
    )));
    assert!(is_git_config_contention_error(&Error::new(
        ErrorKind::Backend(format!(
            "git config --global --type=bool --get key failed: {windows_access_denied}"
        ))
    )));
    assert!(!is_git_config_contention_error(&git_config_failure(
        "fatal: could not read Username for 'https://example.com': terminal prompts disabled"
    )));
}

#[test]
fn git_config_contention_retry_retries_known_contention() {
    let attempts = Cell::new(0);
    retry_git_config_contention(|| {
        attempts.set(attempts.get() + 1);
        if attempts.get() == 1 {
            return Err(git_config_failure(
                "error: could not lock config file .gitconfig: File exists",
            ));
        }
        Ok(())
    })
    .expect("known config contention should be retried");

    assert_eq!(attempts.get(), 2);
}

#[test]
fn git_config_contention_retry_does_not_retry_unrelated_failures() {
    let attempts = Cell::new(0);
    let err = retry_git_config_contention(|| {
        attempts.set(attempts.get() + 1);
        Err::<(), _>(git_config_failure("fatal: invalid config value"))
    })
    .expect_err("unrelated config failure should not be retried");

    assert_eq!(attempts.get(), 1);
    assert_eq!(
        err.to_string(),
        "git config --global gitcomet.submodule.allowfiletransport-example true failed"
    );
}

#[test]
fn consent_key_depends_on_root_and_source_path() {
    let a =
        submodule_file_transport_consent_key(Path::new("/repo-a"), Path::new("/sources/local-one"));
    let b =
        submodule_file_transport_consent_key(Path::new("/repo-a"), Path::new("/sources/local-two"));
    let c =
        submodule_file_transport_consent_key(Path::new("/repo-b"), Path::new("/sources/local-one"));

    assert_ne!(a, b);
    assert_ne!(a, c);
}
