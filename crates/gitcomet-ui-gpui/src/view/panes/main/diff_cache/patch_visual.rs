use super::*;

pub(super) fn patch_diff_content_signature(diff: &gitcomet_core::domain::Diff) -> u64 {
    use std::hash::Hasher;

    let mut hasher = FxHasher::default();
    hasher.write_usize(diff.lines.len());
    for line in diff.lines.iter() {
        let kind = match line.kind {
            gitcomet_core::domain::DiffLineKind::Header => 0,
            gitcomet_core::domain::DiffLineKind::Hunk => 1,
            gitcomet_core::domain::DiffLineKind::Add => 2,
            gitcomet_core::domain::DiffLineKind::Remove => 3,
            gitcomet_core::domain::DiffLineKind::Context => 4,
        };
        hasher.write_u8(kind);
        hasher.write_usize(line.text.len());
        hasher.write(line.text.as_ref().as_bytes());
    }
    hasher.finish()
}

pub(super) fn append_non_whitespace(text: &str, out: &mut String) {
    out.extend(text.chars().filter(|ch| !ch.is_whitespace()));
}

pub(super) fn diff_line_content_text(line: &gitcomet_core::domain::DiffLine) -> &str {
    match line.kind {
        gitcomet_core::domain::DiffLineKind::Add => {
            line.text.strip_prefix('+').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Remove => {
            line.text.strip_prefix('-').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Context => {
            line.text.strip_prefix(' ').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Header | gitcomet_core::domain::DiffLineKind::Hunk => {
            &line.text
        }
    }
}

pub(super) fn is_unified_no_newline_marker(text: &str) -> bool {
    text.starts_with("\\ No newline")
}

pub(super) fn is_patch_diff_whitespace_group_line(line: &gitcomet_core::domain::DiffLine) -> bool {
    matches!(
        line.kind,
        gitcomet_core::domain::DiffLineKind::Remove | gitcomet_core::domain::DiffLineKind::Add
    ) || is_unified_no_newline_marker(&line.text)
}

pub(super) fn visual_line_kinds_for_patch_diff(
    diff: &gitcomet_core::domain::Diff,
    mode: DiffWhitespaceMode,
) -> Vec<gitcomet_core::domain::DiffLineKind> {
    use gitcomet_core::domain::DiffLineKind as DK;

    let mut visual = diff.lines.iter().map(|line| line.kind).collect::<Vec<_>>();
    if mode == DiffWhitespaceMode::Show {
        return visual;
    }

    let mut ix = 0usize;
    while ix < diff.lines.len() {
        if !matches!(diff.lines[ix].kind, DK::Remove | DK::Add) {
            ix += 1;
            continue;
        }

        let group_start = ix;
        let mut old_stripped = String::new();
        let mut new_stripped = String::new();
        while ix < diff.lines.len() && is_patch_diff_whitespace_group_line(&diff.lines[ix]) {
            let line = &diff.lines[ix];
            match line.kind {
                DK::Remove => {
                    append_non_whitespace(diff_line_content_text(line), &mut old_stripped)
                }
                DK::Add => append_non_whitespace(diff_line_content_text(line), &mut new_stripped),
                DK::Context | DK::Header | DK::Hunk => {}
            }
            ix += 1;
        }

        if old_stripped == new_stripped {
            for kind in &mut visual[group_start..ix] {
                *kind = DK::Context;
            }
        }
    }

    visual
}

impl MainPaneView {
    pub(super) fn rebuild_patch_visual_line_kinds_from_ready_diff(
        &mut self,
        diff: &gitcomet_core::domain::Diff,
    ) {
        self.diff_visual_line_kind_for_src_ix =
            visual_line_kinds_for_patch_diff(diff, self.diff_whitespace_mode);
    }

    pub(in crate::view) fn rebuild_patch_visual_line_kinds_from_current_diff(&mut self) {
        let ready_diff = match self.rendered_patch_diff_loadable() {
            Some(Loadable::Ready(diff)) => Some(Arc::clone(diff)),
            _ => None,
        };
        if let Some(diff) = ready_diff {
            self.rebuild_patch_visual_line_kinds_from_ready_diff(diff.as_ref());
        } else {
            self.diff_visual_line_kind_for_src_ix = self.diff_line_kind_for_src_ix.clone();
        }
    }

    pub(in crate::view) fn patch_diff_rows_slice(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<AnnotatedDiffLine> {
        if let Some(provider) = self.diff_row_provider.as_ref() {
            provider.slice(start, end).collect()
        } else {
            let end = end.min(self.diff_cache.len());
            if start >= end {
                Vec::new()
            } else {
                self.diff_cache[start..end].to_vec()
            }
        }
    }

    pub(in crate::view) fn patch_diff_split_row_len(&self) -> usize {
        self.diff_split_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.diff_split_cache.len())
    }

    pub(in crate::view) fn patch_diff_split_row(&self, row_ix: usize) -> Option<PatchSplitRow> {
        if let Some(provider) = self.diff_split_row_provider.as_ref() {
            provider.row(row_ix)
        } else {
            self.diff_split_cache.get(row_ix).cloned()
        }
    }

    pub(super) fn patch_split_visible_meta_from_source(&self) -> PatchSplitVisibleMeta {
        build_patch_split_visible_meta_from_src(
            self.diff_line_kind_for_src_ix.as_slice(),
            self.diff_visual_line_kind_for_src_ix.as_slice(),
            self.diff_click_kinds.as_slice(),
            self.diff_hide_unified_header_for_src_ix.as_slice(),
        )
    }

    pub(in crate::view) fn ensure_patch_diff_word_highlight_for_src_ix(&mut self, src_ix: usize) {
        use gitcomet_core::domain::DiffLineKind as DK;

        let len = self.patch_diff_row_len();
        if src_ix >= len {
            return;
        }
        if self.diff_word_highlights.len() != len {
            self.diff_word_highlights.resize(len, None);
        }
        if self
            .diff_word_highlights
            .get(src_ix)
            .and_then(Option::as_ref)
            .is_some()
        {
            return;
        }

        if self.patch_diff_row(src_ix).is_none() {
            return;
        }
        if !matches!(self.patch_visual_line_kind(src_ix), DK::Add | DK::Remove) {
            return;
        }

        let mut group_start = src_ix;
        while group_start > 0 {
            let Some(prev) = self.patch_diff_row(group_start.saturating_sub(1)) else {
                break;
            };
            if matches!(prev.kind, DK::Remove) {
                group_start = group_start.saturating_sub(1);
            } else {
                break;
            }
        }

        let mut ix = group_start;
        let mut removed: Vec<(usize, AnnotatedDiffLine)> = Vec::new();
        while ix < len {
            let Some(line) = self.patch_diff_row(ix) else {
                break;
            };
            if !matches!(line.kind, DK::Remove) {
                break;
            }
            removed.push((ix, line));
            ix += 1;
        }

        let mut added: Vec<(usize, AnnotatedDiffLine)> = Vec::new();
        while ix < len {
            let Some(line) = self.patch_diff_row(ix) else {
                break;
            };
            if !matches!(line.kind, DK::Add) {
                break;
            }
            added.push((ix, line));
            ix += 1;
        }

        let pairs = removed.len().min(added.len());
        for i in 0..pairs {
            let (old_ix, old_line) = &removed[i];
            let (new_ix, new_line) = &added[i];
            let (old_ranges, new_ranges) =
                capped_word_diff_ranges(diff_content_text(old_line), diff_content_text(new_line));
            if matches!(self.patch_visual_line_kind(*old_ix), DK::Remove) && !old_ranges.is_empty()
            {
                self.diff_word_highlights[*old_ix] = Some(old_ranges);
            }
            if matches!(self.patch_visual_line_kind(*new_ix), DK::Add) && !new_ranges.is_empty() {
                self.diff_word_highlights[*new_ix] = Some(new_ranges);
            }
        }

        for (old_ix, old_line) in removed.into_iter().skip(pairs) {
            let text = diff_content_text(&old_line);
            if matches!(self.patch_visual_line_kind(old_ix), DK::Remove) && !text.is_empty() {
                self.diff_word_highlights[old_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
        for (new_ix, new_line) in added.into_iter().skip(pairs) {
            let text = diff_content_text(&new_line);
            if matches!(self.patch_visual_line_kind(new_ix), DK::Add) && !text.is_empty() {
                self.diff_word_highlights[new_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
    }
}
