//! Proves the patterns `gitcomet_core::gitignore` generates are ones real git
//! honours.
//!
//! The unit tests in that module assert the exact strings, but a wrong escaping
//! rule would look perfectly reasonable there and still match nothing. Only git
//! itself can settle whether `/out\[1\].log` ignores `out[1].log`, and the cost
//! of getting it wrong is a feature that silently does nothing.

use gitcomet_core::gitignore::{self, GitignoreScope};
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let status = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

/// Whether git considers `relative` ignored, via `git check-ignore`.
fn is_ignored(repo: &Path, relative: &Path) -> bool {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.arg("-C")
        .arg(repo)
        .args(["check-ignore", "-q", "--no-index"])
        .arg(relative)
        .status()
        .expect("git check-ignore to run")
        .success()
}

/// The untracked paths git still reports, so we can prove the *unignored*
/// control file survives every pattern we wrote.
fn untracked(repo: &Path) -> Vec<String> {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let out = cmd
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain", "-uall"])
        .output()
        .expect("git status to run");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn generated_patterns_are_honoured_by_git() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    // Every name here exercises one escaping rule: a bracket that would read as
    // a character class, a `*`/`?` that would glob, a leading `#` that would be
    // a comment without the anchor, a leading `!` that would negate, and a
    // trailing space git strips unless it is quoted.
    let names = [
        "build/out[1].log",
        "build/wild*card.txt",
        "build/what?.txt",
        "#notes.txt",
        "!important.txt",
        "trailing ",
    ];
    // Never matched by any pattern below: if a stray `*` or an over-broad
    // folder rule slipped in, this file would vanish and the test would catch it.
    let control = "build/keep.txt";

    fs::create_dir_all(repo.join("build")).expect("create build dir");
    for name in names.iter().chain(std::iter::once(&control)) {
        fs::write(repo.join(name), b"x").expect("create test file");
    }

    let patterns: Vec<String> = names
        .iter()
        .map(|name| {
            gitignore::pattern_for(Path::new(name), GitignoreScope::File)
                .unwrap_or_else(|| panic!("expected a file pattern for {name}"))
        })
        .collect();
    let contents =
        gitignore::append_patterns("", &patterns).expect("expected patterns to be appended");
    fs::write(repo.join(gitignore::FILE_NAME), &contents).expect("write .gitignore");

    for name in names {
        assert!(
            is_ignored(&repo, Path::new(name)),
            "git does not honour the generated pattern for {name:?}; .gitignore was:\n{contents}"
        );
    }
    assert!(
        !is_ignored(&repo, Path::new(control)),
        "{control:?} matches no generated pattern and must stay visible"
    );

    let still_untracked = untracked(&repo);
    assert!(
        still_untracked.contains(&control.to_string()),
        "the control file should still be listed, got {still_untracked:?}"
    );
    for name in names {
        assert!(
            !still_untracked.iter().any(|path| path == name),
            "{name:?} should have dropped out of the untracked list, got {still_untracked:?}"
        );
    }
}

#[test]
fn folder_and_extension_patterns_are_honoured_by_git() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);

    fs::create_dir_all(repo.join("target/debug")).expect("create nested dir");
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("target/debug/app"), b"x").expect("write file");
    fs::write(repo.join("src/main.rs"), b"x").expect("write file");
    fs::write(repo.join("src/notes.log"), b"x").expect("write file");
    fs::write(repo.join("root.log"), b"x").expect("write file");

    let folder = gitignore::pattern_for(Path::new("target/debug/app"), GitignoreScope::Folder)
        .expect("expected a folder pattern");
    let extension = gitignore::pattern_for(Path::new("src/notes.log"), GitignoreScope::Extension)
        .expect("expected an extension pattern");
    assert_eq!(folder, "/target/debug/");
    assert_eq!(extension, "*.log");

    let contents = gitignore::append_patterns("", &[folder, extension])
        .expect("expected patterns to be appended");
    fs::write(repo.join(gitignore::FILE_NAME), &contents).expect("write .gitignore");

    assert!(is_ignored(&repo, Path::new("target/debug/app")));
    assert!(
        is_ignored(&repo, Path::new("src/notes.log")),
        "the extension pattern is deliberately unanchored, so it matches at any depth"
    );
    assert!(
        is_ignored(&repo, Path::new("root.log")),
        "…including at the repository root"
    );
    assert!(
        !is_ignored(&repo, Path::new("src/main.rs")),
        "a sibling with a different extension must be untouched"
    );
}
