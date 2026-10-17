use crate::diff::{AnnotatedDiffLine, annotate_unified};
use crate::domain::{Diff, DiffArea, DiffLineKind, DiffTarget};
use crate::partial_patch::{
    build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply,
};
use rustc_hash::FxHashSet;
use std::collections::BTreeSet;
use std::sync::Arc;

pub const SOURCE_BYTE_LIMIT: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchOperation {
    Stage,
    Unstage,
    Discard,
}

impl PatchOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Unstage => "Unstage",
            Self::Discard => "Discard",
        }
    }

    pub fn area(self) -> DiffArea {
        match self {
            Self::Stage | Self::Discard => DiffArea::Unstaged,
            Self::Unstage => DiffArea::Staged,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceLineSelection {
    pub old: BTreeSet<u32>,
    pub new: BTreeSet<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffActionSnapshot {
    pub diff: Diff,
    pub old: Arc<str>,
    pub new: Arc<str>,
    pub policy: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedDiffPatch {
    snapshot: DiffActionSnapshot,
    operation: PatchOperation,
    patch: Arc<[u8]>,
}

impl PreparedDiffPatch {
    pub fn patch(&self) -> &[u8] {
        &self.patch
    }
    pub fn target(&self) -> &DiffTarget {
        &self.snapshot.diff.target
    }
    pub fn operation(&self) -> PatchOperation {
        self.operation
    }
    pub fn validate(&self, current: &DiffActionSnapshot) -> Result<(), PatchActionError> {
        if &self.snapshot != current {
            return Err(PatchActionError::Stale);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PatchActionError {
    #[error("The comparison changed. Try again on the current diff.")]
    Stale,
    #[error("This selection has no applicable changed lines.")]
    Empty,
    #[error(
        "The exact Git patch does not match the displayed sources. Use Line by line to review this change."
    )]
    Unmapped,
    #[error("This operation is not available for the selected comparison.")]
    Target,
    #[error(
        "Partial structural actions require UTF-8 patch content. Use Line by line for this file."
    )]
    Encoding,
}

pub fn parse_exact_diff(target: DiffTarget, bytes: Vec<u8>) -> Result<Diff, PatchActionError> {
    let text = String::from_utf8(bytes).map_err(|_| PatchActionError::Encoding)?;
    let mut diff = Diff::from_unified(target, &text);
    for (line, raw) in diff.lines.iter_mut().zip(text.split_terminator('\n')) {
        line.text = raw.to_owned().into();
    }
    Ok(diff)
}

pub fn prepare(
    snapshot: DiffActionSnapshot,
    operation: PatchOperation,
    selection: &SourceLineSelection,
    displayed_old: &str,
    displayed_new: &str,
) -> Result<PreparedDiffPatch, PatchActionError> {
    let DiffTarget::WorkingTree { area, ref path, .. } = snapshot.diff.target else {
        return Err(PatchActionError::Target);
    };
    if area != operation.area() {
        return Err(PatchActionError::Target);
    }
    if snapshot.old.as_ref() != displayed_old || snapshot.new.as_ref() != displayed_new {
        return Err(PatchActionError::Stale);
    }
    if snapshot
        .diff
        .lines
        .iter()
        .filter(|line| line.text.starts_with("diff --git "))
        .count()
        != 1
    {
        return Err(PatchActionError::Unmapped);
    }
    let expected = path.to_str().ok_or(PatchActionError::Encoding)?;
    let expected = if cfg!(windows) {
        expected.replace('\\', "/")
    } else {
        expected.to_owned()
    };
    if crate::diff::unified_diff_file_path(
        snapshot.diff.lines.iter().map(|line| line.text.as_ref()),
    )
    .as_deref()
        != Some(&expected)
    {
        return Err(PatchActionError::Unmapped);
    }
    let rows = interleave_replacements(annotate_unified(&snapshot.diff));
    let old: Vec<_> = snapshot.old.split_terminator('\n').collect();
    let new: Vec<_> = snapshot.new.split_terminator('\n').collect();
    let mut selected = FxHashSet::default();
    for (index, row) in rows.iter().enumerate() {
        if row.text.raw_bytes().starts_with(b"\\") {
            continue;
        }
        if matches!(
            row.kind,
            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
        ) {
            let content = row
                .text
                .raw_bytes()
                .get(1..)
                .ok_or(PatchActionError::Unmapped)?;
            for (line, source) in [(row.old_line, &old), (row.new_line, &new)] {
                if let Some(line) = line
                    && source.get(line as usize - 1).map(|text| text.as_bytes()) != Some(content)
                {
                    return Err(PatchActionError::Unmapped);
                }
            }
        }
        if (row.kind == DiffLineKind::Remove
            && row
                .old_line
                .is_some_and(|line| selection.old.contains(&line)))
            || (row.kind == DiffLineKind::Add
                && row
                    .new_line
                    .is_some_and(|line| selection.new.contains(&line)))
        {
            selected.insert(index);
        }
    }
    let patch = match operation {
        PatchOperation::Stage => {
            build_unified_patch_for_selected_lines_across_hunks(&rows, &selected)
        }
        PatchOperation::Unstage | PatchOperation::Discard => {
            build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(&rows, &selected)
        }
    }
    .ok_or(PatchActionError::Empty)?;
    Ok(PreparedDiffPatch {
        snapshot,
        operation,
        patch: content_patch_headers(patch)?.into(),
    })
}

fn content_patch_headers(patch: Vec<u8>) -> Result<Vec<u8>, PatchActionError> {
    let text = std::str::from_utf8(&patch).map_err(|_| PatchActionError::Encoding)?;
    let mut output = Vec::with_capacity(patch.len());
    let mut start = 0;
    for (index, _) in text.match_indices("\ndiff --git ") {
        output.extend(content_patch_section_headers(&text[start..index + 1])?);
        start = index + 1;
    }
    output.extend(content_patch_section_headers(&text[start..])?);
    Ok(output)
}

fn content_patch_section_headers(text: &str) -> Result<Vec<u8>, PatchActionError> {
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let hunk = lines
        .iter()
        .position(|line| line.starts_with("@@ "))
        .ok_or(PatchActionError::Empty)?;
    let header = &lines[..hunk];
    let partial_presence = lines[hunk..].iter().any(|line| line.starts_with(' '));
    let old = header
        .iter()
        .find_map(|line| line.strip_prefix("--- "))
        .ok_or(PatchActionError::Unmapped)?;
    let new = header
        .iter()
        .find_map(|line| line.strip_prefix("+++ "))
        .ok_or(PatchActionError::Unmapped)?;
    let peer_path = |path: &str, from: char, to: char| {
        path.strip_prefix(&format!("{from}/"))
            .map(|tail| format!("{to}/{tail}"))
            .or_else(|| {
                path.strip_prefix(&format!("\"{from}/"))
                    .map(|tail| format!("\"{to}/{tail}"))
            })
            .ok_or(PatchActionError::Unmapped)
    };
    let mut output = String::with_capacity(text.len());
    for line in header {
        if line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("index ")
        {
            continue;
        }
        if line.starts_with("rename ") || line.starts_with("copy ") {
            return Err(PatchActionError::Unmapped);
        }
        if partial_presence
            && (line.starts_with("new file mode ") || line.starts_with("deleted file mode "))
        {
            continue;
        }
        if partial_presence && *line == "--- /dev/null\n" {
            output.push_str("--- ");
            output.push_str(&peer_path(new, 'b', 'a')?);
        } else if partial_presence && *line == "+++ /dev/null\n" {
            output.push_str("+++ ");
            output.push_str(&peer_path(old, 'a', 'b')?);
        } else {
            output.push_str(line);
        }
    }
    for line in &lines[hunk..] {
        output.push_str(line);
    }
    Ok(output.into_bytes())
}

fn interleave_replacements(rows: Vec<AnnotatedDiffLine>) -> Vec<AnnotatedDiffLine> {
    let mut result = Vec::with_capacity(rows.len());
    let mut cursor = 0;
    while cursor < rows.len() {
        if !matches!(rows[cursor].kind, DiffLineKind::Add | DiffLineKind::Remove) {
            result.push(rows[cursor].clone());
            cursor += 1;
            continue;
        }
        let mut old = Vec::new();
        let mut new = Vec::new();
        while cursor < rows.len()
            && matches!(rows[cursor].kind, DiffLineKind::Add | DiffLineKind::Remove)
        {
            let start = cursor;
            cursor += 1;
            if rows
                .get(cursor)
                .is_some_and(|line| line.text.raw_bytes().starts_with(b"\\"))
            {
                cursor += 1;
            }
            let side = if rows[start].kind == DiffLineKind::Remove {
                &mut old
            } else {
                &mut new
            };
            side.push(start..cursor);
        }
        for index in 0..old.len().max(new.len()) {
            for range in [old.get(index), new.get(index)].into_iter().flatten() {
                result.extend_from_slice(&rows[range.clone()]);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> DiffActionSnapshot {
        DiffActionSnapshot {
            diff: Diff::from_unified(
                DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
                "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,2 +1,2 @@\n-old\n-keep old\n+new\n+keep new\n",
            ),
            old: "old\nkeep old\n".into(),
            new: "new\nkeep new\n".into(),
            policy: Arc::from([]),
        }
    }

    #[test]
    fn side_only_selection_does_not_capture_the_paired_removal_or_neighbour() {
        let snapshot = snapshot();
        let selected = SourceLineSelection {
            new: BTreeSet::from([1]),
            ..Default::default()
        };
        let patch = prepare(
            snapshot.clone(),
            PatchOperation::Stage,
            &selected,
            &snapshot.old,
            &snapshot.new,
        )
        .unwrap();
        let text = std::str::from_utf8(patch.patch()).unwrap();
        assert!(text.contains("\n+new\n"));
        assert!(text.contains("\n old\n+new\n keep old\n"));
        assert!(!text.contains("\n-old\n"));
        assert!(!text.contains("keep new"));
        assert!(patch.validate(&snapshot).is_ok());
        let mut changed = snapshot;
        changed.new = "new\nchanged outside selection\n".into();
        assert_eq!(patch.validate(&changed), Err(PatchActionError::Stale));
    }

    #[test]
    fn changed_policy_and_stale_display_cannot_produce_an_applicable_patch() {
        let mut snapshot = snapshot();
        let selected = SourceLineSelection {
            old: BTreeSet::from([1]),
            new: BTreeSet::from([1]),
        };
        assert_eq!(
            prepare(
                snapshot.clone(),
                PatchOperation::Discard,
                &selected,
                "stale",
                &snapshot.new
            ),
            Err(PatchActionError::Stale)
        );
        let patch = prepare(
            snapshot.clone(),
            PatchOperation::Discard,
            &selected,
            &snapshot.old,
            &snapshot.new,
        )
        .unwrap();
        snapshot.policy = Arc::from(b"attributes changed".as_slice());
        assert_eq!(patch.validate(&snapshot), Err(PatchActionError::Stale));
    }
}
