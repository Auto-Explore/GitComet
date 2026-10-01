//! Non-UTF-8 content through the real backend: attributes, detection,
//! transcoded file-diff sides and decoded patches.

use gitcomet_core::domain::{
    CommitId, Diff, DiffArea, DiffLineKind, DiffTarget, FileDiffText, FileDiffTextSource,
};
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use gitcomet_core::text_format::{FormatSource, LineEnding, TextEncoding};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "you@example.com"]);
    git(repo, &["config", "user.name", "You"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    git(repo, &["config", "core.autocrlf", "false"]);
    dir
}

fn commit_all(repo: &Path, message: &str) -> CommitId {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
    let head = String::from_utf8(git(repo, &["rev-parse", "HEAD"])).unwrap();
    CommitId(head.trim().into())
}

fn open(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repo")
}

fn unstaged(path: &str) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged)
}

fn staged(path: &str) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), DiffArea::Staged)
}

fn file_text(
    repo: &dyn GitRepository,
    target: &DiffTarget,
    encoding: Option<TextEncoding>,
) -> FileDiffText {
    repo.diff_file_text_with_encoding_cancellable(target, encoding, &CancellationToken::new())
        .expect("file text")
        .expect("file text present")
}

fn read_side(source: Option<&FileDiffTextSource>) -> String {
    let source = source.expect("side present");
    String::from_utf8(fs::read(&source.path).unwrap()).expect("side is UTF-8")
}

fn patch(repo: &dyn GitRepository, target: &DiffTarget, encoding: Option<TextEncoding>) -> Diff {
    repo.diff_parsed_with_encoding_cancellable(target, encoding, &CancellationToken::new())
        .expect("patch parses")
}

fn changed_lines(diff: &Diff) -> Vec<&str> {
    diff.lines
        .iter()
        .filter(|line| matches!(line.kind, DiffLineKind::Add | DiffLineKind::Remove))
        .map(|line| line.text.as_ref())
        .collect()
}

#[test]
fn staged_single_latin1_byte_decodes_in_patch_and_file_text() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(repo_dir.join("seed.txt"), "seed\n").unwrap();
    commit_all(repo_dir, "seed");
    fs::write(repo_dir.join("test (1).txt"), b"\xe9").unwrap();
    git(repo_dir, &["add", "test (1).txt"]);

    let repo = open(repo_dir);
    let target = staged("test (1).txt");
    let diff = patch(&*repo, &target, None);
    assert_eq!(changed_lines(&diff), vec!["+é"]);
    let add = diff
        .lines
        .iter()
        .find(|line| line.kind == DiffLineKind::Add)
        .unwrap();
    assert_eq!(add.text.raw_bytes(), b"+\xe9");

    let text = file_text(&*repo, &target, None);
    assert!(text.old_source.is_none());
    assert_eq!(read_side(text.new_source.as_ref()), "é");
    let format = text.new_source.unwrap().format.unwrap();
    assert_eq!(format.format.encoding, TextEncoding::WINDOWS_1252);
    assert!(matches!(format.source, FormatSource::Detected { .. }));
}

#[test]
fn modified_latin1_file_diffs_as_text_and_keeps_raw_patch_bytes() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9 cr\xe8me\r\nCr\xeape Suzette\r\n",
    )
    .unwrap();
    commit_all(repo_dir, "menu");
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e\r\nCr\xeape Suzette\r\n",
    )
    .unwrap();

    let repo = open(repo_dir);
    let target = unstaged("menu.txt");
    let diff = patch(&*repo, &target, None);
    assert_eq!(
        changed_lines(&diff),
        vec!["-Café crème", "+Café crème brûlée"]
    );
    let removed = diff
        .lines
        .iter()
        .find(|line| line.kind == DiffLineKind::Remove)
        .unwrap();
    assert_eq!(removed.text.raw_bytes(), b"-Caf\xe9 cr\xe8me\r");

    let text = file_text(&*repo, &target, None);
    assert_eq!(
        read_side(text.old_source.as_ref()),
        "Café crème\r\nCrêpe Suzette\r\n"
    );
    assert_eq!(
        read_side(text.new_source.as_ref()),
        "Café crème brûlée\r\nCrêpe Suzette\r\n"
    );
    let new = text.new_source.unwrap().format.unwrap();
    assert_eq!(new.line_endings.uniform(), Some(LineEnding::CrLf));
}

#[test]
fn override_changes_how_both_views_read_the_file() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    // "Привет" in KOI8-R; detection alone may pick another Cyrillic encoding.
    fs::write(repo_dir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").unwrap();
    commit_all(repo_dir, "ru");
    fs::write(repo_dir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4!\n").unwrap();

    let repo = open(repo_dir);
    let koi8 = TextEncoding::from_label("koi8-r");
    let target = unstaged("ru.txt");
    let diff = patch(&*repo, &target, koi8);
    assert_eq!(changed_lines(&diff), vec!["-Привет", "+Привет!"]);
    let text = file_text(&*repo, &target, koi8);
    assert_eq!(read_side(text.new_source.as_ref()), "Привет!\n");
    let new = text.new_source.as_ref().unwrap();
    assert_eq!(new.format.unwrap().source, FormatSource::Override);

    // A different choice is a different source identity, so views rebuild.
    let other = file_text(&*repo, &target, TextEncoding::from_label("windows-1251"));
    assert_ne!(other.new_source.unwrap().identity, new.identity);
}

#[test]
fn encoding_attribute_is_followed() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(repo_dir.join(".gitattributes"), "*.txt encoding=koi8-r\n").unwrap();
    fs::write(repo_dir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").unwrap();
    commit_all(repo_dir, "ru");
    fs::write(repo_dir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4?\n").unwrap();

    let repo = open(repo_dir);
    let attributes = repo.text_attributes(Path::new("ru.txt")).unwrap();
    assert_eq!(
        attributes.encoding.as_ref().and_then(|attr| attr.encoding),
        TextEncoding::from_label("koi8-r")
    );
    let target = unstaged("ru.txt");
    assert_eq!(
        changed_lines(&patch(&*repo, &target, None)),
        vec!["-Привет", "+Привет?"]
    );
    let text = file_text(&*repo, &target, None);
    assert_eq!(
        text.new_source.unwrap().format.unwrap().source,
        FormatSource::EncodingAttribute
    );
}

#[test]
fn config_changes_refresh_attributes_and_decoding_without_reopening() {
    use gitcomet_core::text_format::TabWidthSource;
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let workdir = dir.path();
    git(workdir, &["config", "gui.encoding", "windows-1252"]);
    git(workdir, &["config", "core.whitespace", "tabwidth=4"]);
    fs::write(workdir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").unwrap();
    commit_all(workdir, "base");
    fs::write(workdir.join("ru.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4!\n").unwrap();
    let commit = commit_all(workdir, "change");
    let repo = open(workdir);
    let target = DiffTarget::commit(commit, "ru.txt".into());
    let original = file_text(&*repo, &target, None);
    assert_eq!(
        original
            .new_source
            .as_ref()
            .unwrap()
            .format
            .unwrap()
            .format
            .encoding,
        TextEncoding::WINDOWS_1252
    );
    assert_eq!(
        repo.text_attributes(Path::new("ru.txt"))
            .unwrap()
            .tab_width
            .unwrap()
            .columns,
        4
    );

    // Included configuration is resolved by gix too, rather than by a custom
    // reader that only checks .git/config.
    git(workdir, &["config", "include.path", "text-config"]);
    fs::write(
        workdir.join(".git/text-config"),
        "[gui]\nencoding = KOI8-R\n[core]\nwhitespace = tabwidth=8\n",
    )
    .unwrap();
    let attributes = repo.text_attributes(Path::new("ru.txt")).unwrap();
    assert_eq!(
        attributes.gui_encoding.unwrap().encoding,
        TextEncoding::from_label("koi8-r")
    );
    let tab = attributes.tab_width.unwrap();
    assert_eq!(tab.columns, 8);
    assert_eq!(tab.source, TabWidthSource::CoreWhitespace);
    assert_eq!(
        read_side(file_text(&*repo, &target, None).new_source.as_ref()),
        "Привет!\n"
    );
    assert_eq!(
        changed_lines(&patch(&*repo, &target, None)),
        vec!["-Привет", "+Привет!"]
    );

    // A later update of an existing include is visible through the same handle.
    fs::write(
        workdir.join(".git/text-config"),
        "[gui]\nencoding = windows-1252\n[core]\nwhitespace = tabwidth=2\n",
    )
    .unwrap();
    let attributes = repo.text_attributes(Path::new("ru.txt")).unwrap();
    assert_eq!(
        attributes.gui_encoding.unwrap().encoding,
        Some(TextEncoding::WINDOWS_1252)
    );
    assert_eq!(attributes.tab_width.unwrap().columns, 2);
    assert_eq!(
        read_side(file_text(&*repo, &target, None).new_source.as_ref()),
        read_side(original.new_source.as_ref())
    );
}

#[test]
fn working_tree_encoding_utf16_with_iconv_label_diffs_as_text() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join(".gitattributes"),
        "*.ps1 text working-tree-encoding=UTF-16LE-BOM eol=crlf\n",
    )
    .unwrap();
    let utf16 = |text: &str| {
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    };
    fs::write(repo_dir.join("script.ps1"), utf16("Write-Host 'é'\r\n")).unwrap();
    commit_all(repo_dir, "script");
    // Git stores UTF-8 with LF.
    assert_eq!(
        git(repo_dir, &["cat-file", "blob", "HEAD:script.ps1"]),
        "Write-Host 'é'\n".as_bytes()
    );
    fs::write(
        repo_dir.join("script.ps1"),
        utf16("Write-Host 'é'\r\nWrite-Host 'ü'\r\n"),
    )
    .unwrap();

    let repo = open(repo_dir);
    let attributes = repo.text_attributes(Path::new("script.ps1")).unwrap();
    assert_eq!(
        attributes.working_tree_encoding(),
        Some(TextEncoding::UTF_16LE)
    );
    assert_eq!(attributes.eol_policy.checkout, Some(LineEnding::CrLf));

    let target = unstaged("script.ps1");
    assert_eq!(
        changed_lines(&patch(&*repo, &target, None)),
        vec!["+Write-Host 'ü'"]
    );
    let text = file_text(&*repo, &target, None);
    assert_eq!(read_side(text.old_source.as_ref()), "Write-Host 'é'\n");
    // The worktree side is normalized like git does: UTF-8, LF.
    assert_eq!(
        read_side(text.new_source.as_ref()),
        "Write-Host 'é'\nWrite-Host 'ü'\n"
    );
    assert_eq!(
        text.old_source.unwrap().format.unwrap().source,
        FormatSource::GitInternalUtf8
    );
}

#[test]
fn working_tree_encoding_preserves_git_ident_normalization() {
    test_git_env::ensure_initialized();
    for encoding in ["UTF-16LE-BOM", "ISO-8859-1"] {
        for (ident, contract) in [("ident", true), ("-ident", false), ("!ident", false)] {
            let dir = init_repo();
            let repo_dir = dir.path();
            fs::write(
                repo_dir.join(".gitattributes"),
                format!("*.txt text eol=crlf working-tree-encoding={encoding} {ident}\n"),
            )
            .unwrap();
            let format = gitcomet_core::text_format::TextFormat {
                encoding: TextEncoding::from_label(encoding).unwrap(),
                bom: encoding.ends_with("-BOM"),
            };
            let write = |text: &str| {
                fs::write(
                    repo_dir.join("notes.txt"),
                    gitcomet_core::text_format::encode(text, format).unwrap(),
                )
                .unwrap();
            };
            write("$Id$\r\ncafé\r\n");
            commit_all(repo_dir, "notes");
            // A checkout expansion is unrelated to the user's edit below it.
            write("$Id: 0123456789abcdef $\r\ncafé!\r\n");

            let repo = open(repo_dir);
            assert_eq!(
                repo.text_attributes(Path::new("notes.txt")).unwrap().ident,
                contract
            );
            let target = unstaged("notes.txt");
            let text = file_text(&*repo, &target, None);
            assert_eq!(read_side(text.old_source.as_ref()), "$Id$\ncafé\n");
            let new = read_side(text.new_source.as_ref());
            let marker = if contract {
                "$Id$"
            } else {
                "$Id: 0123456789abcdef $"
            };
            assert_eq!(new, format!("{marker}\ncafé!\n"));
            let diff = patch(&*repo, &target, None);
            if contract {
                assert_eq!(changed_lines(&diff), vec!["-café", "+café!"]);
            }
            // Ask Git to normalize the same worktree content and compare its
            // index bytes, so this covers both the ident and EOL filters.
            git(repo_dir, &["add", "notes.txt"]);
            assert_eq!(new.as_bytes(), git(repo_dir, &["show", ":notes.txt"]));
        }
    }
}

#[test]
fn working_tree_encoding_utf16_follows_the_files_big_endian_bom() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    // Git requires a BOM under the bare label and reads either byte order.
    fs::write(
        repo_dir.join(".gitattributes"),
        "*.txt working-tree-encoding=UTF-16\n",
    )
    .unwrap();
    let mut utf16be = vec![0xfe, 0xff];
    for unit in "héllo\n".encode_utf16() {
        utf16be.extend_from_slice(&unit.to_be_bytes());
    }
    fs::write(repo_dir.join("notes.txt"), &utf16be).unwrap();
    commit_all(repo_dir, "notes");
    assert_eq!(
        git(repo_dir, &["cat-file", "blob", "HEAD:notes.txt"]),
        "héllo\n".as_bytes()
    );
    assert!(git(repo_dir, &["status", "--porcelain"]).is_empty());

    let repo = open(repo_dir);
    let text = file_text(&*repo, &unstaged("notes.txt"), None);
    assert_eq!(read_side(text.old_source.as_ref()), "héllo\n");
    assert_eq!(read_side(text.new_source.as_ref()), "héllo\n");
}

#[test]
fn shift_jis_that_would_not_write_back_the_same_is_not_writable() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join(".gitattributes"),
        "*.txt encoding=shift_jis\n",
    )
    .unwrap();
    // 日本 plus 0xED40, an NEC duplicate that encodes back as 0xFA5C.
    fs::write(repo_dir.join("notes.txt"), b"\x93\xfa\x96\x7b\xed\x40\n").unwrap();
    commit_all(repo_dir, "notes");
    fs::write(
        repo_dir.join("notes.txt"),
        b"\x93\xfa\x96\x7b\xed\x40\n\x93\xfa\n",
    )
    .unwrap();

    let repo = open(repo_dir);
    let text = file_text(&*repo, &unstaged("notes.txt"), None);
    for side in [text.old_source.as_ref(), text.new_source.as_ref()] {
        let format = side.and_then(|side| side.format).expect("side format");
        assert_eq!(format.format.encoding.name(), "Shift_JIS");
        assert!(!format.malformed);
        assert!(
            format.lossy,
            "the editor opens this read-only; so must the diff say"
        );
        assert!(!format.is_writable());
    }
}

#[test]
fn utf16_without_attributes_is_binary_to_git_but_text_to_the_file_view() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    let utf16 = |text: &str| {
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    };
    fs::write(repo_dir.join("notes.txt"), utf16("hello\r\n")).unwrap();
    commit_all(repo_dir, "notes");
    fs::write(repo_dir.join("notes.txt"), utf16("hello\r\nworld\r\n")).unwrap();

    let repo = open(repo_dir);
    let target = unstaged("notes.txt");
    let diff = patch(&*repo, &target, None);
    assert!(
        diff.lines
            .iter()
            .any(|line| line.text.starts_with("Binary files")),
        "git reports UTF-16 without working-tree-encoding as binary"
    );
    let text = file_text(&*repo, &target, None);
    assert_eq!(read_side(text.old_source.as_ref()), "hello\r\n");
    assert_eq!(read_side(text.new_source.as_ref()), "hello\r\nworld\r\n");
    for (source, original) in [
        (text.old_source.as_ref().unwrap(), "hello\r\n"),
        (text.new_source.as_ref().unwrap(), "hello\r\nworld\r\n"),
    ] {
        assert_eq!(
            fs::read(source.raw_path.as_ref().expect("original UTF-16 source")).unwrap(),
            utf16(original)
        );
    }
    let cached = file_text(&*repo, &target, None);
    assert_eq!(
        cached.new_source.as_ref().unwrap().raw_path,
        text.new_source.as_ref().unwrap().raw_path
    );
    let new = text.new_source.unwrap().format.unwrap();
    assert_eq!(new.format.encoding, TextEncoding::UTF_16LE);
    assert!(new.format.bom);
    assert_eq!(new.line_endings.crlf, 2);
}

#[test]
fn commit_converting_latin1_to_utf8_decodes_each_side_in_its_own_encoding() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join("readme.txt"),
        b"Gr\xfc\xdfe aus K\xf6ln und sch\xf6ne Stra\xdfen\n",
    )
    .unwrap();
    commit_all(repo_dir, "latin1");
    fs::write(
        repo_dir.join("readme.txt"),
        "Grüße aus Köln und schöne Straßen\n",
    )
    .unwrap();
    let converted = commit_all(repo_dir, "convert to utf-8");

    let repo = open(repo_dir);
    let target = DiffTarget::commit(converted, PathBuf::from("readme.txt"));
    assert_eq!(
        changed_lines(&patch(&*repo, &target, None)),
        vec![
            "-Grüße aus Köln und schöne Straßen",
            "+Grüße aus Köln und schöne Straßen"
        ]
    );
    let text = file_text(&*repo, &target, None);
    assert_eq!(
        text.old_source.unwrap().format.unwrap().format.encoding,
        TextEncoding::WINDOWS_1252
    );
    assert_eq!(
        text.new_source.unwrap().format.unwrap().format.encoding,
        TextEncoding::UTF_8
    );
}

#[test]
fn utf8_sources_are_not_copied() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(repo_dir.join("a.txt"), "one\n").unwrap();
    commit_all(repo_dir, "a");
    fs::write(repo_dir.join("a.txt"), "one\ntwo\n").unwrap();

    let repo = open(repo_dir);
    let text = file_text(&*repo, &unstaged("a.txt"), None);
    let old = text.old_source.unwrap();
    assert!(old.identity.starts_with("blob:"), "{}", old.identity);
    assert!(!old.identity.contains('@'));
    assert_eq!(old.format.unwrap().source, FormatSource::Utf8);
}

#[test]
fn eol_policy_reads_attributes_and_config() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join(".gitattributes"),
        "*.bat text eol=crlf\n*.bin binary\n*.go whitespace=tabwidth=8\n",
    )
    .unwrap();
    let repo = open(repo_dir);
    let bat = repo.text_attributes(Path::new("run.bat")).unwrap();
    assert_eq!(bat.eol_policy.checkout, Some(LineEnding::CrLf));
    assert!(bat.eol_policy.normalized);
    let bin = repo.text_attributes(Path::new("x.bin")).unwrap();
    assert!(bin.diff_unset);
    assert!(!bin.eol_policy.normalized);
    let go = repo.text_attributes(Path::new("main.go")).unwrap();
    assert_eq!(go.tab_width.map(|width| width.columns), Some(8));
    let plain = repo.text_attributes(Path::new("a.txt")).unwrap();
    assert_eq!(plain.eol_policy.checkout, None);
}

/// What the UI's hunk builder emits: every line's raw bytes, newline-joined.
fn whole_patch_from_raw_lines(diff: &Diff) -> Vec<u8> {
    let mut patch = Vec::new();
    for line in &diff.lines {
        patch.extend_from_slice(line.text.raw_bytes());
        patch.push(b'\n');
    }
    patch
}

fn stage_whole_patch_and_compare_index_with_worktree(repo_dir: &Path, file: &str) {
    let repo = open(repo_dir);
    let diff = patch(&*repo, &unstaged(file), None);
    repo.apply_unified_patch_to_index_with_output(&whole_patch_from_raw_lines(&diff), false)
        .expect("patch built from raw lines applies");
    assert_eq!(
        git(repo_dir, &["cat-file", "blob", &format!(":{file}")]),
        fs::read(repo_dir.join(file)).unwrap(),
        "the index must hold the worktree's exact bytes"
    );
}

#[test]
fn staging_a_latin1_hunk_writes_latin1_bytes_to_the_index() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(repo_dir.join("menu.txt"), b"Caf\xe9\n").unwrap();
    commit_all(repo_dir, "menu");
    // A pure addition: its text would be written as UTF-8 if built from
    // the decoded rows.
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9\nCr\xe8me br\xfbl\xe9e\n",
    )
    .unwrap();
    stage_whole_patch_and_compare_index_with_worktree(repo_dir, "menu.txt");
}

#[test]
fn staging_a_crlf_hunk_keeps_its_carriage_returns() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    // No text attribute and autocrlf off: git stores the CRs, so the patch
    // context must carry them. Stripping them made `git apply` reject it.
    fs::write(repo_dir.join("crlf.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
    commit_all(repo_dir, "crlf");
    fs::write(repo_dir.join("crlf.txt"), "one\r\nTWO\r\nthree\r\n").unwrap();
    stage_whole_patch_and_compare_index_with_worktree(repo_dir, "crlf.txt");
}

fn check_attr(repo: &Path, attribute: &str, path: &str) -> String {
    let output = git(repo, &["check-attr", "-z", attribute, "--", path]);
    let fields: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    String::from_utf8(fields[2].to_vec()).unwrap()
}

#[test]
fn saving_an_earlier_encoding_again_overrides_later_assignments() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let append = |rule: &str| {
        let path = dir.path().join(".gitattributes");
        let existing = fs::read(&path).unwrap_or_default();
        if let Some(updated) = gitcomet_core::gitattributes::append_rule(&existing, rule) {
            fs::write(path, updated).unwrap();
        }
    };
    for encoding in ["windows-1252", "koi8-r", "windows-1252"] {
        append(&format!("/menu.txt encoding={encoding}"));
        assert_eq!(check_attr(dir.path(), "encoding", "menu.txt"), encoding);
    }
    let before = fs::read(dir.path().join(".gitattributes")).unwrap();
    append("/menu.txt encoding=windows-1252");
    assert_eq!(fs::read(dir.path().join(".gitattributes")).unwrap(), before);
    append("*.txt encoding=koi8-r");
    append("/menu.txt encoding=windows-1252");
    assert_eq!(
        check_attr(dir.path(), "encoding", "menu.txt"),
        "windows-1252"
    );
}

#[test]
fn written_gitattributes_patterns_match_exactly_their_path_in_git() {
    test_git_env::ensure_initialized();
    use gitcomet_core::gitattributes::{append_rule, pattern_for_extension, pattern_for_path};
    let cases = [
        ("plain.txt", "plain.txtx"),
        ("dir/my file.txt", "dir/my  file.txt"),
        ("x[1]*.txt", "x1ab.txt"),
        ("a b[1].txt", "a b1.txt"),
        ("#hash.txt", "hash.txt"),
        ("!bang.txt", "bang.txt"),
        ("sub/dir/deep.txt", "other/sub/dir/deep.txt"),
        ("café.txt", "cafe.txt"),
        #[cfg(unix)]
        (r"dir\name.txt", "dir/name.txt"),
        #[cfg(unix)]
        (r"dir\my file.txt", "dir/my file.txt"),
        #[cfg(unix)]
        ("a\tname.txt", "a name.txt"),
        #[cfg(unix)]
        ("a\"name.txt", "aname.txt"),
    ];
    for (path, decoy) in cases {
        let dir = init_repo();
        let repo_dir = dir.path();
        let rule = format!("{} encoding=koi8-r", pattern_for_path(Path::new(path)));
        fs::write(
            repo_dir.join(".gitattributes"),
            append_rule(b"# existing\n", &rule).unwrap(),
        )
        .unwrap();
        assert_eq!(check_attr(repo_dir, "encoding", path), "koi8-r", "{rule}");
        assert_eq!(
            check_attr(repo_dir, "encoding", decoy),
            "unspecified",
            "{rule} vs {decoy}"
        );
    }

    let dir = init_repo();
    let repo_dir = dir.path();
    let rule = format!(
        "{} working-tree-encoding=UTF-16LE-BOM",
        pattern_for_extension(Path::new("scripts/run.ps1")).unwrap()
    );
    fs::write(
        repo_dir.join(".gitattributes"),
        append_rule(b"", &rule).unwrap(),
    )
    .unwrap();
    assert_eq!(
        check_attr(repo_dir, "working-tree-encoding", "deep/other.ps1"),
        "UTF-16LE-BOM"
    );
}

#[test]
#[cfg(unix)]
fn attribute_patterns_with_line_breaks_do_not_add_rules_for_other_files() {
    use gitcomet_core::gitattributes::{append_rule, pattern_for_extension, pattern_for_path};
    test_git_env::ensure_initialized();
    for (path, extension, decoy) in [
        ("a\nx.txt encoding=KOI8-R\nz.txt", false, "x.txt"),
        ("a\r\nx.txt encoding=KOI8-R\r\nz.txt", false, "x.txt"),
        ("a.ext\nvictim encoding=KOI8-R\nz", true, "victim"),
    ] {
        let dir = init_repo();
        let pattern = if extension {
            pattern_for_extension(Path::new(path)).unwrap()
        } else {
            pattern_for_path(Path::new(path))
        };
        let rule = format!("{pattern} encoding=windows-1252");
        let contents = append_rule(b"", &rule).unwrap();
        assert_eq!(contents.iter().filter(|&&byte| byte == b'\n').count(), 1);
        fs::write(dir.path().join(".gitattributes"), contents).unwrap();
        assert_eq!(check_attr(dir.path(), "encoding", path), "windows-1252");
        assert_eq!(check_attr(dir.path(), "encoding", decoy), "unspecified");
    }
}

#[test]
fn latin1_merge_conflict_decodes_every_side_and_remembers_the_file_encoding() {
    use gitcomet_core::conflict_session::ConflictPayload;
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e\n",
    )
    .unwrap();
    commit_all(repo_dir, "base");
    git(repo_dir, &["checkout", "-q", "-b", "theirs"]);
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e pour deux\n",
    )
    .unwrap();
    commit_all(repo_dir, "theirs");
    git(repo_dir, &["checkout", "-q", "-"]);
    fs::write(
        repo_dir.join("menu.txt"),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e maison\n",
    )
    .unwrap();
    commit_all(repo_dir, "ours");
    let mut merge = Command::new("git");
    test_git_env::apply(&mut merge);
    let status = merge
        .arg("-C")
        .arg(repo_dir)
        .args(["merge", "-q", "theirs"])
        .output()
        .unwrap()
        .status;
    assert!(!status.success(), "the merge must conflict");

    let repo = open(repo_dir);
    let session = repo
        .conflict_session_with_encoding(Path::new("menu.txt"), None)
        .unwrap()
        .expect("conflict session");
    let text = |payload: &ConflictPayload| payload.as_text().expect("decoded text").to_string();
    assert_eq!(text(&session.ours), "Café crème brûlée maison\n");
    assert_eq!(text(&session.theirs), "Café crème brûlée pour deux\n");
    assert_eq!(text(&session.base), "Café crème brûlée\n");
    assert!(
        session.merge_plan.is_some(),
        "decoded stages still build a text merge plan"
    );
    assert_eq!(
        session.base_bytes().unwrap(),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e\n"
    );
    let current = session.current_format.expect("worktree file read");
    assert_eq!(current.format.encoding, TextEncoding::WINDOWS_1252);
    assert!(
        text(session.current.as_ref().unwrap()).contains("<<<<<<<"),
        "the working-tree file with markers decodes too"
    );
    git(repo_dir, &["config", "gui.encoding", "koi8-r"]);
    let refreshed = repo
        .conflict_session_with_encoding(Path::new("menu.txt"), None)
        .unwrap()
        .unwrap();
    let format = refreshed.current_format.unwrap();
    assert_eq!(format.source, FormatSource::GuiEncoding);
    assert_eq!(
        Some(format.format.encoding),
        TextEncoding::from_label("koi8-r")
    );
    assert_ne!(text(&refreshed.base), text(&session.base));
    assert_eq!(refreshed.base_bytes(), session.base_bytes());
}

#[test]
fn mixed_encoding_conflict_can_save_either_decoded_stage() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let root = dir.path();
    fs::write(root.join("menu.txt"), b"Caf\xe9 cr\xe8me br\xfbl\xe9e\n").unwrap();
    commit_all(root, "base");
    git(root, &["checkout", "-q", "-b", "theirs"]);
    fs::write(root.join("menu.txt"), "Café crème brûlée 日本語\n").unwrap();
    commit_all(root, "utf8");
    git(root, &["checkout", "-q", "-"]);
    fs::write(
        root.join("menu.txt"),
        b"Caf\xe9 cr\xe8me br\xfbl\xe9e maison\n",
    )
    .unwrap();
    commit_all(root, "latin1");
    let mut merge = Command::new("git");
    test_git_env::apply(&mut merge);
    assert!(
        !merge
            .arg("-C")
            .arg(root)
            .args(["merge", "-q", "theirs"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let repo = open(root);
    let session = repo
        .conflict_session_with_encoding(Path::new("menu.txt"), None)
        .unwrap()
        .unwrap();
    assert_eq!(session.theirs.as_text(), Some("Café crème brûlée 日本語\n"));
    let format = session.output_format.expect("output encoding");
    let current = session.current.as_ref().unwrap().as_text().unwrap();
    let (ours, theirs) =
        gitcomet_core::conflict_session::reconstruct_conflict_marker_sides(current);
    assert_eq!(Some(ours.as_str()), session.ours.as_text());
    assert_eq!(Some(theirs.as_str()), session.theirs.as_text());
    assert_eq!(
        session.current.as_ref().unwrap().as_bytes().unwrap(),
        fs::read(root.join("menu.txt")).unwrap()
    );
    assert!(format.is_writable());
    for side in [&session.ours, &session.theirs] {
        let text = side.as_text().unwrap();
        let encoded = gitcomet_core::text_format::encode(text, format.format)
            .expect("resolution can be saved");
        assert_eq!(
            gitcomet_core::text_format::decode(&encoded, format.format).text,
            text
        );
    }
}

#[test]
fn textconv_patch_decodes_its_output_without_reading_original_file_text() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let root = dir.path();
    fs::write(root.join(".gitattributes"), "*.txt diff=menu\n").unwrap();
    fs::write(root.join("menu.txt"), "old\n").unwrap();
    commit_all(root, "base");
    fs::write(root.join("menu.txt"), "new\n").unwrap();
    git(
        root,
        &["config", "diff.menu.textconv", "printf 'caf\\351\\n'; cat"],
    );
    let diff = patch(&*open(root), &unstaged("menu.txt"), None);
    assert!(diff.lines.iter().any(|line| line.text.as_ref() == " café"));
}

#[test]
fn exported_patch_keeps_latin1_bytes_and_applies() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(repo_dir.join("menu.txt"), b"Caf\xe9\n").unwrap();
    commit_all(repo_dir, "base");
    fs::write(repo_dir.join("menu.txt"), b"Caf\xe9 cr\xe8me\n").unwrap();
    let commit = commit_all(repo_dir, "latin1 change");

    let repo = open(repo_dir);
    let patch_path = repo_dir
        .join("..")
        .join(format!("gitcomet-latin1-{}.patch", std::process::id()));
    repo.export_patch_with_output(&commit, &patch_path).unwrap();
    let patch = fs::read(&patch_path).unwrap();
    assert!(
        patch
            .windows(b"+Caf\xe9 cr\xe8me".len())
            .any(|window| window == b"+Caf\xe9 cr\xe8me"),
        "the patch carries the file's own bytes"
    );
    git(repo_dir, &["reset", "-q", "--hard", "HEAD~1"]);
    git(repo_dir, &["apply", patch_path.to_str().unwrap()]);
    assert_eq!(
        fs::read(repo_dir.join("menu.txt")).unwrap(),
        b"Caf\xe9 cr\xe8me\n"
    );
    let _ = fs::remove_file(&patch_path);
}

#[test]
fn review_conflict_stages_share_legacy_detection_evidence() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let root = dir.path();
    for (branch, bytes) in [
        ("base", b"Caf\xe9 cr\xe8me br\xfbl\xe9e\n".as_slice()),
        ("theirs", b"\xf8\n".as_slice()),
        ("ours", b"Caf\xe9 cr\xe8me br\xfbl\xe9e maison\n".as_slice()),
    ] {
        if branch == "theirs" {
            git(root, &["checkout", "-q", "-b", "theirs"]);
        }
        if branch == "ours" {
            git(root, &["checkout", "-q", "-"]);
        }
        fs::write(root.join("menu.txt"), bytes).unwrap();
        commit_all(root, branch);
    }
    let mut merge = Command::new("git");
    test_git_env::apply(&mut merge);
    assert!(
        !merge
            .arg("-C")
            .arg(root)
            .args(["merge", "-q", "theirs"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let session = open(root)
        .conflict_session_with_encoding(Path::new("menu.txt"), None)
        .unwrap()
        .unwrap();
    assert_eq!(session.theirs.as_text(), Some("ø\n"));
    assert!(
        session.output_format.is_none(),
        "one legacy encoding must not trigger UTF-8 conversion"
    );
    assert_eq!(
        session.current_format.unwrap().format.encoding,
        TextEncoding::WINDOWS_1252
    );
}

/// A renamed file's old side reads from its source path, decoded by that
/// path's attributes; before rename-aware loading it had no old side at all.
#[test]
fn a_renamed_files_old_side_decodes_with_its_source_paths_encoding() {
    test_git_env::ensure_initialized();
    let dir = init_repo();
    let repo_dir = dir.path();
    fs::write(
        repo_dir.join(".gitattributes"),
        "*.sjis encoding=shift_jis\n",
    )
    .unwrap();
    // 日本 on each of enough lines that the edit keeps it a rename.
    let lines = b"\x93\xfa\x96\x7b\n".repeat(12);
    fs::write(repo_dir.join("old.sjis"), &lines).unwrap();
    commit_all(repo_dir, "base");
    git(repo_dir, &["mv", "old.sjis", "new.sjis"]);
    let mut edited = lines.clone();
    edited.extend_from_slice(b"\x93\xfa\n");
    fs::write(repo_dir.join("new.sjis"), &edited).unwrap();
    let renamed = commit_all(repo_dir, "rename");

    let repo = open(repo_dir);
    let details = repo.commit_details(&renamed).unwrap();
    let file = details
        .files
        .iter()
        .find(|file| file.path == Path::new("new.sjis"))
        .expect("the renamed file is listed");
    assert_eq!(file.old_path.as_deref(), Some(Path::new("old.sjis")));
    let target = DiffTarget::commit_change(renamed, file);
    let text = file_text(&*repo, &target, None);
    assert_eq!(read_side(text.old_source.as_ref()), "日本\n".repeat(12));
    assert_eq!(
        read_side(text.new_source.as_ref()),
        format!("{}日\n", "日本\n".repeat(12))
    );
    let format = text
        .old_source
        .and_then(|side| side.format)
        .expect("old format");
    assert_eq!(format.format.encoding.name(), "Shift_JIS");
}
