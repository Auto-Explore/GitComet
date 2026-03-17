use super::super::*;
use gitcomet_core::domain::DiffRowProvider;

pub(in crate::view) const PATCH_DIFF_PAGE_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
struct DiffLineNumberState {
    old_line: Option<u32>,
    new_line: Option<u32>,
}

#[derive(Debug)]
pub(in crate::view) struct PagedPatchDiffRows {
    diff: Arc<gitcomet_core::domain::Diff>,
    page_size: usize,
    page_start_states: Vec<DiffLineNumberState>,
    pages: std::sync::Mutex<HashMap<usize, Arc<[AnnotatedDiffLine]>>>,
}

impl PagedPatchDiffRows {
    pub(in crate::view) fn new(diff: Arc<gitcomet_core::domain::Diff>, page_size: usize) -> Self {
        let page_size = page_size.max(1);
        let line_count = diff.lines.len();
        let page_count = line_count.div_ceil(page_size);
        let mut page_start_states = Vec::with_capacity(page_count);
        let mut state = DiffLineNumberState::default();

        for page_ix in 0..page_count {
            page_start_states.push(state);
            let start = page_ix * page_size;
            let end = (start + page_size).min(line_count);
            for line in &diff.lines[start..end] {
                state = Self::advance_state(state, line);
            }
        }

        Self {
            diff,
            page_size,
            page_start_states,
            pages: std::sync::Mutex::new(HashMap::default()),
        }
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.diff.lines.len()).then(|| {
            let end = start
                .saturating_add(self.page_size)
                .min(self.diff.lines.len());
            (start, end)
        })
    }

    fn parse_hunk_start(text: &str) -> Option<(u32, u32)> {
        let text = text.strip_prefix("@@")?.trim_start();
        let text = text.split("@@").next()?.trim();
        let mut it = text.split_whitespace();
        let old = it.next()?.strip_prefix('-')?;
        let new = it.next()?.strip_prefix('+')?;
        let old_start = old.split(',').next()?.parse::<u32>().ok()?;
        let new_start = new.split(',').next()?.parse::<u32>().ok()?;
        Some((old_start, new_start))
    }

    fn advance_state(
        mut state: DiffLineNumberState,
        line: &gitcomet_core::domain::DiffLine,
    ) -> DiffLineNumberState {
        match line.kind {
            gitcomet_core::domain::DiffLineKind::Hunk => {
                if let Some((old_start, new_start)) = Self::parse_hunk_start(line.text.as_ref()) {
                    state.old_line = Some(old_start);
                    state.new_line = Some(new_start);
                } else {
                    state.old_line = None;
                    state.new_line = None;
                }
            }
            gitcomet_core::domain::DiffLineKind::Context => {
                if let Some(v) = state.old_line.as_mut() {
                    *v += 1;
                }
                if let Some(v) = state.new_line.as_mut() {
                    *v += 1;
                }
            }
            gitcomet_core::domain::DiffLineKind::Remove => {
                if let Some(v) = state.old_line.as_mut() {
                    *v += 1;
                }
            }
            gitcomet_core::domain::DiffLineKind::Add => {
                if let Some(v) = state.new_line.as_mut() {
                    *v += 1;
                }
            }
            gitcomet_core::domain::DiffLineKind::Header => {}
        }
        state
    }

    fn build_page(&self, page_ix: usize) -> Option<Arc<[AnnotatedDiffLine]>> {
        let (start, end) = self.page_bounds(page_ix)?;
        let mut state = self
            .page_start_states
            .get(page_ix)
            .copied()
            .unwrap_or_default();
        let mut rows = Vec::with_capacity(end - start);

        for line in &self.diff.lines[start..end] {
            let (old_line, new_line) = match line.kind {
                gitcomet_core::domain::DiffLineKind::Context => (state.old_line, state.new_line),
                gitcomet_core::domain::DiffLineKind::Remove => (state.old_line, None),
                gitcomet_core::domain::DiffLineKind::Add => (None, state.new_line),
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => (None, None),
            };
            rows.push(AnnotatedDiffLine {
                kind: line.kind,
                text: Arc::clone(&line.text),
                old_line,
                new_line,
            });
            state = Self::advance_state(state, line);
        }

        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[AnnotatedDiffLine]>> {
        if let Ok(pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
        if let Ok(mut pages) = self.pages.lock() {
            return Some(Arc::clone(
                pages.entry(page_ix).or_insert_with(|| Arc::clone(&page)),
            ));
        }
        Some(page)
    }

    fn row_at(&self, ix: usize) -> Option<AnnotatedDiffLine> {
        if ix >= self.diff.lines.len() {
            return None;
        }
        let page_ix = ix / self.page_size;
        let row_ix = ix % self.page_size;
        let page = self.load_page(page_ix)?;
        page.get(row_ix).cloned()
    }

    #[cfg(test)]
    fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedPatchDiffRows {
    type RowRef = AnnotatedDiffLine;
    type SliceIter<'a>
        = std::vec::IntoIter<AnnotatedDiffLine>
    where
        Self: 'a;

    fn len_hint(&self) -> usize {
        self.diff.lines.len()
    }

    fn row(&self, ix: usize) -> Option<Self::RowRef> {
        self.row_at(ix)
    }

    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_> {
        if start >= end || start >= self.diff.lines.len() {
            return Vec::new().into_iter();
        }
        let end = end.min(self.diff.lines.len());
        let mut rows = Vec::with_capacity(end - start);
        let mut ix = start;
        while ix < end {
            if let Some(line) = self.row_at(ix) {
                rows.push(line);
                ix += 1;
            } else {
                break;
            }
        }
        rows.into_iter()
    }
}

#[derive(Debug, Default)]
struct PatchSplitMaterializationState {
    rows: Vec<PatchSplitRow>,
    next_src_ix: usize,
    pending_removes: Vec<usize>,
    pending_adds: Vec<usize>,
    done: bool,
}

#[derive(Debug)]
pub(in crate::view) struct PagedPatchSplitRows {
    source: Arc<PagedPatchDiffRows>,
    len_hint: usize,
    state: std::sync::Mutex<PatchSplitMaterializationState>,
}

impl PagedPatchSplitRows {
    pub(in crate::view) fn new(source: Arc<PagedPatchDiffRows>) -> Self {
        let len_hint = Self::count_rows(source.diff.lines.as_slice());
        Self {
            source,
            len_hint,
            state: std::sync::Mutex::new(PatchSplitMaterializationState::default()),
        }
    }

    fn count_rows(lines: &[gitcomet_core::domain::DiffLine]) -> usize {
        use gitcomet_core::domain::DiffLineKind as DK;

        let mut out = 0usize;
        let mut ix = 0usize;
        let mut pending_removes = 0usize;
        let mut pending_adds = 0usize;
        let flush_pending =
            |out: &mut usize, pending_removes: &mut usize, pending_adds: &mut usize| {
                *out = out.saturating_add((*pending_removes).max(*pending_adds));
                *pending_removes = 0;
                *pending_adds = 0;
            };

        while ix < lines.len() {
            let line = &lines[ix];
            let is_file_header =
                matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");

            if is_file_header {
                flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
                out = out.saturating_add(1);
                ix += 1;
                continue;
            }

            if matches!(line.kind, DK::Hunk) {
                flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
                out = out.saturating_add(1);
                ix += 1;

                while ix < lines.len() {
                    let line = &lines[ix];
                    let is_next_file_header =
                        matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");
                    if is_next_file_header || matches!(line.kind, DK::Hunk) {
                        break;
                    }
                    match line.kind {
                        DK::Context => {
                            flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
                            out = out.saturating_add(1);
                        }
                        DK::Remove => pending_removes = pending_removes.saturating_add(1),
                        DK::Add => pending_adds = pending_adds.saturating_add(1),
                        DK::Header | DK::Hunk => {
                            flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
                            out = out.saturating_add(1);
                        }
                    }
                    ix += 1;
                }

                flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
                continue;
            }

            out = out.saturating_add(1);
            ix += 1;
        }

        flush_pending(&mut out, &mut pending_removes, &mut pending_adds);
        out
    }

    fn flush_pending(&self, state: &mut PatchSplitMaterializationState) {
        let pairs = state.pending_removes.len().max(state.pending_adds.len());
        for i in 0..pairs {
            let left_ix = state.pending_removes.get(i).copied();
            let right_ix = state.pending_adds.get(i).copied();
            let left = left_ix.and_then(|ix| self.source.row_at(ix));
            let right = right_ix.and_then(|ix| self.source.row_at(ix));
            let kind = match (left_ix.is_some(), right_ix.is_some()) {
                (true, true) => gitcomet_core::file_diff::FileDiffRowKind::Modify,
                (true, false) => gitcomet_core::file_diff::FileDiffRowKind::Remove,
                (false, true) => gitcomet_core::file_diff::FileDiffRowKind::Add,
                (false, false) => gitcomet_core::file_diff::FileDiffRowKind::Context,
            };
            state.rows.push(PatchSplitRow::Aligned {
                row: FileDiffRow {
                    kind,
                    old_line: left.as_ref().and_then(|line| line.old_line),
                    new_line: right.as_ref().and_then(|line| line.new_line),
                    old: left.as_ref().map(|line| diff_content_text(line).into()),
                    new: right.as_ref().map(|line| diff_content_text(line).into()),
                    eof_newline: None,
                },
                old_src_ix: left_ix,
                new_src_ix: right_ix,
            });
        }
        state.pending_removes.clear();
        state.pending_adds.clear();
    }

    fn materialize_until(&self, target_ix: usize) {
        use gitcomet_core::domain::DiffLineKind as DK;
        if target_ix >= self.len_hint {
            return;
        }

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        while state.rows.len() <= target_ix && !state.done {
            if state.next_src_ix >= self.source.len_hint() {
                self.flush_pending(&mut state);
                state.done = true;
                break;
            }

            let src_ix = state.next_src_ix;
            let Some(line) = self.source.row_at(src_ix) else {
                state.done = true;
                break;
            };
            let is_file_header =
                matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");
            if is_file_header {
                self.flush_pending(&mut state);
                state.rows.push(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::FileHeader,
                });
                state.next_src_ix += 1;
                continue;
            }

            if matches!(line.kind, DK::Hunk) {
                self.flush_pending(&mut state);
                state.rows.push(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                });
                state.next_src_ix += 1;

                while state.next_src_ix < self.source.len_hint() {
                    let src_ix = state.next_src_ix;
                    let Some(line) = self.source.row_at(src_ix) else {
                        break;
                    };
                    let is_next_file_header =
                        matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");
                    if is_next_file_header || matches!(line.kind, DK::Hunk) {
                        break;
                    }

                    match line.kind {
                        DK::Context => {
                            self.flush_pending(&mut state);
                            let text: Arc<str> = diff_content_text(&line).into();
                            state.rows.push(PatchSplitRow::Aligned {
                                row: FileDiffRow {
                                    kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                                    old_line: line.old_line,
                                    new_line: line.new_line,
                                    old: Some(Arc::clone(&text)),
                                    new: Some(text),
                                    eof_newline: None,
                                },
                                old_src_ix: Some(src_ix),
                                new_src_ix: Some(src_ix),
                            });
                        }
                        DK::Remove => state.pending_removes.push(src_ix),
                        DK::Add => state.pending_adds.push(src_ix),
                        DK::Header | DK::Hunk => {
                            self.flush_pending(&mut state);
                            state.rows.push(PatchSplitRow::Raw {
                                src_ix,
                                click_kind: DiffClickKind::Line,
                            });
                        }
                    }
                    state.next_src_ix += 1;
                }

                self.flush_pending(&mut state);
                continue;
            }

            state.rows.push(PatchSplitRow::Raw {
                src_ix,
                click_kind: DiffClickKind::Line,
            });
            state.next_src_ix += 1;
        }
    }

    fn row_at(&self, ix: usize) -> Option<PatchSplitRow> {
        self.materialize_until(ix);
        self.state
            .lock()
            .ok()
            .and_then(|state| state.rows.get(ix).cloned())
    }

    #[cfg(test)]
    fn materialized_row_count(&self) -> usize {
        self.state.lock().map(|state| state.rows.len()).unwrap_or(0)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedPatchSplitRows {
    type RowRef = PatchSplitRow;
    type SliceIter<'a>
        = std::vec::IntoIter<PatchSplitRow>
    where
        Self: 'a;

    fn len_hint(&self) -> usize {
        self.len_hint
    }

    fn row(&self, ix: usize) -> Option<Self::RowRef> {
        self.row_at(ix)
    }

    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_> {
        if start >= end || start >= self.len_hint {
            return Vec::new().into_iter();
        }
        let end = end.min(self.len_hint);
        self.materialize_until(end.saturating_sub(1));
        if let Ok(state) = self.state.lock() {
            let mut rows = Vec::with_capacity(end.saturating_sub(start));
            rows.extend(state.rows[start..end].iter().cloned());
            return rows.into_iter();
        }
        Vec::new().into_iter()
    }
}

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct PatchInlineVisibleMap {
    src_len: usize,
    hidden_src_ixs: Vec<usize>,
}

impl PatchInlineVisibleMap {
    pub(in crate::view) fn from_hidden_flags(hidden_flags: &[bool]) -> Self {
        let mut hidden_src_ixs = Vec::new();
        for (src_ix, hide) in hidden_flags.iter().copied().enumerate() {
            if hide {
                hidden_src_ixs.push(src_ix);
            }
        }
        Self {
            src_len: hidden_flags.len(),
            hidden_src_ixs,
        }
    }

    pub(in crate::view) fn visible_len(&self) -> usize {
        self.src_len.saturating_sub(self.hidden_src_ixs.len())
    }

    pub(in crate::view) fn src_ix_for_visible_ix(&self, visible_ix: usize) -> Option<usize> {
        if visible_ix >= self.visible_len() {
            return None;
        }

        let mut lo = 0usize;
        let mut hi = self.src_len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let hidden_through_mid = self.hidden_src_ixs.partition_point(|&ix| ix <= mid);
            let visible_through_mid = mid + 1 - hidden_through_mid;
            if visible_through_mid <= visible_ix {
                lo = mid.saturating_add(1);
            } else {
                hi = mid;
            }
        }
        (lo < self.src_len).then_some(lo)
    }
}

#[derive(Debug, Default)]
pub(super) struct PatchSplitVisibleMeta {
    pub(super) visible_indices: Vec<usize>,
    pub(super) visible_flags: Vec<u8>,
    pub(super) total_rows: usize,
}

pub(super) fn should_hide_unified_diff_header_raw(
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> bool {
    matches!(kind, gitcomet_core::domain::DiffLineKind::Header)
        && (text.starts_with("index ") || text.starts_with("--- ") || text.starts_with("+++ "))
}

pub(super) fn build_patch_split_visible_meta_from_src(
    line_kinds: &[gitcomet_core::domain::DiffLineKind],
    click_kinds: &[DiffClickKind],
    hide_unified_header_for_src_ix: &[bool],
) -> PatchSplitVisibleMeta {
    use gitcomet_core::domain::DiffLineKind as DK;

    let src_len = line_kinds
        .len()
        .min(click_kinds.len())
        .min(hide_unified_header_for_src_ix.len());

    let mut visible_indices = Vec::with_capacity(src_len);
    let mut visible_flags = Vec::with_capacity(src_len);
    let mut row_ix = 0usize;
    let mut src_ix = 0usize;
    let mut pending_removes = 0usize;
    let mut pending_adds = 0usize;

    let flush_pending = |visible_indices: &mut Vec<usize>,
                         visible_flags: &mut Vec<u8>,
                         row_ix: &mut usize,
                         pending_removes: &mut usize,
                         pending_adds: &mut usize| {
        let pairs = (*pending_removes).max(*pending_adds);
        for pair_ix in 0..pairs {
            let has_remove = pair_ix < *pending_removes;
            let has_add = pair_ix < *pending_adds;
            let flag = match (has_remove, has_add) {
                (true, true) => 3,
                (true, false) => 2,
                (false, true) => 1,
                (false, false) => 0,
            };
            visible_indices.push(*row_ix);
            visible_flags.push(flag);
            *row_ix = row_ix.saturating_add(1);
        }
        *pending_removes = 0;
        *pending_adds = 0;
    };

    let push_raw = |visible_indices: &mut Vec<usize>,
                    visible_flags: &mut Vec<u8>,
                    row_ix: &mut usize,
                    hide: bool| {
        if !hide {
            visible_indices.push(*row_ix);
            visible_flags.push(0);
        }
        *row_ix = row_ix.saturating_add(1);
    };

    while src_ix < src_len {
        let kind = line_kinds[src_ix];
        let is_file_header = matches!(click_kinds[src_ix], DiffClickKind::FileHeader);
        let hide = hide_unified_header_for_src_ix[src_ix];

        if is_file_header {
            flush_pending(
                &mut visible_indices,
                &mut visible_flags,
                &mut row_ix,
                &mut pending_removes,
                &mut pending_adds,
            );
            push_raw(&mut visible_indices, &mut visible_flags, &mut row_ix, hide);
            src_ix += 1;
            continue;
        }

        if matches!(kind, DK::Hunk) {
            flush_pending(
                &mut visible_indices,
                &mut visible_flags,
                &mut row_ix,
                &mut pending_removes,
                &mut pending_adds,
            );
            push_raw(&mut visible_indices, &mut visible_flags, &mut row_ix, hide);
            src_ix += 1;

            while src_ix < src_len {
                let kind = line_kinds[src_ix];
                let hide = hide_unified_header_for_src_ix[src_ix];
                let is_next_file_header = matches!(click_kinds[src_ix], DiffClickKind::FileHeader);
                if is_next_file_header || matches!(kind, DK::Hunk) {
                    break;
                }

                match kind {
                    DK::Context => {
                        flush_pending(
                            &mut visible_indices,
                            &mut visible_flags,
                            &mut row_ix,
                            &mut pending_removes,
                            &mut pending_adds,
                        );
                        push_raw(&mut visible_indices, &mut visible_flags, &mut row_ix, hide);
                    }
                    DK::Remove => pending_removes = pending_removes.saturating_add(1),
                    DK::Add => pending_adds = pending_adds.saturating_add(1),
                    DK::Header | DK::Hunk => {
                        flush_pending(
                            &mut visible_indices,
                            &mut visible_flags,
                            &mut row_ix,
                            &mut pending_removes,
                            &mut pending_adds,
                        );
                        push_raw(&mut visible_indices, &mut visible_flags, &mut row_ix, hide);
                    }
                }

                src_ix += 1;
            }

            flush_pending(
                &mut visible_indices,
                &mut visible_flags,
                &mut row_ix,
                &mut pending_removes,
                &mut pending_adds,
            );
            continue;
        }

        push_raw(&mut visible_indices, &mut visible_flags, &mut row_ix, hide);
        src_ix += 1;
    }

    flush_pending(
        &mut visible_indices,
        &mut visible_flags,
        &mut row_ix,
        &mut pending_removes,
        &mut pending_adds,
    );

    PatchSplitVisibleMeta {
        visible_indices,
        visible_flags,
        total_rows: row_ix,
    }
}

pub(super) fn scrollbar_markers_from_visible_flags(
    visible_flags: &[u8],
) -> Vec<components::ScrollbarMarker> {
    scrollbar_markers_from_flags(visible_flags.len(), |visible_ix| {
        visible_flags.get(visible_ix).copied().unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Diff, DiffArea, DiffTarget};
    use std::path::PathBuf;

    fn split_visible_meta_for_diff(diff: &Diff) -> PatchSplitVisibleMeta {
        let line_kinds = diff.lines.iter().map(|line| line.kind).collect::<Vec<_>>();
        let click_kinds = diff
            .lines
            .iter()
            .map(|line| {
                if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
                    DiffClickKind::HunkHeader
                } else if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                    && line.text.starts_with("diff --git ")
                {
                    DiffClickKind::FileHeader
                } else {
                    DiffClickKind::Line
                }
            })
            .collect::<Vec<_>>();
        let hidden = diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_raw(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        build_patch_split_visible_meta_from_src(
            line_kinds.as_slice(),
            click_kinds.as_slice(),
            hidden.as_slice(),
        )
    }

    #[test]
    fn paged_patch_rows_load_pages_on_demand() {
        let diff = Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
@@ -1,4 +1,4 @@\n\
 old1\n\
-old2\n\
+new2\n\
 old3\n",
        );
        let provider = PagedPatchDiffRows::new(Arc::new(diff), 2);

        assert_eq!(provider.cached_page_count(), 0);
        assert!(provider.row_at(3).is_some());
        assert_eq!(provider.cached_page_count(), 1);
        assert!(provider.row_at(0).is_some());
        assert_eq!(provider.cached_page_count(), 2);

        let slice = provider
            .slice(2, 5)
            .map(|line| line.text.to_string())
            .collect::<Vec<_>>();
        assert_eq!(slice, vec!["@@ -1,4 +1,4 @@", "old1", "-old2"]);
        assert_eq!(provider.cached_page_count(), 3);
    }

    #[test]
    fn paged_patch_split_rows_materialize_prefix_before_full_scan() {
        let diff = Arc::new(Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
@@ -1,5 +1,6 @@\n\
 old1\n\
-old2\n\
-old3\n\
+new2\n\
+new3\n\
 old4\n",
        ));
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&diff), 2));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));

        let eager = build_patch_split_rows(&annotate_unified(&diff));
        assert_eq!(split_provider.len_hint(), eager.len());
        assert_eq!(split_provider.materialized_row_count(), 0);

        let first = split_provider.row_at(0).expect("first split row");
        assert!(matches!(
            first,
            PatchSplitRow::Raw {
                click_kind: DiffClickKind::FileHeader,
                ..
            }
        ));
        assert!(split_provider.materialized_row_count() < split_provider.len_hint());

        let _ = split_provider
            .row_at(split_provider.len_hint().saturating_sub(1))
            .expect("last split row");
        assert_eq!(
            split_provider.materialized_row_count(),
            split_provider.len_hint()
        );
    }

    #[test]
    fn patch_inline_visible_map_matches_eager_visible_indices() {
        let diff = Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,3 +1,3 @@\n\
 old1\n\
-old2\n\
+new2\n",
        );
        let hidden = diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_raw(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let map = PatchInlineVisibleMap::from_hidden_flags(hidden.as_slice());

        let eager_visible = hidden
            .iter()
            .enumerate()
            .filter_map(|(src_ix, hide)| (!hide).then_some(src_ix))
            .collect::<Vec<_>>();
        let mapped_visible = (0..map.visible_len())
            .filter_map(|visible_ix| map.src_ix_for_visible_ix(visible_ix))
            .collect::<Vec<_>>();

        assert_eq!(mapped_visible, eager_visible);
        assert!(map.visible_len() < diff.lines.len());
    }

    #[test]
    fn patch_inline_visible_map_build_does_not_load_paged_rows() {
        let diff = Arc::new(Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,4 +1,4 @@\n\
 old1\n\
-old2\n\
+new2\n\
 old3\n",
        ));
        let provider = PagedPatchDiffRows::new(Arc::clone(&diff), 2);
        assert_eq!(provider.cached_page_count(), 0);

        let hidden = diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_raw(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let map = PatchInlineVisibleMap::from_hidden_flags(hidden.as_slice());

        assert_eq!(provider.cached_page_count(), 0);
        assert_eq!(map.visible_len(), diff.lines.len().saturating_sub(3));
        assert_eq!(map.src_ix_for_visible_ix(0), Some(0));
    }

    #[test]
    fn split_visible_meta_filters_hidden_unified_headers() {
        let diff = Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,3 +1,3 @@\n\
 old1\n\
-old2\n\
+new2\n",
        );
        let annotated = annotate_unified(&diff);
        let eager_split = build_patch_split_rows(&annotated);
        let expected_visible = eager_split
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| match row {
                PatchSplitRow::Raw { src_ix, .. } => {
                    (!should_hide_unified_diff_header_line(&annotated[*src_ix])).then_some(ix)
                }
                PatchSplitRow::Aligned { .. } => Some(ix),
            })
            .collect::<Vec<_>>();

        let meta = split_visible_meta_for_diff(&diff);
        assert_eq!(meta.total_rows, eager_split.len());
        assert_eq!(meta.visible_indices, expected_visible);
        assert!(meta.visible_indices.len() < meta.total_rows);
    }

    #[test]
    fn split_visible_meta_builds_non_empty_scrollbar_markers() {
        let diff = Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,6 +1,7 @@\n\
 old0\n\
-old1\n\
+new1\n\
-old2\n\
+new2\n\
+new3\n\
 old4\n",
        );
        let annotated = annotate_unified(&diff);
        let eager_split = build_patch_split_rows(&annotated);
        let expected_visible_flags = eager_split
            .iter()
            .filter_map(|row| match row {
                PatchSplitRow::Raw { src_ix, .. } => {
                    (!should_hide_unified_diff_header_line(&annotated[*src_ix])).then_some(0)
                }
                PatchSplitRow::Aligned { row, .. } => Some(match row.kind {
                    gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                    gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                    gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                }),
            })
            .collect::<Vec<_>>();

        let meta = split_visible_meta_for_diff(&diff);
        assert_eq!(meta.visible_flags, expected_visible_flags);

        let markers = scrollbar_markers_from_visible_flags(meta.visible_flags.as_slice());
        assert!(!markers.is_empty());
        assert_eq!(
            markers,
            scrollbar_markers_from_visible_flags(expected_visible_flags.as_slice())
        );
    }
}
