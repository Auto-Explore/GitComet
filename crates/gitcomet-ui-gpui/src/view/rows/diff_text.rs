use super::super::perf::{self, ViewPerfSpan};
use super::*;
use gitcomet_core::domain::DiffLineKind;
use memchr::memchr2_iter;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

mod build;
mod prepared;
mod syntax;

use build::*;

pub(in crate::view) use build::PreparedDocumentByteRangeHighlights;
#[cfg(feature = "benchmarks")]
pub(super) use build::build_cached_diff_styled_text_with_palette;
pub(in crate::view) use build::syntax_highlights_for_line;
pub(super) use build::{
    build_cached_diff_query_overlay_styled_text, build_cached_diff_styled_text,
    build_cached_diff_styled_text_from_relative_highlights,
    build_cached_diff_styled_text_with_source_identity, diff_line_colors,
    selectable_cached_diff_text,
};
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use prepared::prepare_diff_syntax_document_in_background_text;
pub(super) use prepared::{
    PreparedDocumentLineStyledText,
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking,
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette,
};
#[cfg(feature = "benchmarks")]
#[allow(unused_imports)]
pub(in crate::view) use prepared::{
    benchmark_diff_syntax_cache_drop_payload_timed_step,
    benchmark_diff_syntax_cache_replacement_drop_step,
    benchmark_diff_syntax_prepared_cache_contains_document,
    benchmark_diff_syntax_prepared_cache_metrics,
    benchmark_diff_syntax_prepared_loaded_chunk_count,
    benchmark_flush_diff_syntax_deferred_drop_queue,
    benchmark_reset_diff_syntax_prepared_cache_metrics,
};
pub(in crate::view) use prepared::{
    build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds,
    has_pending_prepared_diff_syntax_chunk_builds_for_document,
    inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_line_for_inline_diff_row, prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_reparse_seed, request_syntax_highlights_for_prepared_document_byte_range,
    request_syntax_highlights_for_prepared_document_line_range,
};
#[cfg(test)]
pub(in crate::view) use prepared::{
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
    syntax_highlights_for_prepared_document_byte_range,
};
pub(in crate::view) use syntax::{
    DiffSyntaxBudget, DiffSyntaxEdit, DiffSyntaxLanguage, DiffSyntaxMode,
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
};

pub(super) fn syntax_highlights_for_streamed_line_slice_heuristic(
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
    requested_slice_range: Range<usize>,
    resolved_slice_range: Range<usize>,
) -> Option<Vec<(Range<usize>, gpui::HighlightStyle)>> {
    let tokens = syntax::syntax_tokens_for_streamed_line_slice_heuristic(
        raw_text,
        language,
        requested_slice_range,
        resolved_slice_range.clone(),
    )?;
    Some(prepared_document_line_highlights_from_tokens(
        theme,
        resolved_slice_range.len(),
        tokens.as_slice(),
    ))
}

/// Extracts the text content of a specific line from a document using precomputed
/// line starts. Returns an empty string if the line index is out of bounds.
/// Strips trailing newline.
pub(in crate::view) fn resolved_output_line_text<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_ix: usize,
) -> &'a str {
    if text.is_empty() {
        return "";
    }
    let (start, end) = line_byte_bounds(text, line_starts, line_ix);
    if start >= text.len() {
        return "";
    }
    text.get(start..end).unwrap_or("")
}

/// Returns `Auto` when a prepared document exists (full-document syntax),
/// `HeuristicOnly` when it doesn't (per-line fallback).
pub(super) fn syntax_mode_for_prepared_document(
    document: Option<PreparedDiffSyntaxDocument>,
) -> DiffSyntaxMode {
    if document.is_some() {
        DiffSyntaxMode::Auto
    } else {
        DiffSyntaxMode::HeuristicOnly
    }
}

const SYNTAX_HIGHLIGHT_STYLE_KINDS: [SyntaxTokenKind; 43] = [
    SyntaxTokenKind::None,
    SyntaxTokenKind::Comment,
    SyntaxTokenKind::CommentDoc,
    SyntaxTokenKind::String,
    SyntaxTokenKind::StringEscape,
    SyntaxTokenKind::StringRegex,
    SyntaxTokenKind::StringSpecial,
    SyntaxTokenKind::Keyword,
    SyntaxTokenKind::KeywordControl,
    SyntaxTokenKind::Preproc,
    SyntaxTokenKind::Number,
    SyntaxTokenKind::Boolean,
    SyntaxTokenKind::Function,
    SyntaxTokenKind::FunctionMethod,
    SyntaxTokenKind::FunctionSpecial,
    SyntaxTokenKind::Constructor,
    SyntaxTokenKind::Type,
    SyntaxTokenKind::TypeBuiltin,
    SyntaxTokenKind::TypeInterface,
    SyntaxTokenKind::Namespace,
    SyntaxTokenKind::Variable,
    SyntaxTokenKind::VariableParameter,
    SyntaxTokenKind::VariableSpecial,
    SyntaxTokenKind::VariableBuiltin,
    SyntaxTokenKind::Property,
    SyntaxTokenKind::Label,
    SyntaxTokenKind::Constant,
    SyntaxTokenKind::ConstantBuiltin,
    SyntaxTokenKind::Operator,
    SyntaxTokenKind::Punctuation,
    SyntaxTokenKind::PunctuationBracket,
    SyntaxTokenKind::PunctuationDelimiter,
    SyntaxTokenKind::PunctuationSpecial,
    SyntaxTokenKind::PunctuationListMarker,
    SyntaxTokenKind::Tag,
    SyntaxTokenKind::Attribute,
    SyntaxTokenKind::MarkupHeading,
    SyntaxTokenKind::MarkupLink,
    SyntaxTokenKind::TextLiteral,
    SyntaxTokenKind::DiffPlus,
    SyntaxTokenKind::DiffMinus,
    SyntaxTokenKind::DiffDelta,
    SyntaxTokenKind::Lifetime,
];

const SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES: usize = 4_096;
const PREPARED_READY_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES: usize = 32_768;
const SINGLE_LINE_STYLED_TEXT_CACHE_MAX_SOURCE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct DiffTextSourceIdentity {
    source_ptr: usize,
    source_len: usize,
}

impl DiffTextSourceIdentity {
    pub(super) fn from_str(text: &str) -> Self {
        Self {
            source_ptr: text.as_ptr() as usize,
            source_len: text.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SingleLineTextSourceCacheKey {
    Hashed(u64),
    Identity(DiffTextSourceIdentity),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SingleLineStyledTextCacheKey {
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    theme_signature: u64,
    source: SingleLineTextSourceCacheKey,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedReadyLineStyledTextCacheKey {
    theme_signature: u64,
    source_ptr: usize,
    source_len: usize,
    tokens_ptr: usize,
    tokens_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SingleLineWordHighlightedTextCacheKey {
    language: Option<DiffSyntaxLanguage>,
    mode: DiffSyntaxMode,
    theme_signature: u64,
    source: SingleLineTextSourceCacheKey,
    word_ranges_hash: u64,
}

#[derive(Clone)]
struct CachedSingleLineStyledText {
    source_text: Option<Arc<str>>,
    styled: CachedDiffStyledText,
}

#[derive(Clone)]
struct CachedSingleLineWordHighlightedText {
    source_text: Option<Arc<str>>,
    word_ranges: Arc<[Range<usize>]>,
    styled: CachedDiffStyledText,
}

#[derive(Clone)]
struct CachedPreparedReadyLineStyledText {
    source_text: Arc<str>,
    tokens: Arc<[syntax::SyntaxToken]>,
    styled: CachedDiffStyledText,
}

#[derive(Clone, Copy)]
struct CachedSingleLineThemeSignature {
    theme: AppTheme,
    signature: u64,
}

struct SingleLineStyledTextCache {
    by_key: FxLruCache<SingleLineStyledTextCacheKey, CachedSingleLineStyledText>,
    word_highlighted_by_key:
        FxLruCache<SingleLineWordHighlightedTextCacheKey, CachedSingleLineWordHighlightedText>,
    prepared_by_key:
        FxLruCache<PreparedReadyLineStyledTextCacheKey, CachedPreparedReadyLineStyledText>,
    cached_theme_signature: Option<CachedSingleLineThemeSignature>,
}

impl SingleLineStyledTextCache {
    fn new() -> Self {
        Self {
            by_key: new_fx_lru_cache(SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            word_highlighted_by_key: new_fx_lru_cache(SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            // Prepared ready-line scroll workloads can revisit tens of thousands
            // of line/side pairs before cycling back to the starting window.
            // Keep a larger LRU here so large prepared documents retain their
            // warmed row styles instead of thrashing after a single sweep.
            prepared_by_key: new_fx_lru_cache(PREPARED_READY_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            cached_theme_signature: None,
        }
    }

    fn theme_signature(&mut self, theme: AppTheme) -> u64 {
        if let Some(cached) = self.cached_theme_signature
            && cached.theme == theme
        {
            return cached.signature;
        }

        let signature = syntax_theme_signature(theme);
        self.cached_theme_signature = Some(CachedSingleLineThemeSignature { theme, signature });
        signature
    }

    fn key_for(
        &mut self,
        theme: AppTheme,
        language: DiffSyntaxLanguage,
        mode: DiffSyntaxMode,
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
    ) -> SingleLineStyledTextCacheKey {
        SingleLineStyledTextCacheKey {
            language,
            mode,
            theme_signature: self.theme_signature(theme),
            source: Self::source_key(text, source_identity),
        }
    }

    fn prepared_key_for(
        &mut self,
        theme: AppTheme,
        text: &str,
        tokens: &Arc<[syntax::SyntaxToken]>,
    ) -> PreparedReadyLineStyledTextCacheKey {
        PreparedReadyLineStyledTextCacheKey {
            theme_signature: self.theme_signature(theme),
            source_ptr: text.as_ptr() as usize,
            source_len: text.len(),
            tokens_ptr: tokens.as_ptr() as usize,
            tokens_len: tokens.len(),
        }
    }

    fn word_highlighted_key_for(
        &mut self,
        theme: AppTheme,
        language: Option<DiffSyntaxLanguage>,
        mode: DiffSyntaxMode,
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
        word_ranges: &[Range<usize>],
    ) -> SingleLineWordHighlightedTextCacheKey {
        SingleLineWordHighlightedTextCacheKey {
            language,
            mode,
            theme_signature: self.theme_signature(theme),
            source: Self::source_key(text, source_identity),
            word_ranges_hash: hash_word_ranges(word_ranges),
        }
    }

    fn source_key(
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
    ) -> SingleLineTextSourceCacheKey {
        source_identity
            .filter(|identity| identity.source_len == text.len())
            .map_or_else(
                || SingleLineTextSourceCacheKey::Hashed(hash_text_content(text)),
                SingleLineTextSourceCacheKey::Identity,
            )
    }

    fn get(
        &mut self,
        key: SingleLineStyledTextCacheKey,
        text: &str,
    ) -> Option<CachedDiffStyledText> {
        self.by_key
            .get(&key)
            .filter(|entry| {
                entry
                    .source_text
                    .as_deref()
                    .unwrap_or(entry.styled.text.as_ref())
                    == text
            })
            .map(|entry| entry.styled.clone())
    }

    fn get_word_highlighted(
        &mut self,
        key: SingleLineWordHighlightedTextCacheKey,
        text: &str,
        word_ranges: &[Range<usize>],
    ) -> Option<CachedDiffStyledText> {
        self.word_highlighted_by_key
            .get(&key)
            .filter(|entry| {
                entry
                    .source_text
                    .as_deref()
                    .unwrap_or(entry.styled.text.as_ref())
                    == text
                    && entry.word_ranges.as_ref() == word_ranges
            })
            .map(|entry| entry.styled.clone())
    }

    fn get_prepared(
        &mut self,
        key: PreparedReadyLineStyledTextCacheKey,
        text: &str,
        tokens: &Arc<[syntax::SyntaxToken]>,
    ) -> Option<CachedDiffStyledText> {
        self.prepared_by_key
            .get(&key)
            .filter(|entry| {
                entry.source_text.as_ref() == text && Arc::ptr_eq(&entry.tokens, tokens)
            })
            .map(|entry| entry.styled.clone())
    }

    fn insert(
        &mut self,
        key: SingleLineStyledTextCacheKey,
        text: &str,
        styled: CachedDiffStyledText,
    ) {
        self.by_key.put(
            key,
            CachedSingleLineStyledText {
                source_text: text.contains('\t').then(|| Arc::<str>::from(text)),
                styled,
            },
        );
    }

    fn insert_word_highlighted(
        &mut self,
        key: SingleLineWordHighlightedTextCacheKey,
        text: &str,
        word_ranges: &[Range<usize>],
        styled: CachedDiffStyledText,
    ) {
        self.word_highlighted_by_key.put(
            key,
            CachedSingleLineWordHighlightedText {
                source_text: text.contains('\t').then(|| Arc::<str>::from(text)),
                word_ranges: Arc::<[Range<usize>]>::from(word_ranges),
                styled,
            },
        );
    }

    fn insert_prepared(
        &mut self,
        key: PreparedReadyLineStyledTextCacheKey,
        text: &str,
        tokens: Arc<[syntax::SyntaxToken]>,
        styled: CachedDiffStyledText,
    ) {
        let source_text = if text.contains('\t') {
            Arc::<str>::from(text)
        } else {
            Arc::<str>::from(styled.text.clone())
        };
        self.prepared_by_key.put(
            key,
            CachedPreparedReadyLineStyledText {
                source_text,
                tokens,
                styled,
            },
        );
    }
}

#[derive(Clone)]
pub(super) struct SyntaxHighlightPalette {
    styles: [Option<gpui::HighlightStyle>; SYNTAX_HIGHLIGHT_STYLE_KINDS.len()],
}

impl SyntaxHighlightPalette {
    fn new(theme: AppTheme) -> Self {
        Self {
            styles: std::array::from_fn(|ix| {
                syntax_highlight_style(theme, SYNTAX_HIGHLIGHT_STYLE_KINDS[ix])
            }),
        }
    }

    fn style(&self, kind: SyntaxTokenKind) -> Option<gpui::HighlightStyle> {
        self.styles[kind as usize]
    }
}

pub(super) fn syntax_highlight_palette(theme: AppTheme) -> SyntaxHighlightPalette {
    SyntaxHighlightPalette::new(theme)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxDocument {
    inner: syntax::PreparedSyntaxDocument,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum PreparedDiffSyntaxParseMode {
    Full,
    Incremental,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct PreparedDiffSyntaxReparseSeed {
    inner: syntax::PreparedSyntaxReparseSeed,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct BackgroundPreparedDiffSyntaxDocument {
    inner: syntax::PreparedSyntaxDocumentData,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum PrepareDiffSyntaxDocumentResult {
    Ready(PreparedDiffSyntaxDocument),
    TimedOut,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiffSyntaxConfig {
    pub language: Option<DiffSyntaxLanguage>,
    pub mode: DiffSyntaxMode,
}

type DiffTextHighlight = (Range<usize>, gpui::HighlightStyle);
type DiffTextHighlights = [DiffTextHighlight];
type SharedDiffTextHighlights = Arc<DiffTextHighlights>;

#[derive(Clone, Copy)]
pub(super) struct DiffTextBuildRequest<'a> {
    pub(super) text: &'a str,
    pub(super) word_ranges: &'a [Range<usize>],
    pub(super) query: &'a str,
    pub(super) syntax: DiffSyntaxConfig,
    pub(super) word_color: Option<gpui::Rgba>,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedDiffTextBuildRequest<'a> {
    pub(super) build: DiffTextBuildRequest<'a>,
    pub(super) prepared_line: PreparedDiffSyntaxLine,
}

#[derive(Clone, Copy)]
pub(in crate::view) struct PreparedDiffSyntaxTextSource {
    pub document: Option<PreparedDiffSyntaxDocument>,
}

#[derive(Clone, Copy)]
pub(in crate::view) struct InlineDiffSyntaxOnlyRow<'a> {
    pub line: &'a AnnotatedDiffLine,
    pub text: &'a str,
}

#[derive(Clone, Copy)]
struct FusedDiffTextBuildRequest<'a> {
    build: DiffTextBuildRequest<'a>,
    syntax_tokens_override: Option<&'a [syntax::SyntaxToken]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxLine {
    pub document: Option<PreparedDiffSyntaxDocument>,
    pub line_ix: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_line_starts(text: &str) -> Arc<[usize]> {
        let mut line_starts = Vec::with_capacity(
            text.as_bytes()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1,
        );
        line_starts.push(0);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix + 1);
            }
        }
        Arc::from(line_starts)
    }

    fn prepare_test_document(
        language: DiffSyntaxLanguage,
        text: &str,
    ) -> PreparedDiffSyntaxDocument {
        let text: SharedString = text.to_owned().into();
        let line_starts = test_line_starts(text.as_ref());
        match prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&line_starts),
            DiffSyntaxBudget {
                foreground_parse: std::time::Duration::from_millis(50),
            },
            None,
            None,
        ) {
            PrepareDiffSyntaxDocumentResult::Ready(document) => document,
            PrepareDiffSyntaxDocumentResult::TimedOut => {
                inject_background_prepared_diff_syntax_document(
                    prepare_diff_syntax_document_in_background_text(
                        language,
                        DiffSyntaxMode::Auto,
                        text,
                        line_starts,
                    )
                    .expect("background parse should be available for supported test documents"),
                )
            }
            PrepareDiffSyntaxDocumentResult::Unsupported => {
                panic!("test document should support prepared syntax parsing")
            }
        }
    }

    #[test]
    fn sorted_range_cursor_advances_without_rescanning() {
        let ranges = [2..4, 6..8];
        let mut cursor = 0usize;

        assert!(!segment_overlaps_sorted_ranges(0, 2, &ranges, &mut cursor));
        assert_eq!(cursor, 0);

        assert!(segment_overlaps_sorted_ranges(2, 3, &ranges, &mut cursor));
        assert_eq!(cursor, 0);

        assert!(!segment_overlaps_sorted_ranges(4, 6, &ranges, &mut cursor));
        assert_eq!(cursor, 1);

        assert!(segment_overlaps_sorted_ranges(6, 7, &ranges, &mut cursor));
        assert_eq!(cursor, 1);

        assert!(!segment_overlaps_sorted_ranges(8, 9, &ranges, &mut cursor));
        assert_eq!(cursor, 2);
    }

    #[test]
    fn build_segments_fast_path_skips_syntax_work() {
        let segments = build_diff_text_segments("a\tb", &[], "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text.as_ref(), "a    b");
        assert!(!segments[0].in_word);
        assert!(!segments[0].in_query);
        assert_eq!(segments[0].syntax, SyntaxTokenKind::None);
    }

    #[test]
    fn build_cached_styled_text_plain_has_no_highlights() {
        let theme = AppTheme::gitcomet_dark();
        let styled =
            build_cached_diff_styled_text(theme, "a\tb", &[], "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(styled.text.as_ref(), "a    b");
        assert!(styled.highlights.is_empty());
        assert_eq!(styled.highlights_hash, 0);
    }

    #[test]
    fn build_segments_does_not_panic_on_non_char_boundary_ranges() {
        // This can happen if token ranges are computed in bytes that don't align to UTF-8
        // boundaries. We should never panic during diff rendering.
        let text = "aé"; // 'é' is 2 bytes in UTF-8
        let ranges = vec![Range { start: 1, end: 2 }];
        let segments =
            build_diff_text_segments(text, &ranges, "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text.as_ref(), text);
    }

    #[test]
    fn styled_text_highlights_cover_combined_ranges() {
        let theme = AppTheme::gitcomet_dark();
        let segments = vec![
            CachedDiffTextSegment {
                text: "abc".into(),
                in_word: false,
                in_query: false,
                syntax: SyntaxTokenKind::None,
            },
            CachedDiffTextSegment {
                text: "def".into(),
                in_word: false,
                in_query: true,
                syntax: SyntaxTokenKind::Keyword,
            },
        ];

        let (text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
        assert_eq!(text.as_ref(), "abcdef");
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 3..6);
        assert_eq!(highlights[0].1.font_weight, None);
        assert!(highlights[0].1.background_color.is_some());

        // Hashing highlights is used for caching shaped layouts; it should be stable for identical
        // highlight sequences within a process.
        let styled = build_cached_diff_styled_text(
            theme,
            "abcdef",
            &[],
            "def",
            None,
            DiffSyntaxMode::Auto,
            None,
        );
        assert_eq!(styled.highlights.len(), 1);
        assert_eq!(styled.highlights[0].0, 3..6);
    }

    #[test]
    fn cached_styled_text_highlights_all_query_occurrences() {
        let theme = AppTheme::gitcomet_dark();
        let styled = build_cached_diff_styled_text(
            theme,
            "abxxab",
            &[],
            "ab",
            None,
            DiffSyntaxMode::Auto,
            None,
        );
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..2);
        assert_eq!(styled.highlights[1].0, 4..6);
    }

    #[test]
    fn styled_text_word_highlight_sets_background() {
        let theme = AppTheme::gitcomet_dark();
        let segments = vec![CachedDiffTextSegment {
            text: "x".into(),
            in_word: true,
            in_query: false,
            syntax: SyntaxTokenKind::None,
        }];
        let (text, highlights) =
            styled_text_for_diff_segments(theme, &segments, Some(theme.colors.diff_remove_text));
        assert_eq!(text.as_ref(), "x");
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].1.background_color.is_some());
    }

    #[test]
    fn syntax_colors_are_softened_for_keywords() {
        let theme = AppTheme::gitcomet_light();
        let segments = vec![CachedDiffTextSegment {
            text: "fn".into(),
            in_word: false,
            in_query: false,
            syntax: SyntaxTokenKind::Keyword,
        }];

        let (_text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 0..2);
        assert_ne!(highlights[0].1.color, Some(theme.colors.accent.into()));
    }

    #[test]
    fn doc_comment_renders_italic() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::CommentDoc);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_style, Some(gpui::FontStyle::Italic));
        // Regular comments should not be italic.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Comment).unwrap();
        assert_eq!(plain.font_style, None);
    }

    #[test]
    fn keyword_control_renders_semibold() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::KeywordControl);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_weight, Some(gpui::FontWeight::SEMIBOLD));
        // Regular keywords should not have font weight.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Keyword).unwrap();
        assert_eq!(plain.font_weight, None);
    }

    #[test]
    fn syntax_highlight_palette_matches_direct_styles() {
        let theme = AppTheme::gitcomet_dark();
        let palette = syntax_highlight_palette(theme);

        for kind in SYNTAX_HIGHLIGHT_STYLE_KINDS {
            let direct = syntax_highlight_style(theme, kind);
            let cached = palette.style(kind);
            assert_eq!(
                cached.is_some(),
                direct.is_some(),
                "style presence mismatch for {kind:?}"
            );
            if let (Some(cached), Some(direct)) = (cached, direct) {
                assert_eq!(cached.color, direct.color, "color mismatch for {kind:?}");
                assert_eq!(
                    cached.background_color, direct.background_color,
                    "background mismatch for {kind:?}"
                );
                assert_eq!(
                    cached.font_style, direct.font_style,
                    "font_style mismatch for {kind:?}"
                );
                assert_eq!(
                    cached.font_weight, direct.font_weight,
                    "font_weight mismatch for {kind:?}"
                );
            }
        }
    }

    #[test]
    fn prepared_yaml_fast_path_matches_legacy_segment_builder_for_real_diff_lines() {
        let theme = AppTheme::gitcomet_dark();
        let text = concat!(
            "name: Deployment CI\n",
            "\n",
            "on:\n",
            "  pull_request:\n",
            "    branches: [\"main\"]\n",
            "    paths:\n",
            "      - \".github/workflows/build-release-artifacts.yml\"\n",
            "      - \".github/workflows/deploy-aur.yml\"\n",
            "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
            "      - \".github/workflows/deploy-apt-repo.yml\"\n",
            "      - \".github/workflows/release-manual-main.yml\"\n",
            "      - \"scripts/update-aur.sh\"\n",
            "      - \"scripts/package-macos.sh\"\n",
            "      - \"scripts/macos-cargo-config.sh\"\n",
            "      - \"scripts/generate-homebrew-formula.sh\"\n",
            "      - \"scripts/generate-homebrew-cask.sh\"\n",
            "      - \"scripts/build-apt-repo.sh\"\n",
            "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
            "  push:\n",
            "    branches: [\"main\"]\n",
            "    paths:\n",
            "      - \".github/workflows/build-release-artifacts.yml\"\n",
            "      - \".github/workflows/deploy-aur.yml\"\n",
            "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
            "      - \".github/workflows/deploy-apt-repo.yml\"\n",
            "      - \".github/workflows/release-manual-main.yml\"\n",
            "      - \"scripts/update-aur.sh\"\n",
            "      - \"scripts/package-macos.sh\"\n",
            "      - \"scripts/macos-cargo-config.sh\"\n",
            "      - \"scripts/generate-homebrew-formula.sh\"\n",
            "      - \"scripts/generate-homebrew-cask.sh\"\n",
            "      - \"scripts/build-apt-repo.sh\"\n",
            "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
            "  workflow_dispatch:\n",
            "\n",
            "permissions:\n",
            "  contents: read\n",
            "\n",
            "env:\n",
            "  CARGO_TERM_COLOR: always\n",
        );
        let lines: Vec<&str> = text.lines().collect();
        let document = prepare_test_document(DiffSyntaxLanguage::Yaml, text);
        let line_ixs = [
            16usize, 17, 18, 21, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33, 35,
        ];

        for line_ix in line_ixs {
            let line_text = lines[line_ix];
            let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)
                .unwrap_or_else(|| panic!("prepared YAML line {line_ix} should be available"));

            let current = match build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                theme,
                line_text,
                &[],
                "",
                DiffSyntaxConfig {
                    language: Some(DiffSyntaxLanguage::Yaml),
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                PreparedDiffSyntaxLine {
                    document: Some(document),
                    line_ix,
                },
            ) {
                PreparedDocumentLineStyledText::Cacheable(styled) => styled,
                PreparedDocumentLineStyledText::Pending(_) => {
                    panic!("prepared YAML line {line_ix} should not still be pending")
                }
            };

            let legacy = segments_to_cached_styled_text(
                theme,
                &build_diff_text_segments(
                    line_text,
                    &[],
                    "",
                    None,
                    DiffSyntaxMode::HeuristicOnly,
                    Some(tokens.as_ref()),
                ),
                None,
            );

            assert_eq!(
                current.text.as_ref(),
                legacy.text.as_ref(),
                "text mismatch for YAML line {line_ix}: {line_text:?}",
            );
            assert_eq!(
                current.highlights.as_ref(),
                legacy.highlights.as_ref(),
                "highlight mismatch for YAML line {line_ix}: {line_text:?}",
            );
        }
    }

    #[test]
    fn heuristic_yaml_highlights_match_prepared_document_for_real_diff_lines() {
        let theme = AppTheme::gitcomet_dark();
        let text = concat!(
            "name: Deployment CI\n",
            "\n",
            "on:\n",
            "  pull_request:\n",
            "    branches: [\"main\"]\n",
            "    paths:\n",
            "      - \".github/workflows/build-release-artifacts.yml\"\n",
            "      - \".github/workflows/deploy-aur.yml\"\n",
            "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
            "      - \".github/workflows/deploy-apt-repo.yml\"\n",
            "      - \".github/workflows/release-manual-main.yml\"\n",
            "      - \"scripts/update-aur.sh\"\n",
            "      - \"scripts/package-macos.sh\"\n",
            "      - \"scripts/macos-cargo-config.sh\"\n",
            "      - \"scripts/generate-homebrew-formula.sh\"\n",
            "      - \"scripts/generate-homebrew-cask.sh\"\n",
            "      - \"scripts/build-apt-repo.sh\"\n",
            "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
            "  push:\n",
            "    branches: [\"main\"]\n",
            "    paths:\n",
            "      - \".github/workflows/build-release-artifacts.yml\"\n",
            "      - \".github/workflows/deploy-aur.yml\"\n",
            "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
            "      - \".github/workflows/deploy-apt-repo.yml\"\n",
            "      - \".github/workflows/release-manual-main.yml\"\n",
            "      - \"scripts/update-aur.sh\"\n",
            "      - \"scripts/package-macos.sh\"\n",
            "      - \"scripts/macos-cargo-config.sh\"\n",
            "      - \"scripts/generate-homebrew-formula.sh\"\n",
            "      - \"scripts/generate-homebrew-cask.sh\"\n",
            "      - \"scripts/build-apt-repo.sh\"\n",
            "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
            "  workflow_dispatch:\n",
            "\n",
            "permissions:\n",
            "  contents: read\n",
            "\n",
            "env:\n",
            "  CARGO_TERM_COLOR: always\n",
        );
        let lines: Vec<&str> = text.lines().collect();
        let document = prepare_test_document(DiffSyntaxLanguage::Yaml, text);
        let line_ixs = [
            16usize, 17, 18, 21, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33, 35,
        ];

        for line_ix in line_ixs {
            let line_text = lines[line_ix];
            let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)
                .unwrap_or_else(|| panic!("prepared YAML line {line_ix} should be available"));
            let prepared =
                prepared_document_line_highlights_from_tokens(theme, line_text.len(), &tokens)
                    .into_iter()
                    .filter(|(_, style)| style.background_color.is_none())
                    .map(|(range, style)| (range, style.color))
                    .collect::<Vec<_>>();
            let heuristic = syntax_highlights_for_line(
                theme,
                line_text,
                DiffSyntaxLanguage::Yaml,
                DiffSyntaxMode::HeuristicOnly,
            )
            .into_iter()
            .filter(|(_, style)| style.background_color.is_none())
            .map(|(range, style)| (range, style.color))
            .collect::<Vec<_>>();

            assert_eq!(
                heuristic, prepared,
                "heuristic YAML highlighting should match prepared document highlighting for line {line_ix}: {line_text:?}",
            );
        }
    }

    #[test]
    fn syntax_highlight_style_uses_theme_syntax_overrides() {
        let theme = AppTheme::from_json_str(
            r##"{
                "name": "Fixture",
                "themes": [
                    {
                        "key": "fixture",
                        "name": "Fixture",
                        "appearance": "dark",
                        "colors": {
                            "window_bg": "#0d1016ff",
                            "surface_bg": "#1f2127ff",
                            "surface_bg_elevated": "#1f2127ff",
                            "active_section": "#2d2f34ff",
                            "border": "#2d2f34ff",
                            "text": "#bfbdb6ff",
                            "text_muted": "#8a8986ff",
                            "accent": "#5ac1feff",
                            "hover": "#2d2f34ff",
                            "active": { "hex": "#2d2f34ff", "alpha": 0.78 },
                            "focus_ring": { "hex": "#5ac1feff", "alpha": 0.60 },
                            "focus_ring_bg": { "hex": "#5ac1feff", "alpha": 0.16 },
                            "scrollbar_thumb": { "hex": "#8a8986ff", "alpha": 0.30 },
                            "scrollbar_thumb_hover": { "hex": "#8a8986ff", "alpha": 0.42 },
                            "scrollbar_thumb_active": { "hex": "#8a8986ff", "alpha": 0.52 },
                            "danger": "#ef7177ff",
                            "warning": "#feb454ff",
                            "success": "#aad84cff"
                        },
                        "syntax": {
                            "keyword": "#112233ff",
                            "variable": "#445566ff",
                            "diff_plus": "#abcdefff",
                            "label": "#fedcbaff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("theme JSON should parse");

        let keyword = syntax_highlight_style(theme, SyntaxTokenKind::Keyword)
            .expect("keyword style should be present");
        assert_eq!(keyword.color, Some(gpui::rgba(0x112233ff).into()));

        let variable = syntax_highlight_style(theme, SyntaxTokenKind::Variable)
            .expect("variable style should be present when overridden");
        assert_eq!(variable.color, Some(gpui::rgba(0x445566ff).into()));

        let diff_plus = syntax_highlight_style(theme, SyntaxTokenKind::DiffPlus)
            .expect("diff_plus style should be present");
        assert_eq!(diff_plus.color, Some(gpui::rgba(0xabcdefff).into()));

        let label = syntax_highlight_style(theme, SyntaxTokenKind::Label)
            .expect("label style should be present when overridden");
        assert_eq!(label.color, Some(gpui::rgba(0xfedcbaff).into()));
    }

    #[test]
    fn syntax_highlight_style_kind_table_covers_new_token_kinds() {
        for kind in [
            SyntaxTokenKind::StringRegex,
            SyntaxTokenKind::StringSpecial,
            SyntaxTokenKind::Preproc,
            SyntaxTokenKind::Constructor,
            SyntaxTokenKind::Namespace,
            SyntaxTokenKind::VariableBuiltin,
            SyntaxTokenKind::Label,
            SyntaxTokenKind::ConstantBuiltin,
            SyntaxTokenKind::PunctuationSpecial,
            SyntaxTokenKind::PunctuationListMarker,
            SyntaxTokenKind::MarkupHeading,
            SyntaxTokenKind::MarkupLink,
            SyntaxTokenKind::TextLiteral,
            SyntaxTokenKind::DiffPlus,
            SyntaxTokenKind::DiffMinus,
            SyntaxTokenKind::DiffDelta,
        ] {
            assert!(
                SYNTAX_HIGHLIGHT_STYLE_KINDS.contains(&kind),
                "syntax highlight palette should cover {kind:?}"
            );
        }
    }

    #[test]
    fn syntax_highlight_style_uses_all_new_theme_syntax_overrides() {
        let theme = AppTheme::from_json_str(
            r##"{
                "name": "Fixture",
                "themes": [
                    {
                        "key": "fixture",
                        "name": "Fixture",
                        "appearance": "dark",
                        "colors": {
                            "window_bg": "#0d1016ff",
                            "surface_bg": "#1f2127ff",
                            "surface_bg_elevated": "#1f2127ff",
                            "active_section": "#2d2f34ff",
                            "border": "#2d2f34ff",
                            "text": "#bfbdb6ff",
                            "text_muted": "#8a8986ff",
                            "accent": "#5ac1feff",
                            "hover": "#2d2f34ff",
                            "active": { "hex": "#2d2f34ff", "alpha": 0.78 },
                            "focus_ring": { "hex": "#5ac1feff", "alpha": 0.60 },
                            "focus_ring_bg": { "hex": "#5ac1feff", "alpha": 0.16 },
                            "scrollbar_thumb": { "hex": "#8a8986ff", "alpha": 0.30 },
                            "scrollbar_thumb_hover": { "hex": "#8a8986ff", "alpha": 0.42 },
                            "scrollbar_thumb_active": { "hex": "#8a8986ff", "alpha": 0.52 },
                            "danger": "#ef7177ff",
                            "warning": "#feb454ff",
                            "success": "#aad84cff"
                        },
                        "syntax": {
                            "string_regex": "#010101ff",
                            "string_special": "#020202ff",
                            "preproc": "#030303ff",
                            "constructor": "#040404ff",
                            "namespace": "#050505ff",
                            "variable_builtin": "#060606ff",
                            "label": "#070707ff",
                            "constant_builtin": "#080808ff",
                            "punctuation_special": "#090909ff",
                            "punctuation_list_marker": "#0a0a0aff",
                            "markup_heading": "#0b0b0bff",
                            "markup_link": "#0c0c0cff",
                            "text_literal": "#0d0d0dff",
                            "diff_plus": "#0e0e0eff",
                            "diff_minus": "#0f0f0fff",
                            "diff_delta": "#101010ff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("theme JSON should parse");

        for (kind, color) in [
            (SyntaxTokenKind::StringRegex, 0x010101ff),
            (SyntaxTokenKind::StringSpecial, 0x020202ff),
            (SyntaxTokenKind::Preproc, 0x030303ff),
            (SyntaxTokenKind::Constructor, 0x040404ff),
            (SyntaxTokenKind::Namespace, 0x050505ff),
            (SyntaxTokenKind::VariableBuiltin, 0x060606ff),
            (SyntaxTokenKind::Label, 0x070707ff),
            (SyntaxTokenKind::ConstantBuiltin, 0x080808ff),
            (SyntaxTokenKind::PunctuationSpecial, 0x090909ff),
            (SyntaxTokenKind::PunctuationListMarker, 0x0a0a0aff),
            (SyntaxTokenKind::MarkupHeading, 0x0b0b0bff),
            (SyntaxTokenKind::MarkupLink, 0x0c0c0cff),
            (SyntaxTokenKind::TextLiteral, 0x0d0d0dff),
            (SyntaxTokenKind::DiffPlus, 0x0e0e0eff),
            (SyntaxTokenKind::DiffMinus, 0x0f0f0fff),
            (SyntaxTokenKind::DiffDelta, 0x101010ff),
        ] {
            let style = syntax_highlight_style(theme, kind)
                .unwrap_or_else(|| panic!("{kind:?} style should be present"));
            assert_eq!(
                style.color,
                Some(gpui::rgba(color).into()),
                "{kind:?} should use its explicit syntax override"
            );
        }
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_expands_tabs_and_remaps_ranges() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "\tlet value",
            &[(0..1, style), (1..4, style)],
        );

        assert_eq!(styled.text.as_ref(), "    let value");
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..4);
        assert_eq!(styled.highlights[1].0, 4..7);
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_handles_multibyte_utf8_with_tabs() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        // "→" is 3 bytes (U+2192), tab is 1 byte.
        // Input: "\t→x" — 5 bytes: tab(0..1), arrow(1..4), x(4..5)
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "\t\u{2192}x",
            &[(0..1, style), (1..4, style), (4..5, style)],
        );

        // Tab expands to 4 spaces, arrow stays 3 bytes, x stays 1 byte.
        assert_eq!(styled.text.as_ref(), "    \u{2192}x");
        assert_eq!(styled.highlights.len(), 3);
        // Tab (0..1) → expanded to 4-space span (0..4).
        assert_eq!(styled.highlights[0].0, 0..4);
        // Arrow (1..4) → starts at 4 (after tab expansion), length 3 bytes.
        assert_eq!(styled.highlights[1].0, 4..7);
        // x (4..5) → starts at 7.
        assert_eq!(styled.highlights[2].0, 7..8);
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_no_tabs_passes_through() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "let x = 1;",
            &[(0..3, style), (8..9, style)],
        );

        assert_eq!(styled.text.as_ref(), "let x = 1;");
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..3);
        assert_eq!(styled.highlights[1].0, 8..9);
    }

    #[test]
    fn cached_styled_text_syntax_only_expands_tabs_without_segment_build() {
        let theme = AppTheme::gitcomet_dark();
        let styled = build_cached_diff_styled_text(
            theme,
            "\tlet value = 42;",
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );

        assert_eq!(styled.text.as_ref(), "    let value = 42;");
        assert!(styled.highlights.iter().any(|(range, _)| *range == (4..7)));
        assert!(
            styled
                .highlights
                .iter()
                .any(|(range, _)| *range == (16..18))
        );
    }

    #[test]
    fn repeated_syntax_only_line_styled_text_reuses_cached_highlights() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let cached_value = 42;";

        let first = build_cached_diff_styled_text(
            theme,
            text,
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
        let second = build_cached_diff_styled_text(
            theme,
            text,
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );

        assert!(
            !first.highlights.is_empty(),
            "heuristic syntax styling should produce highlights for Rust keywords"
        );
        assert!(
            Arc::ptr_eq(&first.highlights, &second.highlights),
            "repeated syntax-only lines should reuse the cached highlight arc"
        );
    }

    #[test]
    fn repeated_word_highlighted_line_styled_text_reuses_cached_highlights() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let cached_value = replacement_value;";
        let word_ranges = [4..16, 19..36];

        let first = build_cached_diff_styled_text(
            theme,
            text,
            &word_ranges,
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
        let second = build_cached_diff_styled_text(
            theme,
            text,
            &word_ranges,
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );

        assert!(
            !first.highlights.is_empty(),
            "word-highlighted syntax styling should produce at least one highlight segment"
        );
        assert!(
            Arc::ptr_eq(&first.highlights, &second.highlights),
            "repeated word-highlighted lines should reuse the cached highlight arc"
        );
    }

    #[test]
    fn repeated_prepared_ready_line_styled_text_reuses_cached_text_and_highlights() {
        let theme = AppTheme::gitcomet_dark();
        let highlight_palette = syntax_highlight_palette(theme);
        let text = "let prepared_cached_value = 42;";
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
        assert!(
            syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some(),
            "prepared document should expose a ready first line"
        );

        let request = PreparedDiffTextBuildRequest {
            build: DiffTextBuildRequest {
                text,
                word_ranges: &[],
                query: "",
                syntax: DiffSyntaxConfig {
                    language: Some(DiffSyntaxLanguage::Rust),
                    mode: DiffSyntaxMode::Auto,
                },
                word_color: None,
            },
            prepared_line: PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 0,
            },
        };

        let first =
            match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                theme,
                &highlight_palette,
                request,
            ) {
                PreparedDocumentLineStyledText::Cacheable(styled) => styled,
                PreparedDocumentLineStyledText::Pending(_) => {
                    panic!("prepared first line should be cacheable once the chunk is loaded")
                }
            };
        let second =
            match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                theme,
                &highlight_palette,
                request,
            ) {
                PreparedDocumentLineStyledText::Cacheable(styled) => styled,
                PreparedDocumentLineStyledText::Pending(_) => {
                    panic!("repeated prepared first line should remain cacheable")
                }
            };

        assert!(
            !first.highlights.is_empty(),
            "prepared syntax styling should produce highlights for Rust keywords"
        );
        assert!(
            Arc::ptr_eq(&first.highlights, &second.highlights),
            "repeated prepared ready-line lookups should reuse the cached highlight arc"
        );
        let first_text: Arc<str> = first.text.clone().into();
        let second_text: Arc<str> = second.text.clone().into();
        assert!(
            Arc::ptr_eq(&first_text, &second_text),
            "repeated prepared ready-line lookups should reuse the cached text arc"
        );
    }

    #[test]
    fn prepared_ready_line_styled_text_cache_respects_full_document_context() {
        let theme = AppTheme::gitcomet_dark();
        let highlight_palette = syntax_highlight_palette(theme);
        let line_text = "still comment */ let x = 1;";
        let multiline_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &format!("/* open comment\n{line_text}"),
        );
        let standalone_document = prepare_test_document(DiffSyntaxLanguage::Rust, line_text);
        assert!(
            syntax::syntax_tokens_for_prepared_document_line(multiline_document.inner, 1).is_some(),
            "multiline prepared document should expose a ready continuation line"
        );
        assert!(
            syntax::syntax_tokens_for_prepared_document_line(standalone_document.inner, 0)
                .is_some(),
            "standalone prepared document should expose a ready first line"
        );

        let build = |document, line_ix| {
            match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                theme,
                &highlight_palette,
                PreparedDiffTextBuildRequest {
                    build: DiffTextBuildRequest {
                        text: line_text,
                        word_ranges: &[],
                        query: "",
                        syntax: DiffSyntaxConfig {
                            language: Some(DiffSyntaxLanguage::Rust),
                            mode: DiffSyntaxMode::Auto,
                        },
                        word_color: None,
                    },
                    prepared_line: PreparedDiffSyntaxLine {
                        document: Some(document),
                        line_ix,
                    },
                },
            ) {
                PreparedDocumentLineStyledText::Cacheable(styled) => styled,
                PreparedDocumentLineStyledText::Pending(_) => {
                    panic!("ready prepared documents should not fall back to pending")
                }
            }
        };

        let multiline = build(multiline_document, 1);
        let standalone = build(standalone_document, 0);
        let start_style = |styled: &CachedDiffStyledText| {
            styled
                .highlights
                .iter()
                .find(|(range, _)| range.start == 0)
                .and_then(|(_, style)| style.color)
        };

        assert_eq!(
            start_style(&multiline),
            Some(theme.syntax.comment.into()),
            "multiline continuation should keep comment highlighting from document context"
        );
        assert_ne!(
            start_style(&standalone),
            Some(theme.syntax.comment.into()),
            "standalone line should not reuse cached comment styling from another prepared document"
        );
    }

    #[test]
    fn syntax_only_line_styled_text_cache_is_scoped_by_theme() {
        let text = "let themed_value = 42;";
        let dark_theme = AppTheme::gitcomet_dark();
        let light_theme = AppTheme::gitcomet_light();

        let dark = build_cached_diff_styled_text(
            dark_theme,
            text,
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
        let light = build_cached_diff_styled_text(
            light_theme,
            text,
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );

        let dark_keyword = dark
            .highlights
            .iter()
            .find(|(range, _)| *range == (0..3))
            .and_then(|(_, style)| style.color);
        let light_keyword = light
            .highlights
            .iter()
            .find(|(range, _)| *range == (0..3))
            .and_then(|(_, style)| style.color);

        assert_eq!(dark_keyword, Some(dark_theme.syntax.keyword.into()));
        assert_eq!(light_keyword, Some(light_theme.syntax.keyword.into()));
        assert_ne!(
            dark_keyword, light_keyword,
            "theme-specific syntax colors should not bleed across cached entries"
        );
    }

    #[test]
    fn prepared_document_byte_range_highlights_multiline_comment_continuation() {
        let theme = AppTheme::gitcomet_dark();
        let text = "/* open comment\nstill comment */ let x = 1;";
        let line_starts = vec![0, "/* open comment\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let second_line_start = line_starts[1];
        let highlights = syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            second_line_start..text.len(),
        )
        .expect("prepared document should still be available");

        assert!(
            highlights
                .iter()
                .all(|(range, _)| range.start >= second_line_start),
            "returned highlights should be clipped to the requested byte range"
        );
        assert!(
            highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.syntax.comment.into())
            }),
            "second line should retain comment highlighting from multiline document context"
        );
    }

    #[test]
    fn nonblocking_prepared_document_byte_range_upgrades_after_chunk_build() {
        syntax::reset_prepared_syntax_cache();
        let theme = AppTheme::gitcomet_dark();
        let text = "/* open comment\nstill comment */ let x = 1;";
        let line_starts = vec![0, "/* open comment\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let second_line_start = line_starts[1];
        let first = request_syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            second_line_start..text.len(),
        )
        .expect("prepared document should be requestable");
        assert!(first.pending);
        assert!(
            !first.highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.syntax.comment.into())
            }),
            "heuristic fallback should not invent multiline comment state before the chunk is ready"
        );

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = request_syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            second_line_start..text.len(),
        )
        .expect("prepared document should still be available after chunk completion");
        assert!(!second.pending);
        assert!(
            second.highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.syntax.comment.into())
            }),
            "resolved output should upgrade to full document-aware comment highlighting"
        );
    }

    #[test]
    fn prepared_document_line_range_reports_ready_and_pending_rows_per_chunk() {
        let theme = AppTheme::gitcomet_dark();
        let lines: Vec<String> = (0..70)
            .map(|ix| format!("let chunk_boundary_value_{ix} = {ix};"))
            .collect();
        let text = lines.join("\n");
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut offset = 0usize;
        for line in &lines {
            line_starts.push(offset);
            offset = offset.saturating_add(line.len()).saturating_add(1);
        }

        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &text);
        assert!(
            syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some(),
            "first chunk should be loadable synchronously"
        );

        let first = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            &text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            63..66,
        )
        .expect("row-range request should succeed");
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].line_ix, 63);
        assert!(!first[0].pending, "loaded chunk row should be ready");
        assert!(
            !first[0].highlights.is_empty(),
            "ready row should include syntax highlights"
        );
        assert_eq!(first[1].line_ix, 64);
        assert_eq!(first[2].line_ix, 65);
        assert!(
            first[1].pending || first[2].pending,
            "at least one row in the next chunk should still be pending before the background drain"
        );

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            &text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            63..66,
        )
        .expect("row-range request should still succeed after chunk drain");
        assert!(second.iter().all(|line| !line.pending));
    }

    #[test]
    fn prepared_document_line_range_clamps_beyond_document_bounds() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let a = 1;\nlet b = 2;";
        let line_starts = vec![0, "let a = 1;\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
        // Load chunk 0 synchronously so the request API returns Ready.
        assert!(syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some());

        // Request range extends beyond the 2-line document.
        let result = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            1..5,
        )
        .expect("line-range request should succeed");
        // Should only return 1 line (line_ix 1), not 4.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line_ix, 1);
        assert!(!result[0].pending);

        // Fully out-of-bounds range returns empty.
        let result = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            10..15,
        )
        .expect("out-of-bounds range should still succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn nonblocking_prepared_line_helper_transitions_from_pending_to_cacheable() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let value = 1;";
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let first = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            theme,
            text,
            &[],
            "",
            DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Rust),
                mode: DiffSyntaxMode::Auto,
            },
            None,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 0,
            },
        );
        match first {
            PreparedDocumentLineStyledText::Pending(styled) => {
                assert_eq!(styled.text.as_ref(), text);
            }
            PreparedDocumentLineStyledText::Cacheable(_) => {
                panic!("first nonblocking prepared-line request should be pending")
            }
        }

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds() == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            theme,
            text,
            &[],
            "",
            DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Rust),
                mode: DiffSyntaxMode::Auto,
            },
            None,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 0,
            },
        );
        match second {
            PreparedDocumentLineStyledText::Cacheable(styled) => {
                assert!(
                    !styled.highlights.is_empty(),
                    "cacheable prepared-line styling should contain syntax highlights"
                );
            }
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("prepared-line helper should become cacheable after chunk drain")
            }
        }
    }

    #[test]
    fn prepared_diff_syntax_line_for_one_based_line_converts_to_zero_based_index() {
        let document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let first = 1;", "let second = 2;"].join("\n"),
        );

        let prepared = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
        assert_eq!(
            prepared,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 1,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_one_based_line_rejects_missing_or_zero_lines() {
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

        assert_eq!(
            prepared_diff_syntax_line_for_one_based_line(Some(document), None),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
        assert_eq!(
            prepared_diff_syntax_line_for_one_based_line(Some(document), Some(0)),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_inline_diff_row_projects_remove_add_and_context_lines() {
        use gitcomet_core::domain::DiffLineKind;

        let old_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let old_one = 1;", "let old_two = 2;"].join("\n"),
        );
        let new_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let new_one = 1;", "let new_two = 2;", "let new_three = 3;"].join("\n"),
        );

        let remove_line = AnnotatedDiffLine {
            kind: DiffLineKind::Remove,
            text: "-let old_two = 2;".into(),
            old_line: Some(2),
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &remove_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(old_document),
                line_ix: 1,
            }
        );

        let add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: "+let new_three = 3;".into(),
            old_line: None,
            new_line: Some(3),
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &add_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 2,
            }
        );

        let context_line = AnnotatedDiffLine {
            kind: DiffLineKind::Context,
            text: " let new_one = 1;".into(),
            old_line: Some(1),
            new_line: Some(1),
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &context_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 0,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_inline_diff_row_rejects_meta_rows_and_missing_lines() {
        use gitcomet_core::domain::DiffLineKind;

        let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

        let header_line = AnnotatedDiffLine {
            kind: DiffLineKind::Header,
            text: "diff --git a/file b/file".into(),
            old_line: None,
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(document),
                Some(document),
                &header_line
            ),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );

        let missing_add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: "+let value = 1;".into(),
            old_line: None,
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(document),
                Some(document),
                &missing_add_line,
            ),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
    }

    #[test]
    fn inline_projection_tokens_come_from_correct_document_side() {
        use gitcomet_core::domain::DiffLineKind;

        // Old document contains a struct definition; new document contains a function.
        // We verify that projected syntax tokens actually carry the expected token kinds
        // from the correct side, not just that the document/line_ix are set correctly.
        let old_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["struct Foo {", "    x: u32,", "}"].join("\n"),
        );
        let new_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["fn bar() {", "    let y = 42;", "}"].join("\n"),
        );

        // Remove line: old_line=1 should project from old document line 0 ("struct Foo {")
        let remove_line = AnnotatedDiffLine {
            kind: DiffLineKind::Remove,
            text: "-struct Foo {".into(),
            old_line: Some(1),
            new_line: None,
        };
        let projected = prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &remove_line,
        );
        assert_eq!(projected.document, Some(old_document));
        let old_tokens = syntax::syntax_tokens_for_prepared_document_line(
            projected.document.unwrap().inner,
            projected.line_ix,
        );
        assert!(
            old_tokens
                .as_ref()
                .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword)),
            "remove line should get tokens from old doc containing 'struct' keyword: {old_tokens:?}"
        );

        // Add line: new_line=2 should project from new document line 1 ("    let y = 42;")
        let add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: "+    let y = 42;".into(),
            old_line: None,
            new_line: Some(2),
        };
        let projected = prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &add_line,
        );
        assert_eq!(projected.document, Some(new_document));
        let new_tokens = syntax::syntax_tokens_for_prepared_document_line(
            projected.document.unwrap().inner,
            projected.line_ix,
        );
        assert!(
            new_tokens
                .as_ref()
                .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number)),
            "add line should get tokens from new doc containing number literal: {new_tokens:?}"
        );
    }

    #[test]
    fn split_view_projection_indexes_real_document_lines() {
        // Verify that prepared_diff_syntax_line_for_one_based_line gives correct
        // syntax tokens when the document is built from real file text rather than
        // the old aligned-row approach (which padded empty lines).
        let document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &[
                "fn greet() {",          // line 1
                "    println!(\"hi\");", // line 2
                "}",                     // line 3
            ]
            .join("\n"),
        );

        // Line 1 should have a keyword ("fn")
        let line1 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(1));
        assert_eq!(line1.line_ix, 0);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line1.document.unwrap().inner,
            line1.line_ix,
        );
        assert!(
            tokens
                .as_ref()
                .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::Keyword)),
            "line 1 should contain 'fn' keyword: {tokens:?}"
        );

        // Line 2 should have a string
        let line2 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
        assert_eq!(line2.line_ix, 1);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line2.document.unwrap().inner,
            line2.line_ix,
        );
        assert!(
            tokens
                .as_ref()
                .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::String)),
            "line 2 should contain a string literal: {tokens:?}"
        );

        // Line 3 should be just punctuation (closing brace)
        let line3 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(3));
        assert_eq!(line3.line_ix, 2);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line3.document.unwrap().inner,
            line3.line_ix,
        );
        assert!(
            tokens.as_ref().is_some_and(|t| t
                .iter()
                .any(|tok| tok.kind == SyntaxTokenKind::PunctuationBracket)),
            "line 3 should contain punctuation bracket: {tokens:?}"
        );
    }

    #[test]
    fn query_overlay_reuses_base_when_query_is_empty_or_missing() {
        let theme = AppTheme::gitcomet_dark();
        let text: SharedString = "abcdef".into();
        let mut text_hasher = FxHasher::default();
        text.as_ref().hash(&mut text_hasher);
        let text_hash = text_hasher.finish();
        let style = gpui::HighlightStyle {
            color: Some(theme.colors.text.into()),
            ..Default::default()
        };
        let base = CachedDiffStyledText {
            text,
            highlights: Arc::from(vec![(0..6, style)]),
            highlights_hash: 42,
            text_hash,
        };

        let empty_query = build_cached_diff_query_overlay_styled_text(theme, &base, "");
        assert!(Arc::ptr_eq(&empty_query.highlights, &base.highlights));
        assert_eq!(empty_query.highlights_hash, base.highlights_hash);

        let missing_query = build_cached_diff_query_overlay_styled_text(theme, &base, "xyz");
        assert!(Arc::ptr_eq(&missing_query.highlights, &base.highlights));
        assert_eq!(missing_query.highlights_hash, base.highlights_hash);
    }

    #[test]
    fn query_overlay_adds_background_without_losing_existing_color() {
        let theme = AppTheme::gitcomet_dark();
        let text: SharedString = "abcdef".into();
        let mut text_hasher = FxHasher::default();
        text.as_ref().hash(&mut text_hasher);
        let text_hash = text_hasher.finish();
        let style = gpui::HighlightStyle {
            color: Some(theme.colors.warning.into()),
            ..Default::default()
        };
        let base = CachedDiffStyledText {
            text,
            highlights: Arc::from(vec![(0..6, style)]),
            highlights_hash: 7,
            text_hash,
        };

        let overlaid = build_cached_diff_query_overlay_styled_text(theme, &base, "cd");
        assert_eq!(overlaid.highlights.len(), 3);
        assert_eq!(overlaid.highlights[1].0, 2..4);
        assert_eq!(
            overlaid.highlights[1].1.color,
            Some(theme.colors.warning.into())
        );
        assert!(overlaid.highlights[1].1.background_color.is_some());
        assert_ne!(overlaid.highlights_hash, base.highlights_hash);
    }

    #[test]
    fn query_overlay_splits_across_base_and_query_boundaries_without_sorting() {
        let theme = AppTheme::gitcomet_dark();
        let text: SharedString = "abcdefghij".into();
        let mut text_hasher = FxHasher::default();
        text.as_ref().hash(&mut text_hasher);
        let text_hash = text_hasher.finish();
        let left = gpui::HighlightStyle {
            color: Some(theme.colors.warning.into()),
            ..Default::default()
        };
        let right = gpui::HighlightStyle {
            color: Some(theme.colors.success.into()),
            ..Default::default()
        };
        let base = CachedDiffStyledText {
            text,
            highlights: Arc::from(vec![(1..3, left), (5..8, right)]),
            highlights_hash: 11,
            text_hash,
        };

        let overlaid = build_cached_diff_query_overlay_styled_text(theme, &base, "cdefg");
        assert_eq!(overlaid.highlights.len(), 5);

        assert_eq!(overlaid.highlights[0], (1..2, left));
        assert_eq!(overlaid.highlights[1].0, 2..3);
        assert_eq!(
            overlaid.highlights[1].1.color,
            Some(theme.colors.warning.into())
        );
        assert!(overlaid.highlights[1].1.background_color.is_some());
        assert_eq!(
            overlaid.highlights[2],
            (
                3..5,
                gpui::HighlightStyle {
                    background_color: Some(
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 })
                            .into()
                    ),
                    ..Default::default()
                }
            )
        );
        assert_eq!(overlaid.highlights[3].0, 5..7);
        assert_eq!(
            overlaid.highlights[3].1.color,
            Some(theme.colors.success.into())
        );
        assert!(overlaid.highlights[3].1.background_color.is_some());
        assert_eq!(overlaid.highlights[4], (7..8, right));
    }
}
