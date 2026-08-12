use super::*;
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitId, LogPage, ReflogEntry, RepoSpec, RepoStatus, StashEntry,
};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CommandOutput, PullMode};
use gitcomet_state::model::Loadable;
use gitcomet_state::msg::{Msg, StoreEvent};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

#[derive(Clone)]
pub(super) struct TrackingRepo {
    spec: RepoSpec,
    branches: Arc<Mutex<Vec<String>>>,
    current_branch: Arc<Mutex<String>>,
    actions: Arc<Mutex<Vec<String>>>,
}

impl TrackingRepo {
    fn new(workdir: PathBuf) -> Self {
        Self {
            spec: RepoSpec { workdir },
            branches: Arc::new(Mutex::new(vec!["main".to_string()])),
            current_branch: Arc::new(Mutex::new("main".to_string())),
            actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn actions(&self) -> Vec<String> {
        self.actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl GitRepository for TrackingRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&gitcomet_core::domain::LogCursor>,
    ) -> Result<LogPage> {
        Ok(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        })
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Err(Error::new(ErrorKind::Unsupported(
            "commit details are not needed in create-branch popover tests",
        )))
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Ok(Vec::new())
    }

    fn current_branch(&self) -> Result<String> {
        Ok(self
            .current_branch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        Ok(self
            .branches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .map(|name| Branch {
                name,
                target: CommitId("HEAD".into()),
                upstream: None,
                divergence: None,
            })
            .collect())
    }

    fn list_remotes(&self) -> Result<Vec<gitcomet_core::domain::Remote>> {
        Ok(Vec::new())
    }

    fn list_remote_branches(&self) -> Result<Vec<gitcomet_core::domain::RemoteBranch>> {
        Ok(Vec::new())
    }

    fn status(&self) -> Result<RepoStatus> {
        Ok(RepoStatus::default())
    }

    fn diff_unified(&self, _target: &gitcomet_core::domain::DiffTarget) -> Result<String> {
        Err(Error::new(ErrorKind::Unsupported(
            "diffs are not needed in create-branch popover tests",
        )))
    }

    fn create_branch(&self, name: &str, _target: &CommitId) -> Result<()> {
        self.actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("create:{name}"));

        let mut branches = self
            .branches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !branches.iter().any(|branch| branch == name) {
            branches.push(name.to_string());
        }
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        let mut branches = self
            .branches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        branches.retain(|branch| branch != name);
        Ok(())
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("checkout:{name}"));
        *self
            .current_branch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = name.to_string();
        Ok(())
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
        self.actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("stash:{message}:{include_untracked}"));
        Ok(())
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        Ok(Vec::new())
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Ok(())
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Ok(())
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn fetch_all(&self) -> Result<()> {
        Ok(())
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        Ok(())
    }

    fn push(&self) -> Result<()> {
        Ok(())
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }

    fn create_tag_with_output(
        &self,
        name: &str,
        target: &str,
        _message: Option<&str>,
        _annotated: bool,
    ) -> Result<CommandOutput> {
        self.actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("tag:{name}:{target}"));
        Ok(CommandOutput::empty_success(format!(
            "git tag {name} {target}"
        )))
    }
}

struct TrackingBackend {
    repo: Arc<TrackingRepo>,
}

impl GitBackend for TrackingBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(self.repo.clone())
    }
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gitcomet-ui-popover-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("test workdir to be created");
    path
}

fn normalize_store_workdir(path: &Path) -> PathBuf {
    canonicalize_or_original(path.to_path_buf())
}

pub(super) fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn create_tracking_store(
    label: &str,
) -> (
    AppStore,
    smol::channel::Receiver<StoreEvent>,
    Arc<TrackingRepo>,
    PathBuf,
) {
    let workdir = unique_temp_dir(label);
    let expected_workdir = normalize_store_workdir(&workdir);
    let repo = Arc::new(TrackingRepo::new(workdir.clone()));
    let (store, events) = AppStore::new(Arc::new(TrackingBackend {
        repo: Arc::clone(&repo),
    }));
    store.dispatch(Msg::OpenRepo(workdir.clone()));
    wait_until("tracked test repo to open", || {
        let snapshot = store.snapshot();
        snapshot
            .active_repo
            .and_then(|repo_id| {
                snapshot
                    .repos
                    .iter()
                    .find(|repo_state| repo_state.id == repo_id)
            })
            .is_some_and(|repo_state| {
                repo_state.spec.workdir == expected_workdir
                    && matches!(repo_state.open, Loadable::Ready(()))
            })
    });
    (store, events, repo, workdir)
}

#[gpui::test]
fn create_branch_popover_escape_cancels(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-branch-escape");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: false,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("create_branch_btn"));
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close create-branch popover");
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, None);
    });
    cx.update(|window, app| {
        let root = view.read(app);
        let main_focus = root
            .popover_host
            .read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone();
        assert!(
            main_focus.is_focused(window),
            "expected Escape to move focus away from the Branch button"
        );
    });
    assert!(
        repo.actions().is_empty(),
        "expected Escape to cancel without creating a branch"
    );
}

#[gpui::test]
fn create_branch_source_picker_selects_items_on_mouse_down(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("create-branch-source-click");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: true,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                let search = host
                    .branch_picker_search_input
                    .as_ref()
                    .expect("branch picker search input");
                // The prompt prefills the source ("HEAD"), which the picker
                // treats as a filter query; clear it so the whole ref list is
                // listed, like a user who types over the prefilled source.
                search.update(cx, |input, cx| input.set_text("", cx));
                let focus = search.read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    click_debug_selector(cx, "picker_prompt_item_1");

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(host.create_branch_source_target, "main");
        assert_eq!(
            host.branch_picker_search_input
                .as_ref()
                .expect("branch picker search input")
                .read(app)
                .text(),
            "main"
        );
        assert_window_focus(
            window,
            app,
            host.create_branch_input.read(app).focus_handle(),
            "expected clicking a source branch to focus the new branch name",
        );
    });
}

#[gpui::test]
fn create_branch_source_picker_enter_selects_and_focuses_name(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("create-branch-source-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: true,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                let search = host
                    .branch_picker_search_input
                    .as_ref()
                    .expect("branch picker search input");
                search.update(cx, |input, cx| input.set_text("", cx));
                let focus = search.read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(host.create_branch_source_target, "main");
        assert_eq!(
            host.branch_picker_search_input
                .as_ref()
                .expect("branch picker search input")
                .read(app)
                .text(),
            "main"
        );
        assert_window_focus(
            window,
            app,
            host.create_branch_input.read(app).focus_handle(),
            "expected Enter on a source branch to focus the new branch name",
        );
    });
}

#[gpui::test]
fn worktree_ref_picker_click_selects_and_focuses_add(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("worktree-ref-click");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::Repo {
                        repo_id,
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.worktree_path_input
                    .update(cx, |input, cx| input.set_text("/tmp/worktree", cx));
                let search = host
                    .branch_picker_search_input
                    .as_ref()
                    .expect("branch picker search input");
                let focus = search.read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    click_debug_selector(cx, "picker_prompt_item_1");

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(host.worktree_ref_source_target, "main");
        assert_eq!(
            host.branch_picker_search_input
                .as_ref()
                .expect("branch picker search input")
                .read(app)
                .text(),
            "main"
        );
        assert_window_focus(
            window,
            app,
            host.worktree_focus.submit.clone(),
            "expected clicking a worktree ref to focus Add",
        );
    });
}

#[gpui::test]
fn worktree_ref_picker_enter_selects_and_focuses_add(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("worktree-ref-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::Repo {
                        repo_id,
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.worktree_path_input
                    .update(cx, |input, cx| input.set_text("/tmp/worktree", cx));
                let search = host
                    .branch_picker_search_input
                    .as_ref()
                    .expect("branch picker search input");
                search.update(cx, |input, cx| input.set_text("", cx));
                let focus = search.read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            matches!(
                host.popover,
                Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                })
            ),
            "expected selecting a worktree ref with Enter to keep the dialog open"
        );
        assert_eq!(host.worktree_ref_source_target, "main");
        assert_window_focus(
            window,
            app,
            host.worktree_focus.submit.clone(),
            "expected Enter on a worktree ref to focus Add",
        );
    });
}

#[gpui::test]
fn rename_branch_prompt_cancel_button_and_escape_close(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    let open_prompt =
        |window: &mut gpui::Window, app: &mut gpui::App, view: &Entity<GitCometView>| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        PopoverKind::RenameBranchPrompt {
                            repo_id: RepoId(1),
                            name: "feature/current".to_string(),
                            is_current_branch: true,
                        },
                        gpui::point(gpui::px(120.0), gpui::px(72.0)),
                        window,
                        cx,
                    );
                });
            });
        };

    cx.update(|window, app| open_prompt(window, app, &view));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        !cx.update(|_window, app| view.read(app).popover_host.read(app).is_open()),
        "expected Escape to close rename-branch prompt"
    );

    cx.update(|window, app| open_prompt(window, app, &view));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    click_debug_selector(cx, "rename_branch_cancel_hint");
    assert!(
        !cx.update(|_window, app| view.read(app).popover_host.read(app).is_open()),
        "expected clicking Cancel to close rename-branch prompt"
    );
}

#[gpui::test]
fn create_branch_popover_renders_shortcut_hints_and_separators(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("create-branch-shortcuts");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: false,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("create_branch_from_ref_cancel_hint")
        .expect("expected create-branch Cancel shortcut hint");
    cx.debug_bounds("create_branch_from_ref_go_hint")
        .expect("expected create-branch Create shortcut hint");
    cx.debug_bounds("create_branch_from_ref_cancel_end_slot_separator")
        .expect("expected create-branch Cancel shortcut separator");
    cx.debug_bounds("create_branch_from_ref_go_end_slot_separator")
        .expect("expected create-branch Create shortcut separator");
}

#[gpui::test]
fn create_branch_from_ref_popover_tabs_to_checkout_and_wraps(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id: RepoId(1),
                        target: "main".to_string(),
                        source_selectable: false,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.create_branch_input
                    .update(cx, |input, cx| input.set_text("feature", cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_input.read(app).focus_handle(),
            "expected create-branch-from-ref to focus the name input first",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_from_ref_checkout_focus_handle.clone(),
            "expected Tab to move from the name input to Checkout",
        );
    });

    simulate_key_press(cx, "space");
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert!(
            !host.create_branch_checkout_enabled,
            "expected Space to toggle Checkout off"
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_from_ref_focus.cancel.clone(),
            "expected Tab to move from Checkout to Cancel",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_from_ref_focus.submit.clone(),
            "expected Tab to move from Cancel to Create",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_input.read(app).focus_handle(),
            "expected Tab to wrap from Create back to the name input",
        );
    });

    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.create_branch_from_ref_focus.submit.clone(),
            "expected Shift-Tab to wrap from the name input back to Create",
        );
    });
}

#[gpui::test]
fn create_branch_popover_enter_creates_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-branch-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: false,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("create_branch_btn"));
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.create_branch_input
                    .update(cx, |input, cx| input.set_text("feature", cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Enter to close create-branch popover");
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, None);
    });

    wait_until("create-branch repo actions", || {
        repo.actions() == vec!["create:feature".to_string(), "checkout:feature".to_string()]
    });
}

#[gpui::test]
fn create_branch_popover_enter_with_empty_input_does_not_close_or_create(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events, repo, _workdir) = create_tracking_store("create-branch-empty-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
                        source_selectable: false,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("create_branch_btn"));
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        is_open,
        "expected Enter to respect the disabled Create action when the name is empty"
    );
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("create_branch_btn"));
    });

    std::thread::sleep(Duration::from_millis(100));
    assert!(
        repo.actions().is_empty(),
        "expected empty input to avoid create-branch actions"
    );
}

mod checkout_picker {
    use super::super::super::branch_picker::{self, BranchPickerNavTarget};
    use super::*;
    use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};
    use crate::view::test_support::{push_test_state, redraw};
    use gitcomet_core::domain::{RefMetadata, RemoteBranch};
    use gitcomet_state::model::{RepoId, RepoState};
    use std::collections::HashMap;

    fn commit_id(hex: &str) -> CommitId {
        CommitId(Arc::from(hex))
    }

    fn local(name: &str) -> Branch {
        Branch {
            name: name.to_string(),
            target: commit_id("399f41d0000000000000000000000000000000aa"),
            upstream: None,
            divergence: None,
        }
    }

    fn remote(remote: &str, name: &str) -> RemoteBranch {
        RemoteBranch {
            remote: remote.to_string(),
            name: name.to_string(),
            target: commit_id("a12bc3d0000000000000000000000000000000bb"),
        }
    }

    /// A repo on `main` with two local branches, two remote refs (one of which
    /// is the `origin/HEAD` symref), and loaded ref metadata.
    fn repo_with_branches(repo_id: RepoId) -> RepoState {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/branches/main"));
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![local("main"), local("feat/badges")]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![
            remote("origin", "main"),
            remote("origin", "HEAD"),
        ]));
        repo.ref_metadata = Loadable::Ready(Arc::new(HashMap::from([
            (
                "main".to_string(),
                RefMetadata {
                    author: "Ada Lovelace".to_string(),
                    committed_at: 1_754_870_400,
                    summary: "improve font loading".to_string(),
                },
            ),
            (
                "origin/main".to_string(),
                RefMetadata {
                    author: "Ada Lovelace".to_string(),
                    committed_at: 1_754_870_400,
                    summary: "improve font loading".to_string(),
                },
            ),
        ])));
        repo
    }

    fn open_checkout_picker(
        cx: &mut gpui::TestAppContext,
        repo: RepoState,
        repo_id: RepoId,
    ) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            crate::app::bind_text_input_keys_for_test(app);
            view.update(app, |this, cx| {
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
            let _ = window.draw(app);
        });
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Checkout,
                        },
                        gpui::point(gpui::px(120.0), gpui::px(72.0)),
                        window,
                        cx,
                    );
                });
            });
        });
        redraw(cx);

        (view, cx)
    }

    #[gpui::test]
    fn lists_local_and_remote_branches_and_marks_head(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let (rows, marked_index) = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            let built = branch_picker::cached(host, "");
            (built.payloads.to_vec(), built.marked_index)
        });

        assert_eq!(
            rows,
            vec![
                BranchPickerNavTarget::Ref("main".to_string()),
                BranchPickerNavTarget::Ref("feat/badges".to_string()),
                BranchPickerNavTarget::RemoteBranch {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                },
            ],
            "origin/HEAD is a symref and must not be listed as a branch"
        );
        assert_eq!(
            marked_index,
            Some(0),
            "the check belongs on the checked-out branch, indexed before filtering"
        );

        redraw(cx);
        assert!(
            cx.debug_bounds("picker_prompt_item_0").is_some(),
            "expected branch rows to render"
        );
    }

    /// The ref name is the row's title; who last touched it and what they said
    /// belongs on a second, quieter line. Every branch row gets that line — one
    /// saying "No commits found" beats a short row in a list of tall ones — so the
    /// list keeps a single row pitch throughout.
    #[gpui::test]
    fn every_branch_row_carries_a_detail_line_of_the_same_height(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let (with_metadata, without_metadata) = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            let built = branch_picker::cached(host, "");
            (
                built.items[0].debug_secondary_text(),
                built.items[1].debug_secondary_text(),
            )
        });

        assert!(
            with_metadata.contains("Ada Lovelace")
                && with_metadata.contains("improve font loading"),
            "author and summary belong on the detail line, got {with_metadata:?}"
        );
        // The fixture has metadata for `main` but not for `feat/badges`.
        assert_eq!(
            without_metadata, "No commits found",
            "a branch with no metadata still gets a detail line"
        );

        redraw(cx);
        let titled = cx
            .debug_bounds("picker_prompt_item_0")
            .expect("expected the branch row to render")
            .size
            .height;
        let plain = cx
            .debug_bounds("picker_prompt_item_1")
            .expect("expected the metadata-less branch row to render")
            .size
            .height;
        assert_eq!(
            titled, plain,
            "rows must share one height so the list does not look ragged"
        );
    }

    /// A backend that does not implement ref metadata latches an empty map rather
    /// than an error, and the trait's contract is that callers fall back to
    /// name-only rows. "No commits found" on every single row would be a worse
    /// answer than no detail line at all.
    #[gpui::test]
    fn rows_stay_name_only_when_the_backend_has_no_ref_metadata(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let mut repo = repo_with_branches(repo_id);
        repo.ref_metadata = Loadable::Ready(Arc::new(HashMap::new()));
        let (view, cx) = open_checkout_picker(cx, repo, repo_id);

        let details = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::cached(host, "")
                .items
                .iter()
                .map(|item| item.debug_secondary_text())
                .collect::<Vec<_>>()
        });

        assert!(
            details.iter().all(|detail| detail.is_empty()),
            "an empty metadata map means the backend has none, not that every branch lacks commits: {details:?}"
        );
    }

    /// The checked-out branch is marked by its leading icon becoming a check, so
    /// no second check is drawn at the row's trailing edge.
    #[gpui::test]
    fn the_checked_out_branch_carries_its_check_in_the_icon_slot(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (_view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        redraw(cx);

        // `main` is HEAD in this fixture, so row 0 is the marked one.
        assert!(
            cx.debug_bounds("picker_prompt_item_icon_0").is_some(),
            "the marked row must still render an icon slot"
        );
        assert!(
            cx.debug_bounds("picker_prompt_item_trailing_check_0")
                .is_none(),
            "a row that turned its icon into a check must not also draw a trailing one"
        );
        assert!(
            cx.debug_bounds("picker_prompt_item_icon_1").is_some(),
            "unmarked rows keep their own branch icon"
        );
    }

    #[gpui::test]
    fn nav_targets_follow_the_rendered_row_order(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let (targets, rendered) = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            let query = "main";
            let targets = branch_picker::nav_targets(host, query);
            let built = branch_picker::cached(host, query);
            // What the panel renders, resolved independently of the cached layout.
            let layout = crate::view::components::picker_prompt_layout(&built.items, query);
            let rendered: Vec<_> = layout
                .item_indices
                .iter()
                .map(|ix| built.payloads[*ix].clone())
                .collect();
            (targets, rendered)
        });

        // Sections and multi-part rows sort differently from a plain name list;
        // if these drift, Enter checks out a branch other than the highlighted one.
        assert_eq!(
            targets, rendered,
            "keyboard order must match the rendered order"
        );
    }

    #[gpui::test]
    fn create_row_appears_for_an_unknown_name_and_survives_filtering(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::nav_targets(host, "brand-new")
        });

        assert_eq!(
            targets,
            vec![BranchPickerNavTarget::CreateBranch("brand-new".to_string())],
            "create row must stay reachable for a name that matches nothing"
        );
    }

    #[gpui::test]
    fn create_row_is_offered_when_only_a_remote_branch_matches(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let mut repo = repo_with_branches(repo_id);
        // Only a remote branch is named "release"; creating a local one is legal.
        repo.remote_branches = Loadable::Ready(Arc::new(vec![remote("origin", "release")]));
        let (view, cx) = open_checkout_picker(cx, repo, repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::nav_targets(host, "release")
        });

        assert!(
            targets.contains(&BranchPickerNavTarget::CreateBranch("release".to_string())),
            "a query matching only a remote branch should still offer creation: {targets:?}"
        );
    }

    #[gpui::test]
    fn create_row_is_hidden_when_the_query_names_an_existing_local_branch(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::nav_targets(host, "main")
        });

        assert!(
            !targets
                .iter()
                .any(|t| matches!(t, BranchPickerNavTarget::CreateBranch(_))),
            "must not offer to create a branch that already exists: {targets:?}"
        );
    }

    #[gpui::test]
    fn remote_row_hands_off_to_the_local_name_prompt(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    branch_picker::activate(
                        host,
                        repo_id,
                        BranchPickerNavTarget::RemoteBranch {
                            remote: "origin".to_string(),
                            branch: "main".to_string(),
                        },
                        window,
                        cx,
                    );
                });
            });
        });
        redraw(cx);

        let kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert!(
            matches!(
                kind,
                Some(PopoverKind::CheckoutRemoteBranchPrompt { ref remote, ref branch, .. })
                    if remote == "origin" && branch == "main"
            ),
            "expected the remote checkout prompt, got {kind:?}"
        );
    }

    #[gpui::test]
    fn metadata_columns_are_not_searchable(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            // "Lovelace" is the author on every row; filtering is by branch
            // name only, so it must not pull branches in.
            branch_picker::nav_targets(host, "Lovelace")
        });

        assert_eq!(
            targets,
            vec![BranchPickerNavTarget::CreateBranch("Lovelace".to_string())],
            "author/date/summary must not participate in filtering: {targets:?}"
        );
    }

    /// A repo with enough branches that the list scrolls several viewports.
    fn repo_with_many_branches(repo_id: RepoId, branches: usize) -> RepoState {
        let mut repo = repo_with_branches(repo_id);
        let mut locals = vec![local("main")];
        locals.extend((0..branches).map(|ix| local(&format!("feat/topic-{ix:04}"))));
        repo.branches = Loadable::Ready(Arc::new(locals));
        repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
        repo
    }

    /// The windowed list stands spacers in for the rows it does not render, sized
    /// from the geometry alone. If that arithmetic drifted from what rows really
    /// paint at, scrolling would drift with it — so this pins one against the
    /// other.
    #[gpui::test]
    fn row_geometry_matches_the_height_rows_actually_paint_at(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);
        redraw(cx);

        let geometry = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            let rows = branch_picker::cached(host, "");
            components::PickerPromptGeometry::new(&rows.items, &rows.layout, 100u32)
        });

        let first = cx
            .debug_bounds("picker_prompt_item_0")
            .expect("expected the first row to render");
        let second = cx
            .debug_bounds("picker_prompt_item_1")
            .expect("expected the second row to render");

        assert_eq!(
            first.size.height,
            geometry.row_height(0),
            "a painted row must be exactly as tall as the geometry says"
        );
        assert_eq!(
            second.origin.y - first.origin.y,
            geometry.row_top(1) - geometry.row_top(0),
            "the stride between rows must match the geometry"
        );
    }

    #[gpui::test]
    fn a_long_branch_list_renders_only_the_rows_in_view(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_many_branches(repo_id, 200), repo_id);
        redraw(cx);

        let matched = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::cached(host, "").layout.item_indices.len()
        });

        assert!(matched > 200, "expected a long list, matched {matched}");
        assert!(
            cx.debug_bounds("picker_prompt_item_0").is_some(),
            "the first row is in view"
        );
        assert!(
            cx.debug_bounds("picker_prompt_item_150").is_none(),
            "a row 150 places down must not be built until it is scrolled to"
        );
    }

    /// Windowing must be invisible until you scroll: the spacer standing in for
    /// the rows above the window covers those rows only, not the list's own
    /// padding. Counting the padding twice would nudge every row down and grow
    /// the scrollable height the moment a list got long enough to be windowed.
    #[gpui::test]
    fn windowing_does_not_move_the_first_row(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (_view, cx) = open_checkout_picker(cx, repo_with_many_branches(repo_id, 200), repo_id);
        redraw(cx);
        let windowed = cx
            .debug_bounds("picker_prompt_item_0")
            .expect("expected the first row of the long list to render");

        let (_view, cx) = open_checkout_picker(cx, repo_with_many_branches(repo_id, 2), repo_id);
        redraw(cx);
        let short = cx
            .debug_bounds("picker_prompt_item_0")
            .expect("expected the first row of the short list to render");

        assert_eq!(
            windowed.origin, short.origin,
            "a windowed list must start its rows where an unwindowed one does"
        );
        assert_eq!(windowed.size.height, short.size.height);
    }

    /// Arrow-up on a freshly opened picker selects the last row. It is far
    /// outside the window, so it only comes into view if navigation scrolls by
    /// the row geometry rather than by a scroll child that does not exist.
    #[gpui::test]
    fn keyboard_navigation_reaches_a_row_outside_the_window(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_many_branches(repo_id, 200), repo_id);
        redraw(cx);

        simulate_key_press(cx, "up");
        redraw(cx);

        let (selected, row_count, last_item_index) = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            let rows = branch_picker::cached(host, "");
            (
                host.branch_picker_selected_index,
                rows.layout.item_indices.len(),
                rows.layout.item_indices.last().copied().unwrap_or_default(),
            )
        });

        assert_eq!(
            selected,
            Some(row_count - 1),
            "arrow-up from an unopened selection lands on the last row"
        );
        // "main" sorts first on name length, then the 200 topics in order, so the
        // last row is the last item that was built.
        assert_eq!(last_item_index, 200);
        assert!(
            cx.debug_bounds("picker_prompt_item_200").is_some(),
            "the selected last row must be scrolled into view and rendered"
        );
    }

    #[gpui::test]
    fn rows_render_without_metadata_before_it_loads(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let mut repo = repo_with_branches(repo_id);
        repo.ref_metadata = Loadable::NotLoaded;
        let (view, cx) = open_checkout_picker(cx, repo, repo_id);

        let rows = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::cached(host, "").payloads.to_vec()
        });

        assert_eq!(rows.len(), 3, "picker must be usable before metadata lands");
        redraw(cx);
        assert!(cx.debug_bounds("picker_prompt_item_0").is_some());
    }

    #[gpui::test]
    fn opening_the_picker_requests_ref_metadata_once(cx: &mut gpui::TestAppContext) {
        // End-to-end: the badge's picker is what triggers the on-demand load,
        // and a backend that cannot supply it must not raise an error banner.
        let (store, events, _repo, _workdir) = create_tracking_store("branch-picker-ref-metadata");
        let repo_id = store.snapshot().active_repo.expect("expected active repo");
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            crate::app::bind_text_input_keys_for_test(app);
            let _ = window.draw(app);
        });

        let ref_metadata_untouched = |snapshot: &Arc<gitcomet_state::model::AppState>| {
            snapshot
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .is_some_and(|r| matches!(r.ref_metadata, Loadable::NotLoaded))
        };
        assert!(
            ref_metadata_untouched(&store.snapshot()),
            "metadata should not be fetched until a picker that shows it opens"
        );
        let diagnostics_before = store
            .snapshot()
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .map(|r| r.diagnostics.len())
            .unwrap_or(0);

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Checkout,
                        },
                        gpui::point(gpui::px(120.0), gpui::px(72.0)),
                        window,
                        cx,
                    );
                });
            });
        });

        wait_until("ref metadata load to be requested", || {
            !ref_metadata_untouched(&store.snapshot())
        });

        // TrackingRepo inherits the trait default (Unsupported), so this lands
        // on the error path — which must stay silent.
        let after = store.snapshot();
        let repo_state = after
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .expect("repo state");
        assert_eq!(
            repo_state.diagnostics.len(),
            diagnostics_before,
            "an unsupported metadata backend must not push a diagnostic"
        );
    }

    #[gpui::test]
    fn create_row_is_suppressed_while_the_branch_list_is_still_loading(
        cx: &mut gpui::TestAppContext,
    ) {
        // With an empty local list, "switch to main" would become "create main",
        // which git rejects.
        let repo_id = RepoId(1);
        let mut repo = repo_with_branches(repo_id);
        repo.branches = Loadable::Loading;
        let (view, cx) = open_checkout_picker(cx, repo, repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::nav_targets(host, "main")
        });

        assert!(
            !targets
                .iter()
                .any(|t| matches!(t, BranchPickerNavTarget::CreateBranch(_))),
            "must not offer creation before the branch list is known: {targets:?}"
        );
    }

    #[gpui::test]
    fn create_row_is_hidden_for_a_case_variant_of_an_existing_branch(
        cx: &mut gpui::TestAppContext,
    ) {
        // `match_items` filters case-insensitively, so "MAIN" shows the `main`
        // row; offering to create "MAIN" too would collide on case-insensitive
        // filesystems.
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        let targets = cx.update(|_window, app| {
            let host = view.read(app).popover_host.read(app);
            branch_picker::nav_targets(host, "MAIN")
        });

        assert!(
            !targets
                .iter()
                .any(|t| matches!(t, BranchPickerNavTarget::CreateBranch(_))),
            "must not offer to create a case-variant duplicate: {targets:?}"
        );
    }

    #[gpui::test]
    fn enter_reaches_the_create_row_without_arrowing(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        cx.simulate_input("brand-new");
        simulate_key_press(cx, "enter");
        cx.run_until_parked();

        // Creating closes the picker; nothing else in this picker does that for
        // a query matching no existing branch.
        let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
        assert!(
            !is_open,
            "Enter after typing an unknown name should create and dismiss"
        );
    }

    #[gpui::test]
    fn delete_picker_keeps_the_plain_list(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = open_checkout_picker(cx, repo_with_branches(repo_id), repo_id);

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Delete,
                        },
                        gpui::point(gpui::px(120.0), gpui::px(72.0)),
                        window,
                        cx,
                    );
                });
            });
        });
        redraw(cx);

        // The delete picker must not gain remote branches or a create row.
        let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
        assert!(is_open, "expected the delete picker to open");
        assert!(
            cx.debug_bounds("picker_prompt_item_0").is_some(),
            "expected the plain delete list to render"
        );
    }
}
