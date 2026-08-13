//! The two worktree pickers — open one, or pick one to remove.
//!
//! They list the same worktrees and differ only in how a row reads and what
//! activating one does, so they share everything but that: one row build, one
//! cache slot, one search input, one selection index. The repository's own
//! workdir is not among them — it is where you already are.

use super::*;
use std::rc::Rc;

/// A row for the open picker, which names the branch a worktree has checked out
/// ahead of its path. The remove picker lists the paths alone.
fn worktree_picker_item(
    branch: Option<&str>,
    detached: bool,
    path: &std::path::Path,
) -> components::PickerPromptItem {
    let mut parts = Vec::new();
    if let Some(branch) = branch {
        parts.push(
            components::PickerPromptItemPart::new(branch.to_owned())
                .profile(components::TextTruncationProfile::End)
                .flexible(false),
        );
        parts.push(components::PickerPromptItemPart::separator("  "));
    } else if detached {
        parts.push(
            components::PickerPromptItemPart::new("(detached)")
                .profile(components::TextTruncationProfile::End)
                .flexible(false),
        );
        parts.push(components::PickerPromptItemPart::separator("  "));
    }
    parts.push(components::PickerPromptItemPart::path(
        path.display().to_string(),
    ));
    components::PickerPromptItem::from_parts(parts)
}

/// Height these pickers cap their row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it, which builds
/// its geometry for exactly this viewport.
pub(super) const WORKTREE_PICKER_LIST_MAX_HEIGHT_PX: f32 = 260.0;

fn repo_for(this: &PopoverHost, repo_id: RepoId) -> Option<&RepoState> {
    this.state.repos.iter().find(|repo| repo.id == repo_id)
}

/// Everything the rows below read, `is_remove` included: the two pickers draw
/// the same worktrees as different rows, so they cannot share a cache entry.
fn rows_signature(this: &PopoverHost, repo_id: RepoId, is_remove: bool) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        is_remove.hash(hasher);
        let Some(repo) = repo_for(this, repo_id) else {
            return;
        };
        repo.id.hash(hasher);
        repo.worktrees_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.worktrees).hash(hasher);
        // The row for the repository's own workdir is dropped, so moving it
        // changes which rows there are.
        repo.spec.workdir.hash(hasher);
    })
}

/// The rows for `query`, built once per change to the worktree list.
pub(super) fn cached(
    this: &PopoverHost,
    repo_id: RepoId,
    is_remove: bool,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<std::path::PathBuf>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Worktree,
        rows_signature(this, repo_id, is_remove),
        query,
    );
    super::rows_cache::get_or_build(&this.worktree_picker_rows_cache, key, |_now| {
        let Some(repo) = repo_for(this, repo_id) else {
            return (Vec::new(), Vec::new(), None);
        };
        let Loadable::Ready(worktrees) = &repo.worktrees else {
            return (Vec::new(), Vec::new(), None);
        };
        let (items, payloads) = worktrees
            .iter()
            .filter(|worktree| worktree.path != repo.spec.workdir)
            .map(|worktree| {
                let item = if is_remove {
                    components::PickerPromptItem::single(
                        worktree.path.display().to_string(),
                        components::TextTruncationProfile::Path,
                    )
                } else {
                    worktree_picker_item(
                        worktree.branch.as_deref(),
                        worktree.detached,
                        &worktree.path,
                    )
                };
                (item, worktree.path.clone())
            })
            .unzip();
        (items, payloads, None)
    })
}

pub(super) fn nav_targets(
    this: &PopoverHost,
    repo_id: RepoId,
    is_remove: bool,
    query: &str,
) -> Vec<std::path::PathBuf> {
    cached(this, repo_id, is_remove, query).filtered_payloads()
}

/// Opens the worktree, or the confirmation for removing it. Shared by the click
/// handler and by Enter, so the two cannot drift apart — `position` is where the
/// click landed, and `None` for a keyboard activation, which has nowhere in
/// particular to anchor and centres instead.
pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    is_remove: bool,
    path: std::path::PathBuf,
    position: Option<gpui::Point<gpui::Pixels>>,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    if is_remove {
        let confirm = PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::RemoveConfirm { path, branch: None },
        );
        match position {
            Some(position) => this.open_popover_at(confirm, position, window, cx),
            None => this.open_popover_centered(confirm, window, cx),
        }
        return;
    }
    this.store.dispatch(Msg::OpenRepo(path));
    this.close_popover(cx);
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    is_remove: bool,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let width = super::LARGE_PICKER_WIDTH;
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let label =
        |this: &PopoverHost, text: gpui::SharedString, cx: &mut gpui::Context<PopoverHost>| {
            components::context_menu_label(
                theme,
                ui_scale_percent,
                text,
                Some(this.tooltip_host.clone()),
                cx,
            )
        };

    let Some(repo) = repo_for(this, repo_id) else {
        return label(this, "No repository".into(), cx);
    };
    match &repo.worktrees {
        Loadable::Loading => return label(this, "Loading".into(), cx),
        Loadable::NotLoaded => return label(this, "Not loaded".into(), cx),
        Loadable::Error(e) => {
            let e = e.clone();
            return label(this, e.into(), cx);
        }
        Loadable::Ready(_) => {}
    }

    let Some(search) = this.worktree_picker_search_input.clone() else {
        return label(this, "Search input not initialized".into(), cx);
    };
    let query = search.read(cx).text().trim().to_string();
    let built = cached(this, repo_id, is_remove, &query);
    let paths = Rc::clone(&built.payloads);

    components::context_menu(
        theme,
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
            .tooltip_host(this.tooltip_host.clone())
            .empty_text("No worktrees")
            .max_height(scaled_px(WORKTREE_PICKER_LIST_MAX_HEIGHT_PX))
            .selected_index(this.worktree_picker_selected_index)
            .render(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, e, window, cx| {
                    let Some(path) = paths.get(ix).cloned() else {
                        return;
                    };
                    activate(
                        this,
                        repo_id,
                        is_remove,
                        path,
                        Some(e.position()),
                        window,
                        cx,
                    );
                },
            ),
    )
    .w(width.preferred_px(ui_scale))
}
