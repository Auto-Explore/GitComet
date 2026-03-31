use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let status = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn blame_file_reports_head_and_explicit_revision() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("story.txt"), "one\ntwo\n").unwrap();
    run_git(repo, &["add", "story.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let base_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    std::fs::write(repo.join("story.txt"), "one\ntwo updated\n").unwrap();
    run_git(repo, &["add", "story.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "update"],
    );
    let head_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let head_blame = opened.blame_file(Path::new("story.txt"), None).unwrap();
    assert_eq!(head_blame.len(), 2);
    assert_eq!(
        head_blame
            .iter()
            .map(|line| line.line.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "two updated"]
    );
    assert_eq!(&*head_blame[0].commit_id, base_id);
    assert_eq!(&*head_blame[0].author, "You");
    assert_eq!(&*head_blame[0].summary, "base");
    assert!(head_blame[0].author_time_unix.is_some());
    assert_eq!(&*head_blame[1].commit_id, head_id);
    assert_eq!(&*head_blame[1].author, "You");
    assert_eq!(&*head_blame[1].summary, "update");
    assert!(head_blame[1].author_time_unix.is_some());

    let base_blame = opened
        .blame_file(Path::new("story.txt"), Some(base_id.as_str()))
        .unwrap();
    assert_eq!(base_blame.len(), 2);
    assert_eq!(
        base_blame
            .iter()
            .map(|line| line.line.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert!(
        base_blame
            .iter()
            .all(|line| line.commit_id.as_ref() == base_id)
    );
    assert!(base_blame.iter().all(|line| line.author.as_ref() == "You"));
    assert!(
        base_blame
            .iter()
            .all(|line| line.summary.as_ref() == "base")
    );
    assert!(
        base_blame
            .iter()
            .all(|line| line.author_time_unix.is_some())
    );
}
