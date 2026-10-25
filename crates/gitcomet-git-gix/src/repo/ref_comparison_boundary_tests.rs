use super::*;
use gitcomet_core::ref_comparison::*;
use std::process::{Command, Output};

const CHILD_ROOT: &str = "GITCOMET_COMPARISON_BOUNDARY_ROOT";

fn git(dir: &Path, args: &[&str]) -> Output {
    let mut command = crate::test_support::git_command();
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    let output = command
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "nul" } else { "/dev/null" },
        )
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    std::fs::create_dir(&a).unwrap();
    git(&a, &["init", "-q", "--template=", "-b", "topic"]);
    std::fs::write(a.join("file"), "base\n").unwrap();
    git(&a, &["add", "."]);
    git(&a, &["commit", "-qm", "base"]);
    let base = String::from_utf8(git(&a, &["rev-parse", "HEAD"]).stdout).unwrap();
    std::fs::write(root.path().join("base"), base.trim()).unwrap();
    std::fs::write(a.join("file"), "target\n").unwrap();
    git(&a, &["commit", "-qam", "target"]);
    let target = String::from_utf8(git(&a, &["rev-parse", "HEAD"]).stdout).unwrap();
    std::fs::write(root.path().join("target"), target.trim()).unwrap();
    git(
        &a,
        &["update-ref", "refs/remotes/origin/dev", target.trim()],
    );
    git(
        &a,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/dev",
        ],
    );
    git(
        root.path(),
        &["clone", "-q", "--no-hardlinks", "--no-checkout", "a", "b"],
    );
    let b = root.path().join("b");
    git(&b, &["update-ref", "refs/remotes/origin/dev", base.trim()]);
    git(
        &b,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/dev",
        ],
    );
    root
}

fn child_command(test: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "nul" } else { "/dev/null" },
        )
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD_ROOT, root);
    command
}

#[test]
fn comparison_targets_and_walks_ignore_inherited_repository_overrides() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        let fixture = fixture();
        let b = fixture.path().join("b");
        let output = child_command("repo::ref_comparison_boundary_tests::comparison_targets_and_walks_ignore_inherited_repository_overrides", fixture.path())
            .env("GIT_DIR", b.join(".git"))
            .env("GIT_COMMON_DIR", b.join(".git"))
            .env("GIT_WORK_TREE", &b)
            .env("GIT_OBJECT_DIRECTORY", b.join(".git/objects"))
            .output().unwrap();
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let root = PathBuf::from(root);
    let repo = crate::test_support::open_repo(&root.join("a"));
    assert_eq!(repo.spec().workdir, root.join("a"));
    let token = CancellationToken::new();
    let target = repo.ref_comparison_target(None, &token).unwrap();
    let base = CommitId(std::fs::read_to_string(root.join("base")).unwrap().into());
    let batch = repo
        .analyze_ref_comparisons(
            &RefComparisonRequest {
                target: target.clone(),
                subjects: vec![base],
                depth: RefComparisonDepth::Counts,
                budget: RefComparisonBudget::automatic(),
            },
            &token,
        )
        .unwrap();
    assert_eq!(
        target.oid.as_ref(),
        std::fs::read_to_string(root.join("target")).unwrap(),
        "wrong repository supplied the target: {batch:?}"
    );
    assert_eq!(
        batch.results[0].counts,
        RefComparisonCounts::Exact {
            ahead: 0,
            behind: 1
        }
    );
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::Merged)
    );
}

#[cfg(unix)]
#[test]
fn target_resolution_rejects_a_repository_replaced_before_final_rev_parse() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        let fixture = fixture();
        let output = child_command("repo::ref_comparison_boundary_tests::target_resolution_rejects_a_repository_replaced_before_final_rev_parse", fixture.path()).output().unwrap();
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    use std::os::unix::fs::PermissionsExt as _;
    let root = PathBuf::from(root);
    let repo = crate::test_support::open_repo(&root.join("a"));
    let wrapper = root.join("comparison-git-wrapper");
    std::fs::write(
        &wrapper,
        r#"#!/bin/sh
set -eu
case " $* " in
    *" rev-parse --verify "*)
        mv "$GITCOMET_COMPARISON_BOUNDARY_ROOT/a" "$GITCOMET_COMPARISON_BOUNDARY_ROOT/original"
        mv "$GITCOMET_COMPARISON_BOUNDARY_ROOT/b" "$GITCOMET_COMPARISON_BOUNDARY_ROOT/a"
        : > "$GITCOMET_COMPARISON_BOUNDARY_ROOT/swapped"
        ;;
esac
exec git "$@"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    gitcomet_core::process::install_git_executable_path(Some(wrapper));
    let result = repo.ref_comparison_target(None, &CancellationToken::new());
    assert!(
        root.join("swapped").exists(),
        "the target lookup never reached the replacement barrier"
    );
    let error =
        result.expect_err("a replaced repository cannot supply a current comparison target");
    assert!(error.to_string().contains("replaced"), "{error}");
}
