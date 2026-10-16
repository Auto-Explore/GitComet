use super::*;

// ---- heuristic tokenizer tests ----

#[test]
fn heuristic_ruby_hash_comment() {
    let tokens = syntax_tokens_for_line_heuristic("x = 1 # comment", DiffSyntaxLanguage::Ruby);
    let comment = tokens.iter().find(|t| t.kind == SyntaxTokenKind::Comment);
    assert!(comment.is_some(), "Ruby '#' should be detected as comment");
    let c = comment.unwrap();
    assert!(c.range.start <= 6, "comment should start at or before '#'");
    assert_eq!(
        c.range.end,
        "x = 1 # comment".len(),
        "comment should extend to end of line"
    );
}

#[test]
fn heuristic_python_hash_comment() {
    let tokens = syntax_tokens_for_line_heuristic("x = 1 # note", DiffSyntaxLanguage::Python);
    assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment));
}

#[test]
fn heuristic_vb_rem_comment() {
    let tokens =
        syntax_tokens_for_line_heuristic("REM this is a comment", DiffSyntaxLanguage::VisualBasic);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
    assert_eq!(tokens[0].range, 0..21);
}

#[test]
fn heuristic_vb_apostrophe_comment() {
    let tokens =
        syntax_tokens_for_line_heuristic("' this is a comment", DiffSyntaxLanguage::VisualBasic);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
}

#[test]
fn heuristic_vb_keywords_are_case_insensitive() {
    let tokens =
        syntax_tokens_for_line_heuristic("dim value As Integer", DiffSyntaxLanguage::VisualBasic);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "Visual Basic keywords should be highlighted regardless of case"
    );
}

#[test]
fn heuristic_rust_line_comment_and_string() {
    let tokens =
        syntax_tokens_for_line_heuristic(r#"let s = "hello"; // done"#, DiffSyntaxLanguage::Rust);
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
    assert!(
        kinds.contains(&SyntaxTokenKind::Keyword),
        "should find 'let'"
    );
    assert!(
        kinds.contains(&SyntaxTokenKind::String),
        "should find string"
    );
    assert!(
        kinds.contains(&SyntaxTokenKind::Comment),
        "should find comment"
    );
}

#[test]
fn heuristic_rust_block_comment_continues_scanning() {
    let tokens =
        syntax_tokens_for_line_heuristic("/* note */ let value = 1", DiffSyntaxLanguage::Rust);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "should find block comment"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "should keep scanning after block comment"
    );
}

#[test]
fn heuristic_fsharp_block_comment_continues_scanning() {
    let tokens =
        syntax_tokens_for_line_heuristic("(* note *) let value = 1", DiffSyntaxLanguage::FSharp);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "should find F# block comment"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "should keep scanning after F# block comment"
    );
}

#[test]
fn heuristic_hcl_hash_comment() {
    let tokens = syntax_tokens_for_line_heuristic("value = 1 # note", DiffSyntaxLanguage::Hcl);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "HCL '#' should be detected as comment"
    );
}

#[test]
fn heuristic_powershell_hash_comment() {
    let tokens =
        syntax_tokens_for_line_heuristic("$value = 1 # note", DiffSyntaxLanguage::PowerShell);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "PowerShell '#' should be detected as comment"
    );
}

#[test]
fn heuristic_html_comment() {
    let tokens =
        syntax_tokens_for_line_heuristic("<!-- comment --> <div>", DiffSyntaxLanguage::Html);
    assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment));
}

#[test]
fn heuristic_lua_block_comment() {
    let tokens = syntax_tokens_for_line_heuristic("--[[ block ]] rest", DiffSyntaxLanguage::Lua);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
    // Should cover "--[[" through "]]"
    assert_eq!(tokens[0].range.end, 13);
}

#[test]
fn heuristic_css_selector() {
    let tokens =
        syntax_tokens_for_line_heuristic(".my-class { color: red; }", DiffSyntaxLanguage::Css);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Type),
        "CSS class selector should be Type"
    );
}

#[test]
fn heuristic_number_literal() {
    let tokens = syntax_tokens_for_line_heuristic("x = 42", DiffSyntaxLanguage::Python);
    assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number));
}
