use super::*;

impl MainPaneView {
    pub(in crate::view) fn file_diff_inline_word_ranges(
        &mut self,
        inline_ix: usize,
    ) -> Vec<Range<usize>> {
        if let Some(ranges) = self.file_diff_inline_word_highlights.get(&inline_ix) {
            return ranges.clone();
        }

        if !matches!(
            self.file_diff_inline_visual_kind(inline_ix),
            gitcomet_core::domain::DiffLineKind::Add | gitcomet_core::domain::DiffLineKind::Remove
        ) {
            self.file_diff_inline_word_highlights
                .put(inline_ix, Vec::new());
            return Vec::new();
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
        self.file_diff_inline_word_highlights
            .put(inline_ix, ranges.clone());
        ranges
    }

    pub(in crate::view) fn file_diff_split_word_ranges(
        &mut self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> Vec<Range<usize>> {
        let is_left = match region {
            DiffTextRegion::SplitLeft => true,
            DiffTextRegion::SplitRight => false,
            DiffTextRegion::Inline => return Vec::new(),
        };

        if let Some(ranges) = self.file_diff_split_word_highlights.get(&row_ix) {
            return if is_left {
                ranges.old.clone()
            } else {
                ranges.new.clone()
            };
        }

        if !matches!(
            self.file_diff_split_visual_kind(row_ix),
            gitcomet_core::file_diff::FileDiffRowKind::Modify
        ) {
            let ranges = FileDiffSplitWordHighlights {
                old: Vec::new(),
                new: Vec::new(),
            };
            self.file_diff_split_word_highlights.put(row_ix, ranges);
            return Vec::new();
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
            old: old_ranges,
            new: new_ranges,
        };
        let selected = if is_left {
            ranges.old.clone()
        } else {
            ranges.new.clone()
        };
        self.file_diff_split_word_highlights.put(row_ix, ranges);
        selected
    }
}
