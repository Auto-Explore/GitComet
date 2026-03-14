use super::*;
use gitcomet_core::domain::DiffRowProvider;

const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
const PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES: usize = 256;
const PATCH_DIFF_PAGE_SIZE: usize = 256;

static IMAGE_DIFF_CACHE_STARTUP_CLEANUP: std::sync::Once = std::sync::Once::new();
static IMAGE_DIFF_CACHE_WRITE_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Full-document views (file diff, worktree preview) always attempt prepared
// syntax and fall back to plain/heuristic rendering until it is ready.
const FULL_DOCUMENT_SYNTAX_MODE: rows::DiffSyntaxMode = rows::DiffSyntaxMode::Auto;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileDiffPreparedSyntaxApplyResult {
    split_left: bool,
    split_right: bool,
}

impl FileDiffPreparedSyntaxApplyResult {
    fn any(self) -> bool {
        self.split_left || self.split_right
    }
}

fn build_inline_text(lines: &[AnnotatedDiffLine]) -> SharedString {
    let total_len = lines
        .iter()
        .map(|line| line.text.len().saturating_add(1))
        .sum::<usize>();
    let mut text = String::with_capacity(total_len);
    for line in lines {
        text.push_str(line.text.as_ref());
        text.push('\n');
    }
    SharedString::from(text)
}

fn build_line_to_split_ix_maps(rows: &[FileDiffRow]) -> (HashMap<u32, usize>, HashMap<u32, usize>) {
    let mut old_map: HashMap<u32, usize> = HashMap::default();
    let mut new_map: HashMap<u32, usize> = HashMap::default();
    old_map.reserve(rows.len());
    new_map.reserve(rows.len());
    for (ix, row) in rows.iter().enumerate() {
        if let Some(old_line) = row.old_line {
            old_map.insert(old_line, ix);
        }
        if let Some(new_line) = row.new_line {
            new_map.insert(new_line, ix);
        }
    }
    (old_map, new_map)
}

fn file_diff_text_signature(file: &gitcomet_core::domain::FileDiffText) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    file.path.hash(&mut hasher);
    file.old.hash(&mut hasher);
    file.new.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug)]
struct ImageDiffCacheEntry {
    path: std::path::PathBuf,
    modified: std::time::SystemTime,
    size: u64,
}

#[derive(Clone, Debug, Default)]
struct FileDiffBackgroundPreparedSyntaxDocuments {
    split_left: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
    split_right: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
}

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
                    old: left
                        .as_ref()
                        .map(|line| diff_content_text(line).to_string()),
                    new: right
                        .as_ref()
                        .map(|line| diff_content_text(line).to_string()),
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
                            let text = diff_content_text(&line).to_string();
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
struct PatchSplitVisibleMeta {
    visible_indices: Vec<usize>,
    visible_flags: Vec<u8>,
    total_rows: usize,
}

fn should_hide_unified_diff_header_raw(
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> bool {
    matches!(kind, gitcomet_core::domain::DiffLineKind::Header)
        && (text.starts_with("index ") || text.starts_with("--- ") || text.starts_with("+++ "))
}

fn build_patch_split_visible_meta_from_src(
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

fn scrollbar_markers_from_visible_flags(visible_flags: &[u8]) -> Vec<components::ScrollbarMarker> {
    scrollbar_markers_from_flags(visible_flags.len(), |visible_ix| {
        visible_flags.get(visible_ix).copied().unwrap_or(0)
    })
}

fn cleanup_image_diff_cache_startup_once() {
    IMAGE_DIFF_CACHE_STARTUP_CLEANUP.call_once(cleanup_image_diff_cache_now);
}

fn maybe_cleanup_image_diff_cache_on_write() {
    let write_count =
        IMAGE_DIFF_CACHE_WRITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if write_count.is_multiple_of(IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL) {
        cleanup_image_diff_cache_now();
    }
}

fn cleanup_image_diff_cache_now() {
    let _ = cleanup_image_diff_cache_dir(
        &std::env::temp_dir(),
        IMAGE_DIFF_CACHE_MAX_AGE,
        IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES,
        std::time::SystemTime::now(),
    );
}

fn cleanup_image_diff_cache_dir(
    cache_dir: &std::path::Path,
    max_age: std::time::Duration,
    max_total_bytes: u64,
    now: std::time::SystemTime,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    let mut cache_entries = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };

        let file_name = entry.file_name();
        let Some(file_name_text) = file_name.to_str() else {
            continue;
        };
        if !file_name_text.starts_with(IMAGE_DIFF_CACHE_FILE_PREFIX) {
            continue;
        }

        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if !metadata.is_file() {
            continue;
        }

        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        let age = now.duration_since(modified).unwrap_or_default();
        if age > max_age {
            let _ = std::fs::remove_file(path);
            continue;
        }

        cache_entries.push(ImageDiffCacheEntry {
            path,
            modified,
            size: metadata.len(),
        });
    }

    let mut total_size = cache_entries
        .iter()
        .fold(0_u64, |acc, entry| acc.saturating_add(entry.size));
    if total_size <= max_total_bytes {
        return Ok(());
    }

    cache_entries.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });

    for entry in cache_entries {
        if total_size <= max_total_bytes {
            break;
        }
        if std::fs::remove_file(&entry.path).is_ok() {
            total_size = total_size.saturating_sub(entry.size);
        }
    }

    Ok(())
}

fn decode_file_image_diff_bytes(
    format: gpui::ImageFormat,
    bytes: &[u8],
    cached_path: Option<&mut Option<std::path::PathBuf>>,
) -> Option<Arc<gpui::Image>> {
    match format {
        gpui::ImageFormat::Svg => {
            if let Some(image) = rasterize_svg_preview_image(bytes) {
                return Some(image);
            }
            if let Some(path) = cached_path {
                *path = Some(cached_image_diff_path(bytes, "svg")?);
            }
            None
        }
        _ => Some(Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))),
    }
}

fn rasterize_svg_preview_png_or_cached_path(
    svg_bytes: &[u8],
) -> (Option<Vec<u8>>, Option<std::path::PathBuf>) {
    if let Some(png) = rasterize_svg_preview_png(svg_bytes) {
        return (Some(png), None);
    }
    (None, cached_image_diff_path(svg_bytes, "svg"))
}

fn cached_image_diff_path(bytes: &[u8], extension: &str) -> Option<std::path::PathBuf> {
    use std::io::Write;

    cleanup_image_diff_cache_startup_once();
    maybe_cleanup_image_diff_cache_on_write();

    let suffix = format!(".{extension}");
    let mut file = tempfile::Builder::new()
        .prefix(IMAGE_DIFF_CACHE_FILE_PREFIX)
        .suffix(&suffix)
        .tempfile()
        .ok()?;
    file.as_file_mut().write_all(bytes).ok()?;
    let (_, path) = file.keep().ok()?;
    Some(path)
}

fn prepared_syntax_document_key(
    repo_id: RepoId,
    target_rev: u64,
    file_path: &std::path::Path,
    view_mode: PreparedSyntaxViewMode,
) -> PreparedSyntaxDocumentKey {
    PreparedSyntaxDocumentKey {
        repo_id,
        target_rev,
        file_path: file_path.to_path_buf(),
        view_mode,
    }
}

impl MainPaneView {
    pub(in crate::view) fn patch_diff_row_len(&self) -> usize {
        self.diff_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.diff_cache.len())
    }

    pub(in crate::view) fn patch_diff_row(&self, src_ix: usize) -> Option<AnnotatedDiffLine> {
        if let Some(provider) = self.diff_row_provider.as_ref() {
            provider.row(src_ix)
        } else {
            self.diff_cache.get(src_ix).cloned()
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

    fn patch_split_visible_meta_from_source(&self) -> PatchSplitVisibleMeta {
        build_patch_split_visible_meta_from_src(
            self.diff_line_kind_for_src_ix.as_slice(),
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

        let Some(line) = self.patch_diff_row(src_ix) else {
            return;
        };
        if !matches!(line.kind, DK::Add | DK::Remove) {
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
            if !old_ranges.is_empty() {
                self.diff_word_highlights[*old_ix] = Some(old_ranges);
            }
            if !new_ranges.is_empty() {
                self.diff_word_highlights[*new_ix] = Some(new_ranges);
            }
        }

        for (old_ix, old_line) in removed.into_iter().skip(pairs) {
            let text = diff_content_text(&old_line);
            if !text.is_empty() {
                self.diff_word_highlights[old_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
        for (new_ix, new_line) in added.into_iter().skip(pairs) {
            let text = diff_content_text(&new_line);
            if !text.is_empty() {
                self.diff_word_highlights[new_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
    }

    fn prepared_syntax_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        self.prepared_syntax_documents.get(key).copied()
    }

    fn prepared_syntax_reparse_seed_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        self.prepared_syntax_documents
            .iter()
            .filter(|(candidate_key, _)| {
                candidate_key.repo_id == key.repo_id
                    && candidate_key.file_path == key.file_path
                    && candidate_key.view_mode == key.view_mode
                    && candidate_key.target_rev != key.target_rev
            })
            .max_by_key(|(candidate_key, _)| candidate_key.target_rev)
            .map(|(_, document)| *document)
    }

    fn insert_prepared_syntax_document(
        &mut self,
        key: PreparedSyntaxDocumentKey,
        document: rows::PreparedDiffSyntaxDocument,
    ) -> bool {
        if self.prepared_syntax_documents.contains_key(&key) {
            return false;
        }
        if self.prepared_syntax_documents.len() >= PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES
            && let Some(evict_key) = self.prepared_syntax_documents.keys().next().cloned()
        {
            self.prepared_syntax_documents.remove(&evict_key);
        }
        self.prepared_syntax_documents.insert(key, document);
        true
    }

    pub(in crate::view) fn file_diff_prepared_syntax_key(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.file_diff_cache_repo_id?;
        let path = self.file_diff_cache_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.file_diff_cache_rev,
            path,
            view_mode,
        ))
    }

    fn file_diff_prepared_syntax_document(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.file_diff_prepared_syntax_key(view_mode)?;
        self.prepared_syntax_document(&key)
    }

    pub(in crate::view) fn file_diff_split_style_cache_epoch(&self, region: DiffTextRegion) -> u64 {
        self.file_diff_style_cache_epochs.split_epoch(region)
    }

    pub(in crate::view) fn file_diff_inline_style_cache_epoch(
        &self,
        line: &AnnotatedDiffLine,
    ) -> u64 {
        self.file_diff_style_cache_epochs.inline_epoch(line.kind)
    }

    /// Project inline-diff syntax from the real old/new (split) documents.
    ///
    /// Instead of parsing the synthetic mixed inline stream, look up the correct
    /// side document and line index based on the diff line kind and old_line/new_line.
    pub(in crate::view) fn file_diff_inline_projected_syntax(
        &self,
        line: &AnnotatedDiffLine,
    ) -> rows::PreparedDiffSyntaxLine {
        use gitcomet_core::domain::DiffLineKind;
        match line.kind {
            DiffLineKind::Remove => {
                let split_ix = line
                    .old_line
                    .and_then(|n| self.file_diff_old_line_to_split_ix.get(&n).copied());
                rows::PreparedDiffSyntaxLine {
                    document: self
                        .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                    line_ix: split_ix.unwrap_or(0),
                }
            }
            DiffLineKind::Add | DiffLineKind::Context => {
                let split_ix = line
                    .new_line
                    .and_then(|n| self.file_diff_new_line_to_split_ix.get(&n).copied());
                rows::PreparedDiffSyntaxLine {
                    document: self
                        .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                    line_ix: split_ix.unwrap_or(0),
                }
            }
            _ => rows::PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            },
        }
    }

    pub(in crate::view) fn file_diff_split_prepared_syntax_document(
        &self,
        region: DiffTextRegion,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let view_mode = match region {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            _ => PreparedSyntaxViewMode::FileDiffSplitRight,
        };
        self.file_diff_prepared_syntax_document(view_mode)
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_key(
        &self,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.active_repo_id()?;
        let path = self.worktree_preview_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.worktree_preview_content_rev,
            path,
            PreparedSyntaxViewMode::WorktreePreview,
        ))
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_document(
        &self,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.worktree_preview_prepared_syntax_key()?;
        self.prepared_syntax_document(&key)
    }

    pub(in crate::view) fn set_worktree_preview_ready_lines(
        &mut self,
        path: std::path::PathBuf,
        lines: Arc<Vec<String>>,
        cx: &mut gpui::Context<Self>,
    ) {
        let source_changed = self.worktree_preview_path.as_ref() != Some(&path)
            || !matches!(
                &self.worktree_preview,
                Loadable::Ready(current) if current.as_ref() == lines.as_ref()
            );

        self.worktree_preview_path = Some(path.clone());
        self.worktree_preview = Loadable::Ready(lines);
        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
        self.worktree_preview_segments_cache_path = Some(path);
        self.worktree_preview_segments_cache.clear();

        if source_changed {
            self.worktree_preview_content_rev = self.worktree_preview_content_rev.wrapping_add(1);
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
        }

        self.refresh_worktree_preview_syntax_document(cx);
    }

    pub(in crate::view) fn refresh_worktree_preview_syntax_document(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(language) = self.worktree_preview_syntax_language else {
            return;
        };
        let Some(key) = self.worktree_preview_prepared_syntax_key() else {
            return;
        };
        let Loadable::Ready(lines) = &self.worktree_preview else {
            return;
        };

        if self.prepared_syntax_document(&key).is_some() {
            return;
        }
        let reparse_seed = self.prepared_syntax_reparse_seed_document(&key);

        let budget = rows::DiffSyntaxBudget::default();
        match rows::prepare_diff_syntax_document_with_budget_reuse(
            language,
            FULL_DOCUMENT_SYNTAX_MODE,
            lines.iter().map(String::as_str),
            budget,
            reparse_seed,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => {
                self.insert_prepared_syntax_document(key, document);
            }
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                let lines = lines.iter().cloned().collect::<Vec<_>>();
                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        let parsed_document = smol::unblock(move || {
                            rows::prepare_diff_syntax_document_in_background(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                lines.iter().map(String::as_str),
                            )
                        })
                        .await;

                        let _ = view.update(cx, |this, cx| {
                            let Some(parsed_document) = parsed_document else {
                                return;
                            };

                            let inserted = this.insert_prepared_syntax_document(
                                key.clone(),
                                rows::inject_background_prepared_diff_syntax_document(
                                    parsed_document,
                                ),
                            );
                            if inserted
                                && this.worktree_preview_prepared_syntax_key().as_ref()
                                    == Some(&key)
                            {
                                this.worktree_preview_style_cache_epoch =
                                    this.worktree_preview_style_cache_epoch.wrapping_add(1);
                                cx.notify();
                            }
                        });
                    },
                )
                .detach();
            }
            rows::PrepareDiffSyntaxDocumentResult::Unsupported => {}
        }
    }

    /// Applies a foreground sync prepare result for one side. Returns `true` if
    /// the side needs a background async parse instead.
    fn apply_sync_syntax_result(
        &mut self,
        attempt: Option<rows::PrepareDiffSyntaxDocumentResult>,
        key: &Option<PreparedSyntaxDocumentKey>,
    ) -> bool {
        match attempt {
            Some(rows::PrepareDiffSyntaxDocumentResult::Ready(document)) => {
                if let Some(key) = key.clone() {
                    self.insert_prepared_syntax_document(key, document);
                }
                false
            }
            Some(rows::PrepareDiffSyntaxDocumentResult::TimedOut) => true,
            _ => false,
        }
    }

    /// Applies background-parsed documents for both sides and reports which
    /// side became newly cacheable.
    fn apply_background_syntax_documents(
        &mut self,
        left_key: &Option<PreparedSyntaxDocumentKey>,
        left_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
        right_key: &Option<PreparedSyntaxDocumentKey>,
        right_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
    ) -> FileDiffPreparedSyntaxApplyResult {
        let mut applied = FileDiffPreparedSyntaxApplyResult::default();
        if let (Some(key), Some(document)) = (left_key.clone(), left_doc) {
            applied.split_left = self.insert_prepared_syntax_document(
                key,
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        if let (Some(key), Some(document)) = (right_key.clone(), right_doc) {
            applied.split_right = self.insert_prepared_syntax_document(
                key,
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        applied
    }

    fn refresh_file_diff_syntax_documents(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(language) = self.file_diff_cache_language else {
            return;
        };

        // Inline syntax is now projected from the split (old/new) documents rather
        // than parsing the synthetic mixed inline stream. Only split documents are
        // prepared here; inline rows look up into them via file_diff_inline_projected_syntax.
        let split_left_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitLeft);
        let split_right_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight);
        let split_left_reparse_seed = split_left_key
            .as_ref()
            .and_then(|key| self.prepared_syntax_reparse_seed_document(key));
        let split_right_reparse_seed = split_right_key
            .as_ref()
            .and_then(|key| self.prepared_syntax_reparse_seed_document(key));

        let needs_split_left_prepare = split_left_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        let needs_split_right_prepare = split_right_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        if !needs_split_left_prepare && !needs_split_right_prepare {
            return;
        }

        let budget = rows::DiffSyntaxBudget::default();

        let split_left_attempt = needs_split_left_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_cache_rows
                    .iter()
                    .map(|row| row.old.as_deref().unwrap_or("")),
                budget,
                split_left_reparse_seed,
            )
        });
        let split_right_attempt = needs_split_right_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_cache_rows
                    .iter()
                    .map(|row| row.new.as_deref().unwrap_or("")),
                budget,
                split_right_reparse_seed,
            )
        });

        let needs_split_left_async =
            self.apply_sync_syntax_result(split_left_attempt, &split_left_key);
        let needs_split_right_async =
            self.apply_sync_syntax_result(split_right_attempt, &split_right_key);

        if !needs_split_left_async && !needs_split_right_async {
            return;
        }

        let syntax_generation = self.file_diff_syntax_generation;
        let repo_id = self.file_diff_cache_repo_id;
        let diff_file_rev = self.file_diff_cache_rev;
        let diff_target = self.file_diff_cache_target.clone();

        let split_left_lines: Option<Vec<String>> = needs_split_left_async.then(|| {
            self.file_diff_cache_rows
                .iter()
                .map(|row| row.old.as_deref().unwrap_or("").to_string())
                .collect()
        });
        let split_right_lines: Option<Vec<String>> = needs_split_right_async.then(|| {
            self.file_diff_cache_rows
                .iter()
                .map(|row| row.new.as_deref().unwrap_or("").to_string())
                .collect()
        });

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let parsed_documents =
                    smol::unblock(move || FileDiffBackgroundPreparedSyntaxDocuments {
                        split_left: split_left_lines.and_then(|lines| {
                            rows::prepare_diff_syntax_document_in_background(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                lines.iter().map(String::as_str),
                            )
                        }),
                        split_right: split_right_lines.and_then(|lines| {
                            rows::prepare_diff_syntax_document_in_background(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                lines.iter().map(String::as_str),
                            )
                        }),
                    })
                    .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_syntax_generation != syntax_generation {
                        return;
                    }
                    if this.file_diff_cache_repo_id != repo_id
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target
                    {
                        return;
                    }

                    let applied = this.apply_background_syntax_documents(
                        &split_left_key,
                        parsed_documents.split_left,
                        &split_right_key,
                        parsed_documents.split_right,
                    );

                    if applied.any() {
                        if applied.split_left {
                            this.file_diff_style_cache_epochs.bump_left();
                        }
                        if applied.split_right {
                            this.file_diff_style_cache_epochs.bump_right();
                        }
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    pub(in super::super::super) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        struct Rebuild {
            file_path: Option<std::path::PathBuf>,
            language: Option<rows::DiffSyntaxLanguage>,
            rows: Vec<FileDiffRow>,
            inline_rows: Vec<AnnotatedDiffLine>,
            inline_text: SharedString,
            inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
            split_word_highlights_old: Vec<Option<Vec<Range<usize>>>>,
            split_word_highlights_new: Vec<Option<Vec<Range<usize>>>>,
        }

        let clear_cache = |this: &mut Self| {
            this.file_diff_cache_repo_id = None;
            this.file_diff_cache_target = None;
            this.file_diff_cache_rev = 0;
            this.file_diff_cache_content_signature = None;
            this.file_diff_cache_inflight = None;
            this.file_diff_syntax_generation = this.file_diff_syntax_generation.wrapping_add(1);
            this.file_diff_style_cache_epochs.bump_both();
            this.file_diff_cache_path = None;
            this.file_diff_cache_language = None;
            this.file_diff_cache_rows.clear();
            this.file_diff_old_line_to_split_ix.clear();
            this.file_diff_new_line_to_split_ix.clear();
            this.file_diff_inline_cache.clear();
            this.file_diff_inline_text = SharedString::default();
            this.file_diff_inline_word_highlights.clear();
            this.file_diff_split_word_highlights_old.clear();
            this.file_diff_split_word_highlights_new.clear();
        };

        let Some((repo_id, diff_file_rev, diff_target, workdir, file)) = (|| {
            let repo = self.active_repo()?;
            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return None;
            }

            let file = match &repo.diff_state.diff_file {
                Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                _ => None,
            };

            Some((
                repo.id,
                repo.diff_state.diff_file_rev,
                repo.diff_state.diff_target.clone(),
                repo.spec.workdir.clone(),
                file,
            ))
        })() else {
            clear_cache(self);
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_diff_text_signature(file.as_ref()));

        let same_repo_and_target = self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_target == diff_target;

        if same_repo_and_target && self.file_diff_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_diff_cache_content_signature == Some(signature)
        {
            // Store-side refreshes can bump diff_file_rev with identical file payloads.
            // Keep the prepared cache and only advance the rev marker after inflight work settles.
            if self.file_diff_cache_inflight.is_none() {
                self.file_diff_cache_rev = diff_file_rev;
            }
            return;
        }

        self.file_diff_cache_repo_id = Some(repo_id);
        self.file_diff_cache_rev = diff_file_rev;
        self.file_diff_cache_content_signature = None;
        self.file_diff_cache_target = diff_target;
        self.file_diff_cache_inflight = None;
        self.file_diff_syntax_generation = self.file_diff_syntax_generation.wrapping_add(1);
        self.file_diff_style_cache_epochs.bump_both();
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows.clear();
        self.file_diff_old_line_to_split_ix.clear();
        self.file_diff_new_line_to_split_ix.clear();
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_text = SharedString::default();
        self.file_diff_inline_word_highlights.clear();
        self.file_diff_split_word_highlights_old.clear();
        self.file_diff_split_word_highlights_new.clear();

        // Reset the segment cache to avoid mixing patch/file indices.
        self.clear_diff_text_style_caches();

        let Some(file) = file else {
            return;
        };
        let content_signature =
            file_content_signature.unwrap_or_else(|| file_diff_text_signature(file.as_ref()));

        self.file_diff_cache_seq = self.file_diff_cache_seq.wrapping_add(1);
        let seq = self.file_diff_cache_seq;
        self.file_diff_cache_inflight = Some(seq);
        self.file_diff_syntax_generation = seq;

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let rebuild = smol::unblock(move || {
                    let old_text = file.old.as_deref().unwrap_or("");
                    let new_text = file.new.as_deref().unwrap_or("");
                    let rows = gitcomet_core::file_diff::side_by_side_rows(old_text, new_text);

                    // Store the file path for syntax highlighting.
                    let file_path = Some(if file.path.is_absolute() {
                        file.path.clone()
                    } else {
                        workdir.join(&file.path)
                    });
                    let language = file_path
                        .as_ref()
                        .and_then(rows::diff_syntax_language_for_path);

                    // Precompute word highlights and inline rows.
                    let mut split_word_highlights_old: Vec<Option<Vec<Range<usize>>>> =
                        vec![None; rows.len()];
                    let mut split_word_highlights_new: Vec<Option<Vec<Range<usize>>>> =
                        vec![None; rows.len()];

                    let mut inline_rows: Vec<AnnotatedDiffLine> =
                        Vec::with_capacity(rows.len().saturating_mul(2));
                    let mut inline_word_highlights: Vec<Option<Vec<Range<usize>>>> =
                        Vec::with_capacity(rows.len().saturating_mul(2));
                    for (row_ix, row) in rows.iter().enumerate() {
                        use gitcomet_core::file_diff::FileDiffRowKind as K;
                        match row.kind {
                            K::Context => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Context,
                                    text: format!(" {}", row.old.as_deref().unwrap_or("")).into(),
                                    old_line: row.old_line,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Add => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Add,
                                    text: format!("+{}", row.new.as_deref().unwrap_or("")).into(),
                                    old_line: None,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Remove => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                                    text: format!("-{}", row.old.as_deref().unwrap_or("")).into(),
                                    old_line: row.old_line,
                                    new_line: None,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Modify => {
                                let old = row.old.as_deref().unwrap_or("");
                                let new = row.new.as_deref().unwrap_or("");
                                let (old_ranges, new_ranges) = capped_word_diff_ranges(old, new);
                                let old_ranges_opt = (!old_ranges.is_empty()).then_some(old_ranges);
                                let new_ranges_opt = (!new_ranges.is_empty()).then_some(new_ranges);

                                split_word_highlights_old[row_ix] = old_ranges_opt.clone();
                                split_word_highlights_new[row_ix] = new_ranges_opt.clone();

                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                                    text: format!("-{}", old).into(),
                                    old_line: row.old_line,
                                    new_line: None,
                                });
                                inline_word_highlights.push(old_ranges_opt);

                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Add,
                                    text: format!("+{}", new).into(),
                                    old_line: None,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(new_ranges_opt);
                            }
                        }
                    }

                    let inline_text = build_inline_text(inline_rows.as_slice());

                    Rebuild {
                        file_path,
                        language,
                        rows,
                        inline_rows,
                        inline_text,
                        inline_word_highlights,
                        split_word_highlights_old,
                        split_word_highlights_new,
                    }
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_cache_inflight != Some(seq) {
                        return;
                    }
                    if this.file_diff_cache_repo_id != Some(repo_id)
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target_for_task
                    {
                        return;
                    }

                    this.file_diff_cache_inflight = None;
                    this.file_diff_cache_path = rebuild.file_path;
                    this.file_diff_cache_language = rebuild.language;
                    let (old_map, new_map) = build_line_to_split_ix_maps(&rebuild.rows);
                    this.file_diff_cache_rows = rebuild.rows;
                    this.file_diff_old_line_to_split_ix = old_map;
                    this.file_diff_new_line_to_split_ix = new_map;
                    this.file_diff_inline_cache = rebuild.inline_rows;
                    this.file_diff_inline_text = rebuild.inline_text;
                    this.file_diff_cache_content_signature = Some(content_signature);
                    this.file_diff_inline_word_highlights = rebuild.inline_word_highlights;
                    this.file_diff_split_word_highlights_old = rebuild.split_word_highlights_old;
                    this.file_diff_split_word_highlights_new = rebuild.split_word_highlights_new;
                    this.refresh_file_diff_syntax_documents(cx);

                    // Reset the segment cache to avoid mixing patch/file indices.
                    this.clear_diff_text_style_caches();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn image_format_for_path(path: &std::path::Path) -> Option<gpui::ImageFormat> {
        image_format_for_path(path)
    }

    pub(in super::super::super) fn ensure_file_image_diff_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        struct Rebuild {
            repo_id: RepoId,
            diff_file_rev: u64,
            diff_target: Option<DiffTarget>,
            file_path: Option<std::path::PathBuf>,
            old: Option<Arc<gpui::Image>>,
            new: Option<Arc<gpui::Image>>,
            old_svg_path: Option<std::path::PathBuf>,
            new_svg_path: Option<std::path::PathBuf>,
        }

        struct RebuildSvgAsync {
            repo_id: RepoId,
            diff_file_rev: u64,
            diff_target: Option<DiffTarget>,
            file_path: Option<std::path::PathBuf>,
            old_svg_bytes: Option<Vec<u8>>,
            new_svg_bytes: Option<Vec<u8>>,
        }

        enum Action {
            Clear,
            Noop,
            Reset {
                repo_id: RepoId,
                diff_file_rev: u64,
                diff_target: Option<DiffTarget>,
            },
            Rebuild(Rebuild),
            RebuildSvgAsync(RebuildSvgAsync),
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };

            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return Action::Clear;
            }

            if self.file_image_diff_cache_repo_id == Some(repo.id)
                && self.file_image_diff_cache_rev == repo.diff_state.diff_file_rev
                && self.file_image_diff_cache_target.as_ref()
                    == repo.diff_state.diff_target.as_ref()
            {
                return Action::Noop;
            }

            let repo_id = repo.id;
            let diff_file_rev = repo.diff_state.diff_file_rev;
            let diff_target = repo.diff_state.diff_target.clone();

            let Loadable::Ready(file_opt) = &repo.diff_state.diff_file_image else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };
            let Some(file) = file_opt.as_ref() else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };

            let format = Self::image_format_for_path(&file.path);
            let is_ico = file
                .path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"));
            let workdir = &repo.spec.workdir;
            let file_path = Some(if file.path.is_absolute() {
                file.path.clone()
            } else {
                workdir.join(&file.path)
            });

            if !is_ico && format == Some(gpui::ImageFormat::Svg) {
                return Action::RebuildSvgAsync(RebuildSvgAsync {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                    file_path,
                    old_svg_bytes: file.old.clone(),
                    new_svg_bytes: file.new.clone(),
                });
            }

            let mut old_svg_path = None;
            let mut new_svg_path = None;
            let old = file.old.as_ref().and_then(|bytes| {
                if is_ico {
                    old_svg_path = cached_image_diff_path(bytes, "ico");
                    None
                } else {
                    format.and_then(|format| {
                        decode_file_image_diff_bytes(format, bytes, Some(&mut old_svg_path))
                    })
                }
            });
            let new = file.new.as_ref().and_then(|bytes| {
                if is_ico {
                    new_svg_path = cached_image_diff_path(bytes, "ico");
                    None
                } else {
                    format.and_then(|format| {
                        decode_file_image_diff_bytes(format, bytes, Some(&mut new_svg_path))
                    })
                }
            });

            Action::Rebuild(Rebuild {
                repo_id,
                diff_file_rev,
                diff_target,
                file_path,
                old,
                new,
                old_svg_path,
                new_svg_path,
            })
        })();

        match action {
            Action::Noop => {}
            Action::Clear => {
                self.file_image_diff_cache_repo_id = None;
                self.file_image_diff_cache_target = None;
                self.file_image_diff_cache_rev = 0;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Reset {
                repo_id,
                diff_file_rev,
                diff_target,
            } => {
                self.file_image_diff_cache_repo_id = Some(repo_id);
                self.file_image_diff_cache_rev = diff_file_rev;
                self.file_image_diff_cache_target = diff_target;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Rebuild(rebuild) => {
                self.file_image_diff_cache_repo_id = Some(rebuild.repo_id);
                self.file_image_diff_cache_rev = rebuild.diff_file_rev;
                self.file_image_diff_cache_target = rebuild.diff_target;
                self.file_image_diff_cache_path = rebuild.file_path;
                self.file_image_diff_cache_old = rebuild.old;
                self.file_image_diff_cache_new = rebuild.new;
                self.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                self.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
            }
            Action::RebuildSvgAsync(rebuild) => {
                self.file_image_diff_cache_repo_id = Some(rebuild.repo_id);
                self.file_image_diff_cache_rev = rebuild.diff_file_rev;
                self.file_image_diff_cache_target = rebuild.diff_target.clone();
                self.file_image_diff_cache_path = rebuild.file_path.clone();
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;

                let repo_id = rebuild.repo_id;
                let diff_file_rev = rebuild.diff_file_rev;
                let diff_target_for_task = rebuild.diff_target.clone();
                let file_path_for_task = rebuild.file_path;
                let old_svg_bytes = rebuild.old_svg_bytes;
                let new_svg_bytes = rebuild.new_svg_bytes;

                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        let (old_png, old_svg_path, new_png, new_svg_path) =
                            smol::unblock(move || {
                                let (old_png, old_svg_path) = old_svg_bytes
                                    .as_deref()
                                    .map(rasterize_svg_preview_png_or_cached_path)
                                    .unwrap_or((None, None));
                                let (new_png, new_svg_path) = new_svg_bytes
                                    .as_deref()
                                    .map(rasterize_svg_preview_png_or_cached_path)
                                    .unwrap_or((None, None));
                                (old_png, old_svg_path, new_png, new_svg_path)
                            })
                            .await;

                        let _ = view.update(cx, |this, cx| {
                            if this.file_image_diff_cache_repo_id != Some(repo_id)
                                || this.file_image_diff_cache_rev != diff_file_rev
                                || this.file_image_diff_cache_target != diff_target_for_task
                            {
                                return;
                            }

                            this.file_image_diff_cache_path = file_path_for_task;
                            this.file_image_diff_cache_old = old_png.map(|png| {
                                Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))
                            });
                            this.file_image_diff_cache_new = new_png.map(|png| {
                                Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))
                            });
                            this.file_image_diff_cache_old_svg_path = old_svg_path;
                            this.file_image_diff_cache_new_svg_path = new_svg_path;
                            cx.notify();
                        });
                    },
                )
                .detach();
            }
        }
    }

    pub(in super::super::super) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        self.diff_cache.clear();
        self.diff_row_provider = None;
        self.diff_split_row_provider = None;
        self.diff_cache_repo_id = None;
        self.diff_cache_rev = 0;
        self.diff_cache_target = None;
        self.diff_file_for_src_ix.clear();
        self.diff_language_for_src_ix.clear();
        self.diff_click_kinds.clear();
        self.diff_line_kind_for_src_ix.clear();
        self.diff_hide_unified_header_for_src_ix.clear();
        self.diff_header_display_cache.clear();
        self.diff_split_cache.clear();
        self.diff_split_cache_len = 0;
        self.diff_visible_indices.clear();
        self.diff_visible_inline_map = None;
        self.diff_visible_cache_len = 0;
        self.diff_visible_is_file_view = false;
        self.diff_scrollbar_markers_cache.clear();
        self.diff_word_highlights.clear();
        self.diff_word_highlights_inflight = None;
        self.diff_file_stats.clear();
        self.clear_diff_text_style_caches();
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.diff_preview_is_new_file = false;
        self.diff_preview_new_file_lines = Arc::new(Vec::new());

        let (repo_id, diff_rev, diff_target, workdir, diff) = {
            let Some(repo) = self.active_repo() else {
                return;
            };
            let workdir = repo.spec.workdir.clone();
            let diff = match &repo.diff_state.diff {
                Loadable::Ready(diff) => Some(Arc::clone(diff)),
                _ => None,
            };
            (
                repo.id,
                repo.diff_state.diff_rev,
                repo.diff_state.diff_target.clone(),
                workdir,
                diff,
            )
        };

        self.diff_cache_repo_id = Some(repo_id);
        self.diff_cache_rev = diff_rev;
        self.diff_cache_target = diff_target;

        let Some(diff) = diff else {
            return;
        };

        let row_provider = Arc::new(PagedPatchDiffRows::new(
            Arc::clone(&diff),
            PATCH_DIFF_PAGE_SIZE,
        ));
        let split_row_provider = Arc::new(PagedPatchSplitRows::new(Arc::clone(&row_provider)));
        self.diff_row_provider = Some(row_provider);
        self.diff_split_row_provider = Some(split_row_provider);

        self.diff_file_for_src_ix = compute_diff_file_for_src_ix(diff.lines.as_slice());
        self.diff_line_kind_for_src_ix = diff.lines.iter().map(|line| line.kind).collect();
        self.diff_hide_unified_header_for_src_ix = diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_raw(line.kind, line.text.as_ref()))
            .collect();
        self.diff_click_kinds = diff
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
            .collect();
        for (src_ix, click_kind) in self.diff_click_kinds.iter().enumerate() {
            match click_kind {
                DiffClickKind::FileHeader => {
                    let Some(line) = diff.lines.get(src_ix) else {
                        continue;
                    };
                    let display = parse_diff_git_header_path(line.text.as_ref())
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::HunkHeader => {
                    let Some(line) = diff.lines.get(src_ix) else {
                        continue;
                    };
                    let display = parse_unified_hunk_header_for_display(line.text.as_ref())
                        .map(|p| {
                            let heading = p.heading.unwrap_or_default();
                            if heading.is_empty() {
                                format!("{} {}", p.old, p.new)
                            } else {
                                format!("{} {}  {heading}", p.old, p.new)
                            }
                        })
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::Line => {}
            }
        }
        self.diff_file_stats = compute_diff_file_stats(diff.lines.as_slice());
        self.diff_word_highlights = vec![None; self.patch_diff_row_len()];
        self.diff_word_highlights_inflight = None;

        let mut current_file: Option<Arc<str>> = None;
        let mut current_language: Option<rows::DiffSyntaxLanguage> = None;
        for (src_ix, line) in diff.lines.iter().enumerate() {
            let file = self
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|p| p.as_ref());
            let file_changed = match (&current_file, file) {
                (Some(cur), Some(next)) => !Arc::ptr_eq(cur, next),
                (None, None) => false,
                _ => true,
            };
            if file_changed {
                current_file = file.cloned();
                current_language =
                    file.and_then(|p| rows::diff_syntax_language_for_path(p.as_ref()));
            }

            let language = match line.kind {
                gitcomet_core::domain::DiffLineKind::Add
                | gitcomet_core::domain::DiffLineKind::Remove
                | gitcomet_core::domain::DiffLineKind::Context => current_language,
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => None,
            };
            self.diff_language_for_src_ix.push(language);
        }

        if let Some((abs_path, lines)) = build_new_file_preview_from_diff(
            diff.lines.as_slice(),
            &workdir,
            self.diff_cache_target.as_ref(),
        ) {
            self.diff_preview_is_new_file = true;
            let preview_lines = Arc::new(lines);
            self.diff_preview_new_file_lines = Arc::clone(&preview_lines);
            self.set_worktree_preview_ready_lines(abs_path, preview_lines, cx);
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }
    }

    fn ensure_diff_split_cache(&mut self) {
        if self.diff_split_row_provider.is_some() {
            return;
        }
        if self.diff_split_cache_len == self.diff_cache.len() && !self.diff_split_cache.is_empty() {
            return;
        }
        self.diff_split_cache_len = self.diff_cache.len();
        self.diff_split_cache = build_patch_split_rows(&self.diff_cache);
    }

    fn diff_scrollbar_markers_patch(&self) -> Vec<components::ScrollbarMarker> {
        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(src_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.patch_diff_row(src_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.visible_indices.as_slice(), self.diff_visible_indices);
                    return scrollbar_markers_from_visible_flags(meta.visible_flags.as_slice());
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.patch_diff_split_row(row_ix) else {
                        return 0;
                    };
                    match &row {
                        PatchSplitRow::Aligned { row, .. } => match row.kind {
                            gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                            gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                            gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                            gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        },
                        PatchSplitRow::Raw { .. } => 0,
                    }
                })
            }
        }
    }

    fn compute_diff_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        if !self.is_file_diff_view_active() {
            return self.diff_scrollbar_markers_patch();
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(inline_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.file_diff_inline_cache.get(inline_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.file_diff_cache_rows.get(row_ix) else {
                        return 0;
                    };
                    match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                    }
                })
            }
        }
    }

    pub(in super::super::super) fn ensure_diff_visible_indices(&mut self) {
        let is_file_view = self.is_file_diff_view_active();
        let current_len = if is_file_view {
            match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_cache.len(),
                DiffViewMode::Split => self.file_diff_cache_rows.len(),
            }
        } else {
            match self.diff_view {
                DiffViewMode::Inline => self.patch_diff_row_len(),
                DiffViewMode::Split => self.patch_diff_split_row_len(),
            }
        };

        if self.diff_visible_cache_len == current_len
            && self.diff_visible_view == self.diff_view
            && self.diff_visible_is_file_view == is_file_view
        {
            return;
        }

        self.diff_visible_cache_len = current_len;
        self.diff_visible_view = self.diff_view;
        self.diff_visible_is_file_view = is_file_view;
        self.diff_horizontal_min_width = px(0.0);
        self.diff_visible_inline_map = None;

        if is_file_view {
            self.diff_visible_indices = (0..current_len).collect();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
                self.diff_search_recompute_matches_for_current_view();
            }
            return;
        }

        let mut split_visible_flags: Option<Vec<u8>> = None;
        match self.diff_view {
            DiffViewMode::Inline => {
                if self.diff_hide_unified_header_for_src_ix.len() == current_len {
                    self.diff_visible_inline_map = Some(PatchInlineVisibleMap::from_hidden_flags(
                        self.diff_hide_unified_header_for_src_ix.as_slice(),
                    ));
                    self.diff_visible_indices = Vec::new();
                } else {
                    self.diff_visible_indices = self
                        .patch_diff_rows_slice(0, current_len)
                        .into_iter()
                        .enumerate()
                        .filter_map(|(ix, line)| {
                            (!should_hide_unified_diff_header_line(&line)).then_some(ix)
                        })
                        .collect();
                }
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.total_rows, current_len);
                    self.diff_visible_indices = meta.visible_indices;
                    split_visible_flags = Some(meta.visible_flags);
                } else {
                    self.ensure_diff_split_cache();

                    self.diff_visible_indices = self
                        .diff_split_cache
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, row)| match row {
                            PatchSplitRow::Raw { src_ix, .. } => self
                                .diff_cache
                                .get(*src_ix)
                                .is_some_and(|line| !should_hide_unified_diff_header_line(line))
                                .then_some(ix),
                            PatchSplitRow::Aligned { .. } => Some(ix),
                        })
                        .collect();
                }
            }
        }

        self.diff_scrollbar_markers_cache = split_visible_flags
            .map(|flags| scrollbar_markers_from_visible_flags(flags.as_slice()))
            .unwrap_or_else(|| self.compute_diff_scrollbar_markers());

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches_for_current_view();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Diff, DiffArea, DiffTarget};
    use std::path::Path;
    use std::path::PathBuf;

    fn write_test_file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write test file");
        path
    }

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
    fn build_inline_text_joins_lines_with_trailing_newline() {
        let rows = vec![
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: Arc::from("diff --git a/file b/file"),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-old"),
                old_line: Some(1),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: Arc::from("+new"),
                old_line: None,
                new_line: Some(1),
            },
        ];

        let text = build_inline_text(rows.as_slice());
        assert_eq!(text.as_ref(), "diff --git a/file b/file\n-old\n+new\n");
    }

    #[test]
    fn build_inline_text_returns_empty_for_empty_rows() {
        let text = build_inline_text(&[]);
        assert!(text.as_ref().is_empty());
    }

    #[test]
    fn full_document_syntax_mode_is_always_auto() {
        assert_eq!(FULL_DOCUMENT_SYNTAX_MODE, rows::DiffSyntaxMode::Auto);
    }

    #[test]
    fn file_diff_style_cache_epochs_map_rows_to_matching_side() {
        let epochs = FileDiffStyleCacheEpochs {
            split_left: 11,
            split_right: 23,
        };

        assert_eq!(
            epochs.split_epoch(crate::view::DiffTextRegion::SplitLeft),
            11
        );
        assert_eq!(
            epochs.split_epoch(crate::view::DiffTextRegion::SplitRight),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Remove),
            11
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Add),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Context),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Header),
            0
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Hunk),
            0
        );
    }

    #[test]
    fn file_diff_style_cache_epochs_bump_only_changed_side() {
        let mut epochs = FileDiffStyleCacheEpochs {
            split_left: 5,
            split_right: 9,
        };

        epochs.bump_left();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 6,
                split_right: 9,
            }
        );

        epochs.bump_right();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 6,
                split_right: 10,
            }
        );

        epochs.bump_both();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 7,
                split_right: 11,
            }
        );
    }

    #[test]
    fn prepared_syntax_document_key_includes_repo_rev_path_and_view_mode() {
        let path = Path::new("src/lib.rs");
        let base = prepared_syntax_document_key(
            RepoId(7),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_rev = prepared_syntax_document_key(
            RepoId(7),
            43,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_view_mode = prepared_syntax_document_key(
            RepoId(7),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitLeft,
        );
        let different_repo = prepared_syntax_document_key(
            RepoId(8),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_path = prepared_syntax_document_key(
            RepoId(7),
            42,
            Path::new("src/main.rs"),
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );

        assert_ne!(base, different_rev);
        assert_ne!(base, different_view_mode);
        assert_ne!(base, different_repo);
        assert_ne!(base, different_path);
    }

    #[test]
    fn image_format_for_path_detects_known_extensions_case_insensitively() {
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.PNG")),
            Some(gpui::ImageFormat::Png)
        );
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.JpEg")),
            Some(gpui::ImageFormat::Jpeg)
        );
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.webp")),
            Some(gpui::ImageFormat::Webp)
        );
    }

    #[test]
    fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.heic")),
            None
        );
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.ico")),
            None
        );
        assert_eq!(MainPaneView::image_format_for_path(Path::new("x")), None);
    }

    #[test]
    fn decode_file_image_diff_bytes_keeps_non_svg_bytes() {
        let bytes = [1_u8, 2, 3, 4, 5];
        let mut svg_path = None;
        let image =
            decode_file_image_diff_bytes(gpui::ImageFormat::Png, &bytes, Some(&mut svg_path))
                .unwrap();
        assert_eq!(image.format(), gpui::ImageFormat::Png);
        assert_eq!(image.bytes(), bytes);
        assert!(svg_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_bytes_rasterizes_svg_to_png() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<rect width="16" height="16" fill="#00aaff"/>
</svg>"##;
        let mut svg_path = None;
        let image = decode_file_image_diff_bytes(gpui::ImageFormat::Svg, svg, Some(&mut svg_path));
        let image = image.expect("svg should rasterize to image");
        assert_eq!(image.format(), gpui::ImageFormat::Png);
        assert!(svg_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_bytes_keeps_svg_path_fallback_for_invalid_svg() {
        let mut svg_path = None;
        let image = decode_file_image_diff_bytes(
            gpui::ImageFormat::Svg,
            b"<not-valid-svg>",
            Some(&mut svg_path),
        );
        assert!(image.is_none());
        assert!(svg_path.is_some());
        assert!(svg_path.unwrap().exists());
    }

    #[test]
    fn rasterize_svg_preview_png_or_cached_path_returns_png_for_valid_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8">
<circle cx="4" cy="4" r="3" fill="#55aa00"/>
</svg>"##;
        let (png, svg_path) = rasterize_svg_preview_png_or_cached_path(svg);
        let png = png.expect("svg should rasterize to png bytes");
        assert!(svg_path.is_none());
        assert!(png.len() >= 8);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn rasterize_svg_preview_png_or_cached_path_falls_back_to_svg_file_for_invalid_svg() {
        let (png, svg_path) = rasterize_svg_preview_png_or_cached_path(b"<not-valid-svg>");
        assert!(png.is_none());
        let svg_path = svg_path.expect("invalid svg should produce fallback path");
        assert!(svg_path.exists());
        assert_eq!(svg_path.extension().and_then(|s| s.to_str()), Some("svg"));
    }

    #[test]
    fn cached_image_diff_path_writes_ico_cache_file() {
        let bytes = [0_u8, 0, 1, 0, 1, 0, 16, 16];
        let path = cached_image_diff_path(&bytes, "ico").expect("cached path");
        assert!(path.exists());
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("ico"));
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

    #[test]
    fn cleanup_image_diff_cache_dir_removes_stale_prefixed_files() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let stale = write_test_file(
            temp_dir.path(),
            "gitcomet-image-diff-stale.svg",
            b"old-cache",
        );
        let non_cache = write_test_file(temp_dir.path(), "keep-me.txt", b"keep");

        cleanup_image_diff_cache_dir(
            temp_dir.path(),
            std::time::Duration::from_secs(60),
            u64::MAX,
            std::time::SystemTime::now() + std::time::Duration::from_secs(60 * 60),
        )
        .expect("cleanup");

        assert!(!stale.exists());
        assert!(non_cache.exists());
    }

    #[test]
    fn cleanup_image_diff_cache_dir_prunes_to_max_total_size() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let a = write_test_file(temp_dir.path(), "gitcomet-image-diff-a.svg", b"1234");
        let b = write_test_file(temp_dir.path(), "gitcomet-image-diff-b.svg", b"1234");
        let c = write_test_file(temp_dir.path(), "gitcomet-image-diff-c.svg", b"1234");
        let non_cache = write_test_file(temp_dir.path(), "unrelated.bin", b"1234567890");

        cleanup_image_diff_cache_dir(
            temp_dir.path(),
            std::time::Duration::from_secs(60 * 60 * 24),
            8,
            std::time::SystemTime::now(),
        )
        .expect("cleanup");

        let cache_paths = [&a, &b, &c];
        let remaining_count = cache_paths.iter().filter(|path| path.exists()).count();
        assert_eq!(remaining_count, 2);

        let remaining_total = cache_paths
            .iter()
            .filter(|path| path.exists())
            .map(|path| std::fs::metadata(path).expect("metadata").len())
            .sum::<u64>();
        assert!(remaining_total <= 8);
        assert!(non_cache.exists());
    }

    #[test]
    fn build_line_to_split_ix_maps_indexes_old_and_new_lines() {
        use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

        let rows = vec![
            FileDiffRow {
                kind: FileDiffRowKind::Context,
                old_line: Some(1),
                new_line: Some(1),
                old: Some("a".to_string()),
                new: Some("a".to_string()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: FileDiffRowKind::Remove,
                old_line: Some(2),
                new_line: None,
                old: Some("b".to_string()),
                new: None,
                eof_newline: None,
            },
            FileDiffRow {
                kind: FileDiffRowKind::Add,
                old_line: None,
                new_line: Some(2),
                old: None,
                new: Some("c".to_string()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: FileDiffRowKind::Context,
                old_line: Some(3),
                new_line: Some(3),
                old: Some("d".to_string()),
                new: Some("d".to_string()),
                eof_newline: None,
            },
        ];

        let (old_map, new_map) = build_line_to_split_ix_maps(&rows);

        // old_line 1 → row 0, old_line 2 → row 1, old_line 3 → row 3
        assert_eq!(old_map.get(&1), Some(&0));
        assert_eq!(old_map.get(&2), Some(&1));
        assert_eq!(old_map.get(&3), Some(&3));
        assert_eq!(old_map.get(&4), None);

        // new_line 1 → row 0, new_line 2 → row 2, new_line 3 → row 3
        assert_eq!(new_map.get(&1), Some(&0));
        assert_eq!(new_map.get(&2), Some(&2));
        assert_eq!(new_map.get(&3), Some(&3));
        assert_eq!(new_map.get(&4), None);
    }

    #[test]
    fn build_line_to_split_ix_maps_empty() {
        let (old_map, new_map) = build_line_to_split_ix_maps(&[]);
        assert!(old_map.is_empty());
        assert!(new_map.is_empty());
    }
}
