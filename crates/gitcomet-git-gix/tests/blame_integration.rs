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

    // The base commit introduced story.txt, so its lines have no prior
    // revision; the update commit modified an existing file, so it does.
    assert!(
        !head_blame[0].prior_exists,
        "line from the file-introducing commit must report no prior revision"
    );
    assert!(
        head_blame[1].prior_exists,
        "line from a later modifying commit must report a prior revision"
    );

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

/// Build a repo where `src/old.txt` is committed, then renamed to `lib/new.txt`
/// in its own commit, then a single line is tweaked in a later commit. The pure
/// rename keeps the file identical, so it is detected and followed (the combined
/// rename+edit-in-one-commit case is one git's own blame does not cross). Returns
/// the temp dir and the base commit id.
fn rename_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/old.txt"), "alpha\nbeta\ngamma\n").unwrap();
    run_git(repo, &["add", "src/old.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let base_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    // Pure rename across directories (content unchanged).
    std::fs::create_dir_all(repo.join("lib")).unwrap();
    run_git(repo, &["mv", "src/old.txt", "lib/new.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rename"],
    );

    // Later, tweak a single line under the new name.
    std::fs::write(repo.join("lib/new.txt"), "alpha\nbeta updated\ngamma\n").unwrap();
    run_git(repo, &["add", "lib/new.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "tweak"],
    );

    (dir, base_id)
}

#[test]
fn blame_follows_renames_and_surfaces_historical_path() {
    let (dir, base_id) = rename_repo();
    let repo = dir.path();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let blame = opened.blame_file(Path::new("lib/new.txt"), None).unwrap();
    assert_eq!(
        blame.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta updated", "gamma"]
    );

    // Lines unchanged since `base` are still attributed to the pre-rename commit,
    // and carry the file's historical name (`src/old.txt`) so navigating to that
    // commit uses a path that actually exists in its tree.
    assert_eq!(&*blame[0].commit_id, base_id);
    assert_eq!(
        blame[0].source_path.as_deref(),
        Some(Path::new("src/old.txt"))
    );
    assert_eq!(&*blame[2].commit_id, base_id);
    assert_eq!(
        blame[2].source_path.as_deref(),
        Some(Path::new("src/old.txt"))
    );

    // The tweaked line originates in the rename commit under the current name, so
    // there is no distinct historical path.
    assert_ne!(&*blame[1].commit_id, base_id);
    assert_eq!(blame[1].source_path, None);

    // The historical path resolves at the base commit, where the current path does
    // not exist yet — this is exactly what the navigation relies on.
    let historical = opened
        .blame_file(Path::new("src/old.txt"), Some(base_id.as_str()))
        .unwrap();
    assert_eq!(
        historical
            .iter()
            .map(|l| l.line.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "gamma"]
    );
    assert!(
        opened
            .blame_file(Path::new("lib/new.txt"), Some(base_id.as_str()))
            .is_err(),
        "current path must not exist in the pre-rename tree"
    );
}

#[test]
fn blame_disables_rename_following_when_configured() {
    let (dir, base_id) = rename_repo();
    let repo = dir.path();
    // Explicitly turn rename detection off; the blame must then treat the rename
    // commit as introducing the file, attributing every line to it.
    run_git(repo, &["config", "diff.renames", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let blame = opened.blame_file(Path::new("lib/new.txt"), None).unwrap();
    assert!(
        blame.iter().all(|l| l.commit_id.as_ref() != base_id),
        "with renames disabled no line should reach the pre-rename commit"
    );
    assert!(
        blame.iter().all(|l| l.source_path.is_none()),
        "with renames disabled there is no historical path to surface"
    );
}

#[test]
fn blame_worktree_synthesizes_local_blame_for_newly_added_file() {
    use gitcomet_core::domain::DiffArea;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // A brand-new untracked file is absent from HEAD, so `git blame` would fail
    // with "no such path ... in HEAD". Every line must come back as a local
    // ("Not Committed Yet") entry rather than erroring.
    std::fs::write(repo.join("gaps.md"), "alpha\nbeta\n").unwrap();
    let untracked = opened
        .blame_worktree_file(Path::new("gaps.md"), DiffArea::Unstaged)
        .unwrap();
    assert_eq!(
        untracked
            .iter()
            .map(|l| l.line.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    for line in &untracked {
        assert_eq!(&*line.commit_id, "0000000000000000000000000000000000000000");
        assert!(!line.prior_exists);
        assert_eq!(line.prior_commit, None);
    }

    // Staging the new file still leaves it absent from HEAD; blaming the staged
    // area synthesizes from the staged blob.
    run_git(repo, &["add", "gaps.md"]);
    let staged = opened
        .blame_worktree_file(Path::new("gaps.md"), DiffArea::Staged)
        .unwrap();
    assert_eq!(
        staged.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    assert_eq!(
        &*staged[0].commit_id,
        "0000000000000000000000000000000000000000"
    );
}

#[test]
fn blame_worktree_surfaces_historical_path_after_rename() {
    use gitcomet_core::domain::DiffArea;

    let (dir, base_id) = rename_repo();
    let repo = dir.path();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Working-tree blame of the clean file: lines unchanged since `base` are
    // attributed to the pre-rename commit and now carry the file's historical
    // name (`src/old.txt`), mirroring the committed-revision blame path so
    // "view file at this commit" navigates to a name that exists in that tree.
    let blame = opened
        .blame_worktree_file(Path::new("lib/new.txt"), DiffArea::Unstaged)
        .unwrap();
    assert_eq!(
        blame.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta updated", "gamma"]
    );
    assert_eq!(&*blame[0].commit_id, base_id);
    assert_eq!(
        blame[0].source_path.as_deref(),
        Some(Path::new("src/old.txt"))
    );
    assert_eq!(
        blame[2].source_path.as_deref(),
        Some(Path::new("src/old.txt"))
    );
    // The tweaked line originates under the current name post-rename.
    assert_eq!(blame[1].source_path, None);

    // An uncommitted edit has no committed history, so it carries no historical
    // path (exercises the uncommitted branch of the porcelain parser).
    std::fs::write(
        repo.join("lib/new.txt"),
        "alpha\nbeta updated\ngamma\nnew tail\n",
    )
    .unwrap();
    let dirty = opened
        .blame_worktree_file(Path::new("lib/new.txt"), DiffArea::Unstaged)
        .unwrap();
    assert_eq!(dirty.len(), 4);
    assert_eq!(dirty[3].line, "new tail");
    assert_eq!(dirty[3].source_path, None);
    assert_eq!(
        dirty[0].source_path.as_deref(),
        Some(Path::new("src/old.txt"))
    );
}

#[test]
fn resolve_file_path_at_commit_follows_renames_both_directions() {
    use gitcomet_core::domain::CommitId;
    use std::sync::Arc;

    let (dir, base_id) = rename_repo();
    let repo = dir.path();
    let rename_id = git_stdout(repo, &["rev-parse", "HEAD~1"]); // the pure-rename commit
    let head_id = git_stdout(repo, &["rev-parse", "HEAD"]); // tweak, file is lib/new.txt

    let opened = GixBackend.open(repo).unwrap();
    let cid = |s: &str| CommitId(Arc::from(s));

    // Fast path: the current name exists in the target commit's tree.
    assert_eq!(
        opened
            .resolve_file_path_at_commit(Path::new("lib/new.txt"), &cid(&head_id))
            .unwrap(),
        Some(Path::new("lib/new.txt").to_path_buf())
    );

    // Backwards: the current name resolves to the pre-rename name at an older commit.
    assert_eq!(
        opened
            .resolve_file_path_at_commit(Path::new("lib/new.txt"), &cid(&base_id))
            .unwrap(),
        Some(Path::new("src/old.txt").to_path_buf())
    );

    // Forwards across the rename boundary: viewing the old name, navigating to the
    // commit that renamed it must resolve to the new name (which exists there).
    assert_eq!(
        opened
            .resolve_file_path_at_commit(Path::new("src/old.txt"), &cid(&rename_id))
            .unwrap(),
        Some(Path::new("lib/new.txt").to_path_buf())
    );

    // Unrelated path with no mapping resolves to nothing (caller falls back).
    assert_eq!(
        opened
            .resolve_file_path_at_commit(Path::new("does/not/exist.txt"), &cid(&head_id))
            .unwrap(),
        None
    );
}

#[test]
fn blame_worktree_staged_succeeds_for_conflicted_file() {
    use gitcomet_core::domain::DiffArea;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("file.txt"), "line1\nline2\nline3\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    // Two divergent edits to the same line conflict on merge, leaving the file
    // unmerged with stages 1/2/3 and no stage-0 entry.
    run_git(repo, &["checkout", "-b", "theirs"]);
    std::fs::write(repo.join("file.txt"), "line1\ntheirs\nline3\n").unwrap();
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "theirs edit"],
    );
    run_git(repo, &["checkout", "main"]);
    std::fs::write(repo.join("file.txt"), "line1\nours\nline3\n").unwrap();
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "ours edit"],
    );

    // The merge conflicts; do not assert success.
    let mut merge = Command::new("git");
    test_git_env::apply(&mut merge);
    let _ = merge
        .arg("-C")
        .arg(repo)
        .args(["merge", "theirs"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git merge to run");

    let opened = GixBackend.open(repo).unwrap();
    // Regression: the staged side has no stage-0 entry for a conflicted file, so
    // blame must fall back to the "ours" stage rather than erroring with
    // "no staged content".
    let staged = opened
        .blame_worktree_file(Path::new("file.txt"), DiffArea::Staged)
        .expect("staged blame on a conflicted file must not error");
    assert_eq!(
        staged.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
        vec!["line1", "ours", "line3"],
        "staged blame falls back to the 'ours' stage content"
    );
}

#[test]
fn blame_file_folds_multiline_subject_and_preserves_body() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    // A commit whose subject paragraph spans two physical lines before the blank
    // separator, plus a body. Committed verbatim so the message is stored exactly.
    std::fs::write(repo.join("a.txt"), "content\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    let lf_msg = repo.join("LF_MSG");
    std::fs::write(
        &lf_msg,
        "subject one\nsubject two\n\nbody line A\nbody line B\n",
    )
    .unwrap();
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--cleanup=verbatim",
            "-F",
            lf_msg.to_str().unwrap(),
        ],
    );

    // A commit using CRLF paragraph separators — the old hand-rolled `\n\n` scan
    // never matched `\r\n\r\n` and dropped the body entirely.
    std::fs::write(repo.join("b.txt"), "x\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    let crlf_msg = repo.join("CRLF_MSG");
    std::fs::write(&crlf_msg, "subject crlf\r\n\r\nbody crlf line\r\n").unwrap();
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--cleanup=verbatim",
            "-F",
            crlf_msg.to_str().unwrap(),
        ],
    );

    let opened = GixBackend.open(repo).unwrap();

    // git folds a multi-line subject into one summary line; the middle line must
    // survive (the old split lost it), and the body is the text after the blank.
    let blame_a = opened.blame_file(Path::new("a.txt"), None).unwrap();
    assert_eq!(blame_a.len(), 1);
    assert_eq!(blame_a[0].summary.as_ref(), "subject one subject two");
    assert_eq!(blame_a[0].body.as_deref(), Some("body line A\nbody line B"));

    // CRLF separators are recognized, so the body is preserved rather than lost.
    let blame_b = opened.blame_file(Path::new("b.txt"), None).unwrap();
    assert_eq!(blame_b.len(), 1);
    assert_eq!(blame_b[0].summary.as_ref(), "subject crlf");
    assert_eq!(blame_b[0].body.as_deref(), Some("body crlf line"));
}
