use std::fmt::Write as _;
use std::path::Path;

use super::*;

fn compare_texts(path: &str, old: &str, new: &str) -> String {
    render(compare(Input {
        path: Path::new(path),
        old,
        new,
    }))
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
fn formatting_only_change_is_unchanged() {
    assert_eq!(
        compare_texts("lib.rs", "fn a() {}\n", "fn a()  {\n}\n"),
        "Unchanged"
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
    );
    assert!(matches!(
        result.file_format,
        FileFormat::TextFallback {
            reason: TextFallbackReason::GraphLimit
        }
    ));
}
