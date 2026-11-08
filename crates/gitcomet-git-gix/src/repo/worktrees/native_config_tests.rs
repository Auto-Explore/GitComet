use super::*;
use crate::test_support::{git_output, git_success, init_test_repo};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::GitRepository as _;

fn open_config_repo(path: &Path) -> GixRepo {
    let mut permissions = gix::open::Permissions::isolated();
    permissions.config.includes = true;
    let repo = crate::open::open_worktree_repo_with(path, permissions).unwrap();
    GixRepo::new(path.to_path_buf(), repo.into_sync()).unwrap()
}

fn fixture(linked: bool) -> (tempfile::TempDir, GixRepo, Option<GixRepo>) {
    let root = tempfile::tempdir().unwrap();
    let path = canonicalize_or_original(root.path().to_path_buf()).join("main");
    std::fs::create_dir(&path).unwrap();
    init_test_repo(&path);
    git_success(&path, &["commit", "--allow-empty", "-m", "initial"]);
    git_success(&path, &["config", "core.ignorecase", "false"]);
    let repo = open_config_repo(&path);
    let caller = linked.then(|| {
        let mut paths = Vec::new();
        for name in ["B", "a"] {
            let checkout = path.parent().unwrap().join(name);
            git_success(
                &path,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    checkout.to_str().unwrap(),
                    "HEAD",
                ],
            );
            paths.push(checkout);
        }
        open_config_repo(&paths[0])
    });
    (root, repo, caller)
}

fn assert_git_fallback(repo: &GixRepo) {
    let expected = git_output(
        &repo.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    assert!(expected.status.success(), "{expected:?}");
    let expected = parse_git_worktree_list_porcelain_z(&expected.stdout).unwrap();
    let (actual, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    assert_eq!(actual.unwrap(), expected, "full ordered CLI rows");
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
    let (unchanged, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    assert_eq!(unchanged.unwrap(), expected);
    assert_eq!(
        trace
            .iter()
            .map(|command| command.label.as_str())
            .collect::<Vec<_>>(),
        ["git worktree list --porcelain -z"]
    );
}

fn unsupported_include(condition: &str, per_worktree: bool, expanded: bool, linked: bool) {
    let (_root, main, caller) = fixture(linked);
    let caller = if per_worktree {
        caller.as_ref().unwrap()
    } else {
        &main
    };
    if per_worktree {
        git_success(
            &main.spec.workdir,
            &["config", "extensions.worktreeConfig", "true"],
        );
    }
    let included = main.spec.workdir.parent().unwrap().join("included.conf");
    std::fs::write(&included, "[core]\nignoreCase = true\n").unwrap();
    let spelling = caller.spec.workdir.to_string_lossy().replace('\\', "/");
    let spelling = if condition == "worktree/i" {
        spelling.to_ascii_uppercase()
    } else {
        spelling
    };
    let key = format!("includeIf.{condition}:{spelling}.path");
    if expanded {
        let outer = included.with_file_name("outer.conf");
        std::fs::write(&outer, "").unwrap();
        git_success(
            &main.spec.workdir,
            &[
                "config",
                "--file",
                outer.to_str().unwrap(),
                &key,
                included.to_str().unwrap(),
            ],
        );
        git_success(
            &main.spec.workdir,
            &["config", "include.path", outer.to_str().unwrap()],
        );
    } else {
        let mut args = vec!["config"];
        if per_worktree {
            args.push("--worktree");
        }
        args.extend([key.as_str(), included.to_str().unwrap()]);
        git_success(&caller.spec.workdir, &args);
    }
    // Diagnostic witness only: older CI Git may not implement this condition.
    // Correctness still requires CLI parity and conservative fallback there.
    let effective = git_output(
        &caller.spec.workdir,
        &["config", "--type=bool", "--get", "core.ignorecase"],
    );
    let rows = git_output(
        &caller.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    eprintln!(
        "condition={condition} per_worktree={per_worktree} expanded={expanded} effective_ignorecase={:?} ordered_rows={:?}",
        String::from_utf8_lossy(&effective.stdout),
        String::from_utf8_lossy(&rows.stdout)
    );
    assert_git_fallback(caller);
    if per_worktree {
        assert_git_fallback(&main);
    }
}

#[test]
fn worktree_condition_in_shared_config_defers_to_git() {
    unsupported_include("worktree", false, false, true);
}

#[test]
fn worktree_i_condition_in_shared_config_defers_to_git() {
    unsupported_include("worktree/i", false, false, true);
}

#[test]
fn worktree_condition_in_caller_config_defers_to_git() {
    unsupported_include("worktree", true, false, true);
}

#[test]
fn worktree_i_condition_in_caller_config_defers_to_git() {
    unsupported_include("worktree/i", true, false, true);
}

#[test]
fn expanded_worktree_condition_defers_to_git() {
    unsupported_include("worktree", false, true, true);
}

#[test]
fn sole_primary_with_worktree_condition_defers_to_git() {
    unsupported_include("worktree", false, false, false);
}

fn invalid_boolean(linked: bool, per_worktree: bool) {
    let (_root, main, caller) = fixture(linked);
    let caller = if per_worktree {
        caller.as_ref().unwrap()
    } else {
        &main
    };
    let mut args = vec!["config"];
    if per_worktree {
        git_success(
            &main.spec.workdir,
            &["config", "extensions.worktreeConfig", "true"],
        );
        args.push("--worktree");
    }
    args.extend(["core.ignorecase", "invalid"]);
    git_success(&caller.spec.workdir, &args);
    let expected = git_output(
        &caller.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    assert!(
        !expected.status.success(),
        "the fixture must make Git reject configuration"
    );
    let (result, trace) =
        crate::command_trace::capture(|| caller.list_worktrees(&CancellationToken::new()));
    let error = result.expect_err("invalid config must preserve the CLI error");
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected CLI failure, got {error:?}")
    };
    assert_eq!(failure.exit_code(), expected.status.code());
    assert_eq!(failure.stdout(), expected.stdout);
    assert_eq!(failure.stderr(), expected.stderr);
    assert_eq!(trace.len(), 1, "{trace:?}");
}

#[test]
fn sole_primary_invalid_ignorecase_preserves_cli_error() {
    invalid_boolean(false, false);
}

#[test]
fn linked_shared_invalid_ignorecase_preserves_cli_error() {
    invalid_boolean(true, false);
}

#[test]
fn linked_caller_invalid_ignorecase_preserves_cli_error() {
    invalid_boolean(true, true);
}

#[test]
fn shadowed_invalid_ignorecase_preserves_cli_error() {
    let (_root, repo, _) = fixture(false);
    let path = repo.spec.workdir.join(".git/config");
    let mut config = std::fs::read_to_string(&path).unwrap();
    config.push_str("\n[core]\nignorecase=invalid\nignorecase=true\n");
    std::fs::write(&path, config).unwrap();
    let expected = git_output(
        &repo.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    assert!(!expected.status.success());
    let (result, trace) =
        crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
    let error = result.unwrap_err();
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected CLI failure, got {error:?}")
    };
    assert_eq!(failure.exit_code(), expected.status.code());
    assert_eq!(failure.stdout(), expected.stdout);
    assert_eq!(failure.stderr(), expected.stderr);
    assert_eq!(trace.len(), 1);
}

#[test]
fn uncommon_numeric_ignorecase_keeps_git_authoritative() {
    let (_root, repo, _) = fixture(false);
    git_success(&repo.spec.workdir, &["config", "core.ignorecase", "2"]);
    assert_git_fallback(&repo);
}

#[test]
fn declared_include_with_disabled_permission_preserves_cli_error() {
    let (_root, main, _) = fixture(false);
    let limited = crate::test_support::open_repo(&main.spec.workdir);
    assert!(!limited.repo().open_options().permissions.config.includes);
    let included = main
        .spec
        .workdir
        .parent()
        .unwrap()
        .join("disabled-include.conf");
    std::fs::write(&included, "[core]\nignorecase=invalid\n").unwrap();
    git_success(
        &main.spec.workdir,
        &["config", "include.path", included.to_str().unwrap()],
    );
    let expected = git_output(
        &main.spec.workdir,
        &["worktree", "list", "--porcelain", "-z"],
    );
    assert!(!expected.status.success());
    let (result, trace) =
        crate::command_trace::capture(|| limited.list_worktrees(&CancellationToken::new()));
    let error = result.expect_err("an uninspected include cannot justify native success");
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected CLI failure, got {error:?}")
    };
    assert_eq!(failure.exit_code(), expected.status.code());
    assert_eq!(failure.stdout(), expected.stdout);
    assert_eq!(failure.stderr(), expected.stderr);
    assert_eq!(trace.len(), 1);
}
