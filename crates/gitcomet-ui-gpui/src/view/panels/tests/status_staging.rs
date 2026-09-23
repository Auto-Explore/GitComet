use super::*;
use gitcomet_core::domain::{DiffArea, DiffTarget, FileSource};
use gitcomet_core::services::GitBackend;
use std::path::PathBuf;

const REPO: RepoId = RepoId(791);
type View = Entity<GitCometView>;

#[derive(Clone, Copy, Debug)]
enum Surface {
    All,
    Row,
    RowUnrelatedSelection,
    Selected,
    UntrackedAll,
    TrackedAll,
    Confirmation,
    ContextMenu,
    CommandPalette,
    Folder,
    Shortcut(&'static str),
    PaletteRename,
    DiscardSelected,
    DiscardPath,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Failure {
    None,
    IndexLocked,
    GitUnavailable,
}

#[derive(Clone, Copy, Debug)]
enum Viewed {
    Diff(DiffArea),
    Content,
    Editor,
}

fn git(dir: &Path, args: &[&str]) {
    git_output(dir, args);
}

fn git_output(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn commit(dir: &Path, message: &str) {
    git(
        dir,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qam",
            message,
        ],
    );
}

fn wait_for(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    store: &AppStore,
    description: &str,
    ready: impl Fn(&RepoState) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let snapshot = store.snapshot();
        let is_ready = ready(&snapshot.repos[0]);
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::push_test_state(this, Arc::clone(&snapshot), cx)
            });
        });
        draw_and_drain_test_window(cx);
        if is_ready {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out: {description}; target={:?}, preview={}, edit={}, git={:?}",
            snapshot.repos[0].diff_state.diff_target,
            snapshot.repos[0].diff_state.content_preview,
            snapshot.repos[0].diff_state.edit_mode,
            snapshot.git_runtime
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn click(cx: &mut gpui::VisualTestContext, selector: &str) {
    let selector = Box::leak(selector.to_string().into_boxed_str());
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
}

fn content_loaded(repo: &RepoState) -> bool {
    matches!(repo.diff_state.diff, Loadable::Ready(_))
        || matches!(repo.diff_state.diff_file, Loadable::Ready(_))
        || matches!(repo.diff_state.diff_preview_text_file, Loadable::Ready(_))
}

fn wait_for_displayed_content(cx: &mut gpui::VisualTestContext, view: &View) {
    super::shortcuts::wait_until(cx, "displayed file content", |cx| {
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx);
            });
        });
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.read(app);
            let Some(repo) = pane.active_repo() else {
                return false;
            };
            if repo.diff_state.edit_mode {
                !pane.file_editor_loading && pane.file_editor_key.is_some()
            } else {
                // Untracked files and content views read directly from disk in
                // the pane; their store diff loadables deliberately stay empty.
                content_loaded(repo) || matches!(pane.worktree_preview, Loadable::Ready(_))
            }
        })
    });
}

/// Real index mutations and their reloads must both finish before checking the
/// view. A missing repository handle would only test the dispatch's first half.
fn exercise(
    cx: &mut gpui::TestAppContext,
    area: DiffArea,
    surface: Surface,
    viewed: Viewed,
    path: &str,
    closes: bool,
) {
    exercise_with_failure(cx, area, surface, viewed, path, closes, Failure::None);
}

fn exercise_with_failure(
    cx: &mut gpui::TestAppContext,
    area: DiffArea,
    surface: Surface,
    viewed: Viewed,
    path: &str,
    closes: bool,
    failure: Failure,
) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let tracked = ["a.txt", "b.txt", "nested/c.txt", "nested/d.txt"];
    for path in tracked {
        std::fs::write(dir.path().join(path), "base\n").unwrap();
    }
    // Long enough that the untouched rename below is detected as one entry.
    let moved: String = (0..20).map(|line| format!("line {line}\n")).collect();
    std::fs::write(dir.path().join("moved.txt"), moved).unwrap();
    git(dir.path(), &["add", "."]);
    commit(dir.path(), "base");
    for path in tracked {
        std::fs::write(dir.path().join(path), "base\nstaged\n").unwrap();
    }
    git(dir.path(), &["add", "."]);
    for path in tracked {
        std::fs::write(dir.path().join(path), "base\nstaged\nunstaged\n").unwrap();
    }
    std::fs::write(dir.path().join("new.txt"), "new\n").unwrap();
    if matches!(surface, Surface::PaletteRename) {
        git(dir.path(), &["mv", "moved.txt", "renamed.txt"]);
    }
    if matches!(surface, Surface::Confirmation) {
        std::fs::write(
            dir.path().join("a.txt"),
            "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n",
        )
        .unwrap();
    }
    let backend = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let mut status = backend.status().unwrap();
    if matches!(surface, Surface::PaletteRename) {
        // The status list shows only the destination, so unstaging its entries
        // one by one would leave the source's staged deletion behind.
        let staged: Vec<_> = status
            .staged
            .iter()
            .filter(|entry| {
                entry.path.ends_with("moved.txt") || entry.path.ends_with("renamed.txt")
            })
            .map(|entry| (entry.path.clone(), entry.kind))
            .collect();
        assert_eq!(
            staged,
            [(
                PathBuf::from("renamed.txt"),
                gitcomet_core::domain::FileStatusKind::Renamed
            )]
        );
    }
    if matches!(surface, Surface::Confirmation) {
        Arc::make_mut(&mut status.unstaged)[0].conflict =
            Some(gitcomet_core::domain::FileConflictKind::BothModified);
    }
    let mut repo = opening_repo_state(REPO, dir.path());
    repo.open = Loadable::Ready(());
    repo.worktree_status = Loadable::Ready(status.unstaged.clone());
    repo.staged_status = Loadable::Ready(status.staged.clone());
    repo.status = Loadable::Ready(Arc::new(status));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    // Finish constructor settings before attaching the fixture. Tests pull
    // store snapshots explicitly because the live UI poller is disabled.
    crate::view::test_support::drain_store_worker(&view, cx);
    store.insert_repo_for_test(REPO, backend);
    super::shortcuts::apply_state(cx, &view, app_state_with_repo(repo, REPO));
    cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, _| {
            pane.auto_save_file_edits = false;
        });
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_file_list_layout(
                if matches!(surface, Surface::Folder) {
                    crate::view::FileListLayout::Tree
                } else {
                    crate::view::FileListLayout::Flat
                },
                cx,
            );
            if matches!(surface, Surface::UntrackedAll | Surface::TrackedAll) {
                pane.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
            }
        });
    });
    let target = DiffTarget::WorkingTree {
        path: path.into(),
        area: match viewed {
            Viewed::Diff(area) => area,
            _ => DiffArea::Unstaged,
        },
    };
    store.dispatch(match viewed {
        Viewed::Diff(_) => Msg::SelectDiff {
            repo_id: REPO,
            target: target.clone(),
        },
        Viewed::Content => Msg::OpenFileContent {
            repo_id: REPO,
            source: FileSource::WorkingDirectory,
            path: path.into(),
        },
        Viewed::Editor => Msg::OpenFileEditor {
            repo_id: REPO,
            path: path.into(),
        },
    });
    let description = format!("selected file target: {surface:?} {area:?} {viewed:?} {path}");
    wait_for(cx, &view, &store, &description, |repo| {
        repo.diff_state.diff_target.as_ref() == Some(&target)
            && match viewed {
                Viewed::Diff(_) => !repo.diff_state.content_preview,
                _ => repo.diff_state.content_preview,
            }
    });
    wait_for_displayed_content(cx, &view);
    if matches!(viewed, Viewed::Editor) {
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.ensure_file_editor_loaded(cx);
            });
        });
        cx.run_until_parked();
        super::shortcuts::wait_until(cx, "editor loaded before typing", |cx| {
            cx.update(|_, app| {
                let pane = view.read(app).main_pane.read(app);
                !pane.file_editor_loading
                    && pane.file_editor_input.read(app).text() == "base\nstaged\nunstaged\n"
            })
        });
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "unsaved\n", cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.read(app);
            assert!(
                pane.file_editor_is_dirty(),
                "the edit must be dirty before staging"
            );
        });
    }
    if matches!(
        surface,
        Surface::Selected
            | Surface::Shortcut(_)
            | Surface::RowUnrelatedSelection
            | Surface::DiscardSelected
    ) {
        cx.update(|_, app| {
            view.read(app).details_pane.clone().update(app, |pane, cx| {
                let mut selection = StatusMultiSelection::default();
                match area {
                    DiffArea::Unstaged => {
                        selection.explicit_section = Some(StatusSection::CombinedUnstaged);
                        selection.unstaged = vec!["a.txt".into(), "new.txt".into()];
                    }
                    DiffArea::Staged => {
                        selection.explicit_section = Some(StatusSection::Staged);
                        selection.staged = vec!["a.txt".into(), "nested/c.txt".into()];
                    }
                }
                if matches!(surface, Surface::RowUnrelatedSelection) {
                    match area {
                        DiffArea::Unstaged => {
                            selection.unstaged = vec!["b.txt".into(), "nested/c.txt".into()]
                        }
                        DiffArea::Staged => {
                            selection.staged = vec!["b.txt".into(), "nested/c.txt".into()]
                        }
                    }
                }
                pane.status_multi_selection.insert(REPO, selection);
                cx.notify();
            });
        });
    }
    let index_before = git_output(dir.path(), &["ls-files", "--stage"]);
    if failure == Failure::IndexLocked {
        std::fs::write(dir.path().join(".git/index.lock"), "locked by test").unwrap();
    } else if failure == Failure::GitUnavailable {
        store.dispatch(Msg::SetGitRuntimeState(
            gitcomet_core::process::GitRuntimeState {
                preference: gitcomet_core::process::GitExecutablePreference::Custom(PathBuf::new()),
                availability: gitcomet_core::process::GitExecutableAvailability::Unavailable {
                    detail: "unavailable in test".into(),
                },
            },
        ));
        crate::view::test_support::drain_store_worker(&view, cx);
    }
    draw_and_drain_test_window(cx);
    let snapshot = store.snapshot();
    let before = snapshot.repos[0].ops_rev;
    let status_rev = match area {
        DiffArea::Unstaged => snapshot.repos[0].worktree_status_rev,
        DiffArea::Staged => snapshot.repos[0].staged_status_rev,
    };
    let selector = match surface {
        Surface::All | Surface::Confirmation => match area {
            DiffArea::Unstaged => "stage_all_button".to_string(),
            DiffArea::Staged => "unstage_all_button".to_string(),
        },
        Surface::Selected => match area {
            DiffArea::Unstaged => "stage_selected_button".to_string(),
            DiffArea::Staged => "unstage_selected_button".to_string(),
        },
        Surface::UntrackedAll => "stage_all_untracked_button".to_string(),
        Surface::TrackedAll => "stage_all_split_unstaged_button".to_string(),
        Surface::ContextMenu
        | Surface::CommandPalette
        | Surface::PaletteRename
        | Surface::Shortcut(_)
        | Surface::DiscardSelected
        | Surface::DiscardPath => String::new(),
        Surface::Folder => {
            let section = if area == DiffArea::Unstaged {
                "unstaged"
            } else {
                "staged"
            };
            let (ix, bounds) = (0..10)
                .find_map(|ix| {
                    let row = format!("status_dir_{}_{section}_{ix}", REPO.0);
                    cx.debug_bounds(Box::leak(row.into_boxed_str()))
                        .map(|bounds| (ix, bounds))
                })
                .expect("folder row");
            cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
            draw_and_drain_test_window(cx);
            format!("status_dir_action_{}_{section}_{ix}", REPO.0)
        }
        Surface::Row | Surface::RowUnrelatedSelection => {
            let section = match area {
                DiffArea::Unstaged => "unstaged",
                DiffArea::Staged => "staged",
            };
            let row = format!("status_row_{}_{section}_0", REPO.0);
            let bounds = cx.debug_bounds(Box::leak(row.into_boxed_str())).unwrap();
            cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
            draw_and_drain_test_window(cx);
            format!("status_stage_button_{}_{section}_0", REPO.0)
        }
    };
    match surface {
        Surface::ContextMenu => cx.update(|window, app| {
            view.read(app).popover_host.clone().update(app, |host, cx| {
                let action = match area {
                    DiffArea::Unstaged => ContextMenuAction::StageSelectionOrPath {
                        repo_id: REPO,
                        area,
                        path: "a.txt".into(),
                    },
                    DiffArea::Staged => ContextMenuAction::UnstageSelectionOrPath {
                        repo_id: REPO,
                        area,
                        path: "a.txt".into(),
                    },
                };
                host.context_menu_activate_action(action, window, cx);
            });
        }),
        Surface::CommandPalette | Surface::PaletteRename => cx.update(|window, app| {
            view.update(app, |this, cx| {
                let command = match area {
                    DiffArea::Unstaged => "stage-all",
                    DiffArea::Staged => "unstage-all",
                };
                this.execute_command(command, Some(window), cx);
            });
        }),
        Surface::Shortcut(key) => {
            cx.update(|window, app| {
                app.clear_key_bindings();
                crate::app::bind_app_keys_for_test(app);
                crate::app::install_global_diff_shortcut_fallback_for_test(app);
                view.read(app)
                    .main_pane
                    .read(app)
                    .diff_panel_focus_handle
                    .clone()
                    .focus(window, app);
            });
            cx.simulate_keystrokes(key);
            draw_and_drain_test_window(cx);
        }
        Surface::DiscardSelected | Surface::DiscardPath => {
            cx.update(|window, app| {
                view.read(app).popover_host.clone().update(app, |host, cx| {
                    host.context_menu_activate_action(
                        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
                            repo_id: REPO,
                            area,
                            path: "a.txt".into(),
                        },
                        window,
                        cx,
                    );
                });
            });
            draw_and_drain_test_window(cx);
            click(cx, "discard_changes_go");
        }
        _ => click(cx, &selector),
    }
    if matches!(surface, Surface::Confirmation) {
        assert!(
            cx.update(|_, app| crate::view::test_support::popover_is_open(view.read(app), app))
        );
        click(cx, "stage_conflict_markers_cancel_hint");
        crate::view::test_support::drain_store_worker(&view, cx);
        assert_eq!(store.snapshot().repos[0].ops_rev, before);
        assert_eq!(
            store.snapshot().repos[0].diff_state.diff_target,
            Some(target.clone())
        );
        click(cx, &selector);
        click(cx, "stage_conflict_markers_go");
    }
    wait_for(
        cx,
        &view,
        &store,
        "status action completion and refresh",
        |repo| {
            repo.ops_rev >= before + 2
                && repo.local_actions_in_flight == 0
                && if failure != Failure::None {
                    repo.feedback.last_error.is_some()
                } else {
                    (match area {
                        DiffArea::Unstaged => repo.worktree_status_rev > status_rev,
                        DiffArea::Staged => repo.staged_status_rev > status_rev,
                    }) && matches!(repo.worktree_status, Loadable::Ready(_))
                        && matches!(repo.staged_status, Loadable::Ready(_))
                }
        },
    );
    // Checked before the content wait so a wrong diff fails by name.
    assert_eq!(
        store.snapshot().repos[0].diff_state.diff_target,
        (!closes).then(|| target.clone()),
        "{surface:?} {area:?} {viewed:?} {path}"
    );
    // Unavailable Git can also reject a reload. The failure cases assert that
    // the selected view survives; successful actions additionally reload it.
    if !closes && failure == Failure::None {
        wait_for_displayed_content(cx, &view);
    }
    let snapshot = store.snapshot();
    let repo = &snapshot.repos[0];
    if failure != Failure::None {
        assert!(
            repo.feedback.last_error.is_some(),
            "the action must report the expected failure: {failure:?}"
        );
        assert_eq!(
            git_output(dir.path(), &["ls-files", "--stage"]),
            index_before,
            "a rejected action must leave the index unchanged"
        );
    }
    assert!(
        failure != Failure::None || repo.feedback.last_error.is_none(),
        "{:?}",
        repo.feedback.last_error
    );
    assert_eq!(
        repo.diff_state.diff_target,
        (!closes).then_some(target),
        "{surface:?} {area:?} {viewed:?} {path}"
    );
    let acted_paths: &[&str] = match surface {
        Surface::UntrackedAll => &["new.txt"],
        Surface::Folder => &["nested/c.txt", "nested/d.txt"],
        // Every selected file moves, not just the one whose diff is shown.
        Surface::Selected | Surface::Shortcut(_) => match area {
            DiffArea::Unstaged => &["a.txt", "new.txt"],
            DiffArea::Staged => &["a.txt", "nested/c.txt"],
        },
        _ => &["a.txt"],
    };
    if failure == Failure::None {
        for acted_path in acted_paths {
            assert!(
                !repo
                    .status_entries_for_area(area)
                    .unwrap()
                    .iter()
                    .any(|entry| entry.path == Path::new(acted_path)),
                "{surface:?} {area:?}: {acted_path} must have left the list"
            );
        }
    }
    if matches!(surface, Surface::PaletteRename) {
        assert!(
            repo.staged_status_entries().unwrap().is_empty(),
            "unstage all must also unstage the old rename path"
        );
    }
    if matches!(surface, Surface::RowUnrelatedSelection) {
        cx.update(|_, app| {
            let pane = view.read(app).details_pane.read(app);
            assert_eq!(
                pane.status_multi_selection
                    .get(&REPO)
                    .unwrap()
                    .selected_paths_for_area(area),
                &[PathBuf::from("b.txt"), PathBuf::from("nested/c.txt")]
            );
        });
    }
    if !closes {
        assert_eq!(
            repo.diff_state.content_preview,
            !matches!(viewed, Viewed::Diff(_))
        );
        assert_eq!(repo.diff_state.edit_mode, matches!(viewed, Viewed::Editor));
        if matches!(viewed, Viewed::Editor) {
            cx.update(|_, app| {
                let pane = view.read(app).main_pane.read(app);
                assert!(pane.is_file_editor_active());
                assert!(pane.file_editor_is_dirty());
                assert!(
                    pane.file_editor_input
                        .read(app)
                        .text()
                        .starts_with("unsaved\n")
                );
            });
        }
    }
    // Windows live until the GPUI test ends; stop their repository monitors
    // before this case's temporary working directory is removed.
    store.dispatch(Msg::CloseRepo { repo_id: REPO });
    crate::view::test_support::drain_store_worker(&view, cx);
    assert!(store.snapshot().repos.is_empty());
}

#[gpui::test]
fn staging_buttons_keep_the_opposite_diff_open(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for surface in [Surface::All, Surface::Row, Surface::Selected] {
        exercise(
            cx,
            DiffArea::Unstaged,
            surface,
            Viewed::Diff(DiffArea::Staged),
            "a.txt",
            false,
        );
        exercise(
            cx,
            DiffArea::Staged,
            surface,
            Viewed::Diff(DiffArea::Unstaged),
            "a.txt",
            false,
        );
    }
}

#[gpui::test]
fn staging_buttons_close_only_the_affected_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for surface in [Surface::All, Surface::Row, Surface::Selected] {
            exercise(cx, area, surface, Viewed::Diff(area), "a.txt", true);
        }
        for surface in [Surface::Row, Surface::Selected] {
            exercise(cx, area, surface, Viewed::Diff(area), "b.txt", false);
        }
    }
}

#[gpui::test]
fn staging_buttons_preserve_content_and_unsaved_editors(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for surface in [Surface::All, Surface::Row] {
            for viewed in [Viewed::Content, Viewed::Editor] {
                exercise(cx, area, surface, viewed, "a.txt", false);
            }
        }
    }
}

#[gpui::test]
fn staging_split_sections_only_close_included_diffs(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    exercise(
        cx,
        DiffArea::Unstaged,
        Surface::UntrackedAll,
        Viewed::Diff(DiffArea::Unstaged),
        "a.txt",
        false,
    );
    exercise(
        cx,
        DiffArea::Unstaged,
        Surface::TrackedAll,
        Viewed::Diff(DiffArea::Unstaged),
        "new.txt",
        false,
    );
    exercise(
        cx,
        DiffArea::Unstaged,
        Surface::TrackedAll,
        Viewed::Diff(DiffArea::Unstaged),
        "a.txt",
        true,
    );
}

#[gpui::test]
fn staging_confirmation_preserves_opposite_diff_and_content(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for viewed in [Viewed::Diff(DiffArea::Staged), Viewed::Content] {
        exercise(
            cx,
            DiffArea::Unstaged,
            Surface::Confirmation,
            viewed,
            "a.txt",
            false,
        );
    }
    exercise(
        cx,
        DiffArea::Unstaged,
        Surface::Confirmation,
        Viewed::Diff(DiffArea::Unstaged),
        "a.txt",
        true,
    );
}

#[gpui::test]
fn staging_menus_preserve_opposite_diffs_and_file_content(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for surface in [Surface::ContextMenu, Surface::CommandPalette] {
        for (area, opposite) in [
            (DiffArea::Unstaged, DiffArea::Staged),
            (DiffArea::Staged, DiffArea::Unstaged),
        ] {
            exercise(cx, area, surface, Viewed::Diff(opposite), "a.txt", false);
            exercise(cx, area, surface, Viewed::Content, "a.txt", false);
        }
    }
}

#[gpui::test]
fn failed_staging_preserves_the_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for failure in [Failure::IndexLocked, Failure::GitUnavailable] {
        for area in [DiffArea::Unstaged, DiffArea::Staged] {
            for surface in [Surface::All, Surface::Folder, Surface::CommandPalette] {
                let path = if matches!(surface, Surface::Folder) {
                    "nested/c.txt"
                } else {
                    "a.txt"
                };
                exercise_with_failure(cx, area, surface, Viewed::Diff(area), path, false, failure);
            }
        }
    }
}

#[gpui::test]
fn palette_unstage_all_includes_both_sides_of_a_rename(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    exercise(
        cx,
        DiffArea::Staged,
        Surface::PaletteRename,
        Viewed::Diff(DiffArea::Staged),
        "renamed.txt",
        true,
    );
}

#[gpui::test]
fn staging_folders_and_palette_close_only_affected_diffs(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        let opposite = if area == DiffArea::Unstaged {
            DiffArea::Staged
        } else {
            DiffArea::Unstaged
        };
        exercise(
            cx,
            area,
            Surface::Folder,
            Viewed::Diff(area),
            "nested/c.txt",
            true,
        );
        exercise(
            cx,
            area,
            Surface::Folder,
            Viewed::Diff(area),
            "a.txt",
            false,
        );
        exercise(
            cx,
            area,
            Surface::Folder,
            Viewed::Diff(opposite),
            "nested/c.txt",
            false,
        );
        exercise(
            cx,
            area,
            Surface::CommandPalette,
            Viewed::Diff(area),
            "a.txt",
            true,
        );
    }
}

#[gpui::test]
fn staging_shortcuts_consume_the_selected_paths(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for key in [
            "space",
            if area == DiffArea::Unstaged {
                "ctrl-s"
            } else {
                "ctrl-u"
            },
        ] {
            exercise(
                cx,
                area,
                Surface::Shortcut(key),
                Viewed::Diff(area),
                "a.txt",
                true,
            );
        }
    }
}

#[gpui::test]
fn staging_a_row_preserves_an_unrelated_multi_selection(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        exercise(
            cx,
            area,
            Surface::RowUnrelatedSelection,
            Viewed::Diff(area),
            "a.txt",
            true,
        );
    }
}

#[gpui::test]
fn discarding_files_closes_only_the_affected_unstaged_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    for surface in [Surface::DiscardSelected, Surface::DiscardPath] {
        for (viewed, path, closes) in [
            (Viewed::Diff(DiffArea::Unstaged), "a.txt", true),
            (Viewed::Diff(DiffArea::Unstaged), "b.txt", false),
            (Viewed::Diff(DiffArea::Staged), "a.txt", false),
        ] {
            exercise(cx, DiffArea::Unstaged, surface, viewed, path, closes);
        }
    }
}

#[gpui::test]
fn checking_out_a_conflict_side_preserves_an_unrelated_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    for path in ["a.txt", "b.txt"] {
        std::fs::write(dir.path().join(path), "base\n").unwrap();
    }
    git(dir.path(), &["add", "."]);
    commit(dir.path(), "base");
    git(dir.path(), &["checkout", "-qb", "theirs"]);
    std::fs::write(dir.path().join("a.txt"), "theirs\n").unwrap();
    commit(dir.path(), "theirs");
    git(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("a.txt"), "ours\n").unwrap();
    commit(dir.path(), "ours");
    let merge = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        // Merge checks the committer identity even with --no-commit.
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "merge",
            "--no-commit",
            "theirs",
        ])
        .output()
        .unwrap();
    // Exit code 1 is the expected conflict; anything else is a broken fixture.
    assert_eq!(
        merge.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&merge.stderr)
    );
    std::fs::write(dir.path().join("b.txt"), "base\nchanged\n").unwrap();
    let backend = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let status = backend.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == Path::new("a.txt") && entry.conflict.is_some())
    );
    let mut repo = opening_repo_state(REPO, dir.path());
    repo.open = Loadable::Ready(());
    repo.worktree_status = Loadable::Ready(status.unstaged.clone());
    repo.staged_status = Loadable::Ready(status.staged.clone());
    repo.status = Loadable::Ready(Arc::new(status));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    crate::view::test_support::drain_store_worker(&view, cx);
    store.insert_repo_for_test(REPO, backend);
    super::shortcuts::apply_state(cx, &view, app_state_with_repo(repo, REPO));
    let target = DiffTarget::WorkingTree {
        path: "b.txt".into(),
        area: DiffArea::Unstaged,
    };
    store.dispatch(Msg::SelectDiff {
        repo_id: REPO,
        target: target.clone(),
    });
    wait_for(cx, &view, &store, "unrelated diff", |repo| {
        repo.diff_state.diff_target.as_ref() == Some(&target) && content_loaded(repo)
    });
    let before = store.snapshot().repos[0].ops_rev;
    cx.update(|window, app| {
        view.read(app).popover_host.clone().update(app, |host, cx| {
            host.context_menu_activate_action(
                ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                    repo_id: REPO,
                    area: DiffArea::Unstaged,
                    path: "a.txt".into(),
                    side: gitcomet_core::services::ConflictSide::Ours,
                },
                window,
                cx,
            );
        });
    });
    wait_for(cx, &view, &store, "checkout conflict side", |repo| {
        repo.ops_rev >= before + 2 && repo.local_actions_in_flight == 0
    });
    let snapshot = store.snapshot();
    assert!(
        snapshot.repos[0].feedback.last_error.is_none(),
        "{:?}",
        snapshot.repos[0].feedback.last_error
    );
    assert_eq!(
        snapshot.repos[0].diff_state.diff_target.as_ref(),
        Some(&target)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "ours\n"
    );
    store.dispatch(Msg::CloseRepo { repo_id: REPO });
    crate::view::test_support::drain_store_worker(&view, cx);
}
