use super::*;

fn copied_path_ends_with(text: &str, suffix: &std::path::Path) -> bool {
    let normalize = |value: &str| value.replace('\\', "/");
    normalize(text).ends_with(&normalize(&suffix.to_string_lossy()))
}

fn declared_shortcuts(model: &ContextMenuModel) -> Vec<String> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { shortcut, .. } => shortcut.as_ref().map(|s| s.to_string()),
            _ => None,
        })
        .collect()
}

fn assert_declared_shortcuts(model: &ContextMenuModel, expected: &[&str]) {
    let expected = expected.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(declared_shortcuts(model), expected);
}

fn shortcut_entry<'a>(
    model: &'a ContextMenuModel,
    shortcut: &str,
) -> (&'a ContextMenuAction, usize) {
    if shortcut == "Enter" {
        let ix = runtime_entry_ix_for_shortcut(model, shortcut)
            .unwrap_or_else(|| panic!("expected shortcut `{shortcut}` to resolve at runtime"));
        return match model.items.get(ix) {
            Some(ContextMenuItem::Entry { action, .. }) => (action.as_ref(), ix),
            _ => panic!("expected runtime shortcut `{shortcut}` to target an entry"),
        };
    }

    model
        .items
        .iter()
        .enumerate()
        .find_map(|(ix, item)| match item {
            ContextMenuItem::Entry {
                shortcut: Some(entry_shortcut),
                action,
                ..
            } if entry_shortcut.as_ref() == shortcut => Some((action.as_ref(), ix)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected shortcut `{shortcut}` to exist"))
}

fn runtime_entry_ix_for_shortcut(model: &ContextMenuModel, shortcut: &str) -> Option<usize> {
    match shortcut {
        "Enter" => super::super::popover::context_menu::context_menu_activate_entry_ix(model, None),
        _ if shortcut.chars().count() == 1 => {
            let key = shortcut.to_ascii_lowercase();
            super::super::popover::context_menu::context_menu_shortcut_entry_ix(model, &key)
        }
        _ => None,
    }
}

macro_rules! assert_shortcut_action {
    ($model:expr, $shortcut:expr, $pat:pat $(if $guard:expr)? ) => {{
        let (action, expected_ix) = shortcut_entry(&$model, $shortcut);
        if let Some(runtime_ix) = runtime_entry_ix_for_shortcut(&$model, $shortcut) {
            assert_eq!(
                runtime_ix, expected_ix,
                "expected runtime resolution for `{}` to target entry {}",
                $shortcut, expected_ix
            );
        }
        assert!(
            matches!(action, $pat $(if $guard)?),
            "unexpected action for shortcut `{}`",
            $shortcut,
        );
    }};
}

fn context_menu_model_for(
    view: &gpui::Entity<super::super::GitCometView>,
    app: &mut gpui::App,
    kind: PopoverKind,
) -> ContextMenuModel {
    view.update(app, |this, cx| {
        this.popover_host.update(cx, |host, cx| {
            host.context_menu_model(&kind, cx)
                .unwrap_or_else(|| panic!("expected context menu model for {kind:?}"))
        })
    })
}

fn apply_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    state: Arc<AppState>,
) {
    cx.update(|window, app| {
        let state_for_host = Arc::clone(&state);
        view.update(app, |this, cx| {
            push_test_state(this, state, cx);
            this.popover_host.update(cx, |host, _cx| {
                host.set_state_for_test(state_for_host);
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn app_state_with_active_repo(repo: RepoState) -> Arc<AppState> {
    let repo_id = repo.id;
    Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    })
}

fn shortcut_fixture_repo(
    repo_id: RepoId,
    workdir: &std::path::Path,
    commit_id: &CommitId,
) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
    repo.log = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id: commit_id.clone(),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "Initial commit".into(),
                author: "Alice".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        }
        .into(),
    );
    repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
        name: "origin".into(),
        url: Some("https://example.com/origin.git".into()),
    }]));
    repo.tags = Loadable::Ready(Arc::new(vec![]));
    repo.remote_tags = Loadable::Ready(Arc::new(vec![]));
    repo.stashes = Loadable::Ready(Arc::new(vec![]));
    repo
}

fn simple_hunk_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target,
        lines: vec![
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "diff --git a/src/lib.rs b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "--- a/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "+++ b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-old".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: "+new".into(),
            },
        ],
    }
}

#[gpui::test]
fn history_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(700);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_settings_history_shortcuts",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let history_filter_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::HistoryBranchFilter { repo_id })
    });
    assert_declared_shortcuts(&history_filter_model, &["C", "A"]);
    assert_shortcut_action!(
        history_filter_model,
        "C",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::LogScope::CurrentBranch
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "A",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::LogScope::AllBranches
        } if *rid == repo_id
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.history_view.update(cx, |history, cx| {
                    history.history_show_author = true;
                    history.history_show_date = true;
                    history.history_show_sha = true;
                    cx.notify();
                });
            });
        });
    });

    let history_columns_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::HistoryColumnSettings)
    });
    assert_declared_shortcuts(&history_columns_model, &["A", "D", "S", "R"]);
    assert_shortcut_action!(
        history_columns_model,
        "A",
        ContextMenuAction::SetHistoryColumns {
            show_author,
            show_date,
            show_sha
        } if !*show_author && *show_date && *show_sha
    );
    assert_shortcut_action!(
        history_columns_model,
        "D",
        ContextMenuAction::SetHistoryColumns {
            show_author,
            show_date,
            show_sha
        } if *show_author && !*show_date && *show_sha
    );
    assert_shortcut_action!(
        history_columns_model,
        "S",
        ContextMenuAction::SetHistoryColumns {
            show_author,
            show_date,
            show_sha
        } if *show_author && *show_date && !*show_sha
    );
    assert_shortcut_action!(
        history_columns_model,
        "R",
        ContextMenuAction::ResetHistoryColumnWidths
    );

    let change_tracking_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::ChangeTrackingSettings)
    });
    assert_declared_shortcuts(&change_tracking_model, &["C", "S"]);
    assert_shortcut_action!(
        change_tracking_model,
        "C",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::Combined
        }
    );
    assert_shortcut_action!(
        change_tracking_model,
        "S",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::SplitUntracked
        }
    );
}

#[gpui::test]
fn repo_operation_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(701);
    let commit_id = CommitId("feedfacefeedface".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_repo_shortcuts",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let pull_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PullPicker));
    assert_declared_shortcuts(&pull_model, &["F", "O", "R", "A"]);
    assert_shortcut_action!(
        pull_model,
        "Enter",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Default
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "F",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardIfPossible
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "O",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardOnly
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "R",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Rebase
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "A",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let push_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PushPicker));
    assert_declared_shortcuts(&push_model, &["F"]);
    assert_shortcut_action!(
        push_model,
        "Enter",
        ContextMenuAction::Push { repo_id: rid } if *rid == repo_id
    );
    assert_shortcut_action!(
        push_model,
        "F",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::ForcePushConfirm { repo_id: rid }
        } if *rid == repo_id
    );

    let branch_section_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchSectionMenu {
                repo_id,
                section: BranchSection::Remote,
            },
        )
    });
    assert_declared_shortcuts(&branch_section_model, &["F"]);
    assert_shortcut_action!(
        branch_section_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::BranchPicker
        }
    );
    assert_shortcut_action!(
        branch_section_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let local_branch_name = "feature".to_string();
    let local_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: local_branch_name.clone(),
            },
        )
    });
    assert_declared_shortcuts(&local_branch_model, &["P", "M", "S"]);
    assert_shortcut_action!(
        local_branch_model,
        "Enter",
        ContextMenuAction::CheckoutBranch { repo_id: rid, name } if *rid == repo_id && name == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "P",
        ContextMenuAction::PullBranch {
            repo_id: rid,
            remote,
            branch
        } if *rid == repo_id && remote == "." && branch == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "M",
        ContextMenuAction::MergeRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "S",
        ContextMenuAction::SquashRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "feature"
    );

    let remote_branch_name = "origin/feature".to_string();
    let remote_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Remote,
                name: remote_branch_name.clone(),
            },
        )
    });
    assert_declared_shortcuts(&remote_branch_model, &["P", "M", "S", "F"]);
    assert_shortcut_action!(
        remote_branch_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id: rid,
                remote,
                branch
            }
        } if *rid == repo_id && remote == "origin" && branch == "feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "P",
        ContextMenuAction::PullBranch {
            repo_id: rid,
            remote,
            branch
        } if *rid == repo_id && remote == "origin" && branch == "feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "M",
        ContextMenuAction::MergeRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "origin/feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "S",
        ContextMenuAction::SquashRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "origin/feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let remote_menu_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::remote(
                repo_id,
                RemotePopoverKind::Menu {
                    name: "origin".into(),
                },
            ),
        )
    });
    assert_declared_shortcuts(&remote_menu_model, &["F"]);
    assert_shortcut_action!(
        remote_menu_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let stash_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StashMenu {
                repo_id,
                index: 3,
                message: "WIP".into(),
            },
        )
    });
    assert_declared_shortcuts(&stash_model, &["A", "P"]);
    assert_shortcut_action!(
        stash_model,
        "A",
        ContextMenuAction::ApplyStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
    assert_shortcut_action!(
        stash_model,
        "P",
        ContextMenuAction::PopStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
}

#[gpui::test]
fn file_and_diff_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(702);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_shortcuts",
        std::process::id()
    ));
    let commit_file_path = std::path::PathBuf::from("src/main.rs");
    let unstaged_path = std::path::PathBuf::from("unstaged.rs");
    let staged_path = std::path::PathBuf::from("staged_added.rs");
    let conflicted_path = std::path::PathBuf::from("conflicted.rs");
    let hunk_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![gitcomet_core::domain::FileStatus {
                path: staged_path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Added,
                conflict: None,
            }],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: unstaged_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: hunk_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: conflicted_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                    conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: hunk_path.clone(),
        area: DiffArea::Unstaged,
    });
    repo.diff_state.diff = Loadable::Ready(
        simple_hunk_diff(DiffTarget::WorkingTree {
            path: hunk_path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let commit_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitMenu {
                repo_id,
                commit_id: commit_id.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_model, &["T", "D", "P", "R"]);
    assert_shortcut_action!(
        commit_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::Commit {
                commit_id: cid,
                path: None
            }
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "T",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateTagPrompt { repo_id: rid, target }
        } if *rid == repo_id && target == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "D",
        ContextMenuAction::CheckoutCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "P",
        ContextMenuAction::CherryPickCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "R",
        ContextMenuAction::RevertCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );

    let commit_file_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: commit_id.clone(),
                path: commit_file_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_file_model, &["H", "C"]);
    assert_shortcut_action!(
        commit_file_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::Commit {
                commit_id: cid,
                path: Some(path)
            }
        } if *rid == repo_id && cid == &commit_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &commit_file_path)
    );

    let unstaged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: unstaged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&unstaged_status_model, &["H", "S", "D", "C"]);
    assert_shortcut_action!(
        unstaged_status_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::WorkingTree { path, area }
        } if *rid == repo_id && path == &unstaged_path && *area == DiffArea::Unstaged
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "S",
        ContextMenuAction::StageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &unstaged_path)
    );

    let staged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Staged,
                path: staged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&staged_status_model, &["H", "U", "D", "C"]);
    assert_shortcut_action!(
        staged_status_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::WorkingTree { path, area }
        } if *rid == repo_id && path == &staged_path && *area == DiffArea::Staged
    );
    assert_shortcut_action!(
        staged_status_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "U",
        ContextMenuAction::UnstageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &staged_path)
    );

    let conflicted_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: conflicted_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&conflicted_status_model, &["H", "O", "T", "M", "D", "C"]);
    assert_shortcut_action!(
        conflicted_status_model,
        "Enter",
        ContextMenuAction::SelectConflictDiff {
            repo_id: rid,
            path
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "O",
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Ours
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "T",
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Theirs
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "M",
        ContextMenuAction::SelectConflictDiff {
            repo_id: rid,
            path
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &conflicted_path)
    );

    let diff_editor_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: Some(unstaged_path.clone()),
                hunk_patch: Some("hunk patch".into()),
                hunks_count: 2,
                lines_patch: Some("line patch".into()),
                discard_lines_patch: Some("discard patch".into()),
                lines_count: 3,
                copy_text: Some("copied selection".into()),
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_unstaged_model, &["S", "D", "C"]);
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "S",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "line patch" && !*reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "D",
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "discard patch" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "C",
        ContextMenuAction::CopyText { text } if text == "copied selection"
    );

    let diff_editor_staged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Staged,
                path: Some(staged_path.clone()),
                hunk_patch: Some("staged hunk".into()),
                hunks_count: 1,
                lines_patch: Some("staged line".into()),
                discard_lines_patch: None,
                lines_count: 1,
                copy_text: Some("staged copy".into()),
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_staged_model, &["U", "C"]);
    assert_shortcut_action!(
        diff_editor_staged_model,
        "U",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "staged line" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_staged_model,
        "C",
        ContextMenuAction::CopyText { text } if text == "staged copy"
    );

    let diff_hunk_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::DiffHunkMenu { repo_id, src_ix: 3 })
    });
    assert_declared_shortcuts(&diff_hunk_unstaged_model, &["S", "D"]);
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "S",
        ContextMenuAction::StageHunk {
            repo_id: rid,
            src_ix
        } if *rid == repo_id && *src_ix == 3
    );
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "D",
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && !patch.is_empty() && *reverse
    );

    let conflict_output_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: 12,
                selected_text: Some("chosen text".into()),
                has_source_a: true,
                has_source_b: true,
                has_source_c: true,
                is_three_way: true,
            },
        )
    });
    assert_declared_shortcuts(&conflict_output_model, &["Ctrl+C", "Ctrl+X", "Ctrl+V"]);
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+C",
        ContextMenuAction::CopyText { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+X",
        ContextMenuAction::ConflictResolverOutputCut { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+V",
        ContextMenuAction::ConflictResolverOutputPaste
    );
}
