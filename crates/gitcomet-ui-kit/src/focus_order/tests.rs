use super::*;
use crate::test_support::refresh_and_draw;
use crate::{TextInput, TextInputOptions, TextInputTab};
use gpui::{Context, Entity, Render, TestAppContext, VisualTestContext, px};

/// A window with a plain field before a form, the form, and a plain field
/// after it.
struct Fixture {
    before: Entity<TextInput>,
    button: FocusHandle,
    after: Entity<TextInput>,
    /// Inside the form; painted in this order.
    form: Vec<Entity<TextInput>>,
    scope: Option<TabScope>,
}

impl Fixture {
    fn new(
        form_indices: &[isize],
        scoped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.activate();
        let mut input = |tab_index: isize, cx: &mut Context<Self>| {
            cx.new(|cx| {
                TextInput::new(
                    TextInputOptions {
                        tab_index,
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
        };
        Self {
            before: input(0, cx),
            button: field(cx),
            after: input(0, cx),
            form: form_indices.iter().map(|index| input(*index, cx)).collect(),
            scope: scoped.then(|| TabScope::new(cx)),
        }
    }

    fn handle(&self, ix: usize, cx: &App) -> FocusHandle {
        self.form[ix].read(cx).focus_handle()
    }
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let form = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "form_label".into())
                    .h(px(20.0))
                    .child("Label"),
            )
            .children(self.form.iter().cloned());
        let form = match &self.scope {
            Some(scope) => scope.wrap(form).into_any_element(),
            None => form.into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(self.before.clone())
            .child(div().track_focus(&self.button).child("Button"))
            .child(form)
            .child(self.after.clone())
    }
}

fn window<'a>(
    cx: &'a mut TestAppContext,
    form_indices: &'static [isize],
    scoped: bool,
) -> (Entity<Fixture>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.bind_keys(key_bindings());
        install(cx);
    });
    let (view, cx) =
        cx.add_window_view(move |window, cx| Fixture::new(form_indices, scoped, window, cx));
    refresh_and_draw(cx);
    (view, cx)
}

fn focus(cx: &mut VisualTestContext, handle: impl FnOnce(&App) -> FocusHandle) {
    cx.update(|window, app| {
        let handle = handle(app);
        window.focus(&handle, app);
    });
    refresh_and_draw(cx);
}

#[track_caller]
fn assert_focused(
    cx: &mut VisualTestContext,
    handle: impl FnOnce(&App) -> FocusHandle,
    what: &str,
) {
    cx.update(|window, app| {
        let handle = handle(app);
        assert!(handle.is_focused(window), "expected focus on {what}");
    });
}

fn press(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    refresh_and_draw(cx);
}

#[gpui::test]
fn tab_moves_out_of_text_inputs_through_every_field(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, &[0, 0], false);
    focus(cx, |app| view.read(app).before.read(app).focus_handle());

    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).button.clone(), "the button");
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).handle(0, app),
        "the first form field",
    );
    press(cx, "tab tab");
    assert_focused(
        cx,
        |app| view.read(app).after.read(app).focus_handle(),
        "the last field",
    );
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).before.read(app).focus_handle(),
        "the first field again: the window wraps",
    );
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| view.read(app).after.read(app).focus_handle(),
        "the last field",
    );

    cx.update(|_, app| {
        let view = view.read(app);
        for input in [&view.before, &view.after].into_iter().chain(&view.form) {
            assert_eq!(
                input.read(app).text(),
                "",
                "Tab moved focus; it typed nothing"
            );
        }
    });
}

#[gpui::test]
fn tab_index_orders_fields_before_paint_order(cx: &mut TestAppContext) {
    // Painted 0, 1, 2; visited 1, 2, 0.
    let (view, cx) = window(cx, &[3, 1, 2], true);
    focus(cx, |app| view.read(app).handle(1, app));
    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).handle(2, app), "index 2");
    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).handle(0, app), "index 3");
    press(cx, "shift-tab shift-tab");
    assert_focused(cx, |app| view.read(app).handle(1, app), "index 1");
}

#[gpui::test]
fn a_scope_keeps_tab_inside_its_form(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, &[0, 0, 0], true);
    focus(cx, |app| view.read(app).handle(2, app));
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).handle(0, app),
        "the form's first field",
    );
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| view.read(app).handle(2, app),
        "the form's last field",
    );
    press(cx, "shift-tab");
    assert_focused(cx, |app| view.read(app).handle(1, app), "the middle field");

    // Entering from either end.
    cx.update(|window, app| {
        let scope = view.read(app).scope.clone().unwrap();
        scope.focus_last(window, app);
        assert!(view.read(app).handle(2, app).is_focused(window));
        scope.focus_first(window, app);
        assert!(view.read(app).handle(0, app).is_focused(window));
        assert!(scope.contains_focused(window, app));
    });
}

#[gpui::test]
fn a_scope_without_fields_keeps_focus_where_it_was(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, &[], true);
    cx.update(|window, app| {
        let scope = view.read(app).scope.clone().unwrap();
        window.focus(scope.container(), app);
        scope.step(Direction::Next, window, app);
        assert!(scope.container().is_focused(window));
    });
}

#[gpui::test]
fn inputs_that_keep_tab_report_or_type_it(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, &[0], false);
    cx.update(|_, app| {
        let input = view.read(app).before.clone();
        input.update(app, |input, cx| input.set_tab(TextInputTab::Report, cx));
    });
    refresh_and_draw(cx);
    focus(cx, |app| view.read(app).before.read(app).focus_handle());
    press(cx, "tab");
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| view.read(app).before.read(app).focus_handle(),
        "the picker input",
    );
    cx.update(|_, app| {
        view.read(app).before.clone().update(app, |input, _| {
            assert!(input.take_tab_pressed());
            assert!(input.take_shift_tab_pressed());
            assert_eq!(input.text(), "", "a reported Tab is not typed");
        });
    });

    cx.update(|_, app| {
        let input = view.read(app).before.clone();
        input.update(app, |input, cx| input.set_tab(TextInputTab::Insert, cx));
    });
    refresh_and_draw(cx);
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).before.read(app).focus_handle(),
        "the editor",
    );
    cx.update(|_, app| assert_eq!(view.read(app).before.read(app).text(), "\t"));
}

#[gpui::test]
fn selectable_text_is_not_a_tab_stop(cx: &mut TestAppContext) {
    let label =
        cx.update(|app| app.new(|cx| TextInput::new_inert(TextInputOptions::selectable(), cx)));
    let field =
        cx.update(|app| app.new(|cx| TextInput::new_inert(TextInputOptions::default(), cx)));
    cx.update(|app| {
        assert!(!label.read(app).focus_handle().tab_stop);
        assert!(field.read(app).focus_handle().tab_stop);
        field.update(app, |field, _| field.set_tab_stop(false));
        assert!(!field.read(app).focus_handle().tab_stop);
        // A document that becomes editable becomes a field.
        label.update(app, |label, cx| label.set_read_only(false, cx));
        assert!(label.read(app).focus_handle().tab_stop);
    });
}

#[test]
fn bindings_stand_aside_where_tab_is_captured() {
    let [next, previous] = key_bindings();
    let captured = [key_context("Terminal", true)];
    let plain = [key_context("TextInput", false)];
    for binding in [next, previous] {
        let predicate = binding.predicate().expect("bound to a context");
        assert!(predicate.eval(&plain));
        assert!(!predicate.eval(&captured));
    }
}

#[gpui::test]
fn pressing_a_form_off_its_controls_keeps_the_focused_field(cx: &mut TestAppContext) {
    let (view, cx) = window(cx, &[0, 0], true);
    let label = cx
        .debug_bounds("form_label")
        .expect("the form's label")
        .center();

    focus(cx, |app| view.read(app).handle(1, app));
    cx.simulate_click(label, gpui::Modifiers::default());
    refresh_and_draw(cx);
    assert_focused(
        cx,
        |app| view.read(app).handle(1, app),
        "the field that had focus",
    );

    // From outside, the press lands on the form, and Tab enters it.
    focus(cx, |app| view.read(app).button.clone());
    cx.simulate_click(label, gpui::Modifiers::default());
    refresh_and_draw(cx);
    assert_focused(
        cx,
        |app| view.read(app).scope.as_ref().unwrap().container().clone(),
        "the form",
    );
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).handle(0, app),
        "the form's first field",
    );
}
