use super::super::super::path_display;
use super::*;
use std::collections::BTreeSet;

pub(super) const PINNED_SECTION: &str = "Pinned";
pub(super) const OPEN_SECTION: &str = "Open Repositories";
pub(super) const RECENTLY_CLOSED_SECTION: &str = "Recently Closed";

/// Sections in render order, paired with the key their collapse state persists
/// under. The keys are deliberately not the labels, so the headings can be
/// reworded without stranding everyone's folded sections.
const SECTIONS: [(&str, &str); 3] = [
    (PINNED_SECTION, "pinned"),
    (OPEN_SECTION, "open"),
    (RECENTLY_CLOSED_SECTION, "recently_closed"),
];

fn section_storage_key(label: &str) -> Option<&'static str> {
    SECTIONS
        .into_iter()
        .find_map(|(section, key)| (section == label).then_some(key))
}

/// One row of the repository picker: either a repository that is already open
/// (switch to it) or one that is not (open it). A pinned repository is whichever
/// of the two it happens to be — the pin only decides which section it sits in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RepoPickerEntry {
    Open(RepoId),
    Closed(std::path::PathBuf),
}

impl RepoPickerEntry {
    fn workdir(&self, this: &PopoverHost) -> Option<std::path::PathBuf> {
        match self {
            Self::Open(repo_id) => this.workdir_for_repo(*repo_id),
            Self::Closed(path) => Some(path.clone()),
        }
    }
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

/// Section labels the picker should fold away right now. A query overrides
/// collapse entirely: typing searches every section, the way the branch
/// sidebar's filter force-expands its own.
fn collapsed_sections(this: &PopoverHost, query: &str) -> BTreeSet<gpui::SharedString> {
    if !query.is_empty() {
        return BTreeSet::new();
    }
    SECTIONS
        .into_iter()
        .filter(|(_, key)| this.cached_collapsed_picker_sections.contains(*key))
        .map(|(section, _)| gpui::SharedString::from(section))
        .collect()
}

/// Folds a section away, or unfolds it. Keyed by the label the header carries;
/// an unknown label is a no-op.
pub(super) fn toggle_section(
    this: &mut PopoverHost,
    label: &gpui::SharedString,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(key) = section_storage_key(label.as_ref()) else {
        return;
    };
    if !this.cached_collapsed_picker_sections.insert(key.to_owned()) {
        this.cached_collapsed_picker_sections.remove(key);
    }
    // Rows above the selection come and go, so a kept index would highlight a
    // different repository than the one it was on.
    this.repo_picker_selected_index = None;
    let _ = session::persist_ui_settings(session::UiSettings {
        repo_picker_collapsed_sections: Some(this.cached_collapsed_picker_sections.clone()),
        ..Default::default()
    });
    cx.notify();
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

/// Pinned repositories first, then the ones that are open, then the session's
/// recent repositories that are neither — i.e. recently closed. A repository
/// appears exactly once: pinning lifts it out of its home section rather than
/// duplicating it, so every row is a distinct arrow-key target.
///
/// The three sections live in one flat list so the rendered rows, keyboard
/// navigation and Enter target share an index space.
pub(super) fn entries(this: &PopoverHost) -> Vec<(RepoPickerEntry, components::PickerPromptItem)> {
    let sort = this.repo_picker_sort;
    // Pins, recents and open workdirs are all canonicalized before they are
    // stored, so plain equality is enough to match them up.
    let is_pinned = |path: &std::path::Path| this.cached_pinned_repos.iter().any(|p| p == path);
    let open_repo_for = |path: &std::path::Path| {
        this.state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == path)
    };

    // A pin outlives both the recents cap and the repository being closed, so
    // this section is built from the pin list itself and nothing else.
    //
    // Pins are stored oldest-first, but `recency` counts the other way in every
    // section, so the index is flipped here — otherwise "Newest" would list the
    // oldest pin at the top while the two sections below it read newest-first.
    let last_pin = this.cached_pinned_repos.len().saturating_sub(1);
    let mut pinned_rows = this
        .cached_pinned_repos
        .iter()
        .enumerate()
        .map(|(pin_ix, path)| {
            let entry = match open_repo_for(path) {
                Some(repo) => RepoPickerEntry::Open(repo.id),
                None => RepoPickerEntry::Closed(path.clone()),
            };
            // Unpinning is a context-menu action, so a pinned row has no `x`:
            // one trailing button cannot mean both "unpin" and "forget".
            sortable_row(entry, path, PINNED_SECTION, last_pin - pin_ix, false)
        })
        .collect::<Vec<_>>();

    // Open repositories rank by last activation, newest first; repos that were
    // never activated (no timestamp) sort as the oldest.
    let mut open_by_recency = this
        .state
        .repos
        .iter()
        .filter(|repo| !is_pinned(&repo.spec.workdir))
        .collect::<Vec<_>>();
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
                OPEN_SECTION,
                recency,
                false,
            )
        })
        .collect::<Vec<_>>();

    // The session's recent list is already most-recent-first, so its index is
    // the recency rank.
    let mut recent_rows = this
        .cached_recent_repos
        .iter()
        .filter(|path| open_repo_for(path).is_none() && !is_pinned(path))
        .enumerate()
        .map(|(recency, path)| {
            sortable_row(
                RepoPickerEntry::Closed(path.clone()),
                path,
                RECENTLY_CLOSED_SECTION,
                recency,
                // Only recents can be forgotten; open repositories leave the
                // list by being closed.
                true,
            )
        })
        .collect::<Vec<_>>();

    sort_rows(&mut pinned_rows, sort);
    sort_rows(&mut open_rows, sort);
    sort_rows(&mut recent_rows, sort);

    pinned_rows
        .into_iter()
        .chain(open_rows)
        .chain(recent_rows)
        .map(|row| (row.entry, row.item))
        .collect()
}

fn sortable_row(
    entry: RepoPickerEntry,
    workdir: &std::path::Path,
    section: &'static str,
    recency: usize,
    removable: bool,
) -> SortableRow {
    SortableRow {
        entry,
        item: {
            let repository_name = path_display::repo_path_name(workdir);
            let item = repo_picker_item(workdir)
                .repository_initials(repository_name.as_ref())
                .section(section);
            if removable { item.removable() } else { item }
        },
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
            RepoPickerEntry::Closed(std::path::PathBuf::from(path)),
            std::path::Path::new(path),
            RECENTLY_CLOSED_SECTION,
            recency,
            true,
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

    #[test]
    fn sort_toggle_includes_the_selected_sort() {
        assert_eq!(sort_toggle_label(RepoPickerSort::Newest), "Sort: Newest");
        assert_eq!(sort_toggle_label(RepoPickerSort::Oldest), "Sort: Oldest");
        assert_eq!(
            sort_toggle_label(RepoPickerSort::Name),
            format!("Sort: {}", RepoPickerSort::Name.label())
        );
        assert_eq!(
            sort_toggle_label(RepoPickerSort::Path),
            format!("Sort: {}", RepoPickerSort::Path.label())
        );
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
    // Must match what `panel` hands the picker, or Enter activates a different
    // row than the highlighted one.
    let layout = components::picker_prompt_layout_with_collapsed(
        &items,
        query,
        &collapsed_sections(this, query),
    );
    let ordered = layout
        .item_indices
        .iter()
        .filter_map(|&ix| entries.get(ix).map(|(entry, _)| entry.clone()))
        .collect();
    (ordered, layout)
}

/// What the arrow keys walk in the picker: repository rows normally, sort
/// options while the sort menu covers the list, and the row actions while a
/// repository's context menu floats over it.
#[derive(Clone, Debug)]
pub(super) enum RepoPickerNavTarget {
    Entry(RepoPickerEntry),
    Sort(RepoPickerSort),
    RowAction(RepoPickerRowAction),
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<RepoPickerNavTarget> {
    if let Some(menu) = &this.repo_picker_row_menu {
        return row_menu_items(this, &menu.entry)
            .into_iter()
            .filter_map(|item| match item {
                RepoPickerRowMenuItem::Entry {
                    action, disabled, ..
                } => (!disabled).then_some(RepoPickerNavTarget::RowAction(action)),
                RepoPickerRowMenuItem::Separator => None,
            })
            .collect();
    }
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
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match target {
        RepoPickerNavTarget::Entry(entry) => activate(this, entry, cx),
        RepoPickerNavTarget::Sort(sort) => apply_sort(this, sort, cx),
        RepoPickerNavTarget::RowAction(action) => activate_row_action(this, action, window, cx),
    }
}

/// Escape backs out of whatever is layered over the repository list — the row
/// menu, then the sort menu — and only closes the picker once the list itself is
/// showing again.
pub(super) fn dismiss(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    if this.repo_picker_row_menu.is_some() {
        close_row_menu(this, cx);
        return;
    }
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
    let hover_overlay = theme.hover_overlay();
    let active_overlay = theme.active_overlay();

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
        .child(sort_toggle_label(this.repo_picker_sort))
        .child(crate::view::icons::svg_icon(
            "icons/chevron_down.svg",
            theme.colors.text_muted,
            scaled_px(12.0),
        ))
        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            toggle_sort_menu(this, cx);
        }))
}

fn sort_toggle_label(sort: RepoPickerSort) -> String {
    format!("Sort: {}", sort.label())
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
        RepoPickerEntry::Closed(path) => {
            this.close_popover(cx);
            this.store.dispatch(Msg::OpenRepo(path));
        }
    }
}

/// Drops a recently-closed entry from the session's recent list. Open and pinned
/// repositories have no `x`, so anything else is a no-op.
pub(super) fn forget(
    this: &mut PopoverHost,
    entry: &RepoPickerEntry,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let RepoPickerEntry::Closed(path) = entry else {
        return;
    };
    let _ = session::remove_recent_repo(path);
    this.cached_recent_repos.retain(|recent| recent != path);
    // The rows below the removed one shift up, so a stale selection would
    // point at a different repository than the one it highlighted.
    this.repo_picker_selected_index = None;
    cx.notify();
}

/// The repository row whose context menu is open, and where to draw it.
#[derive(Clone, Debug)]
pub(super) struct RepoPickerRowMenu {
    pub(super) entry: RepoPickerEntry,
    pub(super) position: gpui::Point<gpui::Pixels>,
    /// Display index of the row the menu belongs to, so it stays highlighted
    /// while the menu floats somewhere else on screen.
    pub(super) display_index: usize,
}

/// What a repository row's context menu can do. Pin state and open state decide
/// which of these a given row offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RepoPickerRowAction {
    Pin,
    Unpin,
    /// Switch to an already open repository.
    Activate,
    /// Open a repository that is not open yet.
    Open,
    RevealLocation,
    OpenInCodeEditor,
    CopyPath,
    Close,
    ForgetRecent,
}

pub(super) enum RepoPickerRowMenuItem {
    Separator,
    Entry {
        label: &'static str,
        icon: &'static str,
        action: RepoPickerRowAction,
        disabled: bool,
    },
}

fn menu_entry(
    label: &'static str,
    icon: &'static str,
    action: RepoPickerRowAction,
) -> RepoPickerRowMenuItem {
    RepoPickerRowMenuItem::Entry {
        label,
        icon,
        action,
        disabled: false,
    }
}

/// The actions a row offers, in menu order. Pure over the host's state so the
/// menu's shape can be asserted without rendering it.
pub(super) fn row_menu_items(
    this: &PopoverHost,
    entry: &RepoPickerEntry,
) -> Vec<RepoPickerRowMenuItem> {
    let workdir = entry.workdir(this);
    let pinned = workdir
        .as_ref()
        .is_some_and(|path| this.cached_pinned_repos.iter().any(|pin| pin == path));

    let mut items = vec![if pinned {
        menu_entry(
            "Unpin repository",
            "icons/pin.svg",
            RepoPickerRowAction::Unpin,
        )
    } else {
        menu_entry("Pin repository", "icons/pin.svg", RepoPickerRowAction::Pin)
    }];

    items.push(RepoPickerRowMenuItem::Separator);
    match entry {
        RepoPickerEntry::Open(repo_id) => items.push(RepoPickerRowMenuItem::Entry {
            label: "Activate",
            icon: "icons/check.svg",
            action: RepoPickerRowAction::Activate,
            disabled: this.state.active_repo == Some(*repo_id),
        }),
        // Same wording and icon as the "+" menu's Open repository, which is the
        // other way into this action.
        RepoPickerEntry::Closed(_) => items.push(menu_entry(
            "Open repository",
            "icons/disk.svg",
            RepoPickerRowAction::Open,
        )),
    }

    if workdir.is_some() {
        items.push(RepoPickerRowMenuItem::Separator);
        items.push(menu_entry(
            "Open repository location",
            "icons/folder.svg",
            RepoPickerRowAction::RevealLocation,
        ));
        if crate::external_editor::configured_setting().is_some() {
            items.push(menu_entry(
                "Open in code editor",
                "icons/open_external.svg",
                RepoPickerRowAction::OpenInCodeEditor,
            ));
        }
        items.push(menu_entry(
            "Copy path",
            "icons/copy.svg",
            RepoPickerRowAction::CopyPath,
        ));
    }

    // A pin is what keeps a closed repository listed, so forgetting it while it
    // is pinned would leave the row exactly where it was — the pinned closed row
    // is the one case with nothing to put here at all.
    let destructive = match entry {
        RepoPickerEntry::Open(_) => Some(menu_entry(
            "Close repository",
            "icons/repo_tab_close.svg",
            RepoPickerRowAction::Close,
        )),
        RepoPickerEntry::Closed(_) if pinned => None,
        RepoPickerEntry::Closed(_) => Some(menu_entry(
            "Remove from recently closed",
            "icons/repo_tab_close.svg",
            RepoPickerRowAction::ForgetRecent,
        )),
    };
    if let Some(destructive) = destructive {
        items.push(RepoPickerRowMenuItem::Separator);
        items.push(destructive);
    }

    items
}

pub(super) fn open_row_menu(
    this: &mut PopoverHost,
    entry: RepoPickerEntry,
    display_index: usize,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.repo_picker_row_menu = Some(RepoPickerRowMenu {
        entry,
        position,
        display_index,
    });
    // The selection index now addresses the menu's own actions, so it restarts
    // from nothing; the invoking row keeps its highlight through
    // `RepoPickerRowMenu::display_index` instead.
    this.repo_picker_selected_index = None;
    this.repo_picker_sort_menu_open = false;
    cx.notify();
}

/// Dismisses the row menu without running anything — Escape, or a press outside
/// it. The selection lands back on the row the menu belonged to, so arrowing on
/// carries from there rather than restarting at the top of the list.
pub(super) fn close_row_menu(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    let display_index = this
        .repo_picker_row_menu
        .take()
        .map(|menu| menu.display_index);
    this.repo_picker_selected_index = display_index;
    cx.notify();
}

/// Closes the row menu on the way into one of its actions. Unlike a dismissal
/// this drops the selection: pinning, closing and forgetting all reorder the
/// rows, so the invoking row's index no longer names the same repository.
fn close_row_menu_for_action(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    this.repo_picker_row_menu = None;
    this.repo_picker_selected_index = None;
    cx.notify();
}

pub(super) fn activate_row_action(
    this: &mut PopoverHost,
    action: RepoPickerRowAction,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(menu) = this.repo_picker_row_menu.clone() else {
        return;
    };
    let entry = menu.entry;
    let workdir = entry.workdir(this);
    close_row_menu_for_action(this, cx);

    // Actions that leave the picker for somewhere else run through the shared
    // context-menu executor, which already handles the toasts, the editor
    // launch and closing the popover behind itself.
    let delegate = |this: &mut PopoverHost, action, window: &mut Window, cx: &mut _| {
        this.context_menu_activate_action(action, window, cx);
    };

    match action {
        RepoPickerRowAction::Pin => {
            let Some(workdir) = workdir else { return };
            let _ = session::persist_pinned_repo(&workdir);
            if !this.cached_pinned_repos.contains(&workdir) {
                this.cached_pinned_repos.push(workdir);
            }
            cx.notify();
        }
        RepoPickerRowAction::Unpin => {
            let Some(workdir) = workdir else { return };
            let _ = session::remove_pinned_repo(&workdir);
            this.cached_pinned_repos.retain(|pin| pin != &workdir);
            cx.notify();
        }
        RepoPickerRowAction::Activate | RepoPickerRowAction::Open => activate(this, entry, cx),
        RepoPickerRowAction::RevealLocation => {
            let Some(path) = workdir else { return };
            delegate(
                this,
                ContextMenuAction::OpenRepositoryLocation { path },
                window,
                cx,
            );
        }
        RepoPickerRowAction::OpenInCodeEditor => {
            let Some(path) = workdir else { return };
            delegate(
                this,
                ContextMenuAction::OpenInCodeEditor {
                    repo_id: None,
                    path,
                },
                window,
                cx,
            );
        }
        RepoPickerRowAction::CopyPath => {
            let Some(workdir) = workdir else { return };
            delegate(
                this,
                ContextMenuAction::CopyText {
                    text: workdir.display().to_string(),
                },
                window,
                cx,
            );
        }
        RepoPickerRowAction::Close => {
            let RepoPickerEntry::Open(repo_id) = entry else {
                return;
            };
            // Closing keeps the picker up so several repositories can go in a
            // row. Only opening a repository records it as recent, so closing
            // one has to do it here — and `cached_recent_repos` is an open-time
            // snapshot, so the same move is made against the session file and
            // the cache. Skipping either leaves the Recently Closed list the
            // user is looking at disagreeing with the one they get next time.
            if let Some(workdir) = workdir {
                let _ = session::persist_recent_repo(&workdir);
                this.cached_recent_repos.retain(|recent| recent != &workdir);
                this.cached_recent_repos.insert(0, workdir);
            }
            this.repo_picker_selected_index = None;
            this.store.dispatch(Msg::CloseRepo { repo_id });
            cx.notify();
        }
        RepoPickerRowAction::ForgetRecent => forget(this, &entry, cx),
    }
}

/// The floating row menu, drawn by [`PopoverHost::render`] above the picker.
/// Returns `None` unless a row menu is open.
pub(super) fn row_menu_layer(
    this: &PopoverHost,
    cx: &mut gpui::Context<PopoverHost>,
) -> Option<gpui::AnyElement> {
    let menu = this.repo_picker_row_menu.clone()?;
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let selected_index = this.repo_picker_selected_index;

    let mut list = div()
        .flex()
        .flex_col()
        .w(super::REPO_TAB_MENU_WIDTH.preferred_px(ui_scale))
        .p(super::popover_scaled_px_from_percent(4.0, ui_scale_percent));
    // Only enabled entries are keyboard targets, so the menu's own selection
    // index counts those alone.
    let mut nav_ix = 0usize;
    for (ix, item) in row_menu_items(this, &menu.entry).into_iter().enumerate() {
        match item {
            RepoPickerRowMenuItem::Separator => {
                list = list.child(components::context_menu_separator(theme, ui_scale_percent));
            }
            RepoPickerRowMenuItem::Entry {
                label,
                icon,
                action,
                disabled,
            } => {
                let selected = !disabled && selected_index == Some(nav_ix);
                if !disabled {
                    nav_ix += 1;
                }
                list = list.child(
                    components::ContextMenuEntry::new(
                        ("repo_picker_row_action", ix),
                        components::ContextMenuText::new(label),
                    )
                    .icon(components::ContextMenuIconSlot::Icon(icon.into()))
                    .disabled(disabled)
                    .selected(selected)
                    .render(theme, ui_scale_percent, cx)
                    .debug_selector(move || format!("repo_picker_row_action_{ix}"))
                    .on_click(cx.listener(
                        move |this, _e: &ClickEvent, window, cx| {
                            if disabled {
                                return;
                            }
                            activate_row_action(this, action, window, cx);
                        },
                    )),
                );
            }
        }
    }

    let dismiss_menu = cx.listener(|this, _e: &MouseDownEvent, _w, cx| {
        cx.stop_propagation();
        close_row_menu(this, cx);
    });

    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Catches the click that dismisses the menu before the picker's own
            // scrim can read it as "close the popover".
            .child(
                div()
                    .id("repo_picker_row_menu_scrim")
                    .debug_selector(|| "repo_picker_row_menu_scrim".to_string())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(gpui::rgba(0x00000000))
                    .occlude()
                    .on_any_mouse_down(dismiss_menu),
            )
            .child(
                anchored().position(menu.position).child(
                    // This menu is its own floating layer rather than a panel
                    // inside the popover container, so it has to bring the
                    // elevated surface with it — `components::context_menu` is
                    // layout only.
                    components::popover_surface(theme)
                        .id("repo_picker_row_menu")
                        .debug_selector(|| "repo_picker_row_menu".to_string())
                        .occlude()
                        .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
                        // `occlude` silences the root view's mouse tracking, so
                        // feed the tooltip host from in here or a truncated-text
                        // tooltip from the picker underneath stays anchored
                        // where the pointer last was outside the menu.
                        .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                            let _ = this
                                .tooltip_host
                                .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
                        }))
                        .child(components::context_menu(theme, list)),
                ),
            )
            .into_any_element(),
    )
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
        // One list behind all three row callbacks: each closure lives as long as
        // the elements it is attached to, so cloning the vector per callback
        // would hold three copies of every path for the frame.
        let row_entries: std::rc::Rc<[RepoPickerEntry]> = entries
            .iter()
            .map(|(entry, _)| entry.clone())
            .collect::<Vec<_>>()
            .into();
        let select_entries = std::rc::Rc::clone(&row_entries);
        let remove_entries = std::rc::Rc::clone(&row_entries);

        let query = search.read(cx).text().trim().to_string();
        let row_menu = this.repo_picker_row_menu.as_ref();
        let mut prompt = components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .items(items)
            .tooltip_host(this.tooltip_host.clone())
            .empty_text("No repositories")
            .max_height(scaled_px(360.0))
            // While a row menu is open the arrow keys walk its actions, so the
            // list's highlight marks the invoking row instead — without the
            // Enter hint, which now belongs to the menu.
            .selected_index(
                row_menu
                    .map(|menu| menu.display_index)
                    .or(this.repo_picker_selected_index),
            )
            .marked_index(active_index)
            .accent_selection()
            .padded_query_row()
            .remove_tooltip("Remove from recently closed")
            .collapsed_sections(collapsed_sections(this, &query))
            .on_context_menu(cx.listener(
                move |this, event: &components::PickerPromptContextMenuEvent, _window, cx| {
                    let Some(entry) = row_entries.get(event.original_index).cloned() else {
                        return;
                    };
                    open_row_menu(this, entry, event.display_index, event.position, cx);
                },
            ))
            .query_row_trailing(sort_toggle(this, cx));
        // A query suspends collapse, so the headers are plain labels while one
        // is active: leaving them clickable would let a click flip the persisted
        // fold with nothing moving on screen to show for it.
        if query.is_empty() {
            prompt = prompt.on_toggle_section(cx.listener(
                |this, label: &gpui::SharedString, _window, cx| {
                    toggle_section(this, label, cx);
                },
            ));
        }
        if row_menu.is_none() {
            prompt = prompt.selected_hint("Enter");
        }
        if this.repo_picker_sort_menu_open {
            prompt = prompt.list_override(sort_menu(this, cx));
        }

        components::context_menu(
            theme,
            prompt.render_with_remove(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, _e, _w, cx| {
                    if let Some(entry) = select_entries.get(ix).cloned() {
                        activate(this, entry, cx);
                    }
                },
                move |this, ix, _w, cx| {
                    if let Some(entry) = remove_entries.get(ix).cloned() {
                        forget(this, &entry, cx);
                    }
                },
            ),
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
        for (ix, (entry, item)) in entries.iter().enumerate() {
            let entry_section = item.section_label().map_or(OPEN_SECTION, |s| s.as_ref());
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
                RepoPickerEntry::Closed(path) => Some(path_display::path_display_shared(path)),
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
