use super::*;
use std::io::Read;

#[cfg(test)]
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

fn prefixed_inline_text(prefix: char, line: &str) -> gitcomet_core::domain::SharedLineText {
    let mut text = String::with_capacity(line.len().saturating_add(1));
    text.push(prefix);
    text.push_str(line);
    text.into()
}

pub(super) fn file_diff_text_signature(file: &gitcomet_core::domain::FileDiffText) -> u64 {
    file.content_signature()
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

/// Whether `line_starts` is the index of exactly this text.
///
/// The one place the indexing convention of [`IndexedFileDiffSource`] is
/// restated as a test: a start at 0, one after every `\n` including a trailing
/// one, and nothing else. A click can re-read a source-backed side long after
/// the diff was indexed, and a stale index is not a cosmetic mismatch -- the
/// pair projection would report ranges of a document the reader is not looking
/// at, and a shortened file leaves starts past the end of the text.
///
/// O(text) with no allocation, which is nothing beside the parse it guards.
pub(in crate::view) fn line_starts_describe(text: &str, line_starts: &[usize]) -> bool {
    let mut expected = line_starts.iter().copied();
    if expected.next() != Some(0) {
        return false;
    }
    for (ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' && expected.next() != Some(ix + 1) {
            return false;
        }
    }
    expected.next().is_none()
}

const FILE_DIFF_INDEX_SCAN_BUFFER_BYTES: usize = 64 * 1024;
const FILE_DIFF_INDEX_LINE_CAPACITY_MAX: usize = 64 * 1024;

#[inline]
fn file_diff_index_line_capacity_hint(source_len_hint: usize) -> usize {
    source_len_hint
        .saturating_div(64)
        .saturating_add(1)
        .min(FILE_DIFF_INDEX_LINE_CAPACITY_MAX)
}

#[derive(Clone, Debug)]
enum IndexedFileDiffContent {
    Empty,
    Shared(Arc<str>),
    File(Arc<std::path::PathBuf>),
}

#[derive(Clone, Debug)]
struct IndexedFileDiffSource {
    content: IndexedFileDiffContent,
    source_len: usize,
    line_starts: Arc<[usize]>,
    line_flags: Arc<[u8]>,
}

impl IndexedFileDiffSource {
    fn empty() -> Self {
        Self {
            content: IndexedFileDiffContent::Empty,
            source_len: 0,
            line_starts: Arc::default(),
            line_flags: Arc::default(),
        }
    }

    fn from_shared(text: Option<&Arc<str>>) -> Self {
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return Self::empty();
        };
        let line_starts: Arc<[usize]> = Arc::from(build_line_starts(text.as_ref()));
        let line_count = file_diff_line_count_from_starts(text.len(), line_starts.as_ref());
        let mut line_flags = Vec::with_capacity(line_count);
        for line_ix in 0..line_count {
            let range = line_byte_range(text.as_ref(), line_starts.as_ref(), line_ix);
            line_flags.push(preview_line_flags_for_text(
                text.get(range).unwrap_or_default(),
            ));
        }
        Self {
            content: IndexedFileDiffContent::Shared(Arc::clone(text)),
            source_len: text.len(),
            line_starts,
            line_flags: Arc::from(line_flags),
        }
    }

    fn from_file(path: &std::path::Path) -> Result<Self, String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            return Err("file diff source is a directory".to_string());
        }

        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut reader = std::io::BufReader::with_capacity(FILE_DIFF_INDEX_SCAN_BUFFER_BYTES, file);
        let source_len_hint = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        let line_capacity_hint = file_diff_index_line_capacity_hint(source_len_hint);
        let mut line_starts = Vec::with_capacity(line_capacity_hint);
        let mut line_flags = Vec::with_capacity(line_capacity_hint);
        let mut validation_buffer =
            Vec::with_capacity(FILE_DIFF_INDEX_SCAN_BUFFER_BYTES.saturating_add(4));
        let mut utf8_tail = Vec::with_capacity(4);
        let mut scan_buffer = vec![0u8; FILE_DIFF_INDEX_SCAN_BUFFER_BYTES];
        let mut source_len = 0usize;
        let mut line_ascii_only = true;
        let mut line_has_tabs = false;
        let mut ended_with_newline = false;

        if source_len_hint > 0 {
            line_starts.push(0);
        }

        loop {
            let read_len = reader
                .read(scan_buffer.as_mut_slice())
                .map_err(|e| e.to_string())?;
            if read_len == 0 {
                break;
            }
            if source_len == 0 && line_starts.is_empty() {
                line_starts.push(0);
            }
            let chunk = &scan_buffer[..read_len];
            validate_file_diff_utf8_chunk_streaming(&mut utf8_tail, &mut validation_buffer, chunk)?;

            for &byte in chunk {
                ended_with_newline = byte == b'\n';
                if byte == b'\n' {
                    line_flags.push(preview_line_flags_from_bools(
                        line_ascii_only,
                        line_has_tabs,
                    ));
                    source_len = source_len.saturating_add(1);
                    line_starts.push(source_len);
                    line_ascii_only = true;
                    line_has_tabs = false;
                    continue;
                }

                if !byte.is_ascii() {
                    line_ascii_only = false;
                }
                if byte == b'\t' {
                    line_has_tabs = true;
                }
                source_len = source_len.saturating_add(1);
            }
        }

        if !utf8_tail.is_empty() {
            return Err("file diff source is not valid UTF-8".to_string());
        }
        if source_len > 0 && !ended_with_newline {
            line_flags.push(preview_line_flags_from_bools(
                line_ascii_only,
                line_has_tabs,
            ));
        }

        Ok(Self {
            content: IndexedFileDiffContent::File(Arc::new(path.to_path_buf())),
            source_len,
            line_starts: Arc::from(line_starts),
            line_flags: Arc::from(line_flags),
        })
    }

    fn line_count(&self) -> usize {
        self.line_flags.len()
    }

    fn line_byte_range(&self, line_ix: usize) -> Option<std::ops::Range<usize>> {
        if line_ix >= self.line_count() {
            return None;
        }
        let start = self
            .line_starts
            .get(line_ix)
            .copied()
            .unwrap_or(self.source_len)
            .min(self.source_len);
        let end = self
            .line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .map(|next| next.saturating_sub(1))
            .unwrap_or(self.source_len)
            .min(self.source_len)
            .max(start);
        Some(start..end)
    }

    fn line_text(&self, line_ix: usize) -> Option<gitcomet_core::file_diff::FileDiffLineText> {
        let range = self.line_byte_range(line_ix)?;
        let flags = self.line_flags.get(line_ix).copied().unwrap_or_default();
        match &self.content {
            IndexedFileDiffContent::Empty => None,
            IndexedFileDiffContent::Shared(text) => Some(
                gitcomet_core::file_diff::FileDiffLineText::shared_slice(Arc::clone(text), range),
            ),
            IndexedFileDiffContent::File(path) => {
                Some(gitcomet_core::file_diff::FileDiffLineText::file_slice(
                    Arc::clone(path),
                    range,
                    preview_line_is_ascii_without_loading(flags),
                    preview_line_has_tabs_without_loading(flags),
                ))
            }
        }
    }
}

fn file_diff_line_count_from_starts(source_len: usize, line_starts: &[usize]) -> usize {
    if source_len == 0 {
        return 0;
    }
    line_starts
        .len()
        .saturating_sub(usize::from(line_starts.last().copied() == Some(source_len)))
}

fn validate_file_diff_utf8_chunk_streaming(
    utf8_tail: &mut Vec<u8>,
    validation_buffer: &mut Vec<u8>,
    chunk: &[u8],
) -> Result<(), String> {
    validation_buffer.clear();
    if !utf8_tail.is_empty() {
        validation_buffer.extend_from_slice(utf8_tail.as_slice());
    }
    validation_buffer.extend_from_slice(chunk);

    match std::str::from_utf8(validation_buffer.as_slice()) {
        Ok(_) => {
            utf8_tail.clear();
            Ok(())
        }
        Err(error) => {
            if error.error_len().is_some() {
                return Err("file diff source is not valid UTF-8".to_string());
            }
            let valid_up_to = error.valid_up_to();
            utf8_tail.clear();
            utf8_tail.extend_from_slice(&validation_buffer[valid_up_to..]);
            Ok(())
        }
    }
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

fn push_file_diff_line_without_whitespace(
    source: &IndexedFileDiffSource,
    line_ix: usize,
    out: &mut String,
) {
    if let Some(text) = source.line_text(line_ix) {
        super::append_non_whitespace(text.as_ref(), out);
    }
}

fn append_file_diff_run_without_whitespace(
    run: &gitcomet_core::file_diff::FileDiffPlanRun,
    old_source: &IndexedFileDiffSource,
    new_source: &IndexedFileDiffSource,
    old_out: &mut String,
    new_out: &mut String,
) {
    match *run {
        gitcomet_core::file_diff::FileDiffPlanRun::Context { .. } => {}
        gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
            for offset in 0..len {
                push_file_diff_line_without_whitespace(
                    old_source,
                    old_start.saturating_add(offset),
                    old_out,
                );
            }
        }
        gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
            for offset in 0..len {
                push_file_diff_line_without_whitespace(
                    new_source,
                    new_start.saturating_add(offset),
                    new_out,
                );
            }
        }
        gitcomet_core::file_diff::FileDiffPlanRun::Modify {
            old_start,
            new_start,
            len,
        } => {
            for offset in 0..len {
                push_file_diff_line_without_whitespace(
                    old_source,
                    old_start.saturating_add(offset),
                    old_out,
                );
                push_file_diff_line_without_whitespace(
                    new_source,
                    new_start.saturating_add(offset),
                    new_out,
                );
            }
        }
    }
}

fn visual_kinds_for_file_diff_plan(
    plan: &gitcomet_core::file_diff::FileDiffPlan,
    old_source: &IndexedFileDiffSource,
    new_source: &IndexedFileDiffSource,
    whitespace_mode: DiffWhitespaceMode,
) -> (
    Box<[gitcomet_core::file_diff::FileDiffRowKind]>,
    Box<[gitcomet_core::domain::DiffLineKind]>,
) {
    use gitcomet_core::domain::DiffLineKind as DK;
    use gitcomet_core::file_diff::{FileDiffPlanRun, FileDiffRowKind as RK};

    let starts = StreamedFileDiffRunStarts::build(plan);
    let mut split = vec![RK::Context; plan.row_count];
    let mut inline = vec![DK::Context; plan.inline_row_count];

    for (run_ix, run) in plan.runs.iter().enumerate() {
        let split_start = starts.split.get(run_ix).copied().unwrap_or(0);
        let inline_start = starts.inline.get(run_ix).copied().unwrap_or(0);
        match *run {
            FileDiffPlanRun::Context { len, .. } => {
                for row_ix in split_start..split_start.saturating_add(len).min(split.len()) {
                    split[row_ix] = RK::Context;
                }
                for inline_ix in inline_start..inline_start.saturating_add(len).min(inline.len()) {
                    inline[inline_ix] = DK::Context;
                }
            }
            FileDiffPlanRun::Remove { len, .. } => {
                for row_ix in split_start..split_start.saturating_add(len).min(split.len()) {
                    split[row_ix] = RK::Remove;
                }
                for inline_ix in inline_start..inline_start.saturating_add(len).min(inline.len()) {
                    inline[inline_ix] = DK::Remove;
                }
            }
            FileDiffPlanRun::Add { len, .. } => {
                for row_ix in split_start..split_start.saturating_add(len).min(split.len()) {
                    split[row_ix] = RK::Add;
                }
                for inline_ix in inline_start..inline_start.saturating_add(len).min(inline.len()) {
                    inline[inline_ix] = DK::Add;
                }
            }
            FileDiffPlanRun::Modify { len, .. } => {
                for row_ix in split_start..split_start.saturating_add(len).min(split.len()) {
                    split[row_ix] = RK::Modify;
                }
                for offset in 0..len {
                    let old_ix = inline_start.saturating_add(offset.saturating_mul(2));
                    let new_ix = old_ix.saturating_add(1);
                    if let Some(kind) = inline.get_mut(old_ix) {
                        *kind = DK::Remove;
                    }
                    if let Some(kind) = inline.get_mut(new_ix) {
                        *kind = DK::Add;
                    }
                }
            }
        }
    }

    if whitespace_mode == DiffWhitespaceMode::Ignore {
        let mut run_ix = 0usize;
        while run_ix < plan.runs.len() {
            if matches!(plan.runs[run_ix].kind(), RK::Context) {
                run_ix += 1;
                continue;
            }

            let group_start = run_ix;
            let mut old_stripped = String::new();
            let mut new_stripped = String::new();
            while run_ix < plan.runs.len() && !matches!(plan.runs[run_ix].kind(), RK::Context) {
                append_file_diff_run_without_whitespace(
                    &plan.runs[run_ix],
                    old_source,
                    new_source,
                    &mut old_stripped,
                    &mut new_stripped,
                );
                run_ix += 1;
            }

            if old_stripped == new_stripped {
                for neutral_run_ix in group_start..run_ix {
                    let run = &plan.runs[neutral_run_ix];
                    let split_start = starts.split.get(neutral_run_ix).copied().unwrap_or(0);
                    let inline_start = starts.inline.get(neutral_run_ix).copied().unwrap_or(0);
                    for row_ix in
                        split_start..split_start.saturating_add(run.row_len()).min(split.len())
                    {
                        split[row_ix] = RK::Context;
                    }
                    for inline_ix in inline_start
                        ..inline_start
                            .saturating_add(run.inline_row_len())
                            .min(inline.len())
                    {
                        inline[inline_ix] = DK::Context;
                    }
                }
            }
        }
    }

    (split.into_boxed_slice(), inline.into_boxed_slice())
}

#[derive(Debug)]
struct StreamedFileDiffSource {
    plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
    old_source: IndexedFileDiffSource,
    new_source: IndexedFileDiffSource,
    split_visual_kinds: Box<[gitcomet_core::file_diff::FileDiffRowKind]>,
    inline_visual_kinds: Box<[gitcomet_core::domain::DiffLineKind]>,
    run_starts: std::sync::OnceLock<StreamedFileDiffRunStarts>,
}

impl StreamedFileDiffSource {
    fn new(
        plan: Arc<gitcomet_core::file_diff::FileDiffPlan>,
        old_source: IndexedFileDiffSource,
        new_source: IndexedFileDiffSource,
        whitespace_mode: DiffWhitespaceMode,
    ) -> Self {
        let (split_visual_kinds, inline_visual_kinds) = visual_kinds_for_file_diff_plan(
            plan.as_ref(),
            &old_source,
            &new_source,
            whitespace_mode,
        );
        Self {
            plan,
            old_source,
            new_source,
            split_visual_kinds,
            inline_visual_kinds,
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

    fn split_visual_kind(
        &self,
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRowKind> {
        self.split_visual_kinds.get(row_ix).copied()
    }

    fn inline_visual_kind(&self, inline_ix: usize) -> Option<gitcomet_core::domain::DiffLineKind> {
        self.inline_visual_kinds.get(inline_ix).copied()
    }

    fn old_line_shared_text(&self, line_ix: usize) -> gitcomet_core::file_diff::FileDiffLineText {
        self.old_source
            .line_text(line_ix)
            .unwrap_or_else(|| gitcomet_core::file_diff::FileDiffLineText::from(""))
    }

    fn new_line_shared_text(&self, line_ix: usize) -> gitcomet_core::file_diff::FileDiffLineText {
        self.new_source
            .line_text(line_ix)
            .unwrap_or_else(|| gitcomet_core::file_diff::FileDiffLineText::from(""))
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
                FileDiffRow {
                    kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                    old: Some(self.old_line_shared_text(old_ix)),
                    new: Some(self.new_line_shared_text(new_ix)),
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
                let text = self.new_line_shared_text(new_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Context,
                    text: prefixed_inline_text(' ', text.as_ref()),
                    old_line: line_number(old_ix),
                    new_line: line_number(new_ix),
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                let text = self.old_line_shared_text(old_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                    text: prefixed_inline_text('-', text.as_ref()),
                    old_line: line_number(old_ix),
                    new_line: None,
                })
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                let text = self.new_line_shared_text(new_ix);
                Some(AnnotatedDiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Add,
                    text: prefixed_inline_text('+', text.as_ref()),
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
                    let text = self.old_line_shared_text(old_ix);
                    Some(AnnotatedDiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Remove,
                        text: prefixed_inline_text('-', text.as_ref()),
                        old_line: line_number(old_ix),
                        new_line: None,
                    })
                } else {
                    let text = self.new_line_shared_text(new_ix);
                    Some(AnnotatedDiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Add,
                        text: prefixed_inline_text('+', text.as_ref()),
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
                    text: self.new_line_shared_text(new_ix),
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

    fn split_modify_pair_texts(
        &self,
        row_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
    )> {
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
        Some((
            self.old_line_shared_text(old_ix),
            self.new_line_shared_text(new_ix),
        ))
    }

    fn split_row_texts(
        &self,
        row_ix: usize,
    ) -> Option<(
        Option<gitcomet_core::file_diff::FileDiffLineText>,
        Option<gitcomet_core::file_diff::FileDiffLineText>,
    )> {
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
                    Some(self.old_line_shared_text(old_ix)),
                    Some(self.new_line_shared_text(new_ix)),
                ))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, .. } => {
                let old_ix = old_start.saturating_add(local_ix);
                Some((Some(self.old_line_shared_text(old_ix)), None))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, .. } => {
                let new_ix = new_start.saturating_add(local_ix);
                Some((None, Some(self.new_line_shared_text(new_ix))))
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                ..
            } => {
                let old_ix = old_start.saturating_add(local_ix);
                let new_ix = new_start.saturating_add(local_ix);
                Some((
                    Some(self.old_line_shared_text(old_ix)),
                    Some(self.new_line_shared_text(new_ix)),
                ))
            }
        }
    }

    fn inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::domain::DiffLineKind,
    )> {
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
        Some((
            self.old_line_shared_text(old_ix),
            self.new_line_shared_text(new_ix),
            kind,
        ))
    }

    fn change_visible_indices_for_runs(&self, inline: bool) -> Vec<usize> {
        let len = if inline {
            self.plan.inline_row_count
        } else {
            self.plan.row_count
        };
        let mut out = Vec::new();

        for row_ix in 0..len {
            let is_change = if inline {
                self.inline_visual_kind(row_ix).is_some_and(|kind| {
                    matches!(
                        kind,
                        gitcomet_core::domain::DiffLineKind::Add
                            | gitcomet_core::domain::DiffLineKind::Remove
                    )
                })
            } else {
                self.split_visual_kind(row_ix).is_some_and(|kind| {
                    !matches!(kind, gitcomet_core::file_diff::FileDiffRowKind::Context)
                })
            };
            if is_change {
                out.push(row_ix);
            }
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
        let mut ranges = Vec::new();
        let mut start = 0usize;
        while start < self.plan.row_count {
            let flag = self
                .split_visual_kind(start)
                .map(file_diff_row_flag)
                .unwrap_or(0);
            let mut end = start.saturating_add(1);
            while end < self.plan.row_count
                && self
                    .split_visual_kind(end)
                    .map(file_diff_row_flag)
                    .unwrap_or(0)
                    == flag
            {
                end = end.saturating_add(1);
            }
            ranges.push((start, end, flag));
            start = end;
        }
        scrollbar_markers_from_row_ranges(self.plan.row_count, ranges)
    }

    fn inline_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        let mut ranges = Vec::new();
        let mut start = 0usize;
        while start < self.plan.inline_row_count {
            let flag = match self.inline_visual_kind(start) {
                Some(gitcomet_core::domain::DiffLineKind::Add) => 1,
                Some(gitcomet_core::domain::DiffLineKind::Remove) => 2,
                _ => 0,
            };
            let mut end = start.saturating_add(1);
            while end < self.plan.inline_row_count {
                let next_flag = match self.inline_visual_kind(end) {
                    Some(gitcomet_core::domain::DiffLineKind::Add) => 1,
                    Some(gitcomet_core::domain::DiffLineKind::Remove) => 2,
                    _ => 0,
                };
                if next_flag != flag {
                    break;
                }
                end = end.saturating_add(1);
            }
            ranges.push((start, end, flag));
            start = end;
        }
        scrollbar_markers_from_row_ranges(self.plan.inline_row_count, ranges)
    }

    #[cfg(test)]
    fn build_inline_text(&self) -> SharedString {
        let mut text = String::with_capacity(self.plan.inline_row_count.saturating_mul(2));
        let mut push_line = |prefix: char, line: gitcomet_core::file_diff::FileDiffLineText| {
            text.push(prefix);
            text.push_str(line.as_ref());
            text.push('\n');
        };

        for run in self.plan.runs.as_slice() {
            match *run {
                gitcomet_core::file_diff::FileDiffPlanRun::Context { new_start, len, .. } => {
                    for offset in 0..len {
                        push_line(
                            ' ',
                            self.new_line_shared_text(new_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                    for offset in 0..len {
                        push_line(
                            '-',
                            self.old_line_shared_text(old_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                    for offset in 0..len {
                        push_line(
                            '+',
                            self.new_line_shared_text(new_start.saturating_add(offset)),
                        );
                    }
                }
                gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                    old_start,
                    new_start,
                    len,
                } => {
                    for offset in 0..len {
                        push_line(
                            '-',
                            self.old_line_shared_text(old_start.saturating_add(offset)),
                        );
                        push_line(
                            '+',
                            self.new_line_shared_text(new_start.saturating_add(offset)),
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

    pub(in crate::view) fn visual_kind(
        &self,
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRowKind> {
        self.source.split_visual_kind(row_ix)
    }

    pub(in crate::view) fn modify_pair_texts(
        &self,
        row_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
    )> {
        self.source.split_modify_pair_texts(row_ix)
    }

    pub(in crate::view) fn split_row_texts(
        &self,
        row_ix: usize,
    ) -> Option<(
        Option<gitcomet_core::file_diff::FileDiffLineText>,
        Option<gitcomet_core::file_diff::FileDiffLineText>,
    )> {
        self.source.split_row_texts(row_ix)
    }

    pub(in crate::view) fn render_data(&self, row_ix: usize) -> Option<FileDiffRow> {
        self.source.split_row(row_ix)
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
    #[cfg(test)]
    full_text: std::sync::OnceLock<SharedString>,
}

impl PagedFileDiffInlineRows {
    fn new(source: Arc<StreamedFileDiffSource>, page_size: usize) -> Self {
        Self {
            source,
            page_size: page_size.max(1),
            pages: std::sync::Mutex::new(rows::new_lru_cache(FILE_DIFF_MAX_CACHED_PAGES)),
            #[cfg(test)]
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

    pub(in crate::view) fn visual_kind(
        &self,
        inline_ix: usize,
    ) -> Option<gitcomet_core::domain::DiffLineKind> {
        self.source.inline_visual_kind(inline_ix)
    }

    pub(in crate::view) fn modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::domain::DiffLineKind,
    )> {
        self.source.inline_modify_pair_texts(inline_ix)
    }

    pub(in crate::view) fn render_data(
        &self,
        inline_ix: usize,
    ) -> Option<InlineFileDiffRowRenderData> {
        self.source.inline_row_render_data(inline_ix)
    }

    #[cfg(test)]
    fn build_full_text(&self) -> SharedString {
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

fn file_diff_source_text(source: &IndexedFileDiffSource) -> SharedString {
    match &source.content {
        IndexedFileDiffContent::Shared(text) => SharedString::from(Arc::clone(text)),
        IndexedFileDiffContent::Empty | IndexedFileDiffContent::File(_) => SharedString::default(),
    }
}

/// Where a side's content lives, when it is a file rather than text in memory.
///
/// A source-backed side deliberately keeps its text off the heap -- that is what
/// lets a huge diff render from per-line slices. Anything that genuinely needs
/// the whole document (a tree-sitter parse, and so delimiter matching) can read
/// it back from here on demand instead of forcing every diff to hold it.
fn file_diff_source_path(source: &IndexedFileDiffSource) -> Option<Arc<std::path::PathBuf>> {
    match &source.content {
        IndexedFileDiffContent::File(path) => Some(Arc::clone(path)),
        IndexedFileDiffContent::Shared(_) | IndexedFileDiffContent::Empty => None,
    }
}

/// What the file behind a source-backed side looked like when this generation
/// indexed it: path, length and mtime, hashed.
///
/// Read back at click time to answer "is this still the document the rows are
/// showing?". [`line_starts_describe`] cannot answer that -- it validates where
/// the newlines are and says nothing about the bytes between them, so an edit
/// that keeps the line lengths passes it while changing every column the pair
/// projection is about to report.
pub(in crate::view) fn file_diff_source_identity(
    path: Option<&Arc<std::path::PathBuf>>,
) -> Option<Arc<str>> {
    path.map(|path| gitcomet_core::domain::FileDiffTextSource::new(path.as_ref().clone()).identity)
}

fn file_diff_source_plan_text(source: &IndexedFileDiffSource) -> Result<SharedString, String> {
    match &source.content {
        IndexedFileDiffContent::Shared(text) => Ok(SharedString::from(Arc::clone(text))),
        IndexedFileDiffContent::Empty => Ok(SharedString::default()),
        IndexedFileDiffContent::File(path) => std::fs::read_to_string(path.as_ref())
            .map(SharedString::from)
            .map_err(|error| {
                format!(
                    "Unable to load file diff source `{}` for diff planning: {error}",
                    path.display()
                )
            }),
    }
}

fn build_file_diff_plan_from_indexed_sources(
    old_source: &IndexedFileDiffSource,
    new_source: &IndexedFileDiffSource,
) -> Result<gitcomet_core::file_diff::FileDiffPlan, String> {
    let old_plan_text = file_diff_source_plan_text(old_source)?;
    let new_plan_text = file_diff_source_plan_text(new_source)?;
    Ok(build_file_diff_plan_from_document_sources(
        &old_plan_text,
        old_source.line_starts.as_ref(),
        &new_plan_text,
        new_source.line_starts.as_ref(),
    ))
}

fn index_file_diff_side(
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    legacy_text: Option<&Arc<str>>,
) -> Result<IndexedFileDiffSource, String> {
    if let Some(source) = source {
        return IndexedFileDiffSource::from_file(&source.path).map_err(|error| {
            format!(
                "Unable to load file diff source `{}`: {error}",
                source.path.display()
            )
        });
    }
    Ok(IndexedFileDiffSource::from_shared(legacy_text))
}

fn file_diff_plan_from_runs(
    runs: Vec<gitcomet_core::file_diff::FileDiffPlanRun>,
) -> gitcomet_core::file_diff::FileDiffPlan {
    let row_count = runs.iter().map(|run| run.row_len()).sum();
    let inline_row_count = runs.iter().map(|run| run.inline_row_len()).sum();
    gitcomet_core::file_diff::FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline: None,
    }
}

fn push_file_diff_plan_run(
    runs: &mut Vec<gitcomet_core::file_diff::FileDiffPlanRun>,
    run: gitcomet_core::file_diff::FileDiffPlanRun,
) {
    if run.row_len() == 0 {
        return;
    }
    runs.push(run);
}

fn push_aligned_file_diff_span(
    runs: &mut Vec<gitcomet_core::file_diff::FileDiffPlanRun>,
    old_start: usize,
    new_start: usize,
    old_len: usize,
    new_len: usize,
) {
    let context_len = old_len.min(new_len);
    push_file_diff_plan_run(
        runs,
        gitcomet_core::file_diff::FileDiffPlanRun::Context {
            old_start,
            new_start,
            len: context_len,
        },
    );
    if old_len > context_len {
        push_file_diff_plan_run(
            runs,
            gitcomet_core::file_diff::FileDiffPlanRun::Remove {
                old_start: old_start.saturating_add(context_len),
                len: old_len.saturating_sub(context_len),
            },
        );
    }
    if new_len > context_len {
        push_file_diff_plan_run(
            runs,
            gitcomet_core::file_diff::FileDiffPlanRun::Add {
                new_start: new_start.saturating_add(context_len),
                len: new_len.saturating_sub(context_len),
            },
        );
    }
}

fn push_file_diff_change_block(
    runs: &mut Vec<gitcomet_core::file_diff::FileDiffPlanRun>,
    old_start: Option<usize>,
    old_len: usize,
    new_start: Option<usize>,
    new_len: usize,
) {
    let pair_len = old_len.min(new_len);
    if pair_len > 0 {
        push_file_diff_plan_run(
            runs,
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start: old_start.unwrap_or_default(),
                new_start: new_start.unwrap_or_default(),
                len: pair_len,
            },
        );
    }
    if old_len > pair_len {
        push_file_diff_plan_run(
            runs,
            gitcomet_core::file_diff::FileDiffPlanRun::Remove {
                old_start: old_start.unwrap_or_default().saturating_add(pair_len),
                len: old_len.saturating_sub(pair_len),
            },
        );
    }
    if new_len > pair_len {
        push_file_diff_plan_run(
            runs,
            gitcomet_core::file_diff::FileDiffPlanRun::Add {
                new_start: new_start.unwrap_or_default().saturating_add(pair_len),
                len: new_len.saturating_sub(pair_len),
            },
        );
    }
}

fn parse_unified_hunk_range(part: &str) -> Option<(usize, usize)> {
    let mut pieces = part.split(',');
    let start = pieces.next()?.parse::<usize>().ok()?;
    let len = pieces
        .next()
        .map(str::parse::<usize>)
        .transpose()
        .ok()?
        .unwrap_or(1);
    Some((start.saturating_sub(1), len))
}

fn parse_unified_hunk_header(text: &str) -> Option<(usize, usize, usize, usize)> {
    let body = text.strip_prefix("@@")?.trim_start();
    let body = body.split("@@").next()?.trim();
    let mut parts = body.split_whitespace();
    let (old_start, old_len) = parse_unified_hunk_range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_len) = parse_unified_hunk_range(parts.next()?.strip_prefix('+')?)?;
    Some((old_start, old_len, new_start, new_len))
}

fn build_file_diff_plan_from_patch(
    diff: &gitcomet_core::domain::Diff,
    old_line_count: usize,
    new_line_count: usize,
) -> gitcomet_core::file_diff::FileDiffPlan {
    let mut runs = Vec::new();
    let mut old_cursor = 0usize;
    let mut new_cursor = 0usize;
    let mut ix = 0usize;

    while ix < diff.lines.len() {
        let line = &diff.lines[ix];
        if !matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
            ix += 1;
            continue;
        }
        let Some((hunk_old_start, _hunk_old_len, hunk_new_start, _hunk_new_len)) =
            parse_unified_hunk_header(line.text.as_ref())
        else {
            ix += 1;
            continue;
        };

        push_aligned_file_diff_span(
            &mut runs,
            old_cursor,
            new_cursor,
            hunk_old_start.saturating_sub(old_cursor),
            hunk_new_start.saturating_sub(new_cursor),
        );

        let mut old_ix = hunk_old_start;
        let mut new_ix = hunk_new_start;
        let mut pending_old_start = None;
        let mut pending_old_len = 0usize;
        let mut pending_new_start = None;
        let mut pending_new_len = 0usize;
        ix += 1;

        while ix < diff.lines.len() {
            let line = &diff.lines[ix];
            if matches!(
                line.kind,
                gitcomet_core::domain::DiffLineKind::Hunk
                    | gitcomet_core::domain::DiffLineKind::Header
            ) {
                break;
            }

            if line.text.as_ref().starts_with("\\ No newline") {
                ix += 1;
                continue;
            }

            match line.kind {
                gitcomet_core::domain::DiffLineKind::Context => {
                    push_file_diff_change_block(
                        &mut runs,
                        pending_old_start.take(),
                        pending_old_len,
                        pending_new_start.take(),
                        pending_new_len,
                    );
                    pending_old_len = 0;
                    pending_new_len = 0;
                    push_file_diff_plan_run(
                        &mut runs,
                        gitcomet_core::file_diff::FileDiffPlanRun::Context {
                            old_start: old_ix,
                            new_start: new_ix,
                            len: 1,
                        },
                    );
                    old_ix = old_ix.saturating_add(1);
                    new_ix = new_ix.saturating_add(1);
                }
                gitcomet_core::domain::DiffLineKind::Remove => {
                    if pending_old_start.is_none() {
                        pending_old_start = Some(old_ix);
                    }
                    pending_old_len = pending_old_len.saturating_add(1);
                    old_ix = old_ix.saturating_add(1);
                }
                gitcomet_core::domain::DiffLineKind::Add => {
                    if pending_new_start.is_none() {
                        pending_new_start = Some(new_ix);
                    }
                    pending_new_len = pending_new_len.saturating_add(1);
                    new_ix = new_ix.saturating_add(1);
                }
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => {}
            }
            ix += 1;
        }

        push_file_diff_change_block(
            &mut runs,
            pending_old_start,
            pending_old_len,
            pending_new_start,
            pending_new_len,
        );
        old_cursor = old_ix;
        new_cursor = new_ix;
    }

    push_aligned_file_diff_span(
        &mut runs,
        old_cursor,
        new_cursor,
        old_line_count.saturating_sub(old_cursor),
        new_line_count.saturating_sub(new_cursor),
    );

    file_diff_plan_from_runs(runs)
}

#[derive(Debug)]
pub(in crate::view) struct FileDiffCacheRebuild {
    pub(in crate::view) file_path: Option<std::path::PathBuf>,
    pub(in crate::view) language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) row_provider: Arc<PagedFileDiffRows>,
    pub(in crate::view) inline_row_provider: Arc<PagedFileDiffInlineRows>,
    pub(in crate::view) old_source_path: Option<Arc<std::path::PathBuf>>,
    pub(in crate::view) new_source_path: Option<Arc<std::path::PathBuf>>,
    /// Filesystem identity of each source-backed side as indexed, so a click can
    /// tell whether the file has moved on. See [`file_diff_source_identity`].
    pub(in crate::view) old_source_identity: Option<Arc<str>>,
    pub(in crate::view) new_source_identity: Option<Arc<str>>,
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

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn build_file_diff_cache_rebuild(
    file: &gitcomet_core::domain::FileDiffText,
    workdir: &std::path::Path,
) -> Result<FileDiffCacheRebuild, String> {
    build_file_diff_cache_rebuild_with_patch(file, workdir, None, DiffWhitespaceMode::Show)
}

pub(in crate::view) fn build_file_diff_cache_rebuild_with_patch(
    file: &gitcomet_core::domain::FileDiffText,
    workdir: &std::path::Path,
    patch_diff: Option<&gitcomet_core::domain::Diff>,
    whitespace_mode: DiffWhitespaceMode,
) -> Result<FileDiffCacheRebuild, String> {
    let old_source = index_file_diff_side(file.old_source.as_ref(), file.old.as_ref())?;
    let new_source = index_file_diff_side(file.new_source.as_ref(), file.new.as_ref())?;
    let old_text = file_diff_source_text(&old_source);
    let new_text = file_diff_source_text(&new_source);
    let old_source_path = file_diff_source_path(&old_source);
    let new_source_path = file_diff_source_path(&new_source);
    // Taken straight after the index read, so it describes the same bytes the
    // rows and `line_starts` below were built from.
    let old_source_identity = file_diff_source_identity(old_source_path.as_ref());
    let new_source_identity = file_diff_source_identity(new_source_path.as_ref());
    let old_line_starts = Arc::clone(&old_source.line_starts);
    let new_line_starts = Arc::clone(&new_source.line_starts);
    let old_line_count = old_source.line_count();
    let new_line_count = new_source.line_count();
    let plan = Arc::new(if let Some(patch_diff) = patch_diff {
        build_file_diff_plan_from_patch(patch_diff, old_line_count, new_line_count)
    } else {
        build_file_diff_plan_from_indexed_sources(&old_source, &new_source)?
    });
    let (old_line_to_row, new_line_to_row) = gitcomet_core::file_diff::plan_line_to_row_maps(
        plan.as_ref(),
        old_line_count,
        new_line_count,
    );
    let (old_line_to_inline_row, new_line_to_inline_row) =
        plan_line_to_inline_row_maps(plan.as_ref(), old_line_count, new_line_count);
    let source = Arc::new(StreamedFileDiffSource::new(
        Arc::clone(&plan),
        old_source,
        new_source,
        whitespace_mode,
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

    Ok(FileDiffCacheRebuild {
        file_path,
        language,
        old_source_path,
        new_source_path,
        old_source_identity,
        new_source_identity,
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn streamed_file_diff_source_for_test_with_mode(
        old: &str,
        new: &str,
        whitespace_mode: DiffWhitespaceMode,
    ) -> Arc<StreamedFileDiffSource> {
        let old_text_arc = Arc::<str>::from(old);
        let new_text_arc = Arc::<str>::from(new);
        let old_source = IndexedFileDiffSource::from_shared(Some(&old_text_arc));
        let new_source = IndexedFileDiffSource::from_shared(Some(&new_text_arc));
        let old_text = file_diff_source_text(&old_source);
        let new_text = file_diff_source_text(&new_source);
        let plan = Arc::new(build_file_diff_plan_from_document_sources(
            &old_text,
            old_source.line_starts.as_ref(),
            &new_text,
            new_source.line_starts.as_ref(),
        ));
        Arc::new(StreamedFileDiffSource::new(
            plan,
            old_source,
            new_source,
            whitespace_mode,
        ))
    }

    fn streamed_file_diff_source_for_test(old: &str, new: &str) -> Arc<StreamedFileDiffSource> {
        streamed_file_diff_source_for_test_with_mode(old, new, DiffWhitespaceMode::Show)
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
    fn file_diff_visual_kinds_ignore_whitespace_only_changes() {
        let source = streamed_file_diff_source_for_test_with_mode(
            "let\tvalue = 1;\n",
            "let value=1;\n",
            DiffWhitespaceMode::Ignore,
        );
        let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
        let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

        let split_raw = split
            .slice(0, split.len_hint())
            .map(|row| row.kind)
            .collect::<Vec<_>>();
        let inline_raw = inline
            .slice(0, inline.len_hint())
            .map(|row| row.kind)
            .collect::<Vec<_>>();
        let split_visual = (0..split.len_hint())
            .map(|ix| split.visual_kind(ix).expect("split visual kind"))
            .collect::<Vec<_>>();
        let inline_visual = (0..inline.len_hint())
            .map(|ix| inline.visual_kind(ix).expect("inline visual kind"))
            .collect::<Vec<_>>();

        assert_eq!(
            split_raw,
            vec![gitcomet_core::file_diff::FileDiffRowKind::Modify]
        );
        assert_eq!(
            inline_raw,
            vec![
                gitcomet_core::domain::DiffLineKind::Remove,
                gitcomet_core::domain::DiffLineKind::Add
            ]
        );
        assert_eq!(
            split_visual,
            vec![gitcomet_core::file_diff::FileDiffRowKind::Context]
        );
        assert_eq!(
            inline_visual,
            vec![
                gitcomet_core::domain::DiffLineKind::Context,
                gitcomet_core::domain::DiffLineKind::Context
            ]
        );
        assert!(split.change_visible_indices().is_empty());
        assert!(inline.change_visible_indices().is_empty());
        assert!(split.scrollbar_markers().is_empty());
        assert!(inline.scrollbar_markers().is_empty());
    }

    #[test]
    fn file_diff_visual_kinds_ignore_line_break_only_changes() {
        let source = streamed_file_diff_source_for_test_with_mode(
            "foo\nbar\n",
            "foobar\n",
            DiffWhitespaceMode::Ignore,
        );
        let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
        let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

        let split_visual = (0..split.len_hint())
            .map(|ix| split.visual_kind(ix).expect("split visual kind"))
            .collect::<Vec<_>>();
        let inline_visual = (0..inline.len_hint())
            .map(|ix| inline.visual_kind(ix).expect("inline visual kind"))
            .collect::<Vec<_>>();

        assert!(!split_visual.is_empty());
        assert!(!inline_visual.is_empty());
        assert!(
            split_visual
                .iter()
                .all(|kind| *kind == gitcomet_core::file_diff::FileDiffRowKind::Context)
        );
        assert!(
            inline_visual
                .iter()
                .all(|kind| *kind == gitcomet_core::domain::DiffLineKind::Context)
        );
        assert!(split.change_visible_indices().is_empty());
        assert!(inline.change_visible_indices().is_empty());
    }

    #[test]
    fn file_diff_change_visible_indices_include_every_changed_row() {
        let source = streamed_file_diff_source_for_test(
            "alpha\nold one\nold two\nold three\nomega\n",
            "alpha\nnew one\nnew two\nnew three\nomega\n",
        );
        let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
        let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

        assert_eq!(split.change_visible_indices(), vec![1, 2, 3]);
        assert_eq!(inline.change_visible_indices(), vec![1, 2, 3, 4, 5, 6]);
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

    /// A side backed by a file on disk -- an unstaged diff's new side is the
    /// worktree file -- keeps its text off the heap, which is what lets a huge
    /// diff render from per-line slices. The rebuild must therefore hand back
    /// the *path*, or nothing can ever parse a whole document for that side.
    ///
    /// Without it `file_diff_old_text`/`new_text` are empty, every prepared
    /// syntax document lookup returns `None`, and anything needing the whole
    /// tree (matching delimiters) silently gets nothing while row colouring
    /// carries on from per-line tokens.
    #[test]
    fn file_backed_diff_sides_report_their_source_path() {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet_file_diff_syntax_text_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        let new_path = dir.join("lib.rs");
        let new_body = "fn main() {\n    let v = vec![1, 2];\n}\n";
        std::fs::write(&new_path, new_body).expect("write new side");

        let file = gitcomet_core::domain::FileDiffText::new_sources(
            std::path::PathBuf::from("lib.rs"),
            None,
            Some(gitcomet_core::domain::FileDiffTextSource::new(
                new_path.clone(),
            )),
        );
        let rebuild = build_file_diff_cache_rebuild(&file, &dir)
            .expect("rebuild should succeed for a file-backed side");

        assert!(
            rebuild.new_text.is_empty(),
            "a file-backed side deliberately keeps its text off the heap"
        );
        assert_eq!(
            rebuild.new_source_path.as_deref(),
            Some(&new_path),
            "but it must report where that text can be read back from"
        );
        assert!(
            !rebuild.new_line_starts.is_empty(),
            "line starts are indexed even when the text is not held"
        );
        // And what the path points at is what the line starts describe.
        let body = std::fs::read_to_string(&new_path).expect("read back");
        assert_eq!(body, new_body);
        // Three lines plus the start following the trailing newline.
        assert_eq!(rebuild.new_line_starts.len(), 4);
        assert_eq!(rebuild.new_line_starts.first(), Some(&0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_diff_index_line_capacity_hint_is_bounded_for_massive_files() {
        assert_eq!(file_diff_index_line_capacity_hint(0), 1);
        assert_eq!(file_diff_index_line_capacity_hint(128), 3);
        assert_eq!(
            file_diff_index_line_capacity_hint(usize::MAX),
            FILE_DIFF_INDEX_LINE_CAPACITY_MAX
        );
    }

    /// The staleness guard the click path leans on, against the indexer that
    /// produced the index it is checking.
    ///
    /// Pinned against `IndexedFileDiffSource` itself rather than against a
    /// hand-written expectation: if the two ever disagree about the convention
    /// -- a start after a trailing newline, no start for an empty file -- every
    /// source-backed side would silently stop pairing instead of failing here.
    #[test]
    fn line_starts_describe_accepts_its_own_index_and_rejects_a_changed_file() {
        for text in [
            "alpha\nbeta\n",
            "alpha\nbeta",
            "\n",
            "one line no newline",
            "trailing\n\n\n",
        ] {
            let indexed = IndexedFileDiffSource::from_shared(Some(&Arc::from(text)));
            assert!(
                line_starts_describe(text, indexed.line_starts.as_ref()),
                "the indexer's own answer for {text:?} must be accepted",
            );
        }

        let indexed = IndexedFileDiffSource::from_shared(Some(&Arc::from("alpha\nbeta\ngamma\n")));
        let starts = indexed.line_starts.as_ref();
        // The file shrank: the index's later starts sit past the end of the
        // text, which is exactly the pairing that used to panic downstream.
        assert!(!line_starts_describe("alpha\n", starts));
        // The file grew, or a line moved: same length class, different lines.
        assert!(!line_starts_describe("alpha\nbeta\ngamma\ndelta\n", starts));
        assert!(!line_starts_describe("alphaX\nbeta\ngamma\n", starts));
        // An empty read against a real index is not a match either.
        assert!(!line_starts_describe("", starts));
    }

    #[test]
    fn build_file_diff_cache_rebuild_preserves_real_document_sources() {
        let file = gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("src/demo.rs"),
            Some("alpha\nbeta\n".to_string()),
            Some("gamma\ndelta".to_string()),
        );

        let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"))
            .expect("shared file diff should index");

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
    fn build_file_diff_cache_rebuild_reports_invalid_source_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source_path = tmp.path().join("binary.dat");
        std::fs::write(&source_path, [0xff, 0xfe, b'\n']).expect("write invalid utf8 source");
        let file = gitcomet_core::domain::FileDiffText::new_sources(
            PathBuf::from("binary.dat"),
            None,
            Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                source_path.clone(),
                "invalid-source",
            )),
        );

        let error = build_file_diff_cache_rebuild(&file, tmp.path())
            .expect_err("invalid UTF-8 source should be reported");

        assert!(error.contains(source_path.to_string_lossy().as_ref()));
        assert!(error.contains("not valid UTF-8"));
    }

    #[test]
    fn indexed_file_diff_source_accepts_utf8_split_at_scan_boundary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source_path = tmp.path().join("split-utf8.txt");
        let mut bytes = vec![b'a'; FILE_DIFF_INDEX_SCAN_BUFFER_BYTES.saturating_sub(1)];
        bytes.extend_from_slice("é".as_bytes());
        bytes.push(b'\n');
        std::fs::write(&source_path, bytes).expect("write split utf8 source");

        let source = IndexedFileDiffSource::from_file(&source_path)
            .expect("valid UTF-8 split across scan buffers should index");

        assert_eq!(source.line_count(), 1);
        assert_eq!(
            source.source_len,
            FILE_DIFF_INDEX_SCAN_BUFFER_BYTES.saturating_add(2)
        );
    }

    #[test]
    fn build_file_diff_cache_rebuild_source_backed_without_patch_uses_source_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let old_source_path = tmp.path().join("old.txt");
        let new_source_path = tmp.path().join("new.txt");
        std::fs::write(&old_source_path, "alpha\nbefore\ngamma\n").expect("write old source");
        std::fs::write(&new_source_path, "alpha\nafter\ngamma\n").expect("write new source");
        let file = gitcomet_core::domain::FileDiffText::new_sources(
            PathBuf::from("demo.txt"),
            Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                old_source_path,
                "old",
            )),
            Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                new_source_path,
                "new",
            )),
        );

        let rebuild = build_file_diff_cache_rebuild(&file, tmp.path())
            .expect("source-backed file diff should build without patch");

        assert!(rebuild.rows.iter().any(|row| {
            row.kind != gitcomet_core::file_diff::FileDiffRowKind::Context
                && (row.old_line == Some(2) || row.new_line == Some(2))
        }));
        assert!(!rebuild.rows.iter().any(|row| {
            row.kind == gitcomet_core::file_diff::FileDiffRowKind::Context
                && row.old_line == Some(2)
                && row.new_line == Some(2)
        }));
    }

    #[test]
    fn build_file_diff_cache_rebuild_inline_rows_keep_file_line_numbers() {
        use gitcomet_core::domain::DiffLineKind;

        let file = gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("src/demo.rs"),
            Some("struct Old;\nfn keep() {}\n".to_string()),
            Some("fn keep() {}\nlet added = 42;\n".to_string()),
        );

        let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"))
            .expect("shared file diff should index");
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
