use super::super::large_files::CommittedPointerScan;
use super::*;
use gitcomet_core::domain::{FileMode, LineStats, ObjectHash};
use gitcomet_core::edit_signature::EditSignatureBuilder;

pub(crate) const COMMIT_STATS_MAX_FILES: usize = 400;
/// Large-file badges cost ~1-2.5 us per file (measured on 20k-file commits),
/// far less than line stats, so asset imports keep them.
pub(crate) const COMMIT_POINTER_MAX_FILES: usize = 10_000;
/// Blobs larger than this are treated as "stats unknown" instead of diffed.
pub(crate) const COMMIT_STATS_MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;
/// Git's binary heuristic: a NUL byte within the leading window.
pub(crate) const COMMIT_STATS_BINARY_SNIFF_BYTES: usize = 8000;

/// Blob buffers reused across every file of one tree diff, so consecutive
/// files do not each allocate (and then drop) two fresh vectors.
#[derive(Default)]
pub(crate) struct CommitStatsScratch {
    old: Vec<u8>,
    new: Vec<u8>,
}

/// Loads one side's blob into `buf`. `false` means the side cannot be diffed
/// (not a blob, over the size cap, or unreadable). An absent side leaves `buf`
/// empty, which diffs as empty content.
pub(crate) fn read_commit_stats_blob(
    repo: &gix::Repository,
    id: Option<gix::ObjectId>,
    buf: &mut Vec<u8>,
) -> bool {
    buf.clear();
    let Some(id) = id.filter(|id| !id.is_null()) else {
        // No blob on this side (pure addition/deletion) diffs as empty content.
        return true;
    };
    // The header is cheap; inflating a multi-megabyte blob only to discard it
    // against the size cap was the dominant cost for large files.
    let Ok(header) = repo.find_header(id) else {
        return false;
    };
    if header.kind() != gix::object::Kind::Blob
        || header.size() > COMMIT_STATS_MAX_BLOB_BYTES as u64
    {
        return false;
    }
    repo.objects.find_blob(&id, buf).is_ok()
}

pub(crate) fn commit_stats_looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(COMMIT_STATS_BINARY_SNIFF_BYTES)].contains(&0)
}

pub(crate) fn commit_stats_line_count(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = memchr::memchr_iter(b'\n', bytes).count();
    let trailing = usize::from(*bytes.last().expect("checked non-empty") != b'\n');
    u32::try_from(newlines + trailing).unwrap_or(u32::MAX)
}

/// Added/removed line counts and the edit between two blob versions; unknown
/// when either side is binary, too large, or unreadable.
pub(crate) fn commit_file_line_stats(
    repo: &gix::Repository,
    old_id: Option<gix::ObjectId>,
    new_id: Option<gix::ObjectId>,
    scratch: &mut CommitStatsScratch,
) -> LineStats {
    if !read_commit_stats_blob(repo, old_id, &mut scratch.old)
        || !read_commit_stats_blob(repo, new_id, &mut scratch.new)
    {
        return LineStats::UNKNOWN;
    }
    line_stats_from_bytes(scratch.old.as_slice(), scratch.new.as_slice())
}

/// The counting core, over content both sides already hold. Shared with the
/// uncommitted lanes, whose new side is a worktree file rather than a blob.
/// The edit comes from the same diff: hashing its changed lines costs a
/// fraction of computing it.
pub(crate) fn line_stats_from_bytes(old: &[u8], new: &[u8]) -> LineStats {
    if commit_stats_looks_binary(old) || commit_stats_looks_binary(new) {
        return LineStats::UNKNOWN;
    }

    let mut edit = EditSignatureBuilder::default();
    // One side empty means every line of the other side changed; skip the diff.
    if old.is_empty() || new.is_empty() {
        edit.removed_lines(old);
        edit.added_lines(new);
        return LineStats {
            additions: Some(commit_stats_line_count(new)),
            deletions: Some(commit_stats_line_count(old)),
            edit: edit.finish(),
        };
    }

    use gix::diff::blob::InternedInput;
    let input = InternedInput::new(old, new);
    let diff = gix::diff::blob::Diff::compute(gix::diff::blob::Algorithm::Histogram, &input);
    for hunk in diff.hunks() {
        for &token in &input.before[hunk.before.start as usize..hunk.before.end as usize] {
            edit.removed(input.interner[token]);
        }
        for &token in &input.after[hunk.after.start as usize..hunk.after.end as usize] {
            edit.added(input.interner[token]);
        }
    }
    LineStats {
        additions: Some(diff.count_additions()),
        deletions: Some(diff.count_removals()),
        edit: edit.finish(),
    }
}

fn file_mode_from_entry(mode: gix::object::tree::EntryMode) -> Option<FileMode> {
    use gix::object::tree::EntryKind;
    match mode.kind() {
        EntryKind::Blob => Some(FileMode::Regular),
        EntryKind::BlobExecutable => Some(FileMode::Executable),
        EntryKind::Link => Some(FileMode::Symlink),
        EntryKind::Commit => Some(FileMode::Gitlink),
        EntryKind::Tree => None,
    }
}

pub(crate) fn commit_file_change_from_diff(
    repo: &gix::Repository,
    change: gix::object::tree::diff::ChangeDetached,
    compute_stats: bool,
    pointers: Option<&CommittedPointerScan>,
    scratch: &mut CommitStatsScratch,
) -> Result<Option<CommitFileChange>> {
    use gitcomet_core::domain::FileStatusKind;
    use gix::object::tree::diff::ChangeDetached;

    // `link`: the current (or deleted) entry is a symlink, as git-annex locks files.
    let (location, source, is_tree, is_submodule, kind, old_id, new_id, old_mode, new_mode, link) =
        match change {
            ChangeDetached::Addition {
                entry_mode,
                location,
                id,
                ..
            } => (
                location,
                None,
                entry_mode.is_tree(),
                entry_mode.is_commit(),
                FileStatusKind::Added,
                None,
                Some(id),
                None,
                Some(entry_mode),
                entry_mode.is_link(),
            ),
            ChangeDetached::Deletion {
                entry_mode,
                location,
                id,
                ..
            } => (
                location,
                None,
                entry_mode.is_tree(),
                entry_mode.is_commit(),
                FileStatusKind::Deleted,
                Some(id),
                None,
                Some(entry_mode),
                None,
                entry_mode.is_link(),
            ),
            ChangeDetached::Modification {
                previous_entry_mode,
                entry_mode,
                location,
                previous_id,
                id,
            } => (
                location,
                None,
                previous_entry_mode.is_tree() || entry_mode.is_tree(),
                previous_entry_mode.is_commit() || entry_mode.is_commit(),
                FileStatusKind::Modified,
                Some(previous_id),
                Some(id),
                Some(previous_entry_mode),
                Some(entry_mode),
                entry_mode.is_link(),
            ),
            ChangeDetached::Rewrite {
                source_location,
                source_entry_mode,
                entry_mode,
                location,
                copy,
                source_id,
                id,
                ..
            } => (
                location,
                Some(source_location),
                source_entry_mode.is_tree() || entry_mode.is_tree(),
                source_entry_mode.is_commit() || entry_mode.is_commit(),
                if copy {
                    FileStatusKind::Added
                } else {
                    FileStatusKind::Renamed
                },
                Some(source_id),
                Some(id),
                Some(source_entry_mode),
                Some(entry_mode),
                entry_mode.is_link(),
            ),
        };

    if is_tree {
        return Ok(None);
    }

    let stats = if compute_stats && !is_submodule {
        commit_file_line_stats(repo, old_id, new_id, scratch)
    } else {
        LineStats::UNKNOWN
    };

    let old_path = source
        .map(|source| path_buf_from_git_bytes(source.as_ref(), "gix rename source path"))
        .transpose()?;
    let large_file = pointers
        .filter(|_| !is_submodule)
        .zip(new_id.or(old_id))
        .and_then(|(pointers, id)| pointers.state(repo, id, link));
    let hash = |id: gix::ObjectId| ObjectHash(id.to_string().into());
    Ok(Some(
        CommitFileChange::new(
            path_buf_from_git_bytes(location.as_ref(), "gix commit details diff path")?,
            kind,
        )
        .with_submodule(is_submodule)
        .with_line_stats(stats)
        .with_old_path(old_path)
        .with_ids(old_id.map(hash), new_id.map(hash))
        .with_modes(
            old_mode.and_then(file_mode_from_entry),
            new_mode.and_then(file_mode_from_entry),
        )
        .with_large_file(large_file),
    ))
}

/// Diff two trees (an absent `old_tree` means an empty tree, i.e. every path in
/// `new_tree` is an addition) into the flat `CommitFileChange` list used by both
/// commit details (parent → commit) and range comparisons (from → to).
pub(crate) fn tree_diff_file_changes(
    owner: &GixRepo,
    repo: &gix::Repository,
    old_tree: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
) -> Result<Vec<CommitFileChange>> {
    let changes = crate::refs::diff_tree_to_tree(repo, old_tree, new_tree)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix diff_tree_to_tree: {e}"))))?;

    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let compute_stats = changes.len() <= COMMIT_STATS_MAX_FILES;
    let pointers =
        (changes.len() <= COMMIT_POINTER_MAX_FILES).then(|| owner.committed_pointer_scan(repo));
    let mut scratch = CommitStatsScratch::default();
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        if let Some(file) = commit_file_change_from_diff(
            repo,
            change,
            compute_stats,
            pointers.as_deref(),
            &mut scratch,
        )? {
            files.push(file);
        }
    }
    Ok(files)
}

pub(crate) fn commit_file_changes(
    owner: &GixRepo,
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    parent_ids: &[gix::ObjectId],
) -> Result<Vec<CommitFileChange>> {
    let commit_tree = commit
        .tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit tree: {e}"))))?;
    let parent_tree = match parent_ids.first() {
        None => None,
        Some(&id) => {
            // Shallow-boundary commits retain parent ids even though the parent
            // objects were intentionally not cloned. The comparison is
            // unavailable in that case, but the rest of the commit metadata is
            // still valid and should remain displayable.
            let Some(parent_object) = repo
                .try_find_object(id)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix parent commit: {e}"))))?
            else {
                return Ok(Vec::new());
            };
            let parent_commit = parent_object
                .try_into_commit()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix parent commit: {e}"))))?;
            Some(
                parent_commit
                    .tree()
                    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix parent tree: {e}"))))?,
            )
        }
    };

    tree_diff_file_changes(owner, repo, parent_tree.as_ref(), &commit_tree)
}

/// List the files that differ between two commits (`from` → `to`), for the
/// compare-selected-commits feature. `from` is the base/older side.
pub(crate) fn diff_range_files(
    owner: &GixRepo,
    repo: &gix::Repository,
    from: &CommitId,
    to: &CommitId,
) -> Result<Vec<CommitFileChange>> {
    // An absent base already means "no content" to the tree diff, which is
    // exactly what the empty tree stands for — so resolve it as absence rather
    // than through the object database, which is not guaranteed to hold it.
    let from_tree = (!is_empty_tree_id(from.as_ref()))
        .then(|| commit_tree_for_id(repo, from, "gix range from"))
        .transpose()?;
    let to_tree = commit_tree_for_id(repo, to, "gix range to")?;
    tree_diff_file_changes(owner, repo, from_tree.as_ref(), &to_tree)
}

/// Resolve a comparison endpoint to the tree it names. Peels to a tree rather
/// than to a commit so a bare tree spec resolves too — the empty tree is how the
/// changes a root commit introduces are expressed, and it is not a commit.
pub(crate) fn commit_tree_for_id<'repo>(
    repo: &'repo gix::Repository,
    id: &CommitId,
    context: &str,
) -> Result<gix::Tree<'repo>> {
    let spec = id.as_ref();
    crate::refs::resolve_required(repo, spec)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "{context} rev-parse {spec}: {e}"
            )))
        })?
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{context} object {spec}: {e}"))))?
        .peel_to_tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{context} peel {spec}: {e}"))))
}
