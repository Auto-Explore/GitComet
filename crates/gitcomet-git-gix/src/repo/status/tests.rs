use rustc_hash::FxHashMap;

use super::{
    STAT_REFRESH_MIN_ENTRIES, apply_porcelain_v2_gitlink_status_record, collect_unmerged_conflicts,
    conflict_kind_from_stage_mask, map_directory_entry_status, map_entry_status,
    map_porcelain_v2_status_char, merge_unmerged_conflicts, remove_conflicted_paths_from_staged,
    should_supplement_unmerged_conflicts, sort_and_dedup_status_entries, tree_id_for_commit,
};
use crate::test_support::{
    git_expect_failure, git_output, git_success, init_test_repo, open_repo, write_file,
};
use gitcomet_core::domain::{FileConflictKind, FileStatus, FileStatusKind};
use std::fs;
use std::path::{Path, PathBuf};

thread_local! {
    pub(super) static STAT_REFRESH_CLOCK: std::cell::Cell<Option<filetime::FileTime>> =
        const { std::cell::Cell::new(None) };
}

struct StatRefreshClock(Option<filetime::FileTime>);

impl StatRefreshClock {
    fn set(now: filetime::FileTime) -> Self {
        Self(STAT_REFRESH_CLOCK.with(|clock| clock.replace(Some(now))))
    }
}

impl Drop for StatRefreshClock {
    fn drop(&mut self) {
        STAT_REFRESH_CLOCK.with(|clock| clock.set(self.0));
    }
}

fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    }
}

#[test]
fn gitlink_query_excludes_ordinary_staged_paths_but_keeps_removed_and_replaced_links() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    init_test_repo(root);
    write_file(root, "ordinary.txt", "base\n");
    git_success(root, &["add", "ordinary.txt"]);
    git_success(root, &["commit", "-m", "base"]);
    let oid = crate::refs::head_oid(&gix::open(root).unwrap())
        .unwrap()
        .unwrap()
        .to_string();
    for path in ["removed", "replaced", "retained"] {
        git_success(
            root,
            &["update-index", "--add", "--cacheinfo", "160000", &oid, path],
        );
    }
    git_success(root, &["commit", "-m", "gitlinks"]);
    git_success(root, &["update-index", "--force-remove", "removed"]);
    git_success(root, &["update-index", "--force-remove", "replaced"]);
    write_file(root, "replaced", "regular file\n");
    write_file(root, "ordinary.txt", "changed\n");
    git_success(root, &["add", "replaced", "ordinary.txt"]);
    let staged = [
        file_status("ordinary.txt", FileStatusKind::Modified),
        file_status("removed", FileStatusKind::Deleted),
        file_status("replaced", FileStatusKind::Modified),
    ];
    assert_eq!(
        super::gitlink_status_paths(&open_repo(root).repo(), &staged),
        Some(
            ["removed", "replaced", "retained"]
                .map(PathBuf::from)
                .to_vec()
        )
    );
    git_success(root, &["update-index", "--force-remove", "retained"]);
    git_success(root, &["commit", "-m", "remove gitlinks"]);
    write_file(root, "ordinary.txt", "changed again\n");
    git_success(root, &["add", "ordinary.txt"]);
    assert_eq!(
        super::gitlink_status_paths(&open_repo(root).repo(), &staged[..1]),
        Some(Vec::new())
    );
}

fn conflicted_file_status(path: &str, conflict: FileConflictKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind: FileStatusKind::Conflicted,
        conflict: Some(conflict),
    }
}

fn setup_both_modified_text_conflict(workdir: &Path, path: &str) {
    init_test_repo(workdir);
    write_file(workdir, path, "base\n");
    git_success(workdir, &["add", path]);
    git_success(workdir, &["commit", "-m", "base"]);

    git_success(workdir, &["checkout", "-b", "feature"]);
    write_file(workdir, path, "theirs\n");
    git_success(workdir, &["commit", "-am", "theirs"]);

    git_success(workdir, &["checkout", "-"]);
    write_file(workdir, path, "ours\n");
    git_success(workdir, &["commit", "-am", "ours"]);

    let _ = git_expect_failure(workdir, &["merge", "feature"]);
}

#[test]
fn conflict_kind_from_stage_mask_covers_all_shapes() {
    assert_eq!(
        conflict_kind_from_stage_mask(0b001),
        Some(FileConflictKind::BothDeleted)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b010),
        Some(FileConflictKind::AddedByUs)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b011),
        Some(FileConflictKind::DeletedByThem)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b100),
        Some(FileConflictKind::AddedByThem)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b101),
        Some(FileConflictKind::DeletedByUs)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b110),
        Some(FileConflictKind::BothAdded)
    );
    assert_eq!(
        conflict_kind_from_stage_mask(0b111),
        Some(FileConflictKind::BothModified)
    );
    assert_eq!(conflict_kind_from_stage_mask(0), None);
}

#[test]
fn collect_unmerged_conflicts_groups_stage_entries_by_path() {
    let stages = vec![
        (PathBuf::from("dd.txt"), 1),
        (PathBuf::from("au.txt"), 2),
        (PathBuf::from("ud.txt"), 1),
        (PathBuf::from("ud.txt"), 2),
        (PathBuf::from("ua.txt"), 3),
        (PathBuf::from("du.txt"), 1),
        (PathBuf::from("du.txt"), 3),
        (PathBuf::from("aa.txt"), 2),
        (PathBuf::from("aa.txt"), 3),
        (PathBuf::from("uu.txt"), 1),
        (PathBuf::from("uu.txt"), 2),
        (PathBuf::from("uu.txt"), 3),
    ];

    let parsed = collect_unmerged_conflicts(stages);
    let by_path = parsed
        .into_iter()
        .collect::<FxHashMap<PathBuf, FileConflictKind>>();

    assert_eq!(
        by_path.get(&PathBuf::from("dd.txt")),
        Some(&FileConflictKind::BothDeleted)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("au.txt")),
        Some(&FileConflictKind::AddedByUs)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("ud.txt")),
        Some(&FileConflictKind::DeletedByThem)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("ua.txt")),
        Some(&FileConflictKind::AddedByThem)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("du.txt")),
        Some(&FileConflictKind::DeletedByUs)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("aa.txt")),
        Some(&FileConflictKind::BothAdded)
    );
    assert_eq!(
        by_path.get(&PathBuf::from("uu.txt")),
        Some(&FileConflictKind::BothModified)
    );
}

#[test]
fn collect_unmerged_conflicts_accepts_a_lazily_filtered_iterator() {
    // The natural way to drop the intermediate `Vec` in `gix_unmerged_conflicts`
    // is to hand the index scan straight in. A `Filter` reports `(0, Some(len))`
    // for its size hint, so the capacity hint must come from the lower bound --
    // taking the upper one would silently reserve the whole index here.
    let stages = [
        (PathBuf::from("src/lib.rs"), 0u8),
        (PathBuf::from("src/main.rs"), 1u8),
        (PathBuf::from("src/main.rs"), 2u8),
        (PathBuf::from("src/main.rs"), 3u8),
    ];

    let parsed = collect_unmerged_conflicts(
        stages
            .into_iter()
            .filter(|(_, stage)| (1..=3).contains(stage)),
    );

    assert_eq!(
        parsed,
        vec![(PathBuf::from("src/main.rs"), FileConflictKind::BothModified)]
    );
}

#[test]
fn collect_unmerged_conflicts_ignores_unconflicted_and_unknown_stages() {
    let stages = vec![
        (PathBuf::from("clean.txt"), 0),
        (PathBuf::from("ignored.txt"), 4),
        (PathBuf::from("conflicted.txt"), 2),
        (PathBuf::from("conflicted.txt"), 3),
    ];

    let parsed = collect_unmerged_conflicts(stages);
    assert_eq!(
        parsed,
        vec![(PathBuf::from("conflicted.txt"), FileConflictKind::BothAdded)]
    );
}

#[test]
fn sort_and_dedup_status_entries_prefers_highest_priority_kind_per_path() {
    let mut entries = vec![
        file_status("b.txt", FileStatusKind::Modified),
        file_status("a.txt", FileStatusKind::Untracked),
        file_status("a.txt", FileStatusKind::Deleted),
        file_status("c.txt", FileStatusKind::Modified),
        file_status("c.txt", FileStatusKind::Added),
        file_status("d.txt", FileStatusKind::Modified),
        file_status("d.txt", FileStatusKind::Renamed),
    ];

    sort_and_dedup_status_entries(&mut entries);

    assert_eq!(
        entries,
        vec![
            file_status("a.txt", FileStatusKind::Deleted),
            file_status("b.txt", FileStatusKind::Modified),
            file_status("c.txt", FileStatusKind::Added),
            file_status("d.txt", FileStatusKind::Renamed),
        ]
    );
}

#[test]
fn remove_conflicted_paths_from_staged_ignores_empty_input() {
    let expected = vec![file_status("a.txt", FileStatusKind::Modified)];
    let mut staged = expected.clone();

    remove_conflicted_paths_from_staged(&mut staged, std::iter::empty::<PathBuf>());

    assert_eq!(staged, expected);
}

#[test]
fn remove_conflicted_paths_from_staged_removes_only_matching_paths() {
    let mut staged = vec![
        file_status("a.txt", FileStatusKind::Modified),
        file_status("b.txt", FileStatusKind::Added),
        file_status("c.txt", FileStatusKind::Deleted),
    ];

    remove_conflicted_paths_from_staged(
        &mut staged,
        [PathBuf::from("b.txt"), PathBuf::from("missing.txt")],
    );

    assert_eq!(
        staged,
        vec![
            file_status("a.txt", FileStatusKind::Modified),
            file_status("c.txt", FileStatusKind::Deleted),
        ]
    );
}

#[test]
fn map_directory_entry_status_only_reports_untracked_entries() {
    use gix::dir::entry::Status;

    assert_eq!(
        map_directory_entry_status(Status::Untracked),
        Some(FileStatusKind::Untracked)
    );
    assert_eq!(map_directory_entry_status(Status::Tracked), None);
    assert_eq!(
        map_directory_entry_status(Status::Ignored(gix::ignore::Kind::Expendable)),
        None
    );
    assert_eq!(
        map_directory_entry_status(Status::Ignored(gix::ignore::Kind::Precious)),
        None
    );
    assert_eq!(map_directory_entry_status(Status::Pruned), None);
}

#[test]
fn map_entry_status_maps_all_conflict_summaries() {
    use gix::status::plumbing::index_as_worktree::{Conflict, EntryStatus};

    for (summary, expected) in [
        (Conflict::BothDeleted, FileConflictKind::BothDeleted),
        (Conflict::AddedByUs, FileConflictKind::AddedByUs),
        (Conflict::DeletedByThem, FileConflictKind::DeletedByThem),
        (Conflict::AddedByThem, FileConflictKind::AddedByThem),
        (Conflict::DeletedByUs, FileConflictKind::DeletedByUs),
        (Conflict::BothAdded, FileConflictKind::BothAdded),
        (Conflict::BothModified, FileConflictKind::BothModified),
    ] {
        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::Conflict {
                summary,
                entries: Box::new([None, None, None]),
            }),
            (FileStatusKind::Conflicted, Some(expected))
        );
    }
}

#[test]
fn map_entry_status_maps_non_conflict_variants() {
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::IntentToAdd),
        (FileStatusKind::Added, None)
    );
    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::NeedsUpdate(gix::index::entry::Stat::default())),
        (FileStatusKind::Modified, None)
    );
    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::Change(Change::Removed)),
        (FileStatusKind::Deleted, None)
    );
    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::Change(Change::Type {
            worktree_mode: gix::index::entry::Mode::FILE,
        })),
        (FileStatusKind::Modified, None)
    );
    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::Change(Change::Modification {
            executable_bit_changed: false,
            content_change: None,
            set_entry_stat_size_zero: false,
        })),
        (FileStatusKind::Modified, None)
    );
    assert_eq!(
        map_entry_status::<(), ()>(EntryStatus::Change(Change::SubmoduleModification(()))),
        (FileStatusKind::Modified, None)
    );
}

#[test]
fn map_porcelain_v2_status_char_maps_supported_values() {
    for (ch, expected) in [
        ('M', Some(FileStatusKind::Modified)),
        ('T', Some(FileStatusKind::Modified)),
        ('A', Some(FileStatusKind::Added)),
        ('D', Some(FileStatusKind::Deleted)),
        ('R', Some(FileStatusKind::Renamed)),
        ('U', Some(FileStatusKind::Conflicted)),
        ('.', None),
        ('?', None),
    ] {
        assert_eq!(map_porcelain_v2_status_char(ch), expected);
    }
}

#[test]
fn supplement_unmerged_conflicts_runs_for_in_progress_repo() {
    assert!(should_supplement_unmerged_conflicts(true, false));
}

#[test]
fn supplement_unmerged_conflicts_runs_for_reported_conflicts() {
    assert!(should_supplement_unmerged_conflicts(false, true));
}

#[test]
fn supplement_unmerged_conflicts_skips_clean_repo() {
    assert!(!should_supplement_unmerged_conflicts(false, false));
}

#[test]
fn porcelain_gitlink_record_maps_committed_unstaged_modification() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
            b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

    assert!(staged.is_empty());
    assert_eq!(unstaged.len(), 1);
    assert_eq!(unstaged[0].path, PathBuf::from("chess3"));
    assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
}

#[test]
fn porcelain_gitlink_record_maps_conflicted_status_chars_to_both_lanes() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
            b"1 UU SC.. 160000 160000 160000 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

    assert_eq!(
        staged,
        vec![file_status("chess3", FileStatusKind::Conflicted)]
    );
    assert_eq!(
        unstaged,
        vec![file_status("chess3", FileStatusKind::Conflicted)]
    );
}

#[test]
fn porcelain_gitlink_record_maps_added_and_unstaged_modified() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
            b"1 AM SC.. 000000 160000 160000 0000000000000000000000000000000000000000 2222222222222222222222222222222222222222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].path, PathBuf::from("chess3"));
    assert_eq!(staged[0].kind, FileStatusKind::Added);

    assert_eq!(unstaged.len(), 1);
    assert_eq!(unstaged[0].path, PathBuf::from("chess3"));
    assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
}

#[test]
fn porcelain_gitlink_record_ignores_non_type_one_records() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
        b"2 R. N... 160000 160000 160000 111 222 chess3",
        &mut staged,
        &mut unstaged,
    )
    .unwrap();

    assert!(staged.is_empty());
    assert!(unstaged.is_empty());
}

#[test]
fn porcelain_gitlink_record_ignores_non_gitlink_modes() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
        b"1 M. N... 100644 100644 100644 111 222 chess3",
        &mut staged,
        &mut unstaged,
    )
    .unwrap();

    assert!(staged.is_empty());
    assert!(unstaged.is_empty());
}

#[test]
fn porcelain_gitlink_record_ignores_missing_path() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
        b"1 M. SC.. 160000 160000 160000 111 222 ",
        &mut staged,
        &mut unstaged,
    )
    .unwrap();

    assert!(staged.is_empty());
    assert!(unstaged.is_empty());
}

#[test]
fn porcelain_gitlink_record_preserves_spaces_in_path() {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    apply_porcelain_v2_gitlink_status_record(
            b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 submodule with spaces",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

    assert!(staged.is_empty());
    assert_eq!(unstaged.len(), 1);
    assert_eq!(unstaged[0].path, PathBuf::from("submodule with spaces"));
    assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
}

#[cfg(unix)]
#[test]
fn porcelain_gitlink_record_preserves_non_utf8_path_bytes() {
    use std::os::unix::ffi::OsStrExt as _;

    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut record = b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 submodule-".to_vec();
    record.push(0xff);
    apply_porcelain_v2_gitlink_status_record(&record, &mut staged, &mut unstaged).unwrap();

    assert!(staged.is_empty());
    assert_eq!(unstaged.len(), 1);
    assert_eq!(unstaged[0].path.as_os_str().as_bytes(), b"submodule-\xff");
    assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
}

#[test]
fn status_impl_matches_lane_specific_statuses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "staged.txt", "base\n");
    write_file(workdir, "unstaged.txt", "base\n");
    git_success(workdir, &["add", "staged.txt", "unstaged.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    write_file(workdir, "staged.txt", "staged change\n");
    git_success(workdir, &["add", "staged.txt"]);
    write_file(workdir, "unstaged.txt", "unstaged change\n");
    write_file(workdir, "untracked.txt", "untracked\n");

    let gix_repo = open_repo(workdir);
    let combined = gix_repo.status_impl().expect("combined status");
    let staged = gix_repo
        .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("staged status");
    let unstaged = gix_repo
        .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("worktree status");

    assert_eq!(*combined.staged, staged);
    assert_eq!(*combined.unstaged, unstaged);
    assert_eq!(
        staged,
        vec![file_status("staged.txt", FileStatusKind::Modified)]
    );
    assert_eq!(
        unstaged,
        vec![
            file_status("unstaged.txt", FileStatusKind::Modified),
            file_status("untracked.txt", FileStatusKind::Untracked),
        ]
    );
}

#[test]
fn status_hides_filesystem_service_staging_areas() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "tracked.txt", "base\n");
    git_success(workdir, &["add", "tracked.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);
    write_file(workdir, ".gitcomet-operation-abc/item", "parked\n");
    write_file(
        workdir,
        "dir/.gitcomet-operation-def/recovery.log",
        "move\n",
    );
    write_file(workdir, "dir/.gitcomet-save-xyz", "staged\n");
    write_file(workdir, "dir/new.txt", "untracked\n");

    let gix_repo = open_repo(workdir);
    // The second call takes the cached-staged fast path.
    for _ in 0..2 {
        let status = gix_repo.status_impl().expect("status");
        assert_eq!(
            *status.unstaged,
            vec![file_status("dir/new.txt", FileStatusKind::Untracked)]
        );
    }
    assert_eq!(
        gix_repo
            .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("worktree status"),
        vec![file_status("dir/new.txt", FileStatusKind::Untracked)]
    );
}

#[test]
fn file_edited_after_staging_appears_in_both_staged_and_unstaged_lanes() {
    // Reproduces the reported state: a file that is staged AND then edited again in the
    // worktree (`MM` in `git status`) must appear in BOTH the staged and the unstaged lanes —
    // not only staged. This is the backend truth the UI's Unstaged section must reflect once
    // the file-watcher delivers the edit event.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "foo.txt", "v0\n");
    git_success(workdir, &["add", "foo.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    // Stage a modification...
    write_file(workdir, "foo.txt", "staged change\n");
    git_success(workdir, &["add", "foo.txt"]);
    // ...then edit the worktree again WITHOUT staging -> `MM`.
    write_file(workdir, "foo.txt", "further worktree edit\n");

    let gix_repo = open_repo(workdir);
    let status = gix_repo.status_impl().expect("combined status");
    assert_eq!(
        *status.staged,
        vec![file_status("foo.txt", FileStatusKind::Modified)],
        "a staged-and-re-edited file must appear in the staged lane"
    );
    assert_eq!(
        *status.unstaged,
        vec![file_status("foo.txt", FileStatusKind::Modified)],
        "the further worktree edit must also appear in the unstaged lane"
    );
    // The per-lane entry points must agree with the combined status.
    assert_eq!(
        gix_repo
            .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("staged lane"),
        *status.staged
    );
    assert_eq!(
        gix_repo
            .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("worktree lane"),
        *status.unstaged
    );
}

#[test]
fn staged_status_impl_on_unborn_head_uses_combined_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "new.txt", "new\n");
    git_success(workdir, &["add", "new.txt"]);

    let gix_repo = open_repo(workdir);
    let combined = gix_repo.status_impl().expect("combined status");
    let staged = gix_repo
        .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("staged status");

    assert_eq!(*combined.staged, staged);
    assert_eq!(staged, vec![file_status("new.txt", FileStatusKind::Added)]);
    assert!(combined.unstaged.is_empty());
}

#[test]
fn status_impl_removes_conflicted_paths_from_staged_lane() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    setup_both_modified_text_conflict(workdir, "tracked.txt");

    let gix_repo = open_repo(workdir);
    let combined = gix_repo.status_impl().expect("combined status");
    let staged = gix_repo
        .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("staged status");
    let worktree = gix_repo
        .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("worktree status");

    assert!(combined.staged.is_empty());
    assert!(staged.is_empty());
    assert_eq!(*combined.unstaged, worktree);
    assert_eq!(
        worktree,
        vec![conflicted_file_status(
            "tracked.txt",
            FileConflictKind::BothModified,
        )]
    );
}

#[test]
fn staged_status_impl_resolves_head_to_tree_before_tree_index_diff() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    fs::write(workdir.join("tracked.txt"), "base\n").expect("write tracked file");
    git_success(workdir, &["add", "tracked.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    fs::write(workdir.join("tracked.txt"), "base\nchanged\n").expect("rewrite tracked file");
    git_success(workdir, &["add", "tracked.txt"]);

    let gix_repo = open_repo(workdir);

    let head_commit_id =
        super::super::history::gix_head_id_or_none(&gix_repo.reopen_repo().expect("reopen repo"))
            .expect("head lookup")
            .expect("head commit");
    let head_tree_id = tree_id_for_commit(
        &gix_repo.reopen_repo().expect("reopen repo"),
        &head_commit_id,
    )
    .expect("head tree");
    assert_ne!(head_commit_id, head_tree_id);

    let staged = gix_repo
        .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
        .expect("staged status");
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].path, PathBuf::from("tracked.txt"));
    assert_eq!(staged[0].kind, FileStatusKind::Modified);
}

#[test]
fn read_index_trailing_hash_distinguishes_content_and_rejects_unusable_trailers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("index");
    let hash_len = gix::hash::Kind::Sha1.len_in_bytes();

    let trailing_hash = |bytes: &[u8]| {
        fs::write(&path, bytes).expect("write index");
        let mut file = std::fs::File::open(&path).expect("open index");
        super::read_index_trailing_hash(&mut file, bytes.len() as u64, gix::hash::Kind::Sha1)
    };

    // A plausible index layout: some body bytes followed by the trailing hash.
    let mut bytes = vec![7u8; 16 + hash_len];
    let trailer_start = bytes.len() - hash_len;
    bytes[trailer_start..].copy_from_slice(&[1u8; 20]);
    let hash_a = trailing_hash(&bytes).expect("trailing hash a");

    // Same length and (re)written file, but different trailing content.
    bytes[trailer_start..].copy_from_slice(&[2u8; 20]);
    let hash_b = trailing_hash(&bytes).expect("trailing hash b");
    assert_ne!(
        hash_a, hash_b,
        "different index content must yield different fingerprints"
    );

    // A null trailer (index.skipHash) is not a usable fingerprint and must be rejected so it
    // cannot make distinct index states compare equal.
    bytes[trailer_start..].copy_from_slice(&[0u8; 20]);
    assert!(
        trailing_hash(&bytes).is_none(),
        "a null trailing hash must be treated as absent"
    );

    // Files shorter than the hash length cannot carry a trailer.
    assert!(trailing_hash(&[0u8; 4]).is_none());

    // Missing files have no fingerprint.
    assert!(std::fs::File::open(tmp.path().join("missing")).is_err());
}

#[test]
fn gitlink_capability_cache_invalidates_on_index_content_change_with_matching_len_and_mtime() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "tracked.txt", "base\n");
    git_success(workdir, &["add", "tracked.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    let gix_repo = open_repo(workdir);
    let repo = gix_repo.reopen_repo().expect("reopen repo");

    // Real state: no .gitmodules and no gitlink entries → no submodule supplement needed.
    assert!(
        !gix_repo.may_have_gitlink_status_supplement(&repo, &super::repo_index_stamp(&repo)),
        "a plain repo should not need the gitlink/submodule supplement"
    );

    // Poison the capability cache to simulate a stale `may_have_gitlinks = true` that was
    // computed from a *different* index content sharing the current length + mtime (an atomic
    // rewrite that collided on a coarse/cached-timestamp filesystem such as f2fs). A
    // len+mtime-only stamp cannot tell the two index states apart.
    let stale_index_stamp = super::repo_file_stamp(repo.index_path().as_path());
    let gitmodules_stamp = super::repo_file_stamp(workdir.join(".gitmodules").as_path());
    *gix_repo
        .gitlink_status_capability
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(super::super::GitlinkStatusCapabilityCacheEntry {
            gitmodules: gitmodules_stamp,
            index: stale_index_stamp,
            may_have_gitlinks: true,
        });

    // The index fingerprint must reject the stale entry and recompute the real (false) answer
    // rather than serving the poisoned value.
    assert!(
        !gix_repo.may_have_gitlink_status_supplement(&repo, &super::repo_index_stamp(&repo)),
        "gitlink capability must not be served stale on an index len+mtime collision"
    );
}

#[cfg(unix)]
#[test]
fn index_stamp_is_uncacheable_when_index_is_present_but_unreadable() {
    // A unix socket is a deterministic stand-in for "the index exists and is stat-able, but
    // File::open fails" (the real-world cases being a momentary permission flip or a Windows
    // sharing/AV lock). open() on a socket fails with ENXIO even for root, while stat()
    // succeeds. A stat-only (len+mtime) fallback could collide with an atomic index rewrite of
    // the same length and an unchanged/coarse mtime and false-hit the cache — exactly the bug
    // the content discriminators exist to prevent. So an unreadable-but-present index must
    // instead yield an *uncacheable* stamp: two such stamps must never compare equal, forcing a
    // fresh read until the index is readable again.
    use std::os::unix::net::UnixListener;

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("index");
    let _listener = UnixListener::bind(&path).expect("bind unix socket as fake index");
    assert!(
        std::fs::File::open(&path).is_err(),
        "precondition: opening the socket file must fail"
    );
    assert!(
        std::fs::metadata(&path).is_ok(),
        "precondition: stat-ing the socket file must succeed"
    );

    let first = super::index_stamp_for(&path, gix::hash::Kind::Sha1);
    let second = super::index_stamp_for(&path, gix::hash::Kind::Sha1);
    assert_ne!(
        first, second,
        "two stamps taken while the index is unreadable must never compare equal, so the cache \
             is forced to miss rather than risk a stale hit"
    );
    assert_ne!(
        first,
        super::super::RepoFileStamp::default(),
        "an unreadable-index stamp must also differ from the absent-index default"
    );
}

#[cfg(unix)]
#[test]
fn index_stamp_detects_rewrites_without_content_hash_via_stat() {
    // Stand in for an `index.skipHash` repository (null trailer → no content fingerprint):
    // force `content_id` to `None` and confirm the inode/ctime discriminators still tell two
    // index revisions apart, so the staged cache cannot serve stale results there.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "tracked.txt", "base\n");
    git_success(workdir, &["add", "tracked.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    let gix_repo = open_repo(workdir);
    let stamp_before = super::repo_index_stamp(&gix_repo.reopen_repo().expect("reopen repo"));
    assert!(
        stamp_before.inode.is_some() && stamp_before.ctime_nanos.is_some(),
        "the index stamp should carry inode + ctime discriminators on Unix"
    );

    // Rewrite the index by staging a same-length change to the tracked file.
    write_file(workdir, "tracked.txt", "next\n");
    git_success(workdir, &["add", "tracked.txt"]);
    let stamp_after = super::repo_index_stamp(&gix_repo.reopen_repo().expect("reopen repo"));

    let without_hash_before = super::super::RepoFileStamp {
        content_id: None,
        ..stamp_before
    };
    let without_hash_after = super::super::RepoFileStamp {
        content_id: None,
        ..stamp_after
    };
    assert_ne!(
        without_hash_before, without_hash_after,
        "an index rewrite must change the stamp even without the content hash (skipHash case)"
    );
}

#[test]
fn staged_cache_invalidates_on_index_content_change_with_matching_len_and_mtime() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "tracked.txt", "base\n");
    git_success(workdir, &["add", "tracked.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    let gix_repo = open_repo(workdir);
    let repo = gix_repo.reopen_repo().expect("reopen repo");
    let head_oid = super::super::history::gix_head_id_or_none(&repo).expect("head lookup");
    let clean_stamp = super::repo_index_stamp(&repo);
    assert!(
        clean_stamp.content_id.is_some(),
        "the index fingerprint should be captured for a real repository"
    );

    // With a clean index nothing is staged; this also primes the cache.
    assert!(
        gix_repo
            .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("staged status")
            .is_empty()
    );

    // Reproduce the bug's precondition: a cached staged result whose stamp matches the
    // current index in length + mtime but reflects *different* index content (an atomic
    // rewrite that collided on a coarse/cached-timestamp filesystem such as f2fs). Only the
    // content fingerprint distinguishes the two.
    let stale_stamp = super::super::RepoFileStamp {
        content_id: Some(
            gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111")
                .expect("placeholder oid"),
        ),
        ..clean_stamp.clone()
    };
    *gix_repo
        .tree_index_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(super::super::TreeIndexCacheEntry {
            head_oid,
            index_stamp: stale_stamp,
            staged: vec![file_status("tracked.txt", FileStatusKind::Modified)],
        });

    // The content fingerprint differs, so the stale entry is rejected and the staged lane is
    // recomputed as empty instead of being served from the cache.
    assert!(
        gix_repo
            .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("recomputed staged status")
            .is_empty(),
        "a content-exact stamp must invalidate a stale staged cache on len+mtime collisions"
    );

    // The cache still works for an exact (content-matching) entry: a poisoned entry whose
    // stamp fully matches the current index is served verbatim, confirming we did not simply
    // disable caching.
    *gix_repo
        .tree_index_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(super::super::TreeIndexCacheEntry {
            head_oid,
            index_stamp: clean_stamp,
            staged: vec![file_status("sentinel.txt", FileStatusKind::Added)],
        });
    assert_eq!(
        gix_repo
            .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("cached staged status"),
        vec![file_status("sentinel.txt", FileStatusKind::Added)],
        "an exact stamp match should still hit the staged cache"
    );
}

#[test]
fn status_lanes_track_external_stage_unstage_transitions_across_the_cache() {
    // End-to-end check on a real repo: as external `git add` / `git reset` rewrite `.git/index`,
    // a file must move between the staged and unstaged lanes, and the staged cache (populated by
    // each `status_impl` call) must invalidate on every index rewrite so it never serves a
    // stale lane. This reproduces the original "file stuck in the wrong section" symptom.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "foo.txt", "v0\n");
    git_success(workdir, &["add", "foo.txt"]);
    git_success(workdir, &["commit", "-m", "initial"]);

    let gix_repo = open_repo(workdir);
    let modified = |kind| vec![file_status("foo.txt", kind)];

    // Stage a modification: foo is staged, nothing unstaged. Primes the staged cache.
    write_file(workdir, "foo.txt", "v1\n");
    git_success(workdir, &["add", "foo.txt"]);
    let status = gix_repo.status_impl().expect("status after staging");
    assert_eq!(*status.staged, modified(FileStatusKind::Modified));
    assert!(status.unstaged.is_empty());

    // Edit the worktree again WITHOUT staging: foo is now in BOTH lanes. The index is unchanged,
    // so the staged lane is served from the cache while the unstaged lane is recomputed.
    write_file(workdir, "foo.txt", "v2\n");
    let status = gix_repo.status_impl().expect("status with worktree edit");
    assert_eq!(
        *status.staged,
        modified(FileStatusKind::Modified),
        "an unstaged worktree edit must not disturb the staged lane (served from cache)"
    );
    assert_eq!(
        *status.unstaged,
        modified(FileStatusKind::Modified),
        "the new worktree edit must appear in the unstaged lane"
    );

    // Externally stage the new content (`git add`): foo leaves the unstaged lane but stays
    // staged. The index changed, so the cached staged result must be invalidated and recomputed.
    git_success(workdir, &["add", "foo.txt"]);
    let status = gix_repo.status_impl().expect("status after restaging");
    assert_eq!(*status.staged, modified(FileStatusKind::Modified));
    assert!(
        status.unstaged.is_empty(),
        "staging the worktree edit must clear the unstaged lane (cache must invalidate)"
    );

    // Externally unstage (`git reset`): foo leaves the staged lane and (re)enters the unstaged
    // lane — the exact transition the freshness bug got wrong. Again the cache must invalidate.
    git_success(workdir, &["reset", "HEAD", "--", "foo.txt"]);
    let status = gix_repo.status_impl().expect("status after unstaging");
    assert!(
        status.staged.is_empty(),
        "unstaging must remove foo from the staged lane (stale cache would keep it)"
    );
    assert_eq!(
        *status.unstaged,
        modified(FileStatusKind::Modified),
        "the unstaged file must appear in the unstaged lane"
    );

    // The per-lane entry points must agree with the combined status after all the churn.
    assert_eq!(
        gix_repo
            .staged_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("staged lane"),
        *status.staged
    );
    assert_eq!(
        gix_repo
            .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("worktree lane"),
        *status.unstaged
    );
}

/// The in-process pass has to beat two `git diff --numstat` spawns without
/// moving the status walk. Reports timings rather than asserting; run with
/// `-- --ignored --nocapture`.
#[test]
#[ignore = "timing probe"]
fn perf_uncommitted_line_stats_baseline() {
    // ~10 KB per file. The `status_dirty_500_files` fixture writes ~30
    // bytes, so it measures lstat throughput, not content I/O.
    fn body(seed: usize, marker: &str) -> String {
        let mut out = String::with_capacity(10_240);
        for line in 0..200 {
            out.push_str(&format!(
                "{marker} {seed:05} line {line:03} some representative source text here\n"
            ));
        }
        out
    }

    for (tracked, dirty, staged) in [(500usize, 25usize, 25usize), (500, 250, 0)] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        for index in 0..tracked {
            write_file(
                workdir,
                &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                &body(index, "base"),
            );
        }
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);

        for index in 0..dirty {
            write_file(
                workdir,
                &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                &body(index, "edit"),
            );
        }
        for index in dirty..dirty + staged {
            write_file(
                workdir,
                &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                &body(index, "edit"),
            );
        }
        if staged > 0 {
            git_success(workdir, &["add", "src"]);
            for index in 0..dirty {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                    &body(index, "again"),
                );
            }
        }

        let gix_repo = open_repo(workdir);
        // Warm the caches so the numbers compare work, not first touch.
        let _ = gix_repo.status_impl().expect("warmup status");
        let _ = git_output(workdir, &["diff", "--numstat", "-z"]);

        let _ = gix_repo
            .uncommitted_line_stats_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("warmup line stats");

        let mut status_ms = u128::MAX;
        let mut spawn_ms = u128::MAX;
        let mut in_process_ms = u128::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let status = gix_repo.status_impl().expect("status");
            status_ms = status_ms.min(start.elapsed().as_millis());
            std::hint::black_box(status);

            let start = std::time::Instant::now();
            let a = git_output(workdir, &["diff", "--numstat", "-z", "--no-renames"]);
            let b = git_output(
                workdir,
                &["diff", "--cached", "--numstat", "-z", "--no-renames"],
            );
            spawn_ms = spawn_ms.min(start.elapsed().as_millis());
            std::hint::black_box((a, b));

            let start = std::time::Instant::now();
            let stats = gix_repo
                .uncommitted_line_stats_impl(&gitcomet_core::services::CancellationToken::new())
                .expect("line stats");
            in_process_ms = in_process_ms.min(start.elapsed().as_millis());
            std::hint::black_box(stats);
        }

        println!(
            "tracked={tracked} dirty={dirty} staged={staged}: gix status {status_ms} ms | \
                 two numstat spawns {spawn_ms} ms | in-process pass {in_process_ms} ms (best of 5)"
        );
    }
}

/// The first full scan must warm the same cache that lane-only scans use.
#[test]
fn cold_status_retains_clean_file_stats_and_invalidates_them_after_staging() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    let timestamp = filetime::FileTime::now().unix_seconds() - 30;
    let touched = STAT_REFRESH_MIN_ENTRIES + 16;
    for ix in 0..touched {
        write_file(workdir, &format!("{ix:03}.txt"), "unchanged\n");
        filetime::set_file_mtime(
            workdir.join(format!("{ix:03}.txt")),
            filetime::FileTime::from_unix_time(timestamp, 0),
        )
        .unwrap();
    }
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-q", "-m", "seed"]);
    let index_before = std::fs::read(workdir.join(".git/index")).unwrap();
    for ix in 0..touched {
        filetime::set_file_mtime(
            workdir.join(format!("{ix:03}.txt")),
            filetime::FileTime::from_unix_time(timestamp + 1, 0),
        )
        .unwrap();
    }
    let repo = open_repo(workdir);
    let status = repo.status_impl().unwrap();
    assert!(status.staged.is_empty() && status.unstaged.is_empty());
    {
        let cache = repo.stat_refreshed_index.lock().unwrap();
        let cache = cache
            .as_ref()
            .expect("the very first status retains clean-file metadata");
        assert!(
            cache
                .index
                .entries()
                .iter()
                .all(|entry| entry.stat.mtime.secs == (timestamp + 1) as u32)
        );
    }
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    assert_eq!(
        std::fs::read(workdir.join(".git/index")).unwrap(),
        index_before,
        "status must not rewrite the disk index"
    );
    write_file(workdir, "000.txt", "different\n");
    assert_eq!(
        repo.status_impl().unwrap().unstaged.as_ref(),
        &[file_status("000.txt", FileStatusKind::Modified)]
    );
    git_success(workdir, &["add", "000.txt"]);
    let status = repo.status_impl().unwrap();
    assert!(status.unstaged.is_empty());
    assert_eq!(
        status.staged.as_ref(),
        &[file_status("000.txt", FileStatusKind::Modified)]
    );
}

#[cfg(unix)]
#[test]
fn repeated_status_does_not_run_clean_filters_again_after_the_first_scan() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, ".gitattributes", "large.txt filter=count\n");
    write_file(workdir, "large.txt", &"unchanged text\n".repeat(40_000));
    let counter = workdir.join(".git/hash-count");
    let clean = format!(
        "printf x >> '{}'; cat",
        counter.to_str().unwrap().replace('\'', "'\\''")
    );
    git_success(workdir, &["config", "filter.count.clean", &clean]);
    let timestamp = filetime::FileTime::now().unix_seconds() - 30;
    filetime::set_file_mtime(
        workdir.join("large.txt"),
        filetime::FileTime::from_unix_time(timestamp, 0),
    )
    .unwrap();
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-q", "-m", "seed"]);
    std::fs::remove_file(workdir.join(".git/hash-count")).unwrap();
    filetime::set_file_mtime(
        workdir.join("large.txt"),
        filetime::FileTime::from_unix_time(timestamp + 1, 0),
    )
    .unwrap();
    let repo = open_repo(workdir);
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    let hashes = std::fs::read(workdir.join(".git/hash-count")).unwrap();
    assert!(
        !hashes.is_empty(),
        "the initial scan verified the stale file metadata"
    );
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    assert_eq!(
        std::fs::read(workdir.join(".git/hash-count")).unwrap(),
        hashes,
        "warm status must skip expensive filtering and hashing of unchanged contents"
    );
    write_file(workdir, "large.txt", &"different text\n".repeat(40_000));
    assert_eq!(
        repo.status_impl().unwrap().unstaged[0].path,
        PathBuf::from("large.txt")
    );
    assert!(
        std::fs::read(workdir.join(".git/hash-count"))
            .unwrap()
            .len()
            > hashes.len(),
        "a subsequent edit is still verified"
    );
}

/// The in-memory stat refresh moves the index timestamp past every entry it
/// verified. A racily clean entry whose content did change must then keep
/// comparing as changed, so the refresh zeroes its size as git's index
/// write does; with only the new timestamp it would read as clean.

#[test]
fn stat_refresh_keeps_a_racily_modified_file_modified() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    // Rewriting the file below changes its ctime; the race is about mtime.
    git_success(workdir, &["config", "core.trustctime", "false"]);
    // Enough touched-but-unchanged files for the refresh to be kept.
    let touched = STAT_REFRESH_MIN_ENTRIES + 16;
    for index in 0..touched {
        write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
    }
    write_file(workdir, "racy.txt", "foo\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-q", "-m", "seed"]);

    let now = filetime::FileTime::now().unix_seconds();
    let entry_mtime = filetime::FileTime::from_unix_time(now - 50, 0);
    filetime::set_file_mtime(workdir.join("racy.txt"), entry_mtime).expect("mtime");
    git_success(workdir, &["update-index", "-q", "--refresh"]);
    // Same size and mtime as the index entry, different content.
    write_file(workdir, "racy.txt", "bar\n");
    filetime::set_file_mtime(workdir.join("racy.txt"), entry_mtime).expect("mtime");
    for index in 0..touched {
        write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
        filetime::set_file_mtime(
            workdir.join(format!("touched/{index:03}.txt")),
            filetime::FileTime::from_unix_time(now - 10, 0),
        )
        .expect("touched mtime");
    }
    // An index older than the entry makes it racy.
    filetime::set_file_mtime(
        workdir.join(".git/index"),
        filetime::FileTime::from_unix_time(now - 100, 0),
    )
    .expect("index mtime");

    let repo = open_repo(workdir);
    let expected = vec![file_status("racy.txt", FileStatusKind::Modified)];
    assert_eq!(
        repo.worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status"),
        expected
    );
    assert!(
        repo.stat_refreshed_index.lock().expect("lock").is_some(),
        "the touched files should have produced an in-memory refresh"
    );
    assert_eq!(
        repo.worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status"),
        expected
    );
}

/// A write landing right after a walk starts can carry a coarser mtime
/// from the second before, below the walk's nanosecond clock. Git's
/// index stamp is such an mtime itself; the in-memory stamp must stay
/// behind the walk's second so that write still compares racy.
#[test]
fn stat_refresh_catches_a_write_stamped_just_before_the_walk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    git_success(workdir, &["config", "core.trustctime", "false"]);
    let touched = STAT_REFRESH_MIN_ENTRIES + 16;
    for index in 0..touched {
        write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
    }
    write_file(workdir, "racy.txt", "foo\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-q", "-m", "seed"]);
    // Touched, so the walk produces the in-memory refresh.
    for index in 0..touched {
        write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
    }
    let repo = open_repo(workdir);
    // Freeze only this thread's walk clock. Filesystem timestamps below
    // are explicit, so a busy runner cannot move the walk into another
    // second or turn this regression into a scheduler timing assertion.
    let now = filetime::FileTime::from_unix_time(1_700_000_100, 100_000_000);
    let _clock = StatRefreshClock::set(now);
    // The stamp a write a few milliseconds after the walk starts can get
    // from the file clock: the last instant of the previous second.
    let just_before = filetime::FileTime::from_unix_time(now.unix_seconds() - 1, 999_000_000);
    filetime::set_file_mtime(workdir.join("racy.txt"), just_before).expect("mtime");
    assert_eq!(
        repo.worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status"),
        vec![]
    );
    assert_eq!(
        repo.stat_refreshed_index
            .lock()
            .expect("lock")
            .as_ref()
            .expect("stat refresh")
            .index
            .timestamp(),
        filetime::FileTime::from_unix_time(now.unix_seconds() - 1, 0),
    );

    // Same size and stat as the refreshed entry, different content.
    write_file(workdir, "racy.txt", "bar\n");
    filetime::set_file_mtime(workdir.join("racy.txt"), just_before).expect("mtime");
    assert_eq!(
        repo.worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status"),
        vec![file_status("racy.txt", FileStatusKind::Modified)]
    );
}

/// Worktree status after files were touched without changing content, as
/// build tools, formatters and editors saving unchanged buffers do. Status
/// never writes the index back, so each refresh re-hashes every such file
/// unless the refreshed stats are remembered.
#[test]
#[ignore = "timing probe"]
fn timing_worktree_status_after_touch() {
    // Process CPU time: the hashing runs on every core, so wall time on a
    // wide machine hides most of it.
    fn cpu_ms() -> f64 {
        let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
        let fields: Vec<&str> = stat
            .rsplit_once(')')
            .map(|(_, rest)| rest.split_whitespace().collect())
            .unwrap_or_default();
        let ticks: f64 = fields
            .get(11..13)
            .map(|t| t.iter().filter_map(|v| v.parse::<f64>().ok()).sum())
            .unwrap_or(0.0);
        ticks * 10.0
    }
    let body = "some representative source text here\n".repeat(256);
    for (files, attributes) in [(2_000usize, false), (10_000, false), (10_000, true)] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        if attributes {
            write_file(workdir, ".gitattributes", "* text=auto\n");
        }
        for index in 0..files {
            write_file(
                workdir,
                &format!("src/mod{:02}/file{index:05}.rs", index % 64),
                &body,
            );
        }
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);
        // Rewrite identical bytes: new mtimes (past git's one-second
        // granularity), same content.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        for index in 0..files {
            write_file(
                workdir,
                &format!("src/mod{:02}/file{index:05}.rs", index % 64),
                &body,
            );
        }
        let gix_repo = open_repo(workdir);
        let start = std::time::Instant::now();
        let first = gix_repo
            .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status");
        let first_ms = start.elapsed().as_secs_f64() * 1e3;
        assert!(first.is_empty(), "touched files are unchanged");
        // Refreshes are debounced (250 ms to 2 s): the next one starts in a
        // later second, when the touched files are no longer racy.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mut best = f64::MAX;
        let cpu_start = cpu_ms();
        for _ in 0..20 {
            let start = std::time::Instant::now();
            let again =
                gix_repo
                    .worktree_status_cancellable_impl(
                        &gitcomet_core::services::CancellationToken::new(),
                    )
                    .expect("status");
            best = best.min(start.elapsed().as_secs_f64() * 1e3);
            assert!(again.is_empty());
        }
        let repeat_cpu = (cpu_ms() - cpu_start) / 20.0;
        // Control: the same tree once git has written the new stats back.
        git_success(workdir, &["update-index", "-q", "--refresh"]);
        let fresh = open_repo(workdir);
        let _ = fresh
            .worktree_status_cancellable_impl(&gitcomet_core::services::CancellationToken::new())
            .expect("status");
        let mut clean = f64::MAX;
        let cpu_start = cpu_ms();
        for _ in 0..20 {
            let start = std::time::Instant::now();
            let _ =
                fresh
                    .worktree_status_cancellable_impl(
                        &gitcomet_core::services::CancellationToken::new(),
                    )
                    .expect("status");
            clean = clean.min(start.elapsed().as_secs_f64() * 1e3);
        }
        let clean_cpu = (cpu_ms() - cpu_start) / 20.0;
        println!(
            "timing worktree_status_after_touch files={files} text_auto={attributes} first={first_ms:.2}ms repeat_best={best:.2}ms repeat_cpu={repeat_cpu:.1}ms refreshed_index={clean:.2}ms refreshed_cpu={clean_cpu:.1}ms"
        );
    }
}

#[test]
fn merge_unmerged_conflicts_marks_the_first_entry_or_appends() {
    let entry = |path: &str, kind| FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    };
    let mut unstaged = vec![
        entry("a.txt", FileStatusKind::Modified),
        entry("b.txt", FileStatusKind::Deleted),
        entry("a.txt", FileStatusKind::Untracked),
    ];
    merge_unmerged_conflicts(
        &mut unstaged,
        vec![
            (PathBuf::from("a.txt"), FileConflictKind::BothModified),
            (PathBuf::from("c.txt"), FileConflictKind::AddedByUs),
            (PathBuf::from("b.txt"), FileConflictKind::DeletedByThem),
        ],
    );
    let summary: Vec<_> = unstaged
        .iter()
        .map(|e| (e.path.to_str().unwrap(), e.kind, e.conflict))
        .collect();
    assert_eq!(
        summary,
        [
            (
                "a.txt",
                FileStatusKind::Conflicted,
                Some(FileConflictKind::BothModified)
            ),
            (
                "b.txt",
                FileStatusKind::Conflicted,
                Some(FileConflictKind::DeletedByThem)
            ),
            ("a.txt", FileStatusKind::Untracked, None),
            (
                "c.txt",
                FileStatusKind::Conflicted,
                Some(FileConflictKind::AddedByUs)
            ),
        ]
    );
}

/// `cargo test -p gitcomet-git-gix --lib timing_merge_unmerged_conflicts --
/// --ignored --nocapture`: status refresh during a merge with many conflicts.
#[test]
#[ignore = "timing probe"]
fn timing_merge_unmerged_conflicts() {
    for (changed, conflicted) in [(2_000usize, 500usize), (20_000, 5_000)] {
        let unstaged: Vec<FileStatus> = (0..changed)
            .map(|ix| FileStatus {
                path: PathBuf::from(format!("src/module_{}/file_{ix}.rs", ix % 97)),
                kind: FileStatusKind::Modified,
                conflict: None,
            })
            .collect();
        // Half the conflicts are listed as modified, half are missing.
        let conflicts: Vec<(PathBuf, FileConflictKind)> = (0..conflicted)
            .map(|ix| {
                let listed = ix * 2 < conflicted;
                let ix = if listed { ix * 3 } else { changed + ix };
                (
                    PathBuf::from(format!("src/module_{}/file_{ix}.rs", ix % 97)),
                    FileConflictKind::BothModified,
                )
            })
            .collect();
        let mut samples = Vec::new();
        for _ in 0..5 {
            let mut entries = unstaged.clone();
            let started = std::time::Instant::now();
            merge_unmerged_conflicts(&mut entries, conflicts.clone());
            samples.push(started.elapsed());
            assert_eq!(entries.len(), changed + conflicted - conflicted.div_ceil(2));
        }
        samples.sort();
        println!(
            "timing merge_unmerged_conflicts changed {changed} conflicted {conflicted} p50 {:?}",
            samples[2]
        );
    }
}
