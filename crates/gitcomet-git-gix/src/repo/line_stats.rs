use super::log::{
    CommitStatsScratch, commit_file_line_stats, line_stats_from_bytes, read_commit_stats_blob,
};
use crate::util::path_buf_from_git_bytes;
use gitcomet_core::domain::{FileStatus, FileStatusKind, LineStats, UncommittedLineStats};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use rustc_hash::FxHashMap;
use std::path::PathBuf;

/// Mirrors the blob-side cap in `commit_stats`.
const WORKTREE_MAX_BYTES: u64 = 4 * 1024 * 1024;

impl super::GixRepo {
    pub(super) fn uncommitted_line_stats_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        let entries = self.worktree_status_cancellable_impl(cancellation)?;
        self.line_stats_for_entries_impl(&entries, cancellation)
    }

    pub(super) fn line_stats_for_entries_impl(
        &self,
        entries: &[FileStatus],
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        cancellation.check_cancelled()?;
        let repo = self.repo();
        let staged = staged_line_stats(&repo, cancellation)?;
        cancellation.check_cancelled()?;
        let unstaged = unstaged_line_stats(self, &repo, entries, cancellation)?;
        Ok(UncommittedLineStats { staged, unstaged })
    }
}

/// HEAD tree vs index. The walk hands us both blob ids, so this costs two
/// object reads per changed file and no worktree traversal.
fn staged_line_stats(
    repo: &gix::Repository,
    cancellation: &CancellationToken,
) -> Result<FxHashMap<PathBuf, LineStats>> {
    let mut out = FxHashMap::default();
    // `tree_index_status` wants a tree; an unborn HEAD measures against the
    // empty tree.
    let head_tree_id = match super::history::gix_head_id_or_none(repo)? {
        Some(head_oid) => super::status::tree_id_for_commit(repo, &head_oid)?,
        None => repo.empty_tree().id().detach(),
    };
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let mut scratch = CommitStatsScratch::default();

    let result = repo.tree_index_status(
        &head_tree_id,
        &index,
        None,
        gix::status::tree_index::TrackRenames::AsConfigured,
        |change, _, _| {
            use gix::diff::index::ChangeRef;
            cancellation.check_cancelled()?;
            let (location, old_id, new_id) = match change {
                ChangeRef::Addition { location, id, .. } => (location, None, Some(id.into_owned())),
                ChangeRef::Deletion { location, id, .. } => (location, Some(id.into_owned()), None),
                ChangeRef::Modification {
                    location,
                    previous_id,
                    id,
                    ..
                } => (
                    location,
                    Some(previous_id.into_owned()),
                    Some(id.into_owned()),
                ),
                // Both ids, so a rename counts the edit, not the whole file.
                ChangeRef::Rewrite {
                    location,
                    source_id,
                    id,
                    ..
                } => (
                    location,
                    Some(source_id.into_owned()),
                    Some(id.into_owned()),
                ),
            };
            let path = path_buf_from_git_bytes(location.as_ref(), "gix staged line stats path")?;
            out.insert(
                path,
                commit_file_line_stats(repo, old_id, new_id, &mut scratch).into(),
            );
            Ok::<_, Error>(std::ops::ControlFlow::Continue(()))
        },
    );
    // The walk wraps callback errors; preserve cancellation as its own error kind.
    cancellation.check_cancelled()?;
    result.map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix tree/index line stats: {e}"
        )))
    })?;

    Ok(out)
}

/// Index blob vs worktree file, the latter run through the worktree->git
/// filter pipeline so CRLF and clean filters apply before counting.
fn unstaged_line_stats(
    gix_repo: &super::GixRepo,
    repo: &gix::Repository,
    entries: &[FileStatus],
    cancellation: &CancellationToken,
) -> Result<FxHashMap<PathBuf, LineStats>> {
    let mut out = FxHashMap::default();
    if entries.is_empty() {
        return Ok(out);
    }

    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    // Only the old side is a blob, so `CommitStatsScratch` does not fit.
    let mut index_blob = Vec::new();
    let mut worktree = Vec::new();
    // Retain attribute caches and reusable filter processes across all files.
    let mut pipeline = repo.filter_pipeline(None).ok();

    for entry in entries.iter() {
        // Untracked is in neither index lane; conflicted has no single before.
        if matches!(
            entry.kind,
            FileStatusKind::Untracked | FileStatusKind::Conflicted
        ) {
            continue;
        }
        // Per file: this loop reads both sides of each one.
        cancellation.check_cancelled()?;
        let stats = unstaged_entry_line_stats(
            gix_repo,
            repo,
            &index,
            entry,
            &mut index_blob,
            &mut worktree,
            pipeline.as_mut(),
        );
        out.insert(entry.path.clone(), stats);
    }

    Ok(out)
}

fn unstaged_entry_line_stats(
    gix_repo: &super::GixRepo,
    repo: &gix::Repository,
    index: &gix::index::File,
    entry: &FileStatus,
    index_blob: &mut Vec<u8>,
    worktree: &mut Vec<u8>,
    pipeline: Option<&mut (
        gix::filter::Pipeline<'_>,
        gix::worktree::IndexPersistedOrInMemory,
    )>,
) -> LineStats {
    let rel = gix::path::into_bstr(entry.path.as_path());
    let index_id = index.entry_by_path(rel.as_ref()).map(|found| found.id);

    if !read_commit_stats_blob(repo, index_id, index_blob) {
        return LineStats::UNKNOWN;
    }

    worktree.clear();
    if entry.kind != FileStatusKind::Deleted
        && !read_worktree_git_bytes(gix_repo, pipeline, &entry.path, worktree)
    {
        return LineStats::UNKNOWN;
    }

    line_stats_from_bytes(index_blob.as_slice(), worktree.as_slice()).into()
}

/// Reads a worktree file as git would store it. `false` means over the size
/// cap or unreadable; binary is left to `line_stats_from_bytes`.
fn read_worktree_git_bytes(
    gix_repo: &super::GixRepo,
    pipeline: Option<&mut (
        gix::filter::Pipeline<'_>,
        gix::worktree::IndexPersistedOrInMemory,
    )>,
    relative: &std::path::Path,
    out: &mut Vec<u8>,
) -> bool {
    use std::io::Read as _;

    let full = gix_repo.spec.workdir.join(relative);
    let Ok(metadata) = std::fs::symlink_metadata(&full) else {
        // Vanished between the status walk and here; empty diffs as a deletion.
        return true;
    };
    if metadata.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(&full) else {
            return false;
        };
        out.extend_from_slice(gix::path::into_bstr(target).as_ref());
        return true;
    }
    if !metadata.is_file() || metadata.len() > WORKTREE_MAX_BYTES {
        return false;
    }
    let Some((pipeline, index)) = pipeline else {
        return false;
    };
    let Ok(file) = std::fs::File::open(&full) else {
        return false;
    };
    let Ok(converted) = pipeline.convert_to_git(file, relative, index) else {
        return false;
    };

    use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
    match converted {
        ToGitOutcome::Unchanged(mut file) => file.read_to_end(out).is_ok(),
        ToGitOutcome::Process(mut stream) => stream.read_to_end(out).is_ok(),
        ToGitOutcome::Buffer(bytes) => {
            out.extend_from_slice(bytes);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::status::tests::{git_success, init_test_repo, open_repo, write_file};

    #[cfg(unix)]
    #[test]
    fn unstaged_symlinks_have_counts() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        symlink("missing-before", dir.join("link")).unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::remove_file(dir.join("link")).unwrap();
        symlink("missing-after", dir.join("link")).unwrap();
        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        assert_eq!(
            stats.unstaged.get(std::path::Path::new("link")),
            Some(&LineStats {
                additions: Some(1),
                deletions: Some(1),
            })
        );
    }

    /// APFS and HFS+ refuse non-UTF-8 file names with `EILSEQ`, so this
    /// fixture cannot be created on macOS.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn unstaged_non_utf8_paths_have_counts() {
        use std::os::unix::ffi::OsStringExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"file-\xff.txt".to_vec()));
        std::fs::write(dir.join(&path), "before\n").unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::write(dir.join(&path), "after\n").unwrap();
        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        assert_eq!(
            stats.unstaged.get(&path),
            Some(&LineStats {
                additions: Some(1),
                deletions: Some(1),
            })
        );
    }

    #[test]
    fn shared_pipeline_applies_each_files_attributes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.txt text eol=crlf\nraw/* -text\n");
        write_file(dir, "a.txt", "base\n");
        write_file(dir, "nested/b.txt", "base\n");
        write_file(dir, "raw/c.txt", "base\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        for path in ["a.txt", "nested/b.txt", "raw/c.txt"] {
            write_file(dir, path, "base\r\nadded\r\n");
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
        for path in ["a.txt", "nested/b.txt"] {
            assert_eq!(
                stats.unstaged.get(std::path::Path::new(path)),
                Some(&LineStats {
                    additions: Some(1),
                    deletions: Some(0)
                })
            );
        }
        assert_eq!(
            stats.unstaged.get(std::path::Path::new("raw/c.txt")),
            Some(&LineStats {
                additions: Some(2),
                deletions: Some(1)
            })
        );
        // The supplied snapshot bounds the scan; it must not rediscover other changes.
        let status = gitcomet_core::domain::RepoStatus {
            unstaged: std::sync::Arc::new(entries[..1].to_vec()),
            staged: Default::default(),
        };
        let subset =
            gitcomet_core::services::GitRepository::uncommitted_line_stats_for_status_cancellable(
                &repo, &status, &token,
            )
            .unwrap();
        assert_eq!(subset.unstaged.len(), 1);
    }

    fn lines(prefix: &str, count: usize) -> String {
        (0..count)
            .map(|index| format!("{prefix} line {index}\n"))
            .collect()
    }

    #[test]
    fn counts_both_lanes_independently() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "staged.txt", &lines("base", 5));
        write_file(workdir, "unstaged.txt", &lines("base", 5));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        write_file(workdir, "staged.txt", &lines("base", 7));
        git_success(workdir, &["add", "staged.txt"]);
        write_file(workdir, "unstaged.txt", &lines("base", 4));

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");

        assert_eq!(
            stats.staged.get(std::path::Path::new("staged.txt")),
            Some(&LineStats {
                additions: Some(2),
                deletions: Some(0)
            })
        );
        assert_eq!(
            stats.unstaged.get(std::path::Path::new("unstaged.txt")),
            Some(&LineStats {
                additions: Some(0),
                deletions: Some(1)
            })
        );
        assert!(
            !stats
                .staged
                .contains_key(std::path::Path::new("unstaged.txt")),
            "an unstaged edit must not leak into the staged lane"
        );
    }

    /// Joining `git diff --numstat` by path would report the whole file as
    /// added; gix hands us both ids, so the counts are the real edit.
    #[test]
    fn a_staged_rename_reports_the_edit_not_the_whole_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "before.txt", &lines("base", 200));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        git_success(workdir, &["mv", "before.txt", "after.txt"]);
        write_file(workdir, "after.txt", &lines("base", 201));
        git_success(workdir, &["add", "after.txt"]);

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        let renamed = stats
            .staged
            .get(std::path::Path::new("after.txt"))
            .copied()
            .expect("renamed file has counts");

        assert_eq!(renamed.additions, Some(1), "only the appended line is new");
        assert_eq!(renamed.deletions, Some(0));
    }

    #[test]
    fn untracked_and_binary_report_no_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "seed.txt", "seed\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        std::fs::write(workdir.join("blob.bin"), b"\0\0\0binary\0\0").expect("write binary");
        git_success(workdir, &["add", "blob.bin"]);
        git_success(workdir, &["commit", "-m", "binary"]);
        std::fs::write(workdir.join("blob.bin"), b"\0\0\0changed\0\0").expect("edit binary");
        write_file(workdir, "brand-new.txt", "hello\n");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");

        assert_eq!(
            stats.unstaged.get(std::path::Path::new("blob.bin")),
            Some(&LineStats::UNKNOWN),
            "binary files are reported as unknown, not zero"
        );
        assert!(
            !stats
                .unstaged
                .contains_key(std::path::Path::new("brand-new.txt")),
            "untracked files are in neither index lane"
        );
    }

    /// A repo that closed mid-scan must not keep this running.
    #[test]
    fn a_cancelled_token_stops_the_scan() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "a.txt", &lines("base", 3));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        write_file(workdir, "a.txt", &lines("edit", 3));

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let err = open_repo(workdir)
            .uncommitted_line_stats_impl(&cancelled)
            .expect_err("a cancelled scan must not return counts");
        assert!(
            matches!(err.kind(), gitcomet_core::error::ErrorKind::Cancelled),
            "expected Cancelled, got {err:?}"
        );

        // Control: the guard is not just refusing everything.
        assert!(
            open_repo(workdir)
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .is_ok()
        );
    }

    #[test]
    fn staged_walk_preserves_cancellation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_test_repo(tmp.path());
        write_file(tmp.path(), "a.txt", &lines("staged", 3));
        git_success(tmp.path(), &["add", "."]);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let repo = open_repo(tmp.path()).repo();
        let err = staged_line_stats(&repo, &cancellation).expect_err("cancelled staged walk");
        assert!(matches!(err.kind(), ErrorKind::Cancelled), "{err:?}");
        assert_eq!(
            staged_line_stats(&repo, &CancellationToken::new())
                .expect("uncancelled staged walk")
                .len(),
            1
        );
    }

    #[test]
    fn staged_counts_work_on_an_unborn_head() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "first.txt", &lines("new", 4));
        git_success(workdir, &["add", "first.txt"]);

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        assert_eq!(
            stats.staged.get(std::path::Path::new("first.txt")),
            Some(&LineStats {
                additions: Some(4),
                deletions: Some(0)
            })
        );
    }

    #[test]
    fn a_deleted_worktree_file_counts_every_line_as_removed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "gone.txt", &lines("base", 3));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        std::fs::remove_file(workdir.join("gone.txt")).expect("remove");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        assert_eq!(
            stats.unstaged.get(std::path::Path::new("gone.txt")),
            Some(&LineStats {
                additions: Some(0),
                deletions: Some(3)
            })
        );
    }
}
