use super::super::super::*;
use super::helpers::{IRebaseDragState, IRebaseViewState};
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::{cell::RefCell, ops::Range, rc::Rc};

const ACTION_BTN_W: f32 = 76.0;
// Fallback row height for drag hit-testing and the gap ghost, used only until
// the first paint provides measured row bounds (see measured_drag_row_height).
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

/// Floating preview shown under the cursor while dragging a rebase entry. With
/// the dragged content riding the cursor, the in-list gap can stay empty, so
/// nothing is ever painted over a real row. Minimal styling for now.
struct IRebaseDragPreview {
    theme: AppTheme,
    ui_scale_percent: u32,
    action: InteractiveRebaseAction,
    sha: String,
    summary: String,
    row_h: f32,
}

impl Render for IRebaseDragPreview {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
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

/// Height of one list row (all rows are uniform). Derived from the uniform
/// list's last measured content height (`item_height * item_count`), since the
/// handle stores the viewport size for `item`, not the row height. Falls back
/// to DRAG_ROW_HEIGHT before the first layout populates it.
fn uniform_item_height(scroll: &gpui::UniformListScrollHandle, item_count: usize) -> f32 {
    if item_count == 0 {
        return DRAG_ROW_HEIGHT;
    }
    let h = scroll
        .0
        .borrow()
        .last_item_size
        .map(|s| f32::from(s.contents.height) / item_count as f32)
        .unwrap_or(0.0);
    if h > 0.0 { h } else { DRAG_ROW_HEIGHT }
}

/// Overlay that draws the drop-target insertion line on top of the uniform
/// list while dragging. `pos` is the insertion position in display order
/// (0..=item_count); `None` renders nothing.
struct IRebaseInsertionLine {
    pos: Option<usize>,
    color: gpui::Rgba,
}

impl gpui::UniformListDecoration for IRebaseInsertionLine {
    fn compute(
        &self,
        _visible_range: Range<usize>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _scroll_offset: gpui::Point<gpui::Pixels>,
        item_height: gpui::Pixels,
        item_count: usize,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> gpui::AnyElement {
        let Some(pos) = self.pos else {
            return div().into_any_element();
        };
        let y = item_height * (pos.min(item_count) as f32);
        // Full-size overlay positioned at the list's content origin; the line
        // is absolutely placed at the row boundary and clipped to the viewport.
        div()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top(y)
                    .left_0()
                    .right_0()
                    .h(px(2.0))
                    .bg(self.color),
            )
            .into_any_element()
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

    /// Update the drop target from a drag-move event over the uniform list.
    /// Uniform row height makes this a single division; also drives auto-scroll
    /// near the viewport edges.
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
        let item_h = uniform_item_height(&st.scroll, entry_count);
        if item_h <= 0.0 {
            return;
        }

        let viewport_h = f32::from(e.bounds.size.height);
        let pointer_vp_y = f32::from(e.event.position.y - e.bounds.origin.y);

        // The uniform list scrolls its inner base handle.
        let base = st.scroll.0.borrow().base_handle.clone();
        let mut offset_y = f32::from(base.offset().y);
        let max_down = f32::from(base.max_offset().y);
        if max_down > 0.0 {
            let edge = item_h.min(viewport_h / 4.0);
            let step = item_h / 2.0;
            let scrolled_y = if pointer_vp_y < edge {
                (offset_y + step).min(0.0)
            } else if pointer_vp_y > viewport_h - edge {
                (offset_y - step).max(-max_down)
            } else {
                offset_y
            };
            if scrolled_y != offset_y {
                offset_y = scrolled_y;
                let mut o = base.offset();
                o.y = px(offset_y);
                base.set_offset(o);
                cx.notify();
            }
        }

        // Content-space Y (scroll offset is <= 0 when scrolled down); the
        // insertion position is the nearest row boundary, 0..=entry_count.
        let content_y = pointer_vp_y - offset_y;
        let display_pos = (content_y / item_h)
            .round()
            .clamp(0.0, entry_count as f32) as usize;

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

    /// Render the visible slice of rebase rows for the uniform list. `range`
    /// is in display order (newest-first); data index is `entry_count-1-pos`.
    fn render_irebase_rows(
        &mut self,
        range: Range<usize>,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let selected_commit_id = self
            .active_repo()
            .and_then(|r| r.history_state.selected_commit.as_ref())
            .map(|c| c.0.as_ref().to_owned());
        let Some(st) = self.interactive_rebase_states.get(&repo_id) else {
            return Vec::new();
        };
        let entry_count = st.entries.len();
        let reorder_anim = st.reorder_anim;
        let drag_from_ix = st.drag_state.map(|s| s.from_ix).unwrap_or(usize::MAX);
        let preview_row_h = uniform_item_height(&st.scroll, entry_count);

        let mut out: Vec<gpui::AnyElement> = Vec::with_capacity(range.len());
        for display_pos in range {
            if display_pos >= entry_count {
                break;
            }
            let ix = entry_count - 1 - display_pos;
            // The dragged row is dimmed in place; its content rides the cursor.
            let is_drag_source = ix == drag_from_ix;
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

            // Data for the floating cursor preview built when this row is dragged.
            let pf_action = action;
            let pf_sha = sha.clone();
            let pf_summary = summary.clone();
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
            let row_div = div()
                .id(("irebase_row", ix))
                .w_full()
                .flex()
                .flex_col()
                .px_2()
                .py_0p5()
                .rounded(px(theme.radii.row))
                .when(is_drag_source, |d| d.opacity(0.4))
                .when(!is_drag_source && is_selected, |d| d.bg(theme.colors.active))
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
            out.push(row_element);
        }
        out
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
            // The map entry is populated by `apply_state` on the same state
            // application that made the entries Ready; guard anyway.
            Loadable::Ready(_) if self.interactive_rebase_states.contains_key(&repo_id) => {
                let st = &self.interactive_rebase_states[&repo_id];
                let entry_count = st.entries.len();
                let scroll = st.scroll.clone();
                // Drop-target line position (display order) while dragging.
                let insertion_pos = st.drag_state.map(|s| s.display_pos);

                let list = uniform_list(
                    "irebase_entries",
                    entry_count,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        this.render_irebase_rows(range, repo_id, cx)
                    }),
                )
                .h_full()
                .min_h(px(0.0))
                .track_scroll(&scroll)
                .with_decoration(IRebaseInsertionLine {
                    pos: insertion_pos,
                    color: theme.colors.accent,
                })
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
                }));
                let list = restrict_scroll_to_vertical_axis(list);

                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_h(px(0.0))
                            .pr(components::Scrollbar::visible_gutter(
                                scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            ))
                            .child(list),
                    )
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

        let (autosquash_enabled, is_modified, entries_empty) = self
            .interactive_rebase_states
            .get(&repo_id)
            .map(|st| {
                (
                    st.autosquash,
                    st.entries != st.original_entries,
                    st.entries.is_empty(),
                )
            })
            .unwrap_or((false, false, true));

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
                                                let Some(st) = this
                                                    .interactive_rebase_states
                                                    .get_mut(&repo_id)
                                                else {
                                                    return;
                                                };
                                                st.autosquash = !st.autosquash;
                                                st.entries = st.original_entries.clone();
                                                if st.autosquash {
                                                    apply_autosquash(&mut st.entries);
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
                                            let Some(st) = this
                                                .interactive_rebase_states
                                                .get_mut(&repo_id)
                                            else {
                                                return;
                                            };
                                            st.entries = st.original_entries.clone();
                                            if st.autosquash {
                                                apply_autosquash(&mut st.entries);
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
                                        entries_empty
                                            || !matches!(loading_state, Loadable::Ready(_)),
                                    )
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            let Some(st) = this
                                                .interactive_rebase_states
                                                .get_mut(&repo_id)
                                            else {
                                                return;
                                            };
                                            if st.entries.is_empty() {
                                                return;
                                            }
                                            let entries = std::mem::take(&mut st.entries);
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
