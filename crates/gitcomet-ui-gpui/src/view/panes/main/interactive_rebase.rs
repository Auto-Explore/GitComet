use super::super::super::*;
use super::helpers::IRebaseDragState;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::{cell::RefCell, rc::Rc};

const ACTION_BTN_W: f32 = 76.0;
// Estimated row height used for gap animation. Does not need to be exact —
// the gap grows/shrinks smoothly and any small mismatch is barely noticeable.
const DRAG_ROW_HEIGHT: f32 = 28.0;

fn squash_target(entries: &[InteractiveRebaseEntry], k: usize) -> Option<usize> {
    (0..k)
        .rev()
        .find(|&j| entries[j].action != InteractiveRebaseAction::Drop)
}

pub(super) fn apply_autosquash(entries: &mut Vec<InteractiveRebaseEntry>) {
    let mut i = 0;
    while i < entries.len() {
        let (prefix_action, target_summary) = {
            let s = &entries[i].summary;
            if let Some(t) = s.strip_prefix("fixup! ") {
                (InteractiveRebaseAction::Fixup, t.to_owned())
            } else if let Some(t) = s.strip_prefix("squash! ") {
                (InteractiveRebaseAction::Squash, t.to_owned())
            } else {
                i += 1;
                continue;
            }
        };
        let target_ix =
            (0..i).find(|&j| entries[j].summary.lines().next().unwrap_or("") == target_summary);
        if let Some(t) = target_ix {
            entries[i].action = prefix_action;
            let entry = entries.remove(i);
            // Skip over already-grouped fixup/squash entries so that multiple
            // fixup!/squash! commits targeting the same base don't swap each
            // other back and forth indefinitely.
            let mut insert_at = t + 1;
            while insert_at < i
                && matches!(
                    entries[insert_at].action,
                    InteractiveRebaseAction::Fixup | InteractiveRebaseAction::Squash
                )
            {
                insert_at += 1;
            }
            entries.insert(insert_at, entry);
            // If inserted at or past i, the same slot now holds an unprocessed entry.
            if insert_at >= i {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    validate_squash_entries(entries);
}

fn validate_squash_entries(entries: &mut [InteractiveRebaseEntry]) {
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

impl MainPaneView {
    pub(in crate::view) fn set_rebase_action(
        &mut self,
        ix: usize,
        action: InteractiveRebaseAction,
        cx: &mut gpui::Context<Self>,
    ) {
        if ix >= self.interactive_rebase_entries.len() {
            return;
        }

        // Prevent dropping the last non-dropped commit.
        if action == InteractiveRebaseAction::Drop {
            let current = self.interactive_rebase_entries[ix].action;
            if current != InteractiveRebaseAction::Drop
                && non_drop_count(&self.interactive_rebase_entries) <= 1
            {
                return;
            }
        }

        let old_action = self.interactive_rebase_entries[ix].action;
        // Capture the former squash target before we change the action.
        let former_squash_target = if matches!(
            old_action,
            InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
        ) {
            squash_target(&self.interactive_rebase_entries, ix)
        } else {
            None
        };

        self.interactive_rebase_entries[ix].action = action;

        if action == InteractiveRebaseAction::Squash {
            // Auto-set the new target to Reword so the combined message can be written.
            if let Some(j) = squash_target(&self.interactive_rebase_entries, ix) {
                if self.interactive_rebase_entries[j].action == InteractiveRebaseAction::Pick {
                    self.interactive_rebase_entries[j].action = InteractiveRebaseAction::Reword;
                }
            }
        } else if let Some(j) = former_squash_target {
            // Was Squash/Fixup, now it isn't. If the former target is Reword and nothing
            // else is squashing into it, revert it back to Pick.
            if self.interactive_rebase_entries[j].action == InteractiveRebaseAction::Reword {
                let still_targeted = (0..self.interactive_rebase_entries.len()).any(|k| {
                    matches!(
                        self.interactive_rebase_entries[k].action,
                        InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
                    ) && squash_target(&self.interactive_rebase_entries, k) == Some(j)
                });
                if !still_targeted && self.interactive_rebase_entries[j].new_message.is_none() {
                    self.interactive_rebase_entries[j].action = InteractiveRebaseAction::Pick;
                }
            }
        }

        if action == InteractiveRebaseAction::Drop {
            validate_squash_entries(&mut self.interactive_rebase_entries);
        }

        cx.notify();
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
            Loadable::Ready(_) => {
                let entry_count = self.interactive_rebase_entries.len();
                let selected_commit_id = self
                    .active_repo()
                    .and_then(|r| r.history_state.selected_commit.as_ref())
                    .map(|c| c.0.as_ref().to_owned());

                let reorder_anim = self.interactive_rebase_reorder_anim;
                let drag_state = self.interactive_rebase_drag_state;
                let is_dragging = drag_state.is_some();
                let drag_from_ix = drag_state.map(|s| s.from_ix).unwrap_or(usize::MAX);
                let drag_to_ix = drag_state.map(|s| s.to_ix).unwrap_or(0);

                // Display order is always newest-first (reversed). During drag we keep items in
                // their original slots — a collapsing source placeholder and an animated gap at
                // the target slot provide the reorder feedback instead.
                let display_order: Vec<usize> = (0..entry_count).rev().collect();

                // Display positions for the source placeholder and the animated gap target.
                let from_display_pos = (is_dragging && drag_from_ix < entry_count)
                    .then(|| (entry_count - 1).saturating_sub(drag_from_ix));
                let gap_display_pos = (is_dragging && drag_to_ix < entry_count)
                    .then(|| (entry_count - 1).saturating_sub(drag_to_ix));

                // Pre-extract the dragged item's display data so the gap can render it on rails.
                let ghost_data = from_display_pos.map(|_| {
                    let fix = drag_from_ix;
                    let g_action = self.interactive_rebase_entries[fix].action;
                    let g_sha = self.interactive_rebase_entries[fix]
                        .commit_id
                        .get(..8)
                        .unwrap_or(&self.interactive_rebase_entries[fix].commit_id)
                        .to_string();
                    let g_summary = self.interactive_rebase_entries[fix]
                        .new_message
                        .as_deref()
                        .and_then(|m| m.lines().next())
                        .unwrap_or(&self.interactive_rebase_entries[fix].summary)
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
                            .h(px(DRAG_ROW_HEIGHT))
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

                // When dragging downward (source above gap) and the gap would land at the
                // very last slot, the insert(to_ix=0, entry) pushes every other item UP,
                // meaning the dragged item ends up BELOW the current last item — not above it.
                // Inserting the gap before that last item would look wrong; append it after.
                let append_gap_after = gap_display_pos == Some(entry_count - 1)
                    && from_display_pos.map_or(false, |fdp| fdp < entry_count - 1);

                let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(entry_count + 1);

                for (display_pos, &ix) in display_order.iter().enumerate() {
                    // Insert an animated slot at the target position. It renders the dragged
                    // item's content so the ghost appears "on rails" within the list.
                    if gap_display_pos == Some(display_pos) && !append_gap_after {
                        let ghost_row =
                            if let Some((g_action, ref g_sha, ref g_summary)) = ghost_data {
                                build_ghost_row(g_action, g_sha, g_summary)
                            } else {
                                div().into_any_element()
                            };
                        rows.push(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .child(ghost_row)
                                .with_animation(
                                    "irebase_gap",
                                    Animation::new(Duration::from_millis(120))
                                        .with_easing(gpui::ease_out_quint()),
                                    |d, delta| d.h(px(DRAG_ROW_HEIGHT * delta)).opacity(delta),
                                )
                                .into_any_element(),
                        );
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

                    let action = self.interactive_rebase_entries[ix].action;
                    let sha = self.interactive_rebase_entries[ix]
                        .commit_id
                        .get(..8)
                        .unwrap_or(&self.interactive_rebase_entries[ix].commit_id)
                        .to_string();
                    let summary = self.interactive_rebase_entries[ix]
                        .new_message
                        .as_deref()
                        .and_then(|m| m.lines().next())
                        .unwrap_or(&self.interactive_rebase_entries[ix].summary)
                        .to_owned();
                    let is_selected = selected_commit_id
                        .as_deref()
                        .is_some_and(|s| s == self.interactive_rebase_entries[ix].commit_id);
                    let is_squash_like = matches!(
                        action,
                        InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
                    );

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
                            let nd = non_drop_count(&this.interactive_rebase_entries);
                            let current_action =
                                this.interactive_rebase_entries.get(ix).map(|e| e.action);
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
                            let len = this.interactive_rebase_entries.len();
                            let entry_display_pos = len - 1 - ix;
                            if entry_display_pos > 0 {
                                let swap_ix = len - 1 - (entry_display_pos - 1);
                                this.interactive_rebase_entries.swap(ix, swap_ix);
                                validate_squash_entries(&mut this.interactive_rebase_entries);
                                let ver = this
                                    .interactive_rebase_reorder_anim
                                    .map(|(_, _, v)| v + 1)
                                    .unwrap_or(0);
                                this.interactive_rebase_reorder_anim = Some((ix, swap_ix, ver));
                            }
                            cx.notify();
                        }));

                    let down_btn = components::Button::new(format!("down_{ix}"), "▼")
                        .style(components::ButtonStyle::Subtle)
                        .no_focus()
                        .disabled(display_pos + 1 >= entry_count)
                        .render(theme, ui_scale_percent)
                        .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                            let len = this.interactive_rebase_entries.len();
                            let entry_display_pos = len - 1 - ix;
                            if entry_display_pos + 1 < len {
                                let swap_ix = len - 1 - (entry_display_pos + 1);
                                this.interactive_rebase_entries.swap(ix, swap_ix);
                                validate_squash_entries(&mut this.interactive_rebase_entries);
                                let ver = this
                                    .interactive_rebase_reorder_anim
                                    .map(|(_, _, v)| v + 1)
                                    .unwrap_or(0);
                                this.interactive_rebase_reorder_anim = Some((ix, swap_ix, ver));
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

                    let commit_id_val =
                        CommitId(self.interactive_rebase_entries[ix].commit_id.clone().into());
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
                                        .overflow_x_hidden()
                                        .whitespace_nowrap()
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
                                let nd = non_drop_count(&this.interactive_rebase_entries);
                                let current_action =
                                    this.interactive_rebase_entries.get(ix).map(|e| e.action);
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

                // When dragging a higher item (lower data index) all the way to the bottom,
                // the gap belongs AFTER the last rendered item, not before it.
                if append_gap_after {
                    let ghost_row = if let Some((g_action, ref g_sha, ref g_summary)) = ghost_data {
                        build_ghost_row(g_action, g_sha, g_summary)
                    } else {
                        div().into_any_element()
                    };
                    rows.push(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .child(ghost_row)
                            .with_animation(
                                "irebase_gap",
                                Animation::new(Duration::from_millis(120))
                                    .with_easing(gpui::ease_out_quint()),
                                |d, delta| d.h(px(DRAG_ROW_HEIGHT * delta)).opacity(delta),
                            )
                            .into_any_element(),
                    );
                }

                div()
                    .id("irebase_entries_scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scroll()
                    .on_drag_move(cx.listener(
                        |this, e: &gpui::DragMoveEvent<IRebaseDragValue>, _w, cx| {
                            let from_ix = e.drag(cx).ix;
                            let entry_count = this.interactive_rebase_entries.len();
                            if entry_count == 0 {
                                return;
                            }
                            let drag_y = e.event.position.y - e.bounds.origin.y;
                            // Count midpoints crossed to find the target display position.
                            // Midpoint between display slot i and i+1 is at (i+0.5)*row_h.
                            let display_pos = (0..entry_count.saturating_sub(1))
                                .filter(|&i| drag_y > px((i as f32 + 0.5) * DRAG_ROW_HEIGHT))
                                .count()
                                .min(entry_count - 1);
                            let to_ix = (entry_count - 1).saturating_sub(display_pos);
                            let already_matches = this
                                .interactive_rebase_drag_state
                                .map_or(false, |s| s.from_ix == from_ix && s.to_ix == to_ix);
                            if !already_matches {
                                this.interactive_rebase_drag_state =
                                    Some(IRebaseDragState { from_ix, to_ix });
                                cx.notify();
                            }
                        },
                    ))
                    .can_drop(move |dragged, _window, _cx| {
                        dragged.downcast_ref::<IRebaseDragValue>().is_some()
                    })
                    .on_drop(cx.listener(move |this, drag: &IRebaseDragValue, _w, cx| {
                        let from = drag.ix;
                        let to = this
                            .interactive_rebase_drag_state
                            .map(|s| s.to_ix)
                            .unwrap_or(from);
                        this.interactive_rebase_drag_state = None;
                        if from != to
                            && from < this.interactive_rebase_entries.len()
                            && to < this.interactive_rebase_entries.len()
                        {
                            let entry = this.interactive_rebase_entries.remove(from);
                            this.interactive_rebase_entries.insert(to, entry);
                            validate_squash_entries(&mut this.interactive_rebase_entries);
                        }
                        cx.notify();
                    }))
                    .children(rows)
                    .into_any_element()
            }
        };

        let autosquash_enabled = self.interactive_rebase_autosquash;
        let is_modified =
            self.interactive_rebase_entries != self.interactive_rebase_original_entries;

        div()
            .flex()
            .flex_col()
            .size_full()
            // Safety net: clear drag state for drops that land outside the scroll container
            // (e.g. releasing the mouse above the list when dragging the topmost item).
            .can_drop(|dragged, _, _| dragged.downcast_ref::<IRebaseDragValue>().is_some())
            .on_drop(cx.listener(|this, _: &IRebaseDragValue, _, cx| {
                this.interactive_rebase_drag_state = None;
                cx.notify();
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if let Some(state) = this.interactive_rebase_drag_state.take() {
                        if state.from_ix != state.to_ix
                            && state.from_ix < this.interactive_rebase_entries.len()
                            && state.to_ix < this.interactive_rebase_entries.len()
                        {
                            let entry = this.interactive_rebase_entries.remove(state.from_ix);
                            this.interactive_rebase_entries.insert(state.to_ix, entry);
                            validate_squash_entries(&mut this.interactive_rebase_entries);
                        }
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
                            .child(format!("onto {base_short}")),
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("Autosquash"),
                            )
                            .child(
                                div()
                                    .cursor(gpui::CursorStyle::PointingHand)
                                    .text_sm()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(
                                            move |this, _e: &gpui::MouseDownEvent, _w, cx| {
                                                this.interactive_rebase_autosquash =
                                                    !this.interactive_rebase_autosquash;
                                                this.interactive_rebase_entries = this
                                                    .interactive_rebase_original_entries
                                                    .clone();
                                                if this.interactive_rebase_autosquash {
                                                    apply_autosquash(
                                                        &mut this.interactive_rebase_entries,
                                                    );
                                                }
                                                cx.notify();
                                            },
                                        ),
                                    )
                                    .child(if autosquash_enabled { "☑" } else { "☐" }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                components::Button::new("irebase_reset", "Reset All")
                                    .style(components::ButtonStyle::Outlined)
                                    .disabled(!is_modified)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            this.interactive_rebase_entries =
                                                this.interactive_rebase_original_entries.clone();
                                            if this.interactive_rebase_autosquash {
                                                apply_autosquash(
                                                    &mut this.interactive_rebase_entries,
                                                );
                                            }
                                            cx.notify();
                                        },
                                    )),
                            )
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
                            .child(
                                components::Button::new("irebase_start", "Start Rebase")
                                    .style(components::ButtonStyle::Filled)
                                    .disabled(
                                        self.interactive_rebase_entries.is_empty()
                                            || !matches!(loading_state, Loadable::Ready(_)),
                                    )
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            if this.interactive_rebase_entries.is_empty() {
                                                return;
                                            }
                                            let entries = std::mem::take(
                                                &mut this.interactive_rebase_entries,
                                            );
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
                            ),
                    ),
            )
    }
}
