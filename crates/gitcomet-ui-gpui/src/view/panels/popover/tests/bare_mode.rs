use super::*;
use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, RepoSpec, RepositoryMode};
use gitcomet_core::large_files::LargeFileCommand;
use gitcomet_state::model::{AppState, Loadable, RepoState};
use std::path::PathBuf;

fn state(mode: RepositoryMode) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from(format!(
                "/tmp/gitcomet-bare-menus-{:?}",
                std::thread::current().id()
            )),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(mode);
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..AppState::test_default()
    })
}

fn entry(action: ContextMenuAction, disabled: bool) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: "Action".into(),
        icon: None,
        shortcut: None,
        disabled,
        action: Box::new(action),
    }
}

#[test]
fn bare_mode_changes_refresh_open_application_and_repository_menus() {
    let normal = state(RepositoryMode::WorkingTree);
    let mut bare = (*normal).clone();
    bare.repos[0].repository_mode = Some(RepositoryMode::Bare);
    for menu in [
        PopoverKind::AppMenu,
        PopoverKind::RepoTabMenu { repo_id: RepoId(1) },
        PopoverKind::DiffActionMenu,
    ] {
        assert_ne!(
            fingerprint::notify_fingerprint(&normal, &menu, None),
            fingerprint::notify_fingerprint(&bare, &menu, None),
        );
    }
}

#[gpui::test]
fn bare_context_menus_preserve_supported_actions_and_existing_disabled_reasons(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            crate::view::test_support::push_test_state(view, state(RepositoryMode::Bare), cx);
            view.popover_host.update(cx, |host, cx| {
                host.state = state(RepositoryMode::Bare);
                let model = host
                    .context_menu_model(
                        &PopoverKind::StashMenu {
                            repo_id: RepoId(1),
                            index: 0,
                            message: "saved".into(),
                        },
                        cx,
                    )
                    .unwrap();
                let mut guarded_entries = 0;
                for (ix, item) in model.items.iter().enumerate() {
                    if let ContextMenuItem::Entry {
                        action, disabled, ..
                    } = item
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::ApplyStash { .. }
                                | ContextMenuAction::PopStash { .. }
                                | ContextMenuAction::DropStashConfirm { .. }
                        )
                    {
                        guarded_entries += 1;
                        assert!(*disabled);
                        assert_eq!(
                            model.entry_tooltips.get(&ix).map(|text| text.as_ref()),
                            Some("Requires a working tree; this repository is bare.")
                        );
                    }
                }
                assert_eq!(guarded_entries, 3);
                let supported = ContextMenuAction::OpenWorktreeAddPrompt {
                    repo_id: RepoId(1),
                    prefill: WorktreeAddPrefill {
                        source: "HEAD".into(),
                        create_branch: false,
                        new_branch: String::new(),
                        open_after: false,
                    },
                };
                assert_eq!(host.bare_action_reason(&supported), None);
                let mut model = ContextMenuModel::new(vec![entry(supported, true)]);
                model
                    .entry_tooltips
                    .insert(0, "Wait for the running Git operation to finish".into());
                let model = host.gate_bare_menu(model);
                assert!(matches!(
                    &model.items[0],
                    ContextMenuItem::Entry { disabled: true, .. }
                ));
                assert_eq!(
                    model.entry_tooltips.get(&0).map(|text| text.as_ref()),
                    Some("Wait for the running Git operation to finish")
                );
            });
        })
    });
}

#[gpui::test]
fn bare_large_file_menu_matrix_keeps_force_and_all_versions_semantics(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            crate::view::test_support::push_test_state(view, state(RepositoryMode::Bare), cx);
            view.popover_host.update(cx, |host, _| {
                host.state = state(RepositoryMode::Bare);
                let path = vec![PathBuf::from("asset.bin")];
                let cases = vec![
                    (
                        LargeFileCommand::LfsTrack {
                            patterns: vec!["*.bin".into()],
                            filename: false,
                            lockable: false,
                            renormalize: Vec::new(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsFetchForDiff {
                            target: DiffTarget::commit_range(CommitId("old".into()), None, None),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsFetchForDiff {
                            target: DiffTarget::commit_range(
                                CommitId("old".into()),
                                Some(CommitId("new".into())),
                                None,
                            ),
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexGetKeys {
                            keys: vec!["SHA256E-s1--test.bin".into()],
                        },
                        true,
                        false,
                    ),
                    (LargeFileCommand::AnnexInit, true, false),
                    (
                        LargeFileCommand::AnnexEnableRemote {
                            name: "archive".into(),
                            params: Vec::new(),
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexInitRemote {
                            name: "archive".into(),
                            special_type: "directory".into(),
                            params: Vec::new(),
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexTrust {
                            repository: "archive".into(),
                            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexDescribe {
                            repository: "here".into(),
                            description: "Archive".into(),
                        },
                        true,
                        false,
                    ),
                    (LargeFileCommand::AnnexNumcopies { copies: 2 }, true, false),
                    (
                        LargeFileCommand::AnnexDropUnused {
                            unused: Arc::new(Default::default()),
                            force: false,
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexAdd {
                            paths: path.clone(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexLock {
                            paths: path.clone(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexUnlock {
                            paths: path.clone(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexAdjust {
                            mode: gitcomet_core::large_files::AnnexAdjustMode::Unlock,
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexLeaveAdjusted {
                            base: "main".into(),
                        },
                        false,
                        false,
                    ),
                    (LargeFileCommand::AnnexWebapp, false, false),
                    (LargeFileCommand::AnnexStopAssistant, false, false),
                    (
                        LargeFileCommand::LfsPull { paths: Vec::new() },
                        false,
                        false,
                    ),
                    (LargeFileCommand::LfsFetchAll, true, false),
                    (
                        LargeFileCommand::LfsPushAll {
                            remote: "origin".into(),
                        },
                        true,
                        false,
                    ),
                    (LargeFileCommand::LfsPrune, true, false),
                    (LargeFileCommand::LfsFsck, true, false),
                    (LargeFileCommand::LfsInstall, true, false),
                    (
                        LargeFileCommand::LfsLock {
                            paths: path.clone(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsUnlock {
                            paths: path.clone(),
                            force: false,
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsUnlock {
                            paths: path.clone(),
                            force: true,
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsFetchForDiff {
                            target: DiffTarget::working_tree(path[0].clone(), DiffArea::Unstaged),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::LfsFetchForDiff {
                            target: DiffTarget::commit(CommitId("abc123".into()), path[0].clone()),
                        },
                        true,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexGet {
                            paths: Vec::new(),
                            from: None,
                        },
                        true,
                        true,
                    ),
                    (
                        LargeFileCommand::AnnexGet {
                            paths: path.clone(),
                            from: None,
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexDrop {
                            paths: Vec::new(),
                            from: None,
                            force: false,
                        },
                        true,
                        true,
                    ),
                    (
                        LargeFileCommand::AnnexDrop {
                            paths: path.clone(),
                            from: None,
                            force: true,
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexCopy {
                            paths: Vec::new(),
                            to: "archive".into(),
                        },
                        true,
                        true,
                    ),
                    (
                        LargeFileCommand::AnnexCopy {
                            paths: path.clone(),
                            to: "archive".into(),
                        },
                        false,
                        false,
                    ),
                    (
                        LargeFileCommand::AnnexMove {
                            paths: Vec::new(),
                            to: "archive".into(),
                        },
                        true,
                        true,
                    ),
                    (
                        LargeFileCommand::AnnexMove {
                            paths: path,
                            to: "archive".into(),
                        },
                        false,
                        false,
                    ),
                    (LargeFileCommand::AnnexPull { content: false }, true, false),
                    (LargeFileCommand::AnnexPull { content: true }, true, false),
                    (LargeFileCommand::AnnexPush { content: true }, true, false),
                    (LargeFileCommand::AnnexSync { content: true }, true, false),
                    (LargeFileCommand::AnnexFsck, true, false),
                    (LargeFileCommand::AnnexRestage, true, false),
                ];
                for (command, allowed, all_versions) in cases {
                    let action = ContextMenuAction::RunLargeFileCommand {
                        repo_id: RepoId(1),
                        command: command.clone(),
                    };
                    let model =
                        host.gate_bare_menu(ContextMenuModel::new(vec![entry(action, false)]));
                    let ContextMenuItem::Entry {
                        disabled, label, ..
                    } = &model.items[0]
                    else {
                        panic!("action entry")
                    };
                    assert_eq!(!*disabled, allowed, "{command:?}");
                    assert_eq!(
                        label.as_ref(),
                        if all_versions {
                            "Action (all versions)"
                        } else {
                            "Action"
                        },
                        "{command:?}"
                    );
                    assert_eq!(
                        model.entry_tooltips.get(&0).map(|text| text.as_ref()),
                        (!allowed).then_some("Requires a working tree; this repository is bare."),
                        "{command:?}"
                    );
                }
            });
        })
    });
}

#[gpui::test]
fn a_stale_checkout_menu_and_keyboard_prompt_cannot_start_checkout_in_bare_mode(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            crate::view::test_support::push_test_state(view, state(RepositoryMode::WorkingTree), cx)
        })
    });
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::Checkout,
                    }
                    .invoked_by("branch_badge".into()),
                    point(px(100.0), px(70.0)),
                    window,
                    cx,
                )
            });
            assert!(view.popover_host.read(cx).is_open());
            crate::view::test_support::push_test_state(view, state(RepositoryMode::Bare), cx);
        })
    });
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            assert!(!view.popover_host.read(cx).is_open());
            view.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StashPrompt.invoked_by("shortcut".into()),
                    point(px(100.0), px(70.0)),
                    window,
                    cx,
                );
                assert!(!host.is_open());
            });
        })
    });
}
