//! A diagram as a block of a markdown document: the height it shows, the bar
//! that collapses it, the reader's choices for it, and its size before drawn.
use super::*;
use gitcomet_ui_kit::components;
use gitcomet_ui_kit::icons::svg_icon;
use gpui::point;
use rustc_hash::FxHashMap;

/// The viewport a markdown block's diagram folds into when collapsed.
pub(super) const COLLAPSED_HEIGHT: f32 = 320.0;

/// How much of a markdown block's diagram its viewport shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BlockHeight {
    /// The whole diagram at 100%.
    Full,
    /// At most `COLLAPSED_HEIGHT`; the rest is reached by panning.
    Collapsed,
}

/// `gap_2`, between the parts of a diagram block.
pub(super) const BLOCK_GAP: f32 = 8.0;
/// Design size and line height of a diagnostic or tile error under a diagram.
const STATUS_TEXT: f32 = 12.0;
pub(super) const STATUS_LINE: f32 = 18.0;

/// A control row and its gap to the viewport: the Preview | Code header above,
/// the bar below.
pub(super) fn block_control_row(scale: gitcomet_ui_kit::ui_scale::UiScale) -> f32 {
    f32::from(components::control_height(scale) + scale.px(BLOCK_GAP))
}

/// A warning line under a diagram, at a fixed line height so an undrawn
/// block's size can count it.
pub(super) fn status_line(
    text: String,
    theme: AppTheme,
    scale: gitcomet_ui_kit::ui_scale::UiScale,
) -> gpui::Div {
    div()
        .text_size(scale.ui_text(STATUS_TEXT))
        .line_height(scale.ui_text(STATUS_LINE))
        .text_color(theme.colors.status.warning.foreground)
        .child(text)
}

/// What the reader chose for one markdown diagram block.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct BlockChoice {
    pub(super) mode: RenderedPreviewMode,
    pub(super) height: BlockHeight,
}

impl Default for BlockChoice {
    fn default() -> Self {
        Self {
            mode: RenderedPreviewMode::Rendered,
            height: BlockHeight::Full,
        }
    }
}

/// The reader's choices for markdown diagram blocks, by diagram, kept for the
/// session so a block rebuilt later, or shown elsewhere, opens the same way.
#[derive(Clone, Default)]
pub(super) struct BlockChoices(pub(super) Rc<RefCell<FxHashMap<DiagramKey, BlockChoice>>>);

impl gpui::Global for BlockChoices {}

impl BlockChoices {
    pub(super) fn current(cx: &mut App) -> Self {
        cx.default_global::<Self>().clone()
    }
    pub(super) fn get(&self, key: &DiagramKey) -> BlockChoice {
        self.0.borrow().get(key).copied().unwrap_or_default()
    }
    /// A choice back at the default is forgotten, and its source with it.
    pub(super) fn update(&self, key: &DiagramKey, change: impl FnOnce(&mut BlockChoice)) {
        let mut choices = self.0.borrow_mut();
        let choice = choices.entry(key.clone()).or_default();
        change(choice);
        if *choice == BlockChoice::default() {
            choices.remove(key);
        }
    }
}

/// What is known about markdown diagram blocks before they are drawn: the
/// prepared scenes and the reader's choices. The default knows nothing.
#[derive(Clone, Default)]
pub struct DiagramHeights {
    pub(super) cache: Arc<DiagramCache>,
    pub(super) choices: BlockChoices,
}

impl DiagramHeights {
    pub fn new(cx: &mut App) -> Self {
        Self {
            cache: cache(cx),
            choices: BlockChoices::current(cx),
        }
    }

    /// A block's height before it is drawn, as the reader left it: its source
    /// listing (`listing` tall), or its diagram once the scene is prepared,
    /// or its error and listing if that failed, else the collapsed viewport.
    pub fn estimate_block(
        &self,
        key: &DiagramKey,
        scale: gitcomet_ui_kit::ui_scale::UiScale,
        listing: f32,
    ) -> f32 {
        // The controls scale with the UI; the diagram is drawn at its own size.
        let row = block_control_row(scale);
        let status = f32::from(scale.ui_text(STATUS_LINE) + scale.px(BLOCK_GAP));
        // As the block draws: a failed diagram shows its error whatever the
        // reader chose, then the source.
        let facts = self.cache.scene_facts(key);
        if facts == Some(SceneFacts::Failed) {
            return row + status + listing;
        }
        let choice = self.choices.get(key);
        if choice.mode == RenderedPreviewMode::Source {
            return row + listing;
        }
        let Some(SceneFacts::Drawn {
            height: scene,
            diagnostics,
        }) = facts
        else {
            return row + COLLAPSED_HEIGHT;
        };
        let full = full_height(scene);
        let viewport = match choice.height {
            BlockHeight::Full => full,
            BlockHeight::Collapsed => full.min(COLLAPSED_HEIGHT),
        };
        let bar = if full > COLLAPSED_HEIGHT { row } else { 0.0 };
        row + viewport + bar + diagnostics as f32 * status
    }
}

impl DiagramState {
    /// A markdown block's viewport, with a bar under a diagram taller than
    /// `COLLAPSED_HEIGHT` that folds it to that height and back.
    pub(super) fn render_block(
        &mut self,
        diagram: Arc<RasterDiagram>,
        theme: AppTheme,
        scroll: Option<gpui::ScrollHandle>,
        key: &DiagramKey,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let full = full_height(diagram.scene.height());
        let block_height = self.choices.get(key).height;
        let height = match block_height {
            BlockHeight::Full => full,
            BlockHeight::Collapsed => full.min(COLLAPSED_HEIGHT),
        };
        let viewport = self.render_viewport(diagram, theme, Some(height), cx);
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(viewport)
            .when(full > COLLAPSED_HEIGHT, |d| {
                d.child(self.block_height_toggle(block_height, key.clone(), scroll, theme, cx))
            })
            .into_any_element()
    }

    fn block_height_toggle(
        &self,
        block_height: BlockHeight,
        key: DiagramKey,
        scroll: Option<gpui::ScrollHandle>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let (label, icon, next) = match block_height {
            BlockHeight::Full => (
                "Collapse diagram",
                "icons/chevron_up.svg",
                BlockHeight::Collapsed,
            ),
            BlockHeight::Collapsed => (
                "Show full diagram",
                "icons/chevron_down.svg",
                BlockHeight::Full,
            ),
        };
        let icon_size = gitcomet_ui_kit::ui_scale::UiScale::current(cx).px(12.0);
        components::Button::new("diagram_block_height_toggle", label)
            .start_slot(svg_icon(icon, theme.colors.foreground.secondary, icon_size))
            .style(components::ButtonStyle::Outlined)
            .on_click(theme, cx, move |this, _, _, cx| {
                // Folding a block whose top has scrolled away would leave the
                // reader below it; scroll back just enough to keep it in view.
                if next == BlockHeight::Collapsed
                    && let Some(scroll) = &scroll
                {
                    let offset = scroll.offset();
                    let shift = px(this.hidden_above);
                    scroll.set_offset(point(offset.x, (offset.y + shift).min(px(0.0))));
                }
                this.choices.update(&key, |choice| choice.height = next);
                // Show the whole diagram, or its top, from 100%.
                *this.viewport.borrow_mut() = Viewport::default();
                this.invalidate_tiles();
                cx.notify();
            })
            .w_full()
            .into_any_element()
    }
}
