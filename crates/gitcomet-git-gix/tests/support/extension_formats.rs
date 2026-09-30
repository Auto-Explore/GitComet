use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, HistoryMode};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "test_git_env.rs"]
mod test_git_env;

pub fn git(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1800000000 +0530");
    let output = cmd.arg("-C").arg(repo).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub fn reftable_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("git");
        test_git_env::apply(&mut cmd);
        let available = cmd
            .arg("-C")
            .arg(dir.path())
            .args(["init", "--ref-format=reftable"])
            .output()
            .unwrap()
            .status
            .success();
        if !available {
            eprintln!("skipping reftable integration: Git has no reftable support");
        }
        available
    })
}

pub fn fixture(hash: &str, refs: &str) -> tempfile::TempDir {
    if refs == "reftable" {
        assert!(reftable_available());
    }
    let dir = tempfile::tempdir().unwrap();
    git(
        dir.path(),
        &[
            "init",
            &format!("--object-format={hash}"),
            &format!("--ref-format={refs}"),
            "--initial-branch=main",
        ],
    );
    git(dir.path(), &["config", "user.name", "Format Test"]);
    git(
        dir.path(),
        &["config", "user.email", "format@example.invalid"],
    );
    dir
}

pub fn open_stage_commit_and_uncommitted_blame(hash: &str, refs: &str) {
    let dir = fixture(hash, refs);
    let width = if hash == "sha256" { 64 } else { 40 };
    let repo = GixBackend
        .open(dir.path())
        .expect("open SHA-256 repository");
    assert_eq!(repo.current_branch().unwrap(), "main");
    assert!(repo.log_head_page(10, None).unwrap().commits.is_empty());
    fs::write(dir.path().join("file.txt"), "first\n").unwrap();
    assert!(!repo.status().unwrap().unstaged.is_empty());
    let blame = repo
        .blame_worktree_file(Path::new("file.txt"), DiffArea::Unstaged)
        .unwrap();
    assert_eq!(&*blame[0].commit_id, "0".repeat(width));
    repo.stage(&[Path::new("file.txt")]).unwrap();
    assert_eq!(repo.status().unwrap().staged.len(), 1);
    repo.unstage(&[Path::new("file.txt")]).unwrap();
    assert!(repo.status().unwrap().staged.is_empty());
    repo.stage(&[Path::new("file.txt")]).unwrap();
    repo.commit("root").unwrap();
    let page = repo.log_head_page(10, None).unwrap();
    let commits = &page.commits;
    assert_eq!(commits[0].id.as_ref().len(), width);
    for width in [12, 40] {
        let prefix = CommitId(commits[0].id.as_ref()[..width].into());
        assert_eq!(repo.commit_details(&prefix).unwrap().message.trim(), "root");
    }
    assert_eq!(repo.list_branches().unwrap()[0].target, commits[0].id);
    assert_eq!(
        repo.blame_file(Path::new("file.txt"), Some(commits[0].id.as_ref()))
            .unwrap()[0]
            .commit_id
            .as_ref(),
        commits[0].id.as_ref()
    );
    fs::write(dir.path().join("file.txt"), "first\nsecond\n").unwrap();
    let target = DiffTarget::WorkingTree {
        path: "file.txt".into(),
        area: DiffArea::Unstaged,
    };
    assert!(repo.diff_unified(&target).unwrap().contains("+second"));
    assert!(repo.diff_file_text(&target).unwrap().is_some());
    repo.stash_create("sha256 stash", false).unwrap();
    assert_eq!(repo.stash_list().unwrap().len(), 1);
    repo.stash_apply(0).unwrap();
    repo.stash_drop(0).unwrap();
    assert!(repo.stash_list().unwrap().is_empty());
}

pub fn history_loose_commit_graph_and_packs(hash: &str, refs: &str) {
    let dir = fixture(hash, refs);
    fs::write(dir.path().join("file.txt"), "root\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "root"]);
    let root = CommitId(git(dir.path(), &["rev-parse", "HEAD"]).into());
    fs::write(dir.path().join("file.txt"), "root\nchild\n").unwrap();
    git(dir.path(), &["commit", "-am", "child"]);
    let head = CommitId(git(dir.path(), &["rev-parse", "HEAD"]).into());
    for storage in 0..3 {
        if storage == 1 {
            git(dir.path(), &["commit-graph", "write", "--reachable"]);
        }
        if storage == 2 {
            git(dir.path(), &["gc"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        let page = repo.log_head_page(10, None).unwrap();
        assert_eq!(page.commits.len(), 2);
        assert_eq!(
            page.commits[0].parent_ids.as_slice(),
            std::slice::from_ref(&root)
        );
        let index = repo
            .build_history_index(
                HistoryMode::AllBranches,
                None,
                &CancellationToken::new(),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        assert_eq!(index.commit_id(0), Some(head.clone()));
        assert_eq!(index.parent_commit_id(0, 0), Some(root.clone()));
        let empty = gitcomet_core::domain::empty_tree_id_like(&root).unwrap();
        assert_eq!(repo.diff_range_files(&empty, Some(&head)).unwrap().len(), 1);
        for path in [None, Some("file.txt".into())] {
            assert!(
                repo.diff_unified(&DiffTarget::CommitRange {
                    from_commit_id: empty.clone(),
                    to_commit_id: Some(head.clone()),
                    path
                })
                .unwrap()
                .contains("+root")
            );
        }
        assert!(
            repo.diff_unified(&DiffTarget::Commit {
                commit_id: head.clone(),
                path: None
            })
            .unwrap()
            .contains("+child")
        );
    }
}

pub fn submodules_and_unconfigured_gitlinks(hash: &str, refs: &str) {
    let child = fixture(hash, refs);
    fs::write(child.path().join("child.txt"), "child\n").unwrap();
    git(child.path(), &["add", "."]);
    git(child.path(), &["commit", "-m", "child"]);
    let child_id = git(child.path(), &["rev-parse", "HEAD"]);
    let parent = fixture(hash, refs);
    git(
        parent.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &format!("--ref-format={refs}"),
            child.path().to_str().unwrap(),
            "module",
        ],
    );
    git(parent.path(), &["commit", "-m", "submodule"]);
    let repo = GixBackend.open(parent.path()).unwrap();
    let modules = repo.list_submodules().unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].recorded_head.as_ref(), child_id);
    assert_eq!(
        modules[0].checked_out_head.as_ref().unwrap().as_ref(),
        child_id
    );
    assert!(repo.status().unwrap().unstaged.is_empty());
    fs::write(parent.path().join("module/child.txt"), "edited\n").unwrap();
    assert_eq!(repo.status().unwrap().unstaged.len(), 1);
    repo.submodule_diff_summary(&DiffTarget::WorkingTree {
        path: "module".into(),
        area: DiffArea::Unstaged,
    })
    .unwrap();
    GixBackend.repository_watch_info(parent.path()).unwrap();
    // An index gitlink alone must not cause gix to consult the placeholder HEAD.
    let unconfigured = fixture(hash, refs);
    git(
        unconfigured.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{child_id},missing-module"),
        ],
    );
    let repo = GixBackend.open(unconfigured.path()).unwrap();
    assert_eq!(repo.status().unwrap().staged.len(), 1);
}
