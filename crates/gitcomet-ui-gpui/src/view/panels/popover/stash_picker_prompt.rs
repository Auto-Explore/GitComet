use super::*;
use std::rc::Rc;

/// Height this picker caps its row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it: the list is
/// windowed once it outgrows a couple of viewports, and it is built for exactly
/// this one, so a navigation assuming another would scroll to the wrong place.
pub(super) const STASH_PICKER_LIST_MAX_HEIGHT_PX: f32 = 240.0;

/// What a stash row acts on: the git stash index, and the message the drop
/// confirmation quotes back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StashRow {
    pub(super) index: usize,
    pub(super) message: String,
}

/// Everything the rows below read.
fn rows_signature(this: &PopoverHost) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        let Some(repo) = this.active_repo() else {
            return;
        };
        repo.id.hash(hasher);
        repo.stashes_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.stashes).hash(hasher);
    })
}

/// The rows for `query`, built once per change to the stash list. The panel and
/// the arrow keys both read them from here, so the row Enter acts on is the row
/// that is highlighted.
pub(super) fn cached(
    this: &PopoverHost,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<StashRow>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Stash,
        rows_signature(this),
        query,
    );
    super::rows_cache::get_or_build(&this.stash_picker_rows_cache, key, |_now| {
        let Some(Loadable::Ready(stashes)) = this.active_repo().map(|repo| &repo.stashes) else {
            return (Vec::new(), Vec::new(), None);
        };
        let (items, payloads) = stashes
            .iter()
            .map(|stash| {
                let message = stash.message.to_string();
                (
                    components::PickerPromptItem::plain(message.clone()),
                    StashRow {
                        index: stash.index,
                        message,
                    },
                )
            })
            .unzip();
        (items, payloads, None)
    })
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<StashRow> {
    cached(this, query).filtered_payloads()
}

/// Runs the picker's purpose against one row. Shared by the click handler and by
/// Enter, so the two cannot drift apart.
pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    purpose: StashPickerPurpose,
    row: StashRow,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match purpose {
        StashPickerPurpose::Pop => {
            this.store.dispatch(Msg::PopStash {
                repo_id,
                index: row.index,
            });
            this.store.dispatch(Msg::LoadStashes { repo_id });
            this.close_popover(cx);
        }
        StashPickerPurpose::Apply => {
            this.store.dispatch(Msg::ApplyStash {
                repo_id,
                index: row.index,
            });
            this.store.dispatch(Msg::LoadStashes { repo_id });
            this.close_popover(cx);
        }
        StashPickerPurpose::Drop => {
            this.open_popover_centered(
                PopoverKind::StashDropConfirm {
                    repo_id,
                    index: row.index,
                    message: row.message,
                },
                window,
                cx,
            );
        }
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    purpose: StashPickerPurpose,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let title = match purpose {
        StashPickerPurpose::Pop => "Pop Stash",
        StashPickerPurpose::Apply => "Apply Stash",
        StashPickerPurpose::Drop => "Drop Stash",
    };

    let mut menu = div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(title))
        .child(div().border_t_1().border_color(theme.colors.border));

    if let Some(search) = this.stash_picker_search_input.clone() {
        match this
            .active_repo()
            .map(|r| matches!(&r.stashes, Loadable::Ready(_)))
        {
            Some(true) => {
                let query = search.read_with(cx, |i, _| i.text().trim().to_string());
                let built = cached(this, &query);
                let rows = Rc::clone(&built.payloads);

                menu = menu.child(
                    components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                        .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text("No stashes")
                        .max_height(scaled_px(STASH_PICKER_LIST_MAX_HEIGHT_PX))
                        .selected_index(this.stash_picker_prompt_selected_index)
                        .render(
                            theme,
                            ui_scale_percent,
                            cx,
                            move |this, ix, _e, window, cx| {
                                if let Some(row) = rows.get(ix).cloned() {
                                    activate(this, repo_id, purpose, row, window, cx);
                                }
                            },
                        ),
                );
            }
            _ => {
                let is_loading = this
                    .active_repo()
                    .map(|r| matches!(&r.stashes, Loadable::Loading))
                    .unwrap_or(false);
                let text = if is_loading {
                    "Loading…"
                } else {
                    "No stashes"
                };
                menu = menu.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    text,
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
        }
    }

    components::context_menu(theme, menu).w(scaled_px(420.0))
}
