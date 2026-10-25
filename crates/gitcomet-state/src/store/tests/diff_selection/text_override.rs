use super::*;
use gitcomet_core::text_format::{TextAttributes, TextEncoding, TextOverride};

fn target(path: &str, area: gitcomet_core::domain::DiffArea) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), area)
}

fn selected(path: &str) -> (RepositoryRuntimes, AppState) {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target(path, gitcomet_core::domain::DiffArea::Unstaged),
        },
    );
    (repos, state)
}

fn set(
    repos: &mut RepositoryRuntimes,
    state: &mut AppState,
    path: &str,
    value: TextOverride,
) -> Vec<Effect> {
    reduce(
        repos,
        &AtomicU64::new(10),
        state,
        Msg::SetTextOverride {
            repo_id: RepoId(1),
            path: PathBuf::from(path),
            value,
        },
    )
}

fn koi8() -> TextOverride {
    TextOverride {
        encoding: TextEncoding::from_label("koi8-r"),
        ..TextOverride::default()
    }
}

#[test]
fn selecting_a_file_loads_its_attributes() {
    let (_, state) = selected("a.txt");
    assert!(state.repos[0].diff_state.text_attributes.is_loading());
}

#[test]
fn ordinary_watcher_batches_do_not_load_attributes_for_commit_views() {
    for (change, expected) in [
        (crate::msg::RepoExternalChange::Worktree, false),
        (crate::msg::RepoExternalChange::Index, false),
        (crate::msg::RepoExternalChange::GitState, false),
        (
            crate::msg::RepoExternalChange {
                text_attributes: true,
                ..Default::default()
            },
            true,
        ),
        (
            crate::msg::RepoExternalChange {
                verification_context: true,
                ..Default::default()
            },
            true,
        ),
    ] {
        let (mut repos, mut state) = selected("a.txt");
        state.repos[0].set_diff_target(Some(DiffTarget::commit(
            CommitId("abc123".into()),
            "a.txt".into(),
        )));
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(10),
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id: RepoId(1),
                change: change.clone(),
            },
        );
        assert_eq!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadSelectedDiff { .. })),
            expected,
            "{change:?}: {effects:?}"
        );
    }
}

#[test]
fn encoding_override_reloads_keeping_content_under_a_new_generation() {
    let (mut repos, mut state) = selected("a.txt");
    let target_rev = state.repos[0].diff_state.diff_target_rev;
    let effects = set(&mut repos, &mut state, "a.txt", koi8());
    let diff_state = &state.repos[0].diff_state;
    assert_eq!(diff_state.selected_encoding_override(), koi8().encoding);
    assert_eq!(diff_state.diff_target_rev, target_rev + 1);
    assert!(diff_state.diff_reload_in_flight);
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            load_patch_diff: true,
            load_file_text: true,
            ..
        }]
    ));
}

#[test]
fn tab_size_only_does_not_reload() {
    let (mut repos, mut state) = selected("a.txt");
    let override_rev = state.repos[0].diff_state.text_override_rev;
    let effects = set(
        &mut repos,
        &mut state,
        "a.txt",
        TextOverride {
            tab_size: Some(8),
            ..TextOverride::default()
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].diff_state.text_override_rev,
        override_rev + 1
    );
    assert_eq!(
        state.repos[0]
            .diff_state
            .text_override_for(Path::new("a.txt"))
            .and_then(|value| value.tab_size),
        Some(8)
    );
}

#[test]
fn override_for_a_file_that_is_not_open_is_ignored() {
    let (mut repos, mut state) = selected("a.txt");
    assert!(set(&mut repos, &mut state, "b.txt", koi8()).is_empty());
    assert!(state.repos[0].diff_state.text_override.is_none());
}

#[test]
fn override_survives_views_of_the_same_file_and_drops_on_another_file() {
    let (mut repos, mut state) = selected("a.txt");
    set(&mut repos, &mut state, "a.txt", koi8());
    let id_alloc = AtomicU64::new(20);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target("a.txt", gitcomet_core::domain::DiffArea::Staged),
        },
    );
    assert_eq!(
        state.repos[0].diff_state.selected_encoding_override(),
        koi8().encoding
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target("b.txt", gitcomet_core::domain::DiffArea::Unstaged),
        },
    );
    assert!(state.repos[0].diff_state.text_override.is_none());
}

#[test]
fn clearing_the_override_reloads_with_detection() {
    let (mut repos, mut state) = selected("a.txt");
    set(&mut repos, &mut state, "a.txt", koi8());
    let effects = set(&mut repos, &mut state, "a.txt", TextOverride::default());
    assert!(state.repos[0].diff_state.text_override.is_none());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff { .. }]
    ));
}

fn attributes_loaded(
    repos: &mut RepositoryRuntimes,
    state: &mut AppState,
    path: &str,
    attributes: TextAttributes,
) -> Vec<Effect> {
    reduce(
        repos,
        &AtomicU64::new(40),
        state,
        Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
            repo_id: RepoId(1),
            target: target(path, gitcomet_core::domain::DiffArea::Unstaged),
            result: Ok(attributes),
        }),
    )
}

fn gitattributes_rule_written(repos: &mut RepositoryRuntimes, state: &mut AppState) -> Vec<Effect> {
    reduce(
        repos,
        &AtomicU64::new(50),
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: crate::msg::RepoCommandKind::AppendGitattributesRule {
                rule: "/a.txt encoding=koi8-r".to_string(),
            },
            result: Ok(CommandOutput::empty_success("update .gitattributes")),
        }),
    )
}

#[test]
fn another_file_is_not_read_under_the_previous_files_attributes() {
    let (mut repos, mut state) = selected("a.txt");
    let utf16 = TextAttributes {
        working_tree_encoding: Some(gitcomet_core::text_format::EncodingAttr::from_label(
            "UTF-16LE-BOM",
        )),
        ..TextAttributes::default()
    };
    attributes_loaded(&mut repos, &mut state, "a.txt", utf16);
    assert!(matches!(
        state.repos[0].diff_state.text_attributes,
        Loadable::Ready(_)
    ));
    let attributes_rev = state.repos[0].diff_state.text_attributes_rev;
    reduce(
        &mut repos,
        &AtomicU64::new(60),
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target("b.txt", gitcomet_core::domain::DiffArea::Unstaged),
        },
    );
    let diff_state = &state.repos[0].diff_state;
    assert!(diff_state.text_attributes.is_loading());
    assert_ne!(diff_state.text_attributes_rev, attributes_rev);

    // The other view of the same file keeps what it has on screen.
    attributes_loaded(&mut repos, &mut state, "b.txt", TextAttributes::default());
    reduce(
        &mut repos,
        &AtomicU64::new(70),
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target("b.txt", gitcomet_core::domain::DiffArea::Staged),
        },
    );
    assert!(matches!(
        state.repos[0].diff_state.text_attributes,
        Loadable::Ready(_)
    ));
}

#[test]
fn a_written_gitattributes_rule_refreshes_attributes_and_keeps_the_choice() {
    let (mut repos, mut state) = selected("a.txt");
    attributes_loaded(&mut repos, &mut state, "a.txt", TextAttributes::default());
    set(&mut repos, &mut state, "a.txt", koi8());
    let effects = gitattributes_rule_written(&mut repos, &mut state);
    let diff_state = &state.repos[0].diff_state;
    assert_eq!(diff_state.selected_encoding_override(), koi8().encoding);
    assert!(matches!(diff_state.text_attributes, Loadable::Ready(_)));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedDiff {
            load_patch_diff: false,
            load_file_text: false,
            ..
        }
    )));
}

#[test]
fn a_written_gitattributes_rule_reloads_an_open_conflict_as_a_conflict() {
    let (mut repos, mut state) = selected("a.txt");
    state.repos[0].conflict_state.conflict_file_path = Some(PathBuf::from("a.txt"));
    attributes_loaded(&mut repos, &mut state, "a.txt", TextAttributes::default());
    set(&mut repos, &mut state, "a.txt", koi8());
    gitattributes_rule_written(&mut repos, &mut state);
    // Content is reloaded only after resolving the effective attributes.
    let effects = attributes_loaded(
        &mut repos,
        &mut state,
        "a.txt",
        TextAttributes {
            encoding: Some(gitcomet_core::text_format::EncodingAttr::from_label(
                "koi8-r",
            )),
            ..TextAttributes::default()
        },
    );
    assert_eq!(
        state.repos[0].diff_state.selected_encoding_override(),
        koi8().encoding
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadConflictFile { .. } | Effect::LoadSelectedConflictFile { .. }
        )),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadSelectedDiff { .. }))
    );
}

#[test]
fn only_decoding_changes_reload_content_and_replies_do_not_loop() {
    use gitcomet_core::text_format::{EncodingAttr, TabWidth, TabWidthSource};
    let (mut repos, mut state) = selected("a.txt");
    let original = TextAttributes {
        encoding: Some(EncodingAttr::from_label("windows-1252")),
        ..TextAttributes::default()
    };
    assert!(attributes_loaded(&mut repos, &mut state, "a.txt", original.clone()).is_empty());
    let mut metadata = original.clone();
    metadata.encoding = Some(EncodingAttr::from_label("cp1252"));
    metadata.tab_width = Some(TabWidth {
        columns: 8,
        source: TabWidthSource::Attribute,
    });
    assert!(attributes_loaded(&mut repos, &mut state, "a.txt", metadata).is_empty());

    for changed in [
        TextAttributes {
            encoding: Some(EncodingAttr::from_label("koi8-r")),
            ..original.clone()
        },
        TextAttributes {
            gui_encoding: Some(EncodingAttr::from_label("koi8-r")),
            ..original.clone()
        },
        TextAttributes {
            working_tree_encoding: Some(EncodingAttr::from_label("UTF-16LE")),
            ..original.clone()
        },
    ] {
        state.repos[0].diff_state.text_attributes = Loadable::Ready(Arc::new(original.clone()));
        let rev = state.repos[0].diff_state.diff_target_rev;
        let effects = attributes_loaded(&mut repos, &mut state, "a.txt", changed.clone());
        assert!(matches!(
            effects.as_slice(),
            [Effect::LoadSelectedDiff {
                load_patch_diff: true,
                load_file_text: true,
                ..
            }]
        ));
        assert_eq!(state.repos[0].diff_state.diff_target_rev, rev + 1);
        assert!(attributes_loaded(&mut repos, &mut state, "a.txt", changed).is_empty());
        assert_eq!(state.repos[0].diff_state.diff_target_rev, rev + 1);
    }
}

#[test]
fn attributes_for_the_open_file_are_stored_and_others_dropped() {
    let (mut repos, mut state) = selected("a.txt");
    let attributes = TextAttributes {
        diff_unset: true,
        ..TextAttributes::default()
    };
    let id_alloc = AtomicU64::new(30);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
            repo_id: RepoId(1),
            target: target("b.txt", gitcomet_core::domain::DiffArea::Unstaged),
            result: Ok(attributes.clone()),
        }),
    );
    assert!(state.repos[0].diff_state.text_attributes.is_loading());
    // The staged view of the same file answers for it too.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
            repo_id: RepoId(1),
            target: target("a.txt", gitcomet_core::domain::DiffArea::Staged),
            result: Ok(attributes.clone()),
        }),
    );
    assert!(matches!(
        &state.repos[0].diff_state.text_attributes,
        Loadable::Ready(loaded) if **loaded == attributes
    ));
}
