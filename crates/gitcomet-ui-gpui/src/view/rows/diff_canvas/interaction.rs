use super::super::*;
use super::AnnotArea;
use super::DiffStageHover;
use super::StageGutterSpec;
use super::annotation::AnnotHitboxes;
use gitcomet_core::domain::DiffLineKind;
use gpui::App;
use gpui::Bounds;
use gpui::DispatchPhase;
use gpui::Hitbox;
use gpui::Pixels;
use gpui::Window;
use rustc_hash::FxHasher;
use std::hash::Hash;
use std::hash::Hasher;

/// Register hover handling for a row's stage-gutter button: on every mouse move
/// it resolves whether the cursor is over this row (which reveals the button) and
/// whether it is on the button itself (which brightens it and shows a tooltip),
/// updating the view only when that changes. `column_bounds` narrows the row to
/// one side in split views, where both columns paint their own button.
pub(super) fn install_stage_gutter_hover_handler(
    window: &mut Window,
    view: &Entity<DiffRenderer>,
    visible_ix: usize,
    spec: StageGutterSpec,
    hitbox: &Hitbox,
    row_hitbox: &Hitbox,
    column_bounds: Option<Bounds<Pixels>>,
) {
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        let row_hitbox = row_hitbox.clone();
        move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // Hover follows the hit test, so a panel painted over the diff hides
            // the button underneath instead of revealing it through the overlay.
            let on_row = row_hitbox.is_hovered(window)
                && column_bounds.is_none_or(|bounds| bounds.contains(&event.position));
            let next = on_row.then(|| DiffStageHover {
                visible_ix,
                slot: spec.slot,
                on_button: hitbox.is_hovered(window),
            });

            // Cheap gate so plain mouse movement doesn't borrow/notify the view
            // for every visible row: only act when this button's hover changes,
            // and never clear a hover that belongs to a different button.
            let current = view.read(cx).diff_stage_gutter_hover;
            if current == next {
                return;
            }
            if next.is_none()
                && !current
                    .is_some_and(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
            {
                return;
            }

            let tooltip = next.filter(|hover| hover.on_button).map(|_| spec.tooltip());
            view.update(cx, |this, cx| {
                this.update_diff_stage_gutter_hover(next, tooltip, cx);
            });
        }
    });
}

/// Register click handling for a row's annotation column.
#[allow(clippy::too_many_arguments)]
pub(super) fn install_blame_annotation_mouse_handler(
    window: &mut Window,
    view: &Entity<DiffRenderer>,
    scope: gpui::ElementId,
    message_hitbox: &Hitbox,
    prior_icon_hitbox: &Hitbox,
    browse_icon_hitbox: &Hitbox,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::sync::Arc<std::path::Path>,
    source_path: Option<std::sync::Arc<std::path::Path>>,
    prior_commit: Option<gitcomet_core::domain::CommitId>,
    message_enabled: bool,
    prior_enabled: bool,
    browse_enabled: bool,
) {
    for (name, hitbox, enabled, action) in [
        (
            "details",
            message_hitbox,
            message_enabled,
            BlameClickAction::OpenDetails,
        ),
        (
            "prior",
            prior_icon_hitbox,
            prior_enabled,
            BlameClickAction::PriorRevision,
        ),
        (
            "browse",
            browse_icon_hitbox,
            browse_enabled,
            BlameClickAction::Browse,
        ),
    ] {
        if !enabled {
            continue;
        }
        let target = crate::kit::click::canvas_target(
            "blame-click",
            (&scope, name, commit_id.as_ref(), &path),
        );
        let view = view.clone();
        let commit_id = commit_id.clone();
        let historical_path = source_path.as_deref().unwrap_or(&path).to_path_buf();
        let prior_commit = prior_commit.clone();
        crate::kit::click::on_canvas_click(
            window,
            target,
            hitbox,
            gpui::MouseButton::Left,
            true,
            move |_, _, cx| {
                view.update(cx, |this, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    let msg = match action {
                        BlameClickAction::Browse => Msg::OpenFileAtCommit {
                            repo_id,
                            commit_id: commit_id.clone(),
                            path: historical_path.clone(),
                        },
                        BlameClickAction::PriorRevision => match prior_commit.clone() {
                            Some(base) => Msg::OpenFileAtCommit {
                                repo_id,
                                commit_id: base,
                                path: historical_path.clone(),
                            },
                            None => Msg::OpenFileAtCommitParent {
                                repo_id,
                                commit_id: commit_id.clone(),
                                path: historical_path.clone(),
                            },
                        },
                        BlameClickAction::OpenDetails => Msg::SelectCommit {
                            request_id: None,
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                    };
                    this.store.dispatch(msg);
                    cx.notify();
                });
            },
        );
    }
}

enum BlameClickAction {
    OpenDetails,
    PriorRevision,
    Browse,
}

/// Register hover handling for a row's annotation column. On every mouse move it
/// resolves which sub-area (message / prior icon / browse icon) the cursor is
/// over and, only when that changes for this row, updates the view's hover state
/// (which drives the accent highlight on repaint) and the shared tooltip host.
pub(super) fn install_blame_annotation_hover_handler(
    window: &mut Window,
    view: &Entity<DiffRenderer>,
    visible_ix: usize,
    hitboxes: &AnnotHitboxes,
    summary: SharedString,
    body: Option<SharedString>,
    message_enabled: bool,
    prior_enabled: bool,
    browse_enabled: bool,
) {
    window.on_mouse_event({
        let view = view.clone();
        let message = hitboxes.message.clone();
        let prior_icon = hitboxes.prior_icon.clone();
        let browse_icon = hitboxes.browse_icon.clone();
        move |_event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // Hover follows the hit test, so a panel painted over the diff hides
            // the annotations underneath instead of highlighting them through it.
            let area = if message_enabled && message.is_hovered(window) {
                Some(AnnotArea::Message)
            } else if prior_enabled && prior_icon.is_hovered(window) {
                Some(AnnotArea::PriorIcon)
            } else if browse_enabled && browse_icon.is_hovered(window) {
                Some(AnnotArea::BrowseIcon)
            } else {
                None
            };

            // Cheap gate so plain mouse movement doesn't borrow/notify the view
            // for every visible row: only act when this row's hover changes, and
            // never clear a hover that belongs to a different row.
            let next = area.map(|a| (visible_ix, a));
            let current = view.read(cx).blame_annot_hover;
            if current == next {
                return;
            }
            if next.is_none() && !matches!(current, Some((ix, _)) if ix == visible_ix) {
                return;
            }

            let tooltip = match area {
                Some(AnnotArea::Message) => Some(body.clone().unwrap_or_else(|| summary.clone())),
                Some(AnnotArea::PriorIcon) => {
                    Some(SharedString::from("View file prior this change"))
                }
                Some(AnnotArea::BrowseIcon) => Some(SharedString::from("View file at this commit")),
                None => None,
            };

            view.update(cx, |this, cx| {
                this.update_blame_annot_hover(next, tooltip, cx);
            });
        }
    });
}

#[derive(Clone, Debug)]
pub(super) enum DiffRowTextRegions {
    Single {
        region: DiffTextRegion,
        bounds: Bounds<Pixels>,
    },
    Split {
        left_bounds: Bounds<Pixels>,
        right_bounds: Bounds<Pixels>,
    },
}

impl DiffRowTextRegions {
    pub(super) fn single(region: DiffTextRegion, bounds: Bounds<Pixels>) -> Self {
        Self::Single { region, bounds }
    }

    pub(super) fn split(left_bounds: Bounds<Pixels>, right_bounds: Bounds<Pixels>) -> Self {
        Self::Split {
            left_bounds,
            right_bounds,
        }
    }

    fn region_at(&self, position: gpui::Point<Pixels>) -> Option<DiffTextRegion> {
        match self {
            Self::Single { region, bounds } => bounds.contains(&position).then_some(*region),
            Self::Split {
                left_bounds,
                right_bounds,
            } => {
                if left_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitLeft)
                } else if right_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitRight)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffRowRightClickBehavior {
    OpenContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffRowMouseUpBehavior {
    None,
    HandlePatchRowClick,
}

#[derive(Clone, Debug)]
pub(super) struct DiffRowMouseHandlers {
    pub(super) row_hitbox: Hitbox,
    pub(super) regions: DiffRowTextRegions,
    pub(super) right_click: DiffRowRightClickBehavior,
    pub(super) mouse_up: DiffRowMouseUpBehavior,
    /// Stage/unstage buttons painted in this row's gutter(s): one per column, so
    /// at most two in split views and at most one anywhere else.
    pub(super) stage: Vec<StageGutterMouse>,
}

/// Click routing for one stage/unstage button.
#[derive(Clone, Debug)]
pub(super) struct StageGutterMouse {
    pub(super) hitbox: Hitbox,
    pub(super) kind: DiffLineKind,
}

impl StageGutterMouse {
    fn hovered(buttons: &[Self], window: &Window) -> Option<DiffLineKind> {
        buttons
            .iter()
            .find(|button| button.hitbox.is_hovered(window))
            .map(|button| button.kind)
    }
}

/// Row mouse handlers are window-level listeners: they see every event, whatever
/// is painted over the diff. Asking the row's hitbox (rather than its bounds)
/// whether it is hovered defers to the hit test, so a click on a panel floating
/// above the diff — the collapsed sidebar's section popover, say — stops there
/// instead of also landing on the row beneath it.
fn should_handle_row_mouse_event(
    phase: DispatchPhase,
    row_hitbox: &Hitbox,
    window: &Window,
) -> bool {
    phase == DispatchPhase::Bubble && row_hitbox.is_hovered(window)
}

/// A row's click target, scoped to the view and to the repo, file and
/// revisions it shows: a redraw keeps a pending click, new content never
/// inherits one.
pub(super) fn diff_canvas_click_target(
    view: &Entity<DiffRenderer>,
    visible_ix: usize,
    action: impl Hash,
    cx: &App,
) -> gpui::ElementId {
    let pane = view.read(cx);
    let mut target = FxHasher::default();
    if let Some(diff_target) = pane.rendered_diff_target() {
        crate::view::fingerprint::hash_diff_target(diff_target, &mut target);
    }
    crate::kit::click::canvas_target(
        "diff-click",
        (
            view.entity_id(),
            pane.active_repo_id(),
            target.finish(),
            pane.rendered_patch_diff_rev(),
            pane.diff_visible_projection_rev,
            visible_ix,
            action,
        ),
    )
}

pub(super) fn install_diff_row_mouse_handlers(
    window: &mut Window,
    view: &Entity<DiffRenderer>,
    visible_ix: usize,
    handlers: DiffRowMouseHandlers,
    cx: &App,
) {
    let DiffRowMouseHandlers {
        row_hitbox,
        regions,
        right_click,
        mouse_up,
        stage,
    } = handlers;
    if mouse_up != DiffRowMouseUpBehavior::None {
        let target = diff_canvas_click_target(view, visible_ix, "row", cx);
        let view = view.clone();
        let stage = stage.clone();
        crate::kit::click::on_canvas_click(
            window,
            target,
            &row_hitbox,
            gpui::MouseButton::Left,
            false,
            move |event, window, cx| {
                if StageGutterMouse::hovered(&stage, window).is_some() {
                    return;
                }
                view.update(cx, |this, cx| {
                    if !this.consume_suppress_click_after_drag() {
                        this.handle_patch_row_click(
                            visible_ix,
                            DiffClickKind::Line,
                            event.modifiers().shift,
                        );
                    }
                    cx.notify();
                });
            },
        );
    }
    let text_hitbox = row_hitbox.clone();
    let text_regions = regions.clone();
    let stage_for_down = stage.clone();
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if event.button != gpui::MouseButton::Left
                || !should_handle_row_mouse_event(phase, &text_hitbox, window)
            {
                return;
            }
            let focus = view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            if StageGutterMouse::hovered(&stage_for_down, window).is_some() {
                return;
            }
            if let Some(region) = text_regions.region_at(event.position) {
                view.update(cx, |this, cx| {
                    this.handle_diff_text_mouse_down(
                        visible_ix,
                        region,
                        event.position,
                        event.click_count,
                        window,
                        cx,
                    );
                    cx.notify();
                });
            }
        }
    });
    let target = diff_canvas_click_target(view, visible_ix, "context", cx);
    let menu_view = view.clone();
    crate::kit::click::on_canvas_click(
        window,
        target,
        &row_hitbox,
        gpui::MouseButton::Right,
        true,
        move |event, window, cx| {
            if let Some(region) = regions.region_at(event.position()) {
                match right_click {
                    DiffRowRightClickBehavior::OpenContextMenu => {
                        menu_view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                visible_ix,
                                region,
                                event.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        })
                    }
                }
            }
        },
    );
    for (slot, button) in stage.into_iter().enumerate() {
        let target = diff_canvas_click_target(
            view,
            visible_ix,
            ("stage", slot, std::mem::discriminant(&button.kind)),
            cx,
        );
        let view = view.clone();
        crate::kit::click::on_canvas_click(
            window,
            target,
            &button.hitbox,
            gpui::MouseButton::Left,
            true,
            move |event, window, cx| {
                if event.click_count() > 1 {
                    return;
                }
                view.update(cx, |this, cx| {
                    window.focus(&this.diff_panel_focus_handle, cx);
                    this.stage_or_unstage_diff_line(visible_ix, button.kind, cx);
                    cx.notify();
                });
            },
        );
    }
}

#[cfg(test)]
mod tests;
