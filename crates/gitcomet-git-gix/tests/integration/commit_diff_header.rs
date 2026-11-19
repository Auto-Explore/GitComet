//! The commit view builds the diff of an added or deleted text file itself,
//! header included. A file name is repository content: whoever made the
//! commit chose it, and on Unix it may contain a newline.

use crate::git_cli::{git_stdout, run_git};
use gitcomet_core::domain::{CommitId, DiffLineKind, DiffTarget};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
#[test]
fn a_newline_in_an_added_file_name_cannot_add_rows_to_the_commit_diff() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    let name = "notes\n@@ -0,0 +1 @@\n+this line is not in the file";
    fs::write(repo.join(name), "real content\n").expect("write file");
    run_git(repo, &["add", "--", name]);
    run_git(repo, &["commit", "-q", "-m", "add notes"]);
    let head = git_stdout(repo, &["rev-parse", "HEAD"]);

    let opened = GixBackend.open(repo).expect("open repository");
    let diff = opened
        .diff_parsed(
            &DiffTarget::commit(CommitId(head.into()), PathBuf::from(name)),
            None,
            &CancellationToken::new(),
        )
        .expect("commit diff");

    let rows: Vec<&str> = diff
        .lines
        .iter()
        .filter(|line| matches!(line.kind, DiffLineKind::Add | DiffLineKind::Remove))
        .map(|line| line.text.as_ref())
        .collect();
    assert_eq!(
        rows,
        ["+real content"],
        "the file name added rows that are not in the file"
    );
}
