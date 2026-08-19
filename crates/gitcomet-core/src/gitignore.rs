//! Turning a repo-relative worktree path into a `.gitignore` pattern.
//!
//! The escaping here is the whole reason this lives in `gitcomet-core` rather
//! than next to the dialog that uses it: a pattern that looks right and matches
//! nothing is indistinguishable from a working one until the user notices the
//! file is still in the status list. `build/out[1].log` written verbatim is a
//! character class, not a filename.

use rustc_hash::FxHashSet;
use std::path::{Component, Path};

/// The ignore file this module writes to, relative to the repository root.
pub const FILE_NAME: &str = ".gitignore";

/// Marker the append command puts in its command output when every pattern was
/// already present, so nothing was written.
///
/// A shared constant rather than a prose match at each end: the summary shown to
/// the user hinges on it, and "Added X to .gitignore" for a run that touched
/// nothing is a lie the user only catches by opening the file.
pub const NOTHING_TO_ADD: &str = "Already present; nothing to add";

/// Which slice of a path a generated pattern covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GitignoreScope {
    /// Just this file, anchored to the repository root.
    File,
    /// The file's immediate parent directory, anchored to the repository root.
    Folder,
    /// Every file sharing this file's extension, at any depth.
    Extension,
}

/// Build the `.gitignore` line for `relative_path` under `scope`.
///
/// `None` when the scope does not apply (a root-level file has no folder to
/// ignore, an extensionless file has no extension to ignore) or the path is not
/// something we can safely write a pattern for.
pub fn pattern_for(relative_path: &Path, scope: GitignoreScope) -> Option<String> {
    let segments = relative_segments(relative_path)?;

    match scope {
        GitignoreScope::File => Some(escape_trailing_spaces(&anchored(&segments))),
        GitignoreScope::Folder => {
            // `segments.len() == 1` is a file sitting directly in the root:
            // its "folder" is the whole repository, which is never what the
            // user means.
            let parent = segments.get(..segments.len().checked_sub(1)?)?;
            if parent.is_empty() {
                return None;
            }
            Some(format!("{}/", anchored(parent)))
        }
        GitignoreScope::Extension => {
            let extension = extension_of(segments.last()?)?;
            Some(escape_trailing_spaces(&format!(
                "*.{}",
                escape_literal(extension)
            )))
        }
    }
}

/// The patterns offerable for a whole status selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitignoreSuggestions {
    /// One anchored pattern per selected path, in selection order.
    files: Vec<String>,
    /// `/dir/`, only when every path shares one immediate parent.
    folder: Option<String>,
    /// `*.ext`, only when every path shares one extension.
    extension: Option<String>,
}

/// Build the suggestions for a status selection.
///
/// `None` when the selection is empty or any member cannot be expressed as a
/// pattern — one unusable path disqualifies the whole action rather than
/// quietly ignoring a subset of what the user selected.
pub fn suggestions_for_paths(paths: &[std::path::PathBuf]) -> Option<GitignoreSuggestions> {
    if paths.is_empty() {
        return None;
    }

    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        files.push(pattern_for(path, GitignoreScope::File)?);
    }

    // Folder and extension are offered only when every path agrees. Deriving
    // either from a mixed selection would ignore files the user never pointed
    // at — `/build/` picked from one of three scattered files would sweep up
    // that whole directory.
    let folder = all_equal(paths.iter().map(|p| pattern_for(p, GitignoreScope::Folder)));
    let extension = all_equal(
        paths
            .iter()
            .map(|p| pattern_for(p, GitignoreScope::Extension)),
    );

    Some(GitignoreSuggestions {
        files,
        folder,
        extension,
    })
}

/// `Some(value)` when every item is the same `Some(value)`, else `None`.
fn all_equal(mut items: impl Iterator<Item = Option<String>>) -> Option<String> {
    let first = items.next()??;
    items
        .all(|item| item.as_deref() == Some(first.as_str()))
        .then_some(first)
}

impl GitignoreSuggestions {
    /// The `.gitignore` lines this scope would write, in order.
    pub fn lines_for(&self, scope: GitignoreScope) -> Vec<String> {
        match scope {
            GitignoreScope::File => self.files.clone(),
            GitignoreScope::Folder => self.folder.clone().into_iter().collect(),
            GitignoreScope::Extension => self.extension.clone().into_iter().collect(),
        }
    }

    /// The scopes worth offering, in menu order. Always contains `File`.
    pub fn applicable_scopes(&self) -> Vec<GitignoreScope> {
        let mut scopes = vec![GitignoreScope::File];
        if self.folder.is_some() {
            scopes.push(GitignoreScope::Folder);
        }
        if self.extension.is_some() {
            scopes.push(GitignoreScope::Extension);
        }
        scopes
    }
}

/// Append `patterns` to the contents of a `.gitignore`.
///
/// Returns `None` when there is nothing to do — every pattern is already in the
/// file — so the caller can skip the write and tell the user rather than
/// touching the file's mtime and tripping the filesystem watcher for nothing.
///
/// Existing lines are never rewritten. Comparison uses [`trim_trailing_spaces`],
/// not a plain `trim`: leading whitespace *is* significant in a pattern, so
/// `  /build/` is genuinely a different line from `/build/` and must not
/// suppress it — and a plain `trim_end` would eat the escaped trailing space
/// that [`pattern_for`] just went to the trouble of adding, leaving a dangling
/// backslash that swallows the newline.
pub fn append_patterns(existing: &str, patterns: &[String]) -> Option<String> {
    let terminator = crate::text_utils::detect_line_ending_from_texts(
        [existing],
        crate::text_utils::LineEndingDetectionMode::DominantCrlfVsLf,
    );

    // Indexed rather than rescanned per pattern: a 500-file selection against a
    // 2000-line `.gitignore` is a plausible bulk case, and the naive form walks
    // both strings a million times on the store's worker thread.
    let existing_line_count = existing.lines().count();
    let mut present = FxHashSet::with_capacity_and_hasher(existing_line_count, Default::default());
    present.extend(existing.lines().map(trim_trailing_spaces));

    let mut seen = FxHashSet::with_capacity_and_hasher(patterns.len(), Default::default());
    let capacity = patterns.iter().fold(existing.len(), |capacity, pattern| {
        capacity.saturating_add(pattern.len().saturating_add(terminator.len()))
    });
    let mut out = String::with_capacity(capacity);
    out.push_str(existing);
    let mut appended = false;
    for pattern in patterns {
        let pattern = trim_trailing_spaces(pattern);
        if pattern.is_empty() {
            continue;
        }
        // `seen` dedupes within `patterns` itself: two selected files in the
        // same folder both produce `/build/`.
        if !present.contains(pattern) && seen.insert(pattern) {
            if !appended && !out.is_empty() && !out.ends_with('\n') {
                out.push_str(terminator);
            }
            out.push_str(pattern);
            out.push_str(terminator);
            appended = true;
        }
    }
    if !appended {
        return None;
    }
    Some(out)
}

/// Trim trailing spaces from a `.gitignore` line the way git itself does.
///
/// Git's `trim_trailing_spaces` walks the line and treats a backslash as
/// consuming the character after it, so an escaped space is kept *and* ends the
/// run being considered for trimming. A plain `str::trim_end` does not know
/// that, and using one on a pattern would silently unescape it.
pub fn trim_trailing_spaces(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut trailing_run: Option<usize> = None;
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b' ' => {
                trailing_run.get_or_insert(ix);
                ix += 1;
            }
            // Skip the escaped character with the backslash. A lone trailing
            // backslash walks past the end, which is fine: the run is already
            // cleared, so nothing is trimmed — git bails out there too.
            b'\\' => {
                trailing_run = None;
                ix += 2;
            }
            _ => {
                trailing_run = None;
                ix += 1;
            }
        }
    }
    // Safe to slice: the index always points at an ASCII space, which is a char
    // boundary even when the line contains multi-byte characters.
    match trailing_run {
        Some(ix) => &line[..ix],
        None => line,
    }
}

/// Split a repo-relative path into UTF-8 segments.
///
/// Anything that is not a plain relative path — absolute, a `..` escape, a
/// Windows drive prefix — is rejected rather than sanitized: those never come
/// out of a status listing, so seeing one means the caller is confused and a
/// silently-corrected pattern would hide that.
///
/// A segment containing a line break is rejected for a sharper reason:
/// `.gitignore` is line-oriented and git has no escape for a newline inside a
/// pattern, so a file legitimately named `"a\nb.log"` cannot be expressed at
/// all. Emitting it anyway would split into two unrelated patterns — `/a`, which
/// ignores an entire unrelated root-level entry, and `b.log`, which ignores that
/// name at every depth. Refusing to offer the action is the only safe answer.
fn relative_segments(relative_path: &Path) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    for component in relative_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part.to_str()?;
                if part.contains(['\n', '\r']) {
                    return None;
                }
                segments.push(part);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!segments.is_empty()).then_some(segments)
}

/// Join segments into a pattern anchored at the `.gitignore`'s own directory.
///
/// The leading `/` does double duty: it pins the pattern to the repository root
/// instead of matching the same name at any depth, and it occupies the first
/// column, so a path starting with `#` or `!` needs no further escaping — those
/// two are only special at the very start of a pattern.
fn anchored(segments: &[&str]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push('/');
        out.push_str(&escape_literal(segment));
    }
    out
}

/// Escape the glob metacharacters so git matches the segment literally.
fn escape_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Escape trailing spaces, which git strips from a pattern unless quoted.
///
/// Every space in the run needs its own backslash: git's `trim_trailing_spaces`
/// walks the line and only an escaped space resets the run, so `foo\  ` still
/// loses its last space.
fn escape_trailing_spaces(pattern: &str) -> String {
    let kept = pattern.trim_end_matches(' ');
    if kept.len() == pattern.len() {
        return pattern.to_string();
    }
    let trailing = pattern.len() - kept.len();
    let mut out = String::with_capacity(pattern.len() + trailing);
    out.push_str(kept);
    for _ in 0..trailing {
        out.push_str("\\ ");
    }
    out
}

/// The file name's extension, or `None` when there isn't a usable one.
fn extension_of(file_name: &str) -> Option<&str> {
    let extension = Path::new(file_name).extension()?.to_str()?;
    (!extension.is_empty()).then_some(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Option<String> {
        pattern_for(Path::new(path), GitignoreScope::File)
    }

    fn folder(path: &str) -> Option<String> {
        pattern_for(Path::new(path), GitignoreScope::Folder)
    }

    fn extension(path: &str) -> Option<String> {
        pattern_for(Path::new(path), GitignoreScope::Extension)
    }

    #[test]
    fn file_patterns_are_anchored_at_the_repository_root() {
        assert_eq!(file("build/out.log").as_deref(), Some("/build/out.log"));
        assert_eq!(file("out.log").as_deref(), Some("/out.log"));
        assert_eq!(
            file("a/b/c/d.txt").as_deref(),
            Some("/a/b/c/d.txt"),
            "every segment keeps its place; only the leading slash is added"
        );
    }

    #[test]
    fn glob_metacharacters_in_names_are_escaped() {
        assert_eq!(
            file("out[1].log").as_deref(),
            Some("/out\\[1\\].log"),
            "unescaped brackets would be read as a character class and match nothing"
        );
        assert_eq!(file("wild*card.txt").as_deref(), Some("/wild\\*card.txt"));
        assert_eq!(file("what?.txt").as_deref(), Some("/what\\?.txt"));
        // Unix-only: on Windows `\` is a path separator, so this same string
        // parses as two components and never reaches `escape_literal`.
        #[cfg(unix)]
        assert_eq!(
            file("back\\slash.txt").as_deref(),
            Some("/back\\\\slash.txt"),
            "a literal backslash in a Unix filename must not become an escape"
        );
        assert_eq!(
            file("dir[x]/out*.log").as_deref(),
            Some("/dir\\[x\\]/out\\*.log"),
            "escaping applies to every segment, not just the file name"
        );
    }

    #[test]
    fn a_leading_hash_or_bang_needs_no_escape_once_anchored() {
        // The leading `/` occupies column zero, so `#` and `!` are ordinary.
        assert_eq!(file("#notes.txt").as_deref(), Some("/#notes.txt"));
        assert_eq!(file("!important.txt").as_deref(), Some("/!important.txt"));
    }

    #[test]
    fn trailing_spaces_are_escaped_one_backslash_each() {
        assert_eq!(file("trailing .txt").as_deref(), Some("/trailing .txt"));
        assert_eq!(
            file("trailing ").as_deref(),
            Some("/trailing\\ "),
            "git strips an unquoted trailing space"
        );
        assert_eq!(
            file("two  ").as_deref(),
            Some("/two\\ \\ "),
            "one backslash per space: git only resets its run on an escaped space"
        );
    }

    #[test]
    fn folder_scope_uses_the_immediate_parent_and_ends_with_a_slash() {
        assert_eq!(folder("build/out.log").as_deref(), Some("/build/"));
        assert_eq!(
            folder("target/debug/deps/foo.rlib").as_deref(),
            Some("/target/debug/deps/"),
            "the immediate parent, not the top-level ancestor"
        );
        assert_eq!(
            folder("out.log"),
            None,
            "a root-level file's folder is the whole repository"
        );
    }

    #[test]
    fn extension_scope_is_unanchored_and_takes_the_last_extension() {
        assert_eq!(extension("build/out.log").as_deref(), Some("*.log"));
        assert_eq!(
            extension("archive.tar.gz").as_deref(),
            Some("*.gz"),
            "git has no notion of a compound extension"
        );
        assert_eq!(extension(".env"), None, "a dotfile has no extension");
        assert_eq!(extension("Makefile"), None);
        assert_eq!(
            extension("trailing."),
            None,
            "an empty extension is unusable"
        );
    }

    /// The scopes offered for a single path, through the same entry point the
    /// dialog uses.
    fn scopes_for(path: &str) -> Option<Vec<GitignoreScope>> {
        suggestions(&[path]).map(|s| s.applicable_scopes())
    }

    #[test]
    fn applicable_scopes_tracks_what_pattern_for_can_build() {
        assert_eq!(
            scopes_for("build/out.log"),
            Some(vec![
                GitignoreScope::File,
                GitignoreScope::Folder,
                GitignoreScope::Extension
            ])
        );
        assert_eq!(
            scopes_for("out.log"),
            Some(vec![GitignoreScope::File, GitignoreScope::Extension])
        );
        assert_eq!(scopes_for("Makefile"), Some(vec![GitignoreScope::File]));
        assert_eq!(
            scopes_for(""),
            None,
            "an empty path yields no action at all"
        );
    }

    #[test]
    fn paths_that_are_not_plainly_relative_are_rejected() {
        assert_eq!(file("../escape.txt"), None);
        assert_eq!(scopes_for("../escape.txt"), None);
        #[cfg(unix)]
        assert_eq!(file("/etc/passwd"), None);
    }

    #[test]
    fn paths_containing_a_line_break_yield_no_pattern() {
        // `.gitignore` is line-oriented and git has no escape for a newline, so
        // emitting one would silently become two patterns: `/a`, which ignores
        // an unrelated root entry, and `b.log`, which ignores that name
        // everywhere. Neither is anything the user asked for.
        assert_eq!(file("a\nb.log"), None);
        assert_eq!(file("dir\nname/file.log"), None);
        assert_eq!(file("a\rb.log"), None);
        assert_eq!(scopes_for("a\nb.log"), None);
        assert_eq!(
            suggestions_for_paths(&[std::path::PathBuf::from("a\nb.log")]),
            None
        );
    }

    #[test]
    fn current_dir_components_are_dropped() {
        assert_eq!(file("./build/out.log").as_deref(), Some("/build/out.log"));
    }

    fn patterns(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn suggestions(paths: &[&str]) -> Option<GitignoreSuggestions> {
        let paths: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();
        suggestions_for_paths(&paths)
    }

    #[test]
    fn suggestions_list_one_file_pattern_per_selected_path_in_order() {
        let suggestions = suggestions(&["b/second.log", "a/first.log"]).expect("suggestions");
        assert_eq!(
            suggestions.lines_for(GitignoreScope::File),
            vec!["/b/second.log", "/a/first.log"],
            "selection order is preserved; the dialog shows what the user picked"
        );
    }

    #[test]
    fn folder_is_offered_only_when_every_path_shares_one_parent() {
        let shared = suggestions(&["a/x.log", "a/y.log"]).expect("suggestions");
        assert_eq!(shared.lines_for(GitignoreScope::Folder), vec!["/a/"]);

        let mixed = suggestions(&["a/x.log", "b/y.log"]).expect("suggestions");
        assert!(
            mixed.lines_for(GitignoreScope::Folder).is_empty(),
            "one folder pattern for scattered files would ignore paths the user never picked"
        );
        assert_eq!(
            mixed.applicable_scopes(),
            vec![GitignoreScope::File, GitignoreScope::Extension]
        );
    }

    #[test]
    fn extension_is_offered_only_when_every_path_shares_one_extension() {
        let shared = suggestions(&["a/x.log", "b/y.log"]).expect("suggestions");
        assert_eq!(shared.lines_for(GitignoreScope::Extension), vec!["*.log"]);

        let mixed = suggestions(&["a/x.log", "a/y.txt"]).expect("suggestions");
        assert!(mixed.lines_for(GitignoreScope::Extension).is_empty());

        let extensionless = suggestions(&["a/x.log", "a/Makefile"]).expect("suggestions");
        assert!(
            extensionless
                .lines_for(GitignoreScope::Extension)
                .is_empty(),
            "a member with no extension disqualifies the scope"
        );
        assert_eq!(
            extensionless.applicable_scopes(),
            vec![GitignoreScope::File, GitignoreScope::Folder]
        );
    }

    #[test]
    fn a_selection_with_an_unusable_path_yields_nothing() {
        assert!(
            suggestions(&[]).is_none(),
            "an empty selection has no action"
        );
        assert!(
            suggestions(&["a/x.log", "../escape.txt"]).is_none(),
            "one unusable path disqualifies the whole selection rather than a silent subset"
        );
    }

    #[test]
    fn append_creates_content_from_an_empty_file() {
        assert_eq!(
            append_patterns("", &patterns(&["/build/out.log"])).as_deref(),
            Some("/build/out.log\n")
        );
    }

    #[test]
    fn append_adds_the_missing_newline_before_extending() {
        assert_eq!(
            append_patterns("/target\n*.tmp", &patterns(&["/build/"])).as_deref(),
            Some("/target\n*.tmp\n/build/\n"),
            "without this the new pattern would fuse onto the last line"
        );
        assert_eq!(
            append_patterns("/target\n", &patterns(&["/build/"])).as_deref(),
            Some("/target\n/build/\n"),
            "an already-terminated file must not gain a blank line"
        );
    }

    #[test]
    fn append_preserves_crlf_line_endings() {
        assert_eq!(
            append_patterns("/target\r\n", &patterns(&["/build/"])).as_deref(),
            Some("/target\r\n/build/\r\n")
        );
        assert_eq!(
            append_patterns("/target\r\n*.tmp", &patterns(&["/build/"])).as_deref(),
            Some("/target\r\n*.tmp\r\n/build/\r\n")
        );
    }

    #[test]
    fn append_is_idempotent() {
        assert_eq!(
            append_patterns("/target\n/build/\n", &patterns(&["/build/"])),
            None,
            "re-running the action must not pile up duplicates"
        );
        assert_eq!(
            append_patterns("/build/  \n", &patterns(&["/build/"])),
            None,
            "git strips trailing spaces, so this is the same line"
        );
        assert_eq!(
            append_patterns("  /build/\n", &patterns(&["/build/"])).as_deref(),
            Some("  /build/\n/build/\n"),
            "leading whitespace is significant, so the indented line is a different pattern"
        );
    }

    #[test]
    fn append_dedupes_within_the_batch_and_keeps_existing_lines() {
        assert_eq!(
            append_patterns(
                "# generated\n/target\n",
                &patterns(&["/build/", "/build/", "/dist/"])
            )
            .as_deref(),
            Some("# generated\n/target\n/build/\n/dist/\n"),
            "two selected files in one folder produce the same pattern once"
        );
    }

    #[test]
    fn append_keeps_the_escaped_trailing_space_pattern_for_produced() {
        // Regression: a plain `trim_end` here turned `/trailing\ ` into
        // `/trailing\`, a dangling escape that swallows the newline and joins
        // the pattern to whatever comes next. Real git then matched nothing.
        let pattern = file("trailing ").expect("expected a file pattern");
        assert_eq!(pattern, "/trailing\\ ");
        assert_eq!(
            append_patterns("", &[pattern]).as_deref(),
            Some("/trailing\\ \n")
        );
    }

    #[test]
    fn trim_trailing_spaces_follows_git_semantics() {
        assert_eq!(trim_trailing_spaces("/foo  "), "/foo");
        assert_eq!(
            trim_trailing_spaces("/foo\\ "),
            "/foo\\ ",
            "an escaped space is kept"
        );
        assert_eq!(
            trim_trailing_spaces("/foo\\  "),
            "/foo\\ ",
            "only the unescaped space at the end of the run is trimmed"
        );
        assert_eq!(trim_trailing_spaces("/foo"), "/foo");
        assert_eq!(trim_trailing_spaces("   "), "");
        assert_eq!(
            trim_trailing_spaces("/foo\\"),
            "/foo\\",
            "a lone trailing backslash is left alone, as git does"
        );
        assert_eq!(
            trim_trailing_spaces("/über  "),
            "/über",
            "multi-byte characters must not shift the slice off a boundary"
        );
    }

    #[test]
    fn append_skips_empty_patterns() {
        assert_eq!(append_patterns("/target\n", &patterns(&["", "   "])), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_yield_no_pattern() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"build/\xff\xfe.log"));
        assert_eq!(
            pattern_for(path, GitignoreScope::File),
            None,
            "a lossy round-trip would write a pattern that matches nothing"
        );
        assert_eq!(suggestions_for_paths(&[path.to_path_buf()]), None);
    }
}
