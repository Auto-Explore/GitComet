use super::*;

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
