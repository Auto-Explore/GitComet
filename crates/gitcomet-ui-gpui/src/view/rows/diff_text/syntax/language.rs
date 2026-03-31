use super::*;

fn diff_syntax_language_for_identifier(identifier: &str) -> Option<DiffSyntaxLanguage> {
    Some(match identifier {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn" => DiffSyntaxLanguage::Markdown,
        "html" | "htm" => DiffSyntaxLanguage::Html,
        "xml" | "svg" | "xsl" | "xslt" | "xsd" | "xhtml" | "plist" | "csproj" | "fsproj"
        | "vbproj" | "sln" | "props" | "targets" | "resx" | "xaml" | "wsdl" | "rss" | "atom"
        | "opml" | "glade" | "ui" | "iml" => DiffSyntaxLanguage::Xml,
        "css" | "less" | "sass" | "scss" => DiffSyntaxLanguage::Css,
        "hcl" | "tf" | "tfvars" => DiffSyntaxLanguage::Hcl,
        "bicep" => DiffSyntaxLanguage::Bicep,
        "lua" => DiffSyntaxLanguage::Lua,
        "mk" | "make" | "makefile" | "gnumakefile" => DiffSyntaxLanguage::Makefile,
        "kt" | "kts" | "kotlin" => DiffSyntaxLanguage::Kotlin,
        "zig" => DiffSyntaxLanguage::Zig,
        "rs" | "rust" => DiffSyntaxLanguage::Rust,
        "py" | "python" => DiffSyntaxLanguage::Python,
        "js" | "mjs" | "cjs" | "javascript" => DiffSyntaxLanguage::JavaScript,
        "jsx" => DiffSyntaxLanguage::Tsx,
        "ts" | "cts" | "mts" | "typescript" => DiffSyntaxLanguage::TypeScript,
        "tsx" => DiffSyntaxLanguage::Tsx,
        "go" | "golang" => DiffSyntaxLanguage::Go,
        "c" | "h" => DiffSyntaxLanguage::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" => DiffSyntaxLanguage::Cpp,
        "cs" | "c#" | "csharp" => DiffSyntaxLanguage::CSharp,
        "fs" | "fsx" | "fsi" | "f#" | "fsharp" => DiffSyntaxLanguage::FSharp,
        "vb" | "vbs" | "vbnet" | "visualbasic" => DiffSyntaxLanguage::VisualBasic,
        "java" => DiffSyntaxLanguage::Java,
        "php" | "phtml" => DiffSyntaxLanguage::Php,
        "rb" | "ruby" => DiffSyntaxLanguage::Ruby,
        "json" => DiffSyntaxLanguage::Json,
        "toml" => DiffSyntaxLanguage::Toml,
        "yaml" | "yml" => DiffSyntaxLanguage::Yaml,
        "sql" => DiffSyntaxLanguage::Sql,
        "sh" | "bash" | "zsh" | "shell" | "console" => DiffSyntaxLanguage::Bash,
        _ => return None,
    })
}

pub(in crate::view) fn diff_syntax_language_for_path(
    path: impl AsRef<std::path::Path>,
) -> Option<DiffSyntaxLanguage> {
    let p = path.as_ref();
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    let ext = ascii_lowercase_for_match(ext);
    diff_syntax_language_for_identifier(ext.as_ref()).or_else(|| {
        let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let file_name = ascii_lowercase_for_match(file_name);
        diff_syntax_language_for_identifier(file_name.as_ref())
    })
}

pub(in crate::view) fn diff_syntax_language_for_code_fence_info(
    info: &str,
) -> Option<DiffSyntaxLanguage> {
    let token = info
        .trim()
        .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .find(|segment| !segment.is_empty())?;
    let token = token.trim_matches(|ch| matches!(ch, '{' | '}'));
    let token = token.trim_start_matches('.');
    let token = token.strip_prefix("language-").unwrap_or(token);
    let token = ascii_lowercase_for_match(token);
    diff_syntax_language_for_identifier(token.as_ref())
}

pub(super) fn empty_line_syntax_tokens() -> Arc<[SyntaxToken]> {
    static EMPTY: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from([])))
}

fn should_cache_single_line_syntax_tokens(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TREESITTER_LINE_BYTES
}

fn single_line_syntax_token_cache_key(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
) -> SingleLineSyntaxTokenCacheKey {
    SingleLineSyntaxTokenCacheKey {
        language,
        mode,
        text_hash: treesitter_text_hash(text),
    }
}

fn syntax_tokens_for_line_uncached(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Vec<SyntaxToken> {
    if matches!(language, DiffSyntaxLanguage::Markdown) {
        return syntax_tokens_for_line_markdown(text);
    }

    match mode {
        DiffSyntaxMode::HeuristicOnly => syntax_tokens_for_line_heuristic(text, language),
        DiffSyntaxMode::Auto => {
            if matches!(language, DiffSyntaxLanguage::Yaml) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if !should_use_treesitter_for_line(text) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if is_heuristic_sufficient_for_line(text, language) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if let Some(tokens) = syntax_tokens_for_line_treesitter(text, language) {
                return tokens;
            }
            syntax_tokens_for_line_heuristic(text, language)
        }
    }
}

pub(in super::super) fn syntax_tokens_for_line_shared(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Arc<[SyntaxToken]> {
    if text.is_empty() {
        return empty_line_syntax_tokens();
    }

    if !should_cache_single_line_syntax_tokens(text) {
        return Arc::from(syntax_tokens_for_line_uncached(text, language, mode));
    }

    let key = single_line_syntax_token_cache_key(language, mode, text);
    if let Some(tokens) = TS_LINE_TOKEN_CACHE.with(|cache| cache.borrow_mut().get(key, text)) {
        return tokens;
    }

    let tokens: Arc<[SyntaxToken]> =
        Arc::from(syntax_tokens_for_line_uncached(text, language, mode));
    TS_LINE_TOKEN_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, text, Arc::clone(&tokens));
    });
    tokens
}

#[cfg(test)]
pub(in super::super) fn syntax_tokens_for_line(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Vec<SyntaxToken> {
    syntax_tokens_for_line_shared(text, language, mode)
        .as_ref()
        .to_vec()
}

/// Single source of truth for tree-sitter grammar + query asset per language.
/// Returns `None` for languages without a wired tree-sitter grammar.
pub(super) fn tree_sitter_grammar(
    language: DiffSyntaxLanguage,
) -> Option<(tree_sitter::Language, TreesitterQueryAsset)> {
    match language {
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Html => Some((
            tree_sitter_html::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(HTML_HIGHLIGHTS_QUERY, HTML_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Css => Some((
            tree_sitter_css::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CSS_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => Some((
            tree_sitter_rust::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(RUST_HIGHLIGHTS_QUERY, RUST_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => Some((
            tree_sitter_python::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_python::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => Some((
            tree_sitter_go::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_go::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => Some((
            tree_sitter_json::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_json::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => Some((
            tree_sitter_yaml::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_yaml::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::TypeScript => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TreesitterQueryAsset::highlights(TYPESCRIPT_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Tsx => Some((
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TreesitterQueryAsset::highlights(TSX_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::JavaScript => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JAVASCRIPT_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-shell"))]
        DiffSyntaxLanguage::Bash => Some((
            tree_sitter_bash::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_bash::HIGHLIGHT_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-xml"))]
        DiffSyntaxLanguage::Xml => Some((
            tree_sitter_xml::LANGUAGE_XML.into(),
            TreesitterQueryAsset::highlights(XML_HIGHLIGHTS_QUERY),
        )),
        // Languages without a wired tree-sitter grammar, or grammars gated off
        // by the current feature set, fall back to heuristic-only highlighting.
        _ => None,
    }
}

fn init_highlight_spec(language: DiffSyntaxLanguage) -> TreesitterHighlightSpec {
    let (ts_language, asset) =
        tree_sitter_grammar(language).expect("tree-sitter grammar should exist");
    let query = tree_sitter::Query::new(&ts_language, asset.highlights)
        .expect("highlights.scm should compile");
    let capture_kinds = query
        .capture_names()
        .iter()
        .map(|name| syntax_kind_from_capture_name(name))
        .collect::<Vec<_>>();
    let injection_query = asset.injections.map(|source| {
        tree_sitter::Query::new(&ts_language, source).expect("injections.scm should compile")
    });
    TreesitterHighlightSpec {
        ts_language,
        query,
        capture_kinds,
        injection_query,
    }
}

macro_rules! highlight_spec_entry {
    ($language_variant:ident) => {{
        static SPEC: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
        Some(SPEC.get_or_init(|| init_highlight_spec(DiffSyntaxLanguage::$language_variant)))
    }};
}

pub(super) fn tree_sitter_highlight_spec(
    language: DiffSyntaxLanguage,
) -> Option<&'static TreesitterHighlightSpec> {
    match language {
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Html => highlight_spec_entry!(Html),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Css => highlight_spec_entry!(Css),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => highlight_spec_entry!(Rust),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => highlight_spec_entry!(Python),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => highlight_spec_entry!(Go),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => highlight_spec_entry!(Json),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => highlight_spec_entry!(Yaml),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::TypeScript => highlight_spec_entry!(TypeScript),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Tsx => highlight_spec_entry!(Tsx),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::JavaScript => highlight_spec_entry!(JavaScript),
        #[cfg(any(test, feature = "syntax-shell"))]
        DiffSyntaxLanguage::Bash => highlight_spec_entry!(Bash),
        #[cfg(any(test, feature = "syntax-xml"))]
        DiffSyntaxLanguage::Xml => highlight_spec_entry!(Xml),
        _ => None,
    }
}

pub(super) fn syntax_kind_from_capture_name(mut name: &str) -> Option<SyntaxTokenKind> {
    // Try the full dotted capture name first and then progressively trim suffix
    // segments so vendored names like `punctuation.bracket.html` keep their
    // semantic class instead of collapsing all the way to `punctuation`.
    loop {
        if let Some(kind) = syntax_kind_for_capture_name(name) {
            return Some(kind);
        }

        let (prefix, _) = name.rsplit_once('.')?;
        name = prefix;
    }
}

fn syntax_kind_for_capture_name(name: &str) -> Option<SyntaxTokenKind> {
    Some(match name {
        // Comments
        "comment.doc" => SyntaxTokenKind::CommentDoc,
        "comment" => SyntaxTokenKind::Comment,
        // Strings
        "string.escape" => SyntaxTokenKind::StringEscape,
        "string" | "string.special" | "string.regex" | "character" => SyntaxTokenKind::String,
        // Keywords
        "keyword.control" => SyntaxTokenKind::KeywordControl,
        "keyword" | "keyword.declaration" | "keyword.import" | "include" | "preproc" => {
            SyntaxTokenKind::Keyword
        }
        // Numbers & booleans
        "number" => SyntaxTokenKind::Number,
        "boolean" => SyntaxTokenKind::Boolean,
        // Functions
        "function.method" => SyntaxTokenKind::FunctionMethod,
        "function.special" | "function.special.definition" => SyntaxTokenKind::FunctionSpecial,
        "function" | "function.definition" | "constructor" | "method" => SyntaxTokenKind::Function,
        // Types
        "type.builtin" => SyntaxTokenKind::TypeBuiltin,
        "type.interface" => SyntaxTokenKind::TypeInterface,
        "type" | "type.class" => SyntaxTokenKind::Type,
        // Variables - general `@variable` renders as plain text (no color) to avoid
        // "everything is highlighted" noise. Sub-captures get distinct treatment.
        "variable.parameter" => SyntaxTokenKind::VariableParameter,
        "variable.special" => SyntaxTokenKind::VariableSpecial,
        "variable" => SyntaxTokenKind::Variable,
        // Properties
        "property" | "field" => SyntaxTokenKind::Property,
        // Tags (HTML/JSX)
        "tag" | "tag.doctype" => SyntaxTokenKind::Tag,
        // Attributes
        "attribute" | "attribute.jsx" => SyntaxTokenKind::Attribute,
        // Constants
        "constant" | "constant.builtin" => SyntaxTokenKind::Constant,
        // Operators
        "operator" => SyntaxTokenKind::Operator,
        // Punctuation
        "punctuation.bracket" => SyntaxTokenKind::PunctuationBracket,
        "punctuation.delimiter" => SyntaxTokenKind::PunctuationDelimiter,
        "punctuation" | "punctuation.special" => SyntaxTokenKind::Punctuation,
        // Lifetime (Rust)
        "lifetime" => SyntaxTokenKind::Lifetime,
        // Labels (goto, DTD notation names)
        "label" => SyntaxTokenKind::Variable,
        // Markup (XML text content, CDATA, URIs)
        "markup.link" => SyntaxTokenKind::String,
        "markup.raw" => SyntaxTokenKind::String,
        "markup.heading" => SyntaxTokenKind::Keyword,
        "markup" => SyntaxTokenKind::Variable,
        // Selectors/namespaces map to Type for CSS/XML
        "namespace" | "selector" => SyntaxTokenKind::Type,
        // Skip `@none`, `@embedded`, `@text.*` and other non-semantic captures
        _ => return None,
    })
}
