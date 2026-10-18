use super::super::*;
use super::DiffStageHover;
use super::StageGutterSpec;
use super::diff_row_horizontal_padding;
use super::diff_scaled_px;
use super::interaction::StageGutterMouse;
use super::interaction::install_stage_gutter_hover_handler;
use super::paint_centered_svg_icon;
use gpui::App;
use gpui::Bounds;
use gpui::CursorStyle;
use gpui::Hitbox;
use gpui::HitboxBehavior;
use gpui::PaintQuad;
use gpui::Pixels;
use gpui::Window;
use gpui::point;
use gpui::px;
use gpui::size;
use rustc_hash::FxHasher;
use std::hash::Hash;
use std::hash::Hasher;

/// Size of the hover-revealed stage/unstage button painted over the empty slack
/// at the far left of a change row. It overlays the line-number gutter rather
/// than reserving a column of its own, so enabling it never shifts diff text.
const DIFF_STAGE_GUTTER_CELL_PX: f32 = 16.0;

/// Rendered (square) size of the icon within the button.
const DIFF_STAGE_GUTTER_GLYPH_PX: f32 = 11.0;

pub(super) const DIFF_STAGE_GUTTER_STAGE_ICON: &str = "icons/plus.svg";

pub(super) const DIFF_STAGE_GUTTER_UNSTAGE_ICON: &str = "icons/minus.svg";

/// Fold a row's stage-gutter button into its canvas revision key so the cached
/// canvas repaints when the button appears, disappears, or flips direction
/// (staging vs. unstaging). `hovered` is this row's own stored hover state, not
/// the raw cursor position, so mouse movement invalidates only the two rows
/// whose button visibility actually changed.
pub(super) fn mix_stage_gutter_revision(
    base: u64,
    specs: &[Option<StageGutterSpec>],
    stage_hover: Option<DiffStageHover>,
    visible_ix: usize,
) -> u64 {
    if specs.iter().all(Option::is_none) {
        return base;
    }
    let mut hasher = FxHasher::default();
    base.hash(&mut hasher);
    for spec in specs {
        match spec {
            Some(spec) => {
                spec.area.hash(&mut hasher);
                spec.slot.hash(&mut hasher);
                // The kind decides which patch line a click resolves to, and the
                // paint closure captures it, so a cached canvas must not outlive
                // a change to it.
                std::mem::discriminant(&spec.kind).hash(&mut hasher);
                stage_hover
                    .filter(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
                    .map(|hover| hover.on_button)
                    .hash(&mut hasher);
            }
            None => u8::MAX.hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// Cell the stage/unstage button occupies: the far left of a row, which with
/// line numbers shown is the empty slack ahead of the right-aligned number.
/// Shared by painting and hit-testing so the two cannot drift apart.
///
/// With line numbers hidden there is no such slack — the diff text starts here
/// — so the button overlaps its first characters and takes the clicks landing on
/// them. That is deliberate: reaching the button everywhere is worth more than
/// the couple of characters it sits on, and the chip is painted opaque so what
/// it covers reads as covered rather than as garbled text.
fn stage_gutter_cell(
    content_left: Pixels,
    row_top: Pixels,
    row_height: Pixels,
    ui_scale_percent: u32,
) -> Bounds<Pixels> {
    let pad = diff_row_horizontal_padding(ui_scale_percent);
    let width = diff_scaled_px(DIFF_STAGE_GUTTER_CELL_PX, ui_scale_percent);
    let height = width.min(row_height);
    Bounds::new(
        point(
            content_left + pad * 0.5,
            row_top + (row_height - height) * 0.5,
        ),
        size(width, height),
    )
}

#[derive(Clone, Debug)]
pub(super) struct StageGutterPrepaint {
    pub(super) spec: StageGutterSpec,
    pub(super) cell: Bounds<Pixels>,
    pub(super) hitbox: Hitbox,
}

/// Reserve the button's hitbox during prepaint. Called after the text hitbox so
/// the button's pointer cursor — and its clicks — win over the text I-beam, and
/// clipped to the content mask so a scrolled-away button cannot be hovered.
pub(super) fn build_stage_gutter(
    window: &mut Window,
    spec: Option<StageGutterSpec>,
    content_left: Pixels,
    row_bounds: Bounds<Pixels>,
    ui_scale_percent: u32,
) -> Option<StageGutterPrepaint> {
    let spec = spec?;
    let cell = stage_gutter_cell(
        content_left,
        row_bounds.top(),
        row_bounds.size.height,
        ui_scale_percent,
    );
    let visible = cell.intersect(&window.content_mask().bounds);
    if visible.size.width <= px(0.0) || visible.size.height <= px(0.0) {
        return None;
    }
    Some(StageGutterPrepaint {
        spec,
        cell,
        hitbox: window.insert_hitbox(visible, HitboxBehavior::Normal),
    })
}

/// Paint a row's stage/unstage button and register its hover handling, returning
/// the click routing for it. Hovering anywhere in the row reveals the button;
/// hovering the button itself brightens it. The hover state comes from the view
/// (never from the live cursor) so it matches the value folded into the canvas
/// revision key. Clicks are handled by `install_diff_row_mouse_handlers`.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_stage_gutter(
    prepaint: Option<&StageGutterPrepaint>,
    visible_ix: usize,
    theme: AppTheme,
    row_bg: gpui::Rgba,
    ui_scale_percent: u32,
    row_hitbox: &Hitbox,
    column_bounds: Option<Bounds<Pixels>>,
    view: &Entity<DiffRenderer>,
    window: &mut Window,
    cx: &mut App,
) -> Option<StageGutterMouse> {
    let prepaint = prepaint?;
    let spec = prepaint.spec;
    window.set_cursor_style(CursorStyle::PointingHand, &prepaint.hitbox);

    let hover = view.update(cx, |this, _cx| {
        this.set_diff_stage_gutter_cell(visible_ix, spec.slot, prepaint.cell);
        this.diff_stage_gutter_hover
            .filter(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
    });

    if let Some(hover) = hover {
        let color = spec.color(theme);
        // The chip is opaque so it masks any line-number digits it covers. Over
        // the row it stays quiet; under the pointer it takes a tint of the
        // action it performs.
        let chip = crate::kit::interaction::InteractionStyle::tinted(theme, color)
            .resolved_background(
                row_bg,
                crate::kit::interaction::InteractionState::default(),
                if hover.on_button {
                    crate::kit::interaction::InteractionFeedback::Hovered
                } else {
                    crate::kit::interaction::InteractionFeedback::Resting
                },
            );
        let icon = if hover.on_button {
            color
        } else {
            with_alpha(color, 0.75)
        };
        window.paint_quad(
            PaintQuad::new(prepaint.cell)
                .background(chip)
                .corner_radii(px(theme.radii.control)),
        );
        paint_centered_svg_icon(
            spec.icon(),
            prepaint.cell,
            diff_scaled_px(DIFF_STAGE_GUTTER_GLYPH_PX, ui_scale_percent),
            icon,
            window,
            cx,
        );
    }

    install_stage_gutter_hover_handler(
        window,
        view,
        visible_ix,
        spec,
        &prepaint.hitbox,
        row_hitbox,
        column_bounds,
    );

    Some(StageGutterMouse {
        hitbox: prepaint.hitbox.clone(),
        kind: spec.kind,
    })
}
