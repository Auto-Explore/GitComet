use crate::diff::AnnotatedDiffLine;
use rustc_hash::FxHashSet;

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

pub fn build_unified_patch_for_hunk(
    diff: &[impl UnifiedDiffLine],
    hunk_src_ix: usize,
) -> Option<Vec<u8>> {
    let lines = diff;
    let (file_start, header_end, hunk_end) =
        unified_patch_file_and_hunk_bounds(lines, hunk_src_ix)?;

    let capacity = unified_patch_capacity(lines, file_start, header_end, hunk_src_ix, hunk_end);
    let mut out = Vec::with_capacity(capacity);
    for line in &lines[file_start..header_end] {
        push_patch_line(&mut out, line.raw());
    }
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

fn append_unselected_hunk_line(
    out: &mut Vec<u8>,
    line: &[u8],
    expected_prefix: u8,
    behavior: UnselectedHunkLineBehavior,
    prev_included: &mut bool,
) {
    match behavior {
        UnselectedHunkLineBehavior::Drop => {
            *prev_included = false;
        }
        UnselectedHunkLineBehavior::KeepAsContext => {
            let content = line.strip_prefix(&[expected_prefix]).unwrap_or(line);
            out.push(b' ');
            push_patch_line(out, content);
            *prev_included = true;
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
    for line in &lines[file_start..header_end] {
        push_patch_line(&mut out, line.raw());
    }

    // Keep the original hunk header; `git apply --recount` will adjust counts.
    push_patch_line(&mut out, lines[hunk_src_ix].raw());

    let mut has_change = false;
    let mut prev_included = false;
    for (ix, line) in lines
        .iter()
        .enumerate()
        .take(hunk_end)
        .skip(hunk_src_ix + 1)
    {
        if line.raw().starts_with(b"\\") {
            if prev_included {
                push_patch_line(&mut out, line.raw());
            }
            continue;
        }

        match line.kind {
            crate::domain::DiffLineKind::Add => {
                if selected_src_ixs.contains(&ix) {
                    push_patch_line(&mut out, line.raw());
                    has_change = true;
                    prev_included = true;
                } else {
                    append_unselected_hunk_line(
                        &mut out,
                        line.raw(),
                        b'+',
                        unselected_add,
                        &mut prev_included,
                    );
                }
            }
            crate::domain::DiffLineKind::Remove => {
                if selected_src_ixs.contains(&ix) {
                    push_patch_line(&mut out, line.raw());
                    has_change = true;
                    prev_included = true;
                } else {
                    append_unselected_hunk_line(
                        &mut out,
                        line.raw(),
                        b'-',
                        unselected_remove,
                        &mut prev_included,
                    );
                }
            }
            crate::domain::DiffLineKind::Context
            | crate::domain::DiffLineKind::Header
            | crate::domain::DiffLineKind::Hunk => {
                push_patch_line(&mut out, line.raw());
                prev_included = true;
            }
        }
    }

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
    use crate::domain::DiffLineKind as K;
    use std::collections::BTreeMap;

    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut by_hunk: BTreeMap<usize, FxHashSet<usize>> = BTreeMap::new();
    for &src_ix in selected_src_ixs {
        let Some(line) = diff.get(src_ix) else {
            continue;
        };
        if !matches!(line.kind, K::Add | K::Remove) {
            continue;
        }
        let Some(hunk_src_ix) = enclosing_hunk_src_ix(diff, src_ix) else {
            continue;
        };
        by_hunk.entry(hunk_src_ix).or_default().insert(src_ix);
    }

    let mut out = Vec::new();
    for (hunk_src_ix, src_ixs) in by_hunk {
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
    use crate::domain::DiffLineKind as K;
    use std::collections::BTreeMap;

    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut by_hunk: BTreeMap<usize, FxHashSet<usize>> = BTreeMap::new();
    for &src_ix in selected_src_ixs {
        let Some(line) = diff.get(src_ix) else {
            continue;
        };
        if !matches!(line.kind, K::Add | K::Remove) {
            continue;
        }
        let Some(hunk_src_ix) = enclosing_hunk_src_ix(diff, src_ix) else {
            continue;
        };
        by_hunk.entry(hunk_src_ix).or_default().insert(src_ix);
    }

    let mut out = Vec::new();
    for (hunk_src_ix, src_ixs) in by_hunk {
        let Some(patch) =
            build_unified_patch_for_hunk_selection_for_reverse_apply(diff, hunk_src_ix, &src_ixs)
        else {
            continue;
        };
        out.extend_from_slice(&patch);
    }

    patch_has_content(&out).then_some(out)
}
