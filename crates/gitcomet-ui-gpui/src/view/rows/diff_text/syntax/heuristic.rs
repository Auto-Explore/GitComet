use super::*;

const STREAMED_HEURISTIC_LINE_CACHE_MAX_ENTRIES: usize = 16;
const STREAMED_HEURISTIC_CHECKPOINT_SPACING_BYTES: usize = 32 * 1024;
const STREAMED_HEURISTIC_SCAN_CHUNK_BYTES: usize = 256 * 1024;
const STREAMED_HEURISTIC_SCAN_CHUNK_LOOKAHEAD_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum HeuristicBlockCommentKind {
    Html,
    FSharp,
    Lua,
    C,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum HeuristicOpenState {
    #[default]
    Normal,
    String {
        quote: u8,
        escaped: bool,
    },
    LineComment,
    BlockComment {
        kind: HeuristicBlockCommentKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StreamedHeuristicLineCacheKey {
    language: DiffSyntaxLanguage,
    line_identity_hash: u64,
    line_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StreamedHeuristicCheckpoint {
    offset: usize,
    state: HeuristicOpenState,
}

#[derive(Clone, Debug)]
struct StreamedHeuristicLineStateCache {
    checkpoints: Vec<StreamedHeuristicCheckpoint>,
    next_checkpoint_offset: usize,
    scanned_to: usize,
    tail_state: HeuristicOpenState,
}

impl Default for StreamedHeuristicLineStateCache {
    fn default() -> Self {
        Self {
            checkpoints: vec![StreamedHeuristicCheckpoint {
                offset: 0,
                state: HeuristicOpenState::Normal,
            }],
            next_checkpoint_offset: STREAMED_HEURISTIC_CHECKPOINT_SPACING_BYTES,
            scanned_to: 0,
            tail_state: HeuristicOpenState::Normal,
        }
    }
}

struct HeuristicCheckpointRecorder<'a> {
    next_offset: usize,
    unsafe_until: usize,
    checkpoints: &'a mut Vec<StreamedHeuristicCheckpoint>,
}

impl<'a> HeuristicCheckpointRecorder<'a> {
    fn new(next_offset: usize, checkpoints: &'a mut Vec<StreamedHeuristicCheckpoint>) -> Self {
        Self {
            next_offset,
            unsafe_until: 0,
            checkpoints,
        }
    }

    fn defer_until(&mut self, offset: usize) {
        self.unsafe_until = self.unsafe_until.max(offset);
    }

    fn record_constant_state_until(&mut self, end_abs: usize, state: HeuristicOpenState) {
        while self.next_offset <= end_abs {
            if self.next_offset < self.unsafe_until {
                self.next_offset = self.unsafe_until;
                continue;
            }
            self.checkpoints.push(StreamedHeuristicCheckpoint {
                offset: self.next_offset,
                state,
            });
            self.next_offset = self
                .next_offset
                .saturating_add(STREAMED_HEURISTIC_CHECKPOINT_SPACING_BYTES);
        }
    }
}

thread_local! {
    static STREAMED_HEURISTIC_LINE_CACHE: RefCell<
        FxLruCache<StreamedHeuristicLineCacheKey, StreamedHeuristicLineStateCache>,
    > = RefCell::new(new_fx_lru_cache(STREAMED_HEURISTIC_LINE_CACHE_MAX_ENTRIES));
}

#[derive(Clone, Copy)]
pub(super) struct HeuristicBlockCommentSpec {
    pub(super) start: &'static str,
    pub(super) end: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct HeuristicCommentConfig {
    pub(super) line_comment: Option<&'static str>,
    pub(super) hash_comment: bool,
    pub(super) block_comment: Option<HeuristicBlockCommentSpec>,
    pub(super) visual_basic_line_comment: bool,
}

#[derive(Clone, Copy)]
struct HeuristicOpenStateScanConfig {
    comment: HeuristicCommentConfig,
    block_comment_kind: Option<HeuristicBlockCommentKind>,
    allow_backtick_strings: bool,
}

impl HeuristicOpenStateScanConfig {
    fn for_language(language: DiffSyntaxLanguage) -> Self {
        Self {
            comment: heuristic_comment_config(language),
            block_comment_kind: heuristic_block_comment_kind(language),
            allow_backtick_strings: heuristic_allows_backtick_strings(language),
        }
    }
}

const HEURISTIC_HTML_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "<!--",
    end: "-->",
};
const HEURISTIC_FSHARP_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "(*",
    end: "*)",
};
const HEURISTIC_LUA_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "--[[",
    end: "]]",
};
const HEURISTIC_C_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "/*",
    end: "*/",
};

pub(super) fn heuristic_comment_config(language: DiffSyntaxLanguage) -> HeuristicCommentConfig {
    match language {
        DiffSyntaxLanguage::Html | DiffSyntaxLanguage::Xml => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: Some(HEURISTIC_HTML_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::FSharp => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: Some(HEURISTIC_FSHARP_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Lua => HeuristicCommentConfig {
            line_comment: Some("--"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_LUA_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Python
        | DiffSyntaxLanguage::Toml
        | DiffSyntaxLanguage::Yaml
        | DiffSyntaxLanguage::Bash
        | DiffSyntaxLanguage::Makefile
        | DiffSyntaxLanguage::Ruby => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: true,
            block_comment: None,
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Sql => HeuristicCommentConfig {
            line_comment: Some("--"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Rust
        | DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx
        | DiffSyntaxLanguage::Go
        | DiffSyntaxLanguage::C
        | DiffSyntaxLanguage::Cpp
        | DiffSyntaxLanguage::CSharp
        | DiffSyntaxLanguage::Java
        | DiffSyntaxLanguage::Kotlin
        | DiffSyntaxLanguage::Zig
        | DiffSyntaxLanguage::Bicep => HeuristicCommentConfig {
            line_comment: Some("//"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Hcl | DiffSyntaxLanguage::Php => HeuristicCommentConfig {
            line_comment: Some("//"),
            hash_comment: true,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::VisualBasic => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: None,
            visual_basic_line_comment: true,
        },
        DiffSyntaxLanguage::Markdown | DiffSyntaxLanguage::Css | DiffSyntaxLanguage::Json => {
            HeuristicCommentConfig {
                line_comment: None,
                hash_comment: false,
                block_comment: None,
                visual_basic_line_comment: false,
            }
        }
    }
}

fn heuristic_block_comment_kind(language: DiffSyntaxLanguage) -> Option<HeuristicBlockCommentKind> {
    match language {
        DiffSyntaxLanguage::Html | DiffSyntaxLanguage::Xml => Some(HeuristicBlockCommentKind::Html),
        DiffSyntaxLanguage::FSharp => Some(HeuristicBlockCommentKind::FSharp),
        DiffSyntaxLanguage::Lua => Some(HeuristicBlockCommentKind::Lua),
        DiffSyntaxLanguage::Sql
        | DiffSyntaxLanguage::Rust
        | DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx
        | DiffSyntaxLanguage::Go
        | DiffSyntaxLanguage::C
        | DiffSyntaxLanguage::Cpp
        | DiffSyntaxLanguage::CSharp
        | DiffSyntaxLanguage::Java
        | DiffSyntaxLanguage::Kotlin
        | DiffSyntaxLanguage::Zig
        | DiffSyntaxLanguage::Bicep
        | DiffSyntaxLanguage::Hcl
        | DiffSyntaxLanguage::Php => Some(HeuristicBlockCommentKind::C),
        _ => None,
    }
}

fn heuristic_block_comment_start_bytes(kind: HeuristicBlockCommentKind) -> &'static [u8] {
    match kind {
        HeuristicBlockCommentKind::Html => b"<!--",
        HeuristicBlockCommentKind::FSharp => b"(*",
        HeuristicBlockCommentKind::Lua => b"--[[",
        HeuristicBlockCommentKind::C => b"/*",
    }
}

fn heuristic_block_comment_end_bytes(kind: HeuristicBlockCommentKind) -> &'static [u8] {
    match kind {
        HeuristicBlockCommentKind::Html => b"-->",
        HeuristicBlockCommentKind::FSharp => b"*)",
        HeuristicBlockCommentKind::Lua => b"]]",
        HeuristicBlockCommentKind::C => b"*/",
    }
}

fn matches_ascii_bytes_at(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    start
        .checked_add(needle.len())
        .and_then(|end| bytes.get(start..end))
        .is_some_and(|candidate| candidate == needle)
}

fn visual_basic_rem_comment_prefix_len(bytes: &[u8], start: usize) -> Option<usize> {
    let prefix = bytes.get(start..start.saturating_add(4))?;
    let rem = prefix.get(..3)?;
    if rem.eq_ignore_ascii_case(b"rem") && prefix[3].is_ascii_whitespace() {
        Some(4)
    } else {
        None
    }
}

fn visual_basic_line_comment_start_len(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) == Some(&b'\'') {
        return Some(1);
    }
    visual_basic_rem_comment_prefix_len(bytes, start)
}

fn line_comment_start_len(
    bytes: &[u8],
    start: usize,
    config: HeuristicCommentConfig,
) -> Option<usize> {
    if config.visual_basic_line_comment {
        return visual_basic_line_comment_start_len(bytes, start);
    }
    if let Some(prefix) = config.line_comment
        && matches_ascii_bytes_at(bytes, start, prefix.as_bytes())
    {
        return Some(prefix.len());
    }
    if config.hash_comment && bytes.get(start) == Some(&b'#') {
        return Some(1);
    }
    None
}

fn comment_prefix_state_at_ascii(
    config: HeuristicOpenStateScanConfig,
    bytes: &[u8],
    start: usize,
) -> Option<(usize, HeuristicOpenState)> {
    if let Some(kind) = config.block_comment_kind {
        let start_bytes = heuristic_block_comment_start_bytes(kind);
        if matches_ascii_bytes_at(bytes, start, start_bytes) {
            return Some((start_bytes.len(), HeuristicOpenState::BlockComment { kind }));
        }
    }
    line_comment_start_len(bytes, start, config.comment)
        .map(|len| (len, HeuristicOpenState::LineComment))
}

fn potential_open_state_lead(config: HeuristicOpenStateScanConfig, byte: u8) -> bool {
    if matches!(byte, b'"' | b'\'') || (config.allow_backtick_strings && byte == b'`') {
        return true;
    }
    if config.comment.hash_comment && byte == b'#' {
        return true;
    }
    if config
        .comment
        .line_comment
        .is_some_and(|prefix| prefix.as_bytes().first().copied() == Some(byte))
    {
        return true;
    }
    if config.block_comment_kind.is_some_and(|kind| {
        heuristic_block_comment_start_bytes(kind).first().copied() == Some(byte)
    }) {
        return true;
    }
    config.comment.visual_basic_line_comment && (byte == b'\'' || byte.eq_ignore_ascii_case(&b'r'))
}

fn open_state_token_kind(state: HeuristicOpenState) -> Option<SyntaxTokenKind> {
    match state {
        HeuristicOpenState::Normal => None,
        HeuristicOpenState::String { .. } => Some(SyntaxTokenKind::String),
        HeuristicOpenState::LineComment | HeuristicOpenState::BlockComment { .. } => {
            Some(SyntaxTokenKind::Comment)
        }
    }
}

fn push_relative_token_if_intersects(
    tokens: &mut Vec<SyntaxToken>,
    token_start: usize,
    token_end: usize,
    visible_range: &Range<usize>,
    kind: SyntaxTokenKind,
) {
    let start = token_start.max(visible_range.start);
    let end = token_end.min(visible_range.end);
    if start < end {
        tokens.push(SyntaxToken {
            range: start.saturating_sub(visible_range.start)
                ..end.saturating_sub(visible_range.start),
            kind,
        });
    }
}

fn emit_segment_tokens_relative(
    tokens: &mut Vec<SyntaxToken>,
    segment_text: &str,
    segment_start: usize,
    visible_range: &Range<usize>,
    language: DiffSyntaxLanguage,
) {
    if segment_text.is_empty() {
        return;
    }

    let mut segment_tokens = Vec::new();
    syntax_tokens_for_line_heuristic_into(segment_text, language, &mut segment_tokens);
    for token in segment_tokens {
        push_relative_token_if_intersects(
            tokens,
            segment_start.saturating_add(token.range.start),
            segment_start.saturating_add(token.range.end),
            visible_range,
            token.kind,
        );
    }
}

fn consume_open_state_ascii_prefix(
    segment_text: &str,
    state: HeuristicOpenState,
) -> (usize, HeuristicOpenState) {
    let bytes = segment_text.as_bytes();
    match state {
        HeuristicOpenState::Normal => (0, HeuristicOpenState::Normal),
        HeuristicOpenState::LineComment => (bytes.len(), HeuristicOpenState::LineComment),
        HeuristicOpenState::String { quote, mut escaped } => {
            let mut ix = 0usize;
            while ix < bytes.len() {
                if escaped {
                    escaped = false;
                    ix += 1;
                    continue;
                }
                match bytes[ix] {
                    b'\\' => {
                        escaped = true;
                        ix += 1;
                    }
                    byte if byte == quote => {
                        return (ix + 1, HeuristicOpenState::Normal);
                    }
                    _ => ix += 1,
                }
            }
            (bytes.len(), HeuristicOpenState::String { quote, escaped })
        }
        HeuristicOpenState::BlockComment { kind } => {
            let end_bytes = heuristic_block_comment_end_bytes(kind);
            let end_first = end_bytes[0];
            let mut ix = 0usize;
            while ix < bytes.len() {
                let remaining = &bytes[ix..];
                if let Some(found) = memchr::memchr(end_first, remaining) {
                    ix += found;
                    if matches_ascii_bytes_at(bytes, ix, end_bytes) {
                        return (ix + end_bytes.len(), HeuristicOpenState::Normal);
                    }
                    ix += 1;
                } else {
                    break;
                }
            }
            (bytes.len(), HeuristicOpenState::BlockComment { kind })
        }
    }
}

fn advance_streamed_heuristic_open_state_ascii(
    bytes: &[u8],
    process_len: usize,
    language: DiffSyntaxLanguage,
    state: &mut HeuristicOpenState,
    abs: &mut usize,
    recorder: &mut Option<HeuristicCheckpointRecorder<'_>>,
) {
    macro_rules! record_constant {
        ($end_abs:expr, $state:expr) => {
            if let Some(recorder) = recorder.as_mut() {
                recorder.record_constant_state_until($end_abs, $state);
            }
        };
    }

    macro_rules! defer_until {
        ($offset:expr) => {
            if let Some(recorder) = recorder.as_mut() {
                recorder.defer_until($offset);
            }
        };
    }

    // This scanner only tracks ASCII delimiter bytes that can open or close the
    // heuristic string/comment states. UTF-8 continuation bytes are opaque data,
    // so scanning raw bytes here is sufficient for restoring the correct state.
    let scan_config = HeuristicOpenStateScanConfig::for_language(language);
    let mut local = 0usize;

    while local < process_len {
        match *state {
            HeuristicOpenState::Normal => {
                let run_start = local;
                while local < process_len && !potential_open_state_lead(scan_config, bytes[local]) {
                    local += 1;
                }
                if local > run_start {
                    *abs = abs.saturating_add(local.saturating_sub(run_start));
                    record_constant!(*abs, HeuristicOpenState::Normal);
                    continue;
                }

                if let Some((prefix_len, next_state)) =
                    comment_prefix_state_at_ascii(scan_config, bytes, local)
                {
                    if prefix_len > 1 {
                        defer_until!(abs.saturating_add(prefix_len));
                    }
                    for prefix_ix in 0..prefix_len {
                        local += 1;
                        *abs = abs.saturating_add(1);
                        *state = if prefix_ix + 1 == prefix_len {
                            next_state
                        } else {
                            HeuristicOpenState::Normal
                        };
                        record_constant!(*abs, *state);
                    }
                    continue;
                }

                let byte = bytes[local];
                if matches!(byte, b'"' | b'\'')
                    || (scan_config.allow_backtick_strings && byte == b'`')
                {
                    local += 1;
                    *abs = abs.saturating_add(1);
                    *state = HeuristicOpenState::String {
                        quote: byte,
                        escaped: false,
                    };
                    record_constant!(*abs, *state);
                    continue;
                }

                local += 1;
                *abs = abs.saturating_add(1);
                record_constant!(*abs, HeuristicOpenState::Normal);
            }
            HeuristicOpenState::LineComment => {
                let remaining = process_len.saturating_sub(local);
                local = process_len;
                *abs = abs.saturating_add(remaining);
                record_constant!(*abs, HeuristicOpenState::LineComment);
            }
            HeuristicOpenState::String {
                quote,
                escaped: true,
            } => {
                local += 1;
                *abs = abs.saturating_add(1);
                *state = HeuristicOpenState::String {
                    quote,
                    escaped: false,
                };
                record_constant!(*abs, *state);
            }
            HeuristicOpenState::String {
                quote,
                escaped: false,
            } => {
                let remainder = &bytes[local..process_len];
                let Some(found) = memchr::memchr2(quote, b'\\', remainder) else {
                    let remaining = process_len.saturating_sub(local);
                    local = process_len;
                    *abs = abs.saturating_add(remaining);
                    record_constant!(*abs, *state);
                    continue;
                };
                if found > 0 {
                    local = local.saturating_add(found);
                    *abs = abs.saturating_add(found);
                    record_constant!(*abs, *state);
                }
                let byte = bytes[local];
                local += 1;
                *abs = abs.saturating_add(1);
                *state = if byte == b'\\' {
                    HeuristicOpenState::String {
                        quote,
                        escaped: true,
                    }
                } else {
                    HeuristicOpenState::Normal
                };
                record_constant!(*abs, *state);
            }
            HeuristicOpenState::BlockComment { kind } => {
                let end_bytes = heuristic_block_comment_end_bytes(kind);
                let end_first = end_bytes[0];
                let remainder = &bytes[local..process_len];
                let Some(found) = memchr::memchr(end_first, remainder) else {
                    let remaining = process_len.saturating_sub(local);
                    local = process_len;
                    *abs = abs.saturating_add(remaining);
                    record_constant!(*abs, *state);
                    continue;
                };
                if found > 0 {
                    local = local.saturating_add(found);
                    *abs = abs.saturating_add(found);
                    record_constant!(*abs, *state);
                }
                if matches_ascii_bytes_at(bytes, local, end_bytes) {
                    if end_bytes.len() > 1 {
                        defer_until!(abs.saturating_add(end_bytes.len()));
                    }
                    for end_ix in 0..end_bytes.len() {
                        local += 1;
                        *abs = abs.saturating_add(1);
                        *state = if end_ix + 1 == end_bytes.len() {
                            HeuristicOpenState::Normal
                        } else {
                            HeuristicOpenState::BlockComment { kind }
                        };
                        record_constant!(*abs, *state);
                    }
                    continue;
                }
                local += 1;
                *abs = abs.saturating_add(1);
                record_constant!(*abs, *state);
            }
        }
    }
}

fn load_streamed_heuristic_line_segment_bytes(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    start: usize,
    process_end: usize,
) -> Option<Cow<'_, [u8]>> {
    let read_end = process_end
        .saturating_add(STREAMED_HEURISTIC_SCAN_CHUNK_LOOKAHEAD_BYTES)
        .min(raw_text.len());
    let bytes = raw_text.slice_bytes(start..read_end)?;
    (bytes.len() >= process_end.saturating_sub(start)).then_some(bytes)
}

fn ensure_streamed_heuristic_line_checkpoints(
    cache: &mut StreamedHeuristicLineStateCache,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
    target_offset: usize,
) -> bool {
    let target_offset = target_offset.min(raw_text.len());
    while cache.scanned_to < target_offset {
        let desired_end = cache
            .scanned_to
            .saturating_add(STREAMED_HEURISTIC_SCAN_CHUNK_BYTES)
            .min(target_offset);
        let chunk = match load_streamed_heuristic_line_segment_bytes(
            raw_text,
            cache.scanned_to,
            desired_end,
        ) {
            Some(chunk) => chunk,
            None => return false,
        };

        let mut abs = cache.scanned_to;
        let mut state = cache.tail_state;
        let mut recorder = Some(HeuristicCheckpointRecorder::new(
            cache.next_checkpoint_offset,
            &mut cache.checkpoints,
        ));
        advance_streamed_heuristic_open_state_ascii(
            chunk.as_ref(),
            desired_end.saturating_sub(cache.scanned_to),
            language,
            &mut state,
            &mut abs,
            &mut recorder,
        );
        cache.scanned_to = abs;
        cache.tail_state = state;
        if let Some(recorder) = recorder.take() {
            cache.next_checkpoint_offset = recorder.next_offset;
        }
    }
    true
}

fn streamed_heuristic_checkpoint_for_offset(
    checkpoints: &[StreamedHeuristicCheckpoint],
    offset: usize,
) -> StreamedHeuristicCheckpoint {
    checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.offset <= offset)
        .copied()
        .unwrap_or(StreamedHeuristicCheckpoint {
            offset: 0,
            state: HeuristicOpenState::Normal,
        })
}

fn streamed_heuristic_line_cache_key(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
) -> StreamedHeuristicLineCacheKey {
    StreamedHeuristicLineCacheKey {
        language,
        line_identity_hash: raw_text.identity_hash_without_loading(),
        line_len: raw_text.len(),
    }
}

pub(super) fn syntax_tokens_for_streamed_line_slice_heuristic(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
    requested_slice_range: Range<usize>,
    resolved_visible_range: Range<usize>,
) -> Option<Vec<SyntaxToken>> {
    let line_len = raw_text.len();
    let requested_visible_range =
        requested_slice_range.start.min(line_len)..requested_slice_range.end.min(line_len);
    if requested_visible_range.is_empty() {
        return Some(Vec::new());
    }

    let visible_range =
        resolved_visible_range.start.min(line_len)..resolved_visible_range.end.min(line_len);
    if visible_range.is_empty() {
        return Some(Vec::new());
    }

    let cache_key = streamed_heuristic_line_cache_key(raw_text, language);
    let checkpoint = STREAMED_HEURISTIC_LINE_CACHE.with(|cache| {
        let mut cache_ref = cache.borrow_mut();
        let mut entry = cache_ref
            .get(&cache_key)
            .cloned()
            .unwrap_or_else(StreamedHeuristicLineStateCache::default);
        let ready = ensure_streamed_heuristic_line_checkpoints(
            &mut entry,
            raw_text,
            language,
            requested_visible_range.start,
        );
        let checkpoint = ready.then(|| {
            streamed_heuristic_checkpoint_for_offset(&entry.checkpoints, visible_range.start)
        });
        cache_ref.put(cache_key, entry);
        checkpoint
    })?;

    let (segment_text, resolved_segment_range) =
        raw_text.slice_text_resolved(checkpoint.offset..visible_range.end)?;
    let mut tokens = Vec::new();
    match checkpoint.state {
        HeuristicOpenState::Normal => {
            emit_segment_tokens_relative(
                &mut tokens,
                segment_text.as_ref(),
                resolved_segment_range.start,
                &visible_range,
                language,
            );
        }
        open_state => {
            if let Some(kind) = open_state_token_kind(open_state) {
                let (open_prefix_len, next_state) =
                    consume_open_state_ascii_prefix(segment_text.as_ref(), open_state);
                push_relative_token_if_intersects(
                    &mut tokens,
                    resolved_segment_range.start,
                    resolved_segment_range.start.saturating_add(open_prefix_len),
                    &visible_range,
                    kind,
                );
                if next_state == HeuristicOpenState::Normal {
                    let tail_text = segment_text.get(open_prefix_len..).unwrap_or("");
                    emit_segment_tokens_relative(
                        &mut tokens,
                        tail_text,
                        resolved_segment_range.start.saturating_add(open_prefix_len),
                        &visible_range,
                        language,
                    );
                }
            }
        }
    }

    Some(tokens)
}

#[cfg(test)]
pub(super) fn reset_streamed_heuristic_line_cache() {
    STREAMED_HEURISTIC_LINE_CACHE.with(|cache| {
        *cache.borrow_mut() = new_fx_lru_cache(STREAMED_HEURISTIC_LINE_CACHE_MAX_ENTRIES);
    });
}

fn heuristic_comment_range(
    text: &str,
    start: usize,
    config: HeuristicCommentConfig,
) -> Option<std::ops::Range<usize>> {
    let rest = &text[start..];

    if let Some(block) = config.block_comment
        && rest.starts_with(block.start)
    {
        let end = rest
            .find(block.end)
            .map(|ix| start + ix + block.end.len())
            .unwrap_or(text.len());
        return Some(start..end);
    }

    if let Some(prefix) = config.line_comment
        && rest.starts_with(prefix)
    {
        return Some(start..text.len());
    }

    if config.visual_basic_line_comment
        && (rest.starts_with('\'')
            || rest
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rem ")))
    {
        return Some(start..text.len());
    }

    if config.hash_comment && rest.starts_with('#') {
        return Some(start..text.len());
    }

    None
}

fn heuristic_string_end(text: &str, start: usize, quote: char) -> usize {
    let len = text.len();
    let mut i = start + quote.len_utf8();
    let mut escaped = false;

    while i < len {
        let Some(next) = text[i..].chars().next() else {
            break;
        };
        let next_len = next.len_utf8();
        if escaped {
            escaped = false;
            i += next_len;
            continue;
        }
        if next == '\\' {
            escaped = true;
            i += next_len;
            continue;
        }
        if next == quote {
            i += next_len;
            break;
        }
        i += next_len;
    }

    i.min(len)
}

fn heuristic_allows_backtick_strings(language: DiffSyntaxLanguage) -> bool {
    matches!(
        language,
        DiffSyntaxLanguage::JavaScript
            | DiffSyntaxLanguage::TypeScript
            | DiffSyntaxLanguage::Tsx
            | DiffSyntaxLanguage::Go
            | DiffSyntaxLanguage::Bash
            | DiffSyntaxLanguage::Sql
    )
}

fn yaml_heuristic_key_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
    {
        end += 1;
    }
    (end > start && bytes.get(end) == Some(&b':')).then_some(end)
}

fn yaml_heuristic_key_context(bytes: &[u8], key_start: usize) -> bool {
    let mut seen_dash = false;
    for &byte in &bytes[..key_start] {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if !seen_dash && byte == b'-' {
            seen_dash = true;
            continue;
        }
        return false;
    }
    true
}

fn yaml_heuristic_value_start(bytes: &[u8], colon_ix: usize) -> usize {
    let mut start = colon_ix.saturating_add(1);
    while bytes
        .get(start)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        start += 1;
    }
    start
}

fn yaml_heuristic_value_end(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return start;
    }

    let mut end = bytes.len();
    while end > start && bytes[end.saturating_sub(1)].is_ascii_whitespace() {
        end = end.saturating_sub(1);
    }

    let mut ix = start;
    while ix < end {
        if bytes[ix] == b'#' && (ix == start || bytes[ix.saturating_sub(1)].is_ascii_whitespace()) {
            let mut comment_start = ix;
            while comment_start > start
                && bytes[comment_start.saturating_sub(1)].is_ascii_whitespace()
            {
                comment_start = comment_start.saturating_sub(1);
            }
            return comment_start;
        }
        ix += 1;
    }

    end
}

fn yaml_heuristic_is_plain_boolean(text: &str) -> bool {
    matches!(
        text,
        "true" | "false" | "yes" | "no" | "on" | "off" | "True" | "False" | "TRUE" | "FALSE"
    )
}

fn yaml_heuristic_is_plain_null(text: &str) -> bool {
    matches!(text, "null" | "Null" | "NULL" | "~")
}

fn yaml_heuristic_is_plain_number(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    let mut ix = 0usize;
    if matches!(bytes[0], b'+' | b'-') {
        ix += 1;
    }
    let mut saw_digit = false;
    while ix < bytes.len() {
        match bytes[ix] {
            b'0'..=b'9' => {
                saw_digit = true;
                ix += 1;
            }
            b'_' | b'.' => {
                ix += 1;
            }
            b'e' | b'E' => {
                ix += 1;
                if matches!(bytes.get(ix), Some(b'+') | Some(b'-')) {
                    ix += 1;
                }
            }
            _ => return false,
        }
    }
    saw_digit
}

fn yaml_heuristic_emit_mapping_value_tokens(
    text: &str,
    bytes: &[u8],
    colon_ix: usize,
    allow_backtick_strings: bool,
    tokens: &mut Vec<SyntaxToken>,
) -> usize {
    let value_start = yaml_heuristic_value_start(bytes, colon_ix);
    if value_start >= bytes.len() {
        return colon_ix.saturating_add(1);
    }

    let value_end = yaml_heuristic_value_end(bytes, value_start);
    if value_start >= value_end {
        return value_end.max(colon_ix.saturating_add(1));
    }

    let value_bytes = &bytes[value_start..value_end];
    match value_bytes.first().copied() {
        Some(b'"' | b'\'') => {
            let quote = value_bytes[0] as char;
            let string_end = heuristic_string_end(text, value_start, quote);
            tokens.push(SyntaxToken {
                range: value_start..string_end,
                kind: SyntaxTokenKind::String,
            });
            string_end
        }
        Some(b'`') if allow_backtick_strings => {
            let string_end = heuristic_string_end(text, value_start, '`');
            tokens.push(SyntaxToken {
                range: value_start..string_end,
                kind: SyntaxTokenKind::String,
            });
            string_end
        }
        Some(b'|' | b'>') => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Punctuation,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_boolean(&text[value_start..value_end]) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Boolean,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_null(&text[value_start..value_end]) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Constant,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_number(value_bytes) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Number,
            });
            value_end
        }
        Some(_) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::String,
            });
            value_end
        }
        None => value_end,
    }
}

pub(super) fn syntax_tokens_for_line_heuristic(
    text: &str,
    language: DiffSyntaxLanguage,
) -> Vec<SyntaxToken> {
    let mut tokens: Vec<SyntaxToken> = Vec::new();
    syntax_tokens_for_line_heuristic_into(text, language, &mut tokens);
    tokens
}

pub(in super::super) fn syntax_tokens_for_line_heuristic_into(
    text: &str,
    language: DiffSyntaxLanguage,
    tokens: &mut Vec<SyntaxToken>,
) {
    tokens.clear();
    let bytes = text.as_bytes();
    let len = text.len();
    let mut i = 0usize;
    let comment_config = heuristic_comment_config(language);
    let allow_backtick_strings = heuristic_allows_backtick_strings(language);
    let highlight_css_selectors = matches!(language, DiffSyntaxLanguage::Css);

    let is_ident_start = |byte: u8| byte == b'_' || byte.is_ascii_alphabetic();
    let is_ident_continue = |byte: u8| byte == b'_' || byte.is_ascii_alphanumeric();
    let is_comment_lead = |byte: u8| {
        comment_config
            .line_comment
            .is_some_and(|prefix| prefix.as_bytes().first().copied() == Some(byte))
            || comment_config
                .block_comment
                .is_some_and(|block| block.start.as_bytes().first().copied() == Some(byte))
            || (comment_config.hash_comment && byte == b'#')
            || (comment_config.visual_basic_line_comment
                && (byte == b'\'' || byte.eq_ignore_ascii_case(&b'r')))
    };

    while i < len {
        let byte = bytes[i];

        if matches!(language, DiffSyntaxLanguage::Yaml) {
            if byte == b'-'
                && bytes[..i]
                    .iter()
                    .all(|candidate| candidate.is_ascii_whitespace())
                && bytes
                    .get(i.saturating_add(1))
                    .is_some_and(|next| next.is_ascii_whitespace())
            {
                tokens.push(SyntaxToken {
                    range: i..i.saturating_add(1),
                    kind: SyntaxTokenKind::Punctuation,
                });
                i = i.saturating_add(1);
                continue;
            }

            if yaml_heuristic_key_context(bytes, i)
                && let Some(key_end) = yaml_heuristic_key_end(bytes, i)
            {
                tokens.push(SyntaxToken {
                    range: i..key_end,
                    kind: SyntaxTokenKind::Property,
                });
                tokens.push(SyntaxToken {
                    range: key_end..key_end.saturating_add(1),
                    kind: SyntaxTokenKind::Punctuation,
                });
                i = yaml_heuristic_emit_mapping_value_tokens(
                    text,
                    bytes,
                    key_end,
                    allow_backtick_strings,
                    tokens,
                );
                continue;
            }
        }

        if is_comment_lead(byte)
            && let Some(comment_range) = heuristic_comment_range(text, i, comment_config)
        {
            tokens.push(SyntaxToken {
                range: comment_range.clone(),
                kind: SyntaxTokenKind::Comment,
            });
            i = comment_range.end;
            if i >= len {
                break;
            }
            continue;
        }

        if matches!(byte, b'"' | b'\'') || (allow_backtick_strings && byte == b'`') {
            let j = heuristic_string_end(text, i, byte as char);
            tokens.push(SyntaxToken {
                range: i..j,
                kind: SyntaxTokenKind::String,
            });
            i = j;
            continue;
        }

        if byte.is_ascii_digit() {
            let mut j = i;
            while j < len {
                let next = bytes[j];
                if next.is_ascii_digit() || matches!(next, b'_' | b'.' | b'x' | b'b') {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > i {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Number,
                });
                i = j;
                continue;
            }
        }

        if is_ident_start(byte) {
            let mut j = i + 1;
            while j < len && is_ident_continue(bytes[j]) {
                j += 1;
            }
            let ident = &text[i..j];
            if is_keyword(language, ident) {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Keyword,
                });
            }
            i = j;
            continue;
        }

        if highlight_css_selectors && matches!(byte, b'.' | b'#') {
            let mut j = i + 1;
            while j < len && (is_ident_continue(bytes[j]) || bytes[j] == b'-') {
                j += 1;
            }
            if j > i + 1 {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Type,
                });
                i = j;
                continue;
            }
        }

        if byte.is_ascii() {
            i += 1;
        } else if let Some(ch) = text[i..].chars().next() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
}

fn is_keyword(language: DiffSyntaxLanguage, ident: &str) -> bool {
    // NOTE: This is a heuristic fallback when we don't want to use tree-sitter for a line.
    match language {
        DiffSyntaxLanguage::Markdown => false,
        DiffSyntaxLanguage::Html
        | DiffSyntaxLanguage::Xml
        | DiffSyntaxLanguage::Css
        | DiffSyntaxLanguage::Toml => matches!(ident, "true" | "false"),
        DiffSyntaxLanguage::Json | DiffSyntaxLanguage::Yaml => {
            matches!(ident, "true" | "false" | "null")
        }
        DiffSyntaxLanguage::Hcl => matches!(
            ident,
            "true" | "false" | "null" | "for" | "in" | "if" | "else" | "endif" | "endfor"
        ),
        DiffSyntaxLanguage::Bicep => matches!(
            ident,
            "param" | "var" | "resource" | "module" | "output" | "existing" | "true" | "false"
        ),
        DiffSyntaxLanguage::Lua => matches!(
            ident,
            "and"
                | "break"
                | "do"
                | "else"
                | "elseif"
                | "end"
                | "false"
                | "for"
                | "function"
                | "goto"
                | "if"
                | "in"
                | "local"
                | "nil"
                | "not"
                | "or"
                | "repeat"
                | "return"
                | "then"
                | "true"
                | "until"
                | "while"
        ),
        DiffSyntaxLanguage::Makefile => matches!(ident, "if" | "else" | "endif"),
        DiffSyntaxLanguage::Kotlin => matches!(
            ident,
            "as" | "break"
                | "class"
                | "continue"
                | "do"
                | "else"
                | "false"
                | "for"
                | "fun"
                | "if"
                | "in"
                | "interface"
                | "is"
                | "null"
                | "object"
                | "package"
                | "return"
                | "super"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typealias"
                | "val"
                | "var"
                | "when"
                | "while"
        ),
        DiffSyntaxLanguage::Zig => matches!(
            ident,
            "const"
                | "var"
                | "fn"
                | "pub"
                | "usingnamespace"
                | "test"
                | "if"
                | "else"
                | "while"
                | "for"
                | "switch"
                | "and"
                | "or"
                | "orelse"
                | "break"
                | "continue"
                | "return"
                | "try"
                | "catch"
                | "true"
                | "false"
                | "null"
        ),
        DiffSyntaxLanguage::Rust => matches!(
            ident,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "Self"
                | "self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        DiffSyntaxLanguage::Python => matches!(
            ident,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx => {
            matches!(
                ident,
                "break"
                    | "case"
                    | "catch"
                    | "class"
                    | "const"
                    | "continue"
                    | "debugger"
                    | "default"
                    | "delete"
                    | "do"
                    | "else"
                    | "export"
                    | "extends"
                    | "false"
                    | "finally"
                    | "for"
                    | "function"
                    | "if"
                    | "import"
                    | "in"
                    | "instanceof"
                    | "new"
                    | "null"
                    | "return"
                    | "super"
                    | "switch"
                    | "this"
                    | "throw"
                    | "true"
                    | "try"
                    | "typeof"
                    | "var"
                    | "void"
                    | "while"
                    | "with"
                    | "yield"
            )
        }
        DiffSyntaxLanguage::Go => matches!(
            ident,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ),
        DiffSyntaxLanguage::C | DiffSyntaxLanguage::Cpp | DiffSyntaxLanguage::CSharp => matches!(
            ident,
            "auto"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "for"
                | "goto"
                | "if"
                | "inline"
                | "new"
                | "nullptr"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "typename"
                | "union"
                | "using"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
        ),
        DiffSyntaxLanguage::FSharp => matches!(
            ident,
            "let"
                | "in"
                | "match"
                | "with"
                | "type"
                | "member"
                | "interface"
                | "abstract"
                | "override"
                | "true"
                | "false"
                | "null"
        ),
        DiffSyntaxLanguage::VisualBasic => matches!(
            ident,
            "Dim"
                | "As"
                | "If"
                | "Then"
                | "Else"
                | "End"
                | "For"
                | "Each"
                | "In"
                | "Next"
                | "While"
                | "Do"
                | "Loop"
                | "True"
                | "False"
                | "Nothing"
        ),
        DiffSyntaxLanguage::Java => matches!(
            ident,
            "abstract"
                | "assert"
                | "boolean"
                | "break"
                | "byte"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extends"
                | "final"
                | "finally"
                | "float"
                | "for"
                | "goto"
                | "if"
                | "implements"
                | "import"
                | "instanceof"
                | "int"
                | "interface"
                | "long"
                | "native"
                | "new"
                | "null"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "short"
                | "static"
                | "strictfp"
                | "super"
                | "switch"
                | "synchronized"
                | "this"
                | "throw"
                | "throws"
                | "transient"
                | "true"
                | "false"
                | "try"
                | "void"
                | "volatile"
                | "while"
        ),
        DiffSyntaxLanguage::Php => {
            let ident = ascii_lowercase_for_match(ident);
            matches!(
                ident.as_ref(),
                "function"
                    | "class"
                    | "public"
                    | "private"
                    | "protected"
                    | "static"
                    | "final"
                    | "abstract"
                    | "extends"
                    | "implements"
                    | "use"
                    | "namespace"
                    | "return"
                    | "if"
                    | "else"
                    | "elseif"
                    | "for"
                    | "foreach"
                    | "while"
                    | "do"
                    | "switch"
                    | "case"
                    | "default"
                    | "try"
                    | "catch"
                    | "finally"
                    | "throw"
                    | "new"
                    | "true"
                    | "false"
                    | "null"
            )
        }
        DiffSyntaxLanguage::Ruby => matches!(
            ident,
            "def"
                | "class"
                | "module"
                | "end"
                | "if"
                | "elsif"
                | "else"
                | "unless"
                | "case"
                | "when"
                | "while"
                | "until"
                | "for"
                | "in"
                | "do"
                | "break"
                | "next"
                | "redo"
                | "retry"
                | "return"
                | "yield"
                | "super"
                | "self"
                | "true"
                | "false"
                | "nil"
        ),
        DiffSyntaxLanguage::Sql => {
            let ident = ascii_lowercase_for_match(ident);
            matches!(
                ident.as_ref(),
                "add"
                    | "all"
                    | "alter"
                    | "and"
                    | "as"
                    | "asc"
                    | "begin"
                    | "between"
                    | "by"
                    | "case"
                    | "check"
                    | "column"
                    | "commit"
                    | "constraint"
                    | "create"
                    | "cross"
                    | "database"
                    | "default"
                    | "delete"
                    | "desc"
                    | "distinct"
                    | "drop"
                    | "else"
                    | "end"
                    | "exists"
                    | "false"
                    | "foreign"
                    | "from"
                    | "full"
                    | "group"
                    | "having"
                    | "if"
                    | "in"
                    | "index"
                    | "inner"
                    | "insert"
                    | "intersect"
                    | "into"
                    | "is"
                    | "join"
                    | "key"
                    | "left"
                    | "like"
                    | "limit"
                    | "materialized"
                    | "not"
                    | "null"
                    | "offset"
                    | "on"
                    | "or"
                    | "order"
                    | "outer"
                    | "primary"
                    | "references"
                    | "returning"
                    | "right"
                    | "rollback"
                    | "select"
                    | "set"
                    | "table"
                    | "then"
                    | "transaction"
                    | "true"
                    | "union"
                    | "unique"
                    | "update"
                    | "values"
                    | "view"
                    | "when"
                    | "where"
                    | "with"
            )
        }
        DiffSyntaxLanguage::Bash => matches!(
            ident,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "in"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "while"
                | "function"
                | "return"
                | "break"
                | "continue"
        ),
    }
}

pub(super) fn syntax_tokens_for_line_markdown(text: &str) -> Vec<SyntaxToken> {
    let len = text.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = text.trim_start_matches([' ', '\t']);
    let indent = len.saturating_sub(trimmed.len());

    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return vec![SyntaxToken {
            range: 0..len,
            kind: SyntaxTokenKind::Keyword,
        }];
    }

    if trimmed.starts_with('>') {
        return vec![SyntaxToken {
            range: indent..len,
            kind: SyntaxTokenKind::Comment,
        }];
    }

    // Headings: up to 6 leading `#` and a following space.
    let mut hashes = 0usize;
    for ch in trimmed.chars() {
        if ch == '#' && hashes < 6 {
            hashes += 1;
        } else {
            break;
        }
    }
    if hashes > 0 {
        let after_hashes = trimmed[hashes..].chars().next();
        if after_hashes.is_some_and(|c| c.is_whitespace()) {
            return vec![SyntaxToken {
                range: indent..len,
                kind: SyntaxTokenKind::Keyword,
            }];
        }
    }

    // Inline code: highlight backtick-delimited ranges.
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut tokens: Vec<SyntaxToken> = Vec::new();
    while i < len {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }

        let start = i;
        let mut tick_len = 0usize;
        while i < len && bytes[i] == b'`' {
            tick_len += 1;
            i += 1;
        }

        let mut j = i;
        while j < len {
            if bytes[j] != b'`' {
                j += 1;
                continue;
            }
            let mut run = 0usize;
            while j + run < len && bytes[j + run] == b'`' {
                run += 1;
            }
            if run == tick_len {
                let end = (j + run).min(len);
                if start < end {
                    tokens.push(SyntaxToken {
                        range: start..end,
                        kind: SyntaxTokenKind::String,
                    });
                }
                i = end;
                break;
            }
            j += run.max(1);
        }
        if j >= len {
            // Unterminated inline code; stop scanning to avoid odd highlighting.
            break;
        }
    }

    tokens
}
