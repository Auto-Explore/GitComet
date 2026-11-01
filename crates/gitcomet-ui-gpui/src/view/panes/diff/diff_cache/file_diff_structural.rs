use super::{
    FileDiffCacheError, IndexedFileDiffSource, PagedFileDiffInlineRows, PagedFileDiffRows,
    StreamedFileDiffSource, build_file_diff_plan_from_document_sources, file_diff_source_plan_text,
    plan_line_to_inline_row_maps,
};
use gitcomet_core::diff_engine::{DiffEngine, DiffEngineFallback, DiffEngineStatus};
use gitcomet_core::diff_engine::{StructuralDiff, StructuralDiffOutcome};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::{FileDiffPlan, FileDiffRowKind};
use gitcomet_core::services::CancellationToken;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

#[cfg(test)]
#[path = "file_diff_structural_tests.rs"]
mod tests;

pub(super) struct Comparison {
    pub(super) plan: Option<FileDiffPlan>,
    pub(super) diff: Option<StructuralDiff>,
    pub(super) status: DiffEngineStatus,
}

pub(super) fn compare(
    file: &gitcomet_core::domain::FileDiffText,
    old: &mut IndexedFileDiffSource,
    new: &mut IndexedFileDiffSource,
    engine: DiffEngine,
    cancellation: &CancellationToken,
) -> Result<Comparison, FileDiffCacheError> {
    let mut result = Comparison {
        plan: None,
        diff: None,
        status: DiffEngineStatus::LineByLine,
    };
    if engine == DiffEngine::LineByLine {
        return Ok(result);
    }
    if old.source_len.max(new.source_len) > gitcomet_difftastic::BYTE_LIMIT {
        result.status = DiffEngineStatus::Fallback(DiffEngineFallback::ByteLimit);
        return Ok(result);
    }
    let old_text = file_diff_source_plan_text(old)?;
    let new_text = file_diff_source_plan_text(new)?;
    let outcome = gitcomet_difftastic::compare(
        gitcomet_difftastic::Input {
            path: &file.path,
            old: &old_text,
            new: &new_text,
        },
        cancellation,
    )
    .map_err(|gitcomet_difftastic::Cancelled| FileDiffCacheError::Cancelled)?;
    match outcome {
        StructuralDiffOutcome::Fallback(reason) => {
            result.status = DiffEngineStatus::Fallback(reason)
        }
        StructuralDiffOutcome::Compared(diff) => {
            result.plan = Some(gitcomet_core::file_diff::structural::plan(
                &diff, &old_text, &new_text,
            ));
            result.diff = Some(diff);
            result.status = DiffEngineStatus::Difftastic {
                syntactic_changes: true,
                text_differs: old_text != new_text,
            };
        }
        StructuralDiffOutcome::Unchanged => {
            result.plan = Some(build_file_diff_plan_from_document_sources(
                &old_text,
                &old.line_starts,
                &new_text,
                &new.line_starts,
            ));
            result.diff = Some(StructuralDiff {
                language: String::new(),
                aligned_lines: Vec::new(),
                old_changes: BTreeMap::new(),
                new_changes: BTreeMap::new(),
                blocks: Vec::new(),
            });
            result.status = DiffEngineStatus::Difftastic {
                syntactic_changes: false,
                text_differs: old_text != new_text,
            };
        }
    }
    if result.diff.is_some() {
        *old = IndexedFileDiffSource::from_shared(Some(&Arc::from(old_text.as_ref())));
        *new = IndexedFileDiffSource::from_shared(Some(&Arc::from(new_text.as_ref())));
    }
    Ok(result)
}

#[derive(Debug)]
pub(super) struct Presentation {
    diff: StructuralDiff,
    pub(super) split_blocks: Vec<Range<usize>>,
    pub(super) inline_blocks: Vec<Range<usize>>,
}

impl Presentation {
    /// Which side of an aligned pair holds its changed tokens: `Remove` for
    /// the old line, `Add` for the new, `Context` for neither.
    pub(super) fn changed_side(&self, old_ix: usize, new_ix: usize) -> DiffLineKind {
        match (
            self.diff.old_changes.contains_key(&old_ix),
            self.diff.new_changes.contains_key(&new_ix),
        ) {
            (true, false) => DiffLineKind::Remove,
            (false, true) => DiffLineKind::Add,
            _ => DiffLineKind::Context,
        }
    }
}

impl StreamedFileDiffSource {
    pub(super) fn apply_structural(&mut self, diff: StructuralDiff) {
        let (old_inline, new_inline) = plan_line_to_inline_row_maps(
            &self.plan,
            self.old_source.line_count(),
            self.new_source.line_count(),
        );
        for row in 0..self.split_len() {
            let source = self.split_row(row).expect("planned split row");
            let old = source
                .old_line
                .is_some_and(|line| diff.old_changes.contains_key(&(line as usize - 1)));
            let new = source
                .new_line
                .is_some_and(|line| diff.new_changes.contains_key(&(line as usize - 1)));
            self.split_visual_kinds[row] = match (old, new) {
                (true, true) => FileDiffRowKind::Modify,
                (true, false) => FileDiffRowKind::Remove,
                (false, true) => FileDiffRowKind::Add,
                (false, false) => FileDiffRowKind::Context,
            };
        }
        self.inline_visual_kinds.fill(DiffLineKind::Context);
        for (changes, rows, kind) in [
            (&diff.old_changes, &old_inline, DiffLineKind::Remove),
            (&diff.new_changes, &new_inline, DiffLineKind::Add),
        ] {
            for line in changes.keys() {
                if let Some(Some(row)) = rows.get(*line) {
                    self.inline_visual_kinds[*row] = kind;
                }
            }
        }
        let mut inline_bounds = Vec::with_capacity(self.split_len() + 1);
        let mut inline = 0;
        for run in &self.plan.runs {
            let step = run.inline_row_len() / run.row_len();
            for _ in 0..run.row_len() {
                inline_bounds.push(inline);
                inline += step;
            }
        }
        inline_bounds.push(inline);
        let inline_blocks = diff
            .blocks
            .iter()
            .map(|block| inline_bounds[block.start]..inline_bounds[block.end])
            .collect();
        self.structural = Some(Presentation {
            split_blocks: diff.blocks.clone(),
            inline_blocks,
            diff,
        });
    }
}

impl PagedFileDiffRows {
    pub(in crate::view) fn structural_ranges(
        &self,
        row: usize,
        old: bool,
    ) -> Option<Arc<[Range<usize>]>> {
        let structural = self.source.structural.as_ref()?;
        let source = self.source.split_row(row)?;
        let (line, changes) = if old {
            (source.old_line, &structural.diff.old_changes)
        } else {
            (source.new_line, &structural.diff.new_changes)
        };
        Some(
            line.and_then(|line| changes.get(&(line as usize - 1)))
                .cloned()
                .unwrap_or_default()
                .into(),
        )
    }
}

impl PagedFileDiffInlineRows {
    pub(in crate::view) fn structural_ranges(&self, row: usize) -> Option<Arc<[Range<usize>]>> {
        let structural = self.source.structural.as_ref()?;
        let source = self.source.inline_row(row)?;
        let (line, changes) = if source.kind == DiffLineKind::Remove {
            (source.old_line, &structural.diff.old_changes)
        } else {
            (source.new_line, &structural.diff.new_changes)
        };
        Some(
            line.and_then(|line| changes.get(&(line as usize - 1)))
                .cloned()
                .unwrap_or_default()
                .into(),
        )
    }
}
