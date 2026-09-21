use super::*;
use std::sync::Arc;

impl MainPaneView {
    pub(in crate::view) fn file_diff_inline_word_ranges(
        &mut self,
        inline_ix: usize,
    ) -> Arc<[Range<usize>]> {
        if let Some(ranges) = self.file_diff_inline_word_highlights.get(&inline_ix) {
            return Arc::clone(ranges);
        }

        if !matches!(
            self.file_diff_inline_visual_kind(inline_ix),
            gitcomet_core::domain::DiffLineKind::Add | gitcomet_core::domain::DiffLineKind::Remove
        ) {
            let empty: Arc<[Range<usize>]> = Arc::from(Vec::new());
            self.file_diff_inline_word_highlights
                .put(inline_ix, Arc::clone(&empty));
            return empty;
        }

        let ranges = self
            .file_diff_inline_modify_pair_texts(inline_ix)
            .map(|(old, new, kind)| {
                let (old_ranges, new_ranges) =
                    capped_word_diff_ranges_for_file_diff_texts(&old, &new);
                match kind {
                    gitcomet_core::domain::DiffLineKind::Remove => old_ranges,
                    gitcomet_core::domain::DiffLineKind::Add => new_ranges,
                    gitcomet_core::domain::DiffLineKind::Context
                    | gitcomet_core::domain::DiffLineKind::Header
                    | gitcomet_core::domain::DiffLineKind::Hunk => Vec::new(),
                }
            })
            .unwrap_or_default();
        let ranges: Arc<[Range<usize>]> = ranges.into();
        self.file_diff_inline_word_highlights
            .put(inline_ix, Arc::clone(&ranges));
        ranges
    }

    pub(in crate::view) fn file_diff_split_word_ranges(
        &mut self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> Arc<[Range<usize>]> {
        let is_left = match region {
            DiffTextRegion::SplitLeft => true,
            DiffTextRegion::SplitRight => false,
            DiffTextRegion::Inline => return Arc::from(Vec::new()),
        };

        if let Some(ranges) = self.file_diff_split_word_highlights.get(&row_ix) {
            return if is_left {
                Arc::clone(&ranges.old)
            } else {
                Arc::clone(&ranges.new)
            };
        }

        if !matches!(
            self.file_diff_split_visual_kind(row_ix),
            gitcomet_core::file_diff::FileDiffRowKind::Modify
        ) {
            let ranges = FileDiffSplitWordHighlights {
                old: Arc::from(Vec::new()),
                new: Arc::from(Vec::new()),
            };
            let empty = Arc::clone(&ranges.old);
            self.file_diff_split_word_highlights.put(row_ix, ranges);
            return empty;
        }

        let pair = self.file_diff_split_modify_pair_texts(row_ix).or_else(|| {
            let row = self.file_diff_cache_rows.get(row_ix)?;
            if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
                return None;
            }
            Some((row.old.clone()?, row.new.clone()?))
        });
        let (old_ranges, new_ranges) = pair
            .map(|(old, new)| capped_word_diff_ranges_for_file_diff_texts(&old, &new))
            .unwrap_or_default();

        let ranges = FileDiffSplitWordHighlights {
            old: old_ranges.into(),
            new: new_ranges.into(),
        };
        let selected = if is_left {
            Arc::clone(&ranges.old)
        } else {
            Arc::clone(&ranges.new)
        };
        self.file_diff_split_word_highlights.put(row_ix, ranges);
        selected
    }
}
