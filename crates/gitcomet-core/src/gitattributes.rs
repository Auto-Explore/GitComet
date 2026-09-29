//! Writing rules into `.gitattributes`.

use std::{borrow::Cow, path::Path};

pub const GITATTRIBUTES_FILE_NAME: &str = ".gitattributes";
pub const NOTHING_TO_ADD: &str = "nothing to add";

/// Escape glob and line syntax so `text` matches only itself.
fn escape_glob(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for (ix, ch) in text.chars().enumerate() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') || (ix == 0 && matches!(ch, '!' | '#')) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Git's C quoting also escapes the backslashes introduced by glob escaping.
fn pattern_token(pattern: String) -> String {
    match gix_quote::ansi_c::quote(pattern.as_bytes().into()) {
        // Spaces alone do not require C escaping, but they separate tokens in
        // an attributes rule, so the pattern still needs quotes.
        Cow::Borrowed(_) if pattern.contains(' ') => format!("\"{pattern}\""),
        quoted => quoted.to_string(),
    }
}

/// A pattern matching exactly `repo_path` (repo-relative), for the root
/// `.gitattributes`.
pub fn pattern_for_path(repo_path: &Path) -> String {
    let path = repo_path.to_string_lossy();
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    pattern_token(format!("/{}", escape_glob(path.trim_start_matches('/'))))
}

/// A pattern matching every file with `repo_path`'s extension, anywhere.
pub fn pattern_for_extension(repo_path: &Path) -> Option<String> {
    let extension = repo_path.extension()?.to_str()?;
    (!extension.is_empty()).then(|| pattern_token(format!("*.{}", escape_glob(extension))))
}

/// `existing` with `line` appended on a line of its own, or `None` when the
/// exact line is the last rule. Earlier copies may have been overridden by
/// later patterns. Works on bytes so the file is never re-encoded.
pub fn append_rule(existing: &[u8], line: &str) -> Option<Vec<u8>> {
    let last_rule = existing
        .rsplit(|byte| *byte == b'\n')
        .map(<[u8]>::trim_ascii)
        .find(|line| !line.is_empty() && !line.starts_with(b"#"));
    if last_rule == Some(line.as_bytes()) {
        return None;
    }
    let crlf = existing.windows(2).any(|pair| pair == b"\r\n");
    let newline: &[u8] = if crlf { b"\r\n" } else { b"\n" };
    let mut out = Vec::with_capacity(existing.len() + line.len() + 2);
    out.extend_from_slice(existing);
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        out.extend_from_slice(newline);
    }
    out.extend_from_slice(line.as_bytes());
    out.extend_from_slice(newline);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_patterns_are_anchored_and_escaped() {
        assert_eq!(pattern_for_path(Path::new("src/a.txt")), "/src/a.txt");
        assert_eq!(
            pattern_for_path(Path::new("x[1]*.txt")),
            r#""/x\\[1\\]\\*.txt""#
        );
        assert_eq!(
            pattern_for_path(Path::new("dir/my file.txt")),
            "\"/dir/my file.txt\""
        );
        // Glob escapes inside C quotes are doubled.
        assert_eq!(
            pattern_for_path(Path::new("a b[1].txt")),
            "\"/a b\\\\[1\\\\].txt\""
        );
    }

    #[test]
    fn backslashes_follow_platform_path_semantics() {
        let pattern = pattern_for_path(Path::new(r"dir\name.txt"));
        #[cfg(windows)]
        assert_eq!(pattern, "/dir/name.txt");
        #[cfg(not(windows))]
        assert_eq!(pattern, r#""/dir\\\\name.txt""#);
    }

    #[test]
    fn pattern_tokens_keep_line_breaks_inside_one_rule() {
        assert_eq!(
            pattern_for_path(Path::new("a\nx.txt encoding=KOI8-R\r\nz.txt")),
            "\"/a\\nx.txt encoding=KOI8-R\\r\\nz.txt\""
        );
        assert_eq!(
            pattern_for_extension(Path::new("a.ext\nvictim encoding=KOI8-R\rz")).as_deref(),
            Some("\"*.ext\\nvictim encoding=KOI8-R\\rz\"")
        );
    }

    #[test]
    fn extension_patterns() {
        assert_eq!(
            pattern_for_extension(Path::new("x/y.txt")).as_deref(),
            Some("*.txt")
        );
        assert_eq!(pattern_for_extension(Path::new("Makefile")), None);
    }

    #[test]
    fn append_keeps_bytes_and_line_style() {
        assert_eq!(
            append_rule(b"", "*.txt encoding=cp1252").unwrap(),
            b"*.txt encoding=cp1252\n"
        );
        assert_eq!(
            append_rule(b"# caf\xe9\r\n*.c text", "/a eol=lf").unwrap(),
            b"# caf\xe9\r\n*.c text\r\n/a eol=lf\r\n"
        );
        assert_eq!(append_rule(b"/a eol=lf\n", "/a eol=lf"), None);
    }

    #[test]
    fn an_earlier_duplicate_does_not_override_a_later_rule() {
        let rule = "/menu.txt encoding=windows-1252";
        for later in [
            "/menu.txt encoding=koi8-r",
            "*.txt encoding=koi8-r",
            "* -encoding",
        ] {
            let existing = format!("{rule}\n{later}\n");
            let updated = append_rule(existing.as_bytes(), rule).unwrap();
            assert_eq!(updated, format!("{existing}{rule}\n").as_bytes());
            assert_eq!(append_rule(&updated, rule), None);
        }
        assert_eq!(
            append_rule(format!("{rule}\r\n\r\n # comment\r\n").as_bytes(), rule),
            None
        );
    }
}
