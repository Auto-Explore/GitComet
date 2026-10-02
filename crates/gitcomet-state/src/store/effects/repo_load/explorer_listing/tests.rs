use super::*;
use crate::model::{AppState, Loadable, RepoId, RepoState};
use crate::msg::{Effect, InternalMsg, Msg};
use gitcomet_core::domain::{FileSource, RepoSpec};
use std::sync::atomic::AtomicU64;

const REPO: RepoId = RepoId(1);

fn dispatch(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    crate::store::reducer::reduce(&mut Default::default(), &AtomicU64::new(2), state, msg)
}

fn fixture(ignored: bool) -> (tempfile::TempDir, AppState) {
    let directory = tempfile::tempdir().unwrap();
    let root = gitcomet_core::path_utils::canonicalize_or_original(directory.path().into());
    for path in [
        "ignored/nested/deeper/file.txt",
        "ignored_extra/nested/sibling.txt",
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "contents").unwrap();
    }
    let mut state = AppState::test_default();
    let mut repo = RepoState::new_opening(REPO, RepoSpec { workdir: root });
    repo.open = Loadable::Ready(());
    repo.file_browser.show_ignored = ignored;
    state.repos.push(repo);
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(list(&state)));
    (directory, state)
}

fn list(state: &AppState) -> Vec<FileEntry> {
    let repo = &state.repos[0];
    // An empty backend listing models these fixture paths being ignored.
    augment(
        &repo.spec.workdir,
        vec![],
        Options::from(&repo.file_browser),
        &CancellationToken::new(),
    )
    .unwrap()
}

fn finish(state: &mut AppState, entries: Vec<FileEntry>) -> Vec<Effect> {
    dispatch(
        state,
        Msg::Internal(InternalMsg::FileBrowserLoaded {
            cancellation: None,
            repo_id: REPO,
            source: FileSource::WorkingDirectory,
            result: Ok(entries),
        }),
    )
}

fn assert_load(effects: &[Effect]) {
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser {
                repo_id: REPO,
                source: FileSource::WorkingDirectory,
            }
        )),
        "{effects:?}"
    );
}

fn expand(state: &mut AppState) -> Vec<Effect> {
    dispatch(
        state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id: REPO,
            path: "ignored".into(),
            expanded: true,
        },
    )
}

#[test]
fn recursive_expansion_loads_and_expands_ignored_descendants_only_under_its_root() {
    check_recursive_expansion(false);
}

#[test]
fn recursive_expansion_survives_an_older_listing_and_uses_the_queued_load() {
    check_recursive_expansion(true);
}

fn check_recursive_expansion(earlier_load: bool) {
    let (_directory, mut state) = fixture(true);
    let initial = list(&state);
    assert_eq!(initial.len(), 2, "collapsed ignored directories stay lazy");
    if earlier_load {
        assert_load(&dispatch(
            &mut state,
            Msg::LoadFileBrowser {
                repo_id: REPO,
                source: FileSource::WorkingDirectory,
            },
        ));
    }
    let effects = expand(&mut state);
    if earlier_load {
        assert!(effects.is_empty(), "the active walk coalesces this request");
        assert_load(&finish(&mut state, initial));
        assert!(
            !state.repos[0]
                .file_browser
                .pending_recursive_expansions
                .is_empty()
        );
    } else {
        assert_load(&effects);
    }
    let entries = list(&state);
    assert!(
        entries
            .iter()
            .any(|e| e.path.as_ref() == Path::new("ignored/nested/deeper/file.txt"))
    );
    assert!(
        !entries
            .iter()
            .any(|e| e.path.starts_with("ignored_extra/nested"))
    );
    assert!(finish(&mut state, entries).is_empty());
    let browser = &state.repos[0].file_browser;
    assert_eq!(
        browser.expanded_dirs,
        ["ignored", "ignored/nested", "ignored/nested/deeper"]
            .into_iter()
            .map(|p| Arc::new(PathBuf::from(p)))
            .collect()
    );
    assert!(browser.pending_recursive_expansions.is_empty());
}

#[test]
fn collapsing_during_a_recursive_load_does_not_reopen_the_subtree() {
    for recursive in [false, true] {
        let (_directory, mut state) = fixture(true);
        assert_load(&expand(&mut state));
        let in_flight = list(&state);
        dispatch(
            &mut state,
            if recursive {
                Msg::SetFileBrowserDirExpandedRecursive {
                    repo_id: REPO,
                    path: "ignored".into(),
                    expanded: false,
                }
            } else {
                Msg::ToggleFileBrowserDir {
                    repo_id: REPO,
                    path: "ignored".into(),
                }
            },
        );
        let effects = finish(&mut state, in_flight);
        if !effects.is_empty() {
            assert_load(&effects);
            let entries = list(&state);
            assert!(finish(&mut state, entries).is_empty());
        }
        let browser = &state.repos[0].file_browser;
        assert!(browser.expanded_dirs.is_empty());
        assert!(browser.pending_recursive_expansions.is_empty());
    }
}

#[test]
fn explicitly_revealed_ignored_files_remain_listed_after_file_or_parent_renames() {
    use gitcomet_core::filesystem::{Filesystem, Operation, Request};
    for parent in [false, true] {
        let (_directory, mut state) = fixture(false);
        let old = Path::new("ignored/nested/deeper/file.txt");
        assert!(list(&state).is_empty());
        assert_load(&dispatch(
            &mut state,
            Msg::RevealFileBrowserPath {
                repo_id: REPO,
                path: old.into(),
            },
        ));
        let entries = list(&state);
        assert!(entries.iter().any(|e| e.path.as_ref() == old));
        finish(&mut state, entries);
        let root = &state.repos[0].spec.workdir;
        let result = Filesystem::default().execute(
            Request::new(Operation::Rename {
                source: root.join(if parent { Path::new("ignored") } else { old }),
                name: if parent { "renamed" } else { "new.txt" }.into(),
            }),
            |_| {},
        );
        assert!(result.succeeded(), "{:?}", result.items);
        assert_load(&dispatch(
            &mut state,
            Msg::FilesystemPathsChanged(result.changes),
        ));
        let new = Path::new(if parent {
            "renamed/nested/deeper/file.txt"
        } else {
            "ignored/nested/deeper/new.txt"
        });
        assert_eq!(
            state.repos[0].file_browser.revealed_paths,
            [new.to_path_buf()].into_iter().collect()
        );
        let entries = list(&state);
        assert!(entries.iter().any(|e| e.path.as_ref() == new));
        assert!(!entries.iter().any(|e| e.path.as_ref() == old));
        assert!(!entries.iter().any(|e| e.path.starts_with("ignored_extra")));
        finish(&mut state, entries);
    }
}

#[test]
fn augment_flags_only_entries_missing_from_the_backend_listing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for path in ["src/main.rs", "src/gen.rs", "target/out.bin", "notes.txt"] {
        std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        std::fs::write(root.join(path), "contents").unwrap();
    }
    let entry = |path: &str, kind, depth| FileEntry {
        name: Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        path: Arc::new(path.into()),
        kind,
        depth,
        ignored: false,
    };
    let base = vec![
        entry("src", FileEntryKind::Directory, 0),
        entry("src/main.rs", FileEntryKind::File, 1),
    ];
    let options = Options {
        ignored: true,
        ..Options::default()
    };
    let flags: Vec<_> = augment(root, base, options, &CancellationToken::new())
        .unwrap()
        .into_iter()
        .map(|e| ((*e.path).clone(), e.ignored))
        .collect();
    let expected = [
        ("src", false),
        ("src/gen.rs", true),
        ("src/main.rs", false),
        ("target", true),
        ("notes.txt", true),
    ];
    // Compared as paths: walked entries use the native separator (`src\gen.rs`
    // on Windows) while backend rows use `/`.
    assert_eq!(
        flags,
        expected.map(|(path, ignored)| (PathBuf::from(path), ignored))
    );
}

#[test]
fn augment_drops_service_owned_entries_from_the_backend_listing() {
    let entry = |path: &str, kind| FileEntry {
        name: Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        path: Arc::new(PathBuf::from(path)),
        kind,
        depth: Path::new(path).components().count() - 1,
        ignored: false,
    };
    let base = vec![
        entry(".gitcomet-operation-abc", FileEntryKind::Directory),
        entry(".gitcomet-operation-abc/item", FileEntryKind::File),
        entry("src", FileEntryKind::Directory),
        entry("src/.gitcomet-save-xyz", FileEntryKind::File),
        entry("src/main.rs", FileEntryKind::File),
    ];
    let directory = tempfile::tempdir().unwrap();
    let paths = |entries: Vec<FileEntry>| -> Vec<PathBuf> {
        entries.into_iter().map(|e| (*e.path).clone()).collect()
    };
    // The early return (nothing ignored or revealed) filters too.
    let listed = augment(
        directory.path(),
        base.clone(),
        Options::default(),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        paths(listed),
        [PathBuf::from("src"), PathBuf::from("src/main.rs")]
    );
    let listed = augment(
        directory.path(),
        base,
        Options {
            ignored: true,
            ..Options::default()
        },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        paths(listed),
        [PathBuf::from("src"), PathBuf::from("src/main.rs")]
    );
}
