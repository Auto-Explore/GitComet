use gitcomet_core::domain::SubmoduleStatus;
use gitcomet_core::services::{GitBackend, SubmoduleTrustDecision};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(windows)]
use std::sync::OnceLock;

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(repo: &Path, args: &[&str]) -> Output {
    git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_string()
}

fn local_submodule_config_entries(repo: &Path) -> Vec<String> {
    let output = git_output(repo, &["config", "--list", "--local"]);
    assert!(output.status.success(), "git config --list --local failed");
    String::from_utf8(output.stdout)
        .expect("git stdout is utf-8")
        .lines()
        .filter(|line| line.starts_with("submodule."))
        .map(ToOwned::to_owned)
        .collect()
}

fn add_submodule_raw(parent_repo: &Path, sub_repo: &Path, path: &Path, name: Option<&str>) {
    let mut cmd = git_command();
    cmd.arg("-C")
        .arg(parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add");
    if let Some(name) = name {
        cmd.arg("--name").arg(name);
    }
    let status = cmd
        .arg(sub_repo)
        .arg(path)
        .status()
        .expect("git submodule add to run");
    assert!(status.success(), "git submodule add failed");
}

fn run_git_with_path(repo: &Path, args: &[&str], path: &Path) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .arg(path)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} {:?} failed", args, path);
}

fn create_stale_submodule_git_dir(
    parent_repo: &Path,
    sub_repo: &Path,
    path: &Path,
    name: Option<&str>,
) {
    add_submodule_raw(parent_repo, sub_repo, path, name);
    run_git(
        parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );
    run_git_with_path(parent_repo, &["submodule", "deinit", "-f", "--"], path);
    run_git_with_path(parent_repo, &["rm", "-f", "--"], path);
    run_git(
        parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "remove submodule",
        ],
    );
}

fn init_repo_with_seed(repo: &Path, file: &str, contents: &str, message: &str) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);

    fs::write(repo.join(file), contents).expect("write seed file");
    run_git(repo, &["add", file]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_submodule_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_submodule_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_submodule_tests() {
            eprintln!(
                "skipping submodule integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn list_submodules_reports_missing_gitmodules_mapping() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).unwrap();
    fs::create_dir_all(&parent_repo).unwrap();

    init_repo_with_seed(&sub_repo, "file.txt", "hi\n", "init");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed");
    let submodule_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);

    let status = git_command()
        .arg("-C")
        .arg(&parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&sub_repo)
        .arg("submod")
        .status()
        .expect("git submodule add to run");
    assert!(status.success(), "git submodule add failed");

    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).unwrap();

    fs::write(parent_repo.join(".gitmodules"), "").unwrap();
    run_git(&parent_repo, &["add", ".gitmodules"]);

    let output = git_output(&parent_repo, &["submodule", "status", "--recursive"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no submodule mapping found in .gitmodules for path"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let submodules = opened.list_submodules().unwrap();
    assert_eq!(submodules.len(), 1);
    assert_eq!(submodules[0].path, PathBuf::from("submod"));
    assert_eq!(submodules[0].status, SubmoduleStatus::MissingMapping);
    assert_eq!(submodules[0].head.as_ref(), submodule_head);
}

#[test]
fn list_submodules_reports_not_initialized_and_head_mismatch() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let original_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);

    let add_status = git_command()
        .arg("-C")
        .arg(&parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&sub_repo)
        .arg("sm")
        .status()
        .expect("git submodule add to run");
    assert!(add_status.success(), "git submodule add failed");
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    fs::remove_dir_all(parent_repo.join("sm")).expect("remove submodule worktree");
    fs::remove_dir_all(parent_repo.join(".git/modules/sm")).expect("remove submodule git dir");

    let not_initialized = opened
        .list_submodules()
        .expect("list uninitialized submodule");
    assert_eq!(not_initialized.len(), 1);
    assert_eq!(not_initialized[0].path, PathBuf::from("sm"));
    assert_eq!(not_initialized[0].status, SubmoduleStatus::NotInitialized);
    assert_eq!(not_initialized[0].head.as_ref(), original_head);

    run_git(
        &parent_repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ],
    );

    fs::write(sub_repo.join("next.txt"), "next\n").expect("write next submodule commit");
    run_git(&sub_repo, &["add", "next.txt"]);
    run_git(
        &sub_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "next"],
    );
    let mismatched_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);

    let fetch_status = git_command()
        .arg("-C")
        .arg(parent_repo.join("sm"))
        .arg("-c")
        .arg("protocol.file.allow=always")
        .args(["fetch", "--quiet"])
        .status()
        .expect("git fetch in submodule to run");
    assert!(fetch_status.success(), "git fetch failed");

    let checkout_status = git_command()
        .arg("-C")
        .arg(parent_repo.join("sm"))
        .args(["checkout", "--quiet", &mismatched_head])
        .status()
        .expect("git checkout in submodule to run");
    assert!(checkout_status.success(), "git checkout failed");

    let head_mismatch = opened.list_submodules().expect("list mismatched submodule");
    assert_eq!(head_mismatch.len(), 1);
    assert_eq!(head_mismatch[0].path, PathBuf::from("sm"));
    assert_eq!(head_mismatch[0].status, SubmoduleStatus::HeadMismatch);
    assert_eq!(head_mismatch[0].head.as_ref(), mismatched_head);
}

#[test]
fn list_submodules_recurses_into_nested_submodules() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let grand_repo = root.join("grand");
    let child_repo = root.join("child");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&grand_repo).expect("create grand repository directory");
    fs::create_dir_all(&child_repo).expect("create child repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&grand_repo, "grand.txt", "grand\n", "seed grand");
    init_repo_with_seed(&child_repo, "child.txt", "child\n", "seed child");
    init_repo_with_seed(&parent_repo, "parent.txt", "parent\n", "seed parent");

    let child_add_status = git_command()
        .arg("-C")
        .arg(&child_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&grand_repo)
        .arg("nested/grand")
        .status()
        .expect("git nested submodule add to run");
    assert!(
        child_add_status.success(),
        "git nested submodule add failed"
    );
    run_git(
        &child_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add nested submodule",
        ],
    );

    let parent_add_status = git_command()
        .arg("-C")
        .arg(&parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&child_repo)
        .arg("mods/child")
        .status()
        .expect("git parent submodule add to run");
    assert!(
        parent_add_status.success(),
        "git parent submodule add failed"
    );
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add child submodule",
        ],
    );

    run_git(
        &parent_repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let listed = opened.list_submodules().expect("list nested submodules");

    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed
            .iter()
            .map(|submodule| submodule.path.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from("mods/child"),
            PathBuf::from("mods/child/nested/grand"),
        ]
    );
    assert!(
        listed
            .iter()
            .all(|submodule| submodule.status == SubmoduleStatus::UpToDate)
    );
}

#[test]
fn list_submodules_reports_merge_conflicted_gitlinks() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    run_git(&sub_repo, &["init"]);
    run_git(&sub_repo, &["config", "user.email", "you@example.com"]);
    run_git(&sub_repo, &["config", "user.name", "You"]);
    run_git(&sub_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(sub_repo.join("file.txt"), "base\n").expect("write base submodule file");
    run_git(&sub_repo, &["add", "file.txt"]);
    run_git(
        &sub_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let base_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);

    run_git(&sub_repo, &["checkout", "-b", "left"]);
    fs::write(sub_repo.join("file.txt"), "left\n").expect("write left submodule file");
    run_git(
        &sub_repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "left"],
    );
    let left_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);

    run_git(&sub_repo, &["checkout", "master"]);
    fs::write(sub_repo.join("file.txt"), "right\n").expect("write right submodule file");
    run_git(
        &sub_repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "right"],
    );
    let right_head = git_stdout(&sub_repo, &["rev-parse", "HEAD"]);
    assert_ne!(left_head, right_head, "submodule branches must diverge");

    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let add_status = git_command()
        .arg("-C")
        .arg(&parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&sub_repo)
        .arg("sm")
        .status()
        .expect("git submodule add to run");
    assert!(add_status.success(), "git submodule add failed");

    let checkout_base_status = git_command()
        .arg("-C")
        .arg(parent_repo.join("sm"))
        .args(["checkout", "--quiet", &base_head])
        .status()
        .expect("git checkout base in submodule to run");
    assert!(checkout_base_status.success(), "git checkout base failed");
    run_git(&parent_repo, &["add", "sm"]);
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule base",
        ],
    );

    run_git(&parent_repo, &["checkout", "-b", "branch-left"]);
    let checkout_left_status = git_command()
        .arg("-C")
        .arg(parent_repo.join("sm"))
        .args(["checkout", "--quiet", &left_head])
        .status()
        .expect("git checkout left in submodule to run");
    assert!(checkout_left_status.success(), "git checkout left failed");
    run_git(&parent_repo, &["add", "sm"]);
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "use left submodule",
        ],
    );

    run_git(&parent_repo, &["checkout", "master"]);
    let checkout_right_status = git_command()
        .arg("-C")
        .arg(parent_repo.join("sm"))
        .args(["checkout", "--quiet", &right_head])
        .status()
        .expect("git checkout right in submodule to run");
    assert!(checkout_right_status.success(), "git checkout right failed");
    run_git(&parent_repo, &["add", "sm"]);
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "use right submodule",
        ],
    );

    let merge_output = git_output(&parent_repo, &["merge", "branch-left"]);
    assert!(!merge_output.status.success(), "merge should conflict");

    let backend = GixBackend;
    let opened = backend
        .open(&parent_repo)
        .expect("open conflicted parent repository");
    let listed = opened
        .list_submodules()
        .expect("list conflicted submodules");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, PathBuf::from("sm"));
    assert_eq!(listed[0].status, SubmoduleStatus::MergeConflict);
    assert_eq!(
        listed[0].head.as_ref(),
        "0000000000000000000000000000000000000000"
    );
}

#[test]
fn submodule_add_update_remove_round_trip() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub source");
    let parent_repo = root.join("parent repo");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    let submodule_path = Path::new("mods/sub-one");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };
    let add_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            None,
            false,
            &approved_sources,
        )
        .expect("add submodule");
    assert_eq!(add_output.exit_code, Some(0));

    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    let listed = opened.list_submodules().expect("list submodules after add");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, PathBuf::from("mods/sub-one"));
    assert_eq!(
        listed[0].status,
        gitcomet_core::domain::SubmoduleStatus::UpToDate
    );
    assert_eq!(listed[0].head.as_ref().len(), 40);

    assert_eq!(
        opened
            .check_submodule_update_trust()
            .expect("check update trust after approval"),
        SubmoduleTrustDecision::Proceed
    );

    let update_output = opened
        .update_submodules_with_output(&[])
        .expect("update submodules");
    assert_eq!(update_output.exit_code, Some(0));

    let remove_output = opened
        .remove_submodule_with_output(submodule_path)
        .expect("remove submodule");
    assert_eq!(remove_output.exit_code, Some(0));
    assert!(remove_output.command.contains("Remove submodule"));

    let listed_after_remove = opened
        .list_submodules()
        .expect("list submodules after remove");
    assert!(listed_after_remove.is_empty());
    assert!(!parent_repo.join("mods/sub-one").exists());
    assert!(local_submodule_config_entries(&parent_repo).is_empty());
    assert!(!parent_repo.join(".git/modules/mods/sub-one").exists());
    assert!(!parent_repo.join(".git/modules/mods").exists());
}

#[test]
fn add_submodule_does_not_restrict_https_or_ssh_transports() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("parent");
    fs::create_dir_all(&repo).expect("create parent repository directory");
    init_repo_with_seed(&repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open parent repository");

    for (url, blocked_transport) in [
        ("https://127.0.0.1:1/repo.git", "https"),
        ("ssh://git@127.0.0.1:1/repo.git", "ssh"),
    ] {
        let err = opened
            .add_submodule_with_output(url, Path::new("mods/sub-one"), None, None, false, &[])
            .expect_err("dummy remote should fail without a reachable server");
        let rendered = err.to_string();
        assert!(
            !rendered.contains(&format!("transport '{blocked_transport}' not allowed")),
            "unexpected protocol allowlist failure for {url}: {rendered}"
        );
    }
}

#[test]
fn add_local_submodule_requires_explicit_trust() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    let submodule_path = Path::new("mods/sub");
    let trust = opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust");
    let approved_sources = match trust {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let err = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            None,
            false,
            &[],
        )
        .expect_err("local submodule should fail without trust");
    assert!(
        err.to_string().contains("Explicit trust is required"),
        "unexpected error: {err}"
    );

    let add_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            None,
            false,
            &approved_sources,
        )
        .expect("add trusted local submodule");
    assert_eq!(add_output.exit_code, Some(0));
}

#[test]
fn add_submodule_supports_branch_selection() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    run_git(&sub_repo, &["branch", "feature"]);
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let submodule_path = Path::new("mods/sub");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let add_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            Some("feature"),
            None,
            false,
            &approved_sources,
        )
        .expect("add submodule with branch");
    assert_eq!(add_output.exit_code, Some(0));
    assert!(add_output.command.contains("--branch feature"));

    let gitmodules = fs::read_to_string(parent_repo.join(".gitmodules")).expect("read .gitmodules");
    assert!(gitmodules.contains("branch = feature"));
    assert_eq!(
        git_stdout(&parent_repo.join("mods/sub"), &["branch", "--show-current"]),
        "feature"
    );
}

#[test]
fn add_submodule_supports_multiple_branches_from_same_source() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    let default_branch = git_stdout(&sub_repo, &["symbolic-ref", "--short", "HEAD"]);
    run_git(&sub_repo, &["checkout", "-b", "feature"]);
    fs::write(sub_repo.join("file.txt"), "feature\n").expect("write feature contents");
    run_git(
        &sub_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-am",
            "feature commit",
        ],
    );
    run_git(&sub_repo, &["checkout", &default_branch]);
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    let main_path = Path::new("mods/main");
    let approved_main = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), main_path)
        .expect("check local submodule trust for main")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };
    let main_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            main_path,
            Some(default_branch.as_str()),
            None,
            false,
            &approved_main,
        )
        .expect("add default-branch submodule");
    assert_eq!(main_output.exit_code, Some(0));

    let feature_path = Path::new("mods/feature");
    let approved_feature = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), feature_path)
        .expect("check local submodule trust for feature")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        SubmoduleTrustDecision::Proceed => Vec::new(),
    };
    let feature_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            feature_path,
            Some("feature"),
            None,
            false,
            &approved_feature,
        )
        .expect("add feature-branch submodule");
    assert_eq!(feature_output.exit_code, Some(0));

    let listed = opened.list_submodules().expect("list added submodules");
    assert_eq!(listed.len(), 2);
    assert_eq!(
        git_stdout(
            &parent_repo.join("mods/main"),
            &["branch", "--show-current"]
        ),
        default_branch
    );
    assert_eq!(
        git_stdout(
            &parent_repo.join("mods/feature"),
            &["branch", "--show-current"]
        ),
        "feature"
    );

    let gitmodules = fs::read_to_string(parent_repo.join(".gitmodules")).expect("read .gitmodules");
    assert!(gitmodules.contains(&format!("branch = {default_branch}")));
    assert!(gitmodules.contains("branch = feature"));
}

#[test]
fn add_submodule_failed_branch_checkout_cleans_partial_clone_and_metadata() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let submodule_path = Path::new("mods/sub");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let err = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            Some("does-not-exist"),
            None,
            false,
            &approved_sources,
        )
        .expect_err("add with missing branch should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("does-not-exist"),
        "unexpected branch failure error: {rendered}"
    );

    assert!(
        !parent_repo.join("mods/sub").exists(),
        "expected failed submodule checkout to be removed"
    );
    assert!(
        !parent_repo.join(".git/modules/mods/sub").exists(),
        "expected failed submodule metadata to be removed"
    );
    assert!(
        !parent_repo.join(".gitmodules").exists(),
        "expected no .gitmodules entry after failed add"
    );
    assert!(local_submodule_config_entries(&parent_repo).is_empty());
    assert!(
        git_stdout(&parent_repo, &["submodule"]).is_empty(),
        "expected git submodule to report no registered submodules"
    );
    assert!(
        opened
            .list_submodules()
            .expect("list submodules")
            .is_empty(),
        "expected failed submodule add not to be listed"
    );
}

#[test]
fn add_submodule_failed_branch_checkout_cleans_partial_clone_with_custom_name() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let submodule_path = Path::new("mods/sub");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let err = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            Some("does-not-exist"),
            Some("custom-name"),
            false,
            &approved_sources,
        )
        .expect_err("add with missing branch and custom name should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("does-not-exist"),
        "unexpected branch failure error: {rendered}"
    );

    assert!(
        !parent_repo.join("mods/sub").exists(),
        "expected failed submodule checkout to be removed"
    );
    assert!(
        !parent_repo.join(".git/modules/custom-name").exists(),
        "expected failed custom-name metadata to be removed"
    );
    assert!(
        !parent_repo.join(".gitmodules").exists(),
        "expected no .gitmodules entry after failed add"
    );
    assert!(local_submodule_config_entries(&parent_repo).is_empty());
}

#[test]
fn add_submodule_supports_custom_logical_name_for_local_git_dir_collision() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let submodule_path = Path::new("sm");
    create_stale_submodule_git_dir(&parent_repo, &sub_repo, submodule_path, None);
    assert!(
        parent_repo.join(".git/modules/sm").exists(),
        "expected stale local submodule git dir"
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let err = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            None,
            false,
            &approved_sources,
        )
        .expect_err("add without custom name or force should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("If you want to reuse this local git directory")
            || rendered.contains("use the '--force' option")
            || rendered.contains("choose another name with the '--name' option"),
        "unexpected collision error: {rendered}"
    );

    let add_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            Some("sm-renamed"),
            false,
            &approved_sources,
        )
        .expect("add submodule with custom logical name");
    assert_eq!(add_output.exit_code, Some(0));
}

#[test]
fn add_submodule_supports_force_for_local_git_dir_collision() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");

    let submodule_path = Path::new("sm");
    create_stale_submodule_git_dir(&parent_repo, &sub_repo, submodule_path, None);
    assert!(
        parent_repo.join(".git/modules/sm").exists(),
        "expected stale local submodule git dir"
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let approved_sources = match opened
        .check_submodule_add_trust(sub_repo.to_string_lossy().as_ref(), submodule_path)
        .expect("check local submodule trust")
    {
        SubmoduleTrustDecision::Prompt { sources } => sources,
        other => panic!("expected trust prompt for local submodule, got {other:?}"),
    };

    let add_output = opened
        .add_submodule_with_output(
            sub_repo.to_string_lossy().as_ref(),
            submodule_path,
            None,
            None,
            true,
            &approved_sources,
        )
        .expect("add submodule with force");
    assert_eq!(add_output.exit_code, Some(0));
}

#[test]
fn remove_submodule_cleans_custom_logical_name_metadata() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).expect("create sub repository directory");
    fs::create_dir_all(&parent_repo).expect("create parent repository directory");

    init_repo_with_seed(&sub_repo, "file.txt", "hello\n", "seed submodule");
    init_repo_with_seed(&parent_repo, "seed.txt", "seed\n", "seed parent");
    add_submodule_raw(
        &parent_repo,
        &sub_repo,
        Path::new("mods/sub"),
        Some("custom"),
    );
    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let remove_output = opened
        .remove_submodule_with_output(Path::new("mods/sub"))
        .expect("remove submodule");
    assert_eq!(remove_output.exit_code, Some(0));
    assert!(local_submodule_config_entries(&parent_repo).is_empty());
    assert!(!parent_repo.join(".git/modules/custom").exists());
}
