use super::*;
use rustc_hash::FxHasher;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn build_inline_text(lines: &[AnnotatedDiffLine]) -> SharedString {
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

fn prefixed_inline_text(prefix: char, line: &str) -> Arc<str> {
    let mut text = String::with_capacity(line.len().saturating_add(1));
    text.push(prefix);
    text.push_str(line);
    text.into()
}

fn append_prefixed_inline_text(target: &mut String, prefix: char, line: &str) {
    target.push(prefix);
    target.push_str(line);
    target.push('\n');
}

pub(super) fn file_diff_text_signature(file: &gitcomet_core::domain::FileDiffText) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    file.path.hash(&mut hasher);
    file.old.hash(&mut hasher);
    file.new.hash(&mut hasher);
    hasher.finish()
}

fn build_file_diff_document_source(text: Option<&str>) -> (SharedString, Arc<[usize]>) {
    let text: SharedString = text.unwrap_or_default().to_owned().into();
    let line_starts = Arc::from(build_line_starts(text.as_ref()));
    (text, line_starts)
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StreamedFileDiffDebugCounters {
    split_page_cache_hits: u64,
    split_page_cache_misses: u64,
    inline_page_cache_hits: u64,
    inline_page_cache_misses: u64,
    split_rows_materialized: u64,
    inline_rows_materialized: u64,
    inline_full_text_materializations: u64,
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct AtomicStreamedFileDiffDebugCounters {
    split_page_cache_hits: AtomicU64,
    split_page_cache_misses: AtomicU64,
    inline_page_cache_hits: AtomicU64,
    inline_page_cache_misses: AtomicU64,
    split_rows_materialized: AtomicU64,
    inline_rows_materialized: AtomicU64,
    inline_full_text_materializations: AtomicU64,
}

#[cfg(debug_assertions)]
impl AtomicStreamedFileDiffDebugCounters {
    const fn new() -> Self {
        Self {
            split_page_cache_hits: AtomicU64::new(0),
            split_page_cache_misses: AtomicU64::new(0),
            inline_page_cache_hits: AtomicU64::new(0),
            inline_page_cache_misses: AtomicU64::new(0),
            split_rows_materialized: AtomicU64::new(0),
            inline_rows_materialized: AtomicU64::new(0),
            inline_full_text_materializations: AtomicU64::new(0),
        }
    }

    fn record_page_hit(&self, inline: bool) {
        let counter = if inline {
            &self.inline_page_cache_hits
        } else {
            &self.split_page_cache_hits
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_page_miss(&self, inline: bool) {
        let counter = if inline {
            &self.inline_page_cache_misses
        } else {
            &self.split_page_cache_misses
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_rows_materialized(&self, inline: bool, count: usize) {
        let counter = if inline {
            &self.inline_rows_materialized
        } else {
            &self.split_rows_materialized
        };
        counter.fetch_add(count.try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    fn record_inline_full_text_materialization(&self) {
        self.inline_full_text_materializations
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn snapshot(&self) -> StreamedFileDiffDebugCounters {
        StreamedFileDiffDebugCounters {
            split_page_cache_hits: self.split_page_cache_hits.load(Ordering::Relaxed),
            split_page_cache_misses: self.split_page_cache_misses.load(Ordering::Relaxed),
            inline_page_cache_hits: self.inline_page_cache_hits.load(Ordering::Relaxed),
            inline_page_cache_misses: self.inline_page_cache_misses.load(Ordering::Relaxed),
            split_rows_materialized: self.split_rows_materialized.load(Ordering::Relaxed),
            inline_rows_materialized: self.inline_rows_materialized.load(Ordering::Relaxed),
            inline_full_text_materializations: self
                .inline_full_text_materializations
                .load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.split_page_cache_hits.store(0, Ordering::Relaxed);
        self.split_page_cache_misses.store(0, Ordering::Relaxed);
        self.inline_page_cache_hits.store(0, Ordering::Relaxed);
        self.inline_page_cache_misses.store(0, Ordering::Relaxed);
        self.split_rows_materialized.store(0, Ordering::Relaxed);
        self.inline_rows_materialized.store(0, Ordering::Relaxed);
        self.inline_full_text_materializations
            .store(0, Ordering::Relaxed);
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
    #[cfg(debug_assertions)]
    debug_counters: Arc<AtomicStreamedFileDiffDebugCounters>,
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
            #[cfg(debug_assertions)]
            debug_counters: Arc::new(AtomicStreamedFileDiffDebugCounters::new()),
        }
    }

    #[inline]
    fn record_page_hit(&self, inline: bool) {
        #[cfg(debug_assertions)]
        {
            self.debug_counters.record_page_hit(inline);
        }
        #[cfg(not(debug_assertions))]
        let _ = inline;
    }

    #[inline]
    fn record_page_miss(&self, inline: bool) {
        #[cfg(debug_assertions)]
        {
            self.debug_counters.record_page_miss(inline);
        }
        #[cfg(not(debug_assertions))]
        let _ = inline;
    }

    #[inline]
    fn record_rows_materialized(&self, inline: bool, count: usize) {
        #[cfg(debug_assertions)]
        {
            self.debug_counters.record_rows_materialized(inline, count);
        }
        #[cfg(not(debug_assertions))]
        let _ = (inline, count);
    }

    #[inline]
    fn record_inline_full_text_materialization(&self) {
        #[cfg(debug_assertions)]
        {
            self.debug_counters
                .record_inline_full_text_materialization();
        }
    }

    #[cfg(test)]
    fn debug_counters_snapshot(&self) -> StreamedFileDiffDebugCounters {
        #[cfg(debug_assertions)]
        {
            self.debug_counters.snapshot()
        }
        #[cfg(not(debug_assertions))]
        {
            StreamedFileDiffDebugCounters::default()
        }
    }

    #[cfg(test)]
    fn reset_debug_counters(&self) {
        #[cfg(debug_assertions)]
        {
            self.debug_counters.reset();
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
                let text: Arc<str> = self.old_line_text(old_ix).into();
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                    old: Some(Arc::clone(&text)),
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
                    old: Some(self.old_line_text(old_ix).into()),
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
                    new: Some(self.new_line_text(new_ix).into()),
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
                    old: Some(self.old_line_text(old_ix).into()),
                    new: Some(self.new_line_text(new_ix).into()),
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
                    text: prefixed_inline_text(' ', self.old_line_text(old_ix)),
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                    text: prefixed_inline_text('-', self.old_line_text(old_ix)),
                    old_line: line_number(old_ix),
                    new_line: None,
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Add,
                    text: prefixed_inline_text('+', self.new_line_text(new_ix)),
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
                        text: prefixed_inline_text('-', self.old_line_text(old_ix)),
                        old_line: line_number(old_ix),
                        new_line: None,
                    })
                } else {
                    Some(AnnotatedDiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Add,
                        text: prefixed_inline_text('+', self.new_line_text(new_ix)),
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
        let mut text = String::with_capacity(
            self.old_text
                .len()
                .saturating_add(self.new_text.len())
                .saturating_add(self.inline_len().saturating_mul(2)),
        );

        for run in &self.plan.runs {
            match *run {
                gitcomet_core::file_diff::FileDiffPlanRun::Context { old_start, len, .. } => {
                    for offset in 0..len {
                        append_prefixed_inline_text(
                            &mut text,
                            ' ',
                            self.old_line_text(old_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                    for offset in 0..len {
                        append_prefixed_inline_text(
                            &mut text,
                            '-',
                            self.old_line_text(old_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                    for offset in 0..len {
                        append_prefixed_inline_text(
                            &mut text,
                            '+',
                            self.new_line_text(new_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                    old_start,
                    new_start,
                    len,
                } => {
                    for offset in 0..len {
                        append_prefixed_inline_text(
                            &mut text,
                            '-',
                            self.old_line_text(old_start.saturating_add(offset)),
                        );
                        append_prefixed_inline_text(
                            &mut text,
                            '+',
                            self.new_line_text(new_start.saturating_add(offset)),
                        );
                    }
                }
            }
        }
        SharedString::from(text)
    }
}

#[derive(Debug)]
pub(in crate::view) struct PagedFileDiffRows {
    source: Arc<StreamedFileDiffSource>,
    page_size: usize,
    pages: std::sync::Mutex<rows::LruCache<usize, Arc<[FileDiffRow]>>>,
}

impl PagedFileDiffRows {
    fn new(source: Arc<StreamedFileDiffSource>, page_size: usize) -> Self {
        Self {
            source,
            page_size: page_size.max(1),
            pages: std::sync::Mutex::new(rows::new_lru_cache(FILE_DIFF_MAX_CACHED_PAGES)),
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
        self.source.record_rows_materialized(false, rows.len());
        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[FileDiffRow]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            self.source.record_page_hit(false);
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
        self.source.record_page_miss(false);
        if let Ok(mut pages) = self.pages.lock() {
            pages.put(page_ix, Arc::clone(&page));
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

    #[cfg(test)]
    #[allow(dead_code)]
    fn page_cache_metrics(&self) -> rows::LruCacheMetrics {
        self.pages
            .lock()
            .map(|pages| pages.metrics())
            .unwrap_or_default()
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
    pages: std::sync::Mutex<rows::LruCache<usize, Arc<[AnnotatedDiffLine]>>>,
    full_text: std::sync::OnceLock<SharedString>,
}

impl PagedFileDiffInlineRows {
    fn new(source: Arc<StreamedFileDiffSource>, page_size: usize) -> Self {
        Self {
            source,
            page_size: page_size.max(1),
            pages: std::sync::Mutex::new(rows::new_lru_cache(FILE_DIFF_MAX_CACHED_PAGES)),
            full_text: std::sync::OnceLock::new(),
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
        self.source.record_rows_materialized(true, rows.len());
        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[AnnotatedDiffLine]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            self.source.record_page_hit(true);
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
        self.source.record_page_miss(true);
        if let Ok(mut pages) = self.pages.lock() {
            pages.put(page_ix, Arc::clone(&page));
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

    pub(super) fn build_full_text(&self) -> SharedString {
        self.full_text
            .get_or_init(|| {
                self.source.record_inline_full_text_materialization();
                self.source.build_inline_text()
            })
            .clone()
    }

    #[cfg(test)]
    fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn page_cache_metrics(&self) -> rows::LruCacheMetrics {
        self.pages
            .lock()
            .map(|pages| pages.metrics())
            .unwrap_or_default()
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

#[derive(Debug)]
pub(super) struct FileDiffCacheRebuild {
    pub(super) file_path: Option<std::path::PathBuf>,
    pub(super) language: Option<rows::DiffSyntaxLanguage>,
    pub(super) row_provider: Arc<PagedFileDiffRows>,
    pub(super) inline_row_provider: Arc<PagedFileDiffInlineRows>,
    pub(super) old_text: SharedString,
    pub(super) old_line_starts: Arc<[usize]>,
    pub(super) new_text: SharedString,
    pub(super) new_line_starts: Arc<[usize]>,
    pub(super) inline_text: SharedString,
    #[cfg(test)]
    pub(super) rows: Vec<FileDiffRow>,
    #[cfg(test)]
    pub(super) inline_rows: Vec<AnnotatedDiffLine>,
}

pub(super) fn build_file_diff_cache_rebuild(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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

    fn streamed_file_diff_debug_counters(
        source: &Arc<StreamedFileDiffSource>,
    ) -> StreamedFileDiffDebugCounters {
        source.debug_counters_snapshot()
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
    fn paged_file_diff_rows_reuse_text_arcs_from_cached_pages() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let provider = PagedFileDiffRows::new(Arc::clone(&source), 2);

        let context = provider.row_at(0).expect("first row should exist");
        let modify_a = provider.row_at(1).expect("middle row should exist");
        let modify_b = provider.row_at(1).expect("middle row should still exist");

        let context_old = context.old.as_ref().expect("context old text");
        let context_new = context.new.as_ref().expect("context new text");
        assert!(
            Arc::ptr_eq(context_old, context_new),
            "context rows should share one text allocation across both sides"
        );

        let modify_old_a = modify_a.old.as_ref().expect("modify old text");
        let modify_old_b = modify_b.old.as_ref().expect("modify old text");
        let modify_new_a = modify_a.new.as_ref().expect("modify new text");
        let modify_new_b = modify_b.new.as_ref().expect("modify new text");
        assert!(
            Arc::ptr_eq(modify_old_a, modify_old_b),
            "re-reading a cached row should clone the old-side arc instead of reallocating"
        );
        assert!(
            Arc::ptr_eq(modify_new_a, modify_new_b),
            "re-reading a cached row should clone the new-side arc instead of reallocating"
        );
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
    fn paged_file_diff_rows_expose_shared_lru_metrics() {
        let line_count = FILE_DIFF_MAX_CACHED_PAGES + 2;
        let old = (0..line_count)
            .map(|ix| format!("line-{ix:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let source = streamed_file_diff_source_for_test(&old, &old);
        let provider = PagedFileDiffRows::new(Arc::clone(&source), 1);

        assert_eq!(
            provider.page_cache_metrics(),
            rows::LruCacheMetrics::default()
        );

        assert!(
            provider.row_at(0).is_some(),
            "first page should miss and load"
        );
        assert!(
            provider.row_at(0).is_some(),
            "same page should hit after load"
        );
        for row_ix in 1..line_count {
            assert!(
                provider.row_at(row_ix).is_some(),
                "row {row_ix} should exist"
            );
        }

        assert_eq!(
            provider.page_cache_metrics(),
            rows::LruCacheMetrics {
                hits: 1,
                misses: line_count as u64,
                evictions: line_count.saturating_sub(FILE_DIFF_MAX_CACHED_PAGES) as u64,
                clears: 0,
            }
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
    fn streamed_file_diff_inline_full_text_matches_materialized_rows_without_paging() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let provider = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);
        source.reset_debug_counters();

        let eager_rows = provider
            .slice(0, provider.len_hint())
            .collect::<Vec<AnnotatedDiffLine>>();
        let direct = provider.build_full_text();

        assert_eq!(direct, build_inline_text(eager_rows.as_slice()));
        let counters = streamed_file_diff_debug_counters(&source);
        assert_eq!(counters.inline_full_text_materializations, 1);
        assert_eq!(
            counters.inline_rows_materialized,
            eager_rows.len() as u64,
            "only the explicit row slice should materialize inline rows"
        );
    }

    #[test]
    fn streamed_file_diff_debug_counters_track_page_cache_hits_and_misses() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let split_provider = PagedFileDiffRows::new(Arc::clone(&source), 2);
        let inline_provider = PagedFileDiffInlineRows::new(Arc::clone(&source), 2);
        source.reset_debug_counters();

        assert!(split_provider.row_at(0).is_some());
        assert!(split_provider.row_at(1).is_some());
        assert!(split_provider.row_at(2).is_some());
        assert!(inline_provider.row_at(0).is_some());
        assert!(inline_provider.row_at(1).is_some());
        assert!(inline_provider.row_at(2).is_some());

        let counters = streamed_file_diff_debug_counters(&source);
        assert_eq!(counters.split_page_cache_hits, 1);
        assert_eq!(counters.split_page_cache_misses, 2);
        assert_eq!(counters.inline_page_cache_hits, 1);
        assert_eq!(counters.inline_page_cache_misses, 2);
        assert_eq!(counters.split_rows_materialized, 3);
        assert_eq!(counters.inline_rows_materialized, 4);
        assert_eq!(counters.inline_full_text_materializations, 0);
    }
}
