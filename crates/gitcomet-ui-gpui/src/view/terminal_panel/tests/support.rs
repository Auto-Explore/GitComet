use super::super::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository};

// A 20x6 grid of 6x12 cells, so the viewport is 120x72 at (100, 200).
pub(super) const TEST_COLS: usize = 20;
pub(super) const TEST_ROWS: usize = 6;
pub(super) const TEST_CELL_W: f32 = 6.0;
pub(super) const TEST_LINE_H: f32 = 12.0;
pub(super) const TEST_SCROLLBACK: usize = 100;

pub(super) struct TerminalTestBackend;

impl GitBackend for TerminalTestBackend {
    fn open(
        &self,
        _workdir: &std::path::Path,
    ) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "terminal test backend does not open repositories",
        )))
    }
}

pub(super) fn test_root_view_with_active_repo(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, RepoId, &mut gpui::VisualTestContext) {
    let repo_id = RepoId(1);
    let (store, events) = AppStore::new(Arc::new(TerminalTestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = Arc::new(AppState {
        repos: vec![RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/terminal-exit-test"),
            },
        )],
        active_repo: Some(repo_id),
        ..AppState::default()
    });
    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.state = state);
    });
    (view, repo_id, cx)
}

pub(super) fn test_terminal_instance(
    session_seq: u64,
    events_rx: Option<smol::channel::Receiver<TerminalBackendEvent>>,
    cx: &mut gpui::Context<GitCometView>,
) -> TerminalInstance {
    let focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
    let viewport_focus = focus_handle.clone();
    let viewport = cx.new(move |_cx| {
        TerminalViewportView::with_backend(AppTheme::gitcomet_dark(), viewport_focus, None, None)
    });
    TerminalInstance {
        focus_handle,
        pty_sender: None,
        child_pid: None,
        events_rx,
        connected: true,
        viewport,
        session_seq,
        title: format!("terminal-{session_seq}"),
    }
}

pub(super) fn test_terminal_session(
    tabs: Vec<(u64, Option<smol::channel::Receiver<TerminalBackendEvent>>)>,
    active_index: usize,
    cx: &mut gpui::Context<GitCometView>,
) -> RepoTerminalSession {
    RepoTerminalSession {
        workdir: PathBuf::from("/tmp/terminal-exit-test"),
        repo_name: "terminal-exit-test".to_string(),
        instances: tabs
            .into_iter()
            .map(|(session_seq, events_rx)| test_terminal_instance(session_seq, events_rx, cx))
            .collect(),
        active_index,
    }
}
