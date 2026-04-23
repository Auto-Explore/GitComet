use super::*;

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

fn prefixed_inline_text(prefix: char, line: &str) -> gitcomet_core::domain::SharedLineText {
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
    file.content_signature()
}

fn build_file_diff_document_source(text: Option<&Arc<str>>) -> (SharedString, Arc<[usize]>) {
    let text = match text {
        Some(text) if !text.is_empty() => SharedString::from(Arc::clone(text)),
        _ => SharedString::default(),
    };
    let line_starts = Arc::from(build_line_starts(text.as_ref()));
    (text, line_starts)
}

fn file_diff_lines_from_starts<'a>(text: &'a str, line_starts: &[usize]) -> Vec<&'a str> {
    if text.is_empty() {
        return Vec::new();
    }

    let line_count = line_starts
        .len()
        .saturating_sub(usize::from(text.ends_with('\n')));
    let mut lines = Vec::with_capacity(line_count);
    for line_ix in 0..line_count {
        lines.push(
            text.get(line_byte_range(text, line_starts, line_ix))
                .unwrap_or_default(),
        );
    }
    lines
}

fn build_file_diff_plan_from_document_sources(
    old_text: &SharedString,
    old_line_starts: &[usize],
    new_text: &SharedString,
    new_line_starts: &[usize],
) -> gitcomet_core::file_diff::FileDiffPlan {
    let old_lines = file_diff_lines_from_starts(old_text.as_ref(), old_line_starts);
    let new_lines = file_diff_lines_from_starts(new_text.as_ref(), new_line_starts);
    gitcomet_core::file_diff::side_by_side_plan_from_lines(
        old_text.as_ref(),
        new_text.as_ref(),
        old_lines.as_slice(),
        new_lines.as_slice(),
    )
}

fn plan_line_to_inline_row_maps(
    plan: &gitcomet_core::file_diff::FileDiffPlan,
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut old_line_to_inline_row = vec![None; old_line_count];
    let mut new_line_to_inline_row = vec![None; new_line_count];
    let mut inline_start = 0usize;

    let assign = |line_to_row: &mut [Option<usize>], line_ix: usize, row_ix: usize| {
        if let Some(slot) = line_to_row.get_mut(line_ix) {
            *slot = Some(row_ix);
        }
    };

    for run in &plan.runs {
        match *run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let row_ix = inline_start.saturating_add(offset);
                    assign(
                        old_line_to_inline_row.as_mut_slice(),
                        old_start.saturating_add(offset),
                        row_ix,
                    );
                    assign(
                        new_line_to_inline_row.as_mut_slice(),
                        new_start.saturating_add(offset),
                        row_ix,
                    );
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..len {
                    assign(
                        old_line_to_inline_row.as_mut_slice(),
                        old_start.saturating_add(offset),
                        inline_start.saturating_add(offset),
                    );
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..len {
                    assign(
                        new_line_to_inline_row.as_mut_slice(),
                        new_start.saturating_add(offset),
                        inline_start.saturating_add(offset),
                    );
                }
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let pair_start = inline_start.saturating_add(offset.saturating_mul(2));
                    assign(
                        old_line_to_inline_row.as_mut_slice(),
                        old_start.saturating_add(offset),
                        pair_start,
                    );
                    assign(
                        new_line_to_inline_row.as_mut_slice(),
                        new_start.saturating_add(offset),
                        pair_start.saturating_add(1),
                    );
                }
            }
        }

        inline_start = inline_start.saturating_add(run.inline_row_len());
    }

    (old_line_to_inline_row, new_line_to_inline_row)
}

fn line_number(line_ix: usize) -> Option<u32> {
    line_ix
        .checked_add(1)
        .and_then(|line| u32::try_from(line).ok())
}

fn line_byte_range(text: &str, line_starts: &[usize], line_ix: usize) -> std::ops::Range<usize> {
    let text_len = text.len();
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    start..end
}

#[derive(Clone, Debug)]
pub(in crate::view) struct InlineFileDiffRowRenderData {
    pub(in crate::view) kind: gitcomet_core::domain::DiffLineKind,
    pub(in crate::view) old_line: Option<u32>,
    pub(in crate::view) new_line: Option<u32>,
    pub(in crate::view) text: gitcomet_core::file_diff::FileDiffLineText,
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
struct StreamedFileDiffRunStarts {
    split: Box<[usize]>,
    inline: Box<[usize]>,
}

impl StreamedFileDiffRunStarts {
    fn build(plan: &gitcomet_core::file_diff::FileDiffPlan) -> Self {
        let mut split = Vec::with_capacity(plan.runs.len());
        let mut inline = Vec::with_capacity(plan.runs.len());
        let mut split_start = 0usize;
        let mut inline_start = 0usize;
        for run in &plan.runs {
            split.push(split_start);
            inline.push(inline_start);
            split_start = split_start.saturating_add(run.row_len());
            inline_start = inline_start.saturating_add(run.inline_row_len());
        }
        Self {
            split: split.into_boxed_slice(),
            inline: inline.into_boxed_slice(),
        }
    }
}

#[derive(Debug)]
struct StreamedFileDiffSource {
    plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
    old_text: SharedString,
    old_text_arc: Arc<str>,
    old_line_starts: Arc<[usize]>,
    new_text: SharedString,
    new_text_arc: Arc<str>,
    new_line_starts: Arc<[usize]>,
    run_starts: std::sync::OnceLock<StreamedFileDiffRunStarts>,
}

impl StreamedFileDiffSource {
    fn new(
        plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
        old_text: SharedString,
        old_line_starts: Arc<[usize]>,
        new_text: SharedString,
        new_line_starts: Arc<[usize]>,
    ) -> Self {
        Self {
            plan,
            old_text_arc: old_text.clone().into(),
            old_text,
            old_line_starts,
            new_text_arc: new_text.clone().into(),
            new_text,
            new_line_starts,
            run_starts: std::sync::OnceLock::new(),
        }
    }

    fn run_starts(&self) -> &StreamedFileDiffRunStarts {
        self.run_starts
            .get_or_init(|| StreamedFileDiffRunStarts::build(self.plan.as_ref()))
    }

    fn split_run_starts(&self) -> &[usize] {
        self.run_starts().split.as_ref()
    }

    fn inline_run_starts(&self) -> &[usize] {
        self.run_starts().inline.as_ref()
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

    fn old_line_shared_text(&self, line_ix: usize) -> gitcomet_core::file_diff::FileDiffLineText {
        let range = line_byte_range(
            self.old_text_arc.as_ref(),
            self.old_line_starts.as_ref(),
            line_ix,
        );
        gitcomet_core::file_diff::FileDiffLineText::shared_slice(
            Arc::clone(&self.old_text_arc),
            range,
        )
    }

    fn new_line_shared_text(&self, line_ix: usize) -> gitcomet_core::file_diff::FileDiffLineText {
        let range = line_byte_range(
            self.new_text_arc.as_ref(),
            self.new_line_starts.as_ref(),
            line_ix,
        );
        gitcomet_core::file_diff::FileDiffLineText::shared_slice(
            Arc::clone(&self.new_text_arc),
            range,
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
        let (run_ix, local_ix) =
            Self::locate_run(self.split_run_starts(), self.plan.row_count, row_ix)?;
        let run = self.plan.runs.get(run_ix)?;
        let mut row = match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                let text = self.old_line_shared_text(old_ix);
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
                    old: Some(self.old_line_shared_text(old_ix)),
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
                    new: Some(self.new_line_shared_text(new_ix)),
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
                    old: Some(self.old_line_shared_text(old_ix)),
                    new: Some(self.new_line_shared_text(new_ix)),
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
            self.inline_run_starts(),
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

    fn inline_row_render_data(&self, inline_ix: usize) -> Option<InlineFileDiffRowRenderData> {
        let (run_ix, local_ix) = Self::locate_run(
            self.inline_run_starts(),
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
                Some(InlineFileDiffRowRenderData {
                    kind: gitcomet_core::domain::DiffLineKind::Context,
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                    text: self.old_line_shared_text(old_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                Some(InlineFileDiffRowRenderData {
                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                    old_line: line_number(old_ix),
                    new_line: None,
                    text: self.old_line_shared_text(old_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                Some(InlineFileDiffRowRenderData {
                    kind: gitcomet_core::domain::DiffLineKind::Add,
                    old_line: None,
                    new_line: line_number(new_ix),
                    text: self.new_line_shared_text(new_ix),
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
                    Some(InlineFileDiffRowRenderData {
                        kind: gitcomet_core::domain::DiffLineKind::Remove,
                        old_line: line_number(old_ix),
                        new_line: None,
                        text: self.old_line_shared_text(old_ix),
                    })
                } else {
                    Some(InlineFileDiffRowRenderData {
                        kind: gitcomet_core::domain::DiffLineKind::Add,
                        old_line: None,
                        new_line: line_number(new_ix),
                        text: self.new_line_shared_text(new_ix),
                    })
                }
            }
        }
    }

    fn split_modify_pair_texts(&self, row_ix: usize) -> Option<(&str, &str)> {
        let (run_ix, local_ix) =
            Self::locate_run(self.split_run_starts(), self.plan.row_count, row_ix)?;
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

    fn split_row_texts(&self, row_ix: usize) -> Option<(Option<&str>, Option<&str>)> {
        let (run_ix, local_ix) =
            Self::locate_run(self.split_run_starts(), self.plan.row_count, row_ix)?;
        let run = self.plan.runs.get(run_ix)?;
        match *run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                Some((
                    Some(self.old_line_text(old_ix)),
                    Some(self.new_line_text(new_ix)),
                ))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                Some((Some(self.old_line_text(old_ix)), None))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                Some((None, Some(self.new_line_text(new_ix))))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                Some((
                    Some(self.old_line_text(old_ix)),
                    Some(self.new_line_text(new_ix)),
                ))
            }
        }
    }

    fn inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(&str, &str, gitcomet_core::domain::DiffLineKind)> {
        let (run_ix, local_ix) = Self::locate_run(
            self.inline_run_starts(),
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
            self.inline_run_starts()
        } else {
            self.split_run_starts()
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
        let starts = self.split_run_starts();
        scrollbar_markers_from_row_ranges(
            self.plan.row_count,
            self.plan.runs.iter().enumerate().map(|(run_ix, run)| {
                let start = starts.get(run_ix).copied().unwrap_or(0);
                let end = start.saturating_add(run.row_len());
                (start, end, file_diff_row_flag(run.kind()))
            }),
        )
    }

    fn inline_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        let starts = self.inline_run_starts();
        scrollbar_markers_from_row_ranges(
            self.plan.inline_row_count,
            self.plan.runs.iter().enumerate().map(|(run_ix, run)| {
                let start = starts.get(run_ix).copied().unwrap_or(0);
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

pub(crate) struct PagedFileDiffRowsSliceIter<'a> {
    provider: &'a PagedFileDiffRows,
    next_ix: usize,
    end_ix: usize,
    current_page_ix: Option<usize>,
    current_page: Option<Arc<[FileDiffRow]>>,
}

impl<'a> PagedFileDiffRowsSliceIter<'a> {
    fn empty(provider: &'a PagedFileDiffRows) -> Self {
        Self {
            provider,
            next_ix: 0,
            end_ix: 0,
            current_page_ix: None,
            current_page: None,
        }
    }

    fn new(provider: &'a PagedFileDiffRows, start: usize, end: usize) -> Self {
        Self {
            provider,
            next_ix: start,
            end_ix: end,
            current_page_ix: None,
            current_page: None,
        }
    }
}

impl Iterator for PagedFileDiffRowsSliceIter<'_> {
    type Item = FileDiffRow;

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

pub(crate) struct PagedFileDiffInlineRowsSliceIter<'a> {
    provider: &'a PagedFileDiffInlineRows,
    next_ix: usize,
    end_ix: usize,
    current_page_ix: Option<usize>,
    current_page: Option<Arc<[AnnotatedDiffLine]>>,
}

impl<'a> PagedFileDiffInlineRowsSliceIter<'a> {
    fn empty(provider: &'a PagedFileDiffInlineRows) -> Self {
        Self {
            provider,
            next_ix: 0,
            end_ix: 0,
            current_page_ix: None,
            current_page: None,
        }
    }

    fn new(provider: &'a PagedFileDiffInlineRows, start: usize, end: usize) -> Self {
        Self {
            provider,
            next_ix: start,
            end_ix: end,
            current_page_ix: None,
            current_page: None,
        }
    }
}

impl Iterator for PagedFileDiffInlineRowsSliceIter<'_> {
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
        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[FileDiffRow]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
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

    pub(in crate::view) fn split_row_texts(
        &self,
        row_ix: usize,
    ) -> Option<(Option<&str>, Option<&str>)> {
        self.source.split_row_texts(row_ix)
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedFileDiffRows {
    type RowRef = FileDiffRow;
    type SliceIter<'a>
        = PagedFileDiffRowsSliceIter<'a>
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
            return PagedFileDiffRowsSliceIter::empty(self);
        }
        let end = end.min(self.source.split_len());
        PagedFileDiffRowsSliceIter::new(self, start, end)
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
        Some(Arc::from(rows))
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[AnnotatedDiffLine]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            return Some(Arc::clone(page));
        }

        let page = self.build_page(page_ix)?;
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

    pub(in crate::view) fn render_data(
        &self,
        inline_ix: usize,
    ) -> Option<InlineFileDiffRowRenderData> {
        self.source.inline_row_render_data(inline_ix)
    }

    pub(super) fn build_full_text(&self) -> SharedString {
        self.full_text
            .get_or_init(|| self.source.build_inline_text())
            .clone()
    }
}

impl gitcomet_core::domain::DiffRowProvider for PagedFileDiffInlineRows {
    type RowRef = AnnotatedDiffLine;
    type SliceIter<'a>
        = PagedFileDiffInlineRowsSliceIter<'a>
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
            return PagedFileDiffInlineRowsSliceIter::empty(self);
        }
        let end = end.min(self.source.inline_len());
        PagedFileDiffInlineRowsSliceIter::new(self, start, end)
    }
}

#[derive(Debug)]
pub(in crate::view) struct FileDiffCacheRebuild {
    pub(in crate::view) file_path: Option<std::path::PathBuf>,
    pub(in crate::view) language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) row_provider: Arc<PagedFileDiffRows>,
    pub(in crate::view) inline_row_provider: Arc<PagedFileDiffInlineRows>,
    pub(in crate::view) old_text: SharedString,
    pub(in crate::view) old_line_starts: Arc<[usize]>,
    pub(in crate::view) old_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) old_line_to_inline_row: Arc<[Option<usize>]>,
    pub(in crate::view) new_text: SharedString,
    pub(in crate::view) new_line_starts: Arc<[usize]>,
    pub(in crate::view) new_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) new_line_to_inline_row: Arc<[Option<usize>]>,
    pub(in crate::view) inline_text: SharedString,
    #[cfg(test)]
    pub(in crate::view) rows: Vec<FileDiffRow>,
    #[cfg(test)]
    pub(in crate::view) inline_rows: Vec<AnnotatedDiffLine>,
}

pub(in crate::view) fn build_file_diff_cache_rebuild(
    file: &gitcomet_core::domain::FileDiffText,
    workdir: &std::path::Path,
) -> FileDiffCacheRebuild {
    let (old_text, old_line_starts) = build_file_diff_document_source(file.old.as_ref());
    let (new_text, new_line_starts) = build_file_diff_document_source(file.new.as_ref());
    let plan = Arc::new(build_file_diff_plan_from_document_sources(
        &old_text,
        old_line_starts.as_ref(),
        &new_text,
        new_line_starts.as_ref(),
    ));
    let old_line_count =
        file_diff_lines_from_starts(old_text.as_ref(), old_line_starts.as_ref()).len();
    let new_line_count =
        file_diff_lines_from_starts(new_text.as_ref(), new_line_starts.as_ref()).len();
    let (old_line_to_row, new_line_to_row) = gitcomet_core::file_diff::plan_line_to_row_maps(
        plan.as_ref(),
        old_line_count,
        new_line_count,
    );
    let (old_line_to_inline_row, new_line_to_inline_row) =
        plan_line_to_inline_row_maps(plan.as_ref(), old_line_count, new_line_count);
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
        file.path.to_path_buf()
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
        old_line_to_row: Arc::from(old_line_to_row),
        old_line_to_inline_row: Arc::from(old_line_to_inline_row),
        new_text,
        new_line_starts,
        new_line_to_row: Arc::from(new_line_to_row),
        new_line_to_inline_row: Arc::from(new_line_to_inline_row),
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
        let old_text_arc = Arc::<str>::from(old);
        let new_text_arc = Arc::<str>::from(new);
        let (old_text, old_line_starts) = build_file_diff_document_source(Some(&old_text_arc));
        let (new_text, new_line_starts) = build_file_diff_document_source(Some(&new_text_arc));
        let plan = Arc::new(build_file_diff_plan_from_document_sources(
            &old_text,
            old_line_starts.as_ref(),
            &new_text,
            new_line_starts.as_ref(),
        ));
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

    #[test]
    fn build_inline_text_joins_lines_with_trailing_newline() {
        let rows = vec![
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "diff --git a/file b/file".into(),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-old".into(),
                old_line: Some(1),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: "+new".into(),
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
    fn file_diff_lines_from_starts_matches_std_lines_semantics() {
        for text in [
            "",
            "alpha",
            "\n",
            "alpha\n",
            "alpha\n\n",
            "alpha\nbeta",
            "alpha\nbeta\n",
            "alpha\n\nbeta\n",
        ] {
            let line_starts = build_line_starts(text);
            assert_eq!(
                file_diff_lines_from_starts(text, line_starts.as_slice()),
                text.lines().collect::<Vec<_>>(),
                "line slicing should keep std::str::lines semantics for {text:?}",
            );
        }
    }

    #[test]
    fn build_file_diff_cache_rebuild_preserves_real_document_sources() {
        let file = gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("src/demo.rs"),
            Some("alpha\nbeta\n".to_string()),
            Some("gamma\ndelta".to_string()),
        );

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

        let file = gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("src/demo.rs"),
            Some("struct Old;\nfn keep() {}\n".to_string()),
            Some("fn keep() {}\nlet added = 42;\n".to_string()),
        );

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
    fn streamed_file_diff_inline_full_text_matches_materialized_rows_without_paging() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta changed\ngamma\n",
        );
        let provider = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

        let eager_rows = provider
            .slice(0, provider.len_hint())
            .collect::<Vec<AnnotatedDiffLine>>();
        let direct = provider.build_full_text();

        assert_eq!(direct, build_inline_text(eager_rows.as_slice()));
    }
}
