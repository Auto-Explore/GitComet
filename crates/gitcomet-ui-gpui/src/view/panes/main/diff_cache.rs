use super::*;
use crate::view::markdown_preview;
use crate::view::perf::{self, ViewPerfSpan};
use gitcomet_core::domain::DiffRowProvider;

const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
const PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES: usize = 256;
const PATCH_DIFF_PAGE_SIZE: usize = 256;
const FILE_DIFF_PAGE_SIZE: usize = 256;
const FILE_DIFF_MAX_CACHED_PAGES: usize = 64;
const STREAMED_CACHE_PARTIAL_EVICT_DIVISOR: usize = 8;

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

fn file_diff_text_signature(file: &gitcomet_core::domain::FileDiffText) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    file.path.hash(&mut hasher);
    file.old.hash(&mut hasher);
    file.new.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
fn preview_lines_source_len(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.len())
        .sum::<usize>()
        .saturating_add(lines.len().saturating_sub(1))
}

fn build_file_diff_document_source(text: Option<&str>) -> (SharedString, Arc<[usize]>) {
    let text: SharedString = text.unwrap_or_default().to_owned().into();
    let line_starts = Arc::from(build_line_starts(text.as_ref()));
    (text, line_starts)
}

fn insert_streamed_cache_entry<K, V>(
    cache: &mut HashMap<K, V>,
    key: K,
    value: V,
    max_entries: usize,
) where
    K: Clone + Eq + std::hash::Hash,
{
    if max_entries == 0 {
        return;
    }

    if !cache.contains_key(&key) && cache.len() >= max_entries {
        let evict_count = (max_entries / STREAMED_CACHE_PARTIAL_EVICT_DIVISOR).max(1);
        let target_len = max_entries.saturating_sub(evict_count);
        let remove_count = cache.len().saturating_sub(target_len);
        let keys_to_remove: Vec<K> = cache.keys().take(remove_count).cloned().collect();
        for old_key in keys_to_remove {
            cache.remove(&old_key);
        }
    }

    cache.insert(key, value);
}

fn line_number(line_ix: usize) -> Option<u32> {
    line_ix
        .checked_add(1)
        .and_then(|line| u32::try_from(line).ok())
}

fn file_diff_row_flag(kind: gitcomet_core::file_diff::FileDiffRowKind) -> u8 {
    match kind {
        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
    }
}

fn scrollbar_markers_from_row_ranges(
    len: usize,
    ranges: impl IntoIterator<Item = (usize, usize, u8)>,
) -> Vec<components::ScrollbarMarker> {
    if len == 0 {
        return Vec::new();
    }

    let bucket_count = 240usize.min(len).max(1);
    let mut buckets = vec![0u8; bucket_count];
    for (start, end, flag) in ranges {
        if flag == 0 || start >= end || start >= len {
            continue;
        }
        let clamped_end = end.min(len);
        if clamped_end <= start {
            continue;
        }
        let bucket_start = (start * bucket_count) / len;
        let bucket_end = ((clamped_end - 1) * bucket_count) / len;
        for bucket_ix in bucket_start..=bucket_end.min(bucket_count.saturating_sub(1)) {
            if let Some(cell) = buckets.get_mut(bucket_ix) {
                *cell |= flag;
            }
        }
    }

    let mut out = Vec::with_capacity(bucket_count);
    let mut ix = 0usize;
    while ix < bucket_count {
        let flag = buckets[ix];
        if flag == 0 {
            ix += 1;
            continue;
        }

        let start = ix;
        ix += 1;
        while ix < bucket_count && buckets[ix] == flag {
            ix += 1;
        }

        let kind = match flag {
            1 => components::ScrollbarMarkerKind::Add,
            2 => components::ScrollbarMarkerKind::Remove,
            _ => components::ScrollbarMarkerKind::Modify,
        };

        out.push(components::ScrollbarMarker {
            start: start as f32 / bucket_count as f32,
            end: ix as f32 / bucket_count as f32,
            kind,
        });
    }

    out
}

#[derive(Debug)]
struct StreamedFileDiffSource {
    plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
    old_text: SharedString,
    old_line_starts: Arc<[usize]>,
    new_text: SharedString,
    new_line_starts: Arc<[usize]>,
    split_run_starts: Vec<usize>,
    inline_run_starts: Vec<usize>,
}

impl StreamedFileDiffSource {
    fn new(
        plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
        old_text: SharedString,
        old_line_starts: Arc<[usize]>,
        new_text: SharedString,
        new_line_starts: Arc<[usize]>,
    ) -> Self {
        let mut split_run_starts = Vec::with_capacity(plan.runs.len());
        let mut inline_run_starts = Vec::with_capacity(plan.runs.len());
        let mut split_start = 0usize;
        let mut inline_start = 0usize;
        for run in &plan.runs {
            split_run_starts.push(split_start);
            inline_run_starts.push(inline_start);
            split_start = split_start.saturating_add(run.row_len());
            inline_start = inline_start.saturating_add(run.inline_row_len());
        }

        Self {
            plan,
            old_text,
            old_line_starts,
            new_text,
            new_line_starts,
            split_run_starts,
            inline_run_starts,
        }
    }

    fn split_len(&self) -> usize {
        self.plan.row_count
    }

    fn inline_len(&self) -> usize {
        self.plan.inline_row_count
    }

    fn old_line_text(&self, line_ix: usize) -> &str {
        rows::resolved_output_line_text(
            self.old_text.as_ref(),
            self.old_line_starts.as_ref(),
            line_ix,
        )
    }

    fn new_line_text(&self, line_ix: usize) -> &str {
        rows::resolved_output_line_text(
            self.new_text.as_ref(),
            self.new_line_starts.as_ref(),
            line_ix,
        )
    }

    fn locate_run(starts: &[usize], total_len: usize, row_ix: usize) -> Option<(usize, usize)> {
        if row_ix >= total_len || starts.is_empty() {
            return None;
        }
        let run_ix = starts
            .partition_point(|&start| start <= row_ix)
            .saturating_sub(1);
        let run_start = *starts.get(run_ix)?;
        Some((run_ix, row_ix.saturating_sub(run_start)))
    }

    fn split_row(&self, row_ix: usize) -> Option<FileDiffRow> {
        let (run_ix, local_ix) = Self::locate_run(
            self.split_run_starts.as_slice(),
            self.plan.row_count,
            row_ix,
        )?;
        let run = self.plan.runs.get(run_ix)?;
        let mut row = match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                let text = self.old_line_text(old_ix).to_string();
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                    old: Some(text.clone()),
                    new: Some(text),
                    eof_newline: None,
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Remove,
                    old_line: line_number(old_ix),
                    new_line: None,
                    old: Some(self.old_line_text(old_ix).to_string()),
                    new: None,
                    eof_newline: None,
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Add,
                    old_line: None,
                    new_line: line_number(new_ix),
                    old: None,
                    new: Some(self.new_line_text(new_ix).to_string()),
                    eof_newline: None,
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Modify,
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                    old: Some(self.old_line_text(old_ix).to_string()),
                    new: Some(self.new_line_text(new_ix).to_string()),
                    eof_newline: None,
                }
            }
        };

        if row_ix + 1 == self.plan.row_count {
            row.eof_newline = self.plan.eof_newline;
        }
        Some(row)
    }

    fn inline_row(&self, inline_ix: usize) -> Option<AnnotatedDiffLine> {
        let (run_ix, local_ix) = Self::locate_run(
            self.inline_run_starts.as_slice(),
            self.plan.inline_row_count,
            inline_ix,
        )?;
        let run = self.plan.runs.get(run_ix)?;
        match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Context,
                    text: format!(" {}", self.old_line_text(old_ix)).into(),
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                    text: format!("-{}", self.old_line_text(old_ix)).into(),
                    old_line: line_number(old_ix),
                    new_line: None,
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Add,
                    text: format!("+{}", self.new_line_text(new_ix)).into(),
                    old_line: None,
                    new_line: line_number(new_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                ..
            } => {
                let pair_ix = local_ix / 2;
                let old_ix = old_start.saturating_add(pair_ix);
                let new_ix = new_start.saturating_add(pair_ix);
                if local_ix % 2 == 0 {
                    Some(AnnotatedDiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Remove,
                        text: format!("-{}", self.old_line_text(old_ix)).into(),
                        old_line: line_number(old_ix),
                        new_line: None,
                    })
                } else {
                    Some(AnnotatedDiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Add,
                        text: format!("+{}", self.new_line_text(new_ix)).into(),
                        old_line: None,
                        new_line: line_number(new_ix),
                    })
                }
            }
        }
    }

    fn split_modify_pair_texts(&self, row_ix: usize) -> Option<(&str, &str)> {
        let (run_ix, local_ix) = Self::locate_run(
            self.split_run_starts.as_slice(),
            self.plan.row_count,
            row_ix,
        )?;
        let gitcomet_core::file_diff::FileDiffPlanRun::Modify {
            old_start,
            new_start,
            ..
        } = self.plan.runs.get(run_ix)?
        else {
            return None;
        };
        let old_ix = old_start.saturating_add(local_ix);
        let new_ix = new_start.saturating_add(local_ix);
        Some((self.old_line_text(old_ix), self.new_line_text(new_ix)))
    }

    fn inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(&str, &str, gitcomet_core::domain::DiffLineKind)> {
        let (run_ix, local_ix) = Self::locate_run(
            self.inline_run_starts.as_slice(),
            self.plan.inline_row_count,
            inline_ix,
        )?;
        let gitcomet_core::file_diff::FileDiffPlanRun::Modify {
            old_start,
            new_start,
            ..
        } = self.plan.runs.get(run_ix)?
        else {
            return None;
        };
        let pair_ix = local_ix / 2;
        let kind = if local_ix % 2 == 0 {
            gitcomet_core::domain::DiffLineKind::Remove
        } else {
            gitcomet_core::domain::DiffLineKind::Add
        };
        let old_ix = old_start.saturating_add(pair_ix);
        let new_ix = new_start.saturating_add(pair_ix);
        Some((self.old_line_text(old_ix), self.new_line_text(new_ix), kind))
    }

    fn change_visible_indices_for_runs(&self, inline: bool) -> Vec<usize> {
        let starts = if inline {
            self.inline_run_starts.as_slice()
        } else {
            self.split_run_starts.as_slice()
        };
        let mut out = Vec::new();
        let mut in_change_block = false;

        for (run_ix, run) in self.plan.runs.iter().enumerate() {
            let is_change = !matches!(
                run.kind(),
                gitcomet_core::file_diff::FileDiffRowKind::Context
            );
            if is_change
                && !in_change_block
                && let Some(start) = starts.get(run_ix).copied()
            {
                out.push(start);
            }
            in_change_block = is_change;
        }

        out
    }

    fn split_change_visible_indices(&self) -> Vec<usize> {
        self.change_visible_indices_for_runs(false)
    }

    fn inline_change_visible_indices(&self) -> Vec<usize> {
        self.change_visible_indices_for_runs(true)
    }

    fn split_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        scrollbar_markers_from_row_ranges(
            self.plan.row_count,
            self.plan.runs.iter().enumerate().map(|(run_ix, run)| {
                let start = self.split_run_starts.get(run_ix).copied().unwrap_or(0);
                let end = start.saturating_add(run.row_len());
                (start, end, file_diff_row_flag(run.kind()))
            }),
        )
    }

    fn inline_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        scrollbar_markers_from_row_ranges(
            self.plan.inline_row_count,
            self.plan.runs.iter().enumerate().map(|(run_ix, run)| {
                let start = self.inline_run_starts.get(run_ix).copied().unwrap_or(0);
                let end = start.saturating_add(run.inline_row_len());
                let flag = match run.kind() {
                    gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                    gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                    gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                    gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                };
                (start, end, flag)
            }),
        )
    }

    fn build_inline_text(&self) -> SharedString {
        let total_len = self.inline_len();
        let mut text = String::new();
        text.reserve(total_len.saturating_mul(32));
        for inline_ix in 0..total_len {
            let Some(line) = self.inline_row(inline_ix) else {
                continue;
            };
            text.push_str(line.text.as_ref());
            text.push('\n');
        }
        SharedString::from(text)
    }
}

#[derive(Debug)]
pub(in crate::view) struct PagedFileDiffRows {
    source: Arc<StreamedFileDiffSource>,
    page_size: usize,
    pages: std::sync::Mutex<HashMap<usize, Arc<[FileDiffRow]>>>,
}

impl PagedFileDiffRows {
    fn new(source: Arc<StreamedFileDiffSource>, page_size: usize) -> Self {
        Self {
            source,
            page_size: page_size.max(1),
            pages: std::sync::Mutex::new(HashMap::default()),
        }
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.source.split_len()).then(|| {
            let end = start
                .saturating_add(self.page_size)
                .min(self.source.split_len());
            (start, end)
        })
    }

    fn build_page(&self, page_ix: usize) -> Option<Arc<[FileDiffRow]>> {
        let (start, end) = self.page_bounds(page_ix)?;
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        for row_ix in start..end {
            rows.push(self.source.split_row(row_ix)?);
        }
        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[FileDiffRow]>> {
        if let Ok(pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
        if let Ok(mut pages) = self.pages.lock() {
            insert_streamed_cache_entry(
                &mut pages,
                page_ix,
                Arc::clone(&page),
                FILE_DIFF_MAX_CACHED_PAGES,
            );
            return Some(Arc::clone(pages.get(&page_ix).unwrap_or(&page)));
        }
        Some(page)
    }

    fn row_at(&self, row_ix: usize) -> Option<FileDiffRow> {
        let page_ix = row_ix / self.page_size;
        let page_row_ix = row_ix % self.page_size;
        let page = self.load_page(page_ix)?;
        page.get(page_row_ix).cloned()
    }

    pub(in crate::view) fn change_visible_indices(&self) -> Vec<usize> {
        self.source.split_change_visible_indices()
    }

    pub(in crate::view) fn scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        self.source.split_scrollbar_markers()
    }

    pub(in crate::view) fn modify_pair_texts(&self, row_ix: usize) -> Option<(&str, &str)> {
        self.source.split_modify_pair_texts(row_ix)
    }

    #[cfg(test)]
    fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedFileDiffRows {
    type RowRef = FileDiffRow;
    type SliceIter<'a>
        = std::vec::IntoIter<FileDiffRow>
    where
        Self: 'a;

    fn len_hint(&self) -> usize {
        self.source.split_len()
    }

    fn row(&self, ix: usize) -> Option<Self::RowRef> {
        self.row_at(ix)
    }

    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_> {
        if start >= end || start >= self.source.split_len() {
            return Vec::new().into_iter();
        }
        let end = end.min(self.source.split_len());
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        for row_ix in start..end {
            let Some(row) = self.row_at(row_ix) else {
                break;
            };
            rows.push(row);
        }
        rows.into_iter()
    }
}

#[derive(Debug)]
pub(in crate::view) struct PagedFileDiffInlineRows {
    source: Arc<StreamedFileDiffSource>,
    page_size: usize,
    pages: std::sync::Mutex<HashMap<usize, Arc<[AnnotatedDiffLine]>>>,
}

impl PagedFileDiffInlineRows {
    fn new(source: Arc<StreamedFileDiffSource>, page_size: usize) -> Self {
        Self {
            source,
            page_size: page_size.max(1),
            pages: std::sync::Mutex::new(HashMap::default()),
        }
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.source.inline_len()).then(|| {
            let end = start
                .saturating_add(self.page_size)
                .min(self.source.inline_len());
            (start, end)
        })
    }

    fn build_page(&self, page_ix: usize) -> Option<Arc<[AnnotatedDiffLine]>> {
        let (start, end) = self.page_bounds(page_ix)?;
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        for inline_ix in start..end {
            rows.push(self.source.inline_row(inline_ix)?);
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
            insert_streamed_cache_entry(
                &mut pages,
                page_ix,
                Arc::clone(&page),
                FILE_DIFF_MAX_CACHED_PAGES,
            );
            return Some(Arc::clone(pages.get(&page_ix).unwrap_or(&page)));
        }
        Some(page)
    }

    fn row_at(&self, inline_ix: usize) -> Option<AnnotatedDiffLine> {
        let page_ix = inline_ix / self.page_size;
        let page_row_ix = inline_ix % self.page_size;
        let page = self.load_page(page_ix)?;
        page.get(page_row_ix).cloned()
    }

    pub(in crate::view) fn change_visible_indices(&self) -> Vec<usize> {
        self.source.inline_change_visible_indices()
    }

    pub(in crate::view) fn scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        self.source.inline_scrollbar_markers()
    }

    pub(in crate::view) fn modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(&str, &str, gitcomet_core::domain::DiffLineKind)> {
        self.source.inline_modify_pair_texts(inline_ix)
    }

    pub(in crate::view) fn build_full_text(&self) -> SharedString {
        self.source.build_inline_text()
    }

    #[cfg(test)]
    fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedFileDiffInlineRows {
    type RowRef = AnnotatedDiffLine;
    type SliceIter<'a>
        = std::vec::IntoIter<AnnotatedDiffLine>
    where
        Self: 'a;

    fn len_hint(&self) -> usize {
        self.source.inline_len()
    }

    fn row(&self, ix: usize) -> Option<Self::RowRef> {
        self.row_at(ix)
    }

    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_> {
        if start >= end || start >= self.source.inline_len() {
            return Vec::new().into_iter();
        }
        let end = end.min(self.source.inline_len());
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        for row_ix in start..end {
            let Some(row) = self.row_at(row_ix) else {
                break;
            };
            rows.push(row);
        }
        rows.into_iter()
    }
}

fn build_single_markdown_preview_document(
    source: &str,
) -> Result<Arc<markdown_preview::MarkdownPreviewDocument>, String> {
    if source.len() > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
        return Err(markdown_preview::single_preview_unavailable_reason(source.len()).to_string());
    }

    markdown_preview::parse_markdown(source)
        .map(Arc::new)
        .ok_or_else(|| {
            markdown_preview::single_preview_unavailable_reason(source.len()).to_string()
        })
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

#[derive(Debug)]
struct FileDiffCacheRebuild {
    file_path: Option<std::path::PathBuf>,
    language: Option<rows::DiffSyntaxLanguage>,
    row_provider: Arc<PagedFileDiffRows>,
    inline_row_provider: Arc<PagedFileDiffInlineRows>,
    old_text: SharedString,
    old_line_starts: Arc<[usize]>,
    new_text: SharedString,
    new_line_starts: Arc<[usize]>,
    inline_text: SharedString,
    #[cfg(test)]
    rows: Vec<FileDiffRow>,
    #[cfg(test)]
    inline_rows: Vec<AnnotatedDiffLine>,
}

fn build_file_diff_cache_rebuild(
    file: &gitcomet_core::domain::FileDiffText,
    workdir: &std::path::Path,
) -> FileDiffCacheRebuild {
    let (old_text, old_line_starts) = build_file_diff_document_source(file.old.as_deref());
    let (new_text, new_line_starts) = build_file_diff_document_source(file.new.as_deref());
    let plan = Arc::new(gitcomet_core::file_diff::side_by_side_plan(
        old_text.as_ref(),
        new_text.as_ref(),
    ));
    let source = Arc::new(StreamedFileDiffSource::new(
        Arc::clone(&plan),
        old_text.clone(),
        Arc::clone(&old_line_starts),
        new_text.clone(),
        Arc::clone(&new_line_starts),
    ));
    let row_provider = Arc::new(PagedFileDiffRows::new(
        Arc::clone(&source),
        FILE_DIFF_PAGE_SIZE,
    ));
    let inline_row_provider = Arc::new(PagedFileDiffInlineRows::new(
        Arc::clone(&source),
        FILE_DIFF_PAGE_SIZE,
    ));

    let file_path = Some(if file.path.is_absolute() {
        file.path.clone()
    } else {
        workdir.join(&file.path)
    });
    let language = file_path
        .as_ref()
        .and_then(rows::diff_syntax_language_for_path);
    let inline_text = SharedString::default();

    #[cfg(test)]
    let rows = row_provider
        .slice(0, row_provider.len_hint())
        .collect::<Vec<_>>();
    #[cfg(test)]
    let inline_rows = inline_row_provider
        .slice(0, inline_row_provider.len_hint())
        .collect::<Vec<_>>();

    FileDiffCacheRebuild {
        file_path,
        language,
        row_provider,
        inline_row_provider,
        old_text,
        old_line_starts,
        new_text,
        new_line_starts,
        inline_text,
        #[cfg(test)]
        rows,
        #[cfg(test)]
        inline_rows,
    }
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

fn diff_syntax_edit_from_text_change(old: &str, new: &str) -> Option<rows::DiffSyntaxEdit> {
    if old == new {
        return None;
    }

    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    let mut prefix = 0usize;
    let max_prefix = old_bytes.len().min(new_bytes.len());
    while prefix < max_prefix && old_bytes[prefix] == new_bytes[prefix] {
        prefix += 1;
    }

    let mut old_suffix_start = old_bytes.len();
    let mut new_suffix_start = new_bytes.len();
    while old_suffix_start > prefix
        && new_suffix_start > prefix
        && old_bytes[old_suffix_start - 1] == new_bytes[new_suffix_start - 1]
    {
        old_suffix_start -= 1;
        new_suffix_start -= 1;
    }

    Some(rows::DiffSyntaxEdit {
        old_range: prefix..old_suffix_start,
        new_range: prefix..new_suffix_start,
    })
}

impl MainPaneView {
    pub(in crate::view) fn file_diff_split_row_len(&self) -> usize {
        self.file_diff_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.file_diff_cache_rows.len())
    }

    pub(in crate::view) fn file_diff_split_row(&self, row_ix: usize) -> Option<FileDiffRow> {
        if let Some(provider) = self.file_diff_row_provider.as_ref() {
            provider.row(row_ix)
        } else {
            self.file_diff_cache_rows.get(row_ix).cloned()
        }
    }

    pub(in crate::view) fn file_diff_inline_row_len(&self) -> usize {
        self.file_diff_inline_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.file_diff_inline_cache.len())
    }

    pub(in crate::view) fn file_diff_inline_row(
        &self,
        inline_ix: usize,
    ) -> Option<AnnotatedDiffLine> {
        if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
            provider.row(inline_ix)
        } else {
            self.file_diff_inline_cache.get(inline_ix).cloned()
        }
    }

    pub(in crate::view) fn file_diff_split_modify_pair_texts(
        &self,
        row_ix: usize,
    ) -> Option<(&str, &str)> {
        self.file_diff_row_provider
            .as_ref()
            .and_then(|provider| provider.modify_pair_texts(row_ix))
    }

    pub(in crate::view) fn file_diff_inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(&str, &str, gitcomet_core::domain::DiffLineKind)> {
        self.file_diff_inline_row_provider
            .as_ref()
            .and_then(|provider| provider.modify_pair_texts(inline_ix))
    }

    pub(in crate::view) fn ensure_file_diff_inline_text_materialized(&mut self) {
        if !self.file_diff_inline_text.is_empty() || self.file_diff_inline_row_len() == 0 {
            return;
        }
        if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
            self.file_diff_inline_text = provider.build_full_text();
        } else {
            self.file_diff_inline_text = build_inline_text(self.file_diff_inline_cache.as_slice());
        }
    }

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

    fn rekey_prepared_syntax_document(
        &mut self,
        old_key: PreparedSyntaxDocumentKey,
        new_key: PreparedSyntaxDocumentKey,
    ) {
        if old_key == new_key {
            return;
        }
        let Some(document) = self.prepared_syntax_documents.remove(&old_key) else {
            return;
        };
        self.prepared_syntax_documents
            .entry(new_key)
            .or_insert(document);
    }

    fn rekey_file_diff_prepared_syntax_documents_for_rev(&mut self, new_rev: u64) {
        let Some(repo_id) = self.file_diff_cache_repo_id else {
            return;
        };
        let Some(path) = self.file_diff_cache_path.clone() else {
            return;
        };
        let old_rev = self.file_diff_cache_rev;
        if old_rev == new_rev {
            return;
        }

        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            let old_key = prepared_syntax_document_key(repo_id, old_rev, &path, view_mode);
            let new_key = prepared_syntax_document_key(repo_id, new_rev, &path, view_mode);
            self.rekey_prepared_syntax_document(old_key, new_key);
        }
    }

    pub(super) fn full_document_syntax_budget(&self) -> rows::DiffSyntaxBudget {
        #[cfg(test)]
        if let Some(budget) = self.diff_syntax_budget_override {
            return budget;
        }

        rows::DiffSyntaxBudget::default()
    }

    #[cfg(test)]
    pub(in crate::view) fn set_full_document_syntax_budget_override_for_tests(
        &mut self,
        budget: rows::DiffSyntaxBudget,
    ) {
        self.diff_syntax_budget_override = Some(budget);
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
    /// Instead of parsing the synthetic mixed inline stream, project each row into
    /// the correct real old/new document using its 1-based diff line numbers.
    pub(in crate::view) fn file_diff_inline_projected_syntax(
        &self,
        line: &AnnotatedDiffLine,
    ) -> rows::PreparedDiffSyntaxLine {
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            line,
        )
    }

    pub(in crate::view) fn file_diff_split_prepared_syntax_document(
        &self,
        region: DiffTextRegion,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let view_mode = match region {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                PreparedSyntaxViewMode::FileDiffSplitRight
            }
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

    pub(in super::super::super) fn ensure_single_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(path) = self.worktree_preview_path.clone() else {
            return;
        };
        let source_rev = self.worktree_preview_content_rev;
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }

        let cache_matches = self.worktree_markdown_preview_path.as_ref() == Some(&path)
            && self.worktree_markdown_preview_source_rev == source_rev;
        if cache_matches {
            match &self.worktree_markdown_preview {
                Loadable::Ready(_) | Loadable::Error(_) => return,
                Loadable::Loading if self.worktree_markdown_preview_inflight.is_some() => return,
                _ => {}
            }
        }

        self.worktree_markdown_preview_path = Some(path.clone());
        self.worktree_markdown_preview_source_rev = source_rev;

        let source_text = self.worktree_preview_text.clone();
        if source_text.len() > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
            self.worktree_markdown_preview = Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source_text.len()).to_string(),
            );
            self.worktree_markdown_preview_inflight = None;
            return;
        }

        self.worktree_markdown_preview = Loadable::Loading;
        self.worktree_markdown_preview_seq = self.worktree_markdown_preview_seq.wrapping_add(1);
        let seq = self.worktree_markdown_preview_seq;
        self.worktree_markdown_preview_inflight = Some(seq);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let result = smol::unblock(move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    build_single_markdown_preview_document(source_text.as_ref())
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.worktree_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.worktree_preview_path.as_ref() != Some(&path)
                        || this.worktree_preview_content_rev != source_rev
                    {
                        return;
                    }

                    this.worktree_markdown_preview_inflight = None;
                    match result {
                        Ok(document) => this.worktree_markdown_preview = Loadable::Ready(document),
                        Err(error) => this.worktree_markdown_preview = Loadable::Error(error),
                    }
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn set_worktree_preview_ready_source(
        &mut self,
        path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_count = indexed_line_count(source_text.as_ref(), line_starts.as_ref());
        let source_changed = self.worktree_preview_path.as_ref() != Some(&path)
            || self.worktree_preview_line_count() != Some(line_count)
            || self.worktree_preview_text.len() != source_text.len()
            || self.worktree_preview_text.as_ref() != source_text.as_ref();
        let cache_binding_changed =
            self.worktree_preview_segments_cache_path.as_ref() != Some(&path);

        self.worktree_preview_path = Some(path.clone());
        self.worktree_preview = Loadable::Ready(line_count);
        self.worktree_preview_text = source_text;
        self.worktree_preview_line_starts = line_starts;
        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
        self.worktree_preview_segments_cache_path = Some(path);
        if source_changed || cache_binding_changed {
            self.worktree_preview_segments_cache.clear();
        }

        if source_changed {
            self.worktree_preview_content_rev = self.worktree_preview_content_rev.wrapping_add(1);
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
            self.worktree_markdown_preview_path = None;
            self.worktree_markdown_preview_source_rev = 0;
            self.worktree_markdown_preview = Loadable::NotLoaded;
            self.worktree_markdown_preview_inflight = None;
        }

        self.refresh_worktree_preview_syntax_document(cx);
    }

    pub(in crate::view) fn set_worktree_preview_ready_rows(
        &mut self,
        path: std::path::PathBuf,
        lines: &[String],
        source_len: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let (source_text, line_starts) =
            preview_source_text_and_line_starts_from_lines(lines, source_len);
        self.set_worktree_preview_ready_source(path, source_text, line_starts, cx);
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
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }
        let source_text = self.worktree_preview_text.clone();
        let line_starts = Arc::clone(&self.worktree_preview_line_starts);

        if self.prepared_syntax_document(&key).is_some() {
            return;
        }
        let reparse_seed = self.prepared_syntax_reparse_seed_document(&key);
        let background_reparse_seed: Option<rows::PreparedDiffSyntaxReparseSeed> =
            reparse_seed.and_then(rows::prepared_diff_syntax_reparse_seed);

        let budget = self.full_document_syntax_budget();
        match rows::prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            FULL_DOCUMENT_SYNTAX_MODE,
            source_text.clone(),
            Arc::clone(&line_starts),
            budget,
            reparse_seed,
            None,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => {
                self.insert_prepared_syntax_document(key, document);
            }
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        let parsed_document = smol::unblock(move || {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                source_text,
                                line_starts,
                                background_reparse_seed,
                                None,
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
                if let Some(key) = key.as_ref() {
                    self.insert_prepared_syntax_document(key.clone(), document);
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
        if let (Some(key), Some(document)) = (left_key.as_ref(), left_doc) {
            applied.split_left = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        if let (Some(key), Some(document)) = (right_key.as_ref(), right_doc) {
            applied.split_right = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        applied
    }

    fn refresh_file_diff_syntax_documents(
        &mut self,
        cx: &mut gpui::Context<Self>,
        split_left_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_right_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_left_edit_hint: Option<rows::DiffSyntaxEdit>,
        split_right_edit_hint: Option<rows::DiffSyntaxEdit>,
    ) {
        let Some(language) = self.file_diff_cache_language else {
            return;
        };

        // Split and inline syntax both project from the real old/new documents.
        // Only those real side documents are parsed here; inline rows later map
        // through old_line/new_line instead of parsing any synthetic diff stream.
        let split_left_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitLeft);
        let split_right_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight);
        let split_left_reparse_seed = split_left_reparse_seed_override.or_else(|| {
            split_left_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });
        let split_right_reparse_seed = split_right_reparse_seed_override.or_else(|| {
            split_right_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });

        let needs_split_left_prepare = split_left_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        let needs_split_right_prepare = split_right_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        if !needs_split_left_prepare && !needs_split_right_prepare {
            return;
        }

        let budget = self.full_document_syntax_budget();

        let split_left_attempt = needs_split_left_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
                budget,
                split_left_reparse_seed,
                split_left_edit_hint.clone(),
            )
        });
        let split_right_attempt = needs_split_right_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
                budget,
                split_right_reparse_seed,
                split_right_edit_hint.clone(),
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

        let split_left_source = needs_split_left_async.then(|| {
            (
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
            )
        });
        let split_left_background_reparse_seed = split_left_reparse_seed
            .filter(|_| needs_split_left_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_left_edit_hint = split_left_edit_hint.filter(|_| needs_split_left_async);
        let split_right_source = needs_split_right_async.then(|| {
            (
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
            )
        });
        let split_right_background_reparse_seed = split_right_reparse_seed
            .filter(|_| needs_split_right_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_right_edit_hint = split_right_edit_hint.filter(|_| needs_split_right_async);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let parsed_documents =
                    smol::unblock(move || FileDiffBackgroundPreparedSyntaxDocuments {
                        split_left: split_left_source.and_then(|(text, line_starts)| {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                text,
                                line_starts,
                                split_left_background_reparse_seed,
                                split_left_edit_hint,
                            )
                        }),
                        split_right: split_right_source.and_then(|(text, line_starts)| {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                text,
                                line_starts,
                                split_right_background_reparse_seed,
                                split_right_edit_hint,
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

    /// Resets file-diff data fields (syntax, rows, text, highlights) without
    /// touching the identity fields (repo_id, target, rev).
    fn reset_file_diff_cache_data(&mut self) {
        self.file_diff_cache_content_signature = None;
        self.file_diff_cache_inflight = None;
        self.file_diff_syntax_generation = self.file_diff_syntax_generation.wrapping_add(1);
        self.file_diff_style_cache_epochs.bump_both();
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows.clear();
        self.file_diff_row_provider = None;
        self.file_diff_old_text = SharedString::default();
        self.file_diff_old_line_starts = Arc::default();
        self.file_diff_new_text = SharedString::default();
        self.file_diff_new_line_starts = Arc::default();
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_row_provider = None;
        self.file_diff_inline_text = SharedString::default();
        self.file_diff_inline_word_highlights.clear();
        self.file_diff_split_word_highlights_old.clear();
        self.file_diff_split_word_highlights_new.clear();
    }

    pub(in super::super::super) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
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
            self.file_diff_cache_repo_id = None;
            self.file_diff_cache_target = None;
            self.file_diff_cache_rev = 0;
            self.reset_file_diff_cache_data();
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_diff_text_signature(file.as_ref()));
        let same_repo_and_target = self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_target == diff_target;
        let previous_split_left_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft))
            .flatten();
        let previous_split_right_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight))
            .flatten();
        let previous_old_text = same_repo_and_target.then(|| self.file_diff_old_text.clone());
        let previous_new_text = same_repo_and_target.then(|| self.file_diff_new_text.clone());

        if same_repo_and_target && self.file_diff_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_diff_cache_content_signature == Some(signature)
        {
            // Store-side refreshes can bump diff_file_rev with identical file payloads.
            // Keep the row cache and prepared syntax documents alive across rev-only refreshes.
            // If syntax was still missing, kick the syntax refresh path for the new active key.
            if self.file_diff_cache_inflight.is_none() {
                self.rekey_file_diff_prepared_syntax_documents_for_rev(diff_file_rev);
                self.file_diff_cache_rev = diff_file_rev;
                self.refresh_file_diff_syntax_documents(cx, None, None, None, None);
            }
            return;
        }

        self.file_diff_cache_repo_id = Some(repo_id);
        self.file_diff_cache_rev = diff_file_rev;
        self.file_diff_cache_target = diff_target;
        self.reset_file_diff_cache_data();

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
                let rebuild =
                    smol::unblock(move || build_file_diff_cache_rebuild(file.as_ref(), &workdir))
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
                    this.file_diff_row_provider = Some(rebuild.row_provider);
                    this.file_diff_old_text = rebuild.old_text;
                    this.file_diff_old_line_starts = rebuild.old_line_starts;
                    this.file_diff_new_text = rebuild.new_text;
                    this.file_diff_new_line_starts = rebuild.new_line_starts;
                    this.file_diff_inline_row_provider = Some(rebuild.inline_row_provider);
                    this.file_diff_inline_text = rebuild.inline_text;
                    this.file_diff_cache_content_signature = Some(content_signature);
                    #[cfg(test)]
                    {
                        this.file_diff_cache_rows = rebuild.rows;
                        this.file_diff_inline_cache = rebuild.inline_rows;
                    }
                    let split_left_edit_hint = previous_old_text.as_ref().and_then(|previous| {
                        diff_syntax_edit_from_text_change(
                            previous.as_ref(),
                            this.file_diff_old_text.as_ref(),
                        )
                    });
                    let split_right_edit_hint = previous_new_text.as_ref().and_then(|previous| {
                        diff_syntax_edit_from_text_change(
                            previous.as_ref(),
                            this.file_diff_new_text.as_ref(),
                        )
                    });
                    this.refresh_file_diff_syntax_documents(
                        cx,
                        previous_split_left_reparse_seed,
                        previous_split_right_reparse_seed,
                        split_left_edit_hint,
                        split_right_edit_hint,
                    );

                    // Reset the segment cache to avoid mixing patch/file indices.
                    this.clear_diff_text_style_caches();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in super::super::super) fn ensure_file_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let clear_cache = |this: &mut Self| {
            this.file_markdown_preview_cache_repo_id = None;
            this.file_markdown_preview_cache_target = None;
            this.file_markdown_preview_cache_rev = 0;
            this.file_markdown_preview_cache_content_signature = None;
            this.file_markdown_preview = Loadable::NotLoaded;
            this.file_markdown_preview_inflight = None;
        };

        let Some((repo_id, diff_file_rev, diff_target, file)) = (|| {
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
        let same_repo_and_target = self.file_markdown_preview_cache_repo_id == Some(repo_id)
            && self.file_markdown_preview_cache_target == diff_target;

        if same_repo_and_target && self.file_markdown_preview_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_markdown_preview_cache_content_signature == Some(signature)
        {
            if self.file_markdown_preview_inflight.is_none() {
                self.file_markdown_preview_cache_rev = diff_file_rev;
            }
            return;
        }

        self.file_markdown_preview_cache_repo_id = Some(repo_id);
        self.file_markdown_preview_cache_rev = diff_file_rev;
        self.file_markdown_preview_cache_content_signature = None;
        self.file_markdown_preview_cache_target = diff_target;
        self.file_markdown_preview = Loadable::NotLoaded;
        self.file_markdown_preview_inflight = None;

        let Some(file) = file else {
            return;
        };
        // `file` was `Some` when `file_content_signature` was computed, so unwrap is safe.
        let content_signature = file_content_signature.unwrap();
        let old_source = file.old.clone().unwrap_or_default();
        let new_source = file.new.clone().unwrap_or_default();

        let combined_len = old_source.len() + new_source.len();
        if combined_len > markdown_preview::MAX_DIFF_PREVIEW_SOURCE_BYTES {
            self.file_markdown_preview = Loadable::Error(
                markdown_preview::diff_preview_unavailable_reason(combined_len).to_string(),
            );
            self.file_markdown_preview_cache_content_signature = Some(content_signature);
            return;
        }

        self.file_markdown_preview = Loadable::Loading;
        self.file_markdown_preview_seq = self.file_markdown_preview_seq.wrapping_add(1);
        let seq = self.file_markdown_preview_seq;
        self.file_markdown_preview_inflight = Some(seq);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let result = smol::unblock(move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    markdown_preview::build_markdown_diff_preview(
                        old_source.as_str(),
                        new_source.as_str(),
                    )
                    .map(Arc::new)
                    .ok_or_else(|| {
                        markdown_preview::diff_preview_unavailable_reason(
                            old_source.len() + new_source.len(),
                        )
                        .to_string()
                    })
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.file_markdown_preview_cache_repo_id != Some(repo_id)
                        || this.file_markdown_preview_cache_rev != diff_file_rev
                        || this.file_markdown_preview_cache_target != diff_target_for_task
                    {
                        return;
                    }

                    this.file_markdown_preview_inflight = None;
                    this.file_markdown_preview_cache_content_signature = Some(content_signature);
                    match result {
                        Ok(preview) => this.file_markdown_preview = Loadable::Ready(preview),
                        Err(error) => this.file_markdown_preview = Loadable::Error(error),
                    }
                    cx.notify();
                });
            },
        )
        .detach();
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

            let format = image_format_for_path(&file.path);
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

        if let Some(preview) = build_new_file_preview_from_diff(
            diff.lines.as_slice(),
            &workdir,
            self.diff_cache_target.as_ref(),
        ) {
            self.diff_preview_is_new_file = true;
            self.set_worktree_preview_ready_rows(
                preview.abs_path,
                preview.lines.as_slice(),
                preview.source_len,
                cx,
            );
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
                if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
                    return provider.scrollbar_markers();
                }
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
                if let Some(provider) = self.file_diff_row_provider.as_ref() {
                    return provider.scrollbar_markers();
                }
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
                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                DiffViewMode::Split => self.file_diff_split_row_len(),
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

    fn streamed_file_diff_source_for_test(old: &str, new: &str) -> Arc<StreamedFileDiffSource> {
        let (old_text, old_line_starts) = build_file_diff_document_source(Some(old));
        let (new_text, new_line_starts) = build_file_diff_document_source(Some(new));
        let plan = Arc::new(gitcomet_core::file_diff::side_by_side_plan(old, new));
        Arc::new(StreamedFileDiffSource::new(
            plan,
            old_text,
            old_line_starts,
            new_text,
            new_line_starts,
        ))
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

    fn prepare_test_document(
        language: rows::DiffSyntaxLanguage,
        text: &str,
    ) -> rows::PreparedDiffSyntaxDocument {
        let text: SharedString = text.to_owned().into();
        let line_starts = Arc::from(build_line_starts(text.as_ref()));
        match rows::prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            rows::DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&line_starts),
            rows::DiffSyntaxBudget {
                foreground_parse: std::time::Duration::from_millis(50),
            },
            None,
            None,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                rows::inject_background_prepared_diff_syntax_document(
                    rows::prepare_diff_syntax_document_in_background_text(
                        language,
                        rows::DiffSyntaxMode::Auto,
                        text,
                        line_starts,
                    )
                    .expect("background parse should be available for supported test documents"),
                )
            }
            rows::PrepareDiffSyntaxDocumentResult::Unsupported => {
                panic!("test document should support prepared syntax parsing")
            }
        }
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
    fn build_file_diff_cache_rebuild_preserves_real_document_sources() {
        let file = gitcomet_core::domain::FileDiffText {
            path: PathBuf::from("src/demo.rs"),
            old: Some("alpha\nbeta\n".to_string()),
            new: Some("gamma\ndelta".to_string()),
        };

        let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"));

        assert_eq!(
            rebuild.file_path,
            Some(PathBuf::from("/tmp/repo/src/demo.rs"))
        );
        assert_eq!(rebuild.language, Some(rows::DiffSyntaxLanguage::Rust));
        assert_eq!(rebuild.old_text.as_ref(), "alpha\nbeta\n");
        assert_eq!(rebuild.old_line_starts.as_ref(), &[0, 6, 11]);
        assert_eq!(rebuild.new_text.as_ref(), "gamma\ndelta");
        assert_eq!(rebuild.new_line_starts.as_ref(), &[0, 6]);
    }

    #[test]
    fn build_file_diff_cache_rebuild_inline_rows_keep_file_line_numbers() {
        use gitcomet_core::domain::DiffLineKind;

        let file = gitcomet_core::domain::FileDiffText {
            path: PathBuf::from("src/demo.rs"),
            old: Some("struct Old;\nfn keep() {}\n".to_string()),
            new: Some("fn keep() {}\nlet added = 42;\n".to_string()),
        };

        let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"));
        let language = rebuild
            .language
            .expect("rust path should resolve a syntax language");
        let old_document = prepare_test_document(language, rebuild.old_text.as_ref());
        let new_document = prepare_test_document(language, rebuild.new_text.as_ref());

        let remove_row = rebuild
            .inline_rows
            .iter()
            .find(|row| row.kind == DiffLineKind::Remove)
            .expect("diff should contain a remove row");
        assert_eq!(remove_row.old_line, Some(1));
        assert_eq!(
            rows::prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                remove_row,
            ),
            rows::PreparedDiffSyntaxLine {
                document: Some(old_document),
                line_ix: 0,
            }
        );

        let context_row = rebuild
            .inline_rows
            .iter()
            .find(|row| row.kind == DiffLineKind::Context)
            .expect("diff should contain a context row");
        assert_eq!(context_row.old_line, Some(2));
        assert_eq!(context_row.new_line, Some(1));
        assert_eq!(
            rows::prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                context_row,
            ),
            rows::PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 0,
            }
        );

        let add_row = rebuild
            .inline_rows
            .iter()
            .find(|row| row.kind == DiffLineKind::Add)
            .expect("diff should contain an add row");
        assert_eq!(add_row.new_line, Some(2));
        assert_eq!(
            rows::prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                add_row,
            ),
            rows::PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 1,
            }
        );
    }

    #[test]
    fn preview_source_text_from_lines_preserves_missing_trailing_newline() {
        let lines = vec![
            "fn main() {".to_string(),
            "    42".to_string(),
            "}".to_string(),
        ];
        let source_len = "fn main() {\n    42\n}".len();

        let source = preview_source_text_from_lines(&lines, source_len);
        let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

        assert_eq!(source.as_ref(), "fn main() {\n    42\n}");
        assert_eq!(line_starts.as_ref(), &[0, 12, 19]);
    }

    #[test]
    fn preview_source_text_from_lines_restores_trailing_newline() {
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        let source_len = "alpha\nbeta\n".len();

        let source = preview_source_text_from_lines(&lines, source_len);
        let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

        assert_eq!(source.as_ref(), "alpha\nbeta\n");
        assert_eq!(line_starts.as_ref(), &[0, 6, 11]);
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
    fn build_single_markdown_preview_document_reports_row_limit() {
        let preview_lines =
            vec!["---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS + 1)];
        let source = preview_source_text_from_lines(
            &preview_lines,
            preview_lines_source_len(&preview_lines),
        );
        assert!(source.len() < crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);

        let error = build_single_markdown_preview_document(source.as_ref())
            .expect_err("row-limit markdown preview should return an error");
        assert!(
            error.contains("row limit"),
            "row-limit markdown preview should mention the rendered row limit: {error}"
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
    fn build_single_markdown_preview_document_respects_exact_source_length() {
        let mut source = "x".repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
        source.push('\n');
        assert_eq!(
            source.len(),
            crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1
        );

        let error = build_single_markdown_preview_document(&source)
            .expect_err("exact source length over the cap should return an error");
        assert!(
            error.contains("1 MiB"),
            "exact-size markdown preview should mention the size limit: {error}"
        );
    }

    #[test]
    fn build_single_markdown_preview_document_from_deleted_markdown_table_preview_parses() {
        let diff = vec![
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: Arc::from("diff --git a/docs/table.md b/docs/table.md"),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: Arc::from("deleted file mode 100644"),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| **Header Bold** | B |"),
                old_line: Some(1),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| --- | --- |"),
                old_line: Some(2),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| [link](https://example.com) | plain |"),
                old_line: Some(3),
                new_line: None,
            },
        ];
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("docs/table.md"),
            area: DiffArea::Unstaged,
        };

        let preview = crate::view::diff_preview::build_deleted_file_preview_from_diff(
            &diff,
            &workdir,
            Some(&target),
        )
        .expect("deleted markdown preview should reconstruct from diff");
        let source = preview_source_text_from_lines(&preview.lines, preview.source_len);
        let document = build_single_markdown_preview_document(source.as_ref())
            .expect("deleted markdown table preview should parse");
        let table_rows = document
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row.kind,
                    crate::view::markdown_preview::MarkdownPreviewRowKind::TableRow { .. }
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(table_rows.len(), 2);
        assert_eq!(table_rows[0].text.as_ref(), "Header Bold | B");
        assert_eq!(table_rows[1].text.as_ref(), "link        | plain");
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
            image_format_for_path(Path::new("x.PNG")),
            Some(gpui::ImageFormat::Png)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.JpEg")),
            Some(gpui::ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.GiF")),
            Some(gpui::ImageFormat::Gif)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.webp")),
            Some(gpui::ImageFormat::Webp)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.BMP")),
            Some(gpui::ImageFormat::Bmp)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.TiFf")),
            Some(gpui::ImageFormat::Tiff)
        );
    }

    #[test]
    fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
        assert_eq!(image_format_for_path(Path::new("x.heic")), None);
        assert_eq!(image_format_for_path(Path::new("x.ico")), None);
        assert_eq!(image_format_for_path(Path::new("x")), None);
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
    fn paged_file_diff_rows_load_pages_on_demand() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let provider = PagedFileDiffRows::new(Arc::clone(&source), 1);

        assert_eq!(provider.cached_page_count(), 0);
        let row = provider.row_at(1).expect("middle row should exist");
        assert_eq!(row.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
        assert_eq!(row.old.as_deref(), Some("beta"));
        assert_eq!(row.new.as_deref(), Some("beta changed"));
        assert_eq!(provider.cached_page_count(), 1);

        let first = provider.row_at(0).expect("first row should exist");
        assert_eq!(
            first.kind,
            gitcomet_core::file_diff::FileDiffRowKind::Context
        );
        assert_eq!(provider.cached_page_count(), 2);
    }

    #[test]
    fn paged_file_diff_rows_bound_cached_pages() {
        let line_count = FILE_DIFF_MAX_CACHED_PAGES + 12;
        let old = (0..line_count)
            .map(|ix| format!("line-{ix:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = old.clone();
        let source = streamed_file_diff_source_for_test(&old, &new);
        let provider = PagedFileDiffRows::new(Arc::clone(&source), 1);

        for row_ix in 0..line_count {
            assert!(
                provider.row_at(row_ix).is_some(),
                "row {row_ix} should exist"
            );
        }

        assert!(
            provider.cached_page_count() <= FILE_DIFF_MAX_CACHED_PAGES,
            "cached split pages should stay bounded"
        );
    }

    #[test]
    fn paged_file_diff_inline_rows_load_pages_on_demand() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let provider = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

        assert_eq!(provider.cached_page_count(), 0);
        let remove = provider.row_at(1).expect("modify remove row should exist");
        assert_eq!(remove.kind, gitcomet_core::domain::DiffLineKind::Remove);
        assert_eq!(remove.text.as_ref(), "-beta");
        assert_eq!(provider.cached_page_count(), 1);

        let add = provider.row_at(2).expect("modify add row should exist");
        assert_eq!(add.kind, gitcomet_core::domain::DiffLineKind::Add);
        assert_eq!(add.text.as_ref(), "+beta changed");
        assert_eq!(provider.cached_page_count(), 2);
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
    fn diff_syntax_edit_identical_texts_returns_none() {
        assert!(diff_syntax_edit_from_text_change("hello world", "hello world").is_none());
        assert!(diff_syntax_edit_from_text_change("", "").is_none());
    }

    #[test]
    fn diff_syntax_edit_completely_different_texts() {
        let edit = diff_syntax_edit_from_text_change("abc", "xyz").unwrap();
        assert_eq!(edit.old_range, 0..3);
        assert_eq!(edit.new_range, 0..3);
    }

    #[test]
    fn diff_syntax_edit_shared_prefix() {
        let edit = diff_syntax_edit_from_text_change("hello world", "hello rust").unwrap();
        assert_eq!(edit.old_range, 6..11);
        assert_eq!(edit.new_range, 6..10);
    }

    #[test]
    fn diff_syntax_edit_shared_suffix() {
        let edit = diff_syntax_edit_from_text_change("old suffix", "new suffix").unwrap();
        assert_eq!(edit.old_range, 0..3);
        assert_eq!(edit.new_range, 0..3);
    }

    #[test]
    fn diff_syntax_edit_shared_prefix_and_suffix() {
        let edit = diff_syntax_edit_from_text_change("fn foo() {}", "fn bar() {}").unwrap();
        // "fn " is shared prefix (3 bytes), "() {}" is shared suffix (5 bytes)
        assert_eq!(edit.old_range, 3..6);
        assert_eq!(edit.new_range, 3..6);
    }

    #[test]
    fn diff_syntax_edit_insertion_at_beginning() {
        let edit = diff_syntax_edit_from_text_change("fn main() {}", "/* comment */\nfn main() {}")
            .unwrap();
        assert_eq!(edit.old_range, 0..0);
        assert_eq!(edit.new_range, 0..14);
    }

    #[test]
    fn diff_syntax_edit_insertion_at_end() {
        let edit =
            diff_syntax_edit_from_text_change("fn main() {}", "fn main() {}\n// end").unwrap();
        // "fn main() {}" is 12 bytes; insertion starts after byte 12
        assert_eq!(edit.old_range, 12..12);
        assert_eq!(edit.new_range, 12..19);
    }

    #[test]
    fn diff_syntax_edit_deletion() {
        let edit = diff_syntax_edit_from_text_change("fn foo() { body }", "fn foo() {}").unwrap();
        // shared prefix: "fn foo() {" (10 bytes), shared suffix: "}" (1 byte)
        assert_eq!(edit.old_range, 10..16);
        assert_eq!(edit.new_range, 10..10);
    }

    #[test]
    fn diff_syntax_edit_one_empty_string() {
        let edit = diff_syntax_edit_from_text_change("", "hello").unwrap();
        assert_eq!(edit.old_range, 0..0);
        assert_eq!(edit.new_range, 0..5);

        let edit = diff_syntax_edit_from_text_change("hello", "").unwrap();
        assert_eq!(edit.old_range, 0..5);
        assert_eq!(edit.new_range, 0..0);
    }

    #[test]
    fn diff_syntax_edit_multibyte_utf8() {
        // "café" is 5 bytes (é is 2 bytes), "caff" is 4 bytes
        let edit = diff_syntax_edit_from_text_change("café", "caff").unwrap();
        // shared prefix: "caf" (3 bytes), diverges at é vs f
        assert_eq!(edit.old_range, 3..5);
        assert_eq!(edit.new_range, 3..4);
    }
}
