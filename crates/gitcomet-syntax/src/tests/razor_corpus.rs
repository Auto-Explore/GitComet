use super::*;

type ExpectedSpan = (&'static str, &'static str, Option<SyntaxTokenKind>);

fn span_failure(
    sample: &Sample,
    context: &str,
    fragment: &str,
    expected: Option<SyntaxTokenKind>,
) -> Option<String> {
    let start = sample
        .text
        .find(context)
        .unwrap_or_else(|| panic!("{} lacks {context:?}", sample.path.display()))
        + context.find(fragment).unwrap();
    let end = start + fragment.len();
    let line_ix = line_ix_for_byte(&sample.line_starts, start);
    let line_start = sample.line_starts[line_ix];
    assert!(
        end <= line_start + sample.line(line_ix).len(),
        "single-line assertion required"
    );
    let tokens = syntax_tokens_for_prepared_document_line(sample.document, line_ix).unwrap();
    for byte in start..end {
        let actual = tokens
            .iter()
            .find(|token| token.range.contains(&(byte - line_start)))
            .map(|token| token.kind);
        if actual != expected {
            return Some(format!(
                "{}: {fragment:?} at byte {byte}: {actual:?}, expected {expected:?}",
                sample.path.display()
            ));
        }
    }
    None
}

#[test]
fn syntax_corpus_razor_classifies_valid_legacy_and_recovery_boundaries() {
    use SyntaxTokenKind as K;
    let root = match corpus_root() {
        Ok(root) => root,
        Err(reason) => {
            assert!(
                std::env::var("GITCOMET_REQUIRE_SYNTAX_CORPUS").as_deref() != Ok("1"),
                "{reason}"
            );
            println!("skipping Razor corpus: {reason}");
            return;
        }
    };
    let cases: &[(&str, &[ExpectedSpan])] = &[
        (
            "Bindings.razor",
            &[
                (
                    "@bind-value:event=\"oninput\"",
                    "@bind-value:event",
                    Some(K::Attribute),
                ),
                ("@bind-value:event=\"oninput\"", "oninput", Some(K::String)),
                ("@bind:format=\"yyyy-MM-dd\"", "yyyy-MM-dd", Some(K::String)),
                ("@bind:set=\"SetName\"", "SetName", Some(K::Variable)),
            ],
        ),
        (
            "Components.razor",
            &[
                ("@page \"/components/{Id:int?}\"", "@page", Some(K::Keyword)),
                (
                    "<RazorCorpus.Components.SamplePanel",
                    "RazorCorpus.Components.SamplePanel",
                    Some(K::Tag),
                ),
                ("<span>@Id</span>", "Id", Some(K::Variable)),
            ],
        ),
        (
            "Embedded.razor",
            &[
                ("<script>const yes = 1 < 2;", "const", Some(K::Keyword)),
                ("\"count\": @count", "@", Some(K::PunctuationSpecial)),
                ("\"count\": @count", "count", Some(K::Property)),
                (".component { color: @color;", "color", Some(K::Property)),
            ],
        ),
        (
            "Events.razor",
            &[
                (
                    "@onclick:preventDefault",
                    "@onclick:preventDefault",
                    Some(K::Attribute),
                ),
                ("@onclick=\"Click\"", "Click", Some(K::Variable)),
                ("@formname=\"sample-form\"", "sample-form", Some(K::String)),
            ],
        ),
        (
            "Generics.razor",
            &[
                ("@typeparam TItem where", "@typeparam", Some(K::Keyword)),
                ("where TItem : class", "class", Some(K::Keyword)),
                ("<GenericPanel TItem=", "GenericPanel", Some(K::Tag)),
            ],
        ),
        (
            "Page.cshtml",
            &[
                ("@model DemoModel", "@model", Some(K::Keyword)),
                ("var name = \"world\";", "\"world\"", Some(K::String)),
                ("<meta charset=\"utf-8\">", "meta", Some(K::Tag)),
                ("const label = `@name has", "name", Some(K::Variable)),
                ("<p>@width - 1</p>", " - 1", None),
            ],
        ),
        (
            "RenderModes.razor",
            &[
                ("@rendermode @(new", "@rendermode", Some(K::Keyword)),
                (
                    "@rendermode=\"InteractiveServer\"",
                    "InteractiveServer",
                    Some(K::Variable),
                ),
            ],
        ),
        (
            "Templates.razor",
            &[
                ("@<h2>Header", "h2", Some(K::Tag)),
                ("@DateTime.UnixEpoch.Year", "Year", Some(K::Variable)),
                ("@<p class=\"row\">", "row", Some(K::String)),
            ],
        ),
        (
            "Transitions.razor",
            &[
                ("user@example.com.", "user@example.com.", None),
                ("@optional?.Title", "Title", Some(K::Variable)),
                (
                    "<!-- Live expression inside an HTML comment: @name -->",
                    "name",
                    Some(K::Variable),
                ),
                ("@:Line output: @name", "Line output: ", None),
            ],
        ),
        (
            "_Imports.razor",
            &[
                ("@using static Microsoft", "static", Some(K::Keyword)),
                (
                    "@namespace RazorCorpus.Imported",
                    "@namespace",
                    Some(K::Keyword),
                ),
            ],
        ),
        (
            "_ViewImports.cshtml",
            &[
                ("@addTagHelper *", "@addTagHelper", Some(K::Keyword)),
                (
                    "@removeTagHelper Microsoft",
                    "@removeTagHelper",
                    Some(K::Keyword),
                ),
            ],
        ),
        (
            "_ViewStart.cshtml",
            &[
                ("Layout = \"_Layout\"", "\"_Layout\"", Some(K::String)),
                (
                    "ViewData[\"Started\"] = true",
                    "true",
                    Some(K::ConstantBuiltin),
                ),
            ],
        ),
        (
            "async_control.cshtml",
            &[
                ("@await foreach", "@await foreach", Some(K::Keyword)),
                ("<p>@item</p>", "item", Some(K::Variable)),
                ("yield return 1", "yield", Some(K::Keyword)),
            ],
        ),
        (
            "attributes.cshtml",
            &[
                ("class=\"prefix @(active", "prefix ", Some(K::String)),
                ("@(active ? \"enabled\"", "active", Some(K::Variable)),
                ("@(active ? \"enabled\"", "\"enabled\"", Some(K::String)),
                ("data-size=@Model.Items.Count", "Count", Some(K::Variable)),
            ],
        ),
        (
            "blocks.cshtml",
            &[
                (
                    "void RenderItem(string item)",
                    "RenderItem",
                    Some(K::Function),
                ),
                ("<span data-length=", "span", Some(K::Tag)),
                ("@foreach (var (item, index)", "@foreach", Some(K::Keyword)),
                (
                    "catch (InvalidOperationException error)",
                    "catch",
                    Some(K::Keyword),
                ),
                ("else if (items.Length == 2)", "else", Some(K::Keyword)),
                ("else if (items.Length == 2)", "Length", Some(K::Variable)),
                ("else {}", "else", Some(K::Keyword)),
                ("finally { <p>finished</p> }", "finally", Some(K::Keyword)),
                ("} while (count > 0);", "while", Some(K::Keyword)),
                (
                    "case string text when text.Length > 0:",
                    "string",
                    Some(K::TypeBuiltin),
                ),
                (
                    "case string text when text.Length > 0:",
                    "when",
                    Some(K::Keyword),
                ),
            ],
        ),
        (
            "comments.cshtml",
            &[
                (
                    "<!-- This HTML comment contains",
                    "This HTML comment contains",
                    Some(K::Comment),
                ),
                ("expression: @Model.Title.", "Title", Some(K::Variable)),
                ("<p>// This is prose,", "// This is prose,", None),
                ("live @Model.Name expression", "Name", Some(K::Variable)),
            ],
        ),
        (
            "directives.cshtml",
            &[
                ("@section Footer", "@section", Some(K::Keyword)),
                ("<footer>@Model.Name</footer>", "footer", Some(K::Tag)),
                (
                    "private void RenderGreeting",
                    "RenderGreeting",
                    Some(K::Function),
                ),
            ],
        ),
        (
            "embedded.cshtml",
            &[
                ("const smaller = 1 < 2", "const", Some(K::Keyword)),
                ("const greeting = `@Model.Name:", "Name", Some(K::Variable)),
                ("@@media (min-width:", "@media", Some(K::Keyword)),
                (
                    "type=\"application/json\">{\"count\":",
                    "count",
                    Some(K::Property),
                ),
                ("onclick=\"if (1 < 2)", "if", Some(K::Keyword)),
            ],
        ),
        (
            "expressions.cshtml",
            &[
                ("@width - 1", " - 1", None),
                ("@optional?.Title", "Title", Some(K::Variable)),
                ("@optional?.Items?[0]", "0", Some(K::Number)),
                ("@await Task.FromResult", "await", Some(K::Keyword)),
                ("Identity<List<string>>", "string", Some(K::TypeBuiltin)),
            ],
        ),
        (
            "inheritance.cshtml",
            &[
                (
                    "@inherits RazorCorpus.CorpusView",
                    "@inherits",
                    Some(K::Keyword),
                ),
                ("<p>@Model.Title</p>", "Title", Some(K::Variable)),
            ],
        ),
        (
            "legacy_helpers.cshtml",
            &[
                ("@helper Card", "@helper", Some(K::Keyword)),
                ("@helper Card", "Card", Some(K::Function)),
                ("<article class=\"card\">", "article", Some(K::Tag)),
                ("<h2>@title</h2>", "title", Some(K::Variable)),
            ],
        ),
        (
            "markup.cshtml",
            &[
                (
                    "Plain markup may precede the first tag.",
                    "Plain markup",
                    None,
                ),
                ("<meta charset=utf-8>", "meta", Some(K::Tag)),
                ("data-name=unquoted", "unquoted", Some(K::String)),
                (
                    "src=\"//cdn.example.test/icon.png\"",
                    "//cdn.example.test/icon.png",
                    Some(K::String),
                ),
            ],
        ),
        (
            "model_types.cshtml",
            &[
                (
                    "@model IEnumerable<Dictionary<string, List<int>>>",
                    "string",
                    Some(K::TypeBuiltin),
                ),
                (
                    "@inject Microsoft.Extensions.Logging.ILogger<DemoModel>",
                    "ILogger",
                    Some(K::Type),
                ),
            ],
        ),
        (
            "modern_language.cshtml",
            &[
                ("item?.Title =", "Title", Some(K::Variable)),
                ("0x_FF + 0b_1010", "0x_FF", Some(K::Number)),
                ("public string Title", "string", Some(K::TypeBuiltin)),
            ],
        ),
        (
            "no_final_newline.cshtml",
            &[
                ("@(1 + 2)", "1", Some(K::Number)),
                ("@@literal / naïve", "literal / naïve", None),
            ],
        ),
        (
            "preprocessor.cshtml",
            &[
                (
                    "var configuration = \"debug\"",
                    "\"debug\"",
                    Some(K::String),
                ),
                ("<p>@configuration</p>", "configuration", Some(K::Variable)),
            ],
        ),
        (
            "route_less.cshtml",
            &[
                ("@page", "@page", Some(K::Keyword)),
                ("<p>@Model.Title</p>", "Title", Some(K::Variable)),
            ],
        ),
        (
            "strings.cshtml",
            &[
                (
                    "@\"double \"\"quote\"\", @* text *@, <tag>\"",
                    "@* text *@",
                    Some(K::String),
                ),
                (
                    "\"\"\"quotes \" and \"\", @* not a Razor comment *@, </script>\"\"\"",
                    "</script>",
                    Some(K::String),
                ),
                (
                    "<div class=\"quoted\">@identifier and { braces }</div>",
                    "@identifier",
                    Some(K::String),
                ),
            ],
        ),
        (
            "tag_helpers.cshtml",
            &[
                ("<th:form asp-controller", "th:form", Some(K::Tag)),
                ("<th:input asp-for=\"Title\"", "asp-for", Some(K::Attribute)),
                ("<!input value=", "input", Some(K::Tag)),
                ("<!input value=", "!", Some(K::PunctuationSpecial)),
            ],
        ),
        (
            "templates.cshtml",
            &[
                ("@<article data-id=", "article", Some(K::Tag)),
                ("<h2>@item.Title</h2>", "Title", Some(K::Variable)),
                ("<strong>enabled</strong>", "enabled", None),
            ],
        ),
        (
            "transitions.cshtml",
            &[
                (
                    "user@example.com and // ordinary prose.",
                    "user@example.com",
                    None,
                ),
                ("@: One line: @name,", "name", Some(K::Variable)),
                ("@café / prefix@(name)suffix", "café", Some(K::Variable)),
                ("prefix@(name)suffix", "suffix", None),
            ],
        ),
        (
            "unicode.cshtml",
            &[
                ("@value.Näme /", "Näme", Some(K::Variable)),
                ("@value.café /", "café", Some(K::Variable)),
                ("@café</p>", "café", Some(K::Variable)),
            ],
        ),
    ];
    let malformed: &[(&str, &[ExpectedSpan])] = &[
        (
            "RecoveryAttribute.razor",
            &[
                (
                    "@bind-value:event=\"oninput\"",
                    "@bind-value:event",
                    Some(K::Attribute),
                ),
                ("@bind-value:event=\"oninput\"", "oninput", Some(K::String)),
            ],
        ),
        (
            "RecoveryDirective.razor",
            &[("@typeparam", "@typeparam", Some(K::Keyword))],
        ),
        (
            "RecoveryQuote.razor",
            &[("<div title=", "div", Some(K::Tag))],
        ),
        (
            "RecoveryTag.razor",
            &[("<div><span>", "span", Some(K::Tag))],
        ),
        (
            "recovery_block.cshtml",
            &[
                ("var count = 1", "var", Some(K::Keyword)),
                ("<p>@count</p>", "count", Some(K::Variable)),
            ],
        ),
        (
            "recovery_comment.cshtml",
            &[
                ("<p>Before</p>", "p", Some(K::Tag)),
                (
                    "@* Unfinished Razor comment",
                    "Unfinished Razor comment",
                    Some(K::Comment),
                ),
            ],
        ),
        (
            "recovery_expression.cshtml",
            &[("@(1 + 2", "1", Some(K::Number))],
        ),
        (
            "recovery_generics.cshtml",
            &[("private T Identity<T>", "Identity", Some(K::Function))],
        ),
        (
            "recovery_string.cshtml",
            &[("var text =", "var", Some(K::Keyword))],
        ),
        (
            "recovery_transition.cshtml",
            &[("<p>Before</p>", "p", Some(K::Tag))],
        ),
    ];
    let directory = root.join("templates/razor");
    let mut failures = Vec::new();
    for (name, spans, valid) in cases
        .iter()
        .map(|(name, spans)| (name, spans, true))
        .chain(malformed.iter().map(|(name, spans)| (name, spans, false)))
    {
        let sample = parse_sample(directory.join(name), &mut failures)
            .unwrap_or_else(|| panic!("cannot parse {name}: {failures:?}"));
        assert_eq!(sample.language, DiffSyntaxLanguage::Razor);
        let state = prepared_document_tree_state(sample.document).unwrap();
        if valid && state.tree.root_node().has_error() {
            failures.push(format!("{name}: unexpected ERROR or MISSING node"));
        }
        for line_ix in 0..sample.line_count() {
            let tokens =
                syntax_tokens_for_prepared_document_line(sample.document, line_ix).unwrap();
            let mut end = 0;
            for token in tokens {
                assert!(
                    token.range.start < token.range.end && token.range.start >= end,
                    "{name}: {token:?}"
                );
                assert!(
                    sample.line(line_ix).get(token.range.clone()).is_some(),
                    "{name}: {token:?}"
                );
                end = token.range.end;
            }
        }
        for &(context, fragment, kind) in *spans {
            if let Some(failure) = span_failure(&sample, context, fragment, kind) {
                failures.push(failure);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
    let registered = sample_files(&directory)
        .into_iter()
        .filter(|path| {
            matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("cshtml" | "razor")
            )
        })
        .count();
    assert_eq!(
        cases.len() + malformed.len(),
        registered,
        "every Razor fixture needs boundary assertions"
    );
}
