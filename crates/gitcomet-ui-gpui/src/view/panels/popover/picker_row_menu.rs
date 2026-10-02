//! The context menu a picker row opens on right-click, floating over the picker
//! rather than replacing it.
//!
//! The picker stays open underneath with its row still highlighted, Escape backs
//! out of the menu before it closes the picker, and the arrow keys walk the
//! menu's entries while it is up.
//!
//! Every menu here is an ordinary [`ContextMenuModel`] whose entries carry
//! ordinary [`ContextMenuAction`]s, run by the shared executor. That is what
//! lets a picker decide what its rows offer without this module knowing what a
//! repository, a branch or a worktree is — and what lets the branch and worktree
//! rows reuse the very menus their sidebar rows open, rather than a second list
//! of actions to keep in step.

use super::*;
use crate::kit::click::PointerClickExt as _;

/// The row whose menu is open, and where to draw it.
#[derive(Clone)]
pub(super) struct PickerRowMenu {
    target: PickerRowMenuTarget,
    position: gpui::Point<gpui::Pixels>,
    /// Display index of the row the menu belongs to, so it stays highlighted
    /// while the menu floats somewhere else on screen.
    pub(super) display_index: usize,
    /// Filter text the menu was opened over. The rows are re-filtered as that
    /// text changes, which moves the row `display_index` names, so the menu is
    /// dismissed rather than left pointing somewhere else.
    query: String,
}

/// Which row the open menu belongs to. The one place this module knows anything
/// about the pickers, and the one place a fourth picker would touch.
#[derive(Clone)]
pub(super) enum PickerRowMenuTarget {
    Repo(repo_picker::RepoPickerEntry),
    FileHistoryCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        path: std::path::PathBuf,
    },
    /// A row of the branch badge's checkout picker. Reuses the menu the branch's
    /// sidebar row opens, so the two offer the same actions by construction.
    Branch {
        repo_id: RepoId,
        row: branch_picker::BranchPickerNavTarget,
    },
    /// A row of the workspace badge's picker. Reuses the menu the worktree's
    /// sidebar row opens.
    Worktree {
        repo_id: RepoId,
        row: worktree_badge_picker::WorktreeBadgeRow,
    },
}

impl PickerRowMenuTarget {
    /// The menu this row offers. Resolved fresh on every frame, so the entries
    /// rendered and the entries the arrow keys walk are one derivation.
    fn model(&self, this: &PopoverHost, cx: &gpui::Context<PopoverHost>) -> ContextMenuModel {
        match self {
            Self::Repo(entry) => this.repo_picker_row_menu_model(entry),
            Self::FileHistoryCommit {
                repo_id,
                commit_id,
                path,
            } => context_menu::file_history_commit::model(this, *repo_id, commit_id, path),
            Self::Branch { .. } | Self::Worktree { .. } => self
                .popover_kind(this)
                .and_then(|kind| this.context_menu_model(&kind, cx))
                .unwrap_or_else(|| ContextMenuModel::new(Vec::new())),
        }
    }

    /// The popover whose menu this row borrows, for the targets that borrow one.
    fn popover_kind(&self, this: &PopoverHost) -> Option<PopoverKind> {
        match self {
            Self::Repo(_) | Self::FileHistoryCommit { .. } => None,
            Self::Branch { repo_id, row } => match row {
                branch_picker::BranchPickerNavTarget::Ref(name) => Some(PopoverKind::BranchMenu {
                    repo_id: *repo_id,
                    target: BranchMenuTarget::local(name),
                }),
                branch_picker::BranchPickerNavTarget::RemoteBranch { remote, branch } => {
                    Some(PopoverKind::BranchMenu {
                        repo_id: *repo_id,
                        target: BranchMenuTarget::remote(remote, branch),
                    })
                }
                // The create row names a branch that does not exist yet, and a
                // menu entry is not a row at all.
                branch_picker::BranchPickerNavTarget::CreateBranch(_)
                | branch_picker::BranchPickerNavTarget::RowAction(_) => None,
            },
            Self::Worktree { repo_id, row } => match row {
                worktree_badge_picker::WorktreeBadgeRow::Worktree(path) => {
                    // The sidebar's menu names the branch checked out in the
                    // worktree, so look it up the same way its rows do.
                    let branch = this
                        .state
                        .repos
                        .iter()
                        .find(|repo| repo.id == *repo_id)
                        .and_then(|repo| match &repo.worktrees {
                            Loadable::Ready(worktrees) => worktrees
                                .iter()
                                .find(|worktree| &worktree.path == path)
                                .and_then(|worktree| worktree.branch.clone()),
                            _ => None,
                        });
                    Some(PopoverKind::worktree(
                        *repo_id,
                        WorktreePopoverKind::Menu {
                            path: path.clone(),
                            branch,
                        },
                    ))
                }
                // The create row names no worktree yet, and a menu entry is not
                // a row at all.
                worktree_badge_picker::WorktreeBadgeRow::CreateNew
                | worktree_badge_picker::WorktreeBadgeRow::RowAction(_) => None,
            },
        }
    }

    /// Whether this row has a menu at all. A row that does not is left alone by
    /// the right-click rather than opening an empty one.
    pub(super) fn has_menu(&self, this: &PopoverHost) -> bool {
        match self {
            Self::Repo(entry) => !this.repo_picker_row_menu_model(entry).items.is_empty(),
            Self::FileHistoryCommit { .. } => true,
            Self::Branch { .. } | Self::Worktree { .. } => self.popover_kind(this).is_some(),
        }
    }

    /// Where this row sits in the picker's list now, so dismissing the menu puts
    /// the selection back on it wherever it has moved to. `None` when the row is
    /// gone — filtered out, or closed from under the menu.
    fn row_position(&self, this: &PopoverHost, query: &str) -> Option<usize> {
        match self {
            Self::Repo(entry) => repo_picker::filtered_layout(this, query)
                .0
                .iter()
                .position(|candidate| candidate == entry),
            Self::FileHistoryCommit { commit_id, .. } => {
                let rows = file_history::cached(this, query);
                rows.layout
                    .item_indices
                    .iter()
                    .position(|ix| rows.payloads.get(*ix) == Some(commit_id))
            }
            Self::Branch { row, .. } => branch_picker::nav_targets(this, query)
                .iter()
                .position(|candidate| candidate == row),
            Self::Worktree { repo_id, row } => {
                worktree_badge_picker::nav_targets(this, *repo_id, query)
                    .iter()
                    .position(|candidate| candidate == row)
            }
        }
    }

    /// Whether the picker underneath survives an entry of this menu. Acting on a
    /// row usually leaves the picker up so the next row can be acted on; the
    /// entries that take you somewhere else are the exceptions.
    fn keeps_picker_open(&self, action: &ContextMenuAction) -> bool {
        match self {
            Self::FileHistoryCommit { .. } => matches!(action, ContextMenuAction::CopyText { .. }),
            Self::Repo(_) => matches!(
                action,
                ContextMenuAction::PinRepository { .. }
                    | ContextMenuAction::UnpinRepository { .. }
                    | ContextMenuAction::ForgetRecentRepository { .. }
                    | ContextMenuAction::CloseRepo { .. }
            ),
            // Every branch and worktree action either navigates away or opens a
            // prompt of its own, and the picker has nothing left to offer once
            // it has.
            Self::Branch { .. } | Self::Worktree { .. } => false,
        }
    }
}

impl PickerRowMenu {
    /// The menu this row is showing, for tests that compare it against the menu
    /// the same row opens elsewhere.
    #[cfg(test)]
    pub(super) fn model_for_test(
        &self,
        this: &PopoverHost,
        cx: &gpui::Context<PopoverHost>,
    ) -> ContextMenuModel {
        self.target.model(this, cx)
    }
}

/// Runs the `ix`th entry the arrow keys can reach. Looked up from the same
/// [`nav_actions`] the selection was made against, in the same frame, so the
/// entry that runs is the entry that was highlighted.
pub(super) fn activate_nth(
    this: &mut PopoverHost,
    ix: usize,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(action) = nav_actions(this, cx).and_then(|actions| actions.into_iter().nth(ix)) else {
        return;
    };
    activate(this, action, window, cx);
}

/// The entries the arrow keys walk while a menu is open — the enabled ones, from
/// the same model the menu renders.
pub(super) fn nav_actions(
    this: &PopoverHost,
    cx: &gpui::Context<PopoverHost>,
) -> Option<Vec<ContextMenuAction>> {
    let menu = this.picker_row_menu.as_ref()?;
    Some(
        menu.target
            .model(this, cx)
            .items
            .into_iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    action, disabled, ..
                } => (!disabled).then_some(*action),
                _ => None,
            })
            .collect(),
    )
}

/// The filter text the picker in front is showing right now. Empty before its
/// search input exists, which is also how the panel renders in that window.
fn current_query(this: &PopoverHost, cx: &gpui::App) -> String {
    this.open_picker_search_input()
        .map(|input| input.read(cx).text().trim().to_string())
        .unwrap_or_default()
}

pub(super) fn open(
    this: &mut PopoverHost,
    target: PickerRowMenuTarget,
    display_index: usize,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.picker_row_menu = Some(PickerRowMenu {
        target,
        position,
        display_index,
        query: current_query(this, cx),
    });
    this.picker_row_menu_scroll
        .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
    this.picker_row_menu_placement.borrow_mut().reset();
    this.picker_row_menu_arrows.reset();
    this.picker_row_menu_rows.borrow_mut().clear();
    this.picker_row_menu_revealed.set(None);
    // The selection index now addresses the menu's own actions, so it restarts
    // from nothing; the invoking row keeps its highlight through
    // `PickerRowMenu::display_index` instead.
    if let Some(index) = this.open_picker_selected_index() {
        *index = None;
    }
    this.repo_picker_sort_menu_open = false;
    cx.notify();
}

/// Dismisses the row menu without running anything — Escape, a press outside
/// it, or an edit to the filter. The selection lands back on the row the menu
/// belonged to, so arrowing on carries from there rather than restarting at the
/// top of the list.
pub(super) fn close(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    let Some(menu) = this.picker_row_menu.take() else {
        // No menu was open, so there is no selection of its to restore: leave
        // whatever the arrow keys were on alone.
        return;
    };
    // The rows can have been re-filtered, re-sorted or closed out from under the
    // menu while it floated over them, so look the invoking row up again rather
    // than trusting the index it had when the menu opened. Gone means gone: a
    // kept index would highlight a different repository.
    let query = current_query(this, cx);
    let restored = menu.target.row_position(this, &query);
    if let Some(index) = this.open_picker_selected_index() {
        *index = restored;
    }
    cx.notify();
}

/// Dismisses the row menu when the filter text has moved out from under it.
/// Called on every input notification, so it has to compare rather than close
/// unconditionally — the arrow keys reach the picker through the same input.
pub(super) fn close_on_query_change(
    this: &mut PopoverHost,
    query: &str,
    cx: &mut gpui::Context<PopoverHost>,
) {
    if this
        .picker_row_menu
        .as_ref()
        .is_some_and(|menu| menu.query != query)
    {
        close(this, cx);
    }
}

/// Closes the row menu on the way into one of its actions. Unlike a dismissal
/// this drops the selection: pinning, closing and forgetting all reorder the
/// rows, so the invoking row's index no longer names the same repository.
fn close_for_action(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    this.picker_row_menu = None;
    if let Some(index) = this.open_picker_selected_index() {
        *index = None;
    }
    cx.notify();
}

/// Runs one of the row menu's entries. The menu goes first — unlike a
/// dismissal this drops the selection, since pinning, closing and forgetting all
/// reorder the rows — and then the entry runs through the shared context-menu
/// executor, the same path every other menu in the app takes.
pub(super) fn activate(
    this: &mut PopoverHost,
    action: ContextMenuAction,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(target) = this
        .picker_row_menu
        .as_ref()
        .map(|menu| menu.target.clone())
    else {
        return;
    };
    close_for_action(this, cx);
    // Acting on a row leaves the picker up so the next one can be acted on
    // straight after; the entries that take you somewhere else are the ones that
    // close it, and they close it themselves.
    this.suppress_popover_close_after_action = target.keeps_picker_open(&action);
    this.context_menu_activate_action(action, window, cx);
    this.suppress_popover_close_after_action = false;
}

/// Hands the tooltip host a pointer position it would otherwise never see. Both
/// halves of the row-menu layer `occlude()`, which stops the root view's own
/// mouse-move listener from firing over them.
fn track_pointer_for_tooltips(
    this: &mut PopoverHost,
    e: &MouseMoveEvent,
    _window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let _ = this
        .tooltip_host
        .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
}

/// The floating row menu, drawn by [`PopoverHost::render`] above the picker.
/// Returns `None` unless a row menu is open.
pub(super) fn layer(
    this: &PopoverHost,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> Option<gpui::AnyElement> {
    let menu = this.picker_row_menu.clone()?;
    let model = menu.target.model(this, cx);
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let selected_index = this.open_picker_selected_index_value();

    let mut list = div()
        .flex()
        .flex_col()
        .w(super::REPO_TAB_MENU_WIDTH.preferred_px(ui_scale))
        .p(crate::ui_scale::design_px_from_percent(
            4.0,
            ui_scale_percent,
        ));
    // Same rule as the shared context menu: once any entry has an icon, the
    // icon-less ones keep its column so every label starts at one edge.
    let reserve_icon_column = model
        .items
        .iter()
        .any(|item| matches!(item, ContextMenuItem::Entry { icon: Some(_), .. }));
    // Only enabled entries are keyboard targets, so the menu's own selection
    // index counts those alone. `child_nav` maps each list child to that index.
    let mut nav_ix = 0usize;
    let mut child_nav: Vec<Option<usize>> = Vec::new();
    for (ix, item) in model.items.into_iter().enumerate() {
        let nav_target = matches!(
            item,
            ContextMenuItem::Entry {
                disabled: false,
                ..
            }
        );
        if !matches!(item, ContextMenuItem::Segmented { .. }) {
            child_nav.push(nav_target.then_some(nav_ix));
        }
        match item {
            ContextMenuItem::Separator => {
                list = list.child(components::context_menu_separator(theme, ui_scale_percent));
            }
            ContextMenuItem::Header(text) => {
                list = list.child(components::context_menu_header(
                    theme,
                    ui_scale_percent,
                    text,
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
            ContextMenuItem::Description(text) | ContextMenuItem::Label(text) => {
                list = list.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    text,
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
            ContextMenuItem::Entry {
                label,
                icon,
                action,
                disabled,
                ..
            } => {
                let selected = !disabled && selected_index == Some(nav_ix);
                if !disabled {
                    nav_ix += 1;
                }
                let icon_slot = match icon {
                    Some(icon) => components::ContextMenuIconSlot::Icon(icon),
                    None if reserve_icon_column => components::ContextMenuIconSlot::Reserved,
                    None => components::ContextMenuIconSlot::None,
                };
                list = list.child(
                    components::ContextMenuEntry::new(
                        ("picker_row_action", ix),
                        components::ContextMenuText::new(label),
                    )
                    .icon(icon_slot)
                    .disabled(disabled)
                    .selected(selected)
                    .on_select(theme, ui_scale_percent, cx, move |this, _e, window, cx| {
                        activate(this, (*action).clone(), window, cx);
                    })
                    .debug_selector(move || format!("picker_row_action_{ix}")),
                );
            }
            // Clicked rather than keyboard-selected, and no row menu has one.
            ContextMenuItem::Segmented { .. } => {}
        }
    }

    let dismiss_menu = cx.listener(|this, _e: &MouseDownEvent, _w, cx| {
        cx.stop_propagation();
        close(this, cx);
    });

    // Placed like every other context menu: flipped, then slid inside the
    // window, and only scrolled (behind arrows) when taller than the window.
    let scroll = this.picker_row_menu_scroll.clone();
    let placement = this.picker_row_menu_placement.clone();
    let now = cx.background_executor().now();
    this.picker_row_menu_arrows
        .drive(&scroll, window, ui_scale, now);
    if selected_index.is_some() {
        placement.borrow_mut().latch();
    }
    let rows = this.picker_row_menu_rows.clone();
    rows.borrow_mut().resize(nav_ix, None);
    if this.picker_row_menu_revealed.replace(selected_index) != selected_index
        && let Some((top, height)) = selected_index
            .and_then(|nav| rows.borrow().get(nav).copied())
            .flatten()
    {
        let offset = scroll.offset();
        let target = crate::kit::menu_placement::reveal_scroll(
            top,
            top + height,
            scroll.bounds().size.height,
            scroll.max_offset().y,
            -offset.y,
            ui_scale.px(components::MENU_SCROLL_ARROW_HEIGHT_PX),
        );
        scroll.set_offset(gpui::point(offset.x, -target));
    }
    let list = list.on_children_prepainted(context_menu::record_row_extents(
        rows,
        scroll.clone(),
        move |child| child_nav.get(child).copied().flatten(),
    ));

    let request = crate::kit::menu_placement::MenuRequest {
        anchor: crate::kit::menu_placement::MenuAnchor::Point(menu.position),
        prefer_below: true,
        prefer_right: true,
        gap: gpui::px(0.0),
    };
    let limits = super::menu_layer::menu_limits(
        window,
        crate::ui_scale::design_px_from_percent(16.0, ui_scale_percent),
    );
    // Less the surface's 1px border on both sides.
    let content_max_h =
        (placement.borrow().height_cap(&request, limits) - gpui::px(2.0)).max(gpui::px(0.0));
    let latch_on_wheel = placement.clone();
    let area = crate::view::restrict_scroll_to_vertical_axis(
        div()
            .id("picker_row_menu_scroll")
            .debug_selector(|| "picker_row_menu_scroll".to_string())
            .min_h(gpui::px(0.0))
            .max_h(content_max_h)
            .track_scroll(&scroll)
            .overflow_y_scroll()
            .on_scroll_wheel(move |_e, _w, _cx| latch_on_wheel.borrow_mut().latch()),
    )
    .child(list);
    let arrows = this.picker_row_menu_arrows.clone();
    let up = arrows.arrow(
        components::MenuScrollDirection::Up,
        &scroll,
        &placement,
        "picker_row_menu_scroll",
        theme,
        ui_scale,
        cx,
    );
    let down = arrows.arrow(
        components::MenuScrollDirection::Down,
        &scroll,
        &placement,
        "picker_row_menu_scroll",
        theme,
        ui_scale,
        cx,
    );
    let list = super::menu_layer::scroll_area_with_arrows(area, up, down);
    let latch_on_press = placement.clone();
    let latch_on_move = placement.clone();

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
                    .id("picker_row_menu_scrim")
                    .debug_selector(|| "picker_row_menu_scrim".to_string())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(gpui::rgba(0x00000000))
                    .occlude()
                    // Both this and the menu below occlude, which silences the
                    // root view's mouse tracking, so both have to feed the
                    // tooltip host themselves — otherwise a truncated-text
                    // tooltip from the picker underneath stays painted wherever
                    // the pointer was when the menu opened.
                    .on_mouse_move(cx.listener(track_pointer_for_tooltips))
                    .on_any_pointer_click(dismiss_menu),
            )
            .child(crate::kit::menu_placement::menu_placement(
                placement,
                request,
                limits,
                // This menu is its own floating layer rather than a panel
                // inside the popover container, so it has to bring the
                // elevated surface with it — `components::context_menu` is
                // layout only.
                components::popover_surface(theme)
                    .id("picker_row_menu")
                    .debug_selector(|| "picker_row_menu".to_string())
                    .occlude()
                    .on_any_mouse_down(move |_e, _w, cx| {
                        latch_on_press.borrow_mut().latch();
                        cx.stop_propagation();
                    })
                    .on_mouse_move(move |_e, _w, _cx| latch_on_move.borrow_mut().latch())
                    .on_mouse_move(cx.listener(track_pointer_for_tooltips))
                    .child(components::context_menu(theme, list)),
            ))
            .into_any_element(),
    )
}
