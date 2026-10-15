use super::*;
use keywords::is_keyword;

mod keywords;

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
    PowerShell,
    Haskell,
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
    /// Haskell spells a comment `--`, but a run of dashes followed by a symbol
    /// character is an *operator* (`-->`, `--|`), not a comment. Lua and SQL share
    /// the `--` prefix with no such rule, so this cannot be keyed off
    /// `line_comment`.
    pub(super) haskell_dashes_line_comment: bool,
}

impl HeuristicCommentConfig {
    const fn base() -> Self {
        Self {
            line_comment: None,
            hash_comment: false,
            block_comment: None,
            visual_basic_line_comment: false,
            haskell_dashes_line_comment: false,
        }
    }

    /// No comment syntax.
    const fn none() -> Self {
        Self::base()
    }

    /// `#` line comments only.
    const fn hash() -> Self {
        Self {
            hash_comment: true,
            ..Self::base()
        }
    }

    /// One line-comment prefix.
    const fn line(prefix: &'static str) -> Self {
        Self {
            line_comment: Some(prefix),
            ..Self::base()
        }
    }

    /// One line-comment prefix plus `#`.
    const fn line_and_hash(prefix: &'static str) -> Self {
        Self {
            line_comment: Some(prefix),
            hash_comment: true,
            ..Self::base()
        }
    }

    /// One block-comment spec.
    const fn block(spec: HeuristicBlockCommentSpec) -> Self {
        Self {
            block_comment: Some(spec),
            ..Self::base()
        }
    }

    /// `#` plus one block-comment spec.
    const fn hash_block(spec: HeuristicBlockCommentSpec) -> Self {
        Self {
            hash_comment: true,
            block_comment: Some(spec),
            ..Self::base()
        }
    }

    /// One line-comment prefix plus one block-comment spec.
    const fn line_block(prefix: &'static str, spec: HeuristicBlockCommentSpec) -> Self {
        Self {
            line_comment: Some(prefix),
            block_comment: Some(spec),
            ..Self::base()
        }
    }

    /// One line-comment prefix, `#`, and one block-comment spec.
    const fn line_hash_block(prefix: &'static str, spec: HeuristicBlockCommentSpec) -> Self {
        Self {
            line_comment: Some(prefix),
            hash_comment: true,
            block_comment: Some(spec),
            ..Self::base()
        }
    }

    /// Visual Basic's apostrophe line comment.
    const fn visual_basic() -> Self {
        Self {
            visual_basic_line_comment: true,
            ..Self::base()
        }
    }

    /// Haskell's `--`/`{- -}` rules with the operator-aware dash handling.
    const fn haskell() -> Self {
        Self {
            block_comment: Some(HEURISTIC_HASKELL_BLOCK_COMMENT),
            haskell_dashes_line_comment: true,
            ..Self::base()
        }
    }
}

#[derive(Clone, Copy)]
struct HeuristicOpenStateScanConfig {
    comment: HeuristicCommentConfig,
    block_comment_kind: Option<HeuristicBlockCommentKind>,
    allow_backtick_strings: bool,
    single_quote: HeuristicSingleQuote,
}

impl HeuristicOpenStateScanConfig {
    fn for_language(language: DiffSyntaxLanguage) -> Self {
        Self {
            comment: heuristic_comment_config(language),
            block_comment_kind: heuristic_block_comment_kind(language),
            allow_backtick_strings: heuristic_allows_backtick_strings(language),
            single_quote: heuristic_single_quote_role(language),
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
const HEURISTIC_HASKELL_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "{-",
    end: "-}",
};
const HEURISTIC_C_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "/*",
    end: "*/",
};
const HEURISTIC_POWERSHELL_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "<#",
    end: "#>",
};

pub(super) fn heuristic_comment_config(language: DiffSyntaxLanguage) -> HeuristicCommentConfig {
    match language {
        // Vue's root grammar is html-like, so it uses html comments here rather
        // than `//`: a template line such as `<img src="//cdn…">` must not be
        // greyed out as a comment.
        // Jinja joins the markup languages for the same reason Vue does, and
        // takes `<!-- -->` rather than its own `{# #}`: the heuristic supports one
        // block-comment spec, a `.njk`/`.j2` file is mostly HTML, and the
        // tree-sitter path already colours `{# #}` as a `(comment)` node. This
        // fallback only runs for lines past MAX_TREESITTER_LINE_BYTES or in
        // HeuristicOnly mode.
        DiffSyntaxLanguage::Html
        | DiffSyntaxLanguage::Xml
        | DiffSyntaxLanguage::Vue
        | DiffSyntaxLanguage::Svelte
        | DiffSyntaxLanguage::Jinja => HeuristicCommentConfig::block(HEURISTIC_HTML_BLOCK_COMMENT),
        // A text-bodied template has no markup to comment, so `<!-- -->` would be
        // wrong; its body is yaml, shell, nginx.conf or dotenv, and all four use
        // `#`. That also lands on Jinja's own `{# … #}`, one byte late.
        DiffSyntaxLanguage::JinjaText => HeuristicCommentConfig::hash(),
        // Pascal's block comment is `(* ... *)`, which is F#'s and OCaml's too.
        // Its other form, `{ ... }`, has no `HeuristicBlockCommentKind` and would
        // swallow every record and set literal if it did.
        // EBNF's comment is `(* ... *)`, the same as Pascal's and F#'s.
        DiffSyntaxLanguage::Ebnf
        | DiffSyntaxLanguage::Pascal
        | DiffSyntaxLanguage::FSharp
        | DiffSyntaxLanguage::OCaml
        | DiffSyntaxLanguage::OCamlInterface => {
            HeuristicCommentConfig::block(HEURISTIC_FSHARP_BLOCK_COMMENT)
        }
        // `line_comment` stays None: `haskell_dashes_line_comment` replaces the
        // plain `--` prefix with a rule that knows `-->` is an operator.
        DiffSyntaxLanguage::Haskell => HeuristicCommentConfig::haskell(),
        DiffSyntaxLanguage::Erlang => HeuristicCommentConfig::line("%"),
        // `.properties` takes `!` as well as `#`, and only at the head of a line;
        // `line_comment` carries the second marker.
        DiffSyntaxLanguage::JavaProperties => HeuristicCommentConfig::line_and_hash("!"),
        DiffSyntaxLanguage::Ini => HeuristicCommentConfig::line_and_hash(";"),
        // `.conf` takes `#` only, even though the INI dialects among them also
        // take `;`. In nginx -- which is most of what people call a `.conf` --
        // `;` ends every statement, so treating it as a comment greys the tail of
        // every line in the file. The line-leading `;` those INI dialects
        // actually write is picked up in the main loop instead.
        // `.env` takes `#` only, and only that: a `;` is an ordinary value byte.
        DiffSyntaxLanguage::Dotenv => HeuristicCommentConfig::hash(),
        DiffSyntaxLanguage::Conf => HeuristicCommentConfig::hash(),
        // Wasm text takes `;;` for a line comment and `(; ... ;)` for a block one;
        // only the first is expressible here.
        DiffSyntaxLanguage::Wat => HeuristicCommentConfig::line(";;"),
        DiffSyntaxLanguage::Llvm | DiffSyntaxLanguage::Spirv => HeuristicCommentConfig::line(";"),
        DiffSyntaxLanguage::Clojure => HeuristicCommentConfig::line(";"),
        // Assembly takes `;` (MASM/NASM) and `/* */` (GAS, every target).
        //
        // Neither GAS line-comment spelling is here. `#` is out because ARM writes
        // immediates as `mov r0, #1`, so a hash comment would grey the operand of
        // every such instruction -- see `assembly_hash_immediate_is_not_a_comment`.
        // `//` (GAS on arm64) has no such conflict and is simply unreachable:
        // HeuristicCommentConfig carries one `line_comment`, the right choice is
        // dialect-dependent, and nothing here knows the target. `/* */` covers GAS
        // block comments and the tree-sitter path handles the rest.
        // CIL takes C's comment syntax, being an assembly language written by a
        // C-family toolchain rather than by hand.
        DiffSyntaxLanguage::Cil => {
            HeuristicCommentConfig::line_block("//", HEURISTIC_C_BLOCK_COMMENT)
        }
        DiffSyntaxLanguage::Assembly => {
            HeuristicCommentConfig::line_block(";", HEURISTIC_C_BLOCK_COMMENT)
        }
        DiffSyntaxLanguage::Lua => {
            HeuristicCommentConfig::line_block("--", HEURISTIC_LUA_BLOCK_COMMENT)
        }
        DiffSyntaxLanguage::Jsonnet
        | DiffSyntaxLanguage::Cmake
        | DiffSyntaxLanguage::Dockerfile
        | DiffSyntaxLanguage::Just
        | DiffSyntaxLanguage::Caddyfile
        | DiffSyntaxLanguage::Gitignore
        | DiffSyntaxLanguage::Crontab
        | DiffSyntaxLanguage::CoffeeScript
        | DiffSyntaxLanguage::Python
        | DiffSyntaxLanguage::Toml
        | DiffSyntaxLanguage::Yaml
        | DiffSyntaxLanguage::Bash
        | DiffSyntaxLanguage::Makefile
        | DiffSyntaxLanguage::Ruby
        | DiffSyntaxLanguage::R
        | DiffSyntaxLanguage::GitCommit
        | DiffSyntaxLanguage::Elixir
        // Julia's block comment is `#= ... =#`; only the `#=` opener is modelled
        // here, which greys the opening line to its end and leaves the rest of the
        // block plain. The tree-sitter path has the real rule.
        | DiffSyntaxLanguage::Julia
        | DiffSyntaxLanguage::Perl => HeuristicCommentConfig::hash(),
        DiffSyntaxLanguage::PowerShell => {
            HeuristicCommentConfig::hash_block(HEURISTIC_POWERSHELL_BLOCK_COMMENT)
        }
        DiffSyntaxLanguage::Sql => HeuristicCommentConfig::line_block("--", HEURISTIC_C_BLOCK_COMMENT),
        DiffSyntaxLanguage::Rust
        | DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx
        | DiffSyntaxLanguage::Go
        | DiffSyntaxLanguage::GoMod
        | DiffSyntaxLanguage::GoWork
        | DiffSyntaxLanguage::C
        | DiffSyntaxLanguage::Cpp
        | DiffSyntaxLanguage::ObjectiveC
        | DiffSyntaxLanguage::CSharp
        | DiffSyntaxLanguage::Java
        | DiffSyntaxLanguage::Kotlin
        | DiffSyntaxLanguage::Swift
        | DiffSyntaxLanguage::Dart
        | DiffSyntaxLanguage::Scala
        | DiffSyntaxLanguage::Zig
        | DiffSyntaxLanguage::Groovy
        | DiffSyntaxLanguage::Solidity
        | DiffSyntaxLanguage::Gleam
        | DiffSyntaxLanguage::Dhall
        | DiffSyntaxLanguage::Kdl
        | DiffSyntaxLanguage::Ron
        | DiffSyntaxLanguage::Cue
        | DiffSyntaxLanguage::V
        | DiffSyntaxLanguage::Proto
        | DiffSyntaxLanguage::Bicep => HeuristicCommentConfig {
            line_comment: Some("//"),
            block_comment: match language {
                DiffSyntaxLanguage::GoMod | DiffSyntaxLanguage::GoWork => None,
                _ => Some(HEURISTIC_C_BLOCK_COMMENT),
            },
            ..HeuristicCommentConfig::base()
        },
        DiffSyntaxLanguage::Hcl | DiffSyntaxLanguage::Php => {
            HeuristicCommentConfig::line_hash_block("//", HEURISTIC_C_BLOCK_COMMENT)
        }
        // Nix looks like the Hcl arm above but must NOT take its `//` line
        // comment: `//` is Nix's attribute-set update operator, so
        // `{ a = 1; } // { b = 2; }` would grey out from the operator to the end
        // of the line. Nix comments are `#` and `/* */` only.
        DiffSyntaxLanguage::Nix => HeuristicCommentConfig::hash_block(HEURISTIC_C_BLOCK_COMMENT),
        DiffSyntaxLanguage::VisualBasic => HeuristicCommentConfig::visual_basic(),
        // CSV has no comment syntax at all -- every byte is data.
        DiffSyntaxLanguage::Csv
        | DiffSyntaxLanguage::Markdown
        | DiffSyntaxLanguage::MarkdownInline
        | DiffSyntaxLanguage::Css
        | DiffSyntaxLanguage::Jsdoc
        | DiffSyntaxLanguage::Json
        | DiffSyntaxLanguage::Diff
        | DiffSyntaxLanguage::Regex => HeuristicCommentConfig::none(),
    }
}

fn heuristic_block_comment_kind(language: DiffSyntaxLanguage) -> Option<HeuristicBlockCommentKind> {
    match language {
        DiffSyntaxLanguage::Html
        | DiffSyntaxLanguage::Xml
        | DiffSyntaxLanguage::Vue
        | DiffSyntaxLanguage::Svelte
        | DiffSyntaxLanguage::Jinja => Some(HeuristicBlockCommentKind::Html),
        // EBNF's comment is `(* ... *)`, the same as Pascal's and F#'s.
        DiffSyntaxLanguage::Ebnf
        | DiffSyntaxLanguage::Pascal
        | DiffSyntaxLanguage::FSharp
        | DiffSyntaxLanguage::OCaml
        | DiffSyntaxLanguage::OCamlInterface => Some(HeuristicBlockCommentKind::FSharp),
        DiffSyntaxLanguage::Haskell => Some(HeuristicBlockCommentKind::Haskell),
        DiffSyntaxLanguage::Lua => Some(HeuristicBlockCommentKind::Lua),
        DiffSyntaxLanguage::PowerShell => Some(HeuristicBlockCommentKind::PowerShell),
        DiffSyntaxLanguage::Sql
        | DiffSyntaxLanguage::Rust
        | DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx
        | DiffSyntaxLanguage::Go
        | DiffSyntaxLanguage::C
        | DiffSyntaxLanguage::Cpp
        | DiffSyntaxLanguage::ObjectiveC
        | DiffSyntaxLanguage::CSharp
        | DiffSyntaxLanguage::Java
        | DiffSyntaxLanguage::Kotlin
        | DiffSyntaxLanguage::Swift
        | DiffSyntaxLanguage::Dart
        | DiffSyntaxLanguage::Scala
        | DiffSyntaxLanguage::Zig
        | DiffSyntaxLanguage::Bicep
        | DiffSyntaxLanguage::Groovy
        | DiffSyntaxLanguage::Solidity
        | DiffSyntaxLanguage::Assembly
        | DiffSyntaxLanguage::Hcl
        | DiffSyntaxLanguage::Nix
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
        HeuristicBlockCommentKind::PowerShell => b"<#",
        HeuristicBlockCommentKind::Haskell => b"{-",
    }
}

fn heuristic_block_comment_end_bytes(kind: HeuristicBlockCommentKind) -> &'static [u8] {
    match kind {
        HeuristicBlockCommentKind::Html => b"-->",
        HeuristicBlockCommentKind::FSharp => b"*)",
        HeuristicBlockCommentKind::Lua => b"]]",
        HeuristicBlockCommentKind::C => b"*/",
        HeuristicBlockCommentKind::PowerShell => b"#>",
        HeuristicBlockCommentKind::Haskell => b"-}",
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

/// Length of the dash run opening a Haskell line comment, if it opens one.
///
/// Per the Haskell report a `dashes` lexeme is two or more consecutive dashes
/// *not* followed by a symbol character; otherwise the run is part of an operator.
/// `-- note` and `--- note` are comments, `-->` and `--|` are not.
fn haskell_dashes_line_comment_start_len(bytes: &[u8], start: usize) -> Option<usize> {
    // Haskell's symbol set minus `-` itself: the dash run is consumed first, so a
    // trailing dash can never be the character that disqualifies the run.
    const SYMBOL: &[u8] = b"!#$%&*+./<=>?@\\^|~:";

    let mut end = start;
    while bytes.get(end) == Some(&b'-') {
        end = end.saturating_add(1);
    }
    let run = end.saturating_sub(start);
    if run < 2 {
        return None;
    }
    match bytes.get(end) {
        Some(next) if SYMBOL.contains(next) => None,
        _ => Some(run),
    }
}

fn line_comment_start_len(
    bytes: &[u8],
    start: usize,
    config: HeuristicCommentConfig,
) -> Option<usize> {
    if config.visual_basic_line_comment {
        return visual_basic_line_comment_start_len(bytes, start);
    }
    if config.haskell_dashes_line_comment {
        return haskell_dashes_line_comment_start_len(bytes, start);
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
    if config.comment.haskell_dashes_line_comment && byte == b'-' {
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
                    // `potential_open_state_lead` over-approximates for `'`; this is
                    // where a prose apostrophe or a Nix identifier tick is refused.
                    if byte == b'\''
                        && !heuristic_single_quote_opens_string(
                            scan_config.single_quote,
                            bytes,
                            local,
                        )
                    {
                        local += 1;
                        *abs = abs.saturating_add(1);
                        record_constant!(*abs, HeuristicOpenState::Normal);
                        continue;
                    }
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

#[cfg(any(test, feature = "test-support"))]
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

    // Every line-comment spelling is decided by `line_comment_start_len`, the same
    // function the streamed scanner uses. Two independent copies of "does a comment
    // start here" is how Haskell's dash rule ended up applying on one path and not
    // the other.
    line_comment_start_len(text.as_bytes(), start, config).map(|_| start..text.len())
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

/// What a `'` means to the heuristic scanner, which has no grammar to ask.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HeuristicSingleQuote {
    /// Opens a string anywhere. The C-family default, and what Rust byte and char
    /// literals (`b'x'`, `'\n'`) need.
    String,
    /// Opens a string only where a value can start.
    ///
    /// Markup is mostly prose, and prose is full of apostrophes: an unconditional
    /// rule painted the rest of the line from the first `It's`, and on the streamed
    /// path the state then bled across lines. The preceding byte separates the two
    /// uses -- a contraction follows what a value *ends* with, a word byte (`It's`)
    /// or a closing delimiter (`{{ user.name }}'s`), while a quote follows an
    /// operator, an opening delimiter or whitespace (`class='x'`,
    /// `{{ x|default('n/a') }}`).
    ValuePositionOnly,
    /// Never a string; part of an identifier instead.
    ///
    /// Nix: `'` is a legal identifier character, so `lib.foldl'` and `mapAttrs'` are
    /// one token each. Nix has no single-quoted string -- only `"..."` and the
    /// indented `''...''` -- so nothing is lost. The `''...''` form is not
    /// represented here and renders unhighlighted rather than mis-highlighted.
    Identifier,
}

pub(super) fn heuristic_single_quote_role(language: DiffSyntaxLanguage) -> HeuristicSingleQuote {
    match language {
        // Haskell primes an identifier (`foldl'`, `go'`), OCaml opens a type
        // variable with one (`'a list`), and Clojure quotes a form (`'sym`). All
        // three would otherwise run a string to the end of the line. The char
        // literals they also spell with `'` are the cost, exactly as for Nix.
        DiffSyntaxLanguage::Nix
        | DiffSyntaxLanguage::Haskell
        | DiffSyntaxLanguage::OCaml
        | DiffSyntaxLanguage::OCamlInterface
        | DiffSyntaxLanguage::Clojure => HeuristicSingleQuote::Identifier,
        // Julia's postfix `'` is the adjoint operator: `A'` transposes, `'c'` is a
        // character. The two are told apart by what precedes the quote, which is
        // what ValuePositionOnly already tests for.
        //
        // The Z80's shadow registers are the same shape: `AF'`, `BC'`, `DE'` and
        // `HL'` end in an apostrophe, in a language that also spells character
        // literals with one. `EX AF, AF'` opened a string that ran to the end of
        // the file -- and the heuristic is what the app falls back to whenever the
        // parse budget blows, so fixing the grammar alone (see
        // vendor/tree-sitter-asm's `shadow_reg`) left this half broken.
        //
        // The cost is a char literal written with no separator before it --
        // `DB'A'` -- which no assembler style writes.
        //
        // `.env` is here for the same reason with a different cause: an apostrophe
        // inside an unquoted value -- `NAME=it's fine` -- is ordinary text to a
        // dotenv loader, and treating it as a quote runs a string to the end of
        // the file. That is exactly what mapping `.env` to Bash used to do, and
        // the dedicated dotenv grammars error on the same line.
        DiffSyntaxLanguage::Julia | DiffSyntaxLanguage::Assembly | DiffSyntaxLanguage::Dotenv => {
            HeuristicSingleQuote::ValuePositionOnly
        }
        DiffSyntaxLanguage::Html
        | DiffSyntaxLanguage::Xml
        | DiffSyntaxLanguage::Vue
        | DiffSyntaxLanguage::Svelte
        | DiffSyntaxLanguage::Jinja => HeuristicSingleQuote::ValuePositionOnly,
        _ => HeuristicSingleQuote::String,
    }
}

fn heuristic_single_quote_opens_string(
    role: HeuristicSingleQuote,
    bytes: &[u8],
    index: usize,
) -> bool {
    match role {
        HeuristicSingleQuote::String => true,
        HeuristicSingleQuote::Identifier => false,
        HeuristicSingleQuote::ValuePositionOnly => index
            .checked_sub(1)
            .and_then(|prev| bytes.get(prev))
            .is_none_or(|prev| {
                !prev.is_ascii_alphanumeric()
                    && !matches!(prev, b'_' | b'}' | b')' | b']' | b'>' | b'"' | b'\'')
            }),
    }
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

pub fn syntax_tokens_for_line_heuristic_into(
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
    let single_quote = heuristic_single_quote_role(language);
    let highlight_css_selectors = matches!(language, DiffSyntaxLanguage::Css);
    // A `.conf` has no grammar behind it, so the one structural fact the
    // heuristic can still state is which word names the setting: in Apache,
    // nginx, and generic key-value config alike it is the first word on the line.
    let highlight_leading_directive = matches!(
        language,
        DiffSyntaxLanguage::Conf | DiffSyntaxLanguage::Dotenv
    );

    let is_ident_start = |byte: u8| byte == b'_' || byte.is_ascii_alphabetic();
    // `'` continues an identifier only where it is not a quote -- Nix's `foldl'`.
    let ident_tick = single_quote == HeuristicSingleQuote::Identifier;
    let is_ident_continue = move |byte: u8| {
        byte == b'_' || byte.is_ascii_alphanumeric() || (ident_tick && byte == b'\'')
    };
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
            || (comment_config.haskell_dashes_line_comment && byte == b'-')
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

        if highlight_leading_directive {
            // A `;` that opens a line is an INI-dialect comment; one anywhere else
            // is nginx ending a statement. See `heuristic_comment_config`.
            if byte == b';' && bytes[..i].iter().all(u8::is_ascii_whitespace) {
                tokens.push(SyntaxToken {
                    range: i..len,
                    kind: SyntaxTokenKind::Comment,
                });
                break;
            }
            // Apache nests blocks in XML-ish section tags -- `<VirtualHost *:80>`,
            // `</Directory>`, `<IfModule !mod_authz_core.c>`. A closing tag is a
            // whole line with nothing else on it, so without this every one of
            // them rendered as plain text.
            if byte == b'<' && bytes[..i].iter().all(u8::is_ascii_whitespace) {
                let mut j = i.saturating_add(1);
                if bytes.get(j) == Some(&b'/') {
                    j = j.saturating_add(1);
                }
                let name_start = j;
                while j < len && is_ident_continue(bytes[j]) {
                    j += 1;
                }
                if j > name_start {
                    // Take the `>` too when the tag ends the line, so a closing
                    // tag is coloured whole rather than all but its last byte.
                    if bytes.get(j) == Some(&b'>') {
                        j = j.saturating_add(1);
                    }
                    tokens.push(SyntaxToken {
                        range: i..j,
                        kind: SyntaxTokenKind::Tag,
                    });
                    i = j;
                    continue;
                }
            }
            // nginx and Apache both nest blocks in braces, and nothing else here
            // would colour them: there is no tree, so `syntax/pairs.rs` cannot
            // match them either, but a visible delimiter still reads as structure.
            if matches!(byte, b'{' | b'}') {
                tokens.push(SyntaxToken {
                    range: i..i.saturating_add(1),
                    kind: SyntaxTokenKind::PunctuationBracket,
                });
                i = i.saturating_add(1);
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

        if (byte == b'"'
            || (byte == b'\'' && heuristic_single_quote_opens_string(single_quote, bytes, i)))
            || (allow_backtick_strings && byte == b'`')
        {
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
            } else if highlight_leading_directive && bytes[..i].iter().all(u8::is_ascii_whitespace)
            {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Property,
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

#[cfg(test)]
mod streamed_open_state_tests {
    use super::*;

    fn tail_state(text: &str, language: DiffSyntaxLanguage) -> HeuristicOpenState {
        let bytes = text.as_bytes();
        let mut state = HeuristicOpenState::Normal;
        let mut abs = 0usize;
        let mut recorder: Option<HeuristicCheckpointRecorder<'_>> = None;
        advance_streamed_heuristic_open_state_ascii(
            bytes,
            bytes.len(),
            language,
            &mut state,
            &mut abs,
            &mut recorder,
        );
        assert_eq!(abs, bytes.len(), "the scanner must consume the whole slice");
        state
    }

    fn is_open_string(state: HeuristicOpenState) -> bool {
        matches!(state, HeuristicOpenState::String { .. })
    }

    /// The nastier half of the apostrophe bug: the string state has no newline
    /// reset, so once opened it stayed open across every following line until the
    /// next stray `'`. A per-line assertion cannot see that.
    #[test]
    fn a_word_apostrophe_never_opens_the_streamed_string_state() {
        for (language, text) in [
            (
                DiffSyntaxLanguage::Jinja,
                "<p>It's a test</p>\n<div class=\"x\">plain</div>\n",
            ),
            (
                DiffSyntaxLanguage::Html,
                "<p>don't panic</p>\n<span>next line</span>\n",
            ),
            (
                DiffSyntaxLanguage::Nix,
                "  x = lib.foldl' add 0 xs;\n  y = 2;\n",
            ),
        ] {
            assert!(
                !is_open_string(tail_state(text, language)),
                "{language:?} left the string state open across lines for {text:?}"
            );
        }
    }

    /// ... while a real unterminated quote still opens it.
    #[test]
    fn an_unterminated_value_quote_still_opens_the_streamed_string_state() {
        assert!(
            is_open_string(tail_state("<div class='card", DiffSyntaxLanguage::Html)),
            "a genuine unterminated attribute quote must still open the state"
        );
        assert!(
            !is_open_string(tail_state("<div class='card'>", DiffSyntaxLanguage::Html)),
            "and close again at the matching quote"
        );
        assert!(
            is_open_string(tail_state("let s = \"open", DiffSyntaxLanguage::Nix)),
            "Nix double quotes are unaffected by the identifier rule"
        );
    }
}
