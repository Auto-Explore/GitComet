//! Pointer listeners installed during paint dirty the view that painted them:
//! a press or drag redraws that view and leaves cached siblings alone.

use crate::interaction::{InteractionState, InteractionStyle};
use crate::interaction_paint::InteractionPaint;
use crate::theme::AppTheme;
use crate::{MinimapColumn, Scrollbar};
use gpui::prelude::*;
use gpui::{
    AnyView, Context, Entity, Modifiers, MouseButton, Pixels, Point, ScrollHandle, StyleRefinement,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Subject {
    Scrollbar,
    Minimap,
    PaintedRow,
}

/// The cached view that paints the control under test.
struct Owner {
    subject: Subject,
    theme: AppTheme,
    scroll: ScrollHandle,
    jumps: Rc<Cell<usize>>,
    renders: usize,
}

impl Render for Owner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = self.theme;
        let subject = match self.subject {
            Subject::Scrollbar => div()
                .relative()
                .size_full()
                .child(
                    div()
                        .id("content")
                        .size_full()
                        .overflow_scroll()
                        .track_scroll(&self.scroll)
                        .child(div().w(px(100.0)).h(px(1200.0)).flex_none()),
                )
                .child(
                    Scrollbar::new("bar", self.scroll.clone())
                        .always_visible()
                        .debug_selector("subject")
                        .render(theme),
                )
                .into_any_element(),
            Subject::Minimap => {
                let jumps = self.jumps.clone();
                div()
                    .debug_selector(|| "subject".to_string())
                    .h_full()
                    .child(
                        MinimapColumn::new("map", Arc::from([true, false]), |_, _: bool| None)
                            .on_jump(move |_, _, _| jumps.set(jumps.get() + 1))
                            .render(theme),
                    )
                    .into_any_element()
            }
            Subject::PaintedRow => InteractionPaint::new(
                InteractionStyle::new(theme).on_surface(theme.colors.surface.chrome),
                InteractionState::default(),
            )
            .apply(
                div()
                    .id("row")
                    .debug_selector(|| "subject".to_string())
                    .w(px(100.0))
                    .h(px(28.0)),
            )
            .into_any_element(),
        };
        div().size_full().child(subject)
    }
}

#[derive(Default)]
struct Sibling {
    renders: usize,
}

impl Render for Sibling {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        div().size_full()
    }
}

struct Root {
    owner: Entity<Owner>,
    sibling: Entity<Sibling>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cached = |view: AnyView| {
            view.cached(
                StyleRefinement::default()
                    .w(px(200.0))
                    .h(px(120.0))
                    .flex_none(),
            )
        };
        div()
            .flex()
            .size_full()
            .child(cached(self.owner.clone().into()))
            .child(cached(self.sibling.clone().into()))
    }
}

struct Fixture<'a> {
    cx: &'a mut VisualTestContext,
    root: Entity<Root>,
    subject: gpui::Bounds<Pixels>,
}

impl Fixture<'_> {
    fn new(cx: &mut TestAppContext, subject: Subject) -> Fixture<'_> {
        let (root, cx) = cx.add_window_view(|_, cx| Root {
            owner: cx.new(|_| Owner {
                subject,
                theme: AppTheme::gitcomet_light(),
                scroll: ScrollHandle::new(),
                jumps: Rc::default(),
                renders: 0,
            }),
            sibling: cx.new(|_| Sibling::default()),
        });
        // Two full frames, so layout settles. Replayed frames keep no debug
        // bounds, so read them now.
        for _ in 0..2 {
            cx.update(|window, app| {
                window.refresh();
                let _ = window.draw(app);
            });
        }
        let subject = cx.debug_bounds("subject").expect("subject bounds");
        cx.run_until_parked();
        Fixture { cx, root, subject }
    }

    /// Render counts of the owner and of the sibling.
    fn renders(&mut self) -> (usize, usize) {
        let root = self.root.clone();
        self.cx.update(|_, app| {
            let root = root.read(app);
            (root.owner.read(app).renders, root.sibling.read(app).renders)
        })
    }

    /// Runs `event`, which must redraw the owner and only the owner.
    fn assert_redraws_owner_only(
        &mut self,
        what: &str,
        event: impl FnOnce(&mut VisualTestContext),
    ) {
        let before = self.renders();
        event(self.cx);
        self.cx.run_until_parked();
        let after = self.renders();
        assert!(after.0 > before.0, "{what} must redraw the owning view");
        assert_eq!(after.1, before.1, "{what} must not redraw a cached sibling");
    }
}

fn down(at: Point<Pixels>) -> impl FnOnce(&mut VisualTestContext) {
    move |cx| cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default())
}

fn drag(to: Point<Pixels>) -> impl FnOnce(&mut VisualTestContext) {
    move |cx| cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default())
}

fn up(at: Point<Pixels>) -> impl FnOnce(&mut VisualTestContext) {
    move |cx| cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default())
}

#[gpui::test]
fn scrollbar_gestures_redraw_only_the_view_that_painted_the_bar(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new(cx, Subject::Scrollbar);
    let bar = fixture.subject;
    let thumb = point(bar.right() - px(2.0), bar.top() + px(6.0));
    let dragged = point(thumb.x, thumb.y + px(40.0));

    fixture.assert_redraws_owner_only("a thumb press", down(thumb));
    fixture.assert_redraws_owner_only("a thumb drag", drag(dragged));
    fixture.assert_redraws_owner_only("a thumb release", up(dragged));
    let root = fixture.root.clone();
    let offset = fixture
        .cx
        .update(|_, app| root.read(app).owner.read(app).scroll.offset());
    assert!(offset.y < px(0.0), "the drag must scroll the content");

    // A press on the track pages the content.
    let track = point(thumb.x, bar.bottom() - px(6.0));
    fixture.assert_redraws_owner_only("a track press", down(track));
}

#[gpui::test]
fn minimap_gestures_redraw_only_the_view_that_painted_the_map(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new(cx, Subject::Minimap);
    let map = fixture.subject;
    let at = point(map.left() + px(5.0), map.top() + px(10.0));
    let dragged = point(at.x, at.y + px(40.0));

    fixture.assert_redraws_owner_only("a minimap press", down(at));
    fixture.assert_redraws_owner_only("a minimap drag", drag(dragged));
    let root = fixture.root.clone();
    let jumps = fixture
        .cx
        .update(|_, app| root.read(app).owner.read(app).jumps.get());
    assert_eq!(jumps, 2, "the press and the drag each jump");
}

#[gpui::test]
fn painted_press_feedback_redraws_only_the_view_that_painted_it(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new(cx, Subject::PaintedRow);
    let at = fixture.subject.center();
    // Hover first, so the press is the only change.
    fixture
        .cx
        .simulate_mouse_move(at, None, Modifiers::default());
    fixture.cx.run_until_parked();

    // gpui refreshes the window itself when a stateful element is pressed or
    // released, so those can only be shown to redraw the row.
    let before = fixture.renders();
    down(at)(fixture.cx);
    fixture.cx.run_until_parked();
    assert!(
        fixture.renders().0 > before.0,
        "a press must redraw the row"
    );

    // A press the release never reaches is cleared by the next plain move.
    fixture.assert_redraws_owner_only("a move with no button held", |cx| {
        cx.simulate_mouse_move(at + point(px(1.0), px(0.0)), None, Modifiers::default())
    });
}
