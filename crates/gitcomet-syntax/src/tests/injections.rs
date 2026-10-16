use super::*;

#[test]
fn rust_treesitter_captures_variable_parameter() {
    let text = "fn foo(bar: u32) {}";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::VariableParameter),
        "Rust function parameter should produce VariableParameter token, got: {tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_self_as_variable_special() {
    let text = "impl Widget { fn render(&self, item: Item) { self.draw(item); } }";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::VariableSpecial, "self"),
        "Rust `self` should produce VariableSpecial token, got: {tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_type_builtin() {
    let text = "let x: u32 = 0;";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::TypeBuiltin),
        "Rust primitive type should produce TypeBuiltin token, got: {tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_macro_as_function_special() {
    let text = "println!(\"hello\");";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::FunctionSpecial),
        "Rust macro invocation should produce FunctionSpecial token, got: {tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_keyword_function_type_and_string_families() {
    let text = r#"fn foo(bar: u32) { let x = "hi"; }"#;
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "fn"),
        "Rust should highlight `fn` as a keyword, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "foo"),
        "Rust function declarations should capture the function name, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "let"),
        "Rust should highlight `let` as a keyword, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::TypeBuiltin, "u32"),
        "Rust primitive types should keep their dedicated type token, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::String, "\"hi\""),
        "Rust string literals should produce String tokens, got: {tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_impl_family_as_preproc() {
    let impl_text = "impl Widget where T: Trait {}";
    let impl_tokens =
        syntax_tokens_for_line(impl_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Preproc, "impl"),
        "Rust `impl` should route through Preproc for the violet family, got: {impl_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Preproc, "where"),
        "Rust `where` should route through Preproc, got: {impl_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Type, "Widget"),
        "Rust impl targets should keep their type token, got: {impl_tokens:?}"
    );

    let trait_text = "trait Painter where Self: Sized {}";
    let trait_tokens =
        syntax_tokens_for_line(trait_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(trait_text, &trait_tokens, SyntaxTokenKind::Preproc, "trait"),
        "Rust `trait` should route through Preproc, got: {trait_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(trait_text, &trait_tokens, SyntaxTokenKind::Preproc, "where"),
        "Rust `where` should stay violet in trait declarations, got: {trait_tokens:?}"
    );

    let dyn_text = "let painter: dyn Painter = todo!();";
    let dyn_tokens =
        syntax_tokens_for_line(dyn_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(dyn_text, &dyn_tokens, SyntaxTokenKind::Preproc, "dyn"),
        "Rust `dyn` should route through Preproc, got: {dyn_tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_use_roots_and_tails() {
    let type_text = "use foo::Bar;";
    let type_tokens =
        syntax_tokens_for_line(type_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "foo"),
        "Non-`crate` import roots should route through Preproc, got: {type_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Type, "Bar"),
        "Imported uppercase tails should keep their type token, got: {type_tokens:?}"
    );

    let function_text = "use foo::bar;";
    let function_tokens = syntax_tokens_for_line(
        function_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            function_text,
            &function_tokens,
            SyntaxTokenKind::Preproc,
            "foo",
        ),
        "Non-`crate` import roots should stay violet, got: {function_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            function_text,
            &function_tokens,
            SyntaxTokenKind::Function,
            "bar",
        ),
        "Imported lowercase tails should route through Function, got: {function_tokens:?}"
    );
}

#[test]
fn rust_treesitter_keeps_use_middle_modules_neutral() {
    let type_text = "use foo::bar::Baz;";
    let type_tokens =
        syntax_tokens_for_line(type_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "foo"),
        "The top import root should stay violet, got: {type_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Type, "Baz"),
        "The imported type should stay green, got: {type_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "bar"),
        "Middle modules should not inherit the root violet accent, got: {type_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Function, "bar"),
        "Middle modules should not be recolored as imported tails, got: {type_tokens:?}"
    );

    let crate_type_text = "use crate::foo::Bar;";
    let crate_type_tokens = syntax_tokens_for_line(
        crate_type_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            crate_type_text,
            &crate_type_tokens,
            SyntaxTokenKind::Keyword,
            "crate",
        ),
        "Rust should keep `crate` on the keyword/orange family, got: {crate_type_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            crate_type_text,
            &crate_type_tokens,
            SyntaxTokenKind::Type,
            "Bar",
        ),
        "Imported types under `crate` should stay green, got: {crate_type_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(
            crate_type_text,
            &crate_type_tokens,
            SyntaxTokenKind::Preproc,
            "foo",
        ),
        "The segment after `crate::` should stay neutral, got: {crate_type_tokens:?}"
    );

    let crate_function_text = "use crate::foo::bar;";
    let crate_function_tokens = syntax_tokens_for_line(
        crate_function_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            crate_function_text,
            &crate_function_tokens,
            SyntaxTokenKind::Function,
            "bar",
        ),
        "The final lowercase import tail should stay blue under `crate`, got: {crate_function_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(
            crate_function_text,
            &crate_function_tokens,
            SyntaxTokenKind::Preproc,
            "foo",
        ),
        "The segment after `crate::` should remain neutral, got: {crate_function_tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_root_modules_before_functions_and_types() {
    let call_text = "let handler = foo::bar::baz();";
    let call_tokens =
        syntax_tokens_for_line(call_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Preproc, "foo"),
        "Rust code paths should color the bare root module as Preproc, got: {call_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Preproc, "bar"),
        "Inner code-path modules should stay neutral instead of inheriting the root violet, got: {call_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Function, "bar"),
        "Inner code-path modules should not be recolored as callable tails, got: {call_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Function, "baz"),
        "Rust function paths should keep the callable name as Function, got: {call_tokens:?}"
    );

    let associated_text = "let factory = foo::bar::Baz::new();";
    let associated_tokens = syntax_tokens_for_line(
        associated_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            associated_text,
            &associated_tokens,
            SyntaxTokenKind::Preproc,
            "foo",
        ),
        "Associated paths should keep the bare root module violet, got: {associated_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(
            associated_text,
            &associated_tokens,
            SyntaxTokenKind::Preproc,
            "bar",
        ),
        "Inner modules before associated functions should stay neutral, got: {associated_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            associated_text,
            &associated_tokens,
            SyntaxTokenKind::Type,
            "Baz",
        ),
        "Associated function paths should keep the type token, got: {associated_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            associated_text,
            &associated_tokens,
            SyntaxTokenKind::Function,
            "new",
        ),
        "Associated function paths should keep the callable name as Function, got: {associated_tokens:?}"
    );

    let crate_text = "let value: crate::foo::Bar = todo!();";
    let crate_tokens =
        syntax_tokens_for_line(crate_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Keyword, "crate"),
        "Rust should keep `crate` on the keyword/orange family, got: {crate_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Preproc, "foo"),
        "The first named segment after `crate::` should stay neutral in code paths, got: {crate_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Type, "Bar"),
        "Rust type tails under `crate` should stay green, got: {crate_tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_constants_in_scoped_paths() {
    let constant_text = "let mode = NotForContentType::SSE;";
    let constant_tokens = syntax_tokens_for_line(
        constant_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            constant_text,
            &constant_tokens,
            SyntaxTokenKind::Type,
            "NotForContentType",
        ),
        "Rust should keep the type side of associated constants green, got: {constant_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            constant_text,
            &constant_tokens,
            SyntaxTokenKind::Constant,
            "SSE",
        ),
        "Rust ALL_CAPS associated constants should route through Constant, got: {constant_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(
            constant_text,
            &constant_tokens,
            SyntaxTokenKind::Type,
            "SSE",
        ),
        "Rust ALL_CAPS associated constants should no longer be typed green, got: {constant_tokens:?}"
    );

    let scoped_text = "let root = foo::BAR;";
    let scoped_tokens =
        syntax_tokens_for_line(scoped_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(scoped_text, &scoped_tokens, SyntaxTokenKind::Preproc, "foo"),
        "Bare module roots should stay violet before constant tails, got: {scoped_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            scoped_text,
            &scoped_tokens,
            SyntaxTokenKind::Constant,
            "BAR",
        ),
        "Scoped ALL_CAPS references should route through Constant, got: {scoped_tokens:?}"
    );

    let crate_scoped_text = "let root = crate::foo::BAR;";
    let crate_scoped_tokens = syntax_tokens_for_line(
        crate_scoped_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            crate_scoped_text,
            &crate_scoped_tokens,
            SyntaxTokenKind::Keyword,
            "crate",
        ),
        "Rust should keep `crate` orange before constant tails, got: {crate_scoped_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(
            crate_scoped_text,
            &crate_scoped_tokens,
            SyntaxTokenKind::Preproc,
            "foo",
        ),
        "The first named segment after `crate::` should stay neutral before constants, got: {crate_scoped_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            crate_scoped_text,
            &crate_scoped_tokens,
            SyntaxTokenKind::Constant,
            "BAR",
        ),
        "ALL_CAPS constant tails under `crate` should stay pink/Constant, got: {crate_scoped_tokens:?}"
    );

    let standalone_text = "let standalone = SSE;";
    let standalone_tokens = syntax_tokens_for_line(
        standalone_text,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            standalone_text,
            &standalone_tokens,
            SyntaxTokenKind::Constant,
            "SSE",
        ),
        "Standalone ALL_CAPS Rust names should route through Constant, got: {standalone_tokens:?}"
    );
}

#[test]
fn rust_treesitter_captures_grouped_use_import_semantics() {
    let text = "use foo::{bar, baz::Qux};";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "foo"),
        "Grouped imports should accent the non-`crate` root, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "bar"),
        "Grouped imports should keep lowercase imported tails blue, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Type, "Qux"),
        "Grouped imports should keep uppercase imported tails green, got: {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "baz"),
        "Grouped middle modules should not inherit the root violet accent, got: {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "baz"),
        "Grouped middle modules should stay neutral when importing a type, got: {tokens:?}"
    );

    let crate_text = "use crate::{foo::bar, baz::Qux};";
    let crate_tokens =
        syntax_tokens_for_line(crate_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Keyword, "crate",),
        "Grouped imports should keep `crate` on the keyword/orange family, got: {crate_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Function, "bar",),
        "Grouped imports under `crate` should keep lowercase tails blue, got: {crate_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Type, "Qux"),
        "Grouped imports under `crate` should keep uppercase tails green, got: {crate_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Preproc, "foo",),
        "Paths under `crate::{{...}}` should not add a violet root accent, got: {crate_tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Function, "baz",),
        "Middle grouped modules should stay neutral before imported types, got: {crate_tokens:?}"
    );
}

#[test]
fn rust_treesitter_keeps_use_aliases_neutral() {
    let text = "use foo::bar as baz;";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "foo"),
        "Aliased imports should keep the non-`crate` root violet, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "bar"),
        "Aliased imports should keep the source tail blue, got: {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "baz"),
        "Import aliases should stay neutral instead of inheriting the root accent, got: {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "baz"),
        "Import aliases should stay neutral instead of inheriting the source tail color, got: {tokens:?}"
    );
}

#[test]
fn tsx_treesitter_highlights_jsx_tag_and_attribute() {
    let text = "const node = <button disabled />;";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Tsx, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "TSX should highlight JSX tags, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
        "TSX should highlight JSX attributes, got: {tokens:?}"
    );
}

#[test]
fn css_treesitter_captures_property_and_keyword() {
    let text = "@media screen { .foo { color: red; } }";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Css, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "CSS should highlight @media as keyword: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "CSS should highlight 'color' as property: {tokens:?}"
    );
}

#[test]
fn javascript_tagged_template_injects_css() {
    let document = prepare_test_document(
        DiffSyntaxLanguage::JavaScript,
        "const styles = css`color: red;`;",
    );
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "tagged CSS template should inject CSS property highlighting: {tokens:?}"
    );
}

#[test]
fn javascript_tagged_template_injects_html() {
    let text = "const markup = html`<div class=\"note\">ok</div>`;";
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
        "tagged HTML template should inject HTML tags in JavaScript: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
        "tagged HTML template should inject HTML attributes in JavaScript: {tokens:?}"
    );
}

#[test]
fn javascript_styled_member_template_injects_css() {
    let text = "const Button = styled.div`color: red;`;";
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "styled member templates should inject CSS properties in JavaScript: {tokens:?}"
    );
}

#[test]
fn javascript_styled_call_template_injects_css() {
    let text = "const Button = styled(Link)`color: red;`;";
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "styled call templates should inject CSS properties in JavaScript: {tokens:?}"
    );
}

#[test]
fn javascript_comment_prefixed_string_injects_html() {
    let text = r#"const markup = /* html */ "<div class='note'>ok</div>";"#;
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
        "comment-prefixed HTML string should inject HTML tags: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
        "comment-prefixed HTML string should inject HTML attributes: {tokens:?}"
    );
}

#[test]
fn javascript_comment_prefixed_string_injects_css() {
    let text = r#"const styles = /* css */ "color: red;";"#;
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "comment-prefixed CSS strings should inject CSS properties in JavaScript: {tokens:?}"
    );
}

#[test]
fn javascript_comment_prefixed_template_literal_injects_css() {
    let text = "const styles = /* css */ `color: red;`;";
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("JavaScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "comment-prefixed CSS template literals should inject CSS properties: {tokens:?}"
    );
}

#[test]
fn typescript_tagged_template_injects_yaml() {
    let text = "const workflow = yaml`enabled: true`;";
    let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TypeScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "enabled"),
        "tagged YAML template should inject YAML properties: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Boolean, "true"),
        "tagged YAML template should inject YAML booleans: {tokens:?}"
    );
}

#[test]
fn typescript_tagged_template_injects_html() {
    let text = "const markup = html`<div class=\"note\">ok</div>`;";
    let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TypeScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
        "tagged HTML template should inject HTML tags in TypeScript: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
        "tagged HTML template should inject HTML attributes in TypeScript: {tokens:?}"
    );
}

#[test]
fn typescript_tagged_template_injects_sql() {
    let text = "const query = sql`select name from users`;";
    let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TypeScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "select"),
        "tagged SQL template should inject SQL keywords in TypeScript: {tokens:?}"
    );
}

#[test]
fn typescript_comment_prefixed_string_injects_css() {
    let text = r#"const styles = /* css */ "color: red;";"#;
    let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TypeScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "comment-prefixed CSS strings should inject CSS properties in TypeScript: {tokens:?}"
    );
}

#[test]
fn typescript_component_styles_array_template_injects_css() {
    let text = "Component({ styles: [`div { color: red; }`] });";
    let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TypeScript document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
        "TypeScript Component styles templates should inject CSS properties: {tokens:?}"
    );
}

#[test]
fn tsx_tagged_template_injects_html() {
    let text = "const markup = html`<div class=\"note\">ok</div>`;";
    let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("TSX document should have prepared tokens");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
        "tagged HTML template should inject HTML tags in TSX: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
        "tagged HTML template should inject HTML attributes in TSX: {tokens:?}"
    );
}

#[test]
fn go_treesitter_captures_function_method_property_and_number() {
    let declaration = "func Hello(a B) C { return C{} }";
    let declaration_tokens =
        syntax_tokens_for_line(declaration, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            declaration,
            &declaration_tokens,
            SyntaxTokenKind::Function,
            "Hello",
        ),
        "Go should capture function declarations: {declaration_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(declaration, &declaration_tokens, SyntaxTokenKind::Type, "B"),
        "Go should capture parameter types: {declaration_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(declaration, &declaration_tokens, SyntaxTokenKind::Type, "C"),
        "Go should capture return or composite literal types: {declaration_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            declaration,
            &declaration_tokens,
            SyntaxTokenKind::Keyword,
            "return",
        ),
        "Go should capture keywords: {declaration_tokens:?}"
    );

    let method_call = "value.Do(42)";
    let method_call_tokens =
        syntax_tokens_for_line(method_call, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            method_call,
            &method_call_tokens,
            SyntaxTokenKind::FunctionMethod,
            "Do",
        ),
        "Go should capture method calls: {method_call_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            method_call,
            &method_call_tokens,
            SyntaxTokenKind::Number,
            "42",
        ),
        "Go should capture numeric literals: {method_call_tokens:?}"
    );

    let field_access = "value.Field";
    let field_access_tokens =
        syntax_tokens_for_line(field_access, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            field_access,
            &field_access_tokens,
            SyntaxTokenKind::Property,
            "Field",
        ),
        "Go should capture field accesses: {field_access_tokens:?}"
    );
}

#[test]
fn go_comment_prefixed_strings_inject_supported_languages() {
    let cases = [
        (
            r#"var payload = /* json */ `{"count": 42}`;"#,
            SyntaxTokenKind::Number,
            "42",
        ),
        (
            r#"var config = /* yaml */ `enabled: true`;"#,
            SyntaxTokenKind::Boolean,
            "true",
        ),
        (
            r#"var markup = /* html */ `<div class="note">ok</div>`;"#,
            SyntaxTokenKind::Tag,
            "div",
        ),
        (
            r#"var markup = /* xml */ `<root attr="value"/>`;"#,
            SyntaxTokenKind::Tag,
            "root",
        ),
        (
            r#"var script = /* js */ `const value = 42;`;"#,
            SyntaxTokenKind::Number,
            "42",
        ),
        (
            r#"var query = /* sql */ `select name from users`;"#,
            SyntaxTokenKind::Keyword,
            "select",
        ),
    ];

    for (text, expected_kind, expected_text) in cases {
        let document = prepare_test_document(DiffSyntaxLanguage::Go, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("Go document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, expected_kind, expected_text),
            "Go comment-prefixed injection should produce {expected_kind:?} token {expected_text:?}: {tokens:?}"
        );
    }
}

#[test]
fn yaml_github_actions_script_injects_javascript() {
    let text = [
        "jobs:",
        "  test:",
        "    steps:",
        "      - uses: actions/github-script@v7",
        "        with:",
        "          script: |",
        "            const value = 42",
    ]
    .join("\n");
    let document = prepare_test_document(DiffSyntaxLanguage::Yaml, &text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 6)
        .expect("YAML github-script line should have prepared tokens");
    assert!(
        tokens.iter().any(|t| {
            t.kind == SyntaxTokenKind::Keyword || t.kind == SyntaxTokenKind::KeywordControl
        }),
        "github-script YAML block should inject JavaScript keywords: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
        "github-script YAML block should inject JavaScript numbers: {tokens:?}"
    );
}

#[test]
fn yaml_github_actions_inline_script_injects_javascript() {
    let text = [
        "jobs:",
        "  test:",
        "    steps:",
        "      - uses: actions/github-script@v7",
        "        with:",
        "          script: const value = 42",
    ]
    .join("\n");
    let inline_line = text.lines().nth(5).unwrap_or_default();
    let document = prepare_test_document(DiffSyntaxLanguage::Yaml, &text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 5)
        .expect("YAML github-script inline line should have prepared tokens");
    assert!(
        has_token_kind_and_text(inline_line, &tokens, SyntaxTokenKind::Keyword, "const")
            || tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::KeywordControl),
        "github-script YAML inline scalars should inject JavaScript keywords: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
        "github-script YAML inline scalars should inject JavaScript numbers: {tokens:?}"
    );
}

#[test]
fn extra_languages_capture_basic_semantic_tokens() {
    let cases = [
        (
            DiffSyntaxLanguage::C,
            "int main(void) { return 0; }",
            SyntaxTokenKind::Function,
        ),
        (
            DiffSyntaxLanguage::Cpp,
            "auto value = std::vector<int>{1, 2};",
            SyntaxTokenKind::Type,
        ),
        (
            DiffSyntaxLanguage::CSharp,
            "public class Example { string Name { get; } }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Bicep,
            "param location string = 'westeurope'",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::ObjectiveC,
            "NSString *value = @\"hi\";",
            SyntaxTokenKind::Property,
        ),
        (
            DiffSyntaxLanguage::FSharp,
            "let value = 42",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Java,
            "class Example { int value() { return 1; } }",
            SyntaxTokenKind::FunctionMethod,
        ),
        (
            DiffSyntaxLanguage::Php,
            "<?php function foo(): int { return 1; }",
            SyntaxTokenKind::Function,
        ),
        (
            DiffSyntaxLanguage::Ruby,
            "class Example; def call(name) = 42 end",
            SyntaxTokenKind::FunctionMethod,
        ),
        (
            DiffSyntaxLanguage::PowerShell,
            "function Invoke-Test { return 42 }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Swift,
            "struct Example { let value = 42 }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::R,
            "if (TRUE) print(1)",
            SyntaxTokenKind::Boolean,
        ),
        (
            DiffSyntaxLanguage::Dart,
            "class Example { int value() => 42; }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Scala,
            "object Example { def run(): Int = 42 }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Toml,
            "enabled = true",
            SyntaxTokenKind::Property,
        ),
        (
            DiffSyntaxLanguage::Lua,
            "local value = 42",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Kotlin,
            "class Example { fun run() = 42 }",
            SyntaxTokenKind::Function,
        ),
        (
            DiffSyntaxLanguage::Zig,
            "const value: u32 = 42;",
            SyntaxTokenKind::TypeBuiltin,
        ),
        (
            DiffSyntaxLanguage::Sql,
            "select name from users",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Groovy,
            "class Example { def run() { return 42 } }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Clojure,
            "(defn run [] 42)",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Elixir,
            "defmodule Example do end",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Erlang,
            "run(X) -> X + 1.",
            SyntaxTokenKind::Function,
        ),
        (
            DiffSyntaxLanguage::Haskell,
            "run :: Int -> Int",
            SyntaxTokenKind::Type,
        ),
        (
            DiffSyntaxLanguage::Julia,
            "function run(x) x + 1 end",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::OCaml,
            "let run x = x + 1",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::OCamlInterface,
            "val run : int -> int",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Solidity,
            "contract Example { uint256 value; }",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Assembly,
            "  mov eax, 1",
            SyntaxTokenKind::Number,
        ),
        (
            DiffSyntaxLanguage::Svelte,
            "<button class=\"go\">go</button>",
            SyntaxTokenKind::Tag,
        ),
    ];

    for (language, text, expected_kind) in cases {
        let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|token| token.kind == expected_kind),
            "{language:?} should capture {expected_kind:?}: {tokens:?}"
        );
    }
}

#[test]
fn repo_languages_capture_basic_semantic_tokens() {
    let cases = [
        (
            DiffSyntaxLanguage::GoMod,
            "module example.com/project",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::GoWork,
            "use ./module",
            SyntaxTokenKind::Keyword,
        ),
        (
            DiffSyntaxLanguage::Diff,
            "diff --git a/src/lib.rs b/src/lib.rs",
            SyntaxTokenKind::VariableBuiltin,
        ),
        (
            DiffSyntaxLanguage::GitCommit,
            "feat: widen syntax support",
            SyntaxTokenKind::MarkupHeading,
        ),
    ];

    for (language, text, expected_kind) in cases {
        let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|token| token.kind == expected_kind),
            "{language:?} should capture {expected_kind:?}: {tokens:?}"
        );
    }
}

#[test]
fn javascript_treesitter_captures_regex_literal() {
    let text = "const re = /foo+/gi;";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::JavaScript, DiffSyntaxMode::Auto);
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::StringRegex),
        "JavaScript regex literal should produce StringRegex token, got: {tokens:?}"
    );
}

#[test]
fn javascript_treesitter_captures_constructor_and_constant_builtin() {
    let constructor_tokens = syntax_tokens_for_line(
        "class Example { constructor() {} }",
        DiffSyntaxLanguage::JavaScript,
        DiffSyntaxMode::Auto,
    );
    assert!(
        constructor_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Constructor),
        "JavaScript constructor should produce Constructor token, got: {constructor_tokens:?}"
    );

    let builtin_tokens = syntax_tokens_for_line(
        "const value = undefined;",
        DiffSyntaxLanguage::JavaScript,
        DiffSyntaxMode::Auto,
    );
    assert!(
        builtin_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::ConstantBuiltin),
        "JavaScript builtins should produce ConstantBuiltin token, got: {builtin_tokens:?}"
    );
}

#[test]
fn go_treesitter_captures_namespace_package_identifier() {
    let tokens =
        syntax_tokens_for_line("package main", DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Namespace),
        "Go package identifier should produce Namespace token, got: {tokens:?}"
    );
}

#[test]
fn lua_and_c_treesitter_capture_preprocessor_and_label() {
    let preproc = syntax_tokens_for_line(
        "#!/usr/bin/env lua",
        DiffSyntaxLanguage::Lua,
        DiffSyntaxMode::Auto,
    );
    assert!(
        preproc.iter().any(|t| t.kind == SyntaxTokenKind::Preproc),
        "Lua hash bang should produce Preproc token, got: {preproc:?}"
    );

    let label = syntax_tokens_for_line(
        "start: return 0;",
        DiffSyntaxLanguage::C,
        DiffSyntaxMode::Auto,
    );
    assert!(
        label.iter().any(|t| t.kind == SyntaxTokenKind::Label),
        "C label should produce Label token, got: {label:?}"
    );
}

#[test]
fn c_treesitter_uses_vendored_zed_query() {
    let preproc = syntax_tokens_for_line(
        "#define VALUE 42",
        DiffSyntaxLanguage::C,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            "#define VALUE 42",
            &preproc,
            SyntaxTokenKind::Preproc,
            "#define"
        ),
        "C preprocessor directives should produce Preproc tokens, got: {preproc:?}"
    );

    let text = "struct Example { int field; };";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::C, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "struct"),
        "C storage/type keywords should be captured, got: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "field"),
        "C field identifiers should produce Property tokens, got: {tokens:?}"
    );
}

#[test]
fn cpp_treesitter_uses_vendored_zed_query() {
    let concept_text = "template <typename T> concept Addable = requires(T a, T b) { a + b; };";
    let concept_tokens =
        syntax_tokens_for_line(concept_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            concept_text,
            &concept_tokens,
            SyntaxTokenKind::TypeInterface,
            "Addable"
        ),
        "C++ concepts should produce TypeInterface tokens, got: {concept_tokens:?}"
    );
    assert!(
        has_token_kind_and_text(
            concept_text,
            &concept_tokens,
            SyntaxTokenKind::Keyword,
            "requires"
        ),
        "C++ requires should produce Keyword tokens, got: {concept_tokens:?}"
    );

    let module_text = "export module math.core; import std;";
    let module_tokens =
        syntax_tokens_for_line(module_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            module_text,
            &module_tokens,
            SyntaxTokenKind::Keyword,
            "module"
        ),
        "C++ module declarations should produce Keyword tokens, got: {module_tokens:?}"
    );
    assert!(
        module_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Namespace),
        "C++ module names should produce Namespace tokens, got: {module_tokens:?}"
    );

    let static_assert_text = "static_assert(sizeof(int) > 0);";
    let static_assert_tokens = syntax_tokens_for_line(
        static_assert_text,
        DiffSyntaxLanguage::Cpp,
        DiffSyntaxMode::Auto,
    );
    assert!(
        has_token_kind_and_text(
            static_assert_text,
            &static_assert_tokens,
            SyntaxTokenKind::Function,
            "static_assert"
        ),
        "C++ static_assert should produce Function tokens, got: {static_assert_tokens:?}"
    );

    let operator_text = "auto cmp = lhs <=> rhs;";
    let operator_tokens =
        syntax_tokens_for_line(operator_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            operator_text,
            &operator_tokens,
            SyntaxTokenKind::Operator,
            "<=>"
        ),
        "C++ spaceship operators should produce Operator tokens, got: {operator_tokens:?}"
    );

    let preproc_text = "#include <vector>";
    let preproc_tokens =
        syntax_tokens_for_line(preproc_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
    assert!(
        has_token_kind_and_text(
            preproc_text,
            &preproc_tokens,
            SyntaxTokenKind::Preproc,
            "#include"
        ),
        "C++ preprocessor directives should produce Preproc tokens, got: {preproc_tokens:?}"
    );
}

#[test]
fn injected_web_helper_languages_capture_basic_tokens() {
    let regex_text = "(foo|bar)+";
    let regex_tokens =
        syntax_tokens_for_line(regex_text, DiffSyntaxLanguage::Regex, DiffSyntaxMode::Auto);
    assert!(
        regex_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Operator),
        "Regex syntax should capture operators, got: {regex_tokens:?}"
    );

    let jsdoc_text = "@param {string} name";
    let jsdoc_tokens =
        syntax_tokens_for_line(jsdoc_text, DiffSyntaxLanguage::Jsdoc, DiffSyntaxMode::Auto);
    assert!(
        jsdoc_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Keyword),
        "JSDoc syntax should capture tags as keywords, got: {jsdoc_tokens:?}"
    );
}

#[test]
fn gitcommit_treesitter_captures_diff_change_kinds() {
    let text = [
        "Subject",
        "",
        "# Changes to be committed:",
        "# new file: src/new.rs",
        "# deleted: src/old.rs",
        "# modified: src/lib.rs",
    ]
    .join("\n");
    let document = prepare_test_document(DiffSyntaxLanguage::GitCommit, &text);

    let plus = syntax_tokens_for_prepared_document_line(document, 3)
        .expect("gitcommit added line should have prepared tokens");
    assert!(
        plus.iter().any(|t| t.kind == SyntaxTokenKind::DiffPlus),
        "gitcommit additions should produce DiffPlus tokens, got: {plus:?}"
    );

    let minus = syntax_tokens_for_prepared_document_line(document, 4)
        .expect("gitcommit removed line should have prepared tokens");
    assert!(
        minus.iter().any(|t| t.kind == SyntaxTokenKind::DiffMinus),
        "gitcommit removals should produce DiffMinus tokens, got: {minus:?}"
    );

    let delta = syntax_tokens_for_prepared_document_line(document, 5)
        .expect("gitcommit modified file line should have prepared tokens");
    assert!(
        delta.iter().any(|t| t.kind == SyntaxTokenKind::DiffDelta),
        "gitcommit modified files should produce DiffDelta tokens, got: {delta:?}"
    );
}

#[test]
fn prepared_documents_capture_markup_specific_tokens() {
    let gitcommit = prepare_test_document(DiffSyntaxLanguage::GitCommit, "Subject\n\ncloses #123");

    let heading = syntax_tokens_for_prepared_document_line(gitcommit, 0)
        .expect("gitcommit subject line should have prepared tokens");
    assert!(
        heading
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::MarkupHeading),
        "gitcommit subject should produce MarkupHeading token, got: {heading:?}"
    );

    let link = syntax_tokens_for_prepared_document_line(gitcommit, 2)
        .expect("gitcommit body line should have prepared tokens");
    assert!(
        link.iter().any(|t| t.kind == SyntaxTokenKind::MarkupLink),
        "gitcommit issue reference should produce MarkupLink token, got: {link:?}"
    );

    let xml = prepare_test_document(DiffSyntaxLanguage::Xml, "<root><![CDATA[code]]></root>");
    let literal = syntax_tokens_for_prepared_document_line(xml, 0)
        .expect("XML CDATA line should have prepared tokens");
    assert!(
        literal
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::TextLiteral),
        "XML CDATA should produce TextLiteral token, got: {literal:?}"
    );
}

#[test]
fn markdown_inline_treesitter_captures_text_literal_and_markup_link() {
    let text = "[link](https://example.com) `code`";
    let tokens = syntax_tokens_for_line(
        text,
        DiffSyntaxLanguage::MarkdownInline,
        DiffSyntaxMode::Auto,
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::MarkupLink),
        "Markdown inline link destination should produce MarkupLink token, got: {tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::TextLiteral),
        "Markdown inline code span should produce TextLiteral token, got: {tokens:?}"
    );
}

#[test]
fn markdown_prepared_document_captures_heading_marker_as_punctuation_special() {
    let document = prepare_test_document(DiffSyntaxLanguage::Markdown, "# Heading");
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("markdown heading line should have prepared tokens");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::PunctuationSpecial),
        "Markdown heading marker should remain PunctuationSpecial, got: {tokens:?}"
    );
}

#[test]
fn ruby_and_swift_treesitter_capture_regex_aliases() {
    let cases = [
        (DiffSyntaxLanguage::Ruby, "value = /foo+/"),
        (DiffSyntaxLanguage::Swift, "let pattern = /foo+/"),
    ];

    for (language, text) in cases {
        let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::StringRegex),
            "{language:?} regex literal should produce StringRegex token, got: {tokens:?}"
        );
    }
}
