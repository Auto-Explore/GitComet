use gitcomet_core::domain::SubmoduleStatus;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", NULL_DEVICE);
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
    run_git(&parent_repo, &["config", "protocol.file.allow", "always"]);

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    let submodule_path = Path::new("mods/sub-one");
    let add_output = opened
        .add_submodule_with_output(sub_repo.to_string_lossy().as_ref(), submodule_path)
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

    let update_output = opened
        .update_submodules_with_output()
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
}
