use super::super::super::path_display;
use super::*;

pub(super) const OPEN_SECTION: &str = "Open Repositories";
pub(super) const RECENTLY_CLOSED_SECTION: &str = "Recently Closed";

/// One row of the repository picker: either a repository that is already open
/// (switch to it) or a session recent that is not (re-open it).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RepoPickerEntry {
    Open(RepoId),
    RecentlyClosed(std::path::PathBuf),
}

/// Row order inside each picker section. Recency means last activated for open
/// repositories and session MRU position for closed ones — the two sections are
/// ordered independently, so the sections themselves never interleave.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum RepoPickerSort {
    #[default]
    Newest,
    Oldest,
    Name,
    Path,
}

impl RepoPickerSort {
    pub(super) const ALL: [Self; 4] = [Self::Newest, Self::Oldest, Self::Name, Self::Path];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
            Self::Name => "Name (A–Z)",
            Self::Path => "Path (A–Z)",
        }
    }

    fn storage_key(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Name => "name",
            Self::Path => "path",
        }
    }

    fn from_storage_key(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|sort| sort.storage_key() == raw.trim())
    }
}

/// Reads the persisted picker sort, falling back to the default when the
/// session has no (or an unrecognized) value.
pub(super) fn sort_from_session(session: &session::UiSession) -> RepoPickerSort {
    session
        .repo_picker_sort
        .as_deref()
        .and_then(RepoPickerSort::from_storage_key)
        .unwrap_or_default()
}

pub(super) fn persist_sort(sort: RepoPickerSort) {
    let _ = session::persist_ui_settings(session::UiSettings {
        repo_picker_sort: Some(sort.storage_key().to_owned()),
        ..Default::default()
    });
}

/// Sort keys for one picker row. `recency` is a per-section rank where 0 is the
/// most recent, so both sections can share one comparator.
struct SortableRow {
    entry: RepoPickerEntry,
    item: components::PickerPromptItem,
    name_key: String,
    path_key: String,
    recency: usize,
}

fn sort_rows(rows: &mut [SortableRow], sort: RepoPickerSort) {
    match sort {
        RepoPickerSort::Newest => rows.sort_by_key(|row| row.recency),
        RepoPickerSort::Oldest => rows.sort_by_key(|row| std::cmp::Reverse(row.recency)),
        RepoPickerSort::Name => rows.sort_by(|a, b| {
            a.name_key
                .cmp(&b.name_key)
                .then_with(|| a.path_key.cmp(&b.path_key))
        }),
        RepoPickerSort::Path => rows.sort_by(|a, b| a.path_key.cmp(&b.path_key)),
    }
}

/// A repo row rendered as `name - parent path`, mirroring the recent-repository
/// picker so both switchers read the same way.
fn repo_picker_item(workdir: &std::path::Path) -> components::PickerPromptItem {
    match (
        workdir.file_name().and_then(|n| n.to_str()),
        workdir.parent(),
    ) {
        (Some(name), Some(parent)) => components::PickerPromptItem::from_parts([
            components::PickerPromptItemPart::new(name.to_owned())
                .profile(components::TextTruncationProfile::End)
                .flexible(false),
            components::PickerPromptItemPart::separator(" - "),
            components::PickerPromptItemPart::path(parent.display().to_string()),
        ]),
        (Some(name), None) => {
            components::PickerPromptItem::from_parts([components::PickerPromptItemPart::new(
                name.to_owned(),
            )
            .profile(components::TextTruncationProfile::End)
            .flexible(false)])
        }
        _ => components::PickerPromptItem::single(
            workdir.display().to_string(),
            components::TextTruncationProfile::Path,
        ),
    }
}

/// Open repositories first, then the session's recent repositories that are no
/// longer open — i.e. recently closed. Both sections live in one flat list so
/// the rendered rows, keyboard navigation and Enter target share an index
/// space.
pub(super) fn entries(this: &PopoverHost) -> Vec<(RepoPickerEntry, components::PickerPromptItem)> {
    let sort = this.repo_picker_sort;

    // Open repositories rank by last activation, newest first; repos that were
    // never activated (no timestamp) sort as the oldest.
    let mut open_by_recency = this.state.repos.iter().collect::<Vec<_>>();
    open_by_recency.sort_by(|a, b| match (a.last_active_at, b.last_active_at) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    let mut open_rows = open_by_recency
        .into_iter()
        .enumerate()
        .map(|(recency, repo)| {
            sortable_row(
                RepoPickerEntry::Open(repo.id),
                &repo.spec.workdir,
                "icons/folder.svg",
                OPEN_SECTION,
                recency,
            )
        })
        .collect::<Vec<_>>();

    // The session's recent list is already most-recent-first, so its index is
    // the recency rank.
    let mut recent_rows = this
        .cached_recent_repos
        .iter()
        .filter(|path| {
            // Recent paths and open workdirs are both canonicalized before they
            // are stored, so plain equality is enough to spot the still-open
            // ones.
            !this
                .state
                .repos
                .iter()
                .any(|repo| &repo.spec.workdir == *path)
        })
        .enumerate()
        .map(|(recency, path)| {
            sortable_row(
                RepoPickerEntry::RecentlyClosed(path.clone()),
                path,
                "icons/history.svg",
                RECENTLY_CLOSED_SECTION,
                recency,
            )
        })
        .collect::<Vec<_>>();

    sort_rows(&mut open_rows, sort);
    sort_rows(&mut recent_rows, sort);

    open_rows
        .into_iter()
        .chain(recent_rows)
        .map(|row| (row.entry, row.item))
        .collect()
}

fn sortable_row(
    entry: RepoPickerEntry,
    workdir: &std::path::Path,
    icon: &'static str,
    section: &'static str,
    recency: usize,
) -> SortableRow {
    SortableRow {
        entry,
        item: repo_picker_item(workdir).icon(icon).section(section),
        name_key: workdir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase(),
        path_key: workdir.display().to_string().to_lowercase(),
        recency,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, recency: usize) -> SortableRow {
        sortable_row(
            RepoPickerEntry::RecentlyClosed(std::path::PathBuf::from(path)),
            std::path::Path::new(path),
            "icons/history.svg",
            RECENTLY_CLOSED_SECTION,
            recency,
        )
    }

    fn names(rows: &[SortableRow]) -> Vec<&str> {
        rows.iter().map(|row| row.name_key.as_str()).collect()
    }

    fn fixture() -> Vec<SortableRow> {
        vec![
            row("/tmp/b-parent/Alpha", 0),
            row("/tmp/a-parent/zulu", 1),
            row("/tmp/c-parent/mike", 2),
        ]
    }

    #[test]
    fn newest_and_oldest_follow_recency_rank() {
        let mut rows = fixture();
        sort_rows(&mut rows, RepoPickerSort::Newest);
        assert_eq!(names(&rows), vec!["alpha", "zulu", "mike"]);

        sort_rows(&mut rows, RepoPickerSort::Oldest);
        assert_eq!(names(&rows), vec!["mike", "zulu", "alpha"]);
    }

    #[test]
    fn name_sort_is_case_insensitive_and_ignores_parent_directories() {
        let mut rows = fixture();
        sort_rows(&mut rows, RepoPickerSort::Name);
        assert_eq!(names(&rows), vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn path_sort_orders_by_full_path_not_repo_name() {
        let mut rows = fixture();
        sort_rows(&mut rows, RepoPickerSort::Path);
        assert_eq!(names(&rows), vec!["zulu", "alpha", "mike"]);
    }

    #[test]
    fn sort_round_trips_through_its_storage_key() {
        for sort in RepoPickerSort::ALL {
            assert_eq!(
                RepoPickerSort::from_storage_key(sort.storage_key()),
                Some(sort)
            );
        }
        assert_eq!(RepoPickerSort::from_storage_key("nonsense"), None);
    }
}

/// The picker rows in display order for the current query, paired with the
/// scroll-child index each row occupies (section headers take child slots too).
pub(super) fn filtered_layout(
    this: &PopoverHost,
    query: &str,
) -> (Vec<RepoPickerEntry>, components::PickerPromptLayout) {
    let entries = entries(this);
    let items = entries
        .iter()
        .map(|(_, item)| item.clone())
        .collect::<Vec<_>>();
    let layout = components::picker_prompt_layout(&items, query);
    let ordered = layout
        .item_indices
        .iter()
        .filter_map(|&ix| entries.get(ix).map(|(entry, _)| entry.clone()))
        .collect();
    (ordered, layout)
}

/// What the arrow keys walk in the picker: repository rows normally, sort
/// options while the sort menu covers the list.
#[derive(Clone, Debug)]
pub(super) enum RepoPickerNavTarget {
    Entry(RepoPickerEntry),
    Sort(RepoPickerSort),
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<RepoPickerNavTarget> {
    if this.repo_picker_sort_menu_open {
        return RepoPickerSort::ALL
            .into_iter()
            .map(RepoPickerNavTarget::Sort)
            .collect();
    }

    filtered_layout(this, query)
        .0
        .into_iter()
        .map(RepoPickerNavTarget::Entry)
        .collect()
}

pub(super) fn activate_nav_target(
    this: &mut PopoverHost,
    target: RepoPickerNavTarget,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match target {
        RepoPickerNavTarget::Entry(entry) => activate(this, entry, cx),
        RepoPickerNavTarget::Sort(sort) => apply_sort(this, sort, cx),
    }
}

/// Escape backs out of the sort menu first, and only closes the picker once the
/// repository list is showing again.
pub(super) fn dismiss(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    if this.repo_picker_sort_menu_open {
        toggle_sort_menu(this, cx);
        return;
    }
    this.close_popover(cx);
}

pub(super) fn toggle_sort_menu(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    this.repo_picker_sort_menu_open = !this.repo_picker_sort_menu_open;
    // The selection index is shared between the repo list and the sort menu, so
    // reset it whenever the two swap places.
    this.repo_picker_selected_index = None;
    cx.notify();
}

pub(super) fn apply_sort(
    this: &mut PopoverHost,
    sort: RepoPickerSort,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.repo_picker_sort = sort;
    this.repo_picker_sort_menu_open = false;
    this.repo_picker_selected_index = None;
    persist_sort(sort);
    cx.notify();
}

/// The `Sort ▾` toggle that sits at the right edge of the query row.
fn sort_toggle(this: &PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> impl IntoElement {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let scaled_px = |value: f32| ui_scale.px(value);
    let menu_open = this.repo_picker_sort_menu_open;
    let hover_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.07 } else { 0.05 });
    let active_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.11 } else { 0.08 });

    div()
        .id("repo_picker_sort_toggle")
        .debug_selector(|| "repo_picker_sort_toggle".to_string())
        .flex()
        .items_center()
        .gap(scaled_px(4.0))
        .h(scaled_px(24.0))
        .px(scaled_px(8.0))
        .rounded(px(theme.radii.control))
        .cursor(CursorStyle::PointingHand)
        .text_xs()
        .text_color(if menu_open {
            theme.colors.text
        } else {
            theme.colors.text_muted
        })
        .when(menu_open, |toggle| toggle.bg(active_overlay))
        .hover(move |s| s.bg(hover_overlay))
        .active(move |s| s.bg(active_overlay))
        .child("Sort")
        .child(crate::view::icons::svg_icon(
            "icons/chevron_down.svg",
            theme.colors.text_muted,
            scaled_px(12.0),
        ))
        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            toggle_sort_menu(this, cx);
        }))
}

/// The sort options, rendered in place of the repository rows while the menu is
/// open. Picking one collapses the menu back to the list.
fn sort_menu(this: &PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> impl IntoElement {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale(cx).percent();
    let current = this.repo_picker_sort;
    let selected_index = this.repo_picker_selected_index;

    let mut menu = div()
        .id("repo_picker_sort_menu")
        .flex()
        .flex_col()
        .w_full()
        .p(super::popover_scaled_px_from_percent(4.0, ui_scale_percent));
    for (ix, sort) in RepoPickerSort::ALL.into_iter().enumerate() {
        menu = menu.child(
            components::ContextMenuEntry::new(
                ("repo_picker_sort_option", ix),
                components::ContextMenuText::new(sort.label()),
            )
            .icon(if sort == current {
                components::ContextMenuIconSlot::Icon("icons/check.svg".into())
            } else {
                components::ContextMenuIconSlot::Reserved
            })
            .selected(selected_index == Some(ix))
            .render(theme, ui_scale_percent, cx)
            .debug_selector(move || format!("repo_picker_sort_option_{ix}"))
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                apply_sort(this, sort, cx);
            })),
        );
    }
    menu
}

pub(super) fn activate(
    this: &mut PopoverHost,
    entry: RepoPickerEntry,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match entry {
        RepoPickerEntry::Open(repo_id) => {
            this.store.dispatch(Msg::SetActiveRepo { repo_id });
            this.close_popover(cx);
        }
        RepoPickerEntry::RecentlyClosed(path) => {
            this.close_popover(cx);
            this.store.dispatch(Msg::OpenRepo(path));
        }
    }
}

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::PICKER_WIDTH;
    let entries = entries(this);

    if let Some(search) = this.repo_picker_search_input.clone() {
        // Match the Create Branch search field: a chromeless input, here with a
        // leading magnifier to read as a search box, sitting in the popover card.
        search.update(cx, |input, cx| {
            input.set_chromeless(true, cx);
            input.set_leading_icon(Some("icons/zoom.svg"), cx);
        });

        let items = entries
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        let active_index = this.state.active_repo.and_then(|active| {
            entries
                .iter()
                .position(|(entry, _)| matches!(entry, RepoPickerEntry::Open(id) if *id == active))
        });
        let select_entries = entries
            .iter()
            .map(|(entry, _)| entry.clone())
            .collect::<Vec<_>>();

        let mut prompt = components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .items(items)
            .tooltip_host(this.tooltip_host.clone())
            .empty_text("No repositories")
            .max_height(scaled_px(360.0))
            .selected_index(this.repo_picker_selected_index)
            .marked_index(active_index)
            .selected_hint("Enter")
            .accent_selection()
            .padded_query_row()
            .query_row_trailing(sort_toggle(this, cx));
        if this.repo_picker_sort_menu_open {
            prompt = prompt.list_override(sort_menu(this, cx));
        }

        components::context_menu(
            theme,
            prompt.render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                if let Some(entry) = select_entries.get(ix).cloned() {
                    activate(this, entry, cx);
                }
            }),
        )
        // Fixed width: PickerPrompt rows size with `w_full`, which does not
        // stretch under fit-content parents.
        .w(width.preferred_px(ui_scale))
    } else {
        let mut menu = div()
            .flex()
            .flex_col()
            .min_w(width.min_px(ui_scale))
            .max_w(width.max_px(ui_scale));
        let mut section: Option<&str> = None;
        for (ix, (entry, _)) in entries.iter().enumerate() {
            let entry_section = match entry {
                RepoPickerEntry::Open(_) => OPEN_SECTION,
                RepoPickerEntry::RecentlyClosed(_) => RECENTLY_CLOSED_SECTION,
            };
            if section != Some(entry_section) {
                section = Some(entry_section);
                menu = menu.child(components::context_menu_header(
                    theme,
                    ui_scale_percent,
                    entry_section.to_owned(),
                    None,
                    cx,
                ));
            }
            let label = match entry {
                RepoPickerEntry::Open(repo_id) => this
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == *repo_id)
                    .map(|repo| path_display::path_display_shared(&repo.spec.workdir)),
                RepoPickerEntry::RecentlyClosed(path) => {
                    Some(path_display::path_display_shared(path))
                }
            };
            let Some(label) = label else {
                continue;
            };
            let entry = entry.clone();
            menu = menu.child(
                components::ContextMenuEntry::new(
                    ("repo_item", ix),
                    components::ContextMenuText::path_single_line(label),
                )
                .tooltip_host(this.tooltip_host.clone())
                .render(theme, ui_scale_percent, cx)
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    activate(this, entry.clone(), cx);
                })),
            );
        }
        components::context_menu(theme, menu)
    }
}
