use super::super::super::*;
use super::helpers::IRebaseDragState;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::{cell::RefCell, rc::Rc};

const ACTION_BTN_W: f32 = 76.0;
// Estimated row height used for gap animation. Does not need to be exact —
// the gap grows/shrinks smoothly and any small mismatch is barely noticeable.
const DRAG_ROW_HEIGHT: f32 = 28.0;

fn squash_target(entries: &[InteractiveRebaseEntry], k: usize) -> Option<usize> {
    (0..k).rev().find(|&j| entries[j].action != InteractiveRebaseAction::Drop)
}

fn validate_squash_entries(entries: &mut [InteractiveRebaseEntry]) {
    for k in 0..entries.len() {
        if !matches!(entries[k].action, InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup) {
            continue;
        }
        let has_target = (0..k).rev().any(|j| entries[j].action != InteractiveRebaseAction::Drop);
        if !has_target {
            entries[k].action = InteractiveRebaseAction::Pick;
        }
    }
}

fn non_drop_count(entries: &[InteractiveRebaseEntry]) -> usize {
    entries.iter().filter(|e| e.action != InteractiveRebaseAction::Drop).count()
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
            if current != InteractiveRebaseAction::Drop && non_drop_count(&self.interactive_rebase_entries) <= 1 {
                return;
            }
        }

        let old_action = self.interactive_rebase_entries[ix].action;
        // Capture the former squash target before we change the action.
        let former_squash_target = if matches!(old_action, InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup) {
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
                    matches!(self.interactive_rebase_entries[k].action, InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup)
                        && squash_target(&self.interactive_rebase_entries, k) == Some(j)
                });
                if !still_targeted {
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
        let base_short: SharedString = if base.len() > 16 && base.chars().all(|c| c.is_ascii_hexdigit()) {
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

                let drag_state = self.interactive_rebase_drag_state.map(|s| (s.from_ix, s.to_ix));
                let is_dragging = drag_state.is_some();
                let (drag_from_ix, drag_to_ix) = drag_state.unwrap_or((usize::MAX, 0));

                // Build display order: normally reversed (newest first). While dragging,
                // pull the drag source out of its natural slot and insert it at the
                // target slot so it visually follows the cursor.
                let display_order: Vec<usize> = if is_dragging && drag_from_ix < entry_count {
                    let from_display = (entry_count - 1).saturating_sub(drag_from_ix);
                    let target_display = (entry_count - 1).saturating_sub(drag_to_ix);
                    let mut order: Vec<usize> = (0..entry_count).rev().collect();
                    order.remove(from_display);
                    order.insert(target_display.min(order.len()), drag_from_ix);
                    order
                } else {
                    (0..entry_count).rev().collect()
                };

                let current_non_drop = non_drop_count(&self.interactive_rebase_entries);

                let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(entry_count);

                for (display_pos, &ix) in display_order.iter().enumerate() {
                    let is_drag_source = is_dragging && ix == drag_from_ix;
                    let is_bottom = display_pos + 1 >= entry_count;

                    let action = self.interactive_rebase_entries[ix].action;
                    let sha = self.interactive_rebase_entries[ix]
                        .commit_id
                        .get(..8)
                        .unwrap_or(&self.interactive_rebase_entries[ix].commit_id)
                        .to_string();
                    let summary = self.interactive_rebase_entries[ix].summary.clone();
                    let is_selected = selected_commit_id
                        .as_deref()
                        .is_some_and(|s| s == self.interactive_rebase_entries[ix].commit_id);
                    let is_squash_like = matches!(action, InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup);

                    // can_drop: allowed if this commit is already dropped, or there are >1 non-drop commits
                    let can_drop = action == InteractiveRebaseAction::Drop || current_non_drop > 1;

                    let btn_bounds: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>> =
                        Rc::new(RefCell::new(None));
                    let btn_bounds_prepaint = Rc::clone(&btn_bounds);
                    let action_btn_w = px(ACTION_BTN_W * ui_scale_percent as f32 / 100.0);
                    let action_label = format!("{} ▾", action_short_label(action));

                    let inner_btn = components::Button::new(
                        format!("action_{ix}"),
                        action_label,
                    )
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
                        let current_action = this.interactive_rebase_entries.get(ix).map(|e| e.action);
                        let can_drop = current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
                        let wh = window.window_handle();
                        let root = this.root_view.clone();
                        cx.defer(move |cx| {
                            let _ = wh.update(cx, |_, window, cx| {
                                let _ = root.update(cx, |root, cx| {
                                    root.open_popover_for_bounds(
                                        PopoverKind::InteractiveRebaseActionMenu { ix, is_bottom, can_drop },
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
                        .disabled(display_pos == 0)
                        .render(theme, ui_scale_percent)
                        .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                            let len = this.interactive_rebase_entries.len();
                            let entry_display_pos = len - 1 - ix;
                            if entry_display_pos > 0 {
                                let swap_ix = len - 1 - (entry_display_pos - 1);
                                this.interactive_rebase_entries.swap(ix, swap_ix);
                                validate_squash_entries(&mut this.interactive_rebase_entries);
                            }
                            cx.notify();
                        }));

                    let down_btn = components::Button::new(format!("down_{ix}"), "▼")
                        .style(components::ButtonStyle::Subtle)
                        .disabled(display_pos + 1 >= entry_count)
                        .render(theme, ui_scale_percent)
                        .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                            let len = this.interactive_rebase_entries.len();
                            let entry_display_pos = len - 1 - ix;
                            if entry_display_pos + 1 < len {
                                let swap_ix = len - 1 - (entry_display_pos + 1);
                                this.interactive_rebase_entries.swap(ix, swap_ix);
                                validate_squash_entries(&mut this.interactive_rebase_entries);
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

                    let commit_id_val = CommitId(self.interactive_rebase_entries[ix].commit_id.clone().into());
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
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .child(crate::view::icons::svg_icon(
                                                "icons/squash_arrow.svg",
                                                with_alpha(theme.colors.accent, 0.7),
                                                px(14.0),
                                            )),
                                    )
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
                                let current_action = this.interactive_rebase_entries.get(ix).map(|e| e.action);
                                let can_drop = current_action == Some(InteractiveRebaseAction::Drop) || nd > 1;
                                let wh = window.window_handle();
                                let root = this.root_view.clone();
                                let pos = e.position;
                                cx.defer(move |cx| {
                                    let _ = wh.update(cx, |_, window, cx| {
                                        let _ = root.update(cx, |root, cx| {
                                            root.open_popover_at(
                                                PopoverKind::InteractiveRebaseActionMenu { ix, is_bottom, can_drop },
                                                pos,
                                                window,
                                                cx,
                                            );
                                        });
                                    });
                                });
                            }),
                        );

                    rows.push(row_div.into_any_element());
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
                                .filter(|&i| {
                                    drag_y > px((i as f32 + 0.5) * DRAG_ROW_HEIGHT)
                                })
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
        let is_modified = self.interactive_rebase_entries != self.interactive_rebase_original_entries;

        div()
            .flex()
            .flex_col()
            .size_full()
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
                                components::Button::new("irebase_reset", "Reset")
                                    .style(components::ButtonStyle::Outlined)
                                    .disabled(!is_modified)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cx.listener(
                                        move |this, _e: &gpui::ClickEvent, _w, cx| {
                                            this.interactive_rebase_entries =
                                                this.interactive_rebase_original_entries.clone();
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
                                            || !matches!(
                                                loading_state,
                                                Loadable::Ready(_)
                                            ),
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
                                            let autosquash = this.interactive_rebase_autosquash;
                                            this.store.dispatch(Msg::InteractiveRebase {
                                                repo_id,
                                                base: base.clone(),
                                                entries,
                                                autosquash,
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
