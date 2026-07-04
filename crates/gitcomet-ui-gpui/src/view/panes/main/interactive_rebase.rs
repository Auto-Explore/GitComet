use super::super::super::*;
use super::helpers::{IRebaseDragState, IRebaseViewState};
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::{cell::RefCell, rc::Rc};

const ACTION_BTN_W: f32 = 76.0;
// Fallback row height for drag hit-testing and the gap ghost, used only until
// the first paint provides measured row bounds (see measured_drag_row_height).
const DRAG_ROW_HEIGHT: f32 = 28.0;

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

/// The commit ids folded into each survivor for a given auto-squash `mode`.
/// Groups commits by identical summary; the surviving commit per group is
/// chosen by the mode, and the others fold (fixup) into it. Empty summaries are
/// never eligible. `entries` are ordered oldest-first (index 0 = oldest).
///
/// Returns `folded_into[i] = Some(survivor_index)` for every commit that is
/// folded away, and `None` for survivors and untouched commits.
fn autosquash_folds(entries: &[InteractiveRebaseEntry], mode: AutosquashMode) -> Vec<Option<usize>> {
    let n = entries.len();
    let mut folded_into: Vec<Option<usize>> = vec![None; n];
    match mode {
        AutosquashMode::ToTop | AutosquashMode::ToBottom => {
            // Group indices by summary, preserving first-seen order for stable output.
            let mut order: Vec<&str> = Vec::new();
            let mut groups: std::collections::HashMap<&str, Vec<usize>> =
                std::collections::HashMap::new();
            for (i, e) in entries.iter().enumerate() {
                if e.summary.trim().is_empty() {
                    continue;
                }
                groups
                    .entry(e.summary.as_str())
                    .or_insert_with(|| {
                        order.push(e.summary.as_str());
                        Vec::new()
                    })
                    .push(i);
            }
            for key in order {
                let indices = &groups[key];
                if indices.len() < 2 {
                    continue;
                }
                // Highest index = newest commit; lowest = oldest.
                let survivor = match mode {
                    AutosquashMode::ToTop => *indices.iter().max().unwrap(),
                    _ => *indices.iter().min().unwrap(),
                };
                for &i in indices {
                    if i != survivor {
                        folded_into[i] = Some(survivor);
                    }
                }
            }
        }
        AutosquashMode::Neighbor => {
            // Collapse each maximal run of adjacent, equal-summary commits into
            // the run's oldest (lowest-index) member.
            let mut i = 0;
            while i < n {
                if entries[i].summary.trim().is_empty() {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < n && entries[j].summary == entries[i].summary {
                    j += 1;
                }
                for k in (i + 1)..j {
                    folded_into[k] = Some(i);
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
        if e.action != InteractiveRebaseAction::Drop {
            if let Some(fixups) = folded.get(&e.commit_id) {
                out.extend(fixups.iter().cloned());
            }
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

/// Height of a real entry row measured from the last paint, so drag
/// hit-testing and the gap ghost track font size and UI scale. Falls back
/// to DRAG_ROW_HEIGHT before the first paint. The gap ghost and the
/// collapsed source row are always shorter than a real row, so the max
/// child height is a real row's height.
fn measured_drag_row_height(scroll: &gpui::ScrollHandle) -> f32 {
    let mut max_h = 0f32;
    let mut i = 0;
    while let Some(b) = scroll.bounds_for_item(i) {
        max_h = max_h.max(f32::from(b.size.height));
        i += 1;
    }
    if max_h > 0.0 { max_h } else { DRAG_ROW_HEIGHT }
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
            if let Some(j) = squash_target(&st.entries, ix) {
                if st.entries[j].action == InteractiveRebaseAction::Pick {
                    st.entries[j].action = InteractiveRebaseAction::Reword;
                }
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
        }
        true
    }

    pub(in crate::view) fn interactive_rebase_view(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;

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
                let st = &self.interactive_rebase_states[&repo_id];
                let entry_count = st.entries.len();
                let drag_row_h = measured_drag_row_height(&st.scroll);
                let selected_commit_id = self
                    .active_repo()
                    .and_then(|r| r.history_state.selected_commit.as_ref())
                    .map(|c| c.0.as_ref().to_owned());

                // While auto-squash is off, flag rows whose summary is shared by
                // another commit — the candidates a mode would fold together.
                let autosquash_active = st.autosquash_mode.is_some();
                let eligible_summaries: std::collections::HashSet<&str> = if autosquash_active {
                    std::collections::HashSet::new()
                } else {
                    let mut counts: std::collections::HashMap<&str, u32> =
                        std::collections::HashMap::new();
                    for e in &st.entries {
                        if !e.summary.trim().is_empty() {
                            *counts.entry(e.summary.as_str()).or_default() += 1;
                        }
                    }
                    counts
                        .into_iter()
                        .filter(|(_, c)| *c > 1)
                        .map(|(s, _)| s)
                        .collect()
                };

                let reorder_anim = st.reorder_anim;
                let drag_state = st.drag_state;
                let is_dragging = drag_state.is_some();
                let drag_from_ix = drag_state.map(|s| s.from_ix).unwrap_or(usize::MAX);
                let drag_display_pos = drag_state.map(|s| s.display_pos).unwrap_or(0);

                // Display order is always newest-first (reversed). During drag we keep items in
                // their original slots — a collapsing source placeholder and an animated gap at
                // the target slot provide the reorder feedback instead.
                let display_order: Vec<usize> = (0..entry_count).rev().collect();

                // Display positions for the source placeholder and the animated gap target.
                let from_display_pos = (is_dragging && drag_from_ix < entry_count)
                    .then(|| (entry_count - 1).saturating_sub(drag_from_ix));
                let gap_display_pos = is_dragging.then_some(drag_display_pos);

                // Pre-extract the dragged item's display data so the gap can render it on rails.
                let ghost_data = from_display_pos.map(|_| {
                    let fix = drag_from_ix;
                    let g_action = st.entries[fix].action;
                    let g_sha = st.entries[fix]
                        .commit_id
                        .get(..8)
                        .unwrap_or(&st.entries[fix].commit_id)
                        .to_string();
                    let g_summary = st.entries[fix]
                        .new_message
                        .as_deref()
                        .and_then(|m| m.lines().next())
                        .unwrap_or(&st.entries[fix].summary)
                        .to_owned();
                    (g_action, g_sha, g_summary)
                });

                // Builds the ghost row that appears in the animated gap — styled to match
                // the real rows: gripper → (squash arrow) → static action button → sha → summary.
                let build_ghost_row =
                    |g_action: InteractiveRebaseAction, g_sha: &str, g_summary: &str| {
                        let action_btn_w = px(ACTION_BTN_W * ui_scale_percent as f32 / 100.0);
                        let is_squash_like = matches!(
                            g_action,
                            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
                        );
                        let outlined_border = with_alpha(
                            theme.colors.text_muted,
                            if theme.is_dark { 0.38 } else { 0.28 },
                        );
                        div()
                            .h(px(drag_row_h))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_0p5()
                            .rounded(px(theme.radii.row))
                            .bg(with_alpha(theme.colors.accent, 0.12))
                            .border_1()
                            .border_color(with_alpha(theme.colors.accent, 0.4))
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
                                    .child(format!("{} ▾", action_short_label(g_action))),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .font_family("monospace")
                                    .child(g_sha.to_owned()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(theme.colors.text)
                                    .overflow_x_hidden()
                                    .whitespace_nowrap()
                                    .child(g_summary.to_owned()),
                            )
                            .into_any_element()
                    };

                // When dragging a higher item all the way to the bottom the drag
                // slot falls past the last display position. In that case render
                // the gap after all rows.
                let append_gap_after = gap_display_pos == Some(entry_count);

                // Gap moves animate as a matched pair: a spacer shrinking where the
                // gap left and the gap slot growing where it landed. Identical duration
                // and easing keep the two heights summing to exactly one row, so
                // rows below both slots stay put and rows in between slide smoothly.
                // At drag start there is no previous slot: the ghost renders at full
                // height, replacing the collapsed source row in place with no shift.
                let gap_prev_display_pos = drag_state.and_then(|s| s.prev_display_pos);
                let gap_anim_ver = drag_state.map(|s| s.anim_ver).unwrap_or(0);
                let animate_gap_move = gap_prev_display_pos.is_some();
                // Only the slot height animates; the ghost row itself stays at full
                // height, pinned to the destination slot. Anchoring it to the growing
                // slot's bottom when the gap moved down (top when it moved up) keeps
                // its absolute position constant throughout the animation, so the
                // dragged row is never clipped away mid-move (which read as a flicker).
                // `deferred` paints it above the neighbor row sliding out from under it.
                let gap_moved_down =
                    gap_prev_display_pos.is_some_and(|prev| prev < drag_display_pos);
                let wrap_gap = move |ghost_row: gpui::AnyElement| -> gpui::AnyElement {
                    if animate_gap_move {
                        div()
                            .w_full()
                            .relative()
                            .child(gpui::deferred(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .h(px(drag_row_h))
                                    .when(gap_moved_down, |d| d.bottom_0())
                                    .when(!gap_moved_down, |d| d.top_0())
                                    .child(ghost_row),
                            ))
                            .with_animation(
                                format!("irebase_gap_in_{gap_anim_ver}"),
                                Animation::new(Duration::from_millis(120))
                                    .with_easing(gpui::ease_out_quint()),
                                move |d, delta| d.h(px(drag_row_h * delta)),
                            )
                            .into_any_element()
                    } else {
                        ghost_row
                    }
                };
                let build_gap_out_spacer = move || -> gpui::AnyElement {
                    div()
                        .w_full()
                        .with_animation(
                            format!("irebase_gap_out_{gap_anim_ver}"),
                            Animation::new(Duration::from_millis(120))
                                .with_easing(gpui::ease_out_quint()),
                            move |d, delta| d.h(px(drag_row_h * (1.0 - delta))),
                        )
                        .into_any_element()
                };

                let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(entry_count + 2);

                for (display_pos, &ix) in display_order.iter().enumerate() {
                    // The shrinking half of the gap-move animation.
                    if gap_prev_display_pos == Some(display_pos) {
                        rows.push(build_gap_out_spacer());
                    }

                    // Insert an animated slot at the target position. It renders the dragged
                    // item's content so the ghost appears "on rails" within the list.
                    if gap_display_pos == Some(display_pos) && !append_gap_after {
                        let ghost_row =
                            if let Some((g_action, ref g_sha, ref g_summary)) = ghost_data {
                                build_ghost_row(g_action, g_sha, g_summary)
                            } else {
                                div().into_any_element()
                            };
                        rows.push(wrap_gap(ghost_row));
                    }

                    // Collapse the source item — the ghost view follows the cursor instead.
                    if from_display_pos == Some(display_pos) {
                        rows.push(
                            div()
                                .id(("irebase_row", ix))
                                .h(px(0.0))
                                .overflow_hidden()
                                .into_any_element(),
                        );
                        continue;
                    }

                    let is_drag_source = false;
                    let is_bottom = display_pos + 1 >= entry_count;

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
                    // Dropped entries are dimmed and struck through so it is
                    // clear they will be discarded when the rebase runs.
                    let is_dropped = action == InteractiveRebaseAction::Drop;
                    // A candidate for auto-squash (only surfaced while it is off).
                    let is_autosquash_eligible =
                        eligible_summaries.contains(st.entries[ix].summary.as_str());
                    // Commits already folded into this survivor by auto-squash;
                    // their short shas are listed beneath the row.
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
                            let can_drop =
                                current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
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

                    let gripper = div()
                        .id(("gripper", ix))
                        .cursor(gpui::CursorStyle::PointingHand)
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("⠿")
                        .on_drag(drag_val, move |_drag, _offset, _window, cx| {
                            cx.new(|_cx| gpui::Empty)
                        });

                    let commit_id_val = CommitId(st.entries[ix].commit_id.clone().into());
                    let row_div = div()
                        .id(("irebase_row", ix))
                        .flex()
                        .flex_col()
                        .px_2()
                        .py_0p5()
                        .rounded(px(theme.radii.row))
                        .when(is_drag_source, |d| {
                            d.bg(with_alpha(theme.colors.accent, 0.15))
                                .border_1()
                                .border_color(with_alpha(theme.colors.accent, 0.5))
                                .opacity(0.85)
                        })
                        .when(!is_drag_source && is_selected, |d| {
                            d.bg(theme.colors.active)
                        })
                        .when(!is_drag_source && !is_selected, |d| {
                            d.hover(move |s| s.bg(theme.colors.hover))
                        })
                        .when(is_dropped, |d| d.opacity(0.5))
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
                                let can_drop =
                                    current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
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

                    let row_element = if let Some((aix, bix, ver)) = reorder_anim {
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
                    };
                    rows.push(row_element);
                }

                // The gap previously sat after the last row and has since moved up.
                if gap_prev_display_pos == Some(entry_count) {
                    rows.push(build_gap_out_spacer());
                }

                // When dragging a higher item (lower data index) all the way to the bottom,
                // the gap belongs AFTER the last rendered item, not before it.
                if append_gap_after {
                    let ghost_row = if let Some((g_action, ref g_sha, ref g_summary)) = ghost_data {
                        build_ghost_row(g_action, g_sha, g_summary)
                    } else {
                        div().into_any_element()
                    };
                    rows.push(wrap_gap(ghost_row));
                }

                let scrollbar_gutter = components::Scrollbar::visible_gutter(
                    st.scroll.clone(),
                    components::ScrollbarAxis::Vertical,
                );
                let scroll_list = div()
                    .id("irebase_entries_scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .pr(scrollbar_gutter)
                    .track_scroll(&st.scroll)
                    .on_drag_move(cx.listener(
                        move |this, e: &gpui::DragMoveEvent<IRebaseDragValue>, _w, cx| {
                            let from_ix = e.drag(cx).ix;
                            let Some(st) = this.interactive_rebase_states.get_mut(&repo_id) else {
                                return;
                            };
                            let entry_count = st.entries.len();
                            if entry_count == 0 {
                                return;
                            }
                            let row_h = measured_drag_row_height(&st.scroll);

                            // Auto-scroll while the pointer is near the viewport
                            // edges so items beyond the visible list are reachable.
                            let viewport_h = f32::from(e.bounds.size.height);
                            let pointer_vp_y = f32::from(e.event.position.y - e.bounds.origin.y);
                            let mut offset_y = f32::from(st.scroll.offset().y);
                            let max_down = f32::from(st.scroll.max_offset().y);
                            if max_down > 0.0 {
                                let edge = row_h.min(viewport_h / 4.0);
                                let step = row_h / 2.0;
                                let scrolled_y = if pointer_vp_y < edge {
                                    (offset_y + step).min(0.0)
                                } else if pointer_vp_y > viewport_h - edge {
                                    (offset_y - step).max(-max_down)
                                } else {
                                    offset_y
                                };
                                if scrolled_y != offset_y {
                                    offset_y = scrolled_y;
                                    let mut o = st.scroll.offset();
                                    o.y = px(offset_y);
                                    st.scroll.set_offset(o);
                                    cx.notify();
                                }
                            }

                            // Pointer Y in content space; the scroll offset is <= 0
                            // when scrolled down.
                            let drag_y = e.event.position.y - e.bounds.origin.y - px(offset_y);

                            let source_dp = (entry_count - 1).saturating_sub(from_ix);
                            let current_state = st.drag_state;
                            let gap_dp = current_state.map_or(source_dp, |s| s.display_pos);
                            let append_gap =
                                gap_dp == entry_count && source_dp < entry_count.saturating_sub(1);

                            // Simulate the rendering layout to get visual Y start
                            // of each non-source display slot. Gap inserted before
                            // its slot (if not past the end) or after all (if at end).
                            let mut slot_ys = vec![0f32; entry_count];
                            let mut y = 0f32;
                            let mut y_at_source = 0f32;
                            for (dp, slot_y) in slot_ys.iter_mut().enumerate() {
                                if dp == gap_dp && !append_gap {
                                    y += row_h;
                                }
                                if dp == source_dp {
                                    y_at_source = y;
                                    continue;
                                }
                                *slot_y = y;
                                y += row_h;
                            }

                            // Count row midpoints the pointer has crossed to find
                            // the gap's display position; entry_count means the gap
                            // goes after the last row.
                            let display_pos = (0..entry_count)
                                .filter(|&i| {
                                    let mid = if i == source_dp {
                                        y_at_source
                                    } else if i == entry_count.saturating_sub(1) {
                                        slot_ys[i] + row_h
                                    } else {
                                        slot_ys[i] + row_h / 2.0
                                    };
                                    drag_y > px(mid)
                                })
                                .count();

                            // Map the gap's display position to the data index the
                            // dragged entry will land on. When the gap sits below
                            // the source, removing the source shifts the rows in
                            // between up by one, hence the second branch.
                            let to_ix = if display_pos <= source_dp {
                                entry_count - 1 - display_pos
                            } else {
                                entry_count - display_pos
                            };
                            let already_matches = current_state.is_some_and(|s| {
                                s.from_ix == from_ix && s.display_pos == display_pos
                            });
                            if !already_matches {
                                let (prev_display_pos, anim_ver) = match current_state {
                                    Some(s) if s.display_pos != display_pos => {
                                        (Some(s.display_pos), s.anim_ver.wrapping_add(1))
                                    }
                                    Some(s) => (s.prev_display_pos, s.anim_ver),
                                    // A drag whose first event already lands away from
                                    // the source slot still animates out of it.
                                    None => ((display_pos != source_dp).then_some(source_dp), 0),
                                };
                                st.drag_state = Some(IRebaseDragState {
                                    from_ix,
                                    to_ix,
                                    display_pos,
                                    prev_display_pos,
                                    anim_ver,
                                });
                                cx.notify();
                            }
                        },
                    ))
                    .can_drop(move |dragged, _window, _cx| {
                        dragged.downcast_ref::<IRebaseDragValue>().is_some()
                    })
                    .on_drop(cx.listener(move |this, _drag: &IRebaseDragValue, _w, cx| {
                        this.commit_interactive_rebase_drag();
                        cx.notify();
                    }))
                    .children(rows);

                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(scroll_list)
                    .child(
                        components::Scrollbar::new("irebase_scrollbar", st.scroll.clone())
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
            into_b.iter().map(|e| e.commit_id.as_str()).collect::<Vec<_>>(),
            vec!["D", "F"]
        );
        assert!(into_b.iter().all(|e| e.action == InteractiveRebaseAction::Fixup));
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
            folded["F"].iter().map(|e| e.commit_id.as_str()).collect::<Vec<_>>(),
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
            folded["D"].iter().map(|e| e.commit_id.as_str()).collect::<Vec<_>>(),
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
        let seq: Vec<(&str, InteractiveRebaseAction)> =
            expanded.iter().map(|e| (e.commit_id.as_str(), e.action)).collect();
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
            expanded.iter().map(|e| e.commit_id.as_str()).collect::<Vec<_>>(),
            vec!["B"]
        );
    }
}
