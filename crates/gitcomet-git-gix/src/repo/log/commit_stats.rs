use super::*;

pub(crate) const COMMIT_STATS_MAX_FILES: usize = 400;
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
fn read_commit_stats_blob(
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

/// Added/removed line counts between two blob versions; `(None, None)` when
/// either side is binary, too large, or unreadable.
pub(crate) fn commit_file_line_stats(
    repo: &gix::Repository,
    old_id: Option<gix::ObjectId>,
    new_id: Option<gix::ObjectId>,
    scratch: &mut CommitStatsScratch,
) -> (Option<u32>, Option<u32>) {
    if !read_commit_stats_blob(repo, old_id, &mut scratch.old)
        || !read_commit_stats_blob(repo, new_id, &mut scratch.new)
    {
        return (None, None);
    }
    let (old, new) = (scratch.old.as_slice(), scratch.new.as_slice());
    if commit_stats_looks_binary(old) || commit_stats_looks_binary(new) {
        return (None, None);
    }

    // One side empty means every line of the other side changed; skip the diff.
    if old.is_empty() || new.is_empty() {
        return (
            Some(commit_stats_line_count(new)),
            Some(commit_stats_line_count(old)),
        );
    }

    use gix::diff::blob::InternedInput;
    let input = InternedInput::new(old, new);
    let diff = gix::diff::blob::Diff::compute(gix::diff::blob::Algorithm::Histogram, &input);
    (Some(diff.count_additions()), Some(diff.count_removals()))
}

pub(crate) fn commit_file_change_from_diff(
    repo: &gix::Repository,
    change: gix::object::tree::diff::ChangeDetached,
    compute_stats: bool,
    scratch: &mut CommitStatsScratch,
) -> Result<Option<CommitFileChange>> {
    use gitcomet_core::domain::FileStatusKind;
    use gix::object::tree::diff::ChangeDetached;

    let (location, is_tree, is_submodule, kind, old_id, new_id) = match change {
        ChangeDetached::Addition {
            entry_mode,
            location,
            id,
            ..
        } => (
            location,
            entry_mode.is_tree(),
            entry_mode.is_commit(),
            FileStatusKind::Added,
            None,
            Some(id),
        ),
        ChangeDetached::Deletion {
            entry_mode,
            location,
            id,
            ..
        } => (
            location,
            entry_mode.is_tree(),
            entry_mode.is_commit(),
            FileStatusKind::Deleted,
            Some(id),
            None,
        ),
        ChangeDetached::Modification {
            previous_entry_mode,
            entry_mode,
            location,
            previous_id,
            id,
        } => (
            location,
            previous_entry_mode.is_tree() || entry_mode.is_tree(),
            previous_entry_mode.is_commit() || entry_mode.is_commit(),
            FileStatusKind::Modified,
            Some(previous_id),
            Some(id),
        ),
        ChangeDetached::Rewrite {
            source_entry_mode,
            entry_mode,
            location,
            copy,
            source_id,
            id,
            ..
        } => (
            location,
            source_entry_mode.is_tree() || entry_mode.is_tree(),
            source_entry_mode.is_commit() || entry_mode.is_commit(),
            if copy {
                FileStatusKind::Added
            } else {
                FileStatusKind::Renamed
            },
            Some(source_id),
            Some(id),
        ),
    };

    if is_tree {
        return Ok(None);
    }

    let (additions, deletions) = if compute_stats && !is_submodule {
        commit_file_line_stats(repo, old_id, new_id, scratch)
    } else {
        (None, None)
    };

    Ok(Some(CommitFileChange {
        path: path_buf_from_git_bytes(location.as_ref(), "gix commit details diff path")?,
        kind,
        is_submodule,
        additions,
        deletions,
    }))
}

/// Diff two trees (an absent `old_tree` means an empty tree, i.e. every path in
/// `new_tree` is an addition) into the flat `CommitFileChange` list used by both
/// commit details (parent → commit) and range comparisons (from → to).
pub(crate) fn tree_diff_file_changes(
    repo: &gix::Repository,
    old_tree: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
) -> Result<Vec<CommitFileChange>> {
    let changes = repo
        .diff_tree_to_tree(old_tree, new_tree, None)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix diff_tree_to_tree: {e}"))))?;

    let compute_stats = changes.len() <= COMMIT_STATS_MAX_FILES;
    let mut scratch = CommitStatsScratch::default();
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        if let Some(file) = commit_file_change_from_diff(repo, change, compute_stats, &mut scratch)?
        {
            files.push(file);
        }
    }
    Ok(files)
}

pub(crate) fn commit_file_changes(
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

    tree_diff_file_changes(repo, parent_tree.as_ref(), &commit_tree)
}

/// List the files that differ between two commits (`from` → `to`), for the
/// compare-selected-commits feature. `from` is the base/older side.
pub(crate) fn diff_range_files(
    repo: &gix::Repository,
    from: &CommitId,
    to: &CommitId,
) -> Result<Vec<CommitFileChange>> {
    // An absent base already means "no content" to the tree diff, which is
    // exactly what the empty tree stands for — so resolve it as absence rather
    // than through the object database, which is not guaranteed to hold it.
    let from_tree = (from.as_ref() != EMPTY_TREE_ID)
        .then(|| commit_tree_for_id(repo, from, "gix range from"))
        .transpose()?;
    let to_tree = commit_tree_for_id(repo, to, "gix range to")?;
    tree_diff_file_changes(repo, from_tree.as_ref(), &to_tree)
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
    repo.rev_parse_single(spec)
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
