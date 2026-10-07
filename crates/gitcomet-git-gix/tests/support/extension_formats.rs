use gitcomet_core::domain::{CommitId, DiffArea, DiffLineKind, DiffTarget, HistoryMode};
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
    let target = DiffTarget::working_tree("file.txt".into(), DiffArea::Unstaged);
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
                repo.diff_unified(&DiffTarget::commit_range(
                    empty.clone(),
                    Some(head.clone()),
                    path
                ))
                .unwrap()
                .contains("+root")
            );
        }
        assert!(
            repo.diff_unified(&DiffTarget::commit(head.clone(), "file.txt".into()))
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
    repo.submodule_diff_summary(&DiffTarget::working_tree(
        "module".into(),
        DiffArea::Unstaged,
    ))
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

fn has_added_line(diff: &gitcomet_core::domain::Diff, text: &str) -> bool {
    diff.lines
        .iter()
        .any(|line| line.kind == DiffLineKind::Add && line.text.as_ref() == text)
}

/// Status, parsed diffs, a backend commit git can read, indexed history, reflog
/// and file history all carry ids of the repository's width.
pub fn status_diff_commit_reflog_and_file_history(hash: &str, refs: &str) {
    let dir = fixture(hash, refs);
    let width = if hash == "sha256" { 64 } else { 40 };
    fs::write(dir.path().join("notes.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "first"]);
    let first = git(dir.path(), &["rev-parse", "HEAD"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    assert!(repo.status().unwrap().unstaged.is_empty());

    fs::write(dir.path().join("notes.txt"), "one\ntwo\n").unwrap();
    let worktree = DiffTarget::working_tree("notes.txt".into(), DiffArea::Unstaged);
    assert!(has_added_line(
        &repo.diff_parsed(&worktree).unwrap(),
        "+two"
    ));
    repo.stage(&[Path::new("notes.txt")]).unwrap();
    repo.commit("second").unwrap();
    let second = repo.head_commit_id().unwrap().unwrap();
    assert_eq!(second.as_ref().len(), width);
    assert_eq!(second.as_ref(), git(dir.path(), &["rev-parse", "HEAD"]));
    assert_eq!(
        git(dir.path(), &["cat-file", "-t", second.as_ref()]),
        "commit"
    );
    let status = repo.status().unwrap();
    assert!(status.staged.is_empty() && status.unstaged.is_empty());
    let commit = DiffTarget::commit(second.clone(), "notes.txt".into());
    assert!(has_added_line(&repo.diff_parsed(&commit).unwrap(), "+two"));

    let cancellation = CancellationToken::new();
    let index = repo
        .build_history_index(HistoryMode::FullReachable, None, &cancellation, &mut |_| {})
        .unwrap()
        .unwrap();
    let range = repo
        .read_history_range(&index, 0..2, &cancellation)
        .unwrap();
    assert_eq!(range.commits[0].id, second);
    assert_eq!(range.commits[0].parent_ids[0].as_ref(), first);
    assert!(range.commits[1].parent_ids.is_empty());

    let details = repo.commit_details(&second).unwrap();
    assert_eq!(details.parent_ids[0].as_ref(), first);
    let reflog = repo.reflog_head(10).unwrap();
    assert_eq!(reflog[0].new_id, second);
    assert!(
        reflog
            .iter()
            .all(|entry| entry.new_id.as_ref().len() == width)
    );
    let file = repo
        .log_file_page(Path::new("notes.txt"), 10, None)
        .unwrap();
    let ids: Vec<_> = file.commits.iter().map(|c| c.id.as_ref()).collect();
    assert_eq!(ids, [second.as_ref(), first.as_str()]);
}

/// gix infers a hex name's hash kind from its length, so every abbreviation
/// form must resolve against the repository's own format, loose and packed.
pub fn revision_lookups(hash: &str, refs: &str) {
    let dir = fixture(hash, refs);
    fs::write(dir.path().join("file.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "first"]);
    git(dir.path(), &["tag", "-a", "v1", "-m", "v1"]);
    let first = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "second"]);
    let second = git(dir.path(), &["rev-parse", "HEAD"]);
    let described = git(dir.path(), &["describe", "--long"]);
    let resolve = |repo: &dyn gitcomet_core::services::GitRepository, spec: &str| {
        repo.resolve_commit(&CommitId(spec.into()))
            .map(|commit| commit.id.as_ref().to_owned())
    };
    for packed in [false, true] {
        if packed {
            git(dir.path(), &["gc", "-q"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        let repo = repo.as_ref();
        for spec in [
            second.clone(),
            second[..12].to_owned(),
            second[..13].to_owned(),
            second[..40].to_owned(),
            second[..12].to_uppercase(),
            second.to_uppercase(),
            described.clone(),
            format!("{}^{{commit}}", &second[..12]),
        ] {
            let resolved = resolve(repo, &spec);
            assert_eq!(
                resolved.as_deref().ok(),
                Some(second.as_str()),
                "{hash}/{refs} packed={packed} {spec}"
            );
        }
        assert_eq!(
            resolve(repo, &format!("{}~1", &second[..12])).unwrap(),
            first,
            "{hash}/{refs} packed={packed}"
        );
        assert!(resolve(repo, &"f".repeat(12)).is_err());
    }

    // Find a four-digit prefix collision in memory and store only its two blobs.
    // Writing every candidate exhausts temporary filesystem inodes unnecessarily.
    let object_hash = gix::ObjectId::from_hex(second.as_bytes()).unwrap().kind();
    let mut prefixes = std::collections::HashMap::new();
    // There are 65,536 four-digit prefixes, so one more candidate guarantees a collision.
    let (first_blob, second_blob, ambiguous) = (0..=65_536)
        .find_map(|i| {
            let contents = format!("blob {i}\n");
            let id =
                gix::objs::compute_hash(object_hash, gix::objs::Kind::Blob, contents.as_bytes())
                    .unwrap();
            let prefix = [id.as_bytes()[0], id.as_bytes()[1]];
            prefixes
                .insert(prefix, i)
                .map(|first| (first, i, id.to_string()[..4].to_owned()))
        })
        .expect("find a four-digit blob/blob collision");
    let blob_file = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let ids = [first_blob, second_blob].map(|i| {
        fs::write(blob_file.path(), format!("blob {i}\n")).unwrap();
        git(
            dir.path(),
            &[
                "hash-object",
                "-w",
                "--no-filters",
                blob_file.path().to_str().unwrap(),
            ],
        )
    });
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|id| id.starts_with(&ambiguous)));
    let repo = GixBackend.open(dir.path()).unwrap();
    let error = resolve(repo.as_ref(), &ambiguous).unwrap_err().to_string();
    assert!(
        error.to_lowercase().contains("ambiguous"),
        "{hash}/{refs} {error}"
    );
}
