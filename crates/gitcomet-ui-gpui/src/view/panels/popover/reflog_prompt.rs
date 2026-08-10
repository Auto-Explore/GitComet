use super::*;

/// A picker row for one reflog entry: a "you are here" marker on `HEAD@{0}`,
/// the selector, the short sha the entry points at, the relative time, and
/// the entry message. The sha is a searchable part (not just display), so
/// typing a partial sha still finds the row the way it did before the picker
/// prompt's own matcher took over query filtering here.
fn reflog_item(
    entry: &gitcomet_core::domain::ReflogEntry,
    is_current: bool,
) -> components::PickerPromptItem {
    let marker = if is_current { "▶ " } else { "  " };
    let sha = entry.new_id.as_ref();
    let short_sha = sha.get(0..8).unwrap_or(sha).to_owned();
    let time = entry
        .time
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            crate::view::date_time::format_relative_time(
                d.as_secs() as i64,
                std::time::SystemTime::now(),
            )
        })
        .unwrap_or_default();
    components::PickerPromptItem::from_parts([
        components::PickerPromptItemPart::new(marker)
            .flexible(false)
            .searchable(false),
        components::PickerPromptItemPart::new(entry.selector.to_string())
            .profile(components::TextTruncationProfile::End)
            .flexible(false),
        components::PickerPromptItemPart::separator("  "),
        components::PickerPromptItemPart::new(short_sha)
            .profile(components::TextTruncationProfile::End)
            .flexible(false)
            .tooltip(false),
        components::PickerPromptItemPart::separator("  "),
        components::PickerPromptItemPart::new(time)
            .profile(components::TextTruncationProfile::End)
            .flexible(false)
            .searchable(false),
        components::PickerPromptItemPart::separator("  "),
        components::PickerPromptItemPart::new(entry.message.to_string())
            .profile(components::TextTruncationProfile::End),
    ])
}

/// Test-only helper documenting the text a query matches against; production
/// filtering goes through the picker's own row-part matcher (see
/// `reflog_item`), not this function.
#[cfg(test)]
fn reflog_match_text(entry: &gitcomet_core::domain::ReflogEntry) -> String {
    let sha = entry.new_id.as_ref();
    let short = sha.get(0..8).unwrap_or(sha);
    format!("{} {} {}", entry.selector, short, entry.message)
}

/// Height this picker caps its row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it, following the
/// same windowing scheme as the other pickers (see `file_history`).
pub(super) const REFLOG_LIST_MAX_HEIGHT_PX: f32 = 340.0;

/// The popover's repository and its reflog entries, or `None` when the popover
/// is not the reflog prompt or the entries are not loaded yet.
fn reflog_repo(this: &PopoverHost) -> Option<(RepoId, &[gitcomet_core::domain::ReflogEntry])> {
    let Some(PopoverKind::ReflogPrompt { repo_id }) = &this.popover else {
        return None;
    };
    let repo = this.state.repos.iter().find(|r| r.id == *repo_id)?;
    let Loadable::Ready(entries) = &repo.reflog else {
        return None;
    };
    Some((*repo_id, entries.as_slice()))
}

/// Everything the rows below read.
fn rows_signature(this: &PopoverHost) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        let Some(PopoverKind::ReflogPrompt { repo_id }) = &this.popover else {
            return;
        };
        let Some(repo) = this.state.repos.iter().find(|r| r.id == *repo_id) else {
            return;
        };
        repo.id.hash(hasher);
        super::rows_cache::loadable_kind(&repo.reflog).hash(hasher);
        if let Loadable::Ready(entries) = &repo.reflog {
            entries.len().hash(hasher);
            for entry in entries {
                entry.index.hash(hasher);
                entry.new_id.hash(hasher);
            }
        }
    })
}

/// The rows for `query`, built once per change to the reflog. The panel and
/// the arrow keys both read them from here.
pub(super) fn cached(
    this: &PopoverHost,
    query: &str,
) -> std::rc::Rc<super::rows_cache::CachedRows<usize>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Reflog,
        rows_signature(this),
        query,
    );
    super::rows_cache::get_or_build(&this.reflog_rows_cache, key, |_now| {
        let Some((_repo_id, entries)) = reflog_repo(this) else {
            return (Vec::new(), Vec::new(), None);
        };
        let (items, payloads) = entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| (reflog_item(entry, ix == 0), entry.index))
            .unzip();
        (items, payloads, None)
    })
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<usize> {
    cached(this, query).filtered_payloads()
}

/// The entry the selection currently sits on, applying the same query filter
/// the picker list uses.
fn selected_entry<'a>(
    this: &PopoverHost,
    entries: &'a [gitcomet_core::domain::ReflogEntry],
    cx: &mut gpui::Context<PopoverHost>,
) -> Option<&'a gitcomet_core::domain::ReflogEntry> {
    let query = this
        .reflog_search_input
        .as_ref()
        .map(|input| input.read_with(cx, |i, _| i.text().trim().to_string()))
        .unwrap_or_default();
    let filtered = nav_targets(this, &query);
    let display_ix = this.reflog_selected_index?;
    let index = *filtered.get(display_ix)?;
    entries.iter().find(|e| e.index == index)
}

/// Footer with the selected entry summary and the reset actions. Each mode
/// routes through the existing `ResetPrompt` confirm dialog so the destructive
/// hard reset always requires explicit confirmation.
fn reflog_footer(
    this: &mut PopoverHost,
    repo_id: RepoId,
    entries: &[gitcomet_core::domain::ReflogEntry],
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let selected = selected_entry(this, entries, cx);
    let summary = if entries.is_empty() {
        "No entries yet.".to_string()
    } else {
        match selected {
            Some(entry) => {
                let sha = entry.new_id.as_ref();
                let short = sha.get(0..8).unwrap_or(sha);
                format!("{} · commit {short}", entry.selector)
            }
            None => "Select an entry with ↑/↓, then reset HEAD to it.".to_string(),
        }
    };
    let target = selected.map(|entry| entry.new_id.clone());

    let reset_button =
        |label: &'static str, mode: ResetMode, cx: &mut gpui::Context<PopoverHost>| {
            let target = target.clone();
            components::Button::new(format!("reflog_reset_{label}"), label)
                .style(components::ButtonStyle::Outlined)
                .disabled(target.is_none())
                .on_click(theme, cx, move |this, _e, window, cx| {
                    if let Some(target) = target.as_ref() {
                        this.open_popover_centered(
                            PopoverKind::ResetPrompt {
                                repo_id,
                                target: target.to_string(),
                                mode,
                            },
                            window,
                            cx,
                        );
                    }
                })
        };

    div()
        .border_t_1()
        .border_color(theme.colors.stroke.default)
        .px(scaled_px(8.0))
        .py(scaled_px(6.0))
        .flex()
        .flex_col()
        .gap(scaled_px(6.0))
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(summary),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(scaled_px(8.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.foreground.secondary)
                        .child("Reset HEAD to here"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(scaled_px(6.0))
                        .child(reset_button("Soft", ResetMode::Soft, cx))
                        .child(reset_button("Mixed", ResetMode::Mixed, cx))
                        .child(reset_button("Hard", ResetMode::Hard, cx)),
                ),
        )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let width = super::LARGE_PICKER_WIDTH;
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);

    let entry_count = repo.and_then(|r| match &r.reflog {
        Loadable::Ready(entries) => Some(entries.len()),
        _ => None,
    });
    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("Reflog"),
                )
                .when_some(entry_count, |col, count| {
                    col.child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .line_height(scaled_px(14.0))
                            .child(format!("{count} entries")),
                    )
                }),
        )
        .child(
            components::Button::new("reflog_close", "Close")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| this.close_popover(cx)),
        );

    let body: AnyElement = match repo.map(|r| &r.reflog) {
        None => components::context_menu_label(
            theme,
            ui_scale_percent,
            "No repository",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Loading) => components::context_menu_label(
            theme,
            ui_scale_percent,
            "Loading",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Error(e)) => components::context_menu_label(
            theme,
            ui_scale_percent,
            e.clone(),
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::NotLoaded) => components::context_menu_label(
            theme,
            ui_scale_percent,
            "Not loaded",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Ready(entries)) => {
            let owned_entries = entries.clone();

            let picker = match this.reflog_search_input.clone() {
                Some(search) => {
                    let query = search.read(cx).text().trim().to_string();
                    let built = cached(this, &query);
                    // The entry index (not the commit id) is what the reset
                    // footer keys its selection off, so the on-select callback
                    // resolves the commit id the same way it does — by looking
                    // the entry back up in the loaded page.
                    let entry_indices = std::rc::Rc::clone(&built.payloads);
                    let repo_entries = owned_entries.clone();
                    components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                        .prebuilt_items(
                            std::rc::Rc::clone(&built.items),
                            std::rc::Rc::clone(&built.layout),
                        )
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text("No reflog entries")
                        .max_height(scaled_px(REFLOG_LIST_MAX_HEIGHT_PX))
                        .selected_index(this.reflog_selected_index)
                        .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                            let Some(entry_index) = entry_indices.get(ix).copied() else {
                                return;
                            };
                            let Some(entry) = repo_entries.iter().find(|e| e.index == entry_index)
                            else {
                                return;
                            };
                            // View the commit the entry points at in the history
                            // and details panes.
                            this.store.dispatch(Msg::SelectCommit {
                                repo_id,
                                commit_id: entry.new_id.clone(),
                            });
                            this.close_popover(cx);
                        })
                        .into_any_element()
                }
                None => components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Search input not initialized",
                    Some(this.tooltip_host.clone()),
                    cx,
                )
                .into_any_element(),
            };

            let footer = reflog_footer(this, repo_id, &owned_entries, cx);
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(picker)
                .child(footer)
                .into_any_element()
        }
    };

    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.stroke.default))
            .child(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::ReflogEntry;

    fn entry(index: usize, new_id: &str, message: &str) -> ReflogEntry {
        ReflogEntry {
            index,
            new_id: gitcomet_core::domain::CommitId(new_id.into()),
            message: message.into(),
            time: None,
            selector: format!("HEAD@{{{index}}}").into(),
        }
    }

    fn entries() -> Vec<ReflogEntry> {
        vec![
            entry(0, "aaaaaaaaaaaa", "commit: initial"),
            entry(1, "bbbbbbbbbbbb", "checkout: moving to feature"),
            entry(2, "cccccccccccc", "reset: moving to HEAD@{1}"),
        ]
    }

    /// Runs the entries through the same row-building and layout-filtering
    /// path the panel and `nav_targets` use, returning the entry indices that
    /// survive `query` in display order.
    fn filtered_entry_indices(entries: &[ReflogEntry], query: &str) -> Vec<usize> {
        let items: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(ix, e)| reflog_item(e, ix == 0))
            .collect();
        // Production always trims the search input's text before handing the
        // query to the picker layout (see `search_inputs.rs`), so mirror that
        // here rather than exercising the untrimmed layout directly.
        let layout = components::picker_prompt_layout(&items, query.trim());
        layout
            .item_indices
            .iter()
            .map(|&ix| entries[ix].index)
            .collect()
    }

    #[test]
    fn empty_query_returns_every_entry_in_order() {
        let indices = filtered_entry_indices(&entries(), "");
        assert_eq!(indices, vec![0, 1, 2]);
        assert_eq!(filtered_entry_indices(&entries(), "   "), vec![0, 1, 2]);
    }

    #[test]
    fn query_filters_by_selector_message_and_short_sha_case_insensitively() {
        assert_eq!(filtered_entry_indices(&entries(), "head@{2}"), vec![2]);
        assert_eq!(filtered_entry_indices(&entries(), "CHECKOUT"), vec![1]);
        assert_eq!(filtered_entry_indices(&entries(), "bbbbbbbb"), vec![1]);
        // A prefix of the selector matches too. All three tie on match start
        // (position 0 in their selector), so the picker's shared tie-break —
        // shortest display text first, same as every other picker — decides
        // the order rather than input order.
        assert_eq!(filtered_entry_indices(&entries(), "head"), vec![0, 2, 1]);
    }

    #[test]
    fn query_with_no_match_returns_empty() {
        assert!(filtered_entry_indices(&entries(), "zzz").is_empty());
    }

    #[test]
    fn match_text_combines_selector_sha_and_message() {
        let text = reflog_match_text(&entry(4, "deadbeefcafe", "merge: branch x"));
        assert_eq!(text, "HEAD@{4} deadbeef merge: branch x");
    }
}
