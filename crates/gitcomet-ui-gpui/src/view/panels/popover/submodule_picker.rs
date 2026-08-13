//! The two submodule pickers — open one, or pick one to remove.
//!
//! They list the same rows and differ only in what activating one does, so they
//! share a row build, a cache slot, a search input and a selection index.

use super::*;
use std::rc::Rc;

/// Height these pickers cap their row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it: the list is
/// windowed once it outgrows a couple of viewports, and it is built for exactly
/// this viewport, so a navigation assuming another would scroll to the wrong
/// place.
pub(super) const SUBMODULE_PICKER_LIST_MAX_HEIGHT_PX: f32 = 260.0;

fn repo_for(this: &PopoverHost, repo_id: RepoId) -> Option<&RepoState> {
    this.state.repos.iter().find(|repo| repo.id == repo_id)
}

/// Everything the rows below read.
fn rows_signature(this: &PopoverHost, repo_id: RepoId) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        let Some(repo) = repo_for(this, repo_id) else {
            return;
        };
        repo.id.hash(hasher);
        repo.submodules_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.submodules).hash(hasher);
    })
}

/// The rows for `query`, built once per change to the submodule list. Payloads
/// are the submodule's path relative to the repository — the open picker joins
/// it onto the workdir when it acts, so both pickers can share one build.
pub(super) fn cached(
    this: &PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<std::path::PathBuf>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Submodule,
        rows_signature(this, repo_id),
        query,
    );
    super::rows_cache::get_or_build(&this.submodule_picker_rows_cache, key, |_now| {
        let Some(Loadable::Ready(submodules)) =
            repo_for(this, repo_id).map(|repo| &repo.submodules)
        else {
            return (Vec::new(), Vec::new(), None);
        };
        let (items, payloads) = submodules
            .iter()
            .map(|submodule| {
                (
                    components::PickerPromptItem::single(
                        submodule.path.display().to_string(),
                        components::TextTruncationProfile::Path,
                    ),
                    submodule.path.clone(),
                )
            })
            .unzip();
        (items, payloads, None)
    })
}

pub(super) fn nav_targets(
    this: &PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> Vec<std::path::PathBuf> {
    cached(this, repo_id, query).filtered_payloads()
}

/// Opens the submodule, or the confirmation for removing it. Shared by the click
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
        let confirm = PopoverKind::submodule(repo_id, SubmodulePopoverKind::RemoveConfirm { path });
        match position {
            Some(position) => this.open_popover_at(confirm, position, window, cx),
            None => this.open_popover_centered(confirm, window, cx),
        }
        return;
    }
    let Some(base) = repo_for(this, repo_id).map(|repo| repo.spec.workdir.clone()) else {
        return;
    };
    this.store.dispatch(Msg::OpenRepo(base.join(path)));
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
    match &repo.submodules {
        Loadable::Loading => return label(this, "Loading".into(), cx),
        Loadable::NotLoaded => return label(this, "Not loaded".into(), cx),
        Loadable::Error(e) => {
            let e = e.clone();
            return label(this, e.into(), cx);
        }
        Loadable::Ready(_) => {}
    }

    let Some(search) = this.submodule_picker_search_input.clone() else {
        return label(this, "Search input not initialized".into(), cx);
    };
    let query = search.read(cx).text().trim().to_string();
    let built = cached(this, repo_id, &query);
    let paths = Rc::clone(&built.payloads);

    components::context_menu(
        theme,
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
            .tooltip_host(this.tooltip_host.clone())
            .empty_text("No submodules")
            .max_height(scaled_px(SUBMODULE_PICKER_LIST_MAX_HEIGHT_PX))
            .selected_index(this.submodule_picker_selected_index)
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
