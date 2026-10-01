//! Comparison services against real repositories: renames in commits and
//! ranges (listing and loading), merge-base comparisons, ancestry, and
//! opt-in untracked files.

use gitcomet_core::domain::{
    CommitFileChange, CommitId, DiffTarget, FileMode, FileStatusKind, ObjectHash,
};
use gitcomet_core::services::{CancellationToken, ComparisonOptions, GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.email", "you@example.com"),
        ("user.name", "You"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(repo, &["config", key, value]);
    }
}

fn commit_all(repo: &Path, message: &str) -> CommitId {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
    CommitId(git(repo, &["rev-parse", "HEAD"]).into())
}

fn open(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

/// Enough lines that a rename with one edited line stays above Git's
/// similarity threshold.
fn body(lines: usize, tail: &str) -> String {
    let mut text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
    text.push_str(tail);
    text
}

fn change<'a>(files: &'a [CommitFileChange], path: &str) -> &'a CommitFileChange {
    files
        .iter()
        .find(|file| file.path == Path::new(path))
        .unwrap_or_else(|| panic!("{path} in {files:?}"))
}

fn old_side_text(repo: &dyn GitRepository, target: &DiffTarget) -> Option<String> {
    let text = repo
        .diff_file_text(target)
        .expect("load file text")
        .expect("a file target has text");
    text.old_source
        .map(|source| fs::read_to_string(source.path).expect("read old side"))
}

#[test]
fn a_renamed_file_lists_its_source_ids_and_modes_and_loads_its_old_side() {
    test_git_env::ensure_initialized();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init(&repo);
    fs::write(repo.join("old.txt"), body(20, "tail\n")).unwrap();
    let base = commit_all(&repo, "base");
    git(&repo, &["mv", "old.txt", "new.txt"]);
    fs::write(repo.join("new.txt"), body(20, "edited tail\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(repo.join("new.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let renamed = commit_all(&repo, "rename");
    let opened = open(&repo);

    let details = opened.commit_details(&renamed).expect("commit details");
    let file = change(&details.files, "new.txt");
    assert_eq!(file.kind, FileStatusKind::Renamed);
    assert_eq!(file.old_path.as_deref(), Some(Path::new("old.txt")));
    let old_blob = git(&repo, &["rev-parse", &format!("{base}:old.txt")]);
    assert_eq!(file.old_id, Some(ObjectHash(old_blob.into())));
    assert!(file.new_id.is_some() && file.new_id != file.old_id);
    assert_eq!(file.old_mode, Some(FileMode::Regular));
    #[cfg(unix)]
    assert_eq!(file.new_mode, Some(FileMode::Executable));

    // The old side loads from the source path, not as an empty addition.
    let target = DiffTarget::commit_change(renamed.clone(), file);
    assert_eq!(target.old_file_path(), Some(Path::new("old.txt")));
    assert_eq!(
        old_side_text(opened.as_ref(), &target).as_deref(),
        Some(body(20, "tail\n").as_str())
    );
    let unified = opened.diff_unified(&target).expect("unified diff");
    assert!(unified.contains("rename from old.txt"), "{unified}");
    assert!(unified.contains("+edited tail"), "{unified}");

    // Without the source the same file is an addition, which is what the
    // old code showed for every rename.
    let bare = DiffTarget::commit(renamed.clone(), PathBuf::from("new.txt"));
    assert_eq!(old_side_text(opened.as_ref(), &bare), None);
    assert_eq!(bare, target, "equality ignores the derived rename source");

    // Ranges carry the rename too.
    let range = opened
        .compare_files(
            &base,
            Some(&renamed),
            &ComparisonOptions::direct(),
            &CancellationToken::new(),
        )
        .expect("compare");
    let file = change(&range.files, "new.txt");
    assert_eq!(file.old_path.as_deref(), Some(Path::new("old.txt")));
    let target = DiffTarget::commit_range(base.clone(), Some(renamed), None).for_change(file);
    assert_eq!(
        old_side_text(opened.as_ref(), &target).as_deref(),
        Some(body(20, "tail\n").as_str())
    );
}

#[test]
fn a_worktree_comparison_keeps_renames_and_lists_untracked_files_only_when_asked() {
    test_git_env::ensure_initialized();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init(&repo);
    fs::write(repo.join("old.txt"), body(20, "tail\n")).unwrap();
    let base = commit_all(&repo, "base");
    git(&repo, &["mv", "old.txt", "moved.txt"]);
    fs::write(repo.join("scratch.txt"), "one\ntwo\n").unwrap();
    let opened = open(&repo);
    let cancel = CancellationToken::new();

    let tracked = opened
        .compare_files(&base, None, &ComparisonOptions::direct(), &cancel)
        .expect("compare with the working tree");
    let moved = change(&tracked.files, "moved.txt");
    assert_eq!(moved.kind, FileStatusKind::Renamed);
    assert_eq!(moved.old_path.as_deref(), Some(Path::new("old.txt")));
    assert!(
        tracked
            .files
            .iter()
            .all(|file| file.path != Path::new("scratch.txt")),
        "untracked files stay out by default"
    );
    // The existing call keeps its behavior.
    assert_eq!(opened.diff_range_files(&base, None).unwrap(), tracked.files);

    let with_untracked = opened
        .compare_files(
            &base,
            None,
            &ComparisonOptions::direct().with_untracked(true),
            &cancel,
        )
        .expect("compare with untracked files");
    let scratch = change(&with_untracked.files, "scratch.txt");
    assert_eq!(scratch.kind, FileStatusKind::Untracked);
    assert_eq!((scratch.additions, scratch.deletions), (Some(2), Some(0)));
}

#[test]
fn merge_base_comparisons_show_only_what_the_branch_added() {
    test_git_env::ensure_initialized();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init(&repo);
    fs::write(repo.join("shared.txt"), "shared\n").unwrap();
    let fork = commit_all(&repo, "fork point");
    git(&repo, &["checkout", "-q", "-b", "feature"]);
    fs::write(repo.join("feature.txt"), "feature\n").unwrap();
    let feature = commit_all(&repo, "feature work");
    git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("main.txt"), "main\n").unwrap();
    let main = commit_all(&repo, "main work");
    let opened = open(&repo);
    let cancel = CancellationToken::new();

    assert_eq!(
        opened.merge_base(&main, &feature).unwrap(),
        Some(fork.clone())
    );
    assert!(opened.is_ancestor(&fork, &feature).unwrap());
    assert!(opened.is_ancestor(&feature, &feature).unwrap());
    assert!(!opened.is_ancestor(&main, &feature).unwrap());

    let direct = opened
        .compare_files(&main, Some(&feature), &ComparisonOptions::direct(), &cancel)
        .unwrap();
    let mut direct_paths: Vec<_> = direct.files.iter().map(|file| file.path.clone()).collect();
    direct_paths.sort();
    assert_eq!(
        direct_paths,
        vec![PathBuf::from("feature.txt"), PathBuf::from("main.txt")],
        "a direct comparison also undoes what main added"
    );
    assert_eq!(direct.base, main);

    let since_fork = opened
        .compare_files(
            &main,
            Some(&feature),
            &ComparisonOptions::merge_base(),
            &cancel,
        )
        .unwrap();
    assert_eq!(since_fork.base, fork);
    let paths: Vec<_> = since_fork
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(paths, vec![PathBuf::from("feature.txt")]);
}

#[test]
fn unrelated_histories_have_no_merge_base() {
    test_git_env::ensure_initialized();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init(&repo);
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    let first = commit_all(&repo, "first root");
    git(&repo, &["checkout", "-q", "--orphan", "other"]);
    git(&repo, &["rm", "-q", "-rf", "."]);
    fs::write(repo.join("b.txt"), "b\n").unwrap();
    let second = commit_all(&repo, "second root");
    let opened = open(&repo);

    assert_eq!(opened.merge_base(&first, &second).unwrap(), None);
    assert!(!opened.is_ancestor(&first, &second).unwrap());
    let error = opened
        .compare_files(
            &first,
            Some(&second),
            &ComparisonOptions::merge_base(),
            &CancellationToken::new(),
        )
        .expect_err("no merge base to compare from");
    assert!(error.to_string().contains("no merge base"), "{error}");
}
