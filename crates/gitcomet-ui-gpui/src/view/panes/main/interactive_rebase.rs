use super::super::super::*;
use super::helpers::{IRebaseDragState, IRebaseViewState};
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::{cell::RefCell, rc::Rc};

const ACTION_BTN_W: f32 = 76.0;

fn squash_target(entries: &[InteractiveRebaseEntry], k: usize) -> Option<usize> {
    (0..k)
        .rev()
        .find(|&j| entries[j].action != InteractiveRebaseAction::Drop)
}

/// Whether any squash/fixup entry currently folds into the entry at `ix`.
fn entry_is_squash_target(entries: &[InteractiveRebaseEntry], ix: usize) -> bool {
    (0..entries.len()).any(|k| {
        matches!(
            entries[k].action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        ) && squash_target(entries, k) == Some(ix)
    })
}

/// Subject used to group commits for auto-squash. Strips leading `fixup! ` /
/// `squash! ` / `amend! ` prefixes (git's autosquash convention, which SmartGit
/// follows) so a `fixup! foo` commit groups with `foo`.
fn autosquash_group_key(summary: &str) -> &str {
    let mut s = summary;
    loop {
        match s
            .strip_prefix("fixup! ")
            .or_else(|| s.strip_prefix("squash! "))
            .or_else(|| s.strip_prefix("amend! "))
        {
            Some(rest) => s = rest,
            None => return s,
        }
    }
}

/// Whether a subject carries an auto-squash prefix — such a commit is not kept
/// as the survivor when an unprefixed sibling exists, so the folded commit's
/// message doesn't leak into the result.
fn is_autosquash_prefixed(summary: &str) -> bool {
    autosquash_group_key(summary) != summary
}

/// The commit ids folded into each survivor for a given auto-squash `mode`.
/// Groups commits by their [`autosquash_group_key`] (so `fixup!`/`squash!`
/// commits fold into their target); the surviving commit per group is chosen by
/// the mode, preferring an unprefixed commit so its message is kept. Empty
/// summaries are never eligible. `entries` are ordered oldest-first (index 0 =
/// oldest).
///
/// Returns `folded_into[i] = Some(survivor_index)` for every commit that is
/// folded away, and `None` for survivors and untouched commits.
fn autosquash_folds(
    entries: &[InteractiveRebaseEntry],
    mode: AutosquashMode,
) -> Vec<Option<usize>> {
    let n = entries.len();
    let mut folded_into: Vec<Option<usize>> = vec![None; n];
    // Pick a group's survivor: prefer an unprefixed member (so its message is
    // kept over a `fixup!`/`squash!` one), then apply the mode's position rule.
    let choose_survivor = |candidates: &[usize]| -> usize {
        let unprefixed: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&i| !is_autosquash_prefixed(entries[i].summary.as_str()))
            .collect();
        let pool: &[usize] = if unprefixed.is_empty() {
            candidates
        } else {
            &unprefixed
        };
        match mode {
            // Highest index = newest commit; lowest = oldest.
            AutosquashMode::ToTop => *pool.iter().max().unwrap(),
            _ => *pool.iter().min().unwrap(),
        }
    };
    match mode {
        AutosquashMode::ToTop | AutosquashMode::ToBottom => {
            // Group indices by normalized summary, preserving first-seen order.
            let mut order: Vec<&str> = Vec::new();
            let mut groups: std::collections::HashMap<&str, Vec<usize>> =
                std::collections::HashMap::new();
            for (i, e) in entries.iter().enumerate() {
                let key = autosquash_group_key(e.summary.as_str());
                if key.trim().is_empty() {
                    continue;
                }
                groups
                    .entry(key)
                    .or_insert_with(|| {
                        order.push(key);
                        Vec::new()
                    })
                    .push(i);
            }
            for key in order {
                let indices = &groups[key];
                if indices.len() < 2 {
                    continue;
                }
                let survivor = choose_survivor(indices);
                for &i in indices {
                    if i != survivor {
                        folded_into[i] = Some(survivor);
                    }
                }
            }
        }
        AutosquashMode::Neighbor => {
            // Collapse each maximal run of adjacent commits with the same
            // normalized summary into one survivor (an unprefixed member if any,
            // else the run's oldest).
            let mut i = 0;
            while i < n {
                let key = autosquash_group_key(entries[i].summary.as_str());
                if key.trim().is_empty() {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < n && autosquash_group_key(entries[j].summary.as_str()) == key {
                    j += 1;
                }
                let run: Vec<usize> = (i..j).collect();
                if run.len() >= 2 {
                    let survivor = choose_survivor(&run);
                    for &k in &run {
                        if k != survivor {
                            folded_into[k] = Some(survivor);
                        }
                    }
                }
                i = j;
            }
        }
    }
    folded_into
}

/// Applies `mode` to `original`, producing the collapsed entry list (one row
/// per surviving/untouched commit) plus the survivor-id → folded-fixup map.
/// The folded map is empty when nothing was eligible.
fn compute_autosquash(
    original: &[InteractiveRebaseEntry],
    mode: AutosquashMode,
) -> (
    Vec<InteractiveRebaseEntry>,
    std::collections::HashMap<String, Vec<InteractiveRebaseEntry>>,
) {
    let folded_into = autosquash_folds(original, mode);
    let mut collapsed = Vec::with_capacity(original.len());
    let mut folded: std::collections::HashMap<String, Vec<InteractiveRebaseEntry>> =
        std::collections::HashMap::new();
    for (i, e) in original.iter().enumerate() {
        match folded_into[i] {
            Some(survivor) => {
                let mut fixup = e.clone();
                fixup.action = InteractiveRebaseAction::Fixup;
                fixup.new_message = None;
                folded
                    .entry(original[survivor].commit_id.clone())
                    .or_default()
                    .push(fixup);
            }
            None => collapsed.push(e.clone()),
        }
    }
    (collapsed, folded)
}

/// Re-expands the collapsed editing list into the todo the rebase executor
/// consumes: each survivor is followed by its folded commits as `fixup` entries.
/// A dropped survivor takes its folded commits with it.
fn expand_folded(
    entries: &[InteractiveRebaseEntry],
    folded: &std::collections::HashMap<String, Vec<InteractiveRebaseEntry>>,
) -> Vec<InteractiveRebaseEntry> {
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        out.push(e.clone());
        if e.action != InteractiveRebaseAction::Drop
            && let Some(fixups) = folded.get(&e.commit_id) {
                out.extend(fixups.iter().cloned());
            }
    }
    out
}

fn validate_squash_entries(entries: &mut [InteractiveRebaseEntry]) {
    // First pass: a squash/fixup with no surviving target above it becomes a
    // plain pick.
    for k in 0..entries.len() {
        if !matches!(
            entries[k].action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        ) {
            continue;
        }
        let has_target = (0..k)
            .rev()
            .any(|j| entries[j].action != InteractiveRebaseAction::Drop);
        if !has_target {
            entries[k].action = InteractiveRebaseAction::Pick;
        }
    }
    // Second pass: an entry auto-promoted to Reword solely to absorb a squash
    // reverts to Pick once nothing folds into it and the user never typed a
    // replacement message. Runs after the squash pass so a squash that just
    // reverted to Pick correctly strands its former target. Mirrors the
    // targeted cleanup in `set_rebase_action` for the reorder paths (▲/▼, drag)
    // which previously only ran the squash pass.
    for k in 0..entries.len() {
        if entries[k].action == InteractiveRebaseAction::Reword
            && entries[k].new_message.is_none()
            && !entry_is_squash_target(entries, k)
        {
            entries[k].action = InteractiveRebaseAction::Pick;
        }
    }
}

fn non_drop_count(entries: &[InteractiveRebaseEntry]) -> usize {
    entries
        .iter()
        .filter(|e| e.action != InteractiveRebaseAction::Drop)
        .count()
}

fn action_short_label(action: InteractiveRebaseAction) -> &'static str {
    match action {
        InteractiveRebaseAction::Pick => "pick",
        InteractiveRebaseAction::Reword => "reword",
        InteractiveRebaseAction::Drop => "drop",
        InteractiveRebaseAction::Squash => "squash",
        InteractiveRebaseAction::Fixup => "fixup",
        InteractiveRebaseAction::Edit => "edit",
    }
}

#[derive(Clone, Copy, Debug)]
struct IRebaseDragValue {
    ix: usize,
}

/// Content signature of the rebase list: changes whenever a row's height
/// could change (reorder, action change, or the folded-into map). Drives the
/// `ListState` remeasure/reset in `interactive_rebase_view`.
fn irebase_list_sig(st: &IRebaseViewState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    for e in &st.entries {
        e.commit_id.hash(&mut h);
        (e.action as u8).hash(&mut h);
    }
    // Folded groups add a header line (taller row); key on survivor + count.
    let mut folded: Vec<(&String, usize)> = st.folded.iter().map(|(k, v)| (k, v.len())).collect();
    folded.sort();
    folded.hash(&mut h);
    h.finish()
}

/// Floating preview shown under the cursor while dragging a rebase entry, so
/// the in-list row can stay put (dimmed) with nothing painted over neighbours.
struct IRebaseDragPreview {
    theme: AppTheme,
    ui_scale_percent: u32,
    action: InteractiveRebaseAction,
    sha: String,
    summary: String,
    row_h: f32,
}

impl Render for IRebaseDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let action_btn_w = px(ACTION_BTN_W * self.ui_scale_percent as f32 / 100.0);
        let is_squash_like = matches!(
            self.action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        );
        let outlined_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.38 } else { 0.28 },
        );
        div()
            .h(px(self.row_h))
            .w(px(440.0 * self.ui_scale_percent as f32 / 100.0))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(px(theme.radii.row))
            .bg(theme.colors.surface_bg_elevated)
            .border_1()
            .border_color(with_alpha(theme.colors.accent, 0.6))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child("⠿"),
            )
            .when(is_squash_like, |d| {
                d.child(div().flex_shrink_0().flex().items_center().child(
                    crate::view::icons::svg_icon(
                        "icons/squash_arrow.svg",
                        with_alpha(theme.colors.accent, 0.7),
                        px(14.0),
                    ),
                ))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .w(action_btn_w)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.radii.row))
                    .border_1()
                    .border_color(outlined_border)
                    .text_sm()
                    .text_color(theme.colors.text)
                    .child(format!("{} ▾", action_short_label(self.action))),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .font_family("monospace")
                    .child(self.sha.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(theme.colors.text)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .child(self.summary.clone()),
            )
    }
}

impl MainPaneView {
    /// The active repo's interactive rebase editing state, if a setup is open.
    pub(in crate::view) fn active_irebase(&self) -> Option<&IRebaseViewState> {
        self.interactive_rebase_states.get(&self.active_repo_id()?)
    }

    pub(in crate::view) fn active_irebase_mut(&mut self) -> Option<&mut IRebaseViewState> {
        let repo_id = self.active_repo_id()?;
        self.interactive_rebase_states.get_mut(&repo_id)
    }

    /// Whether a later commit squashes into the active-setup entry at `ix`.
    pub(in crate::view) fn active_entry_is_squash_target(&self, ix: usize) -> bool {
        self.active_irebase()
            .is_some_and(|st| entry_is_squash_target(&st.entries, ix))
    }

    /// Applies an auto-squash `mode` to the active setup as a one-shot action,
    /// recomputing from the original commit list. Returns `false` when no
    /// commits were eligible — the caller then surfaces a toast and the state is
    /// left unchanged.
    pub(in crate::view) fn apply_autosquash_mode(
        &mut self,
        mode: AutosquashMode,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(st) = self.active_irebase_mut() else {
            return true;
        };
        let (collapsed, folded) = compute_autosquash(&st.original_entries, mode);
        if folded.is_empty() {
            return false;
        }
        st.autosquash_mode = Some(mode);
        st.entries = collapsed;
        st.folded = folded;
        cx.notify();
        true
    }

    pub(in crate::view) fn set_rebase_action(
        &mut self,
        ix: usize,
        action: InteractiveRebaseAction,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(st) = self.active_irebase_mut() else {
            return;
        };
        if ix >= st.entries.len() {
            return;
        }

        // Prevent dropping the last non-dropped commit.
        if action == InteractiveRebaseAction::Drop {
            let current = st.entries[ix].action;
            if current != InteractiveRebaseAction::Drop && non_drop_count(&st.entries) <= 1 {
                return;
            }
        }

        let old_action = st.entries[ix].action;
        // Capture the former squash target before we change the action.
        let former_squash_target = if matches!(
            old_action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        ) {
            squash_target(&st.entries, ix)
        } else {
            None
        };

        st.entries[ix].action = action;

        if action == InteractiveRebaseAction::Squash {
            // Auto-set the new target to Reword so the combined message can be written.
            if let Some(j) = squash_target(&st.entries, ix)
                && st.entries[j].action == InteractiveRebaseAction::Pick {
                    st.entries[j].action = InteractiveRebaseAction::Reword;
                }
        } else if let Some(j) = former_squash_target {
            // Was Squash/Fixup, now it isn't. If the former target is Reword and nothing
            // else is squashing into it, revert it back to Pick.
            if st.entries[j].action == InteractiveRebaseAction::Reword {
                let still_targeted = (0..st.entries.len()).any(|k| {
                    matches!(
                        st.entries[k].action,
                        InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
                    ) && squash_target(&st.entries, k) == Some(j)
                });
                if !still_targeted && st.entries[j].new_message.is_none() {
                    st.entries[j].action = InteractiveRebaseAction::Pick;
                }
            }
        }

        if action == InteractiveRebaseAction::Drop {
            validate_squash_entries(&mut st.entries);
        }

        cx.notify();
    }

    /// Commit the pending drag reorder. Shared by every way a drag can end
    /// (drop on the list, drop outside it, mouse released out of the window)
    /// so the paths cannot diverge. Returns true if there was a drag to end.
    fn commit_interactive_rebase_drag(&mut self) -> bool {
        let Some(st) = self.active_irebase_mut() else {
            return false;
        };
        let Some(state) = st.drag_state.take() else {
            return false;
        };
        if state.from_ix != state.to_ix
            && state.from_ix < st.entries.len()
            && state.to_ix < st.entries.len()
        {
            let entry = st.entries.remove(state.from_ix);
            st.entries.insert(state.to_ix, entry);
            validate_squash_entries(&mut st.entries);
            // Fade the landed row in at its new position.
            let ver = st.reorder_anim.map(|(_, _, v)| v + 1).unwrap_or(0);
            st.reorder_anim = Some((state.to_ix, state.to_ix, ver));
        }
        true
    }

    /// Update the drop target from a drag-move over the virtualized list.
    /// Uses per-item bounds (rows are variable height) and auto-scrolls near
    /// the viewport edges.
    fn irebase_drag_move(
        &mut self,
        e: &gpui::DragMoveEvent<IRebaseDragValue>,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let from_ix = e.drag(cx).ix;
        let Some(st) = self.interactive_rebase_states.get_mut(&repo_id) else {
            return;
        };
        let entry_count = st.entries.len();
        if entry_count == 0 {
            return;
        }
        let Some(list) = st.scroll.clone() else {
            return;
        };

        // Auto-scroll while the pointer is near the viewport edges.
        let vp_h = e.bounds.size.height;
        let pointer_vp = e.event.position.y - e.bounds.origin.y;
        let edge = px(28.0);
        if pointer_vp < edge {
            list.scroll_by(-(edge - pointer_vp));
            cx.notify();
        } else if pointer_vp > vp_h - edge {
            list.scroll_by(pointer_vp - (vp_h - edge));
            cx.notify();
        }

        // Insertion position in display order (0..=entry_count): the first
        // laid-out row whose vertical midpoint is below the pointer; past the
        // last row means "append at the bottom".
        let pointer_y = e.event.position.y;
        let mut display_pos = entry_count;
        for dp in 0..entry_count {
            if let Some(b) = list.bounds_for_item(dp)
                && pointer_y < b.origin.y + b.size.height / 2.0 {
                    display_pos = dp;
                    break;
                }
        }

        let source_dp = (entry_count - 1).saturating_sub(from_ix);
        let to_ix = if display_pos <= source_dp {
            entry_count - 1 - display_pos
        } else {
            entry_count - display_pos
        };
        let already = st
            .drag_state
            .is_some_and(|s| s.from_ix == from_ix && s.display_pos == display_pos);
        if !already {
            st.drag_state = Some(IRebaseDragState {
                from_ix,
                to_ix,
                display_pos,
            });
            cx.notify();
        }
    }

    /// Render one rebase row for the virtualized list. `display_pos` is the
    /// list index (newest-first); the data index is `entry_count-1-display_pos`.
    fn render_irebase_row(
        &mut self,
        display_pos: usize,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let selected_commit_id = self
            .active_repo()
            .and_then(|r| r.history_state.selected_commit.as_ref())
            .map(|c| c.0.as_ref().to_owned());
        let Some(st) = self.interactive_rebase_states.get(&repo_id) else {
            return div().into_any_element();
        };
        let entry_count = st.entries.len();
        if display_pos >= entry_count {
            return div().into_any_element();
        }
        let ix = entry_count - 1 - display_pos;
        let reorder_anim = st.reorder_anim;
        let drag_state = st.drag_state;
        let drag_from_ix = drag_state.map(|s| s.from_ix).unwrap_or(usize::MAX);
        let insertion_pos = drag_state.map(|s| s.display_pos);
        let is_drag_source = ix == drag_from_ix;
        let is_bottom = display_pos + 1 >= entry_count;
        // Insertion indicator: a line above this row when it is the drop
        // target, or below it when dropping past the last row. Absolutely
        // positioned so it never changes row height (no remeasure churn).
        let show_top_line = insertion_pos == Some(display_pos);
        let show_bottom_line = is_bottom && insertion_pos == Some(entry_count);

        let action = st.entries[ix].action;
        let sha = st.entries[ix]
            .commit_id
            .get(..8)
            .unwrap_or(&st.entries[ix].commit_id)
            .to_string();
        let summary = st.entries[ix]
            .new_message
            .as_deref()
            .and_then(|m| m.lines().next())
            .unwrap_or(&st.entries[ix].summary)
            .to_owned();
        let is_selected = selected_commit_id
            .as_deref()
            .is_some_and(|s| s == st.entries[ix].commit_id);
        let is_squash_like = matches!(
            action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        );
        let is_dropped = action == InteractiveRebaseAction::Drop;
        let autosquash_active = st.autosquash_mode.is_some();
        let is_autosquash_eligible = !autosquash_active && {
            let key = autosquash_group_key(st.entries[ix].summary.as_str());
            !key.trim().is_empty()
                && st
                    .entries
                    .iter()
                    .filter(|e| autosquash_group_key(e.summary.as_str()) == key)
                    .count()
                    > 1
        };
        let folded_shas: Vec<String> = st
            .folded
            .get(&st.entries[ix].commit_id)
            .map(|folds| {
                folds
                    .iter()
                    .map(|f| f.commit_id.get(..8).unwrap_or(&f.commit_id).to_string())
                    .collect()
            })
            .unwrap_or_default();

        let btn_bounds: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>> =
            Rc::new(RefCell::new(None));
        let btn_bounds_prepaint = Rc::clone(&btn_bounds);
        let action_btn_w = px(ACTION_BTN_W * ui_scale_percent as f32 / 100.0);
        let action_label = format!("{} ▾", action_short_label(action));

        let inner_btn = components::Button::new(format!("action_{ix}"), action_label)
            .style(components::ButtonStyle::Outlined)
            .render(theme, ui_scale_percent)
            .w(action_btn_w)
            .flex_shrink_0()
            .on_click(cx.listener(move |this, _e, window, cx| {
                let bounds = (*btn_bounds.borrow()).unwrap_or(gpui::Bounds {
                    origin: gpui::point(px(0.0), px(0.0)),
                    size: gpui::size(px(0.0), px(0.0)),
                });
                let Some(st) = this.interactive_rebase_states.get(&repo_id) else {
                    return;
                };
                let nd = non_drop_count(&st.entries);
                let current_action = st.entries.get(ix).map(|e| e.action);
                let can_drop = current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
                let can_squash = squash_target(&st.entries, ix).is_some();
                let wh = window.window_handle();
                let root = this.root_view.clone();
                cx.defer(move |cx| {
                    let _ = wh.update(cx, |_, window, cx| {
                        let _ = root.update(cx, |root, cx| {
                            root.open_popover_for_bounds(
                                PopoverKind::InteractiveRebaseActionMenu {
                                    ix,
                                    is_bottom,
                                    can_drop,
                                    can_squash,
                                },
                                bounds,
                                window,
                                cx,
                            );
                        });
                    });
                });
            }));

        let action_btn = div()
            .on_children_prepainted(move |children_bounds, _w, _cx| {
                if let Some(b) = children_bounds.first() {
                    *btn_bounds_prepaint.borrow_mut() = Some(*b);
                }
            })
            .child(inner_btn)
            .id(format!("action_w_{ix}"));

        let up_btn = components::Button::new(format!("up_{ix}"), "▲")
            .style(components::ButtonStyle::Subtle)
            .no_focus()
            .disabled(display_pos == 0)
            .render(theme, ui_scale_percent)
            .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                let Some(st) = this.interactive_rebase_states.get_mut(&repo_id) else {
                    return;
                };
                let len = st.entries.len();
                let entry_display_pos = len - 1 - ix;
                if entry_display_pos > 0 {
                    let swap_ix = len - 1 - (entry_display_pos - 1);
                    st.entries.swap(ix, swap_ix);
                    validate_squash_entries(&mut st.entries);
                    let ver = st.reorder_anim.map(|(_, _, v)| v + 1).unwrap_or(0);
                    st.reorder_anim = Some((ix, swap_ix, ver));
                }
                cx.notify();
            }));

        let down_btn = components::Button::new(format!("down_{ix}"), "▼")
            .style(components::ButtonStyle::Subtle)
            .no_focus()
            .disabled(display_pos + 1 >= entry_count)
            .render(theme, ui_scale_percent)
            .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                let Some(st) = this.interactive_rebase_states.get_mut(&repo_id) else {
                    return;
                };
                let len = st.entries.len();
                let entry_display_pos = len - 1 - ix;
                if entry_display_pos + 1 < len {
                    let swap_ix = len - 1 - (entry_display_pos + 1);
                    st.entries.swap(ix, swap_ix);
                    validate_squash_entries(&mut st.entries);
                    let ver = st.reorder_anim.map(|(_, _, v)| v + 1).unwrap_or(0);
                    st.reorder_anim = Some((ix, swap_ix, ver));
                }
                cx.notify();
            }));

        let drag_val = IRebaseDragValue { ix };
        let pf_action = action;
        let pf_sha = sha.clone();
        let pf_summary = summary.clone();
        let preview_row_h = 28.0 * ui_scale_percent as f32 / 100.0;
        let gripper = div()
            .id(("gripper", ix))
            .cursor(gpui::CursorStyle::PointingHand)
            .text_xs()
            .text_color(theme.colors.text_muted)
            .child("⠿")
            .on_drag(drag_val, move |_drag, _offset, _window, cx| {
                cx.new(|_cx| IRebaseDragPreview {
                    theme,
                    ui_scale_percent,
                    action: pf_action,
                    sha: pf_sha.clone(),
                    summary: pf_summary.clone(),
                    row_h: preview_row_h,
                })
            });

        let commit_id_val = CommitId(st.entries[ix].commit_id.clone().into());
        let accent = theme.colors.accent;
        let row_div = div()
            .id(("irebase_row", ix))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .px_2()
            .py_0p5()
            .rounded(px(theme.radii.row))
            .when(!is_drag_source && is_selected, |d| {
                d.bg(theme.colors.active)
            })
            .when(!is_drag_source && !is_selected, |d| {
                d.hover(move |s| s.bg(theme.colors.hover))
            })
            .when(is_dropped, |d| d.opacity(0.5))
            .when(show_top_line, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(accent),
                )
            })
            .when(show_bottom_line, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(accent),
                )
            })
            // The folded-commit list sits above the survivor row.
            .when(!folded_shas.is_empty(), |d| {
                d.child(
                    div()
                        .pl(px(20.0))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(crate::view::icons::svg_icon(
                            "icons/squash_arrow.svg",
                            with_alpha(theme.colors.accent, 0.7),
                            px(12.0),
                        ))
                        .child(format!("squashed {}", folded_shas.join(", "))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(gripper)
                    .when(is_squash_like, |d| {
                        d.child(div().flex_shrink_0().flex().items_center().child(
                            crate::view::icons::svg_icon(
                                "icons/squash_arrow.svg",
                                with_alpha(theme.colors.accent, 0.7),
                                px(14.0),
                            ),
                        ))
                    })
                    .child(action_btn)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .font_family("monospace")
                            .child(sha.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.colors.text)
                            .when(is_autosquash_eligible, |d| {
                                d.text_color(theme.colors.accent)
                            })
                            .overflow_x_hidden()
                            .whitespace_nowrap()
                            .when(is_dropped, |d| d.line_through())
                            .child(summary),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap_0p5()
                            .child(up_btn)
                            .child(down_btn),
                    ),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _e: &gpui::MouseDownEvent, _w, cx| {
                    this.store.dispatch(Msg::SelectCommit {
                        repo_id,
                        commit_id: commit_id_val.clone(),
                    });
                    cx.notify();
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseUpEvent, window, cx| {
                    cx.stop_propagation();
                    let Some(st) = this.interactive_rebase_states.get(&repo_id) else {
                        return;
                    };
                    let nd = non_drop_count(&st.entries);
                    let current_action = st.entries.get(ix).map(|e| e.action);
                    let can_drop = current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
                    let can_squash = squash_target(&st.entries, ix).is_some();
                    let wh = window.window_handle();
                    let root = this.root_view.clone();
                    let pos = e.position;
                    cx.defer(move |cx| {
                        let _ = wh.update(cx, |_, window, cx| {
                            let _ = root.update(cx, |root, cx| {
                                root.open_popover_at(
                                    PopoverKind::InteractiveRebaseActionMenu {
                                        ix,
                                        is_bottom,
                                        can_drop,
                                        can_squash,
                                    },
                                    pos,
                                    window,
                                    cx,
                                );
                            });
                        });
                    });
                }),
            );

        if is_drag_source {
            row_div
                .with_animation(
                    ("irebase_source_dim", ix),
                    Animation::new(Duration::from_millis(120)).with_easing(gpui::ease_out_quint()),
                    |d, delta| d.opacity(1.0 - 0.6 * delta),
                )
                .into_any_element()
        } else if let Some((aix, bix, ver)) = reorder_anim {
            if ix == aix || ix == bix {
                row_div
                    .with_animation(
                        format!("reorder_{ix}_{ver}"),
                        Animation::new(Duration::from_millis(200))
                            .with_easing(gpui::ease_out_quint()),
                        |d, delta| d.opacity(delta),
                    )
                    .into_any_element()
            } else {
                row_div.into_any_element()
            }
        } else {
            row_div.into_any_element()
        }
    }

    pub(in crate::view) fn interactive_rebase_view(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;

        // Sync the variable-height virtualized list state before borrowing the
        // repo below (this needs `&mut self`; the match on `repo` holds `self`
        // immutably for its whole body).
        if let Some(repo_id) = self.active_repo_id()
            && self.interactive_rebase_states.contains_key(&repo_id) {
                let sig = irebase_list_sig(&self.interactive_rebase_states[&repo_id]);
                let count = self.interactive_rebase_states[&repo_id].entries.len();
                let st = self
                    .interactive_rebase_states
                    .get_mut(&repo_id)
                    .expect("checked contains_key");
                if let Some(ls) = &st.scroll {
                    if st.list_sig != (sig, count) {
                        if st.list_sig.1 == count {
                            ls.remeasure();
                        } else {
                            ls.reset(count);
                        }
                        st.list_sig = (sig, count);
                    }
                } else {
                    st.scroll = Some(gpui::ListState::new(
                        count,
                        gpui::ListAlignment::Top,
                        px(400.0),
                    ));
                    st.list_sig = (sig, count);
                }
            }

        let Some(repo) = self.active_repo() else {
            return div().child("No active repo");
        };
        let Some(setup) = repo.interactive_rebase_setup.as_ref() else {
            return div().child("No interactive rebase setup");
        };
        let repo_id = repo.id;
        let base = setup.base.clone();
        // Only abbreviate full 40-char SHAs; leave branch names intact.
        let base_short: SharedString =
            if base.len() > 16 && base.chars().all(|c| c.is_ascii_hexdigit()) {
                base.get(..8).unwrap_or(&base).to_string().into()
            } else {
                base.clone().into()
            };
        // Prefer a branch name that points at the base commit; fall back to the
        // abbreviated sha.
        let base_display: SharedString = match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|b| b.target.as_ref() == base.as_str())
                .map(|b| SharedString::from(b.name.clone()))
                .unwrap_or_else(|| base_short.clone()),
            _ => base_short.clone(),
        };

        let loading_state = &setup.entries;
        let entry_content: gpui::AnyElement = match loading_state {
            Loadable::NotLoaded => div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("Preparing…")
                .into_any_element(),
            Loadable::Loading => div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("Loading commits…")
                .into_any_element(),
            Loadable::Error(e) => div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Error: {e}"))
                .into_any_element(),
            // The map entry is populated by `apply_state` on the same state
            // application that made the entries Ready; guard anyway.
            // Nothing to rebase (base is already at HEAD): the editing UI is
            // useless here, so show a plain message instead of an empty list.
            // The footer hides its rebase options in the same empty case.
            Loadable::Ready(_)
                if self
                    .interactive_rebase_states
                    .get(&repo_id)
                    .is_some_and(|st| st.entries.is_empty()) =>
            {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(format!("No commits to rebase onto {base_display}."))
                    .into_any_element()
            }
            Loadable::Ready(_) if self.interactive_rebase_states.contains_key(&repo_id) => {
                // ListState was created/synced at the top of this fn.
                let scroll = self.interactive_rebase_states[&repo_id]
                    .scroll
                    .clone()
                    .expect("synced above");

                let list_el = gpui::list(
                    scroll.clone(),
                    cx.processor(move |this, ix: usize, _window, cx| {
                        this.render_irebase_row(ix, repo_id, cx)
                    }),
                )
                .size_full();

                let scrollbar_gutter = components::Scrollbar::visible_gutter(
                    scroll.clone(),
                    components::ScrollbarAxis::Vertical,
                );

                // `list` (unlike `uniform_list`) is not interactive, so the drag
                // handlers live on the wrapping div.
                let list_wrap = div()
                    .id("irebase_list_wrap")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .pr(scrollbar_gutter)
                    .on_drag_move(cx.listener(
                        move |this, e: &gpui::DragMoveEvent<IRebaseDragValue>, _w, cx| {
                            this.irebase_drag_move(e, repo_id, cx);
                        },
                    ))
                    .can_drop(move |dragged, _window, _cx| {
                        dragged.downcast_ref::<IRebaseDragValue>().is_some()
                    })
                    .on_drop(cx.listener(move |this, _drag: &IRebaseDragValue, _w, cx| {
                        this.commit_interactive_rebase_drag();
                        cx.notify();
                    }))
                    .child(list_el);

                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(list_wrap)
                    .child(
                        components::Scrollbar::new("irebase_scrollbar", scroll.clone())
                            .render(theme),
                    )
                    .into_any_element()
            }
            // Ready, but apply_state has not populated the editing state yet.
            Loadable::Ready(_) => div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("Loading commits…")
                .into_any_element(),
        };

        let (is_modified, entries_empty) = self
            .interactive_rebase_states
            .get(&repo_id)
            .map(|st| (st.entries != st.original_entries, st.entries.is_empty()))
            .unwrap_or((false, true));

        div()
            .flex()
            .flex_col()
            .size_full()
            // Safety net: end the drag for drops that land outside the scroll container
            // (e.g. releasing the mouse above the list when dragging the topmost item).
            // Commits at the last previewed position, same as dropping on the list.
            .can_drop(|dragged, _, _| dragged.downcast_ref::<IRebaseDragValue>().is_some())
            .on_drop(cx.listener(|this, _: &IRebaseDragValue, _, cx| {
                if this.commit_interactive_rebase_drag() {
                    cx.notify();
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.commit_interactive_rebase_drag() {
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("Interactive Rebase"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(format!("onto {base_display}")),
                    ),
            )
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(entry_content)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .when(!entries_empty, |footer| footer.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("Auto Squash"),
                            )
                            .child(
                                components::Button::new("irebase_autosquash", "Auto Squash ▾")
                                    .style(components::ButtonStyle::Outlined)
                                .on_click_with_bounds(
                                    theme,
                                    cx,
                                    move |this, _e, bounds, window, cx| {
                                        let wh = window.window_handle();
                                        let root = this.root_view.clone();
                                        cx.defer(move |cx| {
                                            let _ = wh.update(cx, |_, window, cx| {
                                                let _ = root.update(cx, |root, cx| {
                                                    root.open_popover_for_bounds(
                                                        PopoverKind::InteractiveRebaseAutosquashMenu,
                                                        bounds,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            });
                                        });
                                    },
                                ),
                            ),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .when(!entries_empty, |row| row.child(
                                components::Button::new("irebase_reset", "Reset All")
                                    .style(components::ButtonStyle::Outlined)
                                    .disabled(!is_modified)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            let Some(st) =
                                                this.interactive_rebase_states.get_mut(&repo_id)
                                            else {
                                                return;
                                            };
                                            st.entries = st.original_entries.clone();
                                            st.folded.clear();
                                            st.autosquash_mode = None;
                                            cx.notify();
                                        },
                                    )),
                            ))
                            .child(
                                components::Button::new("irebase_cancel", "Cancel")
                                    .style(components::ButtonStyle::Outlined)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            this.store.dispatch(
                                                Msg::CancelInteractiveRebaseSetup { repo_id },
                                            );
                                            cx.notify();
                                        },
                                    )),
                            )
                            .when(!entries_empty, |row| row.child(
                                components::Button::new("irebase_start", "Start Rebase")
                                    .style(components::ButtonStyle::Filled)
                                    .disabled(
                                        entries_empty
                                            || !matches!(loading_state, Loadable::Ready(_)),
                                    )
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            let Some(st) =
                                                this.interactive_rebase_states.get_mut(&repo_id)
                                            else {
                                                return;
                                            };
                                            if st.entries.is_empty() {
                                                return;
                                            }
                                            let entries = expand_folded(&st.entries, &st.folded);
                                            this.store.dispatch(Msg::InteractiveRebase {
                                                repo_id,
                                                base: base.clone(),
                                                entries,
                                            });
                                            this.store.dispatch(
                                                Msg::CancelInteractiveRebaseSetup { repo_id },
                                            );
                                            cx.notify();
                                        },
                                    )),
                            )),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        action: InteractiveRebaseAction,
        id: &str,
        new_message: Option<&str>,
    ) -> InteractiveRebaseEntry {
        InteractiveRebaseEntry {
            action,
            commit_id: id.to_string(),
            summary: format!("summary {id}"),
            new_message: new_message.map(|s| s.to_string()),
        }
    }

    #[test]
    fn squash_folds_into_preceding_entry() {
        // Data index 0 is the oldest commit; a squash at a higher index folds
        // into the nearest non-drop entry below it.
        let entries = vec![
            entry(InteractiveRebaseAction::Pick, "a", None),
            entry(InteractiveRebaseAction::Squash, "b", None),
        ];
        assert!(entry_is_squash_target(&entries, 0));
        assert!(!entry_is_squash_target(&entries, 1));
    }

    #[test]
    fn stranded_auto_reword_reverts_to_pick() {
        // A Reword with no user message and nothing squashing into it — the
        // state left behind when a squash is reordered away from its target
        // (finding #5) — reverts to Pick.
        let mut entries = vec![
            entry(InteractiveRebaseAction::Pick, "a", None),
            entry(InteractiveRebaseAction::Reword, "b", None),
        ];
        validate_squash_entries(&mut entries);
        assert_eq!(entries[1].action, InteractiveRebaseAction::Pick);
    }

    #[test]
    fn deliberate_reword_is_preserved() {
        // A Reword the user actually typed a message for is never downgraded.
        let mut entries = vec![
            entry(InteractiveRebaseAction::Pick, "a", None),
            entry(InteractiveRebaseAction::Reword, "b", Some("new subject")),
        ];
        validate_squash_entries(&mut entries);
        assert_eq!(entries[1].action, InteractiveRebaseAction::Reword);
    }

    #[test]
    fn reword_kept_while_still_a_squash_target() {
        // The auto-promoted Reword target stays Reword as long as a squash
        // continues to fold into it.
        let mut entries = vec![
            entry(InteractiveRebaseAction::Reword, "a", None),
            entry(InteractiveRebaseAction::Squash, "b", None),
        ];
        validate_squash_entries(&mut entries);
        assert_eq!(entries[0].action, InteractiveRebaseAction::Reword);
        assert_eq!(entries[1].action, InteractiveRebaseAction::Squash);
    }

    #[test]
    fn squash_without_target_reverts_to_pick() {
        // A squash at the bottom (nothing below to fold into) becomes a pick,
        // and the now-stranded reword above it also reverts.
        let mut entries = vec![
            entry(InteractiveRebaseAction::Squash, "a", None),
            entry(InteractiveRebaseAction::Reword, "b", None),
        ];
        validate_squash_entries(&mut entries);
        assert_eq!(entries[0].action, InteractiveRebaseAction::Pick);
        assert_eq!(entries[1].action, InteractiveRebaseAction::Pick);
    }

    // Commit with an explicit summary, for auto-squash grouping tests.
    // Order is oldest-first (index 0 = oldest), matching the entries vector.
    fn sc(id: &str, summary: &str) -> InteractiveRebaseEntry {
        InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Pick,
            commit_id: id.to_string(),
            summary: summary.to_string(),
            new_message: None,
        }
    }

    #[test]
    fn autosquash_to_bottom_folds_into_oldest() {
        // oldest→newest: B "fix", C "wip", D "fix", F "fix"
        let original = vec![
            sc("B", "fix"),
            sc("C", "wip"),
            sc("D", "fix"),
            sc("F", "fix"),
        ];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
        // Only B (oldest "fix") and C survive.
        let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
        assert_eq!(ids, vec!["B", "C"]);
        let into_b = &folded["B"];
        assert_eq!(
            into_b
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["D", "F"]
        );
        assert!(
            into_b
                .iter()
                .all(|e| e.action == InteractiveRebaseAction::Fixup)
        );
    }

    #[test]
    fn autosquash_fixup_prefix_folds_into_target() {
        // `fixup! add feature` groups with `add feature` even though the exact
        // subjects differ; the unprefixed commit survives (so its clean message
        // is kept) even under ToTop, which would otherwise keep the newest.
        let original = vec![
            sc("A", "add feature"),
            sc("B", "unrelated"),
            sc("C", "fixup! add feature"),
        ];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToTop);
        let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B"]);
        assert_eq!(
            folded["A"]
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["C"]
        );
        assert!(
            folded["A"]
                .iter()
                .all(|e| e.action == InteractiveRebaseAction::Fixup)
        );
    }

    #[test]
    fn autosquash_neighbor_folds_adjacent_fixup_prefix() {
        let original = vec![
            sc("A", "add feature"),
            sc("B", "fixup! add feature"),
            sc("C", "unrelated"),
        ];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::Neighbor);
        let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
        assert_eq!(ids, vec!["A", "C"]);
        assert_eq!(
            folded["A"]
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["B"]
        );
    }

    #[test]
    fn autosquash_to_top_folds_into_newest() {
        let original = vec![
            sc("B", "fix"),
            sc("C", "wip"),
            sc("D", "fix"),
            sc("F", "fix"),
        ];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToTop);
        // F (newest "fix") and C survive; F keeps its slot (after C).
        let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
        assert_eq!(ids, vec!["C", "F"]);
        assert_eq!(
            folded["F"]
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "D"]
        );
    }

    #[test]
    fn autosquash_neighbor_only_merges_adjacent() {
        // Two "fix" are adjacent (D,E); a separate "fix" (B) is not.
        let original = vec![
            sc("B", "fix"),
            sc("C", "wip"),
            sc("D", "fix"),
            sc("E", "fix"),
        ];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::Neighbor);
        // B stays (not adjacent to another "fix"); D survives its run, E folds in.
        let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
        assert_eq!(ids, vec!["B", "C", "D"]);
        assert_eq!(
            folded["D"]
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["E"]
        );
        assert!(!folded.contains_key("B"));
    }

    #[test]
    fn autosquash_no_duplicates_yields_empty_fold() {
        let original = vec![sc("A", "one"), sc("B", "two"), sc("C", "three")];
        let (_, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
        assert!(folded.is_empty());
    }

    #[test]
    fn expand_folded_reinserts_fixups_after_survivor() {
        let original = vec![sc("B", "fix"), sc("C", "wip"), sc("D", "fix")];
        let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
        let expanded = expand_folded(&collapsed, &folded);
        let seq: Vec<(&str, InteractiveRebaseAction)> = expanded
            .iter()
            .map(|e| (e.commit_id.as_str(), e.action))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("B", InteractiveRebaseAction::Pick),
                ("D", InteractiveRebaseAction::Fixup),
                ("C", InteractiveRebaseAction::Pick),
            ]
        );
    }

    #[test]
    fn expand_folded_drops_fixups_with_dropped_survivor() {
        let original = vec![sc("B", "fix"), sc("D", "fix")];
        let (mut collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
        // Drop the survivor B; its folded commit D should not be emitted.
        collapsed[0].action = InteractiveRebaseAction::Drop;
        let expanded = expand_folded(&collapsed, &folded);
        assert_eq!(
            expanded
                .iter()
                .map(|e| e.commit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["B"]
        );
    }
}
