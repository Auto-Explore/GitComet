use super::*;
use gitcomet_core::domain::UpstreamDivergence;
use std::fs;

fn git_success(workdir: &Path, args: &[&str]) {
    let mut cmd = crate::util::git_workdir_cmd_for(workdir);
    let output = cmd.args(args).output().expect("spawn git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(workdir: &Path, args: &[&str]) -> String {
    let mut cmd = crate::util::git_workdir_cmd_for(workdir);
    let output = cmd.args(args).output().expect("spawn git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("utf8 stdout")
        .trim()
        .to_string()
}

fn init_test_repo(workdir: &Path) {
    git_success(workdir, &["init"]);
    for args in [
        ["config", "core.autocrlf", "false"].as_slice(),
        ["config", "core.eol", "lf"].as_slice(),
        ["config", "commit.gpgsign", "false"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
        ["config", "user.email", "test@example.com"].as_slice(),
    ] {
        git_success(workdir, args);
    }
}

fn write_file(workdir: &Path, relative: &str, contents: &str) {
    let path = workdir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

fn commit_file(workdir: &Path, path: &str, contents: &str, message: &str) {
    write_file(workdir, path, contents);
    git_success(workdir, &["add", path]);
    git_success(workdir, &["commit", "-m", message]);
}

fn open_repo(workdir: &Path) -> GixRepo {
    let thread_safe_repo = gix::open(workdir).expect("open repo").into_sync();
    GixRepo::new(workdir.to_path_buf(), thread_safe_repo)
}

#[test]
fn cursor_gate_skips_until_after_last_seen() {
    let cursor = LogCursor {
        last_seen: CommitId("c2".into()),
        resume_from: None,
        resume_token: None,
    };
    let mut gate = CursorGate::new(Some(&cursor));

    assert!(gate.should_skip("c1"));
    assert!(gate.should_skip("c2"));
    assert!(!gate.should_skip("c3"));
    assert!(!gate.should_skip("c4"));
}

#[test]
fn object_id_from_commit_id_rejects_invalid_hex() {
    assert!(object_id_from_commit_id(&CommitId("not-a-sha".into())).is_none());
}

#[test]
fn shallow_snapshot_uses_contents_even_when_stat_metadata_collides() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    let repo = open_repo(workdir);
    let local_repo = repo._repo.to_thread_local();
    let shallow_file = local_repo.shallow_file();

    fs::write(&shallow_file, b"1111111111111111111111111111111111111111\n")
        .expect("write first shallow boundary");
    let first_metadata = fs::metadata(&shallow_file).expect("first shallow metadata");
    let first_modified = first_metadata.modified().expect("first shallow mtime");
    let first = shallow_snapshot(&local_repo).expect("first shallow snapshot");

    fs::write(&shallow_file, b"2222222222222222222222222222222222222222\n")
        .expect("replace shallow boundary with the same length");
    fs::OpenOptions::new()
        .write(true)
        .open(&shallow_file)
        .expect("open shallow boundary")
        .set_times(fs::FileTimes::new().set_modified(first_modified))
        .expect("restore shallow mtime");

    let second_metadata = fs::metadata(&shallow_file).expect("second shallow metadata");
    assert_eq!(second_metadata.len(), first_metadata.len());
    assert_eq!(second_metadata.modified().ok(), Some(first_modified));
    let second = shallow_snapshot(&local_repo).expect("second shallow snapshot");
    assert_ne!(
        first, second,
        "cache identity must come from the object ids"
    );
}

#[test]
fn only_a_missing_object_allows_the_date_order_fallback() {
    let oid =
        gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").expect("valid oid");
    let missing =
        gix::traverse::commit::topo::Error::Find(gix::objs::find::existing_iter::Error::NotFound {
            oid,
        });

    assert!(topo_build_error_is_missing_object(&missing));
    assert!(!topo_build_error_is_missing_object(
        &gix::traverse::commit::topo::Error::MissingStateUnexpected
    ));
}

#[test]
fn diff_range_files_lists_changes_between_two_commits() {
    use gitcomet_core::domain::FileStatusKind;
    use gitcomet_core::services::GitRepository;

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    // Base commit: two files.
    write_file(repo, "keep.txt", "one\ntwo\n");
    write_file(repo, "gone.txt", "delete me\n");
    git_success(repo, &["add", "."]);
    git_success(repo, &["commit", "-m", "base"]);
    let from = git_stdout(repo, &["rev-parse", "HEAD"]);

    // Target commit: modify keep.txt, delete gone.txt, add new.txt.
    write_file(repo, "keep.txt", "one\ntwo\nthree\n");
    fs::remove_file(repo.join("gone.txt")).expect("remove gone.txt");
    write_file(repo, "new.txt", "brand new\n");
    git_success(repo, &["add", "-A"]);
    git_success(repo, &["commit", "-m", "target"]);
    let to = git_stdout(repo, &["rev-parse", "HEAD"]);

    let opened = open_repo(repo);
    let mut files = opened
        .diff_range_files(&CommitId(from.into()), Some(&CommitId(to.into())))
        .expect("diff_range_files should succeed");
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let by_path: Vec<(String, FileStatusKind)> = files
        .iter()
        .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
        .collect();
    assert_eq!(
        by_path,
        vec![
            ("gone.txt".to_string(), FileStatusKind::Deleted),
            ("keep.txt".to_string(), FileStatusKind::Modified),
            ("new.txt".to_string(), FileStatusKind::Added),
        ]
    );
}

#[test]
fn diff_range_files_lists_changes_against_the_working_tree() {
    use gitcomet_core::domain::FileStatusKind;
    use gitcomet_core::services::GitRepository;

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    // Committed base: two files.
    write_file(repo, "keep.txt", "one\ntwo\n");
    write_file(repo, "gone.txt", "delete me\n");
    git_success(repo, &["add", "."]);
    git_success(repo, &["commit", "-m", "base"]);
    let from = git_stdout(repo, &["rev-parse", "HEAD"]);

    // Uncommitted worktree changes: modify keep.txt (unstaged), delete
    // gone.txt (staged), add new.txt (staged). `git diff <from>` compares the
    // commit directly to the worktree, so all three show; the untracked
    // scratch file does not.
    write_file(repo, "keep.txt", "one\ntwo\nthree\n");
    fs::remove_file(repo.join("gone.txt")).expect("remove gone.txt");
    write_file(repo, "new.txt", "brand new\n");
    git_success(repo, &["add", "new.txt", "gone.txt"]);
    write_file(repo, "untracked.txt", "scratch\n");

    let opened = open_repo(repo);
    // `None` tip = compare `from` against the working tree.
    let mut files = opened
        .diff_range_files(&CommitId(from.into()), None)
        .expect("diff_range_files should succeed");
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let by_path: Vec<(String, FileStatusKind)> = files
        .iter()
        .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
        .collect();
    assert_eq!(
        by_path,
        vec![
            ("gone.txt".to_string(), FileStatusKind::Deleted),
            ("keep.txt".to_string(), FileStatusKind::Modified),
            ("new.txt".to_string(), FileStatusKind::Added),
        ]
    );
}

/// A gitlink has to be flagged as a submodule on both comparison paths. The
/// tree-diff path reads the entry mode; the working-tree path only gets modes
/// out of `git diff --raw`, so a plain `--name-status` listing would render
/// the same submodule as an ordinary file.
#[test]
fn diff_range_files_flags_a_submodule_pointer_against_the_working_tree() {
    use gitcomet_core::services::GitRepository;

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    write_file(repo, "keep.txt", "one\n");
    git_success(repo, &["add", "."]);
    git_success(repo, &["commit", "-m", "base"]);
    let from = git_stdout(repo, &["rev-parse", "HEAD"]);

    // A gitlink staged straight into the index — no submodule clone needed
    // to produce the 160000 entry mode the flag is derived from. The
    // directory has to exist or `git diff <commit>` skips the entry when
    // comparing against the working tree.
    fs::create_dir_all(repo.join("vendor/sub")).expect("create gitlink dir");
    git_success(
        repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,vendor/sub",
        ],
    );

    let opened = open_repo(repo);
    let files = opened
        .diff_range_files(&CommitId(from.into()), None)
        .expect("diff_range_files should succeed");
    let gitlink = files
        .iter()
        .find(|f| f.path.to_string_lossy() == "vendor/sub")
        .expect("the gitlink should be listed");
    assert!(
        gitlink.is_submodule,
        "a gitlink must be reported as a submodule, not as a plain file"
    );
    assert!(
        files
            .iter()
            .all(|f| f.is_submodule == (f.path.to_string_lossy() == "vendor/sub")),
        "ordinary files must not be flagged as submodules"
    );
}

/// A rename is the one `git diff --raw` record that carries *two* path
/// fields instead of one. Mis-counting them shifts every following record by
/// a field, silently pairing paths with the wrong statuses for the rest of
/// the listing, so the shape is worth pinning directly.
#[test]
fn diff_range_files_parses_renames_against_the_working_tree() {
    use gitcomet_core::domain::FileStatusKind;
    use gitcomet_core::services::GitRepository;

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    // Enough identical content that git scores the move as a rename.
    let body = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
    write_file(repo, "old_name.txt", body);
    write_file(repo, "untouched.txt", "steady\n");
    git_success(repo, &["add", "."]);
    git_success(repo, &["commit", "-m", "base"]);
    let from = git_stdout(repo, &["rev-parse", "HEAD"]);

    // Rename, then change a second file so a mis-parse would visibly shift
    // the records that follow the two-path one.
    fs::remove_file(repo.join("old_name.txt")).expect("remove old_name.txt");
    write_file(repo, "new_name.txt", body);
    write_file(repo, "untouched.txt", "steady\nplus one\n");
    git_success(repo, &["add", "-A"]);

    let opened = open_repo(repo);
    let mut files = opened
        .diff_range_files(&CommitId(from.into()), None)
        .expect("diff_range_files should succeed");
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let by_path: Vec<(String, FileStatusKind)> = files
        .iter()
        .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
        .collect();
    assert_eq!(
        by_path,
        vec![
            ("new_name.txt".to_string(), FileStatusKind::Renamed),
            ("untouched.txt".to_string(), FileStatusKind::Modified),
        ],
        "the rename must report its destination path, and the record after \
             it must not be shifted"
    );
}

/// The empty tree is how the changes a root commit *introduces* are
/// expressed — a root has no parent to diff from — so it has to resolve as a
/// comparison base even though it is not a commit.
#[test]
fn diff_range_files_accepts_the_empty_tree_as_a_base() {
    use gitcomet_core::domain::{EMPTY_TREE_ID, FileStatusKind};
    use gitcomet_core::services::GitRepository;

    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    write_file(repo, "a.txt", "one\n");
    write_file(repo, "b.txt", "two\n");
    git_success(repo, &["add", "."]);
    git_success(repo, &["commit", "-m", "root"]);
    let root = git_stdout(repo, &["rev-parse", "HEAD"]);

    let opened = open_repo(repo);
    let mut files = opened
        .diff_range_files(
            &CommitId(EMPTY_TREE_ID.into()),
            Some(&CommitId(root.into())),
        )
        .expect("the empty tree should resolve as a base");
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let by_path: Vec<(String, FileStatusKind)> = files
        .iter()
        .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
        .collect();
    // Everything the root commit introduces shows up, rather than nothing.
    assert_eq!(
        by_path,
        vec![
            ("a.txt".to_string(), FileStatusKind::Added),
            ("b.txt".to_string(), FileStatusKind::Added),
        ]
    );
}

#[test]
fn rename_destination_at_commit_resolves_rename_introduced_at_merge() {
    // Regression: a bare `git diff-tree <merge>` prints nothing (it needs
    // -m/-c/--cc), so resolving a rename introduced at a merge commit used to
    // fail and the followed file fell back to a now-nonexistent path. The fix
    // diffs against the first parent explicitly, which works for merges too.
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path();
    init_test_repo(repo);

    commit_file(repo, "src/old.txt", "alpha\nbeta\ngamma\n", "base");
    let main = git_stdout(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);

    // A side branch edits the file so the merge is a real (non-ff) merge.
    git_success(repo, &["checkout", "-b", "feature"]);
    commit_file(
        repo,
        "src/old.txt",
        "alpha\nbeta two\ngamma\n",
        "edit on feature",
    );

    // Back on the mainline, advance unrelated history, then merge feature but
    // resolve it by renaming the file — a rename that exists only at the merge
    // commit relative to its first parent.
    git_success(repo, &["checkout", &main]);
    commit_file(repo, "other.txt", "x\n", "main advance");
    git_success(repo, &["merge", "--no-commit", "--no-ff", "feature"]);
    fs::create_dir_all(repo.join("lib")).expect("create lib dir");
    git_success(repo, &["mv", "src/old.txt", "lib/new.txt"]);
    git_success(repo, &["commit", "-m", "evil merge rename"]);
    let merge = git_stdout(repo, &["rev-parse", "HEAD"]);

    let opened = open_repo(repo);
    let resolved = opened
        .rename_destination_at_commit(&CommitId(merge.into()), Path::new("src/old.txt"))
        .expect("rename resolution should not error");
    assert_eq!(resolved, Some(PathBuf::from("lib/new.txt")));
}

#[test]
fn recent_commit_message_limits_cap_large_requests_without_panicking() {
    assert_eq!(recent_commit_message_limits(0), None);
    assert_eq!(recent_commit_message_limits(1), Some((1, 5)));
    assert_eq!(recent_commit_message_limits(10), Some((10, 50)));
    assert_eq!(recent_commit_message_limits(20), Some((20, 100)));
    assert_eq!(recent_commit_message_limits(21), Some((21, 100)));
    assert_eq!(recent_commit_message_limits(100), Some((100, 100)));
    assert_eq!(recent_commit_message_limits(101), Some((100, 100)));
    assert_eq!(recent_commit_message_limits(usize::MAX), Some((100, 100)));
}

#[test]
fn recent_commit_messages_large_limit_reads_available_messages() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "tracked.txt", "one\n", "first");
    commit_file(workdir, "tracked.txt", "two\n", "second");
    commit_file(workdir, "tracked.txt", "three\n", "third");

    let repo = open_repo(workdir);
    let messages = repo
        .recent_commit_messages_impl(usize::MAX)
        .expect("recent commit messages");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].message, "third");
    assert_eq!(messages[1].message, "second");
    assert_eq!(messages[2].message, "first");
}

#[test]
fn apply_first_parent_resume_hint_uses_first_parent_of_last_commit() {
    let mut page = LogPage {
        commits: vec![
            Commit {
                id: CommitId("c1".into()),
                parent_ids: CommitParentIds::from_vec(vec![CommitId("p0".into())]),
                summary: Arc::from("one"),
                author: Arc::from("you"),
                time: std::time::SystemTime::UNIX_EPOCH,
            },
            Commit {
                id: CommitId("c2".into()),
                parent_ids: CommitParentIds::from_vec(vec![
                    CommitId("p1".into()),
                    CommitId("p2".into()),
                ]),
                summary: Arc::from("two"),
                author: Arc::from("you"),
                time: std::time::SystemTime::UNIX_EPOCH,
            },
        ],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c2".into()),
            resume_from: None,
            resume_token: None,
        }),
    };

    apply_first_parent_resume_hint(&mut page);

    assert_eq!(
        page.next_cursor
            .as_ref()
            .and_then(|cursor| cursor.resume_from.clone()),
        Some(CommitId("p1".into()))
    );
}

#[test]
fn apply_first_parent_resume_hint_clears_stale_resume_hint_when_no_parent_exists() {
    let mut page = LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: CommitParentIds::new(),
            summary: Arc::from("one"),
            author: Arc::from("you"),
            time: std::time::SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: Some(CommitId("stale".into())),
            resume_token: None,
        }),
    };

    apply_first_parent_resume_hint(&mut page);

    assert_eq!(
        page.next_cursor.as_ref().expect("next cursor").resume_from,
        None
    );
}

#[test]
fn repeated_author_cache_reuses_arc_for_identical_names() {
    let mut cache = RepeatedAuthorCache::default();

    let first = cache.intern(b"Bench");
    let second = cache.intern(b"Bench");
    let third = cache.intern(b"Other");

    assert!(Arc::ptr_eq(&first, &second));
    assert!(!Arc::ptr_eq(&second, &third));
}

#[test]
fn next_commit_id_cache_reuses_commit_id_for_matching_first_parent() {
    let mut cache = NextCommitIdCache::default();

    let parent = CommitId(Arc::from("1111111111111111111111111111111111111111"));
    let oid = gix::ObjectId::from_hex(parent.as_ref().as_bytes()).expect("valid oid");
    cache.remember(oid.as_ref(), &parent);

    let reused = cache.reuse_or_new(oid.as_ref(), || CommitId(Arc::from("other")));
    let other_oid =
        gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").expect("valid oid");
    let fresh = cache.reuse_or_new(other_oid.as_ref(), || CommitId(Arc::from("fresh")));

    assert!(Arc::ptr_eq(&parent.0, &reused.0));
    assert_eq!(fresh.as_ref(), "fresh");
}

#[test]
fn cursor_file_history_pages_reuse_cached_follow_history() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "tracked.txt", "one\n", "one");
    commit_file(workdir, "tracked.txt", "two\n", "two");
    git_success(workdir, &["mv", "tracked.txt", "renamed.txt"]);
    git_success(workdir, &["commit", "-m", "rename"]);
    commit_file(workdir, "renamed.txt", "four\n", "four");

    let repo = open_repo(workdir);
    let page1 = repo
        .log_file_page_impl(Path::new("renamed.txt"), 1, None)
        .expect("first file log page");
    assert_eq!(page1.commits.len(), 1);
    assert!(page1.next_cursor.is_some());
    assert!(
        repo.log_file_follow_cache
            .lock()
            .expect("log file follow cache")
            .is_empty(),
        "first page should stay bounded and avoid the full-history cache"
    );

    let page2 = repo
        .log_file_page_impl(Path::new("renamed.txt"), 1, page1.next_cursor.as_ref())
        .expect("second file log page");
    assert_eq!(page2.commits.len(), 1);
    assert!(page2.next_cursor.is_some());

    let cached_commits = {
        let cache = repo
            .log_file_follow_cache
            .lock()
            .expect("log file follow cache");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].key.path.as_path(), Path::new("renamed.txt"));
        assert_eq!(cache[0].commits.len(), 4);
        Arc::clone(&cache[0].commits)
    };

    let page3 = repo
        .log_file_page_impl(Path::new("renamed.txt"), 1, page2.next_cursor.as_ref())
        .expect("third file log page");
    assert_eq!(page3.commits.len(), 1);

    let cache = repo
        .log_file_follow_cache
        .lock()
        .expect("log file follow cache");
    assert_eq!(cache.len(), 1);
    assert!(
        Arc::ptr_eq(&cached_commits, &cache[0].commits),
        "third page should use the cached full follow result"
    );
}

#[test]
fn reflog_head_entries_carry_the_committer_as_author() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "a.txt", "one\n", "first");
    commit_file(workdir, "a.txt", "two\n", "second");

    let repo = open_repo(workdir);
    let entries = repo.reflog_head_impl(10).expect("reflog_head_impl");

    assert_eq!(entries.len(), 2);
    // Newest first: the reflog is read in reverse, matching `HEAD@{0}`
    // being the current position.
    assert_eq!(entries[0].selector.as_ref(), "HEAD@{0}");
    assert_eq!(entries[1].selector.as_ref(), "HEAD@{1}");
    for entry in &entries {
        // `user.name` from `init_test_repo` is what git records as the
        // reflog line's committer identity.
        assert_eq!(entry.author.as_ref(), "Test User");
    }
}

#[test]
fn reflog_head_impl_handles_an_unbounded_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    commit_file(workdir, "a.txt", "one\n", "first");

    let repo = open_repo(workdir);
    // `usize::MAX` reads as "every entry": it must not be reserved up front.
    let entries = repo.reflog_head_impl(usize::MAX).expect("reflog_head_impl");
    assert_eq!(entries.len(), 1);
}

#[test]
fn reflog_head_impl_returns_empty_for_a_zero_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    commit_file(workdir, "a.txt", "one\n", "first");

    let repo = open_repo(workdir);
    let entries = repo.reflog_head_impl(0).expect("reflog_head_impl");
    assert!(entries.is_empty());
}

#[test]
fn finishing_a_page_reports_a_request_that_was_superseded_while_it_was_built() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    commit_file(workdir, "a.txt", "one\n", "first");
    let repo = open_repo(workdir);
    let shallow = shallow_snapshot(&repo._repo.to_thread_local()).expect("shallow snapshot");

    let key = repo.log_page_cache_key(
        HistoryMode::AllBranches,
        super::super::LogPageSeed::Tips(std::sync::Arc::from(Vec::new())),
        &shallow,
        10,
        None,
        None,
    );
    let page = empty_log_page();

    let live = CancellationToken::new();
    repo.finish_log_page(key.clone(), page.clone(), Some(&live))
        .expect("a live request gets its page");

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = repo
        .finish_log_page(key.clone(), page, Some(&cancelled))
        .expect_err("a superseded request must not be handed a page");
    assert!(
        matches!(error.kind(), ErrorKind::Cancelled),
        "expected Cancelled, got {:?}",
        error.kind()
    );

    assert!(
        repo.cached_log_page(&key).is_some(),
        "a cancelled request still leaves the page it finished in the cache"
    );
}

#[test]
fn deep_log_pages_share_a_total_cache_row_budget() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", "one\n", "first");
    let repo = open_repo(tmp.path());
    let shallow = shallow_snapshot(&repo._repo.to_thread_local()).expect("shallow snapshot");
    let commit = repo.log_head_page_impl(1, None).unwrap().commits.remove(0);
    for limit in [4000, 4001, 4002, 20_000] {
        let key = repo.log_page_cache_key(
            HistoryMode::AllBranches,
            super::super::LogPageSeed::Tips(Arc::from(Vec::new())),
            &shallow,
            limit,
            None,
            None,
        );
        repo.store_log_page(
            key.clone(),
            &LogPage {
                commits: vec![commit.clone(); limit],
                next_cursor: None,
            },
        );
        let rows: usize = repo
            .log_page_cache
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.page.commits.len())
            .sum();
        assert!(rows <= 10_000, "cached {rows} rows");
        assert_eq!(repo.cached_log_page(&key).is_some(), limit <= 10_000);
    }
}

#[test]
fn a_cached_page_keeps_the_page_that_follows_it_from_being_evicted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    for index in 0..12 {
        commit_file(
            workdir,
            "a.txt",
            &format!("v{index}\n"),
            &format!("c{index}"),
        );
    }
    let repo = open_repo(workdir);

    let mode = HistoryMode::FullReachable;
    let first = repo
        .log_history_mode_page_impl(mode, 4, None)
        .expect("first page");
    let cursor = first.next_cursor.clone().expect("a second page exists");
    repo.log_history_mode_page_impl(mode, 4, Some(&cursor))
        .expect("second page");

    let local_repo = repo._repo.to_thread_local();
    let head = gix_head_id_or_none(&local_repo).expect("head");
    let shallow = shallow_snapshot(&local_repo).expect("shallow snapshot");
    let second_key = repo.log_page_cache_key(
        mode,
        super::super::LogPageSeed::Head(head),
        &shallow,
        4,
        Some(&cursor),
        None,
    );

    for round in 0..super::super::LOG_PAGE_CACHE_LIMIT + 4 {
        repo.log_history_mode_page_impl(mode, 5 + round, None)
            .expect("filler page");
        repo.log_history_mode_page_impl(mode, 4, None)
            .expect("refresh of the first page");
    }

    assert!(
        repo.cached_log_page(&second_key).is_some(),
        "the page below the one being refreshed was evicted, so scrolling \
             down rebuilds the walk instead of resuming it"
    );
}

#[test]
fn a_page_chunk_counts_lookahead_and_rejected_commits_as_visited() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    for index in 0..3 {
        commit_file(
            workdir,
            "a.txt",
            &format!("v{index}\n"),
            &format!("c{index}"),
        );
    }
    let repo = open_repo(workdir);
    let local_repo = repo._repo.to_thread_local();
    let shallow = shallow_snapshot(&local_repo).expect("shallow snapshot");
    let head = gix_head_id_or_none(&local_repo)
        .expect("read head")
        .expect("head commit");

    let mut walk = new_log_paged_walk(
        &repo._repo,
        [head],
        HistoryMode::FullReachable,
        &shallow,
        None,
        None,
    )
    .expect("build page walk");
    let mut scanned = Vec::new();
    let (commits, has_more) = {
        let mut on_chunk = |chunk: LogChunk| scanned.push(chunk.scanned);
        let mut emitter = ChunkEmitter::with_interval(&mut on_chunk, std::time::Duration::ZERO);
        log_page_from_paged_walk_state(
            &repo._repo,
            &mut walk,
            2,
            None,
            None,
            None,
            Some(&mut emitter),
            |_| true,
        )
        .expect("build page")
    };
    assert_eq!(commits.len(), 2);
    assert!(has_more);
    assert_eq!(
        scanned.last(),
        Some(&3),
        "the lookahead row was visited even though it was not decoded"
    );

    let mut rejected_walk = new_log_paged_walk(
        &repo._repo,
        [head],
        HistoryMode::FullReachable,
        &shallow,
        None,
        None,
    )
    .expect("build rejected-row walk");
    let mut rejected_scanned = Vec::new();
    let (commits, has_more) = {
        let mut on_chunk = |chunk: LogChunk| rejected_scanned.push(chunk.scanned);
        let mut emitter = ChunkEmitter::with_interval(&mut on_chunk, std::time::Duration::ZERO);
        log_page_from_paged_walk_state(
            &repo._repo,
            &mut rejected_walk,
            1,
            None,
            None,
            None,
            Some(&mut emitter),
            |_| false,
        )
        .expect("build page with rejected rows")
    };
    assert!(commits.is_empty());
    assert!(!has_more);
    assert_eq!(
        rejected_scanned.last(),
        Some(&3),
        "mode-rejected candidates still count as visited"
    );
}

#[test]
fn topo_setup_emits_a_chunk_and_honours_cancellation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    commit_file(workdir, "a.txt", "one\n", "one");
    let repo = open_repo(workdir);
    let cancellation = CancellationToken::new();
    let cancel_from_chunk = cancellation.clone();
    let mut chunks = Vec::new();
    let error = {
        let mut on_chunk = |chunk: LogChunk| {
            chunks.push(chunk);
            cancel_from_chunk.cancel();
        };

        repo.log_history_mode_page_streaming_impl(
            HistoryMode::FullReachable,
            None,
            10,
            None,
            &cancellation,
            &mut on_chunk,
        )
        .expect_err("cancellation during topo setup must stop the request")
    };

    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert_eq!(
        chunks,
        vec![LogChunk::default()],
        "the ordering phase must become visible before it starts"
    );
}

#[test]
fn a_resumed_topo_walk_uses_the_new_pages_cancellation_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    for index in 0..3 {
        commit_file(
            workdir,
            "a.txt",
            &format!("v{index}\n"),
            &format!("c{index}"),
        );
    }
    let repo = open_repo(workdir);
    let first_cancellation = CancellationToken::new();
    let first = repo
        .log_history_mode_page_streaming_impl(
            HistoryMode::FullReachable,
            None,
            1,
            None,
            &first_cancellation,
            &mut |_| {},
        )
        .expect("first page");
    first_cancellation.cancel();

    let second_cancellation = CancellationToken::new();
    let second = repo
        .log_history_mode_page_streaming_impl(
            HistoryMode::FullReachable,
            None,
            1,
            first.next_cursor.as_ref(),
            &second_cancellation,
            &mut |_| {},
        )
        .expect("the old page token must not cancel the resumed walk");

    assert_eq!(second.commits.len(), 1);
    assert!(second.next_cursor.is_some());
}

// ---------------------------------------------------------------------------
// Backend cache regression tests (object cache, commit stats, divergence memo,
// all-branches tips fingerprint, worktree-source and preview-blob memos).
// ---------------------------------------------------------------------------

/// Appends `count` linear commits on `branch` via `git fast-import`, each
/// touching `file`. `from` is the parent ref of the first commit, if any.
fn fast_import_commits(workdir: &Path, branch: &str, from: Option<&str>, count: usize, file: &str) {
    let mut stream = String::with_capacity(count * 160);
    for ix in 1..=count {
        let message = format!("commit {ix}");
        let content = format!("line {ix}\n");
        stream.push_str(&format!("commit refs/heads/{branch}\nmark :{ix}\n"));
        stream.push_str(&format!(
            "committer Test User <test@example.com> {} +0000\n",
            1_700_000_000 + ix as u64
        ));
        stream.push_str(&format!("data {}\n{message}\n", message.len()));
        if ix == 1 {
            if let Some(from) = from {
                stream.push_str(&format!("from {from}\n"));
            }
        } else {
            stream.push_str(&format!("from :{}\n", ix - 1));
        }
        stream.push_str(&format!(
            "M 100644 inline {file}\ndata {}\n{content}\n",
            content.len()
        ));
    }
    let mut cmd = crate::util::git_workdir_cmd_for(workdir);
    let mut child = cmd
        .args(["fast-import", "--quiet"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn git fast-import");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("fast-import stdin")
            .write_all(stream.as_bytes())
            .expect("write fast-import stream");
    }
    let output = child.wait_with_output().expect("fast-import exit");
    assert!(
        output.status.success(),
        "fast-import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn head_commit_id(workdir: &Path) -> CommitId {
    CommitId(git_stdout(workdir, &["rev-parse", "HEAD"]).into())
}

#[test]
fn commit_details_stats_skip_oversized_blobs_without_inflating_them() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "small.txt", "a\nb\n", "base");
    let oversized = "x".repeat(COMMIT_STATS_MAX_BLOB_BYTES + 1);
    write_file(tmp.path(), "small.txt", "a\nb\nc\n");
    write_file(tmp.path(), "big.bin", &oversized);
    git_success(tmp.path(), &["add", "."]);
    git_success(tmp.path(), &["commit", "-m", "grow"]);

    let repo = open_repo(tmp.path());
    let details = repo
        .commit_details_impl(&head_commit_id(tmp.path()))
        .expect("commit details");
    let by_path = |name: &str| {
        details
            .files
            .iter()
            .find(|file| file.path == Path::new(name))
            .unwrap_or_else(|| panic!("{name} in details"))
    };
    assert_eq!(
        (
            by_path("small.txt").additions,
            by_path("small.txt").deletions
        ),
        (Some(1), Some(0))
    );
    assert_eq!(
        (by_path("big.bin").additions, by_path("big.bin").deletions),
        (None, None),
        "a blob over the size cap must report unknown stats"
    );
}

#[test]
fn commit_details_stats_treat_binary_and_absent_sides_as_before() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "keep.txt", "one\ntwo\n", "base");
    write_file(tmp.path(), "binary.dat", "ab\0cd");
    std::fs::remove_file(tmp.path().join("keep.txt")).expect("delete keep");
    write_file(tmp.path(), "new.txt", "1\n2\n3\n");
    git_success(tmp.path(), &["add", "-A"]);
    git_success(tmp.path(), &["commit", "-m", "mixed"]);

    let repo = open_repo(tmp.path());
    let details = repo
        .commit_details_impl(&head_commit_id(tmp.path()))
        .expect("commit details");
    let stats = |name: &str| {
        let file = details
            .files
            .iter()
            .find(|file| file.path == Path::new(name))
            .unwrap_or_else(|| panic!("{name} in details"));
        (file.additions, file.deletions)
    };
    assert_eq!(stats("binary.dat"), (None, None));
    assert_eq!(stats("keep.txt"), (Some(0), Some(2)));
    assert_eq!(stats("new.txt"), (Some(3), Some(0)));
}

fn set_self_upstream(workdir: &Path, branch: &str) {
    let path = workdir.to_string_lossy().into_owned();
    git_success(workdir, &["remote", "add", "origin", &path]);
    git_success(workdir, &["fetch", "-q", "origin"]);
    git_success(
        workdir,
        &[
            "branch",
            &format!("--set-upstream-to=origin/{branch}"),
            branch,
        ],
    );
}

#[test]
fn upstream_divergence_is_memoized_by_tip_pair_and_recomputed_when_tips_move() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    set_self_upstream(tmp.path(), "main");
    commit_file(tmp.path(), "a.txt", "2\n", "c2");

    let repo = open_repo(tmp.path());
    let first = repo.upstream_divergence_impl().expect("divergence");
    assert_eq!(
        first,
        Some(UpstreamDivergence {
            ahead: 1,
            behind: 0
        })
    );
    assert_eq!(repo.divergence_cache.lock().expect("cache").len(), 1);

    let again = repo.upstream_divergence_impl().expect("divergence again");
    assert_eq!(again, first);
    assert_eq!(repo.divergence_cache.lock().expect("cache").len(), 1);

    commit_file(tmp.path(), "a.txt", "3\n", "c3");
    let moved = repo
        .upstream_divergence_impl()
        .expect("divergence after move");
    assert_eq!(
        moved,
        Some(UpstreamDivergence {
            ahead: 2,
            behind: 0
        })
    );
    assert_eq!(repo.divergence_cache.lock().expect("cache").len(), 2);

    // Upstream catching up produces the equal-tip fast path (no walk, no entry).
    git_success(tmp.path(), &["fetch", "-q", "origin"]);
    let caught_up = repo
        .upstream_divergence_impl()
        .expect("divergence caught up");
    assert_eq!(caught_up, Some(UpstreamDivergence::default()));
    assert_eq!(repo.divergence_cache.lock().expect("cache").len(), 2);
}

#[test]
fn upstream_divergence_honours_cancellation_inside_the_walk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    set_self_upstream(tmp.path(), "main");
    fast_import_commits(tmp.path(), "main", Some("refs/heads/main^0"), 200, "a.txt");

    let repo = open_repo(tmp.path());
    let token = CancellationToken::new();
    token.cancel();
    let err = repo
        .upstream_divergence_cancellable_impl(&token)
        .expect_err("cancelled before walking");
    assert!(matches!(err.kind(), ErrorKind::Cancelled));
    assert!(
        repo.divergence_cache.lock().expect("cache").is_empty(),
        "a cancelled walk must not be memoized"
    );
}

#[test]
fn upstream_divergence_cache_invalidates_on_shallow_boundary_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    set_self_upstream(tmp.path(), "main");
    commit_file(tmp.path(), "a.txt", "2\n", "c2");
    commit_file(tmp.path(), "a.txt", "3\n", "c3");
    let middle = git_stdout(tmp.path(), &["rev-parse", "HEAD"]);
    commit_file(tmp.path(), "a.txt", "4\n", "c4");
    let tips = git_stdout(tmp.path(), &["show-ref"]);
    let head = git_stdout(tmp.path(), &["rev-parse", "HEAD"]);
    write_file(tmp.path(), ".git/shallow", &format!("{head}\n"));
    let repo = open_repo(tmp.path());
    // Model depth 1, depth 2, and unshallow without updating either ref.
    for (boundary, expected_ahead) in [(Some(head), 1), (Some(middle), 2), (None, 3)] {
        if let Some(boundary) = boundary {
            write_file(tmp.path(), ".git/shallow", &format!("{boundary}\n"));
        } else {
            fs::remove_file(tmp.path().join(".git/shallow")).expect("unshallow");
        }
        assert_eq!(git_stdout(tmp.path(), &["show-ref"]), tips);
        let fresh = open_repo(tmp.path())
            .upstream_divergence_impl()
            .expect("fresh divergence");
        let ahead: usize = git_stdout(
            tmp.path(),
            &["rev-list", "--count", "HEAD", "--not", "@{upstream}"],
        )
        .parse()
        .unwrap();
        assert_eq!(ahead, expected_ahead);
        assert_eq!(fresh.unwrap().ahead, ahead);
        assert_eq!(repo.upstream_divergence_impl().expect("divergence"), fresh);
    }
}

#[test]
fn all_branches_tips_reuse_cached_tips_until_the_ref_namespace_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    git_success(tmp.path(), &["branch", "topic"]);
    git_success(tmp.path(), &["tag", "-m", "v1", "v1"]);

    let repo = open_repo(tmp.path());
    let handle = repo.repo();
    let first = repo.all_branches_tips(&handle, None).expect("tips");
    let second = repo.all_branches_tips(&handle, None).expect("tips again");
    assert!(
        Arc::ptr_eq(&first, &second),
        "an unchanged ref namespace must serve the cached tips"
    );
    assert_eq!(
        first.len(),
        1,
        "main and topic share one tip; the tag is excluded"
    );

    commit_file(tmp.path(), "a.txt", "2\n", "c2");
    let handle = repo.repo();
    let moved = repo
        .all_branches_tips(&handle, None)
        .expect("tips after commit");
    assert_eq!(moved.len(), 2, "main moved away from topic");
    assert!(!Arc::ptr_eq(&first, &moved));

    git_success(tmp.path(), &["tag", "-m", "v2", "v2"]);
    let tagged = repo
        .all_branches_tips(&handle, None)
        .expect("tips after tag");
    assert!(
        Arc::ptr_eq(&moved, &tagged),
        "tags do not seed the walk, so a new tag must not invalidate"
    );

    git_success(tmp.path(), &["branch", "-D", "topic"]);
    let deleted = repo
        .all_branches_tips(&handle, None)
        .expect("tips after delete");
    assert_eq!(deleted.len(), 1);

    write_file(tmp.path(), "a.txt", "dirty\n");
    git_success(tmp.path(), &["stash", "-q"]);
    let stashed = repo
        .all_branches_tips(&handle, None)
        .expect("tips after stash");
    assert!(
        stashed.len() > deleted.len(),
        "a stash tip must be picked up: {} vs {}",
        stashed.len(),
        deleted.len()
    );
}

#[test]
fn all_branches_tips_follow_symbolic_refs_into_the_tag_namespace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    git_success(tmp.path(), &["config", "tag.gpgSign", "false"]);
    git_success(tmp.path(), &["update-ref", "refs/tags/moving", "HEAD"]);
    git_success(
        tmp.path(),
        &["symbolic-ref", "refs/custom/latest", "refs/tags/moving"],
    );

    let repo = open_repo(tmp.path());
    let handle = repo.repo();
    let first = repo.all_branches_tips(&handle, None).expect("tips");
    assert_eq!(
        first.len(),
        1,
        "latest resolves through the tag to main's tip"
    );

    commit_file(tmp.path(), "a.txt", "2\n", "c2");
    let handle = repo.repo();
    let moved = repo
        .all_branches_tips(&handle, None)
        .expect("tips after commit");
    assert_eq!(moved.len(), 2, "latest still reaches c1 through the tag");

    // Only the tag moved: its name is excluded from the fingerprint, but the
    // symbolic ref that follows it now seeds a different commit.
    git_success(tmp.path(), &["update-ref", "refs/tags/moving", "HEAD"]);
    let retagged = repo
        .all_branches_tips(&handle, None)
        .expect("tips after retag");
    let fresh = open_repo(tmp.path())
        .all_branches_tips(&handle, None)
        .expect("fresh tips");
    assert_eq!(retagged.as_ref(), fresh.as_ref());
    assert_eq!(retagged.len(), 1, "latest now resolves to main's tip");

    // An annotated tag makes the symbolic chain end at a tag object.
    git_success(tmp.path(), &["tag", "-f", "-m", "old", "moving", "HEAD~1"]);
    let annotated = repo
        .all_branches_tips(&handle, None)
        .expect("tips after annotated retag");
    assert_eq!(annotated.len(), 2, "latest resolves back to c1");
}

#[test]
fn ref_metadata_cache_follows_symbolic_branches_into_the_tag_namespace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    commit_file(tmp.path(), "a.txt", "2\n", "c2");
    git_success(tmp.path(), &["update-ref", "refs/tags/moving", "HEAD~1"]);
    git_success(
        tmp.path(),
        &["symbolic-ref", "refs/heads/latest", "refs/tags/moving"],
    );

    let repo = open_repo(tmp.path());
    let summary = |repo: &GixRepo| {
        repo.list_ref_metadata_impl().expect("ref metadata")["latest"]
            .summary
            .clone()
    };
    assert_eq!(summary(&repo), "c1");

    git_success(tmp.path(), &["update-ref", "refs/tags/moving", "HEAD"]);
    assert_eq!(summary(&open_repo(tmp.path())), "c2");
    assert_eq!(summary(&repo), "c2", "cache must follow the moved tag");
}

#[cfg(unix)]
#[test]
fn worktree_file_source_memo_serves_unchanged_files_and_notices_edits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "src.txt", "one\ntwo\n", "base");
    let file = tmp.path().join("src.txt");
    let age_out = |path: &Path| {
        let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open for utime")
            .set_modified(stale)
            .expect("set mtime");
    };
    age_out(&file);

    let repo = open_repo(tmp.path());
    let handle = repo.repo();
    let first = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("source")
        .expect("file exists");
    assert_eq!(repo.worktree_source_memo.lock().expect("memo").len(), 1);
    let second = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("source again")
        .expect("file exists");
    assert_eq!(first.path, second.path);
    assert_eq!(first.identity, second.identity);

    // Same length, different bytes: the in-place write changes ctime, so the
    // memo must miss and the identity (content hash) must change.
    std::fs::write(&file, "one\nTWO\n").expect("edit in place");
    age_out(&file);
    let edited = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("source after edit")
        .expect("file exists");
    assert_ne!(edited.identity, first.identity);
    assert_eq!(
        std::fs::read(&edited.path).expect("read cache"),
        b"one\nTWO\n"
    );

    // A freshly written file (within the racy window) is served but not memoized.
    std::fs::write(&file, "fresh\n").expect("fresh write");
    let fresh = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("fresh source")
        .expect("file exists");
    assert_eq!(std::fs::read(&fresh.path).expect("read cache"), b"fresh\n");
    let memo = repo.worktree_source_memo.lock().expect("memo");
    assert_ne!(
        memo.get(Path::new("src.txt"))
            .map(|entry| entry.identity.clone()),
        Some(fresh.identity.clone()),
        "a racy-fresh file must not be memoized"
    );
}

#[cfg(unix)]
#[test]
fn worktree_file_source_memo_invalidates_on_gitattributes_change() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "src.txt", "one\r\ntwo\r\n", "base");
    let file = tmp.path().join("src.txt");
    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .expect("open")
        .set_modified(stale)
        .expect("mtime");

    let repo = open_repo(tmp.path());
    let handle = repo.repo();
    let first = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("source")
        .expect("exists");
    assert_eq!(std::fs::read(&first.path).expect("read"), b"one\r\ntwo\r\n");

    write_file(tmp.path(), ".gitattributes", "*.txt text eol=lf\n");
    let handle = repo.repo();
    let normalized = repo
        .cached_git_normalized_worktree_file_source(&handle, Path::new("src.txt"))
        .expect("source after attributes")
        .expect("exists");
    assert_ne!(normalized.identity, first.identity);
    assert_eq!(
        std::fs::read(&normalized.path).expect("read"),
        b"one\ntwo\n"
    );
}

#[cfg(unix)]
#[test]
fn worktree_file_source_memo_bypasses_external_clean_filters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "src.txt", "one\ntwo\n", "base");
    let scripts = tempfile::tempdir().expect("script dir");
    let script = scripts.path().join("clean.sh");
    fs::write(&script, "exec tr a-z A-Z\n").unwrap();
    git_success(
        tmp.path(),
        &[
            "config",
            "filter.upper.clean",
            &format!("sh {}", script.display()),
        ],
    );
    write_file(tmp.path(), ".gitattributes", "*.txt filter=upper\n");
    fs::File::options()
        .write(true)
        .open(tmp.path().join("src.txt"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(30))
        .unwrap();

    let repo = open_repo(tmp.path());
    let read_source = |repo: &GixRepo| {
        let source = repo
            .cached_git_normalized_worktree_file_source(&repo.repo(), Path::new("src.txt"))
            .expect("source")
            .expect("exists");
        fs::read(source.path).unwrap()
    };
    assert_eq!(read_source(&repo), b"ONE\nTWO\n");
    assert!(
        repo.worktree_source_memo.lock().unwrap().is_empty(),
        "an external driver's output cannot be validated, so it is never memoized"
    );

    // The driver changes behind git's back: no tracked input differs.
    fs::write(&script, "exec sed s/one/uno/\n").unwrap();
    assert_eq!(read_source(&open_repo(tmp.path())), b"uno\ntwo\n");
    assert_eq!(
        read_source(&repo),
        b"uno\ntwo\n",
        "the open repository must run the changed driver"
    );
}

#[test]
fn worktree_file_source_memo_invalidates_on_global_attributes_change() {
    assert_attribute_source_invalidates_memo(false);
}

#[test]
fn worktree_file_source_memo_invalidates_on_index_attributes_change() {
    assert_attribute_source_invalidates_memo(true);
}

fn assert_attribute_source_invalidates_memo(index_only: bool) {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    commit_file(tmp.path(), "src.txt", "one\r\ntwo\r\n", "base");
    let global = tempfile::NamedTempFile::new().expect("global attributes");
    git_success(
        tmp.path(),
        &[
            "config",
            "core.attributesFile",
            global.path().to_str().unwrap(),
        ],
    );
    let attributes = if index_only {
        tmp.path().join(".gitattributes")
    } else {
        global.path().to_path_buf()
    };
    fs::write(&attributes, "*.txt -text\n").unwrap();
    if index_only {
        git_success(tmp.path(), &["add", ".gitattributes"]);
        fs::remove_file(&attributes).unwrap();
    }
    fs::File::options()
        .write(true)
        .open(tmp.path().join("src.txt"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(30))
        .unwrap();
    let repo = open_repo(tmp.path());
    let read_source = |repo: &GixRepo| {
        let source = repo
            .cached_git_normalized_worktree_file_source(&repo.repo(), Path::new("src.txt"))
            .expect("source")
            .expect("exists");
        fs::read(source.path).unwrap()
    };
    assert_eq!(read_source(&repo), b"one\r\ntwo\r\n");
    // DiskFileStamp has no inode/ctime off Unix, so nothing is memoized there.
    if cfg!(unix) {
        assert_eq!(repo.worktree_source_memo.lock().unwrap().len(), 1);
    }
    fs::write(&attributes, "*.txt text eol=lf\n").unwrap();
    if index_only {
        git_success(tmp.path(), &["add", ".gitattributes"]);
        fs::remove_file(&attributes).unwrap();
    }
    assert_eq!(
        read_source(&open_repo(tmp.path())),
        b"one\ntwo\n",
        "fresh repository sees changed attributes"
    );
    assert_eq!(
        read_source(&repo),
        b"one\ntwo\n",
        "memo must see changed attributes"
    );
}

#[cfg(unix)]
#[test]
fn preview_blob_verification_memo_rechecks_a_rewritten_cache_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    write_file(tmp.path(), "image.bin", "real blob bytes");
    git_success(tmp.path(), &["add", "image.bin"]);
    let blob_id = gix::objs::compute_hash(
        gix::hash::Kind::Sha1,
        gix::objs::Kind::Blob,
        b"real blob bytes",
    )
    .expect("blob id");

    let repo = open_repo(tmp.path());
    let first = repo
        .cached_preview_blob_file_path(blob_id, Path::new("image.bin"))
        .expect("materialize")
        .expect("blob");
    assert!(
        repo.preview_blob_verified.lock().expect("memo").is_empty(),
        "a newly materialized file must be re-verified outside its timestamp race window"
    );
    let second = repo
        .cached_preview_blob_file_path(blob_id, Path::new("image.bin"))
        .expect("reuse")
        .expect("blob");
    assert_eq!(first, second);

    // Same-length tampering may preserve every stamp field on filesystems with
    // coarse timestamps. A fresh file must still be hashed again and repaired.
    std::fs::write(&first, b"fake blob bytes").expect("tamper");
    let third = repo
        .cached_preview_blob_file_path(blob_id, Path::new("image.bin"))
        .expect("re-verify")
        .expect("blob");
    assert_eq!(
        std::fs::read(&third).expect("read served"),
        b"real blob bytes"
    );
}

// Timing probes: `cargo test -p gitcomet-git-gix -- --ignored --nocapture timing_`
// They print durations rather than asserting them, for before/after comparison.

#[test]
#[ignore = "timing probe"]
fn timing_commit_details_with_oversized_blobs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let files = 40;
    let oversized = "y".repeat(COMMIT_STATS_MAX_BLOB_BYTES + 1);
    for ix in 0..files {
        write_file(tmp.path(), &format!("big{ix}.bin"), &oversized);
    }
    git_success(tmp.path(), &["add", "."]);
    git_success(tmp.path(), &["commit", "-q", "-m", "base"]);
    for ix in 0..files {
        write_file(
            tmp.path(),
            &format!("big{ix}.bin"),
            &format!("z{oversized}"),
        );
    }
    git_success(tmp.path(), &["add", "."]);
    git_success(tmp.path(), &["commit", "-q", "-m", "grow"]);
    git_success(tmp.path(), &["gc", "-q"]);

    let repo = open_repo(tmp.path());
    let id = head_commit_id(tmp.path());
    let started = std::time::Instant::now();
    let details = repo.commit_details_impl(&id).expect("details");
    let elapsed = started.elapsed();
    assert_eq!(details.files.len(), files);
    println!("timing commit_details {files} oversized blobs: {elapsed:?}");
}

#[test]
#[ignore = "timing probe"]
fn timing_upstream_divergence_far_behind() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    fast_import_commits(
        tmp.path(),
        "upstream",
        Some("refs/heads/main^0"),
        20_000,
        "a.txt",
    );
    let path = tmp.path().to_string_lossy().into_owned();
    git_success(tmp.path(), &["remote", "add", "origin", &path]);
    git_success(tmp.path(), &["fetch", "-q", "origin"]);
    git_success(tmp.path(), &["config", "branch.main.remote", "origin"]);
    git_success(
        tmp.path(),
        &["config", "branch.main.merge", "refs/heads/upstream"],
    );
    git_success(tmp.path(), &["commit-graph", "write", "--reachable"]);

    let repo = open_repo(tmp.path());
    for round in 1..=3 {
        let started = std::time::Instant::now();
        let divergence = repo.upstream_divergence_impl().expect("divergence");
        println!(
            "timing upstream_divergence round {round}: {:?} -> {divergence:?}",
            started.elapsed()
        );
    }
}

#[test]
#[ignore = "timing probe"]
fn timing_all_branches_page_with_many_refs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    fast_import_commits(
        tmp.path(),
        "main",
        Some("refs/heads/main^0"),
        2_000,
        "a.txt",
    );
    // One remote-tracking style ref per commit, as a fetched mirror would have.
    let mut refs = String::new();
    for ix in 0..2_000 {
        let id = git_stdout(tmp.path(), &["rev-parse", &format!("main~{ix}")]);
        refs.push_str(&format!("create refs/remotes/origin/b{ix} {id}\n"));
    }
    let mut cmd = crate::util::git_workdir_cmd_for(tmp.path());
    let mut child = cmd
        .args(["update-ref", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("update-ref");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(refs.as_bytes())
            .expect("write refs");
    }
    assert!(child.wait().expect("update-ref exit").success());
    git_success(tmp.path(), &["pack-refs", "--all"]);

    let repo = open_repo(tmp.path());
    for round in 1..=3 {
        let started = std::time::Instant::now();
        let page = repo
            .log_all_branches_page_impl(50, None)
            .expect("all branches page");
        println!(
            "timing all_branches_page round {round}: {:?} ({} commits)",
            started.elapsed(),
            page.commits.len()
        );
    }
}

#[test]
#[ignore = "timing probe"]
fn timing_ref_metadata_with_many_refs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(tmp.path(), &["checkout", "-q", "-b", "main"]);
    commit_file(tmp.path(), "a.txt", "1\n", "c1");
    fast_import_commits(
        tmp.path(),
        "main",
        Some("refs/heads/main^0"),
        2_000,
        "a.txt",
    );
    let mut refs = String::new();
    for ix in 0..2_000 {
        let id = git_stdout(tmp.path(), &["rev-parse", &format!("main~{ix}")]);
        refs.push_str(&format!("create refs/remotes/origin/b{ix} {id}\n"));
    }
    let mut cmd = crate::util::git_workdir_cmd_for(tmp.path());
    let mut child = cmd
        .args(["update-ref", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("update-ref");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(refs.as_bytes())
            .expect("write refs");
    }
    assert!(child.wait().expect("update-ref exit").success());
    git_success(tmp.path(), &["pack-refs", "--all"]);

    let repo = open_repo(tmp.path());
    for round in 1..=3 {
        let started = std::time::Instant::now();
        let metadata = repo.list_ref_metadata_impl().expect("ref metadata");
        println!(
            "timing ref_metadata round {round}: {:?} ({} refs)",
            started.elapsed(),
            metadata.len()
        );
    }
}
