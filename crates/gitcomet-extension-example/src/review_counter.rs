//! A small view built only from the UI kit: a button that counts reviews.

use gitcomet_ui_kit::components::{Button, ButtonStyle};
use gitcomet_ui_kit::gpui::prelude::*;
use gitcomet_ui_kit::gpui::{Context, Window, div};
use gitcomet_ui_kit::theme::AppTheme;

pub struct ReviewCounter {
    theme: AppTheme,
    reviews: usize,
}

impl ReviewCounter {
    pub fn new(theme: AppTheme) -> Self {
        Self { theme, reviews: 0 }
    }

    pub fn reviews(&self) -> usize {
        self.reviews
    }

    pub fn set_reviews(&mut self, reviews: usize) {
        self.reviews = reviews;
    }
}

impl Render for ReviewCounter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div().p_2().child(
            Button::new(
                "review_counter_button",
                format!("Reviewed {}", self.reviews),
            )
            .style(ButtonStyle::Filled)
            .on_click(theme, cx, |this, _event, _window, cx| {
                this.reviews += 1;
                cx.notify();
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_ui_kit::test_support::{lock_visual_test, refresh_and_draw};
    use gpui::Modifiers;

    #[gpui::test]
    fn a_kit_button_counts_clicks(cx: &mut gpui::TestAppContext) {
        let _guard = lock_visual_test();
        let (view, cx) = cx.add_window_view(|_, _| ReviewCounter::new(AppTheme::gitcomet_dark()));
        refresh_and_draw(cx);
        let bounds = cx
            .debug_bounds("review_counter_button")
            .expect("the kit button draws with its debug selector");
        cx.simulate_click(bounds.center(), Modifiers::default());
        refresh_and_draw(cx);
        assert_eq!(cx.update(|_, app| view.read(app).reviews()), 1);
    }
}
