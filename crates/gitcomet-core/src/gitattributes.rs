//! Writing rules into `.gitattributes`.

use std::path::Path;

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

/// A pattern token for a rule line. Git reads a pattern with whitespace or a
/// quote only in C quotes, and inside them every backslash — the glob escapes
/// included — is itself escaped.
fn pattern_token(pattern: String) -> String {
    if !pattern.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return pattern;
    }
    let mut out = String::with_capacity(pattern.len() + 4);
    out.push('"');
    for ch in pattern.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// A pattern matching exactly `repo_path` (repo-relative), for the root
/// `.gitattributes`.
pub fn pattern_for_path(repo_path: &Path) -> String {
    let path = repo_path.to_string_lossy().replace('\\', "/");
    pattern_token(format!("/{}", escape_glob(path.trim_start_matches('/'))))
}

/// A pattern matching every file with `repo_path`'s extension, anywhere.
pub fn pattern_for_extension(repo_path: &Path) -> Option<String> {
    let extension = repo_path.extension()?.to_str()?;
    (!extension.is_empty()).then(|| pattern_token(format!("*.{}", escape_glob(extension))))
}

/// `existing` with `line` appended on a line of its own, or `None` when the
/// exact line is already there. Works on bytes so a file in any encoding is
/// never re-encoded.
pub fn append_rule(existing: &[u8], line: &str) -> Option<Vec<u8>> {
    let already = existing.split(|byte| *byte == b'\n').any(|existing_line| {
        existing_line.strip_suffix(b"\r").unwrap_or(existing_line) == line.as_bytes()
    });
    if already {
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
        assert_eq!(pattern_for_path(Path::new("x[1]*.txt")), "/x\\[1\\]\\*.txt");
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
}
