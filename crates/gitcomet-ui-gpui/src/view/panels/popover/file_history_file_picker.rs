use super::*;
use std::path::PathBuf;
use std::rc::Rc;

/// Same cap as the stash picker: the list is windowed for exactly this height.
pub(super) const LIST_MAX_HEIGHT_PX: f32 = 240.0;

/// The file the main diff shows, if any (same rule as Ctrl/Cmd+H).
fn open_file_path(repo: &RepoState) -> Option<PathBuf> {
    match repo.diff_state.diff_target.as_ref()? {
        DiffTarget::WorkingTree { path, .. } | DiffTarget::Commit { path, .. } => {
            Some(path.clone())
        }
        DiffTarget::CommitRange { path, .. } => path.clone(),
    }
}

fn untracked_paths(repo: &RepoState) -> FxHashSet<&PathBuf> {
    let Loadable::Ready(status) = &repo.status else {
        return FxHashSet::default();
    };
    status
        .unstaged
        .iter()
        .chain(status.staged.iter())
        .filter(|s| s.kind == FileStatusKind::Untracked)
        .map(|s| &s.path)
        .collect()
}

fn rows_signature(this: &PopoverHost) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        let Some(repo) = this.active_repo() else {
            return;
        };
        repo.id.hash(hasher);
        repo.file_browser.file_browser_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.file_browser.entries).hash(hasher);
        repo.status_rev.hash(hasher);
        repo.diff_state.diff_target_rev.hash(hasher);
    })
}

/// Tracked files, the open file first. Built once per change to its inputs.
pub(super) fn cached(
    this: &PopoverHost,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<PathBuf>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::FileHistoryFilePicker,
        rows_signature(this),
        query,
    );
    super::rows_cache::get_or_build(&this.file_history_file_picker_rows_cache, key, |_now| {
        let Some(repo) = this.active_repo() else {
            return (Vec::new(), Vec::new(), None);
        };
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            return (Vec::new(), Vec::new(), None);
        };
        let untracked = untracked_paths(repo);
        let open = open_file_path(repo).filter(|p| !untracked.contains(p));
        let paths: Vec<PathBuf> = open
            .iter()
            .cloned()
            .chain(
                entries
                    .iter()
                    .filter(|e| e.kind == gitcomet_core::domain::FileEntryKind::File)
                    .map(|e| e.path.as_ref())
                    .filter(|p| !untracked.contains(p) && Some(*p) != open.as_ref())
                    .cloned(),
            )
            .collect();
        let items = paths
            .iter()
            .map(|p| components::PickerPromptItem::plain(p.display().to_string()))
            .collect();
        (items, paths, None)
    })
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<PathBuf> {
    cached(this, query).filtered_payloads()
}

pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: PathBuf,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.close_popover(cx);
    let _ = this.root_view.request(
        RootRequest::OpenFileHistory { repo_id, path },
        Some(window),
        cx,
    );
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    let mut menu = div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(theme, "File History"))
        .child(super::popover_rule(theme));

    if let Some(search) = this.file_history_file_picker_search_input.clone() {
        // Always draw the prompt: the focused input must stay in the tree so
        // Escape and typing work while the list loads.
        let loading = this.active_repo().is_some_and(|r| {
            matches!(
                r.file_browser.entries,
                Loadable::Loading | Loadable::NotLoaded
            )
        });
        let query = search.read_with(cx, |i, _| i.text().trim().to_string());
        let built = cached(this, &query);
        let rows = Rc::clone(&built.payloads);
        menu = menu.child(
            components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
                .tooltip_host(this.tooltip_host.clone())
                .empty_text(if loading { "Loading…" } else { "No files" })
                .max_height(scaled_px(LIST_MAX_HEIGHT_PX))
                .selected_index(this.file_history_file_picker_selected_index)
                .render(
                    theme,
                    ui_scale_percent,
                    cx,
                    move |this, ix, _e, window, cx| {
                        if let Some(path) = rows.get(ix).cloned() {
                            activate(this, repo_id, path, window, cx);
                        }
                    },
                ),
        );
    }

    components::context_menu(theme, menu).w(scaled_px(420.0))
}
