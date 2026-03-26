use super::*;
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitId, LogPage, ReflogEntry, RepoSpec, RepoStatus, StashEntry,
};
use gitcomet_core::services::{CommandOutput, PullMode};
use gitcomet_state::model::Loadable;
use gitcomet_state::msg::{Msg, StoreEvent};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

    fn create_tag_with_output(&self, name: &str, target: &str) -> Result<CommandOutput> {
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
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_windows_verbatim_prefix(path)
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };

    let mut out = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:", char::from(letter))),
        Prefix::VerbatimUNC(server, share) => {
            let mut out = PathBuf::from(r"\\");
            out.push(server);
            out.push(share);
            out
        }
        Prefix::Verbatim(raw) => PathBuf::from(raw),
        _ => return path,
    };

    for component in components {
        out.push(component.as_os_str());
    }
    out
}

#[cfg(not(windows))]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
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
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
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
fn create_branch_popover_renders_shortcut_hints_and_separators(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("create-branch-shortcuts");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
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

    cx.debug_bounds("create_branch_cancel_hint")
        .expect("expected create-branch Cancel shortcut hint");
    cx.debug_bounds("create_branch_go_hint")
        .expect("expected create-branch Create shortcut hint");
    cx.debug_bounds("create_branch_cancel_end_slot_separator")
        .expect("expected create-branch Cancel shortcut separator");
    cx.debug_bounds("create_branch_go_end_slot_separator")
        .expect("expected create-branch Create shortcut separator");
}

#[gpui::test]
fn create_branch_popover_enter_creates_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-branch-enter");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
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
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_active_context_menu_invoker(Some("create_branch_btn".into()), cx);
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
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
