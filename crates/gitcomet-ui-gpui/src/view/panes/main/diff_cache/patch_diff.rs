use super::super::*;
use crate::view::diff_utils::diff_content_line_text;
use gitcomet_core::domain::DiffRowProvider;
use smallvec::SmallVec;

pub(in crate::view) const PATCH_DIFF_PAGE_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
struct DiffLineNumberState {
    old_line: Option<u32>,
    new_line: Option<u32>,
}

pub(crate) struct PagedPatchDiffRowsSliceIter<'a> {
    provider: &'a PagedPatchDiffRows,
    next_ix: usize,
    end_ix: usize,
    current_page_ix: Option<usize>,
    current_page: Option<Arc<[AnnotatedDiffLine]>>,
}

impl<'a> PagedPatchDiffRowsSliceIter<'a> {
    fn empty(provider: &'a PagedPatchDiffRows) -> Self {
        Self {
            provider,
            next_ix: 0,
            end_ix: 0,
            current_page_ix: None,
            current_page: None,
        }
    }

    fn new(provider: &'a PagedPatchDiffRows, start: usize, end: usize) -> Self {
        Self {
            provider,
            next_ix: start,
            end_ix: end,
            current_page_ix: None,
            current_page: None,
        }
    }
}

impl Iterator for PagedPatchDiffRowsSliceIter<'_> {
    type Item = AnnotatedDiffLine;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_ix >= self.end_ix {
            return None;
        }

        let page_ix = self.next_ix / self.provider.page_size;
        if self.current_page_ix != Some(page_ix) {
            self.current_page = self.provider.load_page(page_ix);
            self.current_page_ix = Some(page_ix);
        }

        let page_row_ix = self.next_ix % self.provider.page_size;
        let row = self.current_page.as_ref()?.get(page_row_ix)?.clone();
        self.next_ix += 1;
        Some(row)
    }
}

#[derive(Debug)]
pub(in crate::view) struct PagedPatchDiffRows {
    diff: Arc<gitcomet_core::domain::Diff>,
    page_size: usize,
    page_start_states: std::sync::Mutex<Vec<DiffLineNumberState>>,
    pages: std::sync::Mutex<HashMap<usize, Arc<[AnnotatedDiffLine]>>>,
}

impl PagedPatchDiffRows {
    pub(in crate::view) fn new(diff: Arc<gitcomet_core::domain::Diff>, page_size: usize) -> Self {
        let page_size = page_size.max(1);
        Self {
            diff,
            page_size,
            page_start_states: std::sync::Mutex::new(vec![DiffLineNumberState::default()]),
            pages: std::sync::Mutex::new(HashMap::default()),
        }
    }

    fn ensure_page_start_state(&self, page_ix: usize) -> Option<DiffLineNumberState> {
        let page_count = self.diff.lines.len().div_ceil(self.page_size);
        if page_ix >= page_count {
            return None;
        }
        {
            let states = self.page_start_states.lock().ok()?;
            if let Some(&state) = states.get(page_ix) {
                return Some(state);
            }
        }
        let mut states = self.page_start_states.lock().ok()?;
        while states.len() <= page_ix {
            let prev_ix = states.len() - 1;
            let mut state = states[prev_ix];
            let start = prev_ix * self.page_size;
            let end = (start + self.page_size).min(self.diff.lines.len());
            for line in &self.diff.lines[start..end] {
                state = Self::advance_state(state, line);
            }
            states.push(state);
        }
        Some(states[page_ix])
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
        let mut state = self.ensure_page_start_state(page_ix)?;
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
                text: line.text.clone(),
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

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn materialized_row_count(&self) -> usize {
        self.pages
            .lock()
            .map(|pages| pages.values().map(|page| page.len()).sum())
            .unwrap_or(0)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedPatchDiffRows {
    type RowRef = AnnotatedDiffLine;
    type SliceIter<'a>
        = PagedPatchDiffRowsSliceIter<'a>
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
            return PagedPatchDiffRowsSliceIter::empty(self);
        }
        let end = end.min(self.diff.lines.len());
        PagedPatchDiffRowsSliceIter::new(self, start, end)
    }
}

#[derive(Debug, Default)]
struct PatchSplitMaterializationState {
    rows: SmallVec<[PatchSplitRow; PATCH_DIFF_PAGE_SIZE]>,
    /// Logical index of `rows[0]`. Rows before this index were skipped via
    /// the fast-forward scan and are not stored.
    row_base: usize,
    next_src_ix: usize,
    pending_removes: SmallVec<[(usize, AnnotatedDiffLine); 8]>,
    pending_adds: SmallVec<[(usize, AnnotatedDiffLine); 8]>,
    done: bool,
}

#[derive(Debug)]
pub(in crate::view) struct PagedPatchSplitRows {
    source: Arc<PagedPatchDiffRows>,
    len_hint: usize,
    state: std::sync::Mutex<PatchSplitMaterializationState>,
}

impl PagedPatchSplitRows {
    #[cfg(test)]
    pub(in crate::view) fn new(source: Arc<PagedPatchDiffRows>) -> Self {
        let len_hint = Self::count_rows(source.diff.lines.as_slice());
        Self::new_with_len_hint(source, len_hint)
    }

    pub(in crate::view) fn new_with_len_hint(
        source: Arc<PagedPatchDiffRows>,
        len_hint: usize,
    ) -> Self {
        Self {
            source,
            len_hint,
            state: std::sync::Mutex::new(PatchSplitMaterializationState::default()),
        }
    }

    #[cfg(test)]
    fn count_rows(lines: &[gitcomet_core::domain::DiffLine]) -> usize {
        use gitcomet_core::domain::DiffLineKind as DK;

        let mut out = 0usize;
        let mut pending_removes = 0usize;
        let mut pending_adds = 0usize;

        for line in lines {
            match line.kind {
                DK::Remove => {
                    pending_removes += 1;
                }
                DK::Add => {
                    pending_adds += 1;
                }
                DK::Context | DK::Header | DK::Hunk => {
                    out += pending_removes.max(pending_adds);
                    pending_removes = 0;
                    pending_adds = 0;
                    out += 1;
                }
            }
        }
        out += pending_removes.max(pending_adds);
        out
    }

    /// Lightweight scan using only `DiffLineKind` to find the last block
    /// boundary at or before `target_split_row`. A block boundary is a
    /// position where both pending-removes and pending-adds are empty,
    /// meaning full materialization can start cleanly from that source index.
    ///
    /// Returns `(src_ix, split_row_count)` at the boundary. This avoids
    /// page-cache overhead and `Arc<str>` cloning — only the `kind` field
    /// of each `DiffLine` is inspected.
    fn scan_to_block_boundary(
        lines: &[gitcomet_core::domain::DiffLine],
        target_split_row: usize,
    ) -> (usize, usize) {
        use gitcomet_core::domain::DiffLineKind as DK;

        let mut row_count = 0usize;
        let mut pending_removes = 0usize;
        let mut pending_adds = 0usize;
        let mut best_src_ix = 0usize;
        let mut best_row_count = 0usize;

        for (src_ix, line) in lines.iter().enumerate() {
            match line.kind {
                DK::Remove => pending_removes += 1,
                DK::Add => pending_adds += 1,
                DK::Context | DK::Header | DK::Hunk => {
                    row_count += pending_removes.max(pending_adds);
                    pending_removes = 0;
                    pending_adds = 0;

                    if row_count > target_split_row {
                        return (best_src_ix, best_row_count);
                    }

                    best_src_ix = src_ix;
                    best_row_count = row_count;

                    row_count += 1;

                    if row_count > target_split_row {
                        return (best_src_ix, best_row_count);
                    }

                    best_src_ix = src_ix + 1;
                    best_row_count = row_count;
                }
            }
        }

        row_count += pending_removes.max(pending_adds);
        if row_count > target_split_row {
            return (best_src_ix, best_row_count);
        }

        (best_src_ix, best_row_count)
    }

    fn flush_pending(state: &mut PatchSplitMaterializationState) {
        let pairs = state.pending_removes.len().max(state.pending_adds.len());
        for i in 0..pairs {
            let left = state.pending_removes.get(i);
            let right = state.pending_adds.get(i);
            let kind = match (left.is_some(), right.is_some()) {
                (true, true) => gitcomet_core::file_diff::FileDiffRowKind::Modify,
                (true, false) => gitcomet_core::file_diff::FileDiffRowKind::Remove,
                (false, true) => gitcomet_core::file_diff::FileDiffRowKind::Add,
                (false, false) => gitcomet_core::file_diff::FileDiffRowKind::Context,
            };
            state.rows.push(PatchSplitRow::Aligned {
                row: FileDiffRow {
                    kind,
                    old_line: left.and_then(|(_, line)| line.old_line),
                    new_line: right.and_then(|(_, line)| line.new_line),
                    old: left.map(|(_, line)| diff_content_line_text(line)),
                    new: right.map(|(_, line)| diff_content_line_text(line)),
                    eof_newline: None,
                },
                old_src_ix: left.map(|(ix, _)| *ix),
                new_src_ix: right.map(|(ix, _)| *ix),
            });
        }
        state.pending_removes.clear();
        state.pending_adds.clear();
    }

    fn push_context_row(
        state: &mut PatchSplitMaterializationState,
        src_ix: usize,
        line: AnnotatedDiffLine,
    ) {
        let text = diff_content_line_text(&line);
        state.rows.push(PatchSplitRow::Aligned {
            row: FileDiffRow {
                kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                old_line: line.old_line,
                new_line: line.new_line,
                old: Some(text.clone()),
                new: Some(text),
                eof_newline: None,
            },
            old_src_ix: Some(src_ix),
            new_src_ix: Some(src_ix),
        });
    }

    fn materialize_source_line(
        state: &mut PatchSplitMaterializationState,
        src_ix: usize,
        line: AnnotatedDiffLine,
    ) {
        use gitcomet_core::domain::DiffLineKind as DK;

        match line.kind {
            DK::Header if line.text.starts_with("diff --git ") => {
                Self::flush_pending(state);
                state.rows.push(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::FileHeader,
                });
            }
            DK::Hunk => {
                Self::flush_pending(state);
                state.rows.push(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                });
            }
            DK::Context => {
                Self::flush_pending(state);
                Self::push_context_row(state, src_ix, line);
            }
            DK::Remove => state.pending_removes.push((src_ix, line)),
            DK::Add => state.pending_adds.push((src_ix, line)),
            DK::Header => {
                Self::flush_pending(state);
                state.rows.push(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::Line,
                });
            }
        }
        state.next_src_ix = src_ix.saturating_add(1);
    }

    fn materialize_until(&self, target_ix: usize) {
        if target_ix >= self.len_hint {
            return;
        }

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };

        // If the target is before the skip window, reset and rematerialize
        // from the beginning (handles backward-scroll in production).
        if target_ix < state.row_base {
            state.rows.clear();
            state.row_base = 0;
            state.next_src_ix = 0;
            state.pending_removes.clear();
            state.pending_adds.clear();
            state.done = false;
        }

        let logical_len = |s: &PatchSplitMaterializationState| s.row_base + s.rows.len();

        // Already materialized far enough.
        if logical_len(&state) > target_ix {
            return;
        }

        self.maybe_fast_forward_to(&mut state, target_ix);

        if state.rows.is_empty() {
            let reserve_rows = target_ix
                .saturating_sub(state.row_base)
                .saturating_add(1)
                .min(self.len_hint.saturating_sub(state.row_base));
            state.rows.reserve(reserve_rows);
        }

        while logical_len(&state) <= target_ix && !state.done {
            let src_start = state.next_src_ix;
            let src_len = self.source.len_hint();
            if src_start >= src_len {
                Self::flush_pending(&mut state);
                state.done = true;
                break;
            }

            // Batch-load source rows to amortize page-cache lock overhead.
            let batch_end = (src_start + self.source.page_size).min(src_len);
            let batch = self.source.slice(src_start, batch_end);

            for (offset, line) in batch.enumerate() {
                let src_ix = src_start + offset;
                Self::materialize_source_line(&mut state, src_ix, line);
                if logical_len(&state) > target_ix {
                    break;
                }
            }
        }
    }

    fn row_at(&self, ix: usize) -> Option<PatchSplitRow> {
        self.materialize_until(ix);
        self.state.lock().ok().and_then(|state| {
            if ix < state.row_base {
                return None;
            }
            state.rows.get(ix - state.row_base).cloned()
        })
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn materialized_row_count(&self) -> usize {
        self.state.lock().map(|state| state.rows.len()).unwrap_or(0)
    }

    fn maybe_fast_forward_to(&self, state: &mut PatchSplitMaterializationState, target_ix: usize) {
        // On a fresh deep access, skip directly to the last clean block
        // boundary before the requested row so we only materialize rows that
        // can actually land in the requested window.
        if !state.rows.is_empty()
            || state.next_src_ix != 0
            || state.row_base != 0
            || target_ix <= self.source.page_size
        {
            return;
        }

        let (skip_src, skip_rows) =
            Self::scan_to_block_boundary(&self.source.diff.lines, target_ix);
        if skip_rows > 0 {
            state.row_base = skip_rows;
            state.next_src_ix = skip_src;
        }
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedPatchSplitRows {
    type RowRef = PatchSplitRow;
    type SliceIter<'a>
        = smallvec::IntoIter<[PatchSplitRow; PATCH_DIFF_PAGE_SIZE]>
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
            return SmallVec::<[PatchSplitRow; PATCH_DIFF_PAGE_SIZE]>::new().into_iter();
        }
        let end = end.min(self.len_hint);

        if let Ok(mut state) = self.state.lock() {
            if start < state.row_base {
                state.rows.clear();
                state.row_base = 0;
                state.next_src_ix = 0;
                state.pending_removes.clear();
                state.pending_adds.clear();
                state.done = false;
            }
            self.maybe_fast_forward_to(&mut state, start);
        }

        self.materialize_until(end.saturating_sub(1));

        if let Ok(state) = self.state.lock() {
            let local_start = start.saturating_sub(state.row_base);
            let local_end = end.saturating_sub(state.row_base).min(state.rows.len());
            if local_start < local_end {
                let mut rows = SmallVec::<[PatchSplitRow; PATCH_DIFF_PAGE_SIZE]>::with_capacity(
                    local_end - local_start,
                );
                rows.extend(state.rows[local_start..local_end].iter().cloned());
                return rows.into_iter();
            }
        }
        SmallVec::<[PatchSplitRow; PATCH_DIFF_PAGE_SIZE]>::new().into_iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PatchInlineVisibleRun {
    start_visible_ix: usize,
    start_src_ix: usize,
    len: usize,
}

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct PatchInlineVisibleMap {
    visible_len: usize,
    visible_runs: Vec<PatchInlineVisibleRun>,
}

impl PatchInlineVisibleMap {
    pub(in crate::view) fn from_hidden_flags(hidden_flags: &[bool]) -> Self {
        let mut visible_runs = Vec::new();
        let mut visible_len = 0usize;
        let mut run_start_src_ix = None;

        for (src_ix, hide) in hidden_flags.iter().copied().enumerate() {
            if hide {
                if let Some(start_src_ix) = run_start_src_ix.take() {
                    let len = src_ix.saturating_sub(start_src_ix);
                    if len > 0 {
                        visible_runs.push(PatchInlineVisibleRun {
                            start_visible_ix: visible_len,
                            start_src_ix,
                            len,
                        });
                        visible_len += len;
                    }
                }
            } else if run_start_src_ix.is_none() {
                run_start_src_ix = Some(src_ix);
            }
        }

        if let Some(start_src_ix) = run_start_src_ix {
            let len = hidden_flags.len().saturating_sub(start_src_ix);
            if len > 0 {
                visible_runs.push(PatchInlineVisibleRun {
                    start_visible_ix: visible_len,
                    start_src_ix,
                    len,
                });
                visible_len += len;
            }
        }

        Self {
            visible_len,
            visible_runs,
        }
    }

    pub(in crate::view) fn visible_len(&self) -> usize {
        self.visible_len
    }

    pub(in crate::view) fn for_each_visible_src_ix(&self, mut visit: impl FnMut(usize, usize)) {
        for run in &self.visible_runs {
            for offset in 0..run.len {
                visit(run.start_visible_ix + offset, run.start_src_ix + offset);
            }
        }
    }

    pub(in crate::view) fn src_ix_for_visible_ix(&self, visible_ix: usize) -> Option<usize> {
        if visible_ix >= self.visible_len {
            return None;
        }

        let run_ix = self
            .visible_runs
            .partition_point(|run| run.start_visible_ix <= visible_ix)
            .checked_sub(1)?;
        let run = self.visible_runs.get(run_ix)?;
        let offset = visible_ix.saturating_sub(run.start_visible_ix);
        (offset < run.len).then_some(run.start_src_ix + offset)
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
    fn paged_patch_split_rows_first_window_stops_inside_large_hunk() {
        let line_count = 20_000usize;
        let mut text = String::new();
        text.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
        text.push_str("index 1111111..2222222 100644\n");
        text.push_str("--- a/src/lib.rs\n");
        text.push_str("+++ b/src/lib.rs\n");
        text.push_str(&format!(
            "@@ -1,{} +1,{} @@ fn synthetic() {{\n",
            line_count.saturating_mul(2),
            line_count.saturating_mul(2)
        ));
        for ix in 0..line_count {
            if ix % 7 == 0 {
                text.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
                text.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
            } else {
                text.push_str(&format!(" let shared_{ix} = keep({ix});\n"));
            }
        }

        let diff = Arc::new(Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            text.as_str(),
        ));
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));

        let first_window = split_provider.slice(0, 200).collect::<Vec<_>>();

        assert_eq!(first_window.len(), 200);
        assert_eq!(rows_provider.cached_page_count(), 1);
        assert_eq!(rows_provider.materialized_row_count(), 256);
        assert!(split_provider.materialized_row_count() < 256);
        assert!(split_provider.materialized_row_count() < split_provider.len_hint());
    }

    #[test]
    fn paged_patch_split_rows_deep_window_returns_full_requested_slice() {
        let line_count = 20_000usize;
        let mut text = String::new();
        text.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
        text.push_str("index 1111111..2222222 100644\n");
        text.push_str("--- a/src/lib.rs\n");
        text.push_str("+++ b/src/lib.rs\n");
        text.push_str(&format!(
            "@@ -1,{} +1,{} @@ fn synthetic() {{\n",
            line_count.saturating_mul(2),
            line_count.saturating_mul(2)
        ));
        for ix in 0..line_count {
            if ix % 7 == 0 {
                text.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
                text.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
            } else {
                text.push_str(&format!(" let shared_{ix} = keep({ix});\n"));
            }
        }

        let diff = Arc::new(Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            text.as_str(),
        ));
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));
        let window = 200usize;
        let start = split_provider
            .len_hint()
            .saturating_mul(9)
            .checked_div(10)
            .unwrap_or(0)
            .min(split_provider.len_hint().saturating_sub(window));

        let deep_window = split_provider
            .slice(start, start + window)
            .collect::<Vec<_>>();

        assert_eq!(deep_window.len(), window);
        assert!(split_provider.materialized_row_count() >= window);
        assert!(split_provider.materialized_row_count() < split_provider.len_hint());
        assert!(rows_provider.cached_page_count() > 0);
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
