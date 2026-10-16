use crate::diff::AnnotatedDiffLine;
use rustc_hash::FxHashSet;
use std::collections::BTreeMap;

pub fn enclosing_hunk_src_ix(diff: &[AnnotatedDiffLine], src_ix: usize) -> Option<usize> {
    let src_ix = src_ix.min(diff.len().saturating_sub(1));
    for ix in (0..=src_ix).rev() {
        let line = diff.get(ix)?;
        if matches!(line.kind, crate::domain::DiffLineKind::Header)
            && line.text.starts_with("diff --git ")
        {
            break;
        }
        if matches!(line.kind, crate::domain::DiffLineKind::Hunk) {
            return Some(ix);
        }
    }
    None
}

pub trait UnifiedDiffLine {
    fn kind(&self) -> crate::domain::DiffLineKind;
    fn text(&self) -> &str;
    /// The line as `git diff` printed it, in the file's encoding and with its
    /// CR — what a patch must carry for `git apply` to match.
    fn raw(&self) -> &[u8];
}

impl UnifiedDiffLine for AnnotatedDiffLine {
    fn kind(&self) -> crate::domain::DiffLineKind {
        self.kind
    }

    fn text(&self) -> &str {
        self.text.as_ref()
    }

    fn raw(&self) -> &[u8] {
        self.text.raw_bytes()
    }
}

impl UnifiedDiffLine for crate::domain::DiffLine {
    fn kind(&self) -> crate::domain::DiffLineKind {
        self.kind
    }

    fn text(&self) -> &str {
        self.text.as_ref()
    }

    fn raw(&self) -> &[u8] {
        self.text.raw_bytes()
    }
}

fn push_patch_line(out: &mut Vec<u8>, line: &[u8]) {
    out.extend_from_slice(line);
    out.push(b'\n');
}

fn patch_has_content(patch: &[u8]) -> bool {
    patch.iter().any(|byte| !byte.is_ascii_whitespace())
}

fn unified_patch_file_and_hunk_bounds<T: UnifiedDiffLine>(
    lines: &[T],
    hunk_src_ix: usize,
) -> Option<(usize, usize, usize)> {
    let hunk = lines.get(hunk_src_ix)?;
    if !matches!(hunk.kind(), crate::domain::DiffLineKind::Hunk) {
        return None;
    }

    let file_start = (0..=hunk_src_ix).rev().find(|&ix| {
        lines
            .get(ix)
            .is_some_and(|l| l.text().starts_with("diff --git "))
    })?;

    let first_hunk = (file_start + 1..lines.len())
        .find(|&ix| {
            let Some(line) = lines.get(ix) else {
                return false;
            };
            matches!(line.kind(), crate::domain::DiffLineKind::Hunk)
                || line.text().starts_with("diff --git ")
        })
        .unwrap_or(lines.len());

    let header_end = first_hunk.min(hunk_src_ix);
    let hunk_end = (hunk_src_ix + 1..lines.len())
        .find(|&ix| {
            let Some(line) = lines.get(ix) else {
                return false;
            };
            matches!(line.kind(), crate::domain::DiffLineKind::Hunk)
                || line.text().starts_with("diff --git ")
        })
        .unwrap_or(lines.len());

    Some((file_start, header_end, hunk_end))
}

fn unified_patch_capacity<T: UnifiedDiffLine>(
    lines: &[T],
    file_start: usize,
    header_end: usize,
    hunk_start: usize,
    hunk_end: usize,
) -> usize {
    lines[file_start..header_end]
        .iter()
        .map(|l| l.raw().len().saturating_add(1))
        .sum::<usize>()
        .saturating_add(
            lines[hunk_start..hunk_end]
                .iter()
                .map(|l| l.raw().len().saturating_add(1))
                .sum::<usize>(),
        )
}

/// The file header without `old mode`/`new mode`: the mode is a file-level
/// change, so a hunk or line patch must neither stage nor reset it.
fn push_hunk_patch_file_header(out: &mut Vec<u8>, header: &[impl UnifiedDiffLine]) {
    for line in header {
        let raw = line.raw();
        if !raw.starts_with(b"old mode ") && !raw.starts_with(b"new mode ") {
            push_patch_line(out, raw);
        }
    }
}

pub fn build_unified_patch_for_hunk(
    diff: &[impl UnifiedDiffLine],
    hunk_src_ix: usize,
) -> Option<Vec<u8>> {
    let lines = diff;
    let (file_start, header_end, hunk_end) =
        unified_patch_file_and_hunk_bounds(lines, hunk_src_ix)?;

    let capacity = unified_patch_capacity(lines, file_start, header_end, hunk_src_ix, hunk_end);
    let mut out = Vec::with_capacity(capacity);
    push_hunk_patch_file_header(&mut out, &lines[file_start..header_end]);
    for line in &lines[hunk_src_ix..hunk_end] {
        push_patch_line(&mut out, line.raw());
    }
    patch_has_content(&out).then_some(out)
}

pub fn build_unified_patch_for_hunks(
    diff: &[AnnotatedDiffLine],
    hunk_src_ixs: &[usize],
) -> Option<Vec<u8>> {
    if hunk_src_ixs.is_empty() {
        return None;
    }

    let mut hunks = hunk_src_ixs.to_vec();
    hunks.sort_unstable();
    hunks.dedup();

    let mut out = Vec::new();
    for hunk_src_ix in hunks {
        let Some(patch) = build_unified_patch_for_hunk(diff, hunk_src_ix) else {
            continue;
        };
        out.extend_from_slice(&patch);
    }

    patch_has_content(&out).then_some(out)
}

#[derive(Clone, Copy)]
enum UnselectedHunkLineBehavior {
    Drop,
    KeepAsContext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionPatchRole {
    Context,
    Remove,
    Add,
}

impl SelectionPatchRole {
    fn in_old_side(self) -> bool {
        matches!(self, Self::Context | Self::Remove)
    }

    fn in_new_side(self) -> bool {
        matches!(self, Self::Context | Self::Add)
    }
}

struct SelectionPatchLine<'a> {
    role: SelectionPatchRole,
    content: &'a [u8],
    /// The diff followed this line with `\ No newline at end of file`.
    no_newline: bool,
}

pub const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";

fn push_selection_patch_line(
    out: &mut Vec<u8>,
    prefix: u8,
    content: &[u8],
    no_newline_marker: bool,
) {
    out.push(prefix);
    push_patch_line(out, content);
    if no_newline_marker {
        push_patch_line(out, NO_NEWLINE_MARKER.as_bytes());
    }
}

/// A missing final newline belongs to whichever line ends a side. A selection
/// can move that end: keeping an unselected `-b` as context puts the staged
/// `+B` after it, so `b` ends the old side but not the new one. Such a line is
/// split into `-b`/`+b` so each side gets its own newline state.
fn push_selection_patch_body(out: &mut Vec<u8>, lines: &[SelectionPatchLine<'_>]) {
    let old_last = lines.iter().rposition(|line| line.role.in_old_side());
    let new_last = lines.iter().rposition(|line| line.role.in_new_side());
    for (ix, line) in lines.iter().enumerate() {
        let old_marker = line.no_newline && old_last == Some(ix);
        let new_marker = line.no_newline && new_last == Some(ix);
        match line.role {
            SelectionPatchRole::Context if old_marker == new_marker => {
                push_selection_patch_line(out, b' ', line.content, old_marker);
            }
            SelectionPatchRole::Context => {
                push_selection_patch_line(out, b'-', line.content, old_marker);
                push_selection_patch_line(out, b'+', line.content, new_marker);
            }
            SelectionPatchRole::Remove => {
                push_selection_patch_line(out, b'-', line.content, old_marker);
            }
            SelectionPatchRole::Add => {
                push_selection_patch_line(out, b'+', line.content, new_marker);
            }
        }
    }
}

fn build_unified_patch_for_hunk_selection_with_unselected_behavior(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &FxHashSet<usize>,
    unselected_add: UnselectedHunkLineBehavior,
    unselected_remove: UnselectedHunkLineBehavior,
) -> Option<Vec<u8>> {
    if selected_src_ixs.is_empty() {
        return None;
    }

    let lines = diff;
    let (file_start, header_end, hunk_end) =
        unified_patch_file_and_hunk_bounds(lines, hunk_src_ix)?;

    let capacity = unified_patch_capacity(lines, file_start, header_end, hunk_src_ix, hunk_end);
    let mut out = Vec::with_capacity(capacity);
    push_hunk_patch_file_header(&mut out, &lines[file_start..header_end]);

    // Keep the original hunk header; `git apply --recount` will adjust counts.
    push_patch_line(&mut out, lines[hunk_src_ix].raw());

    use crate::domain::DiffLineKind as K;
    let mut has_change = false;
    let mut body: Vec<SelectionPatchLine<'_>> = Vec::with_capacity(hunk_end - hunk_src_ix);
    for (ix, line) in lines
        .iter()
        .enumerate()
        .take(hunk_end)
        .skip(hunk_src_ix + 1)
    {
        let raw = line.raw();
        if raw.starts_with(b"\\") {
            continue;
        }
        let role = match line.kind {
            K::Add | K::Remove if selected_src_ixs.contains(&ix) => {
                has_change = true;
                if line.kind == K::Add {
                    SelectionPatchRole::Add
                } else {
                    SelectionPatchRole::Remove
                }
            }
            K::Add | K::Remove => {
                let unselected = if line.kind == K::Add {
                    unselected_add
                } else {
                    unselected_remove
                };
                match unselected {
                    UnselectedHunkLineBehavior::Drop => continue,
                    UnselectedHunkLineBehavior::KeepAsContext => SelectionPatchRole::Context,
                }
            }
            K::Context | K::Header | K::Hunk => SelectionPatchRole::Context,
        };
        body.push(SelectionPatchLine {
            role,
            content: raw.get(1..).unwrap_or_default(),
            no_newline: lines
                .get(ix + 1)
                .is_some_and(|next| next.raw().starts_with(b"\\")),
        });
    }

    push_selection_patch_body(&mut out, &body);
    has_change.then_some(out)
}

pub fn build_unified_patch_for_hunk_selection(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &FxHashSet<usize>,
) -> Option<Vec<u8>> {
    build_unified_patch_for_hunk_selection_with_unselected_behavior(
        diff,
        hunk_src_ix,
        selected_src_ixs,
        UnselectedHunkLineBehavior::Drop,
        UnselectedHunkLineBehavior::KeepAsContext,
    )
}

/// Patch for a selection that will be applied **in reverse**, where the side
/// that has to match is the diff's *new* side: the worktree for a discard, the
/// index for an unstage. Unselected additions are therefore kept as context
/// (they are present in that side and must survive) and unselected removals are
/// dropped (they are not). Applying the forward build in reverse instead makes
/// git reject the patch, because its context would describe the old side.
pub fn build_unified_patch_for_hunk_selection_for_reverse_apply(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &FxHashSet<usize>,
) -> Option<Vec<u8>> {
    build_unified_patch_for_hunk_selection_with_unselected_behavior(
        diff,
        hunk_src_ix,
        selected_src_ixs,
        UnselectedHunkLineBehavior::KeepAsContext,
        UnselectedHunkLineBehavior::Drop,
    )
}

/// Patch for a selection that will be applied **forward**, i.e. staging into the
/// index. See [`build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply`]
/// for the unstage/discard direction, which is not the same patch.
pub fn build_unified_patch_for_selected_lines_across_hunks(
    diff: &[AnnotatedDiffLine],
    selected_src_ixs: &FxHashSet<usize>,
) -> Option<Vec<u8>> {
    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for (hunk_src_ix, src_ixs) in selected_lines_by_hunk(diff, selected_src_ixs) {
        let Some(patch) = build_unified_patch_for_hunk_selection(diff, hunk_src_ix, &src_ixs)
        else {
            continue;
        };
        out.extend_from_slice(&patch);
    }

    patch_has_content(&out).then_some(out)
}

/// Multi-hunk form of [`build_unified_patch_for_hunk_selection_for_reverse_apply`].
pub fn build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
    diff: &[AnnotatedDiffLine],
    selected_src_ixs: &FxHashSet<usize>,
) -> Option<Vec<u8>> {
    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for (hunk_src_ix, src_ixs) in selected_lines_by_hunk(diff, selected_src_ixs) {
        let Some(patch) =
            build_unified_patch_for_hunk_selection_for_reverse_apply(diff, hunk_src_ix, &src_ixs)
        else {
            continue;
        };
        out.extend_from_slice(&patch);
    }

    patch_has_content(&out).then_some(out)
}

fn selected_lines_by_hunk(
    diff: &[AnnotatedDiffLine],
    selected: &FxHashSet<usize>,
) -> BTreeMap<usize, FxHashSet<usize>> {
    use crate::domain::DiffLineKind as K;
    let mut by_hunk: BTreeMap<usize, FxHashSet<usize>> = BTreeMap::new();
    let mut hunk = None;
    for (ix, line) in diff.iter().enumerate() {
        match line.kind {
            K::Header if line.text.starts_with("diff --git ") => hunk = None,
            K::Hunk => hunk = Some(ix),
            K::Add | K::Remove if selected.contains(&ix) => {
                if let Some(hunk) = hunk {
                    by_hunk.entry(hunk).or_default().insert(ix);
                }
            }
            _ => {}
        }
    }
    by_hunk
}

#[cfg(test)]
mod tests;
