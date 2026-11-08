use super::status_nav::status_navigation_context_for_repo;
use super::*;
use crate::view::panes::diff::helpers::state::{NextFileAdvance, OpenFileChange};
use gitcomet_state::model::LoadKind;

/// What a dispatch did to the open file, carried to the deferred arming.
struct ArmedChange {
    repo_id: RepoId,
    path: std::path::PathBuf,
    area: DiffArea,
    change: OpenFileChange,
    /// `ops_rev` before the action began.
    ops_rev: u64,
}

impl DiffRenderer {
    pub(in crate::view) fn set_open_next_file_after_last_change(&mut self, enabled: bool) {
        self.open_next_file_after_last_change = enabled;
        if !enabled {
            self.next_file_advance = None;
        }
    }

    /// Sends a Stage, Unstage or Discard, and arms the next-file advance when
    /// it changes the open working-tree file.
    pub(in crate::view) fn dispatch_open_file_change(
        &mut self,
        msg: Msg,
        cx: &mut gpui::Context<Self>,
    ) {
        let armed = self.arm_for(&msg);
        self.store.dispatch(msg);
        let Some(armed) = armed else {
            return;
        };
        // Deferred: the list order is read from the root view, which may be
        // updating us right now (keys pressed outside the diff).
        let pane = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = pane.update(cx, |pane, cx| pane.arm_next_file_advance(armed, cx));
        });
    }

    /// What `msg` does to the open working-tree file, or `None` when it
    /// leaves that file alone or the advance is off.
    fn arm_for(&self, msg: &Msg) -> Option<ArmedChange> {
        if !self.open_next_file_after_last_change || self.hosted_decor.is_some() {
            return None;
        }
        let repo = self.active_repo()?;
        let Some(DiffTarget::WorkingTree {
            path: open, area, ..
        }) = self.bound_diff_state(repo).diff_target.as_ref()
        else {
            return None;
        };
        let change = match msg {
            Msg::StageHunk { repo_id, .. }
            | Msg::UnstageHunk { repo_id, .. }
            | Msg::ApplyWorktreePatch { repo_id, .. }
            | Msg::ApplyPreparedDiffPatch { repo_id, .. } => {
                (*repo_id == repo.id).then_some(OpenFileChange::Part)
            }
            Msg::DiscardWorktreeChangesPath { repo_id, path } => {
                (*repo_id == repo.id && path == open).then_some(OpenFileChange::WholeFile)
            }
            Msg::DiscardWorktreeChangesPaths { repo_id, paths } => {
                (*repo_id == repo.id && paths.contains(open)).then_some(OpenFileChange::WholeFile)
            }
            _ => None,
        }?;
        Some(ArmedChange {
            repo_id: repo.id,
            path: open.clone(),
            area: *area,
            change,
            ops_rev: repo.ops_rev,
        })
    }

    fn arm_next_file_advance(&mut self, armed: ArmedChange, cx: &mut gpui::Context<Self>) {
        self.next_file_advance = None;
        let view = self.active_change_tracking_view(cx);
        let order = self.active_status_section_order(armed.repo_id, view, cx);
        let Some(repo) = self.active_repo().filter(|repo| repo.id == armed.repo_id) else {
            return;
        };
        let target = DiffTarget::working_tree(armed.path.clone(), armed.area);
        let Some((section, next)) =
            status_navigation_context_for_repo(repo, &target, view, order.as_deref()).and_then(
                |navigation| Some((navigation.section(), navigation.next_or_prev_path()?)),
            )
        else {
            return;
        };
        self.next_file_advance = Some(NextFileAdvance {
            repo_id: armed.repo_id,
            path: armed.path,
            area: armed.area,
            change: armed.change,
            next,
            section,
            ops_rev: armed.ops_rev,
        });
    }

    /// Runs on every new pane input. Once the armed action finished and the
    /// status after it loaded, decides once: opens the neighbour if the file
    /// has left its list. Showing another file disarms it.
    pub(in crate::view) fn resolve_next_file_advance(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(advance) = self.next_file_advance.take() else {
            return;
        };
        let Some(repo) = self.active_repo().filter(|repo| repo.id == advance.repo_id) else {
            return;
        };
        let still_shown = match self.bound_diff_state(repo).diff_target.as_ref() {
            Some(DiffTarget::WorkingTree { path, area, .. }) => {
                *path == advance.path && *area == advance.area
            }
            Some(_) => false,
            None => advance.change == OpenFileChange::WholeFile,
        };
        if !still_shown {
            return;
        }
        let status_loading = repo.loads_in_flight.is_in_flight(LoadKind::WorktreeStatus)
            || repo.loads_in_flight.is_in_flight(LoadKind::StagedStatus);
        if repo.ops_rev == advance.ops_rev || repo.local_actions_in_flight > 0 || status_loading {
            self.next_file_advance = Some(advance);
            return;
        }
        if repo
            .status_entry_for_path(advance.area, &advance.path)
            .is_some()
        {
            return;
        }
        let Some(next) = repo.status_entry_for_path(advance.area, &advance.next) else {
            return;
        };
        let is_conflicted = next.kind == gitcomet_core::domain::FileStatusKind::Conflicted;
        // Deferred: this runs while the window applies a new state.
        let pane = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = pane.update(cx, |pane, cx| {
                pane.open_status_file(
                    advance.repo_id,
                    advance.section,
                    advance.area,
                    advance.next,
                    is_conflicted,
                    cx,
                );
            });
        });
    }
}
