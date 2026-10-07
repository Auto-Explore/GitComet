//! Comparisons shared by History, commit ranges, submodules, and hosted
//! views: which files differ between two points, with rename sources, object
//! ids, and modes; merge bases; ancestry.
//!
//! Commit-to-commit comparisons diff trees in-process; a working-tree tip
//! has no tree object, so those use `git diff --raw` against the worktree,
//! which is also what the unified diff in the main pane shows.

use super::GixRepo;
use super::log::diff_range_files;
use crate::util::{
    git_workdir_cmd_for, path_buf_from_git_bytes, run_git_capture_bytes_cancellable,
};
use gitcomet_core::domain::{CommitFileChange, CommitId, FileMode, FileStatusKind, ObjectHash};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CancellationToken, Comparison, ComparisonBase, ComparisonOptions, Result,
};
use std::path::{Path, PathBuf};

pub(super) type NumstatLineCounts = (Option<u32>, Option<u32>);
/// Lookup only (never iterated in order), so the unseeded hash map suffices.
pub(super) type NumstatCounts = rustc_hash::FxHashMap<PathBuf, NumstatLineCounts>;

/// One entry of a `git diff --raw` listing.
pub(super) struct RangeStatusChange {
    pub(super) path: PathBuf,
    pub(super) old_path: Option<PathBuf>,
    pub(super) kind: FileStatusKind,
    /// A gitlink on either side: a submodule pointer rather than a file.
    pub(super) is_submodule: bool,
    pub(super) old_id: Option<ObjectHash>,
    pub(super) new_id: Option<ObjectHash>,
    pub(super) old_mode: Option<FileMode>,
    pub(super) new_mode: Option<FileMode>,
}

/// Git's tree entry mode for a gitlink (a submodule pointer).
const GITLINK_ENTRY_MODE: &[u8] = b"160000";

pub(super) fn parse_numstat_field(field: &[u8]) -> Option<u32> {
    if field == b"-" {
        return None;
    }
    std::str::from_utf8(field).ok()?.parse::<u32>().ok()
}

fn next_non_empty_nul_field<'a, I>(fields: &mut I) -> Option<&'a [u8]>
where
    I: Iterator<Item = &'a [u8]>,
{
    fields.find(|field| !field.is_empty())
}

/// `git diff <from> [<to>]` in one of git's machine formats. Omitting `to`
/// compares `from` against the working tree, where git would write refreshed
/// stats back to the index even without optional locks; the watcher would
/// report that as an index change and reload every worktree view.
fn range_diff_command(
    workdir: &Path,
    format: &[&str],
    from: &CommitId,
    to: Option<&CommitId>,
) -> std::process::Command {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("-c")
        .arg("diff.autoRefreshIndex=false")
        .arg("diff")
        .args(format)
        .arg("-z")
        .arg("--find-renames")
        .arg(from.as_ref());
    if let Some(to) = to {
        command.arg(to.as_ref());
    }
    command
}

/// `--raw` rather than `--name-status` because the entry modes are the only
/// thing in a CLI diff that identifies a submodule pointer, and callers that
/// build `CommitFileChange` have to flag those the same way the gix tree-diff
/// path does.
pub(super) fn git_range_status_changes(
    workdir: &Path,
    from: &CommitId,
    to: Option<&CommitId>,
    cancellation: &CancellationToken,
) -> Result<Vec<RangeStatusChange>> {
    let command = range_diff_command(workdir, &["--raw", "--no-abbrev"], from, to);
    let label = "git diff --raw -z --find-renames";
    let output = run_git_capture_bytes_cancellable(command, label, cancellation)?;
    parse_raw_changes(&output, cancellation)
}

/// Parses `git diff --raw -z` records:
/// `:<srcmode> <dstmode> <srcsha> <dstsha> <status>\0<path>\0`, with renames
/// and copies adding the destination as a second path.
fn parse_raw_changes(
    output: &[u8],
    cancellation: &CancellationToken,
) -> Result<Vec<RangeStatusChange>> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(header) = next_non_empty_nul_field(&mut fields) {
        cancellation.check_cancelled()?;
        let Some(header) = header.strip_prefix(b":") else {
            continue;
        };
        let tokens: Vec<&[u8]> = header
            .split(|byte| *byte == b' ')
            .filter(|token| !token.is_empty())
            .collect();
        let [src_mode, dst_mode, src_id, dst_id, status_field] = tokens[..] else {
            continue;
        };
        let Some(status_code) = status_field.first().copied() else {
            continue;
        };
        let kind = match status_code {
            b'A' | b'C' => FileStatusKind::Added,
            b'D' => FileStatusKind::Deleted,
            b'R' => FileStatusKind::Renamed,
            b'U' => FileStatusKind::Conflicted,
            _ => FileStatusKind::Modified,
        };

        let (old_path_bytes, path_bytes) = if matches!(status_code, b'R' | b'C') {
            let old = next_non_empty_nul_field(&mut fields);
            (
                old,
                next_non_empty_nul_field(&mut fields).unwrap_or_default(),
            )
        } else {
            (
                None,
                next_non_empty_nul_field(&mut fields).unwrap_or_default(),
            )
        };
        if path_bytes.is_empty() {
            continue;
        }
        let old_path = old_path_bytes
            .map(|bytes| path_buf_from_git_bytes(bytes, "git diff --raw source path"))
            .transpose()?;

        let hex = |id: &[u8]| std::str::from_utf8(id).ok().and_then(ObjectHash::from_hex);
        changes.push(RangeStatusChange {
            path: path_buf_from_git_bytes(path_bytes, "git diff --raw path")?,
            old_path,
            kind,
            is_submodule: src_mode == GITLINK_ENTRY_MODE || dst_mode == GITLINK_ENTRY_MODE,
            old_id: hex(src_id),
            new_id: hex(dst_id),
            old_mode: FileMode::from_octal(src_mode),
            new_mode: FileMode::from_octal(dst_mode),
        });
    }
    Ok(changes)
}

pub(super) fn git_range_numstat_counts(
    workdir: &Path,
    from: &CommitId,
    to: Option<&CommitId>,
    cancellation: &CancellationToken,
) -> Result<NumstatCounts> {
    let command = range_diff_command(workdir, &["--numstat"], from, to);
    let label = "git diff --numstat -z --find-renames";
    let output = run_git_capture_bytes_cancellable(command, label, cancellation)?;

    let mut counts = NumstatCounts::default();
    let mut fields = output.split(|byte| *byte == 0);
    while let Some(record) = next_non_empty_nul_field(&mut fields) {
        cancellation.check_cancelled()?;
        let mut columns = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_field(columns.next().unwrap_or_default());
        let deletions = parse_numstat_field(columns.next().unwrap_or_default());
        let path_field = columns.next().unwrap_or_default();
        let path_bytes = if path_field.is_empty() {
            let _old_path = next_non_empty_nul_field(&mut fields);
            next_non_empty_nul_field(&mut fields).unwrap_or_default()
        } else {
            path_field
        };
        if path_bytes.is_empty() {
            continue;
        }
        counts.insert(
            path_buf_from_git_bytes(path_bytes, "git diff --numstat path")?,
            (additions, deletions),
        );
    }

    Ok(counts)
}

/// Untracked, not ignored files in the working tree.
fn untracked_paths(workdir: &Path, cancellation: &CancellationToken) -> Result<Vec<PathBuf>> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("ls-files")
        .arg("--others")
        .arg("--exclude-standard")
        .arg("-z");
    let output = run_git_capture_bytes_cancellable(
        command,
        "git ls-files --others --exclude-standard -z",
        cancellation,
    )?;
    output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| path_buf_from_git_bytes(field, "git ls-files path"))
        .collect()
}

/// The files that differ between commit `from` and the live working tree
/// (`git diff <from>`). Untracked files are listed only when asked for.
pub(super) fn commit_to_worktree_files(
    workdir: &Path,
    from: &CommitId,
    include_untracked: bool,
    cancellation: &CancellationToken,
) -> Result<Vec<CommitFileChange>> {
    let status_changes = git_range_status_changes(workdir, from, None, cancellation)?;
    let counts = git_range_numstat_counts(workdir, from, None, cancellation)?;
    let mut files: Vec<CommitFileChange> = status_changes
        .into_iter()
        // Without the index refresh `--raw` also lists a file whose stat
        // moved but not its content; numstat reads the content and leaves
        // it out, while every real change (mode-only, binary) is in it.
        .filter(|change| {
            change.kind != FileStatusKind::Modified
                || change.is_submodule
                || counts.contains_key(&change.path)
        })
        .map(|change| {
            let (additions, deletions) = counts.get(&change.path).cloned().unwrap_or((None, None));
            CommitFileChange::new(change.path, change.kind)
                .with_submodule(change.is_submodule)
                .with_line_counts(additions, deletions)
                .with_old_path(change.old_path)
                .with_ids(change.old_id, change.new_id)
                .with_modes(change.old_mode, change.new_mode)
        })
        .collect();
    if include_untracked {
        for path in untracked_paths(workdir, cancellation)? {
            // Counted from disk, under the tracked lanes' size cap.
            let stats = super::line_stats::read_worktree_file_capped(&workdir.join(&path))
                .map(|bytes| super::log::line_stats_from_bytes(&[], &bytes))
                .unwrap_or_default();
            files.push(
                CommitFileChange::new(path, FileStatusKind::Untracked).with_line_stats(stats),
            );
        }
    }
    Ok(files)
}

impl GixRepo {
    pub(super) fn compare_files_impl(
        &self,
        from: &CommitId,
        to: Option<&CommitId>,
        options: &ComparisonOptions,
        cancellation: &CancellationToken,
    ) -> Result<Comparison> {
        let base = match options.base {
            ComparisonBase::Direct => from.clone(),
            ComparisonBase::MergeBase => {
                let head = match to {
                    Some(to) => to.clone(),
                    None => super::history::gix_head_id_or_none(&self.repo())?
                        .map(|id| CommitId(id.to_string().into()))
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Backend(
                                "a merge-base comparison with the working tree needs a HEAD commit"
                                    .to_string(),
                            ))
                        })?,
                };
                self.merge_base_impl(from, &head)?.ok_or_else(|| {
                    Error::new(ErrorKind::Backend(format!(
                        "{from} and {head} have no merge base"
                    )))
                })?
            }
        };
        cancellation.check_cancelled()?;
        let files = match to {
            Some(to) => diff_range_files(self, &self.repo(), &base, to)?,
            None => {
                let mut files = commit_to_worktree_files(
                    &self.spec.workdir,
                    &base,
                    options.include_untracked,
                    cancellation,
                )?;
                self.add_worktree_edits(&mut files, cancellation)?;
                files
            }
        };
        cancellation.check_cancelled()?;
        Ok(Comparison::new(base, files))
    }

    pub(super) fn merge_base_impl(&self, a: &CommitId, b: &CommitId) -> Result<Option<CommitId>> {
        let repo = self.repo();
        let a = resolve_commit(&repo, a)?;
        let b = resolve_commit(&repo, b)?;
        Ok(merge_base_ids(&repo, a, b)?.map(|id| CommitId(id.to_string().into())))
    }

    pub(super) fn is_ancestor_impl(
        &self,
        ancestor: &CommitId,
        descendant: &CommitId,
    ) -> Result<bool> {
        let repo = self.repo();
        let ancestor = resolve_commit(&repo, ancestor)?;
        let descendant = resolve_commit(&repo, descendant)?;
        // `ancestor` is reachable from `descendant` exactly when it is their
        // merge base; a commit is its own merge base.
        Ok(merge_base_ids(&repo, ancestor, descendant)? == Some(ancestor))
    }
}

/// The best merge base, or `None` for unrelated histories. Through the
/// commit-graph when the repository has one enabled.
fn merge_base_ids(
    repo: &gix::Repository,
    a: gix::ObjectId,
    b: gix::ObjectId,
) -> Result<Option<gix::ObjectId>> {
    let cache = repo
        .commit_graph_if_enabled()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit-graph: {e}"))))?;
    let mut graph = repo.revision_graph(cache.as_ref());
    gix::revision::plumbing::merge_base(a, &[b], &mut graph)
        .map(|bases| bases.map(|bases| *bases.first()))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix merge-base: {e}"))))
}

fn resolve_commit(repo: &gix::Repository, id: &CommitId) -> Result<gix::ObjectId> {
    let spec = id.as_ref();
    crate::refs::resolve_required(repo, spec)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("rev-parse {spec}: {e}"))))?
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("object {spec}: {e}"))))?
        .peel_to_commit()
        .map(|commit| commit.id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{spec} is not a commit: {e}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comparison against the working tree is a read: git must not write
    /// refreshed stats back to the index (the watcher would report an index
    /// change and reload every worktree view), and a file whose stat moved
    /// without its content is not a change.
    #[test]
    fn comparison_against_the_worktree_does_not_write_the_index() {
        use crate::repo::status::tests::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "touched.txt", "unchanged content\n");
        write_file(dir, "edited.txt", "one\ntwo\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        // Stale stats: git would re-hash both and record the new stats.
        let stale =
            filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 120, 0);
        write_file(dir, "edited.txt", "one\n2\n");
        for name in ["touched.txt", "edited.txt"] {
            filetime::set_file_mtime(dir.join(name), stale).unwrap();
        }
        let index = dir.join(".git/index");
        let before = std::fs::read(&index).unwrap();

        let repo = open_repo(dir);
        let head = super::super::history::gix_head_id_or_none(&repo.repo())
            .unwrap()
            .unwrap();
        let files = commit_to_worktree_files(
            dir,
            &CommitId(head.to_string().into()),
            false,
            &CancellationToken::new(),
        )
        .unwrap();
        let listed: Vec<_> = files
            .iter()
            .map(|file| {
                (
                    file.path.as_path(),
                    file.kind,
                    file.additions,
                    file.deletions,
                )
            })
            .collect();
        assert_eq!(
            listed,
            vec![(
                Path::new("edited.txt"),
                FileStatusKind::Modified,
                Some(1),
                Some(1)
            )]
        );
        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "the comparison refreshed index stat metadata"
        );
    }

    /// Untracked files are counted from disk under the cap the tracked lanes
    /// use; past it they report no counts instead of being read whole.
    #[test]
    fn untracked_counts_stop_at_the_worktree_size_cap() {
        use crate::repo::status::tests::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "tracked.txt", "one\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "small.txt", "a\nb\n");
        let oversized =
            "line\n".repeat(super::super::line_stats::WORKTREE_MAX_BYTES as usize / 5 + 1);
        std::fs::write(dir.join("big.txt"), oversized).unwrap();

        let repo = open_repo(dir);
        let head = super::super::history::gix_head_id_or_none(&repo.repo())
            .unwrap()
            .unwrap();
        let files = commit_to_worktree_files(
            dir,
            &CommitId(head.to_string().into()),
            true,
            &CancellationToken::new(),
        )
        .unwrap();
        let counts = |name: &str| {
            let file = files
                .iter()
                .find(|file| file.path == Path::new(name))
                .unwrap();
            (file.kind, file.additions, file.deletions)
        };
        assert_eq!(
            counts("small.txt"),
            (FileStatusKind::Untracked, Some(2), Some(0))
        );
        assert_eq!(counts("big.txt"), (FileStatusKind::Untracked, None, None));
    }

    #[test]
    fn raw_records_keep_rename_sources_ids_and_modes() {
        let zero = "0".repeat(40);
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let output = format!(
            ":100644 100755 {a} {b} R087\0old/name.txt\0new/name.txt\0\
             :000000 160000 {zero} {b} A\0vendor/lib\0\
             :100644 100644 {a} {zero} M\0edited.txt\0"
        );
        let changes = parse_raw_changes(output.as_bytes(), &CancellationToken::new()).unwrap();
        assert_eq!(changes.len(), 3);

        let rename = &changes[0];
        assert_eq!(rename.path, PathBuf::from("new/name.txt"));
        assert_eq!(rename.old_path, Some(PathBuf::from("old/name.txt")));
        assert_eq!(rename.kind, FileStatusKind::Renamed);
        assert_eq!(rename.old_mode, Some(FileMode::Regular));
        assert_eq!(rename.new_mode, Some(FileMode::Executable));
        assert_eq!(rename.old_id.as_ref().map(AsRef::as_ref), Some(a.as_str()));

        let submodule = &changes[1];
        assert!(submodule.is_submodule);
        assert_eq!(submodule.old_id, None, "the zero id is no object");
        assert_eq!(submodule.old_mode, None);
        assert_eq!(submodule.new_mode, Some(FileMode::Gitlink));

        // A working-tree side has no object id yet.
        assert_eq!(changes[2].new_id, None);
        assert_eq!(changes[2].old_path, None);
    }
}
