use tree_sitter::{Node, Parser, Tree};

fn parse(source: &str) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&super::LANGUAGE.into()).unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{source}\n{}",
        tree.root_node().to_sexp()
    );
    tree
}

fn captured<'a>(node: Node<'_>, source: &'a str, kind: &str) -> Vec<&'a str> {
    let mut result = Vec::new();
    if node.kind() == kind {
        result.push(&source[node.byte_range()]);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        result.extend(captured(child, source, kind));
    }
    result
}

#[test]
fn implicit_chains_stop_before_markup_and_keep_unicode_members() {
    let source = "<p>@value.Näme / @value.café / @value?.Items?[0] / @value.😀 / @value (text) / @value [0]</p>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        [
            "@value.Näme",
            "@value.café",
            "@value?.Items?[0]",
            "@value",
            "@value",
            "@value"
        ]
    );
}

#[test]
fn comments_line_transitions_and_raw_markup_expose_expressions() {
    let source = "<!-- @name -->\n@{\n@:Hello @name\n}\n<script>if (1 < 2) { const value = '@name'; }</script>\n<style>.a { color: @color; }</style>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@name", "@name", "@name", "@color"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_raw_markup"),
        [
            "if (1 < 2) { const value = '@name'; }",
            ".a { color: @color; }"
        ]
    );
}

#[test]
fn raw_markup_continues_member_call_and_index_chains_before_resuming_text() {
    let source = "<!-- @Model.Name. -->\n@{\n@:Line @Model.Name.\n}\n<script>const s = '@Model.Name'; const n = @Html.Raw(items[0]);</script><style>.a { width: @Model.Width; }</style>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        [
            "@Model.Name",
            "@Model.Name",
            "@Model.Name",
            "@Html.Raw(items[0])",
            "@Model.Width"
        ]
    );
}

#[test]
fn partial_raw_closing_tags_do_not_turn_razor_transitions_into_email_text() {
    let source = "<script>const s = 'a<@Model.Name / a</@Model.Name';</script><!-- a-@Model.Name --><style>.x { content: 'a<@Model.Name / a</@Model.Name'; }</style>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        [
            "@Model.Name",
            "@Model.Name",
            "@Model.Name",
            "@Model.Name",
            "@Model.Name"
        ]
    );
}

#[test]
fn escapes_contribute_exactly_the_second_at() {
    let source = "<p>@@ / @@@name</p><style>@@media screen {}</style>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "escaped_at"),
        ["@", "@", "@"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@name"]
    );
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if node.kind() == "escaped_at" {
            assert_eq!(source.as_bytes()[node.start_byte() - 1], b'@');
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

#[test]
fn literal_binding_modifiers_do_not_become_csharp() {
    let source = r#"<input @bind-value:event="oninput" @bind:format="yyyy-MM-dd" @bind:after="After" /><form @formname="form-@(name)"></form>"#;
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_literal_attribute_name"),
        ["@bind-value:event", "@bind:format", "@formname"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_attribute_literal_text"),
        ["oninput", "yyyy-MM-dd", "form-"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_attribute_value"),
        ["\"After\""]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_explicit_expression"),
        ["@(name)"]
    );
}

#[test]
fn special_tag_names_do_not_consume_name_prefixes() {
    let source =
        "<scripture>prose</scripture><inputBox>prose</inputBox><th:input value=x><SCRIPT></SCRIPT>";
    let tree = parse(source);
    assert_eq!(captured(tree.root_node(), source, "element").len(), 4);
    assert!(captured(tree.root_node(), source, "razor_raw_markup").is_empty());
}

#[test]
fn csharp_fourteen_scanner_and_raw_strings_keep_their_boundaries() {
    let source = r####"@functions {
        public static class Helpers {
            extension(string value) { public int Count() => value.Length; }
        }
        private void Run() {
            var action = (ref int café) => café++;
            var raw = $$"""<script>@notRazor {{1 + 2}} </script>""";
            var @class = "escaped";
        }
    }"####;
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "lambda_expression"),
        ["(ref int café) => café++"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "extension_declaration").len(),
        1
    );
    assert!(captured(tree.root_node(), source, "element").is_empty());
    assert!(captured(tree.root_node(), source, "razor_implicit_expression").is_empty());
}

#[test]
fn preprocessor_and_legacy_templates_keep_nested_markup() {
    let source = "@helper Card(string name) { <p>@name</p> }\n@{ Func<dynamic, object> render = @<p>@item.Name</p>; }";
    let tree = parse(source);
    assert_eq!(captured(tree.root_node(), source, "razor_helper").len(), 1);
    assert_eq!(
        captured(tree.root_node(), source, "razor_template"),
        ["@<p>@item.Name</p>"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@name", "@item.Name"]
    );
}

#[test]
fn unterminated_razor_comments_own_the_remainder_without_hiding_following_valid_markup() {
    for (source, expected) in [
        (
            "<p>Before</p>@* <p>@name</p> \"text\"",
            "@* <p>@name</p> \"text\"",
        ),
        ("@* closed *@<p>@name</p>", "@* closed *@"),
    ] {
        let tree = parse(source);
        assert_eq!(
            captured(tree.root_node(), source, "razor_comment"),
            [expected]
        );
        assert_eq!(captured(tree.root_node(), source, "element").len(), 1);
    }
}

#[test]
fn unicode_identifiers_cannot_alias_ascii_control_or_parameter_keywords() {
    let source = "<p>@ũf(value)</p>@{ var action = (rťf) => rťf; }";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@ũf(value)"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "lambda_expression"),
        ["(rťf) => rťf"]
    );
    assert!(captured(tree.root_node(), source, "razor_if").is_empty());
}

#[test]
fn control_continuations_remain_code_after_nested_markup() {
    let source = "@if (ok) { <p>yes</p> } else { <p>no</p> } @try { <p>try</p> } catch (Exception error) { <p>@error.Message</p> } finally { <p>end</p> } @do { <p>do</p> } while (ok);";
    let tree = parse(source);
    for keyword in ["else", "catch", "finally", "while"] {
        assert_eq!(captured(tree.root_node(), source, keyword), [keyword]);
    }
}

#[test]
fn prose_after_control_blocks_keeps_email_addresses_as_markup() {
    let source = "@if (ok) { <p>yes</p> } user@example.com";
    let tree = parse(source);
    assert!(captured(tree.root_node(), source, "razor_implicit_expression").is_empty());
}

#[test]
fn markup_comment_delimiters_do_not_hide_live_razor_expressions() {
    let source = "<p>/* prose @Model.Name */</p>@{ /* C# comment @notRazor */ }";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@Model.Name"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "comment"),
        ["/* C# comment @notRazor */"]
    );
}

#[test]
fn tag_helper_opt_out_markers_are_separate_from_html_tag_names() {
    let source = "<!input value=\"@Model.Title\" /><!div>text</!div>";
    let tree = parse(source);
    assert_eq!(
        captured(tree.root_node(), source, "razor_tag_opt_out"),
        ["!", "!", "!"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_tag_name"),
        ["div", "div"]
    );
    assert_eq!(
        captured(tree.root_node(), source, "razor_implicit_expression"),
        ["@Model.Title"]
    );
}
