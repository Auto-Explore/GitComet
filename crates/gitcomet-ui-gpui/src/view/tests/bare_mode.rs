use super::*;
use crate::appearance::{Appearance, UiDensity};
use gitcomet_core::domain::{
    BareConversionHead, BareConversionLayout, BareConversionPreview, RepositoryMode,
};
use gitcomet_extension_api::{
    BarItemLocation, BuiltinBarItem, Extension, ExtensionId, Registrar, Registry,
};
use gitcomet_extension_example::review::ReviewExtension;
use gitcomet_state::model::BareModeConversionState;

fn state(mode: RepositoryMode) -> Arc<AppState> {
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.git_runtime = available_git_runtime_state();
    let repo = &mut state.repos[0];
    repo.repository_mode = Some(mode);
    repo.spec.workdir = PathBuf::from(format!(
        "/tmp/gitcomet-bare-ui-{:?}/a-repository-with-a-deliberately-long-name-for-bare-mode",
        std::thread::current().id(),
    ));
    repo.head_branch = Loadable::Ready("feature/a-long-branch-name-that-needs-truncation".into());
    repo.head_commit = Some(CommitId("0123456789abcdef0123456789abcdef01234567".into()));
    Arc::new(state)
}

fn preview() -> Arc<BareConversionPreview> {
    Arc::new(BareConversionPreview::new(
        state(RepositoryMode::Bare).repos[0].spec.workdir.clone(),
        BareConversionLayout::BareDirectory,
        BareConversionHead::Branch {
            name: "feature/a-long-branch-name-that-needs-truncation".into(),
            commit: Some(CommitId("0123456789abcdef0123456789abcdef01234567".into())),
        },
        Vec::new(),
    ))
}

fn open(
    cx: &mut gpui::TestAppContext,
    registry: Registry,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let initial = state(RepositoryMode::Bare);
    store.replace_snapshot_for_test(initial.clone());
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, initial, cx)
        });
        extension_host::window_opened(&view, app);
    });
    test_support::redraw(cx);
    (view, cx)
}

fn replace(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext, state: Arc<AppState>) {
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
        });
    });
    test_support::redraw(cx);
}

#[gpui::test]
fn bare_badge_survives_extension_views_and_branch_click_cannot_checkout(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open(
        cx,
        Registry::build(vec![Box::new(ReviewExtension)]).unwrap(),
    );
    cx.simulate_resize(gpui::size(px(820.0), px(700.0)));
    test_support::redraw(cx);
    assert!(cx.debug_bounds("repository_mode_badge").is_some());
    assert!(cx.debug_bounds("bare_mode_status").is_some());
    let badge = cx.debug_bounds("repository_mode_badge").unwrap();
    cx.simulate_mouse_move(badge.center(), None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(test_support::tooltip_text(cx, &view).map(|text| text.to_string()), Some("This repository stores Git history without an active working tree. You can browse history, fetch, and push. Checkout, staging, and committing working files are unavailable.".into()));
    let tooltip = cx.debug_bounds("tooltip_bubble").unwrap();
    assert!(
        tooltip.left() >= px(0.0) && tooltip.right() <= px(820.0),
        "{tooltip:?}"
    );
    click_debug_selector(cx, "branch_badge");
    cx.update(|_, app| assert!(test_support::popover_kind(view.read(app), app).is_none()));
    click_debug_selector(cx, "repository_view_1");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("extension_action_bar").is_some());
    assert!(cx.debug_bounds("example_changes_actions").is_some());
    assert!(cx.debug_bounds("example_changes_view").is_some());
    assert!(cx.debug_bounds("tracking_actions").is_none());
    let worktree = cx
        .debug_bounds("worktree_badge")
        .expect("worktree stays visible in extension views");
    let mode = cx
        .debug_bounds("repository_mode_badge")
        .expect("mode stays visible in extension views");
    assert!(worktree.right() <= mode.left());
    click_debug_selector(cx, "repository_mode_badge");
    cx.update(|_, app| {
        assert!(matches!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::RepositoryModeMenu,
                ..
            })
        ))
    });
    replace(&view, cx, state(RepositoryMode::WorkingTree));
    assert!(cx.debug_bounds("repository_mode_badge").is_none());
}

struct MovedMode;

impl Extension for MovedMode {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.bare-placement").unwrap()
    }
    fn register(&self, registrar: &mut Registrar) {
        registrar.place_bar_item(
            BuiltinBarItem::RepositoryMode,
            BarItemLocation::StatusBarStart,
        );
    }
}

#[gpui::test]
fn bare_badge_obeys_the_public_bar_placement_contract(cx: &mut gpui::TestAppContext) {
    let (_, cx) = open(cx, Registry::build(vec![Box::new(MovedMode)]).unwrap());
    let mode = cx.debug_bounds("repository_mode_badge").unwrap();
    let footer = cx.debug_bounds("bottom_status_bar").unwrap();
    assert!(footer.contains(&mode.center()));
    assert_eq!(
        BuiltinBarItem::RepositoryMode.default_location(),
        BarItemLocation::ActionBarStart
    );
}

#[gpui::test]
fn bare_badge_and_confirmation_fit_minimum_width_at_every_density(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open(cx, Registry::build(Vec::new()).unwrap());
    cx.simulate_resize(gpui::size(px(820.0), px(700.0)));
    for density in UiDensity::ALL {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            cx.update(|_, app| {
                app.set_global(Appearance {
                    density,
                    ..Default::default()
                });
                view.update(app, |view, cx| {
                    view.set_theme(theme, cx);
                    view.notify_font_preferences_changed(cx);
                });
            });
            test_support::redraw(cx);
            let worktree = cx.debug_bounds("worktree_badge").unwrap();
            let mode = cx.debug_bounds("repository_mode_badge").unwrap();
            let left = cx.debug_bounds("left_action_group").unwrap();
            let right = cx.debug_bounds("right_action_group").unwrap();
            assert!(worktree.right() <= mode.left(), "{density:?}");
            assert!(
                mode.right() <= right.left(),
                "{density:?}: {mode:?} vs {right:?}"
            );
            assert!(
                left.right() <= right.left(),
                "{density:?}: {left:?} vs {right:?}"
            );
        }
    }
    let mut confirming = (*state(RepositoryMode::Bare)).clone();
    confirming.repos[0].bare_mode_conversion =
        BareModeConversionState::AwaitingConfirmation(preview());
    replace(&view, cx, Arc::new(confirming));
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::Repo {
                        repo_id: RepoId(1),
                        kind: RepoPopoverKind::DisableBareModeConfirm,
                    }
                    .invoked_by("repository_mode_badge".into()),
                    point(px(410.0), px(100.0)),
                    window,
                    cx,
                )
            });
        })
    });
    test_support::redraw(cx);
    let dialog = cx.debug_bounds("bare_mode_confirmation").unwrap();
    assert!(
        dialog.left() >= px(0.0) && dialog.right() <= px(820.0),
        "{dialog:?}"
    );
    assert!(
        dialog.top() >= px(0.0) && dialog.bottom() <= px(700.0),
        "{dialog:?}"
    );
    let cancel = cx.debug_bounds("bare_mode_cancel").unwrap();
    let confirm = cx.debug_bounds("bare_mode_confirm").unwrap();
    assert!(cancel.right() <= confirm.left());
}

#[gpui::test]
fn bare_badge_remains_through_reopening_and_disappears_only_with_working_tree_mode(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open(cx, Registry::build(Vec::new()).unwrap());
    for conversion in [
        BareModeConversionState::Previewing,
        BareModeConversionState::Converting,
        BareModeConversionState::Reopening,
    ] {
        let mut next = (*state(RepositoryMode::Bare)).clone();
        next.repos[0].bare_mode_conversion = conversion;
        replace(&view, cx, Arc::new(next));
        assert!(cx.debug_bounds("repository_mode_badge").is_some());
    }
    replace(&view, cx, state(RepositoryMode::WorkingTree));
    assert!(cx.debug_bounds("repository_mode_badge").is_none());
}

#[gpui::test]
fn bare_mode_replaces_stale_working_file_surfaces_but_keeps_committed_diffs(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open(cx, Registry::build(Vec::new()).unwrap());
    let mut next = (*state(RepositoryMode::Bare)).clone();
    next.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
        "file.txt".into(),
        DiffArea::Unstaged,
    ));
    replace(&view, cx, Arc::new(next.clone()));
    assert!(cx.debug_bounds("bare_mode_diff").is_some());
    assert!(cx.debug_bounds("conflict_save").is_none());
    assert!(cx.debug_bounds("file_save").is_none());
    next.repos[0].diff_state.diff_target =
        Some(DiffTarget::commit(CommitId("c1".into()), "file.txt".into()));
    replace(&view, cx, Arc::new(next));
    assert!(cx.debug_bounds("bare_mode_diff").is_none());
}

#[gpui::test]
fn bare_operation_controls_explain_why_a_stale_merge_or_sequencer_cannot_run(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open(cx, Registry::build(Vec::new()).unwrap());
    for (merging, selector) in [(true, "abort_merge"), (false, "continue_cherry_pick")] {
        let mut next = (*state(RepositoryMode::Bare)).clone();
        let repo = &mut next.repos[0];
        repo.merge_commit_message = Loadable::Ready(merging.then(|| "Merge branch".into()));
        repo.sequencer_state = Loadable::Ready(gitcomet_core::services::SequencerState::CherryPick);
        repo.merge_message_rev += 1;
        replace(&view, cx, Arc::new(next));
        let button = cx.debug_bounds(selector).expect("paused operation control");
        cx.simulate_mouse_move(button.center(), None, gpui::Modifiers::default());
        test_support::wait_for_native_tooltip(cx);
        assert_eq!(
            test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
            Some("Requires a working tree; this repository is bare.".into())
        );
        click_debug_selector(cx, selector);
        cx.update(|_, app| {
            assert!(test_support::popover_kind(view.read(app), app).is_none());
            assert_eq!(
                view.read(app).store.snapshot().repos[0].sequencer_actions_in_flight,
                0
            );
        });
        cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, gpui::Modifiers::default());
    }
}

#[test]
fn bare_palette_refuses_working_file_actions_but_keeps_repository_and_tool_prerequisites() {
    use crate::view::command_palette::{COMMANDS, PaletteContext, command_unavailable_reason};
    use crate::view::commands::Command;
    let mut context = PaletteContext {
        has_active_repo: true,
        is_bare: true,
        repo_uses_lfs: true,
        repo_uses_annex: true,
        annex_initialized: true,
        ..Default::default()
    };
    let reason = |command: Command, context: &PaletteContext| {
        command_unavailable_reason(
            COMMANDS
                .iter()
                .find(|entry| entry.id == command.id())
                .unwrap(),
            context,
        )
    };
    for command in [
        Command::Commit,
        Command::StageAll,
        Command::UnstageAll,
        Command::CheckoutBranch,
        Command::Rebase,
        Command::Stash,
        Command::StashPop,
        Command::StashApply,
        Command::StashDrop,
        Command::ApplyPatch,
        Command::AddSubmodule,
        Command::UpdateSubmodules,
        Command::LfsDownloadAll,
        Command::AnnexAdjustUnlocked,
        Command::AnnexLeaveAdjusted,
        Command::AnnexWebapp,
    ] {
        assert_eq!(
            reason(command, &context),
            Some("Requires a working tree; this repository is bare."),
            "{command:?}"
        );
    }
    for command in [
        Command::CreateBranch,
        Command::DeleteBranch,
        Command::RenameBranch,
        Command::FetchAll,
        Command::Push,
        Command::AddWorktree,
        Command::LfsFetchAll,
        Command::LfsPrune,
        Command::LfsFsck,
        Command::LfsInstall,
        Command::AnnexGetAll,
        Command::AnnexPull,
        Command::AnnexPush,
        Command::AnnexSync,
        Command::AnnexFsck,
        Command::AnnexRestage,
    ] {
        assert_eq!(reason(command, &context), None, "{command:?}");
    }
    // Pull's own reason covers bare repositories and annex takeover.
    assert_eq!(reason(Command::Pull, &context), None);
    context.pull_unavailable = Some(gitcomet_core::domain::WORKING_TREE_REQUIRED_MESSAGE);
    assert_eq!(
        reason(Command::Pull, &context),
        Some("Requires a working tree; this repository is bare.")
    );
    context.git_lfs_missing = true;
    assert_eq!(
        reason(Command::LfsFetchAll, &context),
        Some("Install Git LFS where Git can find it first")
    );
    context.annex_initialized = false;
    assert!(reason(Command::AnnexGetAll, &context).is_some());
    context.is_bare = false;
    assert_eq!(reason(Command::CheckoutBranch, &context), None);
}

struct WorkingTreeCommand(std::rc::Rc<std::cell::Cell<usize>>);

impl Extension for WorkingTreeCommand {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.bare-command").unwrap()
    }
    fn register(&self, registrar: &mut Registrar) {
        let calls = self.0.clone();
        registrar.command(
            "edit",
            gitcomet_extension_api::CommandDescriptor::new(
                "Edit working file",
                "Test",
                move |_, _, _| calls.set(calls.get() + 1),
            )
            .requiring_working_tree(),
        );
    }
}

#[gpui::test]
fn extension_working_tree_command_refuses_bare_and_rechecks_before_deferred_callback(
    cx: &mut gpui::TestAppContext,
) {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let (view, cx) = open(
        cx,
        Registry::build(vec![Box::new(WorkingTreeCommand(calls.clone()))]).unwrap(),
    );
    let run = |view: &mut GitCometView, cx: &mut gpui::Context<GitCometView>| {
        assert!(view.run_extension_command("com.example.bare-command/edit", cx))
    };
    cx.update(|_, app| view.update(app, run));
    cx.run_until_parked();
    assert_eq!(calls.get(), 0);
    replace(&view, cx, state(RepositoryMode::WorkingTree));
    cx.update(|_, app| view.update(app, run));
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            run(view, cx);
            let next = state(RepositoryMode::Bare);
            view.store.replace_snapshot_for_test(next.clone());
            test_support::push_test_state(view, next, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        calls.get(),
        1,
        "a command deferred before the mode changed must not run"
    );
}

#[cfg(target_os = "linux")]
fn capture_frame(
    cx: &mut gpui::HeadlessAppContext,
    handle: gpui::WindowHandle<GitCometView>,
    path: &Path,
) {
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, app| {
        window.refresh();
        let _ = window.draw(app);
    })
    .unwrap();
    cx.capture_screenshot(handle.into())
        .unwrap()
        .save(path)
        .unwrap();
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "renders bare mode previews using the native text backend and a GPU"]
fn bare_mode_visual_capture() {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = gpui::HeadlessAppContext::with_platform(
        text_system,
        Arc::new(crate::assets::GitCometAssets::default()),
        || {
            Some(Box::new(
                gpui_wgpu::WgpuHeadlessRenderer::new().expect("headless GPU renderer"),
            ))
        },
    );
    cx.update(|app| {
        extension_host::install(
            Registry::build(vec![Box::new(ReviewExtension)]).unwrap(),
            app,
        )
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let base = state(RepositoryMode::Bare);
    store.replace_snapshot_for_test(base.clone());
    let handle = cx
        .open_window(gpui::size(px(820.0), px(700.0)), |window, app| {
            let view = app.new(|cx| GitCometView::new(store.clone(), events, None, window, cx));
            extension_host::window_opened(&view, app);
            view
        })
        .unwrap();
    let directory =
        PathBuf::from(std::env::var_os("GITCOMET_BARE_UI_CAPTURE_DIR").expect("capture directory"));
    std::fs::create_dir_all(&directory).unwrap();
    for density in UiDensity::ALL {
        for (name, theme) in [
            ("dark", AppTheme::gitcomet_dark()),
            ("light", AppTheme::gitcomet_light()),
        ] {
            for font_size in [14, 20] {
                cx.update(|app| {
                    app.set_global(Appearance {
                        density,
                        ui_font_size_px: font_size,
                        ..Default::default()
                    })
                });
                handle
                    .update(&mut cx, |view, _, cx| {
                        test_support::push_test_state(view, base.clone(), cx);
                        view.set_theme(theme, cx);
                        view.notify_font_preferences_changed(cx);
                    })
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(handle.into(), |_, window, app| {
                    window.refresh();
                    let _ = window.draw(app);
                })
                .unwrap();
                cx.capture_screenshot(handle.into())
                    .unwrap()
                    .save(
                        directory.join(format!("{}-{name}-{font_size}-chrome.png", density.key())),
                    )
                    .unwrap();
                handle
                    .update(&mut cx, |view, window, cx| {
                        let mut next = (*base).clone();
                        next.repos[0].bare_mode_conversion =
                            BareModeConversionState::AwaitingConfirmation(preview());
                        test_support::push_test_state(view, Arc::new(next), cx);
                        view.popover_host.update(cx, |host, cx| {
                            host.open_popover_at(
                                PopoverKind::Repo {
                                    repo_id: RepoId(1),
                                    kind: RepoPopoverKind::DisableBareModeConfirm,
                                }
                                .invoked_by("repository_mode_badge".into()),
                                point(px(410.0), px(70.0)),
                                window,
                                cx,
                            )
                        });
                    })
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(handle.into(), |_, window, app| {
                    window.refresh();
                    let _ = window.draw(app);
                })
                .unwrap();
                cx.capture_screenshot(handle.into())
                    .unwrap()
                    .save(directory.join(format!(
                        "{}-{name}-{font_size}-confirmation.png",
                        density.key()
                    )))
                    .unwrap();
                handle
                    .update(&mut cx, |view, _, cx| {
                        view.popover_host
                            .update(cx, |host, cx| host.close_popover(cx))
                    })
                    .unwrap();
            }
        }
    }
    for (name, conversion) in [
        ("converting", BareModeConversionState::Converting),
        ("reopening", BareModeConversionState::Reopening),
        (
            "refusal",
            BareModeConversionState::Failed(Arc::new(Error::new(
                ErrorKind::BareConversionRefused(
                    gitcomet_core::error::BareConversionRefusal::BranchInUse {
                        branch: "feature/a-long-branch-name-that-needs-truncation".into(),
                        worktree: PathBuf::from(
                            "/tmp/another-checkout-with-a-long-name-that-must-release-the-branch",
                        ),
                    },
                ),
            ))),
        ),
    ] {
        handle
            .update(&mut cx, |view, window, cx| {
                let mut next = (*base).clone();
                next.repos[0].bare_mode_conversion = conversion;
                test_support::push_test_state(view, Arc::new(next), cx);
                view.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        PopoverKind::Repo {
                            repo_id: RepoId(1),
                            kind: RepoPopoverKind::DisableBareModeConfirm,
                        }
                        .invoked_by("repository_mode_badge".into()),
                        point(px(410.0), px(70.0)),
                        window,
                        cx,
                    )
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, app| {
            window.refresh();
            let _ = window.draw(app);
        })
        .unwrap();
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(directory.join(format!("{name}.png")))
            .unwrap();
    }
    handle
        .update(&mut cx, |view, _, cx| {
            view.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
            test_support::push_test_state(view, base.clone(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|app| {
        app.set_global(Appearance {
            density: UiDensity::Comfortable,
            ..Default::default()
        })
    });
    handle
        .update(&mut cx, |view, _, cx| {
            view.set_theme(AppTheme::gitcomet_dark(), cx);
            view.notify_font_preferences_changed(cx);
        })
        .unwrap();
    capture_frame(&mut cx, handle, &directory.join("badge-before-hover.png"));
    let badge = cx
        .debug_bounds(handle.into(), "repository_mode_badge")
        .unwrap()
        .unwrap()
        .center();
    cx.update_window(handle.into(), |_, window, app| {
        window.dispatch_event(
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position: badge,
                ..Default::default()
            }),
            app,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.advance_clock(Duration::from_millis(500));
    capture_frame(&mut cx, handle, &directory.join("badge-tooltip.png"));
    let tooltip = cx
        .debug_bounds(handle.into(), "tooltip_bubble")
        .unwrap()
        .unwrap();
    assert!(
        tooltip.left() >= px(0.0) && tooltip.right() <= px(820.0),
        "{tooltip:?}"
    );
    assert!(
        tooltip.size.height > px(30.0),
        "the explanation must wrap: {tooltip:?}"
    );
    handle.update(&mut cx, |view, _, cx| {
        assert_eq!(view.tooltip_text_for_test(cx).map(|text| text.to_string()), Some("This repository stores Git history without an active working tree. You can browse history, fetch, and push. Checkout, staging, and committing working files are unavailable.".into()));
    }).unwrap();
    cx.update_window(handle.into(), |_, window, app| {
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: badge,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            app,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: badge,
                modifiers: Default::default(),
                click_count: 1,
            }),
            app,
        );
    })
    .unwrap();
    capture_frame(&mut cx, handle, &directory.join("badge-menu.png"));
    handle
        .update(&mut cx, |view, _, cx| {
            assert!(matches!(
                test_support::popover_kind(view, cx),
                Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::RepositoryModeMenu,
                    ..
                })
            ));
            view.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(&mut cx, |view, window, cx| {
            view.select_routed_view(
                crate::view::repository_views::RoutedArea::Main,
                Some(1),
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, app| {
        window.refresh();
        let _ = window.draw(app);
    })
    .unwrap();
    assert!(
        cx.debug_bounds(handle.into(), "example_changes_actions")
            .unwrap()
            .is_some()
    );
    assert!(
        cx.debug_bounds(handle.into(), "example_changes_view")
            .unwrap()
            .is_some()
    );
    assert!(
        cx.debug_bounds(handle.into(), "tracking_actions")
            .unwrap()
            .is_none()
    );
    cx.capture_screenshot(handle.into())
        .unwrap()
        .save(directory.join("extension-view.png"))
        .unwrap();
}
