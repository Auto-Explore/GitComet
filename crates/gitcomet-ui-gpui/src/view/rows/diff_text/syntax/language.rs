use super::*;

fn diff_syntax_language_for_identifier(identifier: &str) -> Option<DiffSyntaxLanguage> {
    Some(match identifier {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn" | "mdx" | "mdc" => {
            DiffSyntaxLanguage::Markdown
        }
        "markdown-inline" | "markdown_inline" => DiffSyntaxLanguage::MarkdownInline,
        "html" | "htm" => DiffSyntaxLanguage::Html,
        "vue" => DiffSyntaxLanguage::Vue,
        "svelte" => DiffSyntaxLanguage::Svelte,
        // The first six are the grammar's own declared file-types; `dj` is the
        // Django addition. The markup-bodied reading is the right default for a bare
        // identifier; `diff_syntax_language_for_path` splits off the rest.
        "njk" | "nunjucks" | "j2" | "jinja" | "jinja2" | "twig" | "dj" => DiffSyntaxLanguage::Jinja,
        "xml" | "svg" | "xsl" | "xslt" | "xsd" | "xhtml" | "plist" | "csproj" | "fsproj"
        | "vbproj" | "sln" | "props" | "targets" | "resx" | "xaml" | "wsdl" | "rss" | "atom"
        | "opml" | "glade" | "ui" | "iml" => DiffSyntaxLanguage::Xml,
        "css" | "less" | "sass" | "scss" | "postcss" | "pcss" => DiffSyntaxLanguage::Css,
        "hcl" | "tf" | "tfvars" => DiffSyntaxLanguage::Hcl,
        "bicep" => DiffSyntaxLanguage::Bicep,
        "lua" => DiffSyntaxLanguage::Lua,
        "nix" => DiffSyntaxLanguage::Nix,
        "mk" | "make" | "makefile" | "gnumakefile" => DiffSyntaxLanguage::Makefile,
        "kt" | "kts" | "kotlin" => DiffSyntaxLanguage::Kotlin,
        "zig" => DiffSyntaxLanguage::Zig,
        // `.gradle` is Groovy unless it is `.gradle.kts`, and that suffix resolves
        // to `kts` before this arm is ever reached. `Jenkinsfile` has no extension
        // and matches on the file-name pass.
        "groovy" | "gvy" | "gy" | "gsh" | "gradle" | "jenkinsfile" => DiffSyntaxLanguage::Groovy,
        "clj" | "cljs" | "cljc" | "cljd" | "edn" | "clojure" => DiffSyntaxLanguage::Clojure,
        "ex" | "exs" | "elixir" => DiffSyntaxLanguage::Elixir,
        "erl" | "hrl" | "escript" | "erlang" | "rebar.config" | "rebar.lock" => {
            DiffSyntaxLanguage::Erlang
        }
        // Not `.lhs`: literate Haskell is bird tracks or LaTeX with Haskell inside,
        // and the grammar parses neither.
        "hs" | "hs-boot" | "haskell" => DiffSyntaxLanguage::Haskell,
        "jl" | "julia" => DiffSyntaxLanguage::Julia,
        "ml" | "ocaml" => DiffSyntaxLanguage::OCaml,
        "mli" => DiffSyntaxLanguage::OCamlInterface,
        "sol" | "solidity" => DiffSyntaxLanguage::Solidity,
        // `.s` is lowercased from `.S` (preprocessed assembly) by the caller, which
        // is what we want -- both are assembly. `.asm` covers the MASM/NASM side.
        "asm" | "s" | "nasm" | "assembly" => DiffSyntaxLanguage::Assembly,
        "rs" | "rust" => DiffSyntaxLanguage::Rust,
        "py" | "python" | "pyi" | "mpy" => DiffSyntaxLanguage::Python,
        "js" | "mjs" | "cjs" | "javascript" => DiffSyntaxLanguage::JavaScript,
        "jsdoc" => DiffSyntaxLanguage::Jsdoc,
        "jsx" => DiffSyntaxLanguage::Tsx,
        "ts" | "cts" | "mts" | "typescript" => DiffSyntaxLanguage::TypeScript,
        "tsx" => DiffSyntaxLanguage::Tsx,
        "regex" | "regexp" => DiffSyntaxLanguage::Regex,
        "go" | "golang" => DiffSyntaxLanguage::Go,
        "gomod" | "go.mod" => DiffSyntaxLanguage::GoMod,
        "gowork" | "go.work" => DiffSyntaxLanguage::GoWork,
        "c" | "h" => DiffSyntaxLanguage::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "cppm" | "ixx" | "cu" | "cuh"
        | "ipp" | "inl" | "ino" | "ccm" | "cxxm" | "c++m" | "h++" => DiffSyntaxLanguage::Cpp,
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

/// Extensions whose body is HTML, so a template wrapping one keeps the injection.
fn jinja_body_is_markup(inner_extension: &str) -> bool {
    matches!(
        inner_extension,
        "" | "html" | "htm" | "xhtml" | "xml" | "svg" | "vue" | "hbs" | "mustache"
    )
}

/// `foo.j2` alone says nothing about what is being templated: `base.html.j2` is
/// HTML, `values.yaml.j2` and `deploy.sh.j2` are not, and injecting HTML into the
/// latter makes the HTML grammar open elements on `<<EOF` and `2>&1`. The inner
/// extension is the only signal available.
fn jinja_language_for_path(p: &std::path::Path) -> DiffSyntaxLanguage {
    let inner_extension = p
        .file_stem()
        .map(std::path::Path::new)
        .and_then(|stem| stem.extension())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let inner_extension = ascii_lowercase_for_match(inner_extension);
    if jinja_body_is_markup(inner_extension.as_ref()) {
        DiffSyntaxLanguage::Jinja
    } else {
        DiffSyntaxLanguage::JinjaText
    }
}

pub(in crate::view) fn diff_syntax_language_for_path(
    path: impl AsRef<std::path::Path>,
) -> Option<DiffSyntaxLanguage> {
    let p = path.as_ref();
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    if matches!(ext, "C" | "H") {
        return Some(DiffSyntaxLanguage::Cpp);
    }
    let ext = ascii_lowercase_for_match(ext);
    let resolved = diff_syntax_language_for_identifier(ext.as_ref()).or_else(|| {
        let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let file_name = ascii_lowercase_for_match(file_name);
        diff_syntax_language_for_identifier(file_name.as_ref())
    })?;
    Some(match resolved {
        DiffSyntaxLanguage::Jinja => jinja_language_for_path(p),
        other => other,
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
        .or_else(|| diff_syntax_language_for_path(token.as_ref()))
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
        DiffSyntaxLanguage::Markdown => Some((
            tree_sitter_md::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                MARKDOWN_HIGHLIGHTS_QUERY,
                MARKDOWN_INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::MarkdownInline => Some((
            tree_sitter_md::INLINE_LANGUAGE.into(),
            TreesitterQueryAsset::highlights(MARKDOWN_INLINE_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Html => Some((
            tree_sitter_html::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(HTML_HIGHLIGHTS_QUERY, HTML_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Jinja => Some((
            tree_sitter_jinja_dialects::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(JINJA_HIGHLIGHTS_QUERY, JINJA_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::JinjaText => Some((
            tree_sitter_jinja_dialects::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JINJA_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Vue => Some((
            tree_sitter_vue::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(VUE_HIGHLIGHTS_QUERY, VUE_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Svelte => Some((
            tree_sitter_svelte_ng::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(SVELTE_HIGHLIGHTS_QUERY, SVELTE_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Css => Some((
            tree_sitter_css::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CSS_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Bicep => Some((
            tree_sitter_bicep::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_bicep::HIGHLIGHTS_QUERY,
                tree_sitter_bicep::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Nix => Some((
            tree_sitter_nix::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(NIX_HIGHLIGHTS_QUERY, NIX_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Lua => Some((
            tree_sitter_lua::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_lua::HIGHLIGHTS_QUERY,
                tree_sitter_lua::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Makefile => Some((
            tree_sitter_make::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_make::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Kotlin => Some((
            tree_sitter_kotlin_sg::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Zig => Some((
            tree_sitter_zig::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_zig::HIGHLIGHTS_QUERY,
                tree_sitter_zig::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Groovy => Some((
            dekobon_tree_sitter_groovy::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(dekobon_tree_sitter_groovy::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Clojure => Some((
            tree_sitter_clojure_orchard::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CLOJURE_HIGHLIGHTS_QUERY),
        )),
        // Highlights only. `tree_sitter_elixir::INJECTIONS_QUERY` sets
        // `injection.combined` on every one of its seven sigil patterns, and a
        // combined layer is parsed as one document via set_included_ranges -- see
        // `combined_injection_declarations_are_exactly_the_known_set`. Wiring it up
        // is a deliberate decision about clipping and cache behaviour, not a
        // drop-in, and `~H` sigils need a HEEx grammar we do not have anyway.
        DiffSyntaxLanguage::Elixir => Some((
            tree_sitter_elixir::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_elixir::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Erlang => Some((
            tree_sitter_erlang::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_erlang::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Haskell => Some((
            tree_sitter_haskell::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_haskell::HIGHLIGHTS_QUERY,
                tree_sitter_haskell::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Julia => Some((
            tree_sitter_julia::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JULIA_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::OCaml => Some((
            tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            TreesitterQueryAsset::highlights(OCAML_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::OCamlInterface => Some((
            tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
            TreesitterQueryAsset::highlights(OCAML_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Solidity => Some((
            tree_sitter_solidity::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(SOLIDITY_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Assembly => Some((
            tree_sitter_asm::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_asm::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Rust => Some((
            tree_sitter_rust::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(RUST_HIGHLIGHTS_QUERY, RUST_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Python => Some((
            tree_sitter_python::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(PYTHON_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Go => Some((
            tree_sitter_go::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(GO_HIGHLIGHTS_QUERY, GO_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::GoMod => Some((
            tree_sitter_gomod::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GOMOD_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::GoWork => Some((
            tree_sitter_gowork::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GOWORK_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::C => Some((
            tree_sitter_c::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(C_HIGHLIGHTS_QUERY, C_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Cpp => Some((
            tree_sitter_cpp::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(CPP_HIGHLIGHTS_QUERY, CPP_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::ObjectiveC => Some((
            tree_sitter_objc::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_objc::HIGHLIGHTS_QUERY,
                tree_sitter_objc::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::CSharp => Some((
            tree_sitter_c_sharp::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CSHARP_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::FSharp => Some((
            tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_fsharp::HIGHLIGHTS_QUERY,
                tree_sitter_fsharp::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Java => Some((
            tree_sitter_java::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_java::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Php => Some((
            tree_sitter_php::LANGUAGE_PHP.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_php::HIGHLIGHTS_QUERY,
                tree_sitter_php::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Ruby => Some((
            tree_sitter_ruby::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_ruby::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::PowerShell => Some((
            tree_sitter_powershell::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(POWERSHELL_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Swift => Some((
            tree_sitter_swift::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                tree_sitter_swift::HIGHLIGHTS_QUERY,
                tree_sitter_swift::INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::R => Some((
            tree_sitter_r::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_r::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Dart => Some((
            tree_sitter_dart::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_dart::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Scala => Some((
            tree_sitter_scala::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_scala::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Json => Some((
            tree_sitter_json::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JSON_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Toml => Some((
            tree_sitter_toml_ng::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Yaml => Some((
            tree_sitter_yaml::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(YAML_HIGHLIGHTS_QUERY, YAML_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::Sql => Some((
            tree_sitter_sequel::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_sequel::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Diff => Some((
            tree_sitter_diff::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_diff::HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::GitCommit => Some((
            tree_sitter_gitcommit::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(GITCOMMIT_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::TypeScript => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TreesitterQueryAsset::with_injections(
                TYPESCRIPT_HIGHLIGHTS_QUERY,
                TYPESCRIPT_INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Tsx => Some((
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TreesitterQueryAsset::with_injections(TSX_HIGHLIGHTS_QUERY, TSX_INJECTIONS_QUERY),
        )),
        DiffSyntaxLanguage::JavaScript => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(
                JAVASCRIPT_HIGHLIGHTS_QUERY,
                JAVASCRIPT_INJECTIONS_QUERY,
            ),
        )),
        DiffSyntaxLanguage::Jsdoc => Some((
            tree_sitter_jsdoc::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JSDOC_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Regex => Some((
            tree_sitter_regex::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(REGEX_HIGHLIGHTS_QUERY),
        )),
        DiffSyntaxLanguage::Bash => Some((
            tree_sitter_bash::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(BASH_HIGHLIGHTS_QUERY),
        )),
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
    let injection_combined_patterns = injection_query
        .as_ref()
        .map(|injection_query| {
            (0..injection_query.pattern_count())
                .map(|pattern_ix| {
                    injection_query
                        .property_settings(pattern_ix)
                        .iter()
                        .any(|setting| setting.key.as_ref() == "injection.combined")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let has_combined_injections = injection_combined_patterns.iter().any(|combined| *combined);
    TreesitterHighlightSpec {
        ts_language,
        query,
        capture_kinds,
        injection_query,
        injection_combined_patterns,
        has_combined_injections,
    }
}

macro_rules! highlight_spec_entry {
    ($language_variant:ident) => {{
        static SPEC: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
        Some(SPEC.get_or_init(|| init_highlight_spec(DiffSyntaxLanguage::$language_variant)))
    }};
}

/// Builds the highlight specs an injection query can reach, so the render path
/// does not have to.
///
/// `init_highlight_spec` compiles a grammar's queries, and for the big grammars
/// that is not cheap: measured cold on this machine, TypeScript is ~86ms, Tsx
/// ~80ms, Rust ~48ms, JavaScript ~30ms (Vue's own is ~3ms, Css ~0.5ms). None of
/// it is covered by `DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST` -- that
/// budget interrupts `parse`, not `Query::new` -- so whichever thread first
/// needs a spec pays the whole cost inline.
///
/// For a root document that lands during prepare, which is already off the UI
/// thread or already budgeted. Injected languages are the problem: their specs
/// are first touched by `ensure_injection_cached`, which runs while building
/// line tokens for lines that are about to be drawn. Vue makes that sharply
/// worse than other languages, because a single `.vue` file reaches three specs
/// -- Vue for the template, TypeScript for `<script lang="ts">` plus every
/// interpolation and directive, Css for `<style>` -- where a `.ts` file reaches
/// one. Scrolling from the template into the script block was an ~86ms stall.
///
/// Targets are read back off the compiled injection query rather than listed
/// here, so this keeps working when a query changes. It only sees
/// `#set! injection.language` targets, not the ones carried in an
/// `@injection.language` capture (Vue's `lang="scss"`), which is fine: the
/// captured ones resolve to Css, and Css is one of the cheap specs.
fn warm_reachable_highlight_specs(language: DiffSyntaxLanguage) {
    let Some(spec) = tree_sitter_highlight_spec(language) else {
        return;
    };
    let Some(injection_query) = spec.injection_query.as_ref() else {
        return;
    };
    for pattern_ix in 0..injection_query.pattern_count() {
        for setting in injection_query.property_settings(pattern_ix) {
            if setting.key.as_ref() != "injection.language" {
                continue;
            }
            let Some(target) = setting
                .value
                .as_deref()
                .and_then(injection_language_from_name)
            else {
                continue;
            };
            let _ = tree_sitter_highlight_spec(target);
        }
    }
}

fn highlight_spec_warmup_sender() -> Option<&'static std::sync::mpsc::Sender<DiffSyntaxLanguage>> {
    static SENDER: OnceLock<Option<std::sync::mpsc::Sender<DiffSyntaxLanguage>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<DiffSyntaxLanguage>();
            let builder = std::thread::Builder::new().name("gitcomet-syntax-warm".to_string());
            let _handle = builder
                .spawn(move || {
                    while let Ok(language) = rx.recv() {
                        warm_reachable_highlight_specs(language);
                    }
                })
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

/// Asks the warm-up thread to build the specs `language` can inject into.
///
/// Cheap and idempotent: at most one request per language per process. Racing
/// the render path is harmless -- `OnceLock::get_or_init` makes the loser wait
/// on the winner instead of duplicating the work -- so the worst case is the
/// status quo and the common case (a `.vue` opens showing its template, and the
/// script block is not drawn until the user scrolls) hides the cost entirely.
pub(super) fn request_highlight_spec_warmup(language: DiffSyntaxLanguage) {
    static REQUESTED: OnceLock<Mutex<FxHashSet<DiffSyntaxLanguage>>> = OnceLock::new();
    let requested = REQUESTED.get_or_init(|| Mutex::new(FxHashSet::default()));
    {
        let Ok(mut requested) = requested.lock() else {
            return;
        };
        if !requested.insert(language) {
            return;
        }
    }
    if let Some(sender) = highlight_spec_warmup_sender() {
        let _ = sender.send(language);
    }
}

pub(super) fn tree_sitter_highlight_spec(
    language: DiffSyntaxLanguage,
) -> Option<&'static TreesitterHighlightSpec> {
    match language {
        DiffSyntaxLanguage::Markdown => highlight_spec_entry!(Markdown),
        DiffSyntaxLanguage::MarkdownInline => highlight_spec_entry!(MarkdownInline),
        DiffSyntaxLanguage::Html => highlight_spec_entry!(Html),
        DiffSyntaxLanguage::Vue => highlight_spec_entry!(Vue),
        DiffSyntaxLanguage::Svelte => highlight_spec_entry!(Svelte),
        DiffSyntaxLanguage::Jinja => highlight_spec_entry!(Jinja),
        DiffSyntaxLanguage::JinjaText => highlight_spec_entry!(JinjaText),
        DiffSyntaxLanguage::Css => highlight_spec_entry!(Css),
        DiffSyntaxLanguage::Bicep => highlight_spec_entry!(Bicep),
        DiffSyntaxLanguage::Lua => highlight_spec_entry!(Lua),
        DiffSyntaxLanguage::Nix => highlight_spec_entry!(Nix),
        DiffSyntaxLanguage::Makefile => highlight_spec_entry!(Makefile),
        DiffSyntaxLanguage::Kotlin => highlight_spec_entry!(Kotlin),
        DiffSyntaxLanguage::Zig => highlight_spec_entry!(Zig),
        DiffSyntaxLanguage::Groovy => highlight_spec_entry!(Groovy),
        DiffSyntaxLanguage::Clojure => highlight_spec_entry!(Clojure),
        DiffSyntaxLanguage::Elixir => highlight_spec_entry!(Elixir),
        DiffSyntaxLanguage::Erlang => highlight_spec_entry!(Erlang),
        DiffSyntaxLanguage::Haskell => highlight_spec_entry!(Haskell),
        DiffSyntaxLanguage::Julia => highlight_spec_entry!(Julia),
        DiffSyntaxLanguage::OCaml => highlight_spec_entry!(OCaml),
        DiffSyntaxLanguage::OCamlInterface => highlight_spec_entry!(OCamlInterface),
        DiffSyntaxLanguage::Solidity => highlight_spec_entry!(Solidity),
        DiffSyntaxLanguage::Assembly => highlight_spec_entry!(Assembly),
        DiffSyntaxLanguage::Rust => highlight_spec_entry!(Rust),
        DiffSyntaxLanguage::Python => highlight_spec_entry!(Python),
        DiffSyntaxLanguage::Go => highlight_spec_entry!(Go),
        DiffSyntaxLanguage::GoMod => highlight_spec_entry!(GoMod),
        DiffSyntaxLanguage::GoWork => highlight_spec_entry!(GoWork),
        DiffSyntaxLanguage::C => highlight_spec_entry!(C),
        DiffSyntaxLanguage::Cpp => highlight_spec_entry!(Cpp),
        DiffSyntaxLanguage::ObjectiveC => highlight_spec_entry!(ObjectiveC),
        DiffSyntaxLanguage::CSharp => highlight_spec_entry!(CSharp),
        DiffSyntaxLanguage::FSharp => highlight_spec_entry!(FSharp),
        DiffSyntaxLanguage::Java => highlight_spec_entry!(Java),
        DiffSyntaxLanguage::Php => highlight_spec_entry!(Php),
        DiffSyntaxLanguage::Ruby => highlight_spec_entry!(Ruby),
        DiffSyntaxLanguage::PowerShell => highlight_spec_entry!(PowerShell),
        DiffSyntaxLanguage::Swift => highlight_spec_entry!(Swift),
        DiffSyntaxLanguage::R => highlight_spec_entry!(R),
        DiffSyntaxLanguage::Dart => highlight_spec_entry!(Dart),
        DiffSyntaxLanguage::Scala => highlight_spec_entry!(Scala),
        DiffSyntaxLanguage::Json => highlight_spec_entry!(Json),
        DiffSyntaxLanguage::Toml => highlight_spec_entry!(Toml),
        DiffSyntaxLanguage::Yaml => highlight_spec_entry!(Yaml),
        DiffSyntaxLanguage::Sql => highlight_spec_entry!(Sql),
        DiffSyntaxLanguage::Diff => highlight_spec_entry!(Diff),
        DiffSyntaxLanguage::GitCommit => highlight_spec_entry!(GitCommit),
        DiffSyntaxLanguage::TypeScript => highlight_spec_entry!(TypeScript),
        DiffSyntaxLanguage::Tsx => highlight_spec_entry!(Tsx),
        DiffSyntaxLanguage::JavaScript => highlight_spec_entry!(JavaScript),
        DiffSyntaxLanguage::Jsdoc => highlight_spec_entry!(Jsdoc),
        DiffSyntaxLanguage::Regex => highlight_spec_entry!(Regex),
        DiffSyntaxLanguage::Bash => highlight_spec_entry!(Bash),
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
        "string.regex" | "string.regexp" | "string.special.regex" => SyntaxTokenKind::StringRegex,
        "string.special" => SyntaxTokenKind::StringSpecial,
        "string" | "character" => SyntaxTokenKind::String,
        "diff.plus" => SyntaxTokenKind::DiffPlus,
        "diff.minus" => SyntaxTokenKind::DiffMinus,
        "diff.delta" => SyntaxTokenKind::DiffDelta,
        // Keywords
        "conditional" | "keyword.control" | "repeat" => SyntaxTokenKind::KeywordControl,
        "exception"
        | "keyword"
        | "keyword.declaration"
        | "keyword.import"
        | "include"
        | "storageclass" => SyntaxTokenKind::Keyword,
        "preproc" => SyntaxTokenKind::Preproc,
        // Numbers & booleans
        "float" | "number" | "number.float" => SyntaxTokenKind::Number,
        "boolean" => SyntaxTokenKind::Boolean,
        // Functions
        "function.method" => SyntaxTokenKind::FunctionMethod,
        "function.special" | "function.special.definition" => SyntaxTokenKind::FunctionSpecial,
        "constructor" => SyntaxTokenKind::Constructor,
        "function" | "function.definition" | "method" => SyntaxTokenKind::Function,
        // Types
        "module.builtin" | "type.builtin" => SyntaxTokenKind::TypeBuiltin,
        "concept" | "type.interface" => SyntaxTokenKind::TypeInterface,
        "module" | "namespace" => SyntaxTokenKind::Namespace,
        "array" | "selector" | "type" | "type.class" => SyntaxTokenKind::Type,
        // Variables - general `@variable` renders as plain text (no color) to avoid
        // "everything is highlighted" noise. Sub-captures get distinct treatment.
        "parameter" | "variable.parameter" => SyntaxTokenKind::VariableParameter,
        "variable.builtin" => SyntaxTokenKind::VariableBuiltin,
        "variable.special" => SyntaxTokenKind::VariableSpecial,
        "variable.member" | "variable.other.member" => SyntaxTokenKind::Property,
        "variable" => SyntaxTokenKind::Variable,
        // Properties
        "field" | "property" | "property.definition" => SyntaxTokenKind::Property,
        // Tags (HTML/JSX)
        "tag" | "tag.doctype" => SyntaxTokenKind::Tag,
        // Attributes
        "attribute" | "attribute.jsx" => SyntaxTokenKind::Attribute,
        // Constants
        "constant.builtin" => SyntaxTokenKind::ConstantBuiltin,
        "constant" => SyntaxTokenKind::Constant,
        // Operators
        "operator" => SyntaxTokenKind::Operator,
        // Punctuation
        "punctuation.bracket" => SyntaxTokenKind::PunctuationBracket,
        "delimiter" | "punctuation.delimiter" => SyntaxTokenKind::PunctuationDelimiter,
        "punctuation.special" => SyntaxTokenKind::PunctuationSpecial,
        "punctuation.list_marker" | "punctuation.list_marker.markup" => {
            SyntaxTokenKind::PunctuationListMarker
        }
        "punctuation" => SyntaxTokenKind::Punctuation,
        // Lifetime (Rust)
        "lifetime" => SyntaxTokenKind::Lifetime,
        // Labels (goto, DTD notation names)
        "label" => SyntaxTokenKind::Label,
        // Markup (XML text content, CDATA, URIs)
        "link_uri.markup" | "markup.link" | "text.uri" => SyntaxTokenKind::MarkupLink,
        "markup.raw" | "text.literal" | "text.literal.markup" => SyntaxTokenKind::TextLiteral,
        "markup.heading" | "text.title" | "title.markup" => SyntaxTokenKind::MarkupHeading,
        "emphasis.markup"
        | "emphasis.strong.markup"
        | "link_text.markup"
        | "markup"
        | "strikethrough.markup" => SyntaxTokenKind::Variable,
        // Skip `@none`, `@embedded`, most `@text.*`, and other non-semantic captures.
        _ => return None,
    })
}
