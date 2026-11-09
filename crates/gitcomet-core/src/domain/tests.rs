mod comparison;

use super::*;
use rustc_hash::FxHashSet;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[test]
fn working_tree_targets_name_their_worktree() {
    let main = DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged);
    let linked = main.clone().in_worktree("/repos/feature/./".into());
    assert_eq!(
        linked.worktree(),
        Some(std::path::Path::new("/repos/feature"))
    );
    assert_eq!(main.worktree(), None);
    assert_ne!(main, linked, "the same file in another worktree differs");
    assert_eq!(
        linked,
        main.clone().in_worktree("/repos/feature".into()),
        "spellings of one path agree"
    );
    let commit = DiffTarget::commit(CommitId("abc".into()), "a.rs".into());
    assert_eq!(commit.clone().in_worktree("/repos/feature".into()), commit);
}

#[test]
fn empty_tree_uses_the_complete_sibling_object_format() {
    for (len, expected) in [(40, EMPTY_TREE_ID_SHA1), (64, EMPTY_TREE_ID_SHA256)] {
        assert_eq!(
            empty_tree_id_like(&CommitId("a".repeat(len).into()))
                .unwrap()
                .as_ref(),
            expected
        );
        assert!(is_empty_tree_id(expected));
    }
    for invalid in ["main", "abc1234", "", &"x".repeat(40)] {
        assert!(empty_tree_id_like(&CommitId(invalid.into())).is_none());
        assert!(!is_empty_tree_id(invalid));
    }
}

#[test]
fn review_truncated_hunk_context_does_not_change_side_encoding() {
    let patch = b"@@ -1 +1 @@ function \xc3\n-\xc3\xa4\n+\xc3\xb6\n";
    assert_eq!(DiffSectionFormats::sniff(patch), DiffSectionFormats::UTF_8);
}

#[test]
fn uncommitted_commit_id_detects_zero_and_empty_for_any_hash_length() {
    // git's all-zero sentinel, SHA-1 (40) and SHA-256 (64) widths.
    assert!(is_uncommitted_commit_id(&"0".repeat(40)));
    assert!(is_uncommitted_commit_id(&"0".repeat(64)));
    // An empty/unparsed id is treated as uncommitted (names no real commit).
    assert!(is_uncommitted_commit_id(""));
    // Real commit ids of either width are committed.
    assert!(!is_uncommitted_commit_id("deadbeef"));
    assert!(!is_uncommitted_commit_id(&format!("{}1", "0".repeat(63))));
    // CommitId delegates to the same rule.
    assert!(CommitId("0".repeat(40).into()).is_uncommitted());
    assert!(CommitId("".into()).is_uncommitted());
    assert!(!CommitId("abc123".into()).is_uncommitted());
}

#[test]
fn submodule_status_maps_known_git_markers() {
    assert_eq!(
        SubmoduleStatus::from_git_status_marker(' '),
        SubmoduleStatus::UpToDate
    );
    assert_eq!(
        SubmoduleStatus::from_git_status_marker('-'),
        SubmoduleStatus::NotInitialized
    );
    assert_eq!(
        SubmoduleStatus::from_git_status_marker('+'),
        SubmoduleStatus::HeadMismatch
    );
    assert_eq!(
        SubmoduleStatus::from_git_status_marker('U'),
        SubmoduleStatus::MergeConflict
    );
    assert_eq!(
        SubmoduleStatus::from_git_status_marker('!'),
        SubmoduleStatus::MissingMapping
    );
}

#[test]
fn submodule_status_round_trips_unknown_git_marker() {
    let status = SubmoduleStatus::from_git_status_marker('M');
    assert_eq!(status, SubmoduleStatus::Unknown('M'));
    assert_eq!(status.git_status_marker(), 'M');
}

#[test]
fn unified_reader_matches_string_parser() {
    let target = DiffTarget::working_tree(PathBuf::from("src/main.rs"), DiffArea::Unstaged);
    let unified = "\
diff --git a/src/main.rs b/src/main.rs\n\
index 1111111..2222222 100644\n\
--- a/src/main.rs\n\
+++ b/src/main.rs\n\
@@ -1,2 +1,3 @@\n\
 fn main() {\n\
-    println!(\"old\");\n\
+    println!(\"new\");\n\
+    println!(\"extra\");\n\
 }\n";

    let from_text = Diff::from_unified(target.clone(), unified);
    let from_reader = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
        .expect("reader parse should succeed");

    assert_eq!(from_reader, from_text);
    assert_eq!(from_reader.lines[0].kind, DiffLineKind::Header);
    assert_eq!(from_reader.lines[4].kind, DiffLineKind::Hunk);
    assert_eq!(from_reader.lines[6].kind, DiffLineKind::Remove);
    assert_eq!(from_reader.lines[7].kind, DiffLineKind::Add);
}

#[test]
fn unified_reader_trims_crlf_line_endings() {
    let target = DiffTarget::working_tree(PathBuf::from("README.md"), DiffArea::Unstaged);
    let unified = "\
@@ -1 +1 @@\r\n\
-old\r\n\
+new\r\n";

    let diff = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
        .expect("reader parse should succeed");
    assert_eq!(diff.lines.len(), 3);
    assert_eq!(diff.lines[0].kind, DiffLineKind::Hunk);
    assert_eq!(diff.lines[1].text.as_ref(), "-old");
    assert_eq!(diff.lines[2].text.as_ref(), "+new");
}

fn target(path: &str) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged)
}

#[test]
fn crlf_lines_keep_their_cr_in_raw_bytes() {
    let unified = "@@ -1 +1 @@\r\n-old\r\n+new\n context\r\r\n";
    let diff = Diff::from_unified_reader(target("a.txt"), Cursor::new(unified.as_bytes())).unwrap();
    assert_eq!(diff.lines[1].text.raw_bytes(), b"-old\r");
    assert_eq!(diff.lines[2].text.raw_bytes(), b"+new");
    // Only the final CR was stripped; one CR stays in the text.
    assert_eq!(diff.lines[3].text.as_ref(), " context\r");
    assert_eq!(diff.lines[3].text.raw_bytes(), b" context\r\r");
    // A slice is not a whole line and gets its text.
    let slice = diff.lines[1].text.slice(1..4).unwrap();
    assert_eq!(slice.raw_bytes(), b"old");
}

#[test]
fn crlf_and_lf_versions_of_a_line_are_not_equal() {
    let crlf = Diff::from_unified(target("a.txt"), "+x\r\n");
    let lf = Diff::from_unified(target("a.txt"), "+x\n");
    assert_eq!(crlf.lines[0].text.as_ref(), lf.lines[0].text.as_ref());
    assert_ne!(crlf, lf);
}

#[test]
fn non_utf8_output_decodes_per_side_and_keeps_raw_bytes() {
    let raw: &[u8] = b"diff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@ caf\xe9\n caf\xe9\r\n-na\xefve\n+na\xc3\xafve\n";
    let latin1 = TextFormat {
        encoding: crate::text_format::TextEncoding::WINDOWS_1252,
        bom: false,
    };
    let diff = Diff::from_unified_bytes(target("a.txt"), raw.to_vec(), |_| DiffSectionFormats {
        old: latin1,
        new: TextFormat::UTF_8,
    });
    let texts: Vec<&str> = diff.lines.iter().map(|line| line.text.as_ref()).collect();
    // The context line decodes as the new side (UTF-8), so 0xE9 is invalid there.
    assert_eq!(texts[4], "@@ -1,2 +1,2 @@ café");
    assert_eq!(texts[5], " caf\u{fffd}");
    assert_eq!(texts[6], "-naïve");
    assert_eq!(texts[7], "+naïve");
    assert_eq!(diff.lines[5].text.raw_bytes(), b" caf\xe9\r");
    assert_eq!(diff.lines[6].text.raw_bytes(), b"-na\xefve");
    assert_eq!(diff.lines[6].kind, DiffLineKind::Remove);
    assert_eq!(diff.lines[0].kind, DiffLineKind::Header);
}

#[test]
fn reader_decodes_a_latin1_patch_instead_of_failing() {
    let raw = b"diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-caf\xe9 cr\xe8me br\xfbl\xe9e\n+caf\xe9 cr\xe8me br\xfbl\xe9e!\n";
    let diff = Diff::from_unified_reader(target("a.txt"), Cursor::new(&raw[..])).unwrap();
    assert_eq!(diff.lines[2].text.as_ref(), "-café crème brûlée");
    assert_eq!(
        diff.lines[3].text.raw_bytes(),
        b"+caf\xe9 cr\xe8me br\xfbl\xe9e!"
    );
}

#[test]
fn hunk_context_supplies_old_side_encoding_when_changed_lines_are_ascii() {
    let diff = Diff::from_unified_bytes(
        target("a.txt"),
        b"@@ -6 +6 @@ caf\xe9\n-old\n+\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n".to_vec(),
        DiffSectionFormats::sniff,
    );
    assert_eq!(diff.lines[0].text.as_ref(), "@@ -6 +6 @@ café");
    assert_eq!(diff.lines[2].text.as_ref(), "+日本語");
}

#[test]
fn sections_split_on_diff_headers() {
    let raw = b"diff --git a/a b/a\n+1\ndiff --git a/b b/b\n+2\n";
    let mut seen = Vec::new();
    let _ = Diff::from_unified_bytes(target("a"), raw.to_vec(), |section| {
        seen.push(section.to_vec());
        DiffSectionFormats::UTF_8
    });
    assert_eq!(
        seen,
        vec![
            b"diff --git a/a b/a\n+1\n".to_vec(),
            b"diff --git a/b b/b\n+2\n".to_vec()
        ]
    );
}

#[test]
fn unified_reader_handles_small_buffer_chunks_without_extra_newline_bytes() {
    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs\r\n\
@@ -1,2 +1,2 @@\r\n\
-alpha beta gamma delta epsilon\r\n\
+omega beta gamma delta epsilon\r\n";

    let reader = std::io::BufReader::with_capacity(7, Cursor::new(unified.as_bytes()));
    let diff = Diff::from_unified_reader(target, reader).expect("reader parse should succeed");

    assert_eq!(diff.lines.len(), 4);
    assert_eq!(
        diff.lines[0].text.as_ref(),
        "diff --git a/src/lib.rs b/src/lib.rs"
    );
    assert_eq!(diff.lines[1].kind, DiffLineKind::Hunk);
    assert_eq!(
        diff.lines[2].text.as_ref(),
        "-alpha beta gamma delta epsilon"
    );
    assert_eq!(
        diff.lines[3].text.as_ref(),
        "+omega beta gamma delta epsilon"
    );
}

#[test]
fn unified_reader_lines_share_backing_storage() {
    let target = DiffTarget::working_tree(PathBuf::from("README.md"), DiffArea::Unstaged);
    let unified = "\
@@ -1 +1 @@\n\
-old\n\
+new\n";

    let diff = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
        .expect("reader parse should succeed");

    assert_eq!(diff.lines.len(), 3);
    assert!(diff.lines[0].text.shares_storage_with(&diff.lines[1].text));
    assert!(diff.lines[1].text.shares_storage_with(&diff.lines[2].text));
}

#[test]
fn unified_reader_keeps_the_limit_result_when_the_drain_fails() {
    // A killed `git diff` leaves the pipe erroring; keep what we read.
    struct FailsWhenDrained(Cursor<&'static [u8]>);
    impl std::io::Read for FailsWhenDrained {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.read(buf)? {
                0 => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "child went away",
                )),
                read => Ok(read),
            }
        }
    }

    let (text, notice) =
        Diff::read_unified_text_with_limits(FailsWhenDrained(Cursor::new(b"aa\nbb\ncc\n")), 5, 100)
            .expect("a failing drain must not discard the diff");
    assert_eq!(text, "aa\n");
    assert!(notice.expect("notice").contains("5-byte"));
}

#[test]
fn unified_reader_drains_the_whole_reader_after_truncating() {
    // The reader is a child Git pipe. Closing it early kills Git with
    // SIGPIPE, and the non-zero exit outranks the diff we just parsed.
    let mut reader = Cursor::new(vec![b'x'; 100]);
    let (text, notice) = Diff::read_unified_text_with_limits(&mut reader, 4, 100).unwrap();

    assert_eq!(text.len(), 4);
    assert!(notice.is_some());
    assert_eq!(
        reader.position(),
        100,
        "Git must reach the end of its output and exit normally"
    );
}

#[test]
fn unified_reader_truncates_instead_of_failing() {
    let (text, notice) =
        Diff::read_unified_text_with_limits(Cursor::new(b"aa\nbb\ncc\n"), 100, 2).unwrap();
    assert_eq!(text, "aa\nbb\n");
    assert!(notice.expect("notice").contains("2-line"));

    // A single line longer than the limit is cut on a UTF-8 boundary.
    let (text, notice) =
        Diff::read_unified_text_with_limits(Cursor::new("aaaä".as_bytes()), 4, 100).unwrap();
    assert_eq!(text, "aaa");
    assert!(notice.expect("notice").contains("4-byte"));
}

#[test]
fn truncated_diffs_end_with_a_notice_row() {
    let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);
    let diff = Diff::from_unified_reader_with_limits(target, Cursor::new(b"aa\nbb\ncc\n"), 100, 2)
        .unwrap();
    let last = diff.lines.last().expect("notice row");
    assert_eq!(last.kind, DiffLineKind::Header);
    assert!(
        last.text
            .as_ref()
            .starts_with(Diff::TRUNCATION_NOTICE_PREFIX),
        "{:?}",
        last.text.as_ref()
    );
    assert_eq!(diff.lines.len(), 3);
}

#[test]
fn unified_reader_leaves_diffs_within_the_limits_untouched() {
    for (text, max_bytes, max_lines) in [("1234", 4, 1), ("a\nb\n", 4, 2)] {
        let (read, notice) =
            Diff::read_unified_text_with_limits(Cursor::new(text), max_bytes, max_lines).unwrap();
        assert_eq!(read, text);
        assert_eq!(notice, None);
    }
}

#[test]
fn paged_provider_loads_pages_on_demand() {
    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs\n\
@@ -1,4 +1,4 @@\n\
 old1\n\
-old2\n\
+new2\n\
 old3\n";
    let diff = Diff::from_unified(target, unified);
    let provider = diff.paged_lines(2);

    assert_eq!(provider.cached_page_count(), 0);
    assert_eq!(provider.len_hint(), diff.lines.len());

    let line = provider.row(3).expect("line 3 should exist");
    assert_eq!(line.text.as_ref(), "-old2");
    assert_eq!(provider.cached_page_count(), 1);

    let line = provider.row(0).expect("line 0 should exist");
    assert_eq!(line.text.as_ref(), "diff --git a/src/lib.rs b/src/lib.rs");
    assert_eq!(provider.cached_page_count(), 2);

    let slice = provider
        .slice(2, 5)
        .map(|line| line.text.to_string())
        .collect::<Vec<_>>();
    assert_eq!(slice, vec!["old1", "-old2", "+new2"]);
    assert_eq!(provider.cached_page_count(), 3);
}

// --- Tests moved from tests/domain_smoke.rs ---

#[test]
fn commit_id_is_hashable() {
    let mut set = FxHashSet::default();
    set.insert(CommitId("a".into()));
    set.insert(CommitId("b".into()));
    assert!(set.contains(&CommitId("a".into())));
}

#[test]
fn log_cursor_roundtrips() {
    let cursor = LogCursor {
        last_seen: CommitId("deadbeef".into()),
        resume_from: Some(CommitId("feedface".into())),
        resume_token: Some(Arc::from("cursor-token")),
    };
    assert_eq!(cursor.last_seen.as_ref(), "deadbeef");
    assert_eq!(
        cursor.resume_from.as_ref().map(AsRef::as_ref),
        Some("feedface")
    );
    assert_eq!(cursor.resume_token.as_deref(), Some("cursor-token"));
}

#[test]
fn commit_struct_is_constructible() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    let commit = Commit {
        id: CommitId("1".into()),
        parent_ids: smallvec::smallvec![CommitId("0".into())],
        summary: "test".into(),
        author: "me".into(),
        time: now,
    };
    assert_eq!(&*commit.summary, "test");
}

// --- Tests moved from tests/diff_from_unified.rs ---

#[test]
fn diff_from_unified_classifies_lines() {
    let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);

    let text = "\
diff --git a/a.txt b/a.txt
index 0000000..1111111 100644
--- a/a.txt
+++ b/a.txt
@@ -0,0 +1,2 @@
+hello
 world
-bye
";

    let diff = Diff::from_unified(target, text);
    assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Header));
    assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Hunk));
    assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Add));
    assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Remove));
    assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Context));
}

#[test]
fn diff_from_unified_treats_three_dash_content_as_removed_line() {
    let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);

    let diff = Diff::from_unified(
        target,
        "\
@@ -1 +1 @@
----keep this as removed content
+++ header
",
    );

    assert_eq!(diff.lines[1].kind, DiffLineKind::Remove);
    assert_eq!(diff.lines[2].kind, DiffLineKind::Add);
}

/// A removed `-- comment` reads `--- comment` and an added `++ x` reads
/// `+++ x`. The hunk's counts make them content; read as file headers they
/// were hidden from the diff view and swept into line-level staging.
#[test]
fn diff_from_unified_reads_header_lookalikes_inside_a_hunk_as_content() {
    let target = DiffTarget::working_tree(PathBuf::from("q.sql"), DiffArea::Unstaged);
    let text = "\
diff --git a/q.sql b/q.sql
index 1111111..2222222 100644
--- a/q.sql
+++ b/q.sql
@@ -1,3 +1,3 @@ create
 select 1;
--- old comment
+++ new comment
 select 2;
diff --git a/r.sql b/r.sql
index 3333333..4444444 100644
--- a/r.sql
+++ b/r.sql
@@ -1 +1 @@
-x
\\ No newline at end of file
+y
";
    use DiffLineKind::{Add, Context, Header, Hunk, Remove};
    let expected = vec![
        Header, Header, Header, Header, Hunk, Context, Remove, Add, Context, Header, Header,
        Header, Header, Hunk, Remove, Context, Add,
    ];
    let kinds = |diff: Diff| diff.lines.iter().map(|line| line.kind).collect::<Vec<_>>();

    assert_eq!(kinds(Diff::from_unified(target.clone(), text)), expected);
    let combined = Diff::from_unified(
        target.clone(),
        "diff --cc q.sql\n@@@ -1,2 -1,2 +1,1 @@@\n  keep\n--- dropped by both\n",
    );
    assert_eq!(kinds(combined), vec![Header, Hunk, Context, Remove]);
    // A Latin-1 byte takes the per-line decoding path.
    let latin1 = text
        .replace("old comment", "old comment \u{1}")
        .into_bytes();
    let latin1: Vec<u8> = latin1
        .into_iter()
        .map(|byte| if byte == 1 { 0xE9 } else { byte })
        .collect();
    assert_eq!(
        kinds(Diff::from_unified_bytes(
            target,
            latin1,
            DiffSectionFormats::sniff
        )),
        expected
    );
}

#[test]
fn stash_and_reflog_entries_share_arc_text_on_clone() {
    let stash = StashEntry {
        index: 0,
        id: CommitId("stash".into()),
        message: "stash message".into(),
        created_at: None,
    };
    let stash_clone = stash.clone();
    assert!(Arc::ptr_eq(&stash.message, &stash_clone.message));

    let reflog = ReflogEntry {
        index: 0,
        new_id: CommitId("head".into()),
        message: "reflog message".into(),
        time: None,
        selector: "HEAD@{0}".into(),
        author: "Jane Doe".into(),
    };
    let reflog_clone = reflog.clone();
    assert!(Arc::ptr_eq(&reflog.message, &reflog_clone.message));
    assert!(Arc::ptr_eq(&reflog.selector, &reflog_clone.selector));
}

mod file_status_count_tests {
    use super::*;

    fn status(kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from("a.txt"),
            kind,
            conflict: None,
        }
    }

    #[test]
    fn counts_bucket_untracked_and_added_together() {
        let entries = [
            status(FileStatusKind::Untracked),
            status(FileStatusKind::Added),
        ];
        assert_eq!(count_file_statuses(&entries), (2, 0, 0));
    }

    #[test]
    fn counts_bucket_renamed_and_conflicted_as_modified() {
        let entries = [
            status(FileStatusKind::Modified),
            status(FileStatusKind::Renamed),
            status(FileStatusKind::Conflicted),
        ];
        assert_eq!(count_file_statuses(&entries), (0, 3, 0));
    }

    #[test]
    fn counts_are_zero_for_an_empty_status_list() {
        assert_eq!(count_file_statuses(&[]), (0, 0, 0));
    }

    #[test]
    fn a_summary_is_dirty_only_when_some_bucket_is_non_zero() {
        let clean = WorktreeDirtySummary {
            path: PathBuf::from("/tmp/wt"),
            head: None,
            branch: None,
            detached: false,
            added: 0,
            modified: 0,
            deleted: 0,
            staged: Vec::new(),
            unstaged: Vec::new(),
            line_stats: UncommittedLineStats::default(),
        };
        assert!(!clean.is_dirty());
        assert!(
            WorktreeDirtySummary {
                deleted: 1,
                ..clean
            }
            .is_dirty()
        );
    }
}

mod history_focus_tests {
    use super::*;

    #[test]
    fn history_focus_names_the_ref_it_seeds_from() {
        assert_eq!(
            HistoryFocus::local_branch("feature").ref_name().as_deref(),
            Some("refs/heads/feature")
        );
        assert_eq!(
            HistoryFocus::remote_branch("origin", "feature/awesome")
                .ref_name()
                .as_deref(),
            Some("refs/remotes/origin/feature/awesome")
        );
        // A whole remote is many refs, so it names none of them.
        assert_eq!(HistoryFocus::remote("origin").ref_name(), None);

        assert_eq!(HistoryFocus::local_branch("feature").label(), "feature");
        assert_eq!(
            HistoryFocus::remote_branch("origin", "feature").label(),
            "origin/feature"
        );
        assert_eq!(HistoryFocus::remote("origin").label(), "origin");
    }

    /// The modes that promise HEAD is the first commit only promise it for a
    /// walk seeded from HEAD; a focus seeds it somewhere else.
    #[test]
    fn a_focus_withdraws_the_head_is_first_guarantee() {
        let empty = HistoryFocusSet::default();
        let focused = HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]);
        for mode in [
            HistoryMode::FullReachable,
            HistoryMode::FirstParent,
            HistoryMode::NoMerges,
            HistoryMode::MergesOnly,
            HistoryMode::AllBranches,
        ] {
            assert_eq!(
                empty.head_is_first(mode),
                mode.guarantees_head_visibility(),
                "an unfocused walk keeps whatever the mode promises"
            );
            assert!(
                !focused.head_is_first(mode),
                "a focused walk promises nothing about HEAD under {mode:?}"
            );
        }
    }

    fn no_remote_branches(_: &str) -> Vec<String> {
        Vec::new()
    }

    /// Per-ref toggles and group entries add and remove targets in bulk without
    /// disturbing the others.
    #[test]
    fn focusing_and_unfocusing_refs_leaves_the_others_alone() {
        let main = HistoryFocus::local_branch("main");
        let feature = HistoryFocus::local_branch("feature");
        let origin = HistoryFocus::remote("origin");

        let set = HistoryFocusSet::default().with_refs_focused(
            &[main.clone(), feature.clone()],
            true,
            no_remote_branches,
        );
        assert_eq!(set.len(), 2);
        assert!(set.contains(&main) && set.contains(&feature));
        assert!(!set.contains(&origin));

        let set = set.with_refs_focused(std::slice::from_ref(&main), false, no_remote_branches);
        assert_eq!(set.len(), 1, "unfocusing one target removes just that one");
        assert!(set.contains(&feature));

        assert!(
            set.with_refs_focused(std::slice::from_ref(&feature), false, no_remote_branches)
                .is_empty()
        );
    }

    /// A whole remote focuses every branch under it, so those branches read as
    /// focused even though the set holds no entry for them.
    #[test]
    fn a_focused_remote_covers_its_branches_only() {
        let set = HistoryFocusSet::from_iter([
            HistoryFocus::remote("origin"),
            HistoryFocus::local_branch("main"),
        ]);
        assert!(set.covers(&HistoryFocus::remote_branch("origin", "feature")));
        assert!(set.covers_remote_branch("origin", "feature"));
        assert!(!set.covers_remote_branch("upstream", "feature"));
        assert!(set.covers_local_branch("main"));
        assert!(
            !set.covers_local_branch("origin"),
            "a remote never covers a local branch that shares its name"
        );
        assert!(!set.covers(&HistoryFocus::remote("upstream")));
    }

    /// Focusing a branch its remote already covers adds nothing: the set stays
    /// the same value, so the walk is not restarted for no reason.
    #[test]
    fn focusing_an_already_covered_branch_changes_nothing() {
        let set = HistoryFocusSet::from_iter([HistoryFocus::remote("origin")]);
        assert_eq!(
            set.with_refs_focused(
                &[HistoryFocus::remote_branch("origin", "feature")],
                true,
                no_remote_branches,
            ),
            set
        );
    }

    /// Unfocusing one branch of a focused remote keeps the rest of the remote in
    /// view: the remote's entry becomes its remaining branches.
    #[test]
    fn unfocusing_a_branch_under_a_focused_remote_keeps_its_siblings() {
        let set = HistoryFocusSet::from_iter([
            HistoryFocus::remote("origin"),
            HistoryFocus::remote("upstream"),
        ]);
        let origin_branches = |remote: &str| -> Vec<String> {
            assert_eq!(
                remote, "origin",
                "only the remote losing a branch is listed"
            );
            vec!["main".into(), "feat/a".into(), "feat/b".into()]
        };

        let set = set.with_refs_focused(
            &[
                HistoryFocus::remote_branch("origin", "feat/a"),
                HistoryFocus::remote_branch("origin", "feat/b"),
            ],
            false,
            origin_branches,
        );
        assert_eq!(
            set,
            HistoryFocusSet::from_iter([
                HistoryFocus::remote_branch("origin", "main"),
                HistoryFocus::remote("upstream"),
            ])
        );
    }

    /// Two sets built in different orders are the same value; walk caches and
    /// snapshots are keyed on it, so a reorder must not look like a change.
    #[test]
    fn set_identity_ignores_insertion_order_and_duplicates() {
        let a = HistoryFocus::local_branch("a");
        let b = HistoryFocus::local_branch("b");
        assert_eq!(
            HistoryFocusSet::from_iter([a.clone(), b.clone()]),
            HistoryFocusSet::from_iter([b.clone(), a.clone(), a.clone()])
        );
    }
}
