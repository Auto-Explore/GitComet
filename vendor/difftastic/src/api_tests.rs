use std::fmt::Write as _;
use std::path::Path;

use super::*;
use crate::options::never_stop;

const NESTING_LIMIT: usize = 10_000;

fn compare_texts(path: &str, old: &str, new: &str) -> String {
    let input = Input {
        path: Path::new(path),
        old,
        new,
    };
    render(compare(input, NESTING_LIMIT, &never_stop).expect("never asked to stop"))
}

fn render(comparison: Comparison) -> String {
    let Comparison::Changed(changes) = comparison else {
        return format!("{comparison:?}");
    };
    let mut out = format!("{} |", changes.language);
    for row in &changes.aligned_lines {
        let (old, new) = match row {
            AlignedLine::Both { old, new } => (old.to_string(), new.to_string()),
            AlignedLine::Old(old) => (old.to_string(), "-".to_owned()),
            AlignedLine::New(new) => ("-".to_owned(), new.to_string()),
        };
        write!(out, " {old}:{new}").unwrap();
    }
    for chunk in &changes.chunks {
        out.push_str(" | ");
        let lines: Vec<String> = chunk
            .iter()
            .map(|line| match line {
                ChunkLine::Both { old, new } => {
                    format!("{} {}", render_side("L", old), render_side("R", new))
                }
                ChunkLine::Old(old) => render_side("L", old),
                ChunkLine::New(new) => render_side("R", new),
            })
            .collect();
        out.push_str(&lines.join("; "));
    }
    out
}

fn render_side(name: &str, side: &LineRanges) -> String {
    let ranges: Vec<String> = side
        .ranges
        .iter()
        .map(|range| format!("{}-{}", range.start, range.end))
        .collect();
    format!("{name}{}[{}]", side.line, ranges.join(","))
}

#[test]
fn javascript_change_marks_the_wrapping_call() {
    assert_eq!(
        compare_texts(
            "example.js",
            "let result = value;\n",
            "let result = wrap(value);\n"
        ),
        "JavaScript | 0:0 | L0[] R0[13-17,17-18,23-24]"
    );
}

#[test]
fn rust_change_aligns_lines_and_marks_tokens() {
    assert_eq!(
        compare_texts(
            "lib.rs",
            "fn total(a: u32) -> u32 {\n    a + 1\n}\n",
            "fn total(a: u32, b: u32) -> u32 {\n    a + b\n}\n"
        ),
        "Rust | 0:0 1:1 2:2 | L0[] R0[15-16,17-18,18-19,20-23]; L1[8-9] R1[8-9]"
    );
}

#[test]
fn missing_final_newline_still_parses_line_oriented_grammars() {
    assert_eq!(
        compare_texts("Makefile", "all:\n\techo a", "all:\n\techo b"),
        "Make | 0:0 1:1 | L1[1-7] R1[1-7]"
    );
}

#[test]
fn kotlin_change_marks_the_parameter_and_interpolation() {
    assert_eq!(
        compare_texts(
            "Greeter.kt",
            "fun greet() = println(\"hi\")\n",
            "fun greet(name: String) = println(\"hi $name\")\n"
        ),
        "Kotlin | 0:0 | L0[22-23,23-25,25-26] \
         R0[10-14,14-15,16-22,34-35,35-37,38-39,39-43,43-44]"
    );
}

#[test]
fn perl_change_marks_the_string_and_new_argument() {
    assert_eq!(
        compare_texts(
            "script.pl",
            "my $x = \"a\";\nprint $x;\n",
            "my $x = \"b\";\nprint $x, \"\\n\";\n"
        ),
        "Perl | 0:0 1:1 | L0[8-11] R0[8-11]; L1[] R1[8-9,10-14]"
    );
}

#[test]
fn perl_substitution_and_heredoc_mark_words_on_their_own_lines() {
    assert_eq!(
        compare_texts(
            "script.pl",
            "$f =~ s/foo/bar/g;\nprint <<END;\nhello\nEND\n",
            "$f =~ s/foo/abc/g;\nprint <<END;\nhello world\nEND\n"
        ),
        "Perl | 0:0 1:1 2:2 3:3 | L0[6-7,7-8,8-11,11-12,12-15,15-16,16-17] \
         R0[6-7,7-8,8-11,11-12,12-15,15-16,16-17]; L1[12-12] R1[12-12]; \
         L2[0-5,5-5] R2[0-5,6-11,11-11]; L3[0-3] R3[0-3]"
    );
}

#[test]
fn replaced_heredoc_leaves_the_opener_line_unmarked() {
    assert_eq!(
        compare_texts(
            "script.pl",
            "print <<END;\nx\nEND\n",
            "print <<END;\nq r s t u\nEND\n"
        ),
        "Perl | 0:0 1:1 2:2 | L1[0-1] R1[0-9]; L2[0-3] R2[0-3]"
    );
}

#[test]
fn latex_change_marks_the_edited_words() {
    assert_eq!(
        compare_texts(
            "paper.tex",
            "\\section{Intro}\nHello world.\n",
            "\\section{Introduction}\nHello there, world.\n"
        ),
        "LaTeX | 0:0 1:1 | L0[9-14] R0[9-21]; L1[] R1[6-11,11-12]"
    );
}

#[test]
fn removed_last_line_is_an_old_only_row() {
    assert_eq!(
        compare_texts("a.py", "x = 1\ny = 2\n", "x = 1\n"),
        "Python | 0:0 1:- | L1[0-1,2-3,4-5]"
    );
}

#[test]
fn formatting_only_change_is_unchanged() {
    assert_eq!(
        compare_texts("lib.rs", "fn a() {}\n", "fn a()  {\n}\n"),
        "Unchanged"
    );
}

#[test]
fn cls_files_are_apex_classes() {
    assert_eq!(
        compare_texts("Invoice.cls", "", "public class Invoice {}\n"),
        "Created { language: \"Apex\" }"
    );
}

#[test]
fn created_and_deleted_files_report_their_language() {
    assert_eq!(
        compare_texts("main.py", "", "x = 1\n"),
        "Created { language: \"Python\" }"
    );
    assert_eq!(
        compare_texts("main.py", "x = 1\n", ""),
        "Deleted { language: \"Python\" }"
    );
}

#[test]
fn deleted_file_language_comes_from_the_old_text() {
    assert_eq!(
        compare_texts("script", "#!/usr/bin/env python\nx = 1\n", ""),
        "Deleted { language: \"Python\" }"
    );
}

#[test]
fn unsupported_and_dropped_languages_fall_back() {
    for path in ["notes.txt", "solver.f90", "counter.sv"] {
        assert_eq!(
            compare_texts(path, "x = 1\n", "x = 2\n"),
            "Fallback(UnsupportedLanguage)",
            "{path}"
        );
    }
}

#[test]
fn parse_errors_fall_back() {
    assert_eq!(
        compare_texts("lib.rs", "fn a() {}\n", "fn a( {\n"),
        "Fallback(ParseErrors)"
    );
}

#[test]
fn sources_over_the_byte_limit_fall_back() {
    let large = format!("let a = \"{}\";\n", "x".repeat(BYTE_LIMIT));
    assert_eq!(
        compare_texts("example.js", "let a = 1;\n", &large),
        "Fallback(ByteLimit)"
    );
}

#[test]
fn graph_limit_is_a_typed_fallback() {
    let result = crate::diff_file_content(
        Path::new("example.js"),
        "let a = 1;\n",
        "let a = 2;\n",
        &DiffOptions {
            graph_limit: 1,
            ..DiffOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        result.file_format,
        FileFormat::TextFallback {
            reason: TextFallbackReason::GraphLimit
        }
    ));
}

#[test]
fn a_stop_request_before_any_work_interrupts() {
    let input = Input {
        path: Path::new("notes.txt"),
        old: "a\n",
        new: "b\n",
    };
    assert_eq!(compare(input, NESTING_LIMIT, &|| true), Err(Interrupted));
}

#[test]
fn a_stop_request_while_parsing_interrupts() {
    use crate::parse::guess_language::Language;
    use crate::parse::tree_sitter_parser::{from_language, to_tree_with_limit, TreeStop};

    let source = "let a = [1, 2, 3];\n".repeat(20_000);
    let options = DiffOptions {
        should_stop: &|| true,
        ..DiffOptions::default()
    };
    let result = to_tree_with_limit(
        &options,
        from_language(Language::JavaScript),
        &source,
        &source,
    );
    assert!(matches!(result, Err(TreeStop::Interrupted)));
}

#[test]
fn a_stop_request_during_the_graph_search_interrupts() {
    use std::cell::Cell;

    use typed_arena::Arena;

    use crate::diff::changes::ChangeMap;
    use crate::diff::shortest_path::{mark_syntax, SearchStop};
    use crate::options::DEFAULT_GRAPH_LIMIT;
    use crate::parse::guess_language::Language;
    use crate::parse::syntax::init_all_info;
    use crate::parse::tree_sitter_parser::{from_language, parse};

    let items = |odd: &str| {
        (0..200)
            .map(|item| {
                if item % 2 == 0 {
                    item.to_string()
                } else {
                    odd.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let old = format!("let a = [{}];\n", items("x"));
    let new = format!("let a = [{}];\n", items("y"));
    let search = |should_stop: &dyn Fn() -> bool| {
        let arena = Arena::new();
        let config = from_language(Language::JavaScript);
        let lhs = parse(&arena, &old, config, false);
        let rhs = parse(&arena, &new, config, false);
        init_all_info(&lhs, &rhs);
        let mut change_map = ChangeMap::default();
        mark_syntax(
            lhs.first().copied(),
            rhs.first().copied(),
            &mut change_map,
            DEFAULT_GRAPH_LIMIT,
            should_stop,
        )
    };
    let checks = Cell::new(0);
    let count = || {
        checks.set(checks.get() + 1);
        false
    };
    assert!(search(&count).is_ok());
    assert!(
        checks.get() > 1,
        "the search must run past one check interval"
    );
    let calls = Cell::new(0);
    let stop_after_first_check = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    assert!(matches!(
        search(&stop_after_first_check),
        Err(SearchStop::Interrupted)
    ));
}

#[test]
fn a_stop_request_while_highlighting_interrupts() {
    use crate::parse::guess_language::Language;
    use crate::parse::tree_sitter_parser::{from_language, to_tree, tree_highlights};

    let source = "let a = [1, \"b\", c];\n".repeat(20_000);
    let config = from_language(Language::JavaScript);
    let tree = to_tree(&source, config);
    assert!(matches!(
        tree_highlights(&tree, &source, config, &|| true),
        Err(Interrupted)
    ));
}

fn nested_json(depth: usize, value: &str) -> String {
    format!("{}{value}{}\n", "[".repeat(depth), "]".repeat(depth))
}

#[test]
fn nesting_at_the_limit_compares_and_one_deeper_falls_back() {
    let old = nested_json(9, "1");
    let new = nested_json(9, "2");
    let input = Input {
        path: Path::new("a.json"),
        old: &old,
        new: &new,
    };
    assert!(matches!(
        compare(input, 10, &never_stop),
        Ok(Comparison::Changed(_))
    ));
    assert_eq!(
        compare(input, 9, &never_stop),
        Ok(Comparison::Fallback(Fallback::NestingLimit))
    );
}

#[test]
fn an_embedded_tree_gets_the_depth_its_host_leaves() {
    use crate::parse::guess_language::Language;
    use crate::parse::tree_sitter_parser::{from_language, parse_subtrees, to_tree};

    // The style text sits 12 levels deep in the HTML tree; the CSS tree alone is 42 deep.
    let css = format!("{}color: red;{}", "a {".repeat(20), "}".repeat(20));
    let html = format!(
        "{}<style>{css}</style>{}\n",
        "<div>".repeat(10),
        "</div>".repeat(10)
    );
    let config = from_language(Language::Html);
    let tree = to_tree(&html, config);
    let subtrees = |nesting_limit| {
        let options = DiffOptions {
            nesting_limit,
            ..DiffOptions::default()
        };
        parse_subtrees(&html, config, &tree, &options)
            .unwrap()
            .len()
    };
    assert_eq!(subtrees(60), 1);
    assert_eq!(subtrees(50), 0);
}

#[test]
fn a_stop_request_cuts_a_query_short() {
    use crate::parse::guess_language::Language;
    use crate::parse::tree_sitter_parser::{for_each_match_unless_stopped, from_language, to_tree};

    let source = "let a = [1, \"b\", c];\n".repeat(20_000);
    let tree = to_tree(&source, from_language(Language::JavaScript));
    let query = tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        "(identifier) @name",
    )
    .unwrap();
    let count = |should_stop: &dyn Fn() -> bool| {
        let mut matches = 0;
        let result =
            for_each_match_unless_stopped(&query, &tree, &source, should_stop, |_| matches += 1);
        (matches, result)
    };
    let (all, finished) = count(&never_stop);
    let (before_stop, stopped) = count(&|| true);
    assert_eq!(finished, Ok(()));
    assert_eq!(stopped, Err(Interrupted));
    assert!(before_stop < all / 2, "{before_stop} of {all} matches ran");
}
