//! Commit form: message drafts, amend and merge prefills, commit dispatch.

use super::*;

/// Read the painted quad and the IME anchor independently of caret row lookup.
fn draw_and_assert_commit_end_caret(
    input: &gpui::Entity<gitcomet_ui_kit::TextInput>,
    cx: &mut gpui::VisualTestContext,
) -> usize {
    use gpui::EntityInputHandler as _;
    for _ in 0..3 {
        crate::test_support::refresh_and_draw(cx);
    }
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            let rows = input.wrap_row_counts().iter().sum::<usize>();
            let first = input.hotspot_bounds(&(0..1)).unwrap();
            let expected_top = first.top() + first.size.height * (rows - 1) as f32;
            let end = input.text().encode_utf16().count();
            let ime = input.bounds_for_range(end..end, first, window, cx).unwrap();
            let text = input.text();
            assert_eq!(ime.top(), expected_top, "commit IME anchor for {text:?}");
            let scale = window.scale_factor();
            let color = input.theme_for_test().colors.editor.cursor;
            let caret = window
                .painted_quads()
                .into_iter()
                .find(|quad| {
                    quad.background == color.into()
                        && (quad.bounds.size.width.0 - scale).abs() < 0.01
                })
                .expect("commit input must paint its caret");
            let inset = (f32::from(first.size.height) - caret.bounds.size.height.0 / scale) / 2.0;
            assert!(
                (caret.bounds.origin.y.0 - (f32::from(expected_top) + inset) * scale).abs() <= 1.0,
                "commit caret for {text:?}: {:?}, expected row {}",
                caret.bounds,
                rows - 1
            );
            assert!((caret.bounds.origin.x.0 - f32::from(ime.left()) * scale).abs() <= 1.0);
            rows
        })
    })
}

#[gpui::test]
fn cached_commit_form_caret_tracks_wrapped_growth_and_shrink(cx: &mut gpui::TestAppContext) {
    use gpui::EntityInputHandler as _;
    let _visual_guard = lock_visual_test();
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(49);
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-wrap-caret"));
    repo.status = gitcomet_state::model::Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: "staged.txt".into(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
            unstaged: Arc::new(Vec::new()),
        }
        .into(),
    );
    let (input, scroll) = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let pane = view.read(app).details_pane.read(app);
        (
            pane.commit_message_input.clone(),
            pane.commit_message_scroll.clone(),
        )
    });
    cx.simulate_resize(gpui::size(px(1200.0), px(900.0)));
    for percent in [100, 150] {
        cx.update(|window, app| {
            crate::ui_scale::set_default(app, percent);
            view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
            input.update(app, |input, cx| input.set_text(String::new(), cx));
            window.focus(&input.read(app).focus_handle(), app);
        });
        for _ in 0..4 {
            crate::test_support::refresh_and_draw(cx);
        }
        let mut heights = [px(0.0); 3];
        let mut two_row_end = 0;
        let message = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        for (index, word) in message
            .split_whitespace()
            .cycle()
            .take(8 * message.split_whitespace().count())
            .enumerate()
        {
            if index > 0 {
                cx.simulate_input(" ");
            }
            cx.simulate_input(word);
            let rows = draw_and_assert_commit_end_caret(&input, cx);
            assert!(rows <= 3, "fixture must pass through each row");
            heights[rows - 1] = scroll.bounds().size.height;
            if rows == 2 {
                two_row_end = cx.update(|_, app| input.read(app).text().encode_utf16().count());
            }
            if rows == 3 {
                break;
            }
        }
        assert!(
            heights[1] > heights[0] && heights[2] > heights[1],
            "commit input must grow: {heights:?}"
        );
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                let end = input.text().encode_utf16().count();
                input.replace_text_in_range(Some(two_row_end..end), "", window, cx);
            })
        });
        assert_eq!(draw_and_assert_commit_end_caret(&input, cx), 2);
        assert_eq!(scroll.bounds().size.height, heights[1]);
    }
}

#[gpui::test]
fn staged_revert_offers_its_message_to_an_empty_commit_box(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    let state_with = |message: Option<&str>, rev: u64| {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo"));
        repo.suggested_commit_message = message.map(str::to_string);
        repo.suggested_commit_message_rev = rev;
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..AppState::test_default()
        })
    };
    let commit_box_text = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let details_pane = view.read(app).details_pane.clone();
            let pane = details_pane.read(app);
            pane.commit_message_input.read(app).text().to_string()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with(None, 0), cx);
        });
    });
    assert_eq!(commit_box_text(cx), "");

    let message = "Revert \"change\"\n\nThis reverts commit deadbeef.";
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with(Some(message), 1), cx);
        });
    });
    assert_eq!(commit_box_text(cx), message);

    // Repeated signing failures can suggest the same message more than once.
    // A successful retry consumes it, and the auto-filled draft clears too.
    for (text, rev, expected) in [(Some(message), 2, message), (None, 3, "")] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, state_with(text, rev), cx);
            });
        });
        assert_eq!(commit_box_text(cx), expected);
    }

    // A message the user is already writing is never overwritten.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("my own words".to_string(), cx)
                });
                cx.notify();
            });
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with(Some("Revert \"other\""), 4), cx);
        });
    });
    assert_eq!(commit_box_text(cx), "my own words");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with(None, 5), cx);
        });
    });
    assert_eq!(commit_box_text(cx), "my own words");
}

#[gpui::test]
fn switching_active_repo_restores_commit_message_draft_per_repo(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_a = gitcomet_state::model::RepoId(41);
    let repo_b = gitcomet_state::model::RepoId(42);
    let make_state = |active_repo: gitcomet_state::model::RepoId| {
        Arc::new(AppState {
            repos: vec![
                opening_repo_state(repo_a, Path::new("/tmp/repo-a")),
                opening_repo_state(repo_b, Path::new("/tmp/repo-b")),
            ],
            active_repo: Some(active_repo),
            ..AppState::test_default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("repo-b draft".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "draft message");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "repo-b draft");
    });
}

#[gpui::test]
fn merge_start_prefills_default_commit_message(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(43);
    let make_state = |merge_message: Option<&str>| {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-merge"));
        repo.merge_commit_message = gitcomet_state::model::Loadable::Ready(
            merge_message.map(std::string::ToString::to_string),
        );
        repo.merge_message_rev = u64::from(merge_message.is_some());
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(None), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(Some("Merge branch 'feature'")), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "Merge branch 'feature'"
        );
    });
}

fn state_with_recent_commit_message(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &str,
    recent: gitcomet_state::model::Loadable<Arc<Vec<gitcomet_core::domain::RecentCommitMessage>>>,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, Path::new(workdir));
    repo.recent_commit_messages_rev = u64::from(!matches!(
        recent,
        gitcomet_state::model::Loadable::NotLoaded
    ));
    repo.recent_commit_messages = recent;
    app_state_with_repo(repo, repo_id)
}

fn recent_messages(
    messages: &[&str],
) -> gitcomet_state::model::Loadable<Arc<Vec<gitcomet_core::domain::RecentCommitMessage>>> {
    gitcomet_state::model::Loadable::Ready(Arc::new(
        messages
            .iter()
            .enumerate()
            .map(|(ix, message)| gitcomet_core::domain::RecentCommitMessage {
                id: gitcomet_core::domain::CommitId(format!("commit{ix}").into()),
                summary: (*message).into(),
                message: (*message).to_string(),
            })
            .collect(),
    ))
}

#[gpui::test]
fn amend_prefills_commit_message_from_previous_commit_when_empty(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-prefill",
                    recent_messages(&["previous subject\n\nbody"]),
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "previous subject\n\nbody"
        );
    });
}

#[gpui::test]
fn amend_does_not_overwrite_existing_commit_message(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-no-overwrite",
                    recent_messages(&["previous subject"]),
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "draft message"
        );
    });
}

#[gpui::test]
fn amend_prefills_commit_message_once_recent_messages_load(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(52);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-deferred",
                    gitcomet_state::model::Loadable::NotLoaded,
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(pane.read(app).commit_message_input.read(app).text(), "");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-deferred",
                    recent_messages(&["deferred subject"]),
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "deferred subject"
        );
    });
}

#[gpui::test]
fn commit_message_focus_after_initial_draw_accepts_typed_input(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let make_state = || {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-focus"));
        repo.status = gitcomet_state::model::Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                    path: std::path::PathBuf::from("staged.txt"),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                }]),
                unstaged: std::sync::Arc::new(Vec::new()),
            }
            .into(),
        );
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(), cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_input("x");

    let text = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .text()
            .to_string()
    });
    assert_eq!(text, "x");
}

#[gpui::test]
fn commit_click_dispatches_after_state_update_without_intermediate_redraw(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let make_state = |staged_count: usize, local_actions_in_flight: u32| {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-click"));
        repo.status = gitcomet_state::model::Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: std::sync::Arc::new(
                    (0..staged_count)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("staged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                ),
                unstaged: std::sync::Arc::new(Vec::new()),
            }
            .into(),
        );
        repo.local_actions_in_flight = local_actions_in_flight;
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(0, 0), cx);
        });
        let _ = window.draw(app);
    });

    let commit_center = cx
        .debug_bounds("commit_button")
        .expect("expected commit button bounds")
        .center();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(1, 0), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input
                    .update(cx, |input, cx| input.set_text("hello".to_string(), cx));
                cx.notify();
            });
        });
    });

    cx.simulate_mouse_move(commit_center, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "",
            "expected first click to execute commit handler and clear the input"
        );
    });
}

/// Typing into the focused commit box re-renders the details pane only. The
/// bottom bar and the toast host paint after it, so a cached mount replays
/// their paint after the focused input's handler slot on every keystroke.
/// That replay panicked inside gpui's `reuse_paint` until gpui-ce took zed's
/// fix (#50665: the frame's input handler was popped, shortening the list
/// the cached ranges index), which is why both used to mount uncached and
/// re-rendered on every frame.
#[gpui::test]
fn typing_in_the_commit_box_replays_the_cached_bottom_bar_and_toasts(
    cx: &mut gpui::TestAppContext,
) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(45);
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-cached-chrome-typing"));
    repo.status = gitcomet_state::model::Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from("staged.txt"),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
            unstaged: std::sync::Arc::new(Vec::new()),
        }
        .into(),
    );
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.toast_host.update(cx, |host, cx| {
                host.push_toast(
                    components::ToastKind::Warning,
                    "a toast stays open while typing".into(),
                    cx,
                )
            });
        });
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });
    // Let the toast's fade-in finish: an animation renders its view per frame.
    for _ in 0..10 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let root = view.read(app);
            (
                root.bottom_status_bar.read(app).render_count,
                root.toast_host.read(app).render_count,
            )
        })
    };
    // The first character refreshes the whole window once (no notify reaches
    // either view; the commit form changes with an empty message). Measure the
    // keystrokes after it.
    cx.simulate_input("h");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let before = renders(cx);

    cx.simulate_input("ello");

    let text = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .text()
            .to_string()
    });
    assert_eq!(text, "hello");
    assert_eq!(
        cx.update(|_window, app| view.read(app).toast_host.read(app).toast_count_for_tests()),
        1
    );
    assert_eq!(
        renders(cx),
        before,
        "typing re-rendered the bottom bar or the toast host"
    );
}

/// The bar draws the pane toggles from the root's collapse flags, so a cached
/// bar must be told when they change.
#[gpui::test]
fn collapsing_a_pane_rerenders_the_cached_bottom_bar(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(46);
    let repo = opening_repo_state(repo_id, Path::new("/tmp/repo-cached-bottom-bar-toggles"));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let _ = window.draw(app);
    });
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).bottom_status_bar.read(app).render_count)
    };
    for collapse in [
        |view: &mut super::super::super::GitCometView,
         cx: &mut gpui::Context<super::super::super::GitCometView>| {
            view.set_sidebar_collapsed(true, cx)
        },
        |view: &mut super::super::super::GitCometView,
         cx: &mut gpui::Context<super::super::super::GitCometView>| {
            view.set_details_collapsed(true, cx)
        },
    ] {
        let before = renders(cx);
        cx.update(|window, app| {
            view.update(app, collapse);
            let _ = window.draw(app);
        });
        assert!(
            renders(cx) > before,
            "the bottom bar must redraw its pane toggles ({before} renders before)"
        );
    }
}

/// The bar rounds its bottom corners by the frame's tiling, which it reads
/// at render, so a cached bar must be told when the decorations change.
#[gpui::test]
fn a_tiling_change_rerenders_the_cached_bottom_bar(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(47);
    let repo = opening_repo_state(repo_id, Path::new("/tmp/repo-cached-bottom-bar-tiling"));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let _ = window.draw(app);
    });
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).bottom_status_bar.read(app).render_count)
    };
    let before = renders(cx);
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            // The test window reports server decorations; the compositor
            // tiled the window's bottom edge at the same size.
            view.sync_frame_decorations(
                gpui::Decorations::Client {
                    tiling: gpui::Tiling {
                        bottom: true,
                        ..Default::default()
                    },
                },
                cx,
            );
        });
        let _ = window.draw(app);
    });
    assert!(
        renders(cx) > before,
        "the bottom bar must redraw its corners ({before} renders before)"
    );
}

/// A held drag selection past the commit box scrolls it. Cached views on: the
/// autoscroll only notifies the input, and the cached details pane around it
/// must still re-lay out its scroll container.
#[gpui::test]
fn holding_a_drag_selection_past_the_commit_box_scrolls_it(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(48);
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-box-drag-scroll"));
    repo.status = gitcomet_state::model::Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from("staged.txt"),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
            unstaged: std::sync::Arc::new(Vec::new()),
        }
        .into(),
    );
    let message: String = (0..40)
        .map(|ix| format!("commit message line {ix}\n"))
        .collect();
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
    draw(cx);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text(message.clone(), cx);
                    input.set_caret(0, cx);
                });
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
    });
    cx.run_until_parked();
    for _ in 0..3 {
        draw(cx);
    }
    let scroll = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_scroll
            .clone()
    });
    let viewport = scroll.bounds();
    assert!(
        scroll.max_offset().y > px(0.0),
        "the message overflows the box"
    );

    let at = point(viewport.left() + px(20.0), viewport.top() + px(12.0));
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    draw(cx);
    let below = point(at.x, viewport.bottom() + px(20.0));
    cx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::default());
    draw(cx);
    for _ in 0..10 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        draw(cx);
    }
    assert!(
        scroll.offset().y < px(0.0),
        "held below, the commit box scrolls"
    );
    let selected = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .selected_range()
    });
    assert!(!selected.is_empty(), "and the drag selects");
    cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::default());
}
