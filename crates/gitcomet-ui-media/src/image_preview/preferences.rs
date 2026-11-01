use super::*;
use gpui::App;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Preferences {
    pub mode: ImageSamplingMode,
    pub filter: ImageSmoothFilter,
}
impl gpui::Global for Preferences {}

impl Preferences {
    /// Installs the saved preferences unless another window already did.
    pub fn initialize(self, cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(self);
        }
    }
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }
    pub fn label(self) -> String {
        if self.mode == ImageSamplingMode::Pixels {
            self.mode.label().to_string()
        } else {
            format!("{} · {}", self.mode.label(), self.filter.label())
        }
    }
}
