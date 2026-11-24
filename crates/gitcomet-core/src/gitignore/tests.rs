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
fn append_adds_the_missing_newline_only_for_patterns_that_survive_filtering() {
    // The already-present patterns are dropped before anything is written,
    // so the decision to terminate the last existing line has to be made
    // against what is actually left to append.
    assert_eq!(
        append_patterns("/target", &patterns(&["/target", "/build/"])).as_deref(),
        Some("/target\n/build/\n"),
        "the surviving pattern still needs the missing newline first"
    );
    assert_eq!(
        append_patterns("/target", &patterns(&["/target"])),
        None,
        "nothing survives, so the file must be left exactly as it was"
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
