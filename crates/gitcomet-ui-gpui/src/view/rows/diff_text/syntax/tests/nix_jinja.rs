use super::*;

// ---- Nix ------------------------------------------------------------------

const NIX_FIXTURE: &[&str] = &[
    /*  0 */ "# Build a demo package.",
    /*  1 */ "{ pkgs, lib ? pkgs.lib, ... }:",
    /*  2 */ "let",
    /*  3 */ "  inherit (pkgs) stdenv;",
    /*  4 */ "  version = \"1.0\";",
    /*  5 */ "  readme = builtins.readFile ./README.md;",
    /*  6 */ "in",
    /*  7 */ "stdenv.mkDerivation rec {",
    /*  8 */ "  pname = \"demo\";",
    /*  9 */ "  meta.description = \"demo v${version}\";",
    /* 10 */ "  buildPhase = ''",
    /* 11 */ "    export OUT=$out",
    /* 12 */ "    if [ -d bin ]; then",
    /* 13 */ "      cp -r bin \"$out/bin\"",
    /* 14 */ "    fi",
    /* 15 */ "  '';",
    /* 16 */ "}",
];

#[test]
fn nix_extension_is_supported() {
    for path in ["flake.nix", "pkgs/demo/default.nix", "nix/modules/web.nix"] {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(DiffSyntaxLanguage::Nix),
            "{path} should resolve to the Nix grammar"
        );
    }
    assert_eq!(
        diff_syntax_language_for_code_fence_info("nix"),
        Some(DiffSyntaxLanguage::Nix),
    );
    // `flake.lock` is JSON and must keep resolving that way.
    assert_eq!(
        diff_syntax_language_for_path("flake.lock"),
        Some(DiffSyntaxLanguage::Json),
    );
}

#[test]
fn prepared_nix_document_highlights_core_syntax() {
    let doc = prepare_nix_document(NIX_FIXTURE);

    let comment = token_kinds_for_line_fragment(doc, 0, NIX_FIXTURE[0], "demo package");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`# …` is a Nix line comment: {comment:?}"
    );

    for (line_ix, keyword) in [(2usize, "let"), (3, "inherit"), (6, "in"), (7, "rec")] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, NIX_FIXTURE[line_ix], keyword);
        assert!(
            kinds.contains(&SyntaxTokenKind::Keyword),
            "`{keyword}` should be a keyword: {kinds:?}"
        );
    }

    let formal = token_kinds_for_line_fragment(doc, 1, NIX_FIXTURE[1], "pkgs");
    assert!(
        formal.contains(&SyntaxTokenKind::VariableParameter),
        "`pkgs` is a formal in the function's argument set: {formal:?}"
    );

    let attr = token_kinds_for_line_fragment(doc, 8, NIX_FIXTURE[8], "pname");
    assert!(
        attr.contains(&SyntaxTokenKind::Property),
        "a binding attrpath should read as a property: {attr:?}"
    );

    let string = token_kinds_for_line_fragment(doc, 8, NIX_FIXTURE[8], "\"demo\"");
    assert!(
        string.contains(&SyntaxTokenKind::String),
        "`\"demo\"` should be a string: {string:?}"
    );

    let path = token_kinds_for_line_fragment(doc, 5, NIX_FIXTURE[5], "./README.md");
    assert!(
        path.contains(&SyntaxTokenKind::StringSpecial),
        "a bare Nix path is `@string.special.path`: {path:?}"
    );

    let interpolation = token_kinds_for_line_fragment(doc, 9, NIX_FIXTURE[9], "${");
    assert!(
        interpolation.contains(&SyntaxTokenKind::PunctuationSpecial),
        "`${{` opens an interpolation: {interpolation:?}"
    );
}

/// The one real guard on the reordering in nix_highlights.scm.
///
/// Two patterns capturing the *same* node tie on start byte, so the tiebreak is
/// pattern index — the later rule in the file wins. Upstream ends with a blanket
/// `(identifier) @variable`, which ported verbatim buries every specific
/// identifier rule. Confirmed to fail against upstream's ordering: `builtins`
/// comes back as `[Variable]`. If this fails after a re-sync, the query was not
/// re-sorted.
#[test]
fn nix_specific_captures_survive_the_generic_identifier_rule() {
    let doc = prepare_nix_document(NIX_FIXTURE);

    let builtins = token_kinds_for_line_fragment(doc, 5, NIX_FIXTURE[5], "builtins");
    assert!(
        builtins.contains(&SyntaxTokenKind::VariableBuiltin)
            && !builtins.contains(&SyntaxTokenKind::Variable),
        "`builtins` must keep its builtin colour rather than falling back to the \
             blanket `(identifier) @variable` rule: {builtins:?}"
    );

    let applied = token_kinds_for_line_fragment(doc, 7, NIX_FIXTURE[7], "mkDerivation");
    assert!(
        applied.contains(&SyntaxTokenKind::Function)
            && !applied.contains(&SyntaxTokenKind::Variable),
        "an identifier in function-application position must read as a function: \
             {applied:?}"
    );

    let inherited = token_kinds_for_line_fragment(doc, 3, NIX_FIXTURE[3], "stdenv");
    assert!(
        inherited.contains(&SyntaxTokenKind::Property),
        "`inherit (pkgs) stdenv` names a property: {inherited:?}"
    );
}

/// An escape inside a string keeps its own colour.
///
/// Not an ordering guard, despite appearances: `normalize_non_overlapping_tokens`
/// hands each slice to the last *containing* capture in emission order, and the
/// cursor emits by node start byte, so a nested `(escape_sequence)` always beats
/// the `(string_expression)` around it whichever order their rules appear in.
/// Verified — this passes against upstream's ordering too. It is here to pin the
/// behaviour, not the query layout.
#[test]
fn nix_escape_sequences_outrank_the_string_rule() {
    let lines = ["{ s = \"a\\nb\"; }"];
    let doc = prepare_nix_document(&lines);
    let escape = token_kinds_for_line_fragment(doc, 0, lines[0], "\\n");
    assert!(
        escape.contains(&SyntaxTokenKind::StringEscape),
        "`\\n` inside a string must outrank the enclosing `@string` capture: {escape:?}"
    );
}

/// The interior of `"demo v${version}"` is Nix code, not string text.
///
/// Like the escape test above, this holds by node position rather than by rule
/// order — the interpolated expression starts after the string does, so it wins
/// its own bytes regardless.
#[test]
fn nix_interpolation_interior_is_not_flat_string() {
    let doc = prepare_nix_document(NIX_FIXTURE);
    let inner = token_kinds_for_line_fragment(doc, 9, NIX_FIXTURE[9], "version");
    assert!(
        !inner.is_empty() && !inner.contains(&SyntaxTokenKind::String),
        "the expression inside `${{…}}` should be highlighted as code, not as part \
             of the surrounding string: {inner:?}"
    );
}

/// `buildPhase = '' … ''` is shell script, and the combined Bash injection is
/// what makes it read as one. Only the injected layer has a concept of `if`.
#[test]
fn nix_build_phase_is_highlighted_as_bash() {
    let doc = prepare_nix_document(NIX_FIXTURE);

    let conditional = token_kinds_for_line_fragment(doc, 12, NIX_FIXTURE[12], "if");
    assert!(
        conditional.contains(&SyntaxTokenKind::KeywordControl)
            || conditional.contains(&SyntaxTokenKind::Keyword),
        "`if` inside buildPhase should come from the injected Bash layer, not read \
             as string text: {conditional:?}"
    );

    // And the injection stays inside the indented string: the Nix binding on
    // the line above is still Nix.
    let binding = token_kinds_for_line_fragment(doc, 10, NIX_FIXTURE[10], "buildPhase");
    assert!(
        binding.contains(&SyntaxTokenKind::Property),
        "`buildPhase` is a Nix attrpath, not part of the shell script: {binding:?}"
    );
}

#[test]
fn nix_injection_targets_resolve_to_working_grammars() {
    let lang: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, NIX_INJECTIONS_QUERY)
        .expect("nix_injections.scm should compile");
    let mut checked = 0usize;
    for pattern_ix in 0..query.pattern_count() {
        for setting in query.property_settings(pattern_ix) {
            if setting.key.as_ref() != "injection.language" {
                continue;
            }
            let Some(value) = setting.value.as_deref() else {
                continue;
            };
            let target = injection_language_from_name(value).unwrap_or_else(|| {
                panic!("nix_injections.scm names an unknown injection language {value:?}")
            });
            assert!(
                tree_sitter_highlight_spec(target).is_some(),
                "nix_injections.scm injects {value:?} but no grammar is wired for \
                     {target:?}, so the injection would silently no-op"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 4,
        "expected the four curated bash rules; upstream's comment-marked \
             arbitrary-language rule is deliberately not ported"
    );
}

#[test]
fn nix_spec_warmup_reaches_bash_through_a_set_directive() {
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Nix).expect("nix spec");
    let injection_query = spec.injection_query.as_ref().expect("nix injection query");
    let reaches_bash = (0..injection_query.pattern_count()).any(|pattern_ix| {
        injection_query
            .property_settings(pattern_ix)
            .iter()
            .filter(|setting| setting.key.as_ref() == "injection.language")
            .any(|setting| {
                setting
                    .value
                    .as_deref()
                    .and_then(injection_language_from_name)
                    == Some(DiffSyntaxLanguage::Bash)
            })
    });
    assert!(
        reaches_bash,
        "warm_reachable_highlight_specs must be able to see the bash target, or the \
             Bash query compile lands on the draw path"
    );
}

#[test]
fn nix_grammar_is_abi_compatible_with_workspace_tree_sitter() {
    let nix: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
    let abi = nix.abi_version();
    assert!(
        (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&abi),
        "tree-sitter-nix ABI {abi} is outside the range this tree-sitter supports \
             ({}..={})",
        tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
        tree_sitter::LANGUAGE_VERSION,
    );
}

#[test]
fn nix_grammar_parses_a_flake() {
    let source = concat!(
        "{\n",
        "  description = \"demo\";\n",
        "  inputs.nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";\n",
        "  outputs = { self, nixpkgs }: {\n",
        "    packages.x86_64-linux.default =\n",
        "      nixpkgs.legacyPackages.x86_64-linux.hello;\n",
        "  };\n",
        "}\n",
    );
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_nix::LANGUAGE.into())
        .expect("nix grammar should load into the workspace tree-sitter");
    let tree = parser.parse(source, None).expect("flake.nix should parse");
    assert!(
        !tree.root_node().has_error(),
        "the nix grammar produced an ERROR node for a well-formed flake: {}",
        tree.root_node().to_sexp(),
    );
}

// ---- Nunjucks / Jinja2 ----------------------------------------------------

/// The `.njk` / `.j2` / `.jinja` fixture, shared by the tests below.
const JINJA_TEMPLATE_FIXTURE: &[&str] = &[
    /* 0 */ "{# page heading #}",
    /* 1 */ "<ul class=\"list\">",
    /* 2 */ "  {% for item in items %}",
    /* 3 */ "    <li>{{ item.name | upper }}</li>",
    /* 4 */ "  {% endfor %}",
    /* 5 */ "</ul>",
];

#[test]
fn jinja_extension_is_supported() {
    for path in [
        "templates/index.njk",
        "templates/base.html.j2",
        "templates/macros.jinja",
        "templates/macros.jinja2",
        "templates/page.twig",
        "templates/page.html.dj",
    ] {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(DiffSyntaxLanguage::Jinja),
            "{path} should resolve to the Jinja grammar"
        );
    }
    // The same table backs markdown fence info.
    for fence in ["njk", "jinja", "jinja2", "twig", "nunjucks"] {
        assert_eq!(
            diff_syntax_language_for_code_fence_info(fence),
            Some(DiffSyntaxLanguage::Jinja),
            "```{fence} should resolve to the Jinja grammar"
        );
    }
}

/// A `.j2` says the file is templated, not that it is markup. Resolving a shell
/// or config template to the HTML-injecting reading hands the HTML grammar
/// `cat <<EOF` and `2>&1`, which open bogus elements.
#[test]
fn text_bodied_jinja_templates_do_not_get_html_injected() {
    for path in [
        "roles/web/templates/nginx.conf.j2",
        "charts/app/values.yaml.j2",
        "deploy/deploy.sh.j2",
        "docker-compose.yml.j2",
        "config/settings.ini.jinja",
        "db/schema.sql.j2",
    ] {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(DiffSyntaxLanguage::JinjaText),
            "{path} has a non-markup body, so it must not inject HTML"
        );
    }

    let markup = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
    let text = tree_sitter_highlight_spec(DiffSyntaxLanguage::JinjaText).expect("text spec");
    assert!(
        markup.injection_query.is_some(),
        "the markup reading is the one that injects HTML"
    );
    assert!(
        text.injection_query.is_none(),
        "the text reading must have no injection query at all"
    );
    assert!(
        !text.has_combined_injections,
        "with no injection query there is no combined group to build"
    );
}

/// The shell-template shape that motivated the split, end to end.
#[test]
fn shell_bodied_jinja_template_does_not_colour_redirects_as_tags() {
    let lines = [
        /* 0 */ "#!/bin/sh",
        /* 1 */ "{% if debug %}",
        /* 2 */ "cat <<EOF > {{ target }}",
        /* 3 */ "  value=1",
        /* 4 */ "EOF",
        /* 5 */ "{% endif %}",
        /* 6 */ "run --flag 2>&1 < input",
    ];
    let doc = prepare_test_document(DiffSyntaxLanguage::JinjaText, &lines.join("\n"));

    for (line_ix, fragment) in [(2usize, "EOF"), (6, "input")] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, lines[line_ix], fragment);
        assert!(
            !kinds.contains(&SyntaxTokenKind::Tag),
            "`{fragment}` on line {line_ix} was coloured as an HTML tag: {kinds:?}"
        );
    }

    // The template tags themselves still highlight -- only the injection is gone.
    let endif = token_kinds_for_line_fragment(doc, 5, lines[5], "endif");
    assert!(
        endif.contains(&SyntaxTokenKind::KeywordControl),
        "template keywords must survive the split: {endif:?}"
    );
}

#[test]
fn prepared_jinja_document_highlights_template_tags() {
    let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);

    let comment = token_kinds_for_line_fragment(doc, 0, JINJA_TEMPLATE_FIXTURE[0], "heading");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`{{# … #}}` is a Jinja comment: {comment:?}"
    );

    let open = token_kinds_for_line_fragment(doc, 2, JINJA_TEMPLATE_FIXTURE[2], "{%");
    assert!(
        open.contains(&SyntaxTokenKind::PunctuationSpecial),
        "the `{{%` delimiter should be punctuation, not plain text: {open:?}"
    );

    for (line_ix, keyword) in [(2usize, "for"), (4, "endfor")] {
        let kinds =
            token_kinds_for_line_fragment(doc, line_ix, JINJA_TEMPLATE_FIXTURE[line_ix], keyword);
        assert!(
            kinds.contains(&SyntaxTokenKind::KeywordControl),
            "`{keyword}` is control flow and should render semibold: {kinds:?}"
        );
    }

    let filter = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "upper");
    assert!(
        filter.contains(&SyntaxTokenKind::Function),
        "a filter name after `|` should read as a function: {filter:?}"
    );

    let property = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "name");
    assert!(
        property.contains(&SyntaxTokenKind::Property),
        "`item.name` should colour `name` as a property: {property:?}"
    );
}

/// The HTML half of a template comes from the combined injection, not the
/// Jinja grammar -- which sees only opaque `text` nodes.
#[test]
fn prepared_jinja_document_highlights_html_via_the_combined_injection() {
    let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);

    let tag = token_kinds_for_line_fragment(doc, 1, JINJA_TEMPLATE_FIXTURE[1], "ul");
    assert!(
        tag.contains(&SyntaxTokenKind::Tag),
        "`<ul>` should be tagged by the injected HTML layer: {tag:?}"
    );
    let attribute = token_kinds_for_line_fragment(doc, 1, JINJA_TEMPLATE_FIXTURE[1], "class");
    assert!(
        attribute.contains(&SyntaxTokenKind::Attribute),
        "`class=` should be an HTML attribute: {attribute:?}"
    );

    // The whole point of the combined injection: `<li>` sits inside the loop
    // body, in a different `text` node from `<ul>`, and still highlights.
    let inner = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "li");
    assert!(
        inner.contains(&SyntaxTokenKind::Tag),
        "`<li>` is in a separate text run from `<ul>`; only a combined layer \
             sees them as one document: {inner:?}"
    );
}

/// Patch rows take the single-line path while file-content rows project from a
/// prepared document. They must run the same injection query: Jinja itself sees
/// this line as opaque text, so only the injected HTML grammar can colour it.
#[test]
fn single_line_jinja_highlighting_matches_the_prepared_document() {
    let line = "<nav class=\"menu\"><span>Home</span></nav>";
    let single_line = syntax_tokens_for_line(line, DiffSyntaxLanguage::Jinja, DiffSyntaxMode::Auto);
    let prepared = prepare_test_document(DiffSyntaxLanguage::Jinja, line);
    let prepared_line = syntax_tokens_for_prepared_document_line(prepared, 0)
        .expect("prepared line tokens should be available");

    assert_eq!(
        single_line, prepared_line,
        "patch and file-content highlighting must use the same host and injected grammars"
    );
    assert!(
        has_token_kind_and_text(line, &single_line, SyntaxTokenKind::Tag, "nav"),
        "the equality must cover real injected HTML tokens, not two empty results: {single_line:?}"
    );
    assert!(
        has_token_kind_and_text(line, &single_line, SyntaxTokenKind::Attribute, "class"),
        "the injected HTML attribute should be highlighted: {single_line:?}"
    );
}

/// The injected HTML must stay off the template tags, which the Jinja
/// grammar owns. See `combined_injection_gaps`.
#[test]
fn jinja_html_injection_does_not_bleed_onto_template_tags() {
    let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);
    let kinds = token_kinds_for_line_fragment(doc, 4, JINJA_TEMPLATE_FIXTURE[4], "endfor");
    assert!(
        !kinds.contains(&SyntaxTokenKind::Tag),
        "`{{% endfor %}}` sits in a gap between two HTML ranges; an HTML element \
             node spanning it must not colour it as a tag: {kinds:?}"
    );
}

#[test]
fn jinja_injection_targets_resolve_to_working_grammars() {
    let lang: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, JINJA_INJECTIONS_QUERY)
        .expect("jinja_injections.scm should compile");
    let mut checked = 0usize;
    for pattern_ix in 0..query.pattern_count() {
        for setting in query.property_settings(pattern_ix) {
            if setting.key.as_ref() != "injection.language" {
                continue;
            }
            let Some(value) = setting.value.as_deref() else {
                continue;
            };
            let target = injection_language_from_name(value).unwrap_or_else(|| {
                panic!("jinja_injections.scm names an unknown injection language {value:?}")
            });
            assert!(
                tree_sitter_highlight_spec(target).is_some(),
                "jinja_injections.scm injects {value:?} but no grammar is wired for \
                     {target:?}, so the injection would silently no-op"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "expected at least one `#set! injection.language`"
    );
}

/// Warm-up reads targets off the compiled query, and only sees `#set!`
/// literals. If the HTML target ever moved into an `@injection.language`
/// capture, the ~0.5ms HTML spec compile would move back onto the draw path.
#[test]
fn jinja_spec_warmup_reaches_html_through_a_set_directive() {
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
    let injection_query = spec
        .injection_query
        .as_ref()
        .expect("jinja injection query");
    let reaches_html = (0..injection_query.pattern_count()).any(|pattern_ix| {
        injection_query
            .property_settings(pattern_ix)
            .iter()
            .filter(|setting| setting.key.as_ref() == "injection.language")
            .any(|setting| {
                setting
                    .value
                    .as_deref()
                    .and_then(injection_language_from_name)
                    == Some(DiffSyntaxLanguage::Html)
            })
    });
    assert!(
        reaches_html,
        "warm_reachable_highlight_specs must be able to see the html target"
    );
}

#[test]
fn jinja_injection_query_stays_under_the_match_limit_on_a_dense_template() {
    let mut lines = vec!["<ul>".to_string()];
    for ix in 0..120 {
        lines.push(format!(
                "  {{% if show{ix} %}}<li class=\"r{ix}\">{{{{ row{ix}.label | title }}}}</li>{{% endif %}}"
            ));
    }
    lines.push("</ul>".to_string());
    let text = lines.join("\n");

    let lang: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, JINJA_INJECTIONS_QUERY)
        .expect("jinja_injections.scm should compile");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang)
        .expect("jinja grammar should load");
    let tree = parser
        .parse(&text, None)
        .expect("dense template should parse");

    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    let mut matched = 0usize;
    {
        let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
        tree_sitter::StreamingIterator::advance(&mut matches);
        while matches.get().is_some() {
            matched += 1;
            tree_sitter::StreamingIterator::advance(&mut matches);
        }
    }

    assert!(
        !cursor.did_exceed_match_limit(),
        "the Jinja injection query overflowed the {TS_QUERY_MATCH_LIMIT}-match \
             in-progress pool on a {}-line template. A combined group that loses ranges \
             assembles a different HTML document, so the engine drops the whole group and \
             the template renders with no HTML highlighting at all",
        lines.len(),
    );
    assert!(matched > 0, "the dense template should produce matches");
}

/// The grammar is a young crates.io release binding through
/// `tree-sitter-language`, so a tree-sitter bump could outrun it.
#[test]
fn jinja_grammar_is_abi_compatible_with_workspace_tree_sitter() {
    let jinja: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
    let abi = jinja.abi_version();
    assert!(
        (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&abi),
        "tree-sitter-jinja-dialects ABI {abi} is outside the range this tree-sitter \
             supports ({}..={})",
        tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
        tree_sitter::LANGUAGE_VERSION,
    );
}

#[test]
fn jinja_grammar_parses_every_dialect_it_claims() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_jinja_dialects::LANGUAGE.into())
        .expect("jinja grammar should load into the workspace tree-sitter");
    // One sample per dialect the crate advertises, since a single grammar
    // serving all of njk/j2/twig/dj is the reason it was chosen.
    for (dialect, source) in [
        ("jinja2", "{% for x in xs %}{{ x|e }}{% endfor %}\n"),
        ("nunjucks", "{% set n = 1 %}{{ n + 1 }}\n"),
        ("twig", "{% if a is not empty %}{{ a.b }}{% endif %}\n"),
        (
            "django",
            "{% extends \"base.html\" %}{% block body %}{% endblock %}\n",
        ),
    ] {
        let tree = parser
            .parse(source, None)
            .unwrap_or_else(|| panic!("{dialect} sample should parse"));
        assert!(
            !tree.root_node().has_error(),
            "{dialect} sample produced an ERROR node: {}",
            tree.root_node().to_sexp(),
        );
    }
}
