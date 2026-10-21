use super::*;
use crate::test_support::{git_stdout, git_success, init_test_repo, open_repo, write_file};
use std::fs::{self, File, FileTimes};
use std::time::{Duration, SystemTime};

fn pattern(source: &str, pattern: &str) -> LfsTrackedPattern {
    LfsTrackedPattern {
        pattern: pattern.to_owned(),
        source: PathBuf::from(source),
    }
}

fn support_without_index_write(repo: &super::super::GixRepo, index: &Path) -> LargeFileSupport {
    let before = fs::read(index).unwrap();
    let modified = fs::metadata(index).unwrap().modified().unwrap();
    let support = repo
        .large_file_support_impl(&CancellationToken::new())
        .unwrap();
    assert_eq!(fs::read(index).unwrap(), before);
    assert_eq!(fs::metadata(index).unwrap().modified().unwrap(), modified);
    support
}

fn replace_preserving_size_and_mtime(path: &Path, contents: &str) {
    let metadata = fs::metadata(path).unwrap();
    let modified = metadata.modified().unwrap();
    assert_eq!(metadata.len(), contents.len() as u64);
    fs::write(path, contents).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let after = fs::metadata(path).unwrap();
    assert_eq!(after.len(), metadata.len());
    assert_eq!(after.modified().unwrap(), modified);
    assert_eq!(fs::read_to_string(path).unwrap(), contents);
}

// These cases exercise a genuinely new parsed-index snapshot, not the separate
// same/backdated-index-mtime freshness limitation inherited from gix.
fn advance_index_mtime(index: &Path, previous: SystemTime) -> SystemTime {
    let current = fs::metadata(index).unwrap().modified().unwrap();
    let next = current.max(previous) + Duration::from_secs(2);
    File::options()
        .write(true)
        .open(index)
        .unwrap()
        .set_times(FileTimes::new().set_modified(next))
        .unwrap();
    let actual = fs::metadata(index).unwrap().modified().unwrap();
    assert_eq!(actual, next);
    assert!(actual > previous);
    actual
}

#[test]
fn repeated_support_reads_same_size_same_mtime_tracked_attribute_edits() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(
        workdir,
        ".gitattributes",
        "*.old filter=lfs\n*.bin lockable\n",
    );
    write_file(workdir, "nested/.gitattributes", "*.child filter=lfs\n");
    git_success(workdir, &["add", ".gitattributes", "nested/.gitattributes"]);
    git_success(workdir, &["commit", "-m", "attributes"]);
    let repo = open_repo(workdir);
    let index = workdir.join(".git/index");
    let initial = support_without_index_write(&repo, &index);
    assert_eq!(
        initial.lfs.tracked_patterns,
        vec![
            pattern(".gitattributes", "*.old"),
            pattern("nested/.gitattributes", "*.child"),
        ]
    );
    assert!(initial.lfs.has_lockable_patterns);

    replace_preserving_size_and_mtime(
        &workdir.join(".gitattributes"),
        "*.new filter=lfs\n*.bin lockable\n",
    );
    let updated = support_without_index_write(&repo, &index);
    assert_eq!(
        updated.lfs.tracked_patterns,
        vec![
            pattern(".gitattributes", "*.new"),
            pattern("nested/.gitattributes", "*.child"),
        ]
    );
    assert!(updated.lfs.has_lockable_patterns);
}

#[test]
fn support_tracks_added_removed_sources_and_changed_index_blob_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, "seed.txt", "seed\n");
    git_success(workdir, &["add", "seed.txt"]);
    git_success(workdir, &["commit", "-m", "seed"]);
    let repo = open_repo(workdir);
    let index = workdir.join(".git/index");
    let mut modified = fs::metadata(&index).unwrap().modified().unwrap();
    assert!(
        support_without_index_write(&repo, &index)
            .lfs
            .tracked_patterns
            .is_empty()
    );

    write_file(workdir, "nested/.gitattributes", "*.old filter=lfs\n");
    git_success(workdir, &["add", "nested/.gitattributes"]);
    assert_eq!(
        git_stdout(workdir, &["show", ":nested/.gitattributes"]),
        "*.old filter=lfs"
    );
    modified = advance_index_mtime(&index, modified);
    fs::remove_file(workdir.join("nested/.gitattributes")).unwrap();
    assert_eq!(
        support_without_index_write(&repo, &index)
            .lfs
            .tracked_patterns,
        vec![pattern("nested/.gitattributes", "*.old")]
    );

    // A different worktree size makes Git observe the content replacement even
    // on a filesystem whose timestamps have coarse precision.
    write_file(workdir, "nested/.gitattributes", "*.changed filter=lfs\n");
    git_success(workdir, &["add", "nested/.gitattributes"]);
    assert_eq!(
        git_stdout(workdir, &["show", ":nested/.gitattributes"]),
        "*.changed filter=lfs"
    );
    modified = advance_index_mtime(&index, modified);
    fs::remove_file(workdir.join("nested/.gitattributes")).unwrap();
    assert_eq!(
        support_without_index_write(&repo, &index)
            .lfs
            .tracked_patterns,
        vec![pattern("nested/.gitattributes", "*.changed")]
    );

    git_success(workdir, &["rm", "--cached", "--", "nested/.gitattributes"]);
    advance_index_mtime(&index, modified);
    assert!(
        support_without_index_write(&repo, &index)
            .lfs
            .tracked_patterns
            .is_empty()
    );
}

#[test]
fn support_reads_linked_worktree_untracked_root_and_common_info_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    let linked = dir.path().join("linked");
    fs::create_dir(&main).unwrap();
    init_test_repo(&main);
    write_file(&main, "seed.txt", "seed\n");
    git_success(&main, &["add", "seed.txt"]);
    git_success(&main, &["commit", "-m", "seed"]);
    git_success(
        &main,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    assert!(linked.join(".git").is_file());
    write_file(&main, ".gitattributes", "*.main filter=lfs\n");
    write_file(&linked, ".gitattributes", "*.root filter=lfs\n");
    write_file(
        &main,
        ".git/info/attributes",
        "*.info filter=lfs\n*.bin lockable\n",
    );
    let repo = open_repo(&linked);
    let index = repo.repo().index_path();
    let initial = support_without_index_write(&repo, &index);
    assert_eq!(
        initial.lfs.tracked_patterns,
        vec![
            pattern(".gitattributes", "*.root"),
            pattern(".git/info/attributes", "*.info"),
        ]
    );
    assert!(initial.lfs.has_lockable_patterns);

    replace_preserving_size_and_mtime(&linked.join(".gitattributes"), "*.next filter=lfs\n");
    replace_preserving_size_and_mtime(
        &main.join(".git/info/attributes"),
        "*.more filter=lfs\n*.bin lockable\n",
    );
    let updated = support_without_index_write(&repo, &index);
    assert_eq!(
        updated.lfs.tracked_patterns,
        vec![
            pattern(".gitattributes", "*.next"),
            pattern(".git/info/attributes", "*.more"),
        ]
    );
    assert!(updated.lfs.has_lockable_patterns);
}

#[test]
fn support_pre_cancellation_is_an_error_before_and_after_a_successful_read() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, ".gitattributes", "*.bin filter=lfs\n");
    git_success(workdir, &["add", ".gitattributes"]);
    git_success(workdir, &["commit", "-m", "attributes"]);
    let repo = open_repo(workdir);
    let index = workdir.join(".git/index");
    let before = fs::read(&index).unwrap();
    let modified = fs::metadata(&index).unwrap().modified().unwrap();
    for warm in [false, true] {
        if warm {
            support_without_index_write(&repo, &index);
        }
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = repo.large_file_support_impl(&cancellation).unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Cancelled));
        assert_eq!(fs::read(&index).unwrap(), before);
        assert_eq!(fs::metadata(&index).unwrap().modified().unwrap(), modified);
    }
}
