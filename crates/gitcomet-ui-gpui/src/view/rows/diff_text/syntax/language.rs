use super::*;

fn diff_syntax_language_for_identifier(identifier: &str) -> Option<DiffSyntaxLanguage> {
    Some(match identifier {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn" | "mdx" | "mdc" => {
            DiffSyntaxLanguage::Markdown
        }
        "markdown-inline" | "markdown_inline" => DiffSyntaxLanguage::MarkdownInline,
        "html" | "htm" => DiffSyntaxLanguage::Html,
        "xml" | "svg" | "xsl" | "xslt" | "xsd" | "xhtml" | "plist" | "csproj" | "fsproj"
        | "vbproj" | "sln" | "props" | "targets" | "resx" | "xaml" | "wsdl" | "rss" | "atom"
        | "opml" | "glade" | "ui" | "iml" => DiffSyntaxLanguage::Xml,
        "css" | "less" | "sass" | "scss" | "postcss" | "pcss" => DiffSyntaxLanguage::Css,
        "hcl" | "tf" | "tfvars" => DiffSyntaxLanguage::Hcl,
        "bicep" => DiffSyntaxLanguage::Bicep,
        "lua" => DiffSyntaxLanguage::Lua,
        "mk" | "make" | "makefile" | "gnumakefile" => DiffSyntaxLanguage::Makefile,
        "kt" | "kts" | "kotlin" => DiffSyntaxLanguage::Kotlin,
        "zig" => DiffSyntaxLanguage::Zig,
        "rs" | "rust" => DiffSyntaxLanguage::Rust,
        "py" | "python" | "pyi" | "mpy" => DiffSyntaxLanguage::Python,
        "js" | "mjs" | "cjs" | "javascript" => DiffSyntaxLanguage::JavaScript,
        "jsx" => DiffSyntaxLanguage::Tsx,
        "ts" | "cts" | "mts" | "typescript" => DiffSyntaxLanguage::TypeScript,
        "tsx" => DiffSyntaxLanguage::Tsx,
        "go" | "golang" => DiffSyntaxLanguage::Go,
        "gomod" | "go.mod" => DiffSyntaxLanguage::GoMod,
        "gowork" | "go.work" => DiffSyntaxLanguage::GoWork,
        "c" | "h" => DiffSyntaxLanguage::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "cppm" | "ixx" | "cu" | "cuh"
        | "ipp" | "inl" | "ccm" | "cxxm" | "c++m" | "h++" => DiffSyntaxLanguage::Cpp,
        "m" | "objc" | "objective-c" => DiffSyntaxLanguage::ObjectiveC,
        "cs" | "c#" | "csharp" => DiffSyntaxLanguage::CSharp,
        "fs" | "fsx" | "fsi" | "f#" | "fsharp" => DiffSyntaxLanguage::FSharp,
        "vb" | "vbs" | "vbnet" | "visualbasic" => DiffSyntaxLanguage::VisualBasic,
        "java" => DiffSyntaxLanguage::Java,
        "php" | "phtml" => DiffSyntaxLanguage::Php,
        "rb" | "ruby" => DiffSyntaxLanguage::Ruby,
        "ps1" | "psm1" | "psd1" | "powershell" | "pwsh" => DiffSyntaxLanguage::PowerShell,
        "swift" => DiffSyntaxLanguage::Swift,
        "r" => DiffSyntaxLanguage::R,
        "dart" => DiffSyntaxLanguage::Dart,
        "scala" | "sc" | "sbt" => DiffSyntaxLanguage::Scala,
        "pl" | "pm" | "perl" => DiffSyntaxLanguage::Perl,
        "json" | "jsonc" | "geojson" | "topojson" | "flake.lock" | "bun.lock" | ".prettierrc"
        | "prettierrc" | ".babelrc" | "babelrc" | ".eslintrc" | "eslintrc" | ".stylelintrc"
        | "stylelintrc" | ".jshintrc" | "jshintrc" | ".swcrc" | "swcrc" | ".luaurc" | "luaurc" => {
            DiffSyntaxLanguage::Json
        }
        "toml" => DiffSyntaxLanguage::Toml,
        "yaml" | "yml" | "pixi.lock" | ".clang-format" | "clang-format" | ".clangd" | "clangd"
        | "bst" => DiffSyntaxLanguage::Yaml,
        "sql" => DiffSyntaxLanguage::Sql,
        "diff" | "patch" => DiffSyntaxLanguage::Diff,
        "commit_editmsg" | "merge_msg" | "tag_editmsg" | "notes_editmsg" | "edit_description"
        | "gitcommit" | "git-commit" => DiffSyntaxLanguage::GitCommit,
        "sh" | "bash" | "zsh" | "shell" | "shellscript" | "console" | ".env" | ".bashrc"
        | "bashrc" | ".bash_profile" | "bash_profile" | ".bash_aliases" | "bash_aliases"
        | ".bash_logout" | "bash_logout" | ".profile" | "profile" | ".zshrc" | "zshrc"
        | ".zshenv" | "zshenv" | ".zsh_profile" | "zsh_profile" | ".zsh_aliases"
        | "zsh_aliases" | ".zsh_histfile" | "zsh_histfile" | ".zlogin" | "zlogin" | ".zprofile"
        | "zprofile" | "bats" | "pkgbuild" | "apkbuild" => DiffSyntaxLanguage::Bash,
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
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::Markdown => Some((
            tree_sitter_md::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                MARKDOWN_HIGHLIGHTS_QUERY,
                MARKDOWN_INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::MarkdownInline => Some((
            tree_sitter_md::INLINE_LANGUAGE.into(),
            TreesitterQueryAsset::highlights(MARKDOWN_INLINE_HIGHLIGHTS_QUERY),
        )),
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
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Bicep => Some((
            tree_sitter_bicep::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_bicep::HIGHLIGHTS_QUERY,
                tree_sitter_bicep::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Lua => Some((
            tree_sitter_lua::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_lua::HIGHLIGHTS_QUERY,
                tree_sitter_lua::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Makefile => Some((
            tree_sitter_make::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_make::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Kotlin => Some((
            tree_sitter_kotlin_sg::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Zig => Some((
            tree_sitter_zig::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_zig::HIGHLIGHTS_QUERY,
                tree_sitter_zig::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => Some((
            tree_sitter_rust::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(RUST_HIGHLIGHTS_QUERY, RUST_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => Some((
            tree_sitter_python::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(PYTHON_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => Some((
            tree_sitter_go::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(GO_HIGHLIGHTS_QUERY, GO_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GoMod => Some((
            tree_sitter_gomod::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GOMOD_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GoWork => Some((
            tree_sitter_gowork::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GOWORK_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::C => Some((
            tree_sitter_c::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_c::HIGHLIGHT_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Cpp => Some((
            tree_sitter_cpp::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_cpp::HIGHLIGHT_QUERY,
                CPP_INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::ObjectiveC => Some((
            tree_sitter_objc::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_objc::HIGHLIGHTS_QUERY,
                tree_sitter_objc::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::CSharp => Some((
            tree_sitter_c_sharp::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CSHARP_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::FSharp => Some((
            tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_fsharp::HIGHLIGHTS_QUERY,
                tree_sitter_fsharp::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Java => Some((
            tree_sitter_java::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_java::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Php => Some((
            tree_sitter_php::LANGUAGE_PHP.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_php::HIGHLIGHTS_QUERY,
                tree_sitter_php::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Ruby => Some((
            tree_sitter_ruby::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_ruby::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::PowerShell => Some((
            tree_sitter_powershell::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(POWERSHELL_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Swift => Some((
            tree_sitter_swift::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_swift::HIGHLIGHTS_QUERY,
                tree_sitter_swift::INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::R => Some((
            tree_sitter_r::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_r::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Dart => Some((
            tree_sitter_dart::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_dart::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Scala => Some((
            tree_sitter_scala::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_scala::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => Some((
            tree_sitter_json::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JSON_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Toml => Some((
            tree_sitter_toml_ng::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => Some((
            tree_sitter_yaml::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(YAML_HIGHLIGHTS_QUERY, YAML_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Sql => Some((
            tree_sitter_sequel::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_sequel::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::Diff => Some((
            tree_sitter_diff::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_diff::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GitCommit => Some((
            tree_sitter_gitcommit::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GITCOMMIT_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::TypeScript => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TreesitterQueryAsset::with_injections(
                TYPESCRIPT_HIGHLIGHTS_QUERY,
                TYPESCRIPT_INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Tsx => Some((
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TreesitterQueryAsset::with_injections(TSX_HIGHLIGHTS_QUERY, TSX_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::JavaScript => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                JAVASCRIPT_HIGHLIGHTS_QUERY,
                JAVASCRIPT_INJECTIONS_QUERY,
            ),
        )),
        #[cfg(any(test, feature = "syntax-shell"))]
        DiffSyntaxLanguage::Bash => Some((
            tree_sitter_bash::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(BASH_HIGHLIGHTS_QUERY),
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
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::Markdown => highlight_spec_entry!(Markdown),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::MarkdownInline => highlight_spec_entry!(MarkdownInline),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Html => highlight_spec_entry!(Html),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Css => highlight_spec_entry!(Css),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Bicep => highlight_spec_entry!(Bicep),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Lua => highlight_spec_entry!(Lua),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Makefile => highlight_spec_entry!(Makefile),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Kotlin => highlight_spec_entry!(Kotlin),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Zig => highlight_spec_entry!(Zig),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => highlight_spec_entry!(Rust),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => highlight_spec_entry!(Python),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => highlight_spec_entry!(Go),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GoMod => highlight_spec_entry!(GoMod),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GoWork => highlight_spec_entry!(GoWork),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::C => highlight_spec_entry!(C),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Cpp => highlight_spec_entry!(Cpp),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::ObjectiveC => highlight_spec_entry!(ObjectiveC),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::CSharp => highlight_spec_entry!(CSharp),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::FSharp => highlight_spec_entry!(FSharp),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Java => highlight_spec_entry!(Java),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Php => highlight_spec_entry!(Php),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Ruby => highlight_spec_entry!(Ruby),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::PowerShell => highlight_spec_entry!(PowerShell),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Swift => highlight_spec_entry!(Swift),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::R => highlight_spec_entry!(R),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Dart => highlight_spec_entry!(Dart),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Scala => highlight_spec_entry!(Scala),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => highlight_spec_entry!(Json),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Toml => highlight_spec_entry!(Toml),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => highlight_spec_entry!(Yaml),
        #[cfg(any(test, feature = "syntax-extra"))]
        DiffSyntaxLanguage::Sql => highlight_spec_entry!(Sql),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::Diff => highlight_spec_entry!(Diff),
        #[cfg(any(test, feature = "syntax-repo"))]
        DiffSyntaxLanguage::GitCommit => highlight_spec_entry!(GitCommit),
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
        "comment.doc" | "comment.documentation" => SyntaxTokenKind::CommentDoc,
        "comment" => SyntaxTokenKind::Comment,
        // Strings
        "escape" | "string.escape" => SyntaxTokenKind::StringEscape,
        "string" | "string.special" | "string.regex" | "character" => SyntaxTokenKind::String,
        "diff.plus" => SyntaxTokenKind::String,
        "diff.minus" => SyntaxTokenKind::Keyword,
        "diff.delta" => SyntaxTokenKind::Type,
        // Keywords
        "conditional" | "keyword.control" | "repeat" => SyntaxTokenKind::KeywordControl,
        "exception"
        | "keyword"
        | "keyword.declaration"
        | "keyword.import"
        | "include"
        | "storageclass"
        | "preproc" => SyntaxTokenKind::Keyword,
        // Numbers & booleans
        "float" | "number" | "number.float" => SyntaxTokenKind::Number,
        "boolean" => SyntaxTokenKind::Boolean,
        // Functions
        "function.method" => SyntaxTokenKind::FunctionMethod,
        "function.special" | "function.special.definition" => SyntaxTokenKind::FunctionSpecial,
        "function" | "function.definition" | "constructor" | "method" => SyntaxTokenKind::Function,
        // Types
        "module.builtin" | "type.builtin" => SyntaxTokenKind::TypeBuiltin,
        "concept" | "type.interface" => SyntaxTokenKind::TypeInterface,
        "array" | "module" | "namespace" | "selector" | "type" | "type.class" => {
            SyntaxTokenKind::Type
        }
        // Variables - general `@variable` renders as plain text (no color) to avoid
        // "everything is highlighted" noise. Sub-captures get distinct treatment.
        "parameter" | "variable.parameter" => SyntaxTokenKind::VariableParameter,
        "variable.builtin" | "variable.special" => SyntaxTokenKind::VariableSpecial,
        "variable.member" | "variable.other.member" => SyntaxTokenKind::Property,
        "variable" => SyntaxTokenKind::Variable,
        // Properties
        "field" | "property" | "property.definition" => SyntaxTokenKind::Property,
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
        "delimiter" | "punctuation.delimiter" => SyntaxTokenKind::PunctuationDelimiter,
        "punctuation" | "punctuation.special" => SyntaxTokenKind::Punctuation,
        // Lifetime (Rust)
        "lifetime" => SyntaxTokenKind::Lifetime,
        // Labels (goto, DTD notation names)
        "label" => SyntaxTokenKind::Variable,
        // Markup (XML text content, CDATA, URIs)
        "link_uri.markup" | "markup.link" => SyntaxTokenKind::String,
        "markup.raw" | "text.literal.markup" => SyntaxTokenKind::String,
        "markup.heading" | "title.markup" => SyntaxTokenKind::Keyword,
        "emphasis.markup"
        | "emphasis.strong.markup"
        | "link_text.markup"
        | "markup"
        | "strikethrough.markup" => SyntaxTokenKind::Variable,
        // Skip `@none`, `@embedded`, `@text.*` and other non-semantic captures
        _ => return None,
    })
}
