use super::*;
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
