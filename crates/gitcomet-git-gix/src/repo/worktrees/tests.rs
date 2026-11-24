use super::{
    GixRepo, Worktree, parse_git_worktree_list_porcelain_z, push_worktree_args,
    sole_primary_worktree,
};
use crate::test_support::{git_output, git_stdout, git_success, init_test_repo, open_repo};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CancellationToken, GitRepository as _};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

fn committed_primary() -> (tempfile::TempDir, GixRepo) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("repository space");
    std::fs::create_dir(&path).unwrap();
    init_test_repo(&path);
    git_success(&path, &["commit", "--allow-empty", "-m", "initial"]);
    git_success(&path, &["branch", "-M", "main"]);
    let repo = open_repo(&path);
    (root, repo)
}

fn oracle(repo: &GixRepo) -> Vec<Worktree> {
    let output = git_output(
        &repo.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    assert!(output.status.success(), "{output:?}");
    parse_git_worktree_list_porcelain_z(&output.stdout).unwrap()
}

fn assert_listing(repo: &GixRepo, commands: usize) {
    let (actual, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    assert_eq!(actual.unwrap(), oracle(repo));
    assert_eq!(trace.len(), commands, "{trace:?}");
}

#[test]
fn native_single_worktree_matches_git_after_head_changes() {
    let (_root, repo) = committed_primary();
    assert_listing(&repo, 0);
    git_success(&repo.spec.workdir, &["checkout", "-b", "topic/nested"]);
    git_success(
        &repo.spec.workdir,
        &["commit", "--allow-empty", "-m", "second"],
    );
    git_success(&repo.spec.workdir, &["pack-refs", "--all"]);
    assert_listing(&repo, 0);
    git_success(&repo.spec.workdir, &["checkout", "--detach", "HEAD~1"]);
    assert_listing(&repo, 0);
    git_success(&repo.spec.workdir, &["checkout", "main"]);
    assert_listing(&repo, 0);
}

#[test]
fn native_single_worktree_rechecks_linked_registrations_each_time() {
    let (root, repo) = committed_primary();
    assert!(repo.may_be_sole_primary_worktree());
    assert_listing(&repo, 0);
    let linked = root.path().join("linked checkout");
    git_success(
        &repo.spec.workdir,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    assert!(!repo.may_be_sole_primary_worktree());
    assert_listing(&repo, 1);
    let linked_repo = open_repo(&linked);
    assert!(!linked_repo.may_be_sole_primary_worktree());
    assert_listing(&linked_repo, 1);
    drop(linked_repo);
    git_success(
        &repo.spec.workdir,
        &["worktree", "lock", linked.to_str().unwrap()],
    );
    assert_listing(&repo, 1);
    git_success(
        &repo.spec.workdir,
        &["worktree", "unlock", linked.to_str().unwrap()],
    );
    git_success(
        &repo.spec.workdir,
        &["worktree", "remove", linked.to_str().unwrap()],
    );
    assert!(repo.may_be_sole_primary_worktree());
    assert_listing(&repo, 0);
}

#[test]
fn native_single_worktree_keeps_git_for_unborn_and_separate_git_dirs() {
    let unborn = tempfile::tempdir().unwrap();
    init_test_repo(unborn.path());
    assert_listing(&open_repo(unborn.path()), 1);

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("checkout");
    std::fs::create_dir(&path).unwrap();
    let separate = root.path().join("separate metadata");
    git_success(
        &path,
        &["init", "--separate-git-dir", separate.to_str().unwrap()],
    );
    git_success(
        &path,
        &[
            "-c",
            "user.name=Test User",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    assert_listing(&open_repo(&path), 1);
}

#[test]
fn native_single_worktree_reads_oid_without_requiring_its_object() {
    let (_root, repo) = committed_primary();
    git_success(&repo.spec.workdir, &["tag", "-a", "tagged", "-m", "tag"]);
    let tag = git_stdout(&repo.spec.workdir, &["rev-parse", "refs/tags/tagged"]);
    std::fs::write(repo.spec.workdir.join(".git/HEAD"), format!("{tag}\n")).unwrap();
    assert_listing(&repo, 0);
    let absent = "1111111111111111111111111111111111111111";
    std::fs::write(repo.spec.workdir.join(".git/HEAD"), format!("{absent}\n")).unwrap();
    assert_listing(&repo, 0);
    std::fs::write(
        repo.spec.workdir.join(".git/refs/heads/main"),
        format!("{absent}\n"),
    )
    .unwrap();
    std::fs::write(
        repo.spec.workdir.join(".git/HEAD"),
        b"ref: refs/heads/main\n",
    )
    .unwrap();
    assert_listing(&repo, 0);
}

#[test]
fn native_single_worktree_defers_reduced_trust_to_git() {
    let (_root, repo) = committed_primary();
    let reduced = gix::open_opts(
        repo.spec.workdir.join(".git"),
        gix::open::Options::default()
            .permissions(gix::open::Permissions::isolated())
            .with_trust(gix::sec::Trust::Reduced),
    )
    .unwrap();
    assert_eq!(reduced.git_dir_trust(), gix::sec::Trust::Reduced);
    assert!(sole_primary_worktree(&reduced).is_none());
}

#[test]
fn native_single_worktree_defers_uncertain_layouts_and_symbolic_chains() {
    let (_root, repo) = committed_primary();
    let registrations = repo.spec.workdir.join(".git/worktrees");
    std::fs::write(&registrations, b"not a registration directory").unwrap();
    assert!(!repo.may_be_sole_primary_worktree());
    assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
    let (_, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    assert_eq!(trace.len(), 1);
    std::fs::remove_file(&registrations).unwrap();
    std::fs::create_dir(&registrations).unwrap();
    std::fs::create_dir(registrations.join("unknown checkout")).unwrap();
    assert!(!repo.may_be_sole_primary_worktree());
    assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
    std::fs::remove_dir(registrations.join("unknown checkout")).unwrap();
    assert!(repo.may_be_sole_primary_worktree());
    assert_listing(&repo, 0);

    git_success(
        &repo.spec.workdir,
        &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
    );
    git_success(
        &repo.spec.workdir,
        &["symbolic-ref", "HEAD", "refs/heads/alias"],
    );
    assert_listing(&repo, 1);
    git_success(
        &repo.spec.workdir,
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    git_success(&repo.spec.workdir, &["config", "core.worktree", "."]);
    // The layout-only preflight remains positive, but the fresh config
    // must still reject the native path after a same-handle change.
    assert!(repo.may_be_sole_primary_worktree());
    assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
    let (actual, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    assert_eq!(actual.unwrap(), oracle(&repo));
    assert_eq!(
        trace
            .iter()
            .map(|command| command.label.as_str())
            .collect::<Vec<_>>(),
        [
            "git rev-parse --is-bare-repository",
            "git worktree list --porcelain -z"
        ]
    );
    assert_listing(&repo, 1);
}

#[test]
fn native_single_worktree_pre_cancelled_load_never_starts_git() {
    let (_root, repo) = committed_primary();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (result, trace) = crate::command_trace::capture(|| repo.list_worktrees(&cancellation));
    assert!(matches!(
        result.unwrap_err().kind(),
        gitcomet_core::error::ErrorKind::Cancelled
    ));
    assert!(trace.is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn native_single_worktree_does_not_follow_registration_directory_links() {
    let (root, repo) = committed_primary();
    let target = root.path().join("registration target");
    std::fs::create_dir(&target).unwrap();
    let registrations = repo.spec.workdir.join(".git/worktrees");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &registrations).unwrap();
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&registrations)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert!(!repo.may_be_sole_primary_worktree());
    assert_listing(&repo, 1);
}

#[test]
fn worktree_add_places_separator_before_path_and_reference() {
    let mut cmd = Command::new("git");
    let label = push_worktree_args(&mut cmd, &["add"], Path::new("-linked"), Some("main"))
        .expect("valid worktree arguments");
    let args: Vec<_> = cmd.get_args().collect();
    assert_eq!(
        args,
        [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("--"),
            OsStr::new("-linked"),
            OsStr::new("main"),
        ]
    );
    assert_eq!(label, "git worktree add -linked main");
}

#[test]
fn worktree_add_rejects_option_like_reference() {
    let mut cmd = Command::new("git");
    let err = push_worktree_args(&mut cmd, &["add"], Path::new("linked"), Some("--detach"))
        .expect_err("an option-looking reference must be refused");
    assert!(err.to_string().contains("worktree reference"), "{err}");
    assert_eq!(
        cmd.get_args().count(),
        0,
        "nothing should be appended on refusal"
    );
}

#[test]
fn worktree_remove_keeps_flags_before_separator() {
    let mut cmd = Command::new("git");
    let label = push_worktree_args(&mut cmd, &["remove", "--force"], Path::new("linked"), None)
        .expect("valid worktree arguments");
    let args: Vec<_> = cmd.get_args().collect();
    assert_eq!(
        args,
        [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            OsStr::new("--force"),
            OsStr::new("--"),
            OsStr::new("linked"),
        ]
    );
    assert_eq!(label, "git worktree remove --force linked");
}

#[test]
fn parse_git_worktree_list_porcelain_z_parses_regular_and_detached_entries() {
    let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0worktree /repo-linked\0HEAD 2222222222222222222222222222222222222222\0detached\0\0",
        )
        .unwrap();

    assert_eq!(parsed.len(), 2);

    assert_eq!(parsed[0].path, PathBuf::from("/repo"));
    assert_eq!(
        parsed[0].head.as_ref().map(|id| id.as_ref()),
        Some("1111111111111111111111111111111111111111")
    );
    assert_eq!(parsed[0].branch.as_deref(), Some("main"));
    assert!(!parsed[0].detached);

    assert_eq!(parsed[1].path, PathBuf::from("/repo-linked"));
    assert_eq!(
        parsed[1].head.as_ref().map(|id| id.as_ref()),
        Some("2222222222222222222222222222222222222222")
    );
    assert!(parsed[1].branch.is_none());
    assert!(parsed[1].detached);
}

#[test]
fn parse_git_worktree_list_porcelain_z_ignores_noise_before_first_worktree() {
    let parsed = parse_git_worktree_list_porcelain_z(
        b"HEAD deadbeef\0branch refs/heads/ignored\0\0worktree /repo\0branch feature/topic\0\0",
    )
    .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, PathBuf::from("/repo"));
    assert_eq!(parsed[0].branch.as_deref(), Some("feature/topic"));
    assert!(parsed[0].head.is_none());
}

#[test]
fn parse_git_worktree_list_porcelain_z_skips_empty_head_values() {
    let parsed =
        parse_git_worktree_list_porcelain_z(b"worktree /repo\0HEAD \0branch refs/heads/main\0\0")
            .unwrap();

    assert_eq!(parsed.len(), 1);
    assert!(parsed[0].head.is_none());
    assert_eq!(parsed[0].branch.as_deref(), Some("main"));
}

#[test]
fn parse_git_worktree_list_porcelain_z_preserves_newlines_in_paths() {
    let parsed = parse_git_worktree_list_porcelain_z(
        b"worktree /repo\nlinked\0HEAD 1111111111111111111111111111111111111111\0detached\0\0",
    )
    .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, PathBuf::from("/repo\nlinked"));
    assert!(parsed[0].detached);
}

#[test]
fn parse_git_worktree_list_porcelain_z_canonicalizes_existing_worktree_paths() {
    let root = std::env::temp_dir().join(format!(
        "gitcomet-worktree-parse-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let nested = root.join("repo");
    std::fs::create_dir_all(&nested).unwrap();

    let input = format!(
        "worktree {}\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0",
        nested.join("..").join("repo").display()
    );
    let parsed = parse_git_worktree_list_porcelain_z(input.as_bytes()).unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, canonicalize_or_original(nested.clone()));
}
