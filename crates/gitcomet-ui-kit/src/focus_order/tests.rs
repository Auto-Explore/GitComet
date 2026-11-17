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

/// What the fixture's form is.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    Plain,
    Scope,
    Modal,
}

impl Fixture {
    fn new(
        form_indices: &[isize],
        form: Form,
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
            scope: match form {
                Form::Plain => None,
                Form::Scope => Some(TabScope::new(cx)),
                Form::Modal => Some(TabScope::modal(cx)),
            },
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
    let form = if scoped { Form::Scope } else { Form::Plain };
    window_with(cx, form_indices, form)
}

fn window_with<'a>(
    cx: &'a mut TestAppContext,
    form_indices: &'static [isize],
    form: Form,
) -> (Entity<Fixture>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.bind_keys(key_bindings());
        install(cx);
    });
    let (view, cx) =
        cx.add_window_view(move |window, cx| Fixture::new(form_indices, form, window, cx));
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

    // Entering from the form itself, at either end.
    let container = |app: &App| view.read(app).scope.as_ref().unwrap().container().clone();
    focus(cx, container);
    press(cx, "shift-tab");
    assert_focused(cx, |app| view.read(app).handle(2, app), "the last field");
    focus(cx, container);
    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).handle(0, app), "the first field");
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
        // Text that becomes editable becomes a field, and the other way.
        label.update(app, |label, cx| label.set_read_only(false, cx));
        assert!(label.read(app).focus_handle().tab_stop);
        label.update(app, |label, cx| label.set_read_only(true, cx));
        assert!(!label.read(app).focus_handle().tab_stop);
    });
}

/// A code editor showing a file that cannot be edited looks like selectable
/// text, and is still a field: Tab reaches its text for selecting and copying.
#[gpui::test]
fn a_read_only_code_editor_stays_a_tab_stop(cx: &mut TestAppContext) {
    let editor = cx.update(|app| {
        app.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    tab: TextInputTab::Insert,
                    ..TextInputOptions::selectable_multiline()
                },
                cx,
            )
        })
    });
    cx.update(|app| {
        assert!(editor.read(app).focus_handle().tab_stop);
        editor.update(app, |editor, cx| editor.set_read_only(false, cx));
        assert!(editor.read(app).focus_handle().tab_stop);
        editor.update(app, |editor, cx| {
            editor.set_read_only(true, cx);
            editor.set_tab(TextInputTab::MoveFocus, cx);
        });
        assert!(
            !editor.read(app).focus_handle().tab_stop,
            "without Tab of its own it is selectable text"
        );
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

/// Two forms side by side, the second holding a nested form, then a code
/// editor: what one window shows when a prompt, a dialog and an editor are
/// all painted.
struct Forms {
    first: TabScope,
    second: TabScope,
    nested: TabScope,
    /// The first form: a field at index 1 painted first, one at index 0, then
    /// a button.
    first_late: Entity<TextInput>,
    first_early: Entity<TextInput>,
    first_button: FocusHandle,
    second_fields: [Entity<TextInput>; 2],
    nested_fields: [Entity<TextInput>; 2],
    editor: Entity<TextInput>,
}

impl Forms {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.activate();
        let mut input = |options: TextInputOptions, cx: &mut Context<Self>| {
            cx.new(|cx| TextInput::new(options, window, cx))
        };
        let plain = TextInputOptions::default;
        Self {
            first: TabScope::new(cx),
            second: TabScope::new(cx),
            nested: TabScope::new(cx),
            first_late: input(
                TextInputOptions {
                    tab_index: 1,
                    ..plain()
                },
                cx,
            ),
            first_early: input(plain(), cx),
            first_button: field(cx),
            second_fields: [input(plain(), cx), input(plain(), cx)],
            nested_fields: [input(plain(), cx), input(plain(), cx)],
            editor: input(
                TextInputOptions {
                    multiline: true,
                    tab: TextInputTab::Insert,
                    ..plain()
                },
                cx,
            ),
        }
    }
}

impl Render for Forms {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            // The window frame's, in the application.
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    |_, _, window, _| crate::text_selection_owner::install_reset(window),
                )
                .size_0(),
            )
            .child(
                div()
                    .tab_scope(&self.first)
                    .child(self.first_late.clone())
                    .child(self.first_early.clone())
                    .child(
                        div()
                            .debug_selector(|| "first_button".into())
                            .track_focus(&self.first_button)
                            .h(px(20.0))
                            .child("Button"),
                    ),
            )
            .child(
                div()
                    .tab_scope(&self.second)
                    .children(self.second_fields.iter().cloned())
                    .child(
                        div()
                            .tab_scope(&self.nested)
                            .children(self.nested_fields.iter().cloned()),
                    ),
            )
            .child(self.editor.clone())
    }
}

fn forms(cx: &mut TestAppContext) -> (Entity<Forms>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.bind_keys(key_bindings());
        install(cx);
    });
    let (view, cx) = cx.add_window_view(Forms::new);
    refresh_and_draw(cx);
    (view, cx)
}

fn handle_of(input: &Entity<TextInput>, cx: &App) -> FocusHandle {
    input.read(cx).focus_handle()
}

/// Forms painted in one window do not share their order: each wraps round its
/// own controls in both directions, whatever indices the other uses.
#[gpui::test]
fn forms_in_one_window_each_keep_their_own_order(cx: &mut TestAppContext) {
    let (view, cx) = forms(cx);

    // The first form, by index: early (0), the button (0), late (1).
    focus(cx, |app| handle_of(&view.read(app).first_early, app));
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).first_late, app),
        "the first form's last field, not the second form's",
    );
    press(cx, "tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).first_early, app),
        "the first form's first field",
    );
    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).first_button.clone(), "its button");
    press(cx, "tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).first_late, app),
        "its field at a later index, past the second form's fields",
    );

    // The second form wraps round its own fields: forward past the nested
    // form's and back to its first, never into the first form.
    focus(cx, |app| handle_of(&view.read(app).second_fields[1], app));
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).second_fields[0], app),
        "the second form's first field",
    );
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).nested_fields[1], app),
        "the second form's last field, in its nested form",
    );
}

/// A form inside a form keeps Tab for itself while focus is in it.
#[gpui::test]
fn a_nested_form_keeps_tab_inside_itself(cx: &mut TestAppContext) {
    let (view, cx) = forms(cx);
    focus(cx, |app| handle_of(&view.read(app).nested_fields[1], app));
    press(cx, "tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).nested_fields[0], app),
        "the nested form's first field",
    );
    press(cx, "shift-tab");
    assert_focused(
        cx,
        |app| handle_of(&view.read(app).nested_fields[1], app),
        "the nested form's last field",
    );
}

/// Only a press that no control took keeps the form as it was: a press on a
/// control is a press elsewhere to the field that held the selection.
#[gpui::test]
fn pressing_a_forms_control_releases_another_fields_selection(cx: &mut TestAppContext) {
    let (view, cx) = forms(cx);
    cx.update(|window, app| {
        let input = view.read(app).first_early.clone();
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            window.focus(&input.focus_handle(), cx);
            input.set_selected_range(0..5, false, window, cx);
        });
    });
    refresh_and_draw(cx);
    let button = cx
        .debug_bounds("first_button")
        .expect("the form's button")
        .center();
    cx.simulate_click(button, gpui::Modifiers::default());
    cx.run_until_parked();
    refresh_and_draw(cx);
    assert_focused(cx, |app| view.read(app).first_button.clone(), "the button");
    cx.update(|_, app| {
        let selection = view.read(app).first_early.read(app).selected_range();
        assert!(selection.is_empty(), "still selected: {selection:?}");
    });
}

/// A code editor types Tab, and is still no trap: Shift-Tab leaves it, and so
/// does Tab while it cannot be typed into.
#[gpui::test]
fn a_code_editor_can_be_left_with_the_keyboard(cx: &mut TestAppContext) {
    let (view, cx) = forms(cx);
    let editor = |app: &App| handle_of(&view.read(app).editor, app);

    focus(cx, editor);
    press(cx, "tab");
    assert_focused(cx, editor, "the editor, which typed the tab");
    press(cx, "shift-tab");
    cx.update(|window, app| {
        assert!(!editor(app).is_focused(window), "Shift-Tab leaves it");
        assert_eq!(view.read(app).editor.read(app).text(), "\t");
    });

    cx.update(|_, app| {
        let input = view.read(app).editor.clone();
        input.update(app, |input, cx| input.set_read_only(true, cx));
    });
    refresh_and_draw(cx);
    focus(cx, editor);
    press(cx, "tab");
    cx.update(|window, app| {
        assert!(
            !editor(app).is_focused(window),
            "Tab leaves a read-only editor"
        );
        assert_eq!(view.read(app).editor.read(app).text(), "\t");
    });
}

/// A dialog takes Tab from the rest of its window: with nothing focused, as
/// after the window was deactivated, and from a control it covers.
#[gpui::test]
fn a_modal_form_takes_tab_from_outside_it(cx: &mut TestAppContext) {
    let (view, cx) = window_with(cx, &[0, 0], Form::Modal);
    let first = |app: &App| view.read(app).handle(0, app);
    let last = |app: &App| view.read(app).handle(1, app);

    cx.update(|window, app| window.blur(app));
    press(cx, "tab");
    assert_focused(cx, first, "the dialog's first field");
    cx.update(|window, app| window.blur(app));
    press(cx, "shift-tab");
    assert_focused(cx, last, "the dialog's last field");

    focus(cx, |app| view.read(app).button.clone());
    press(cx, "tab");
    assert_focused(cx, first, "the dialog, not the control behind it");
    focus(cx, |app| view.read(app).before.read(app).focus_handle());
    press(cx, "shift-tab");
    assert_focused(cx, last, "the dialog, from a field behind it");
}

/// A dialog that is not shown, or shown in another window, takes nothing.
#[gpui::test]
fn a_modal_form_that_is_not_shown_leaves_tab_to_the_window(cx: &mut TestAppContext) {
    let hidden = cx.update(TabScope::modal);
    let (view, cx) = window_with(cx, &[0], Form::Plain);
    focus(cx, |app| view.read(app).before.read(app).focus_handle());
    press(cx, "tab");
    assert_focused(cx, |app| view.read(app).button.clone(), "the next control");
    cx.update(|window, app| window.blur(app));
    press(cx, "tab");
    assert_focused(
        cx,
        |app| view.read(app).before.read(app).focus_handle(),
        "the window's first field",
    );
    drop(hidden);
}

/// Making an element a form adds nothing to its layout: its children stay
/// where they were, gaps included.
#[gpui::test]
fn a_scope_leaves_its_elements_layout_alone(cx: &mut TestAppContext) {
    struct Gapped(Option<TabScope>);
    impl Render for Gapped {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let column = div().flex().flex_col().gap(px(8.0)).p(px(4.0));
            let column = match &self.0 {
                Some(scope) => column.tab_scope(scope),
                None => column,
            };
            column
                .debug_selector(|| "column".into())
                .child(
                    div()
                        .debug_selector(|| "first".into())
                        .h(px(10.0))
                        .child("First"),
                )
                .child(
                    div()
                        .debug_selector(|| "second".into())
                        .h(px(10.0))
                        .child("Second"),
                )
        }
    }
    let mut measure = |scoped: bool| {
        let (_, cx) = cx.add_window_view(move |_, cx| Gapped(scoped.then(|| TabScope::new(cx))));
        refresh_and_draw(cx);
        ["column", "first", "second"].map(|name| cx.debug_bounds(name).expect(name))
    };
    assert_eq!(measure(true), measure(false));
}

/// A dialog with nothing to Tab to still takes Tab: focus goes to the dialog
/// itself, where its own keys are, never to a control behind it.
#[gpui::test]
fn a_modal_form_without_fields_takes_focus_itself(cx: &mut TestAppContext) {
    let (view, cx) = window_with(cx, &[], Form::Modal);
    let container = |app: &App| view.read(app).scope.as_ref().unwrap().container().clone();

    cx.update(|window, app| window.blur(app));
    press(cx, "tab");
    assert_focused(cx, container, "the dialog");
    focus(cx, |app| view.read(app).button.clone());
    press(cx, "shift-tab");
    assert_focused(cx, container, "the dialog, from a control behind it");
}

/// Of two forms shown at once, Tab enters the dialog painted on top, whichever
/// was created first: with nothing focused, and from a field of the form
/// under it, a dialog itself or not.
#[gpui::test]
fn the_modal_form_painted_last_takes_tab(cx: &mut TestAppContext) {
    struct Stacked {
        // Created in this order; `under` is painted first.
        over: TabScope,
        under: TabScope,
        over_field: Entity<TextInput>,
        under_field: Entity<TextInput>,
    }
    impl Render for Stacked {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .child(div().tab_scope(&self.under).child(self.under_field.clone()))
                .child(div().tab_scope(&self.over).child(self.over_field.clone()))
        }
    }
    cx.update(|cx| {
        cx.bind_keys(key_bindings());
        install(cx);
    });
    for under_is_modal in [true, false] {
        let (view, cx) = cx.add_window_view(|window, cx| {
            window.activate();
            let mut input = || cx.new(|cx| TextInput::new(TextInputOptions::default(), window, cx));
            let (over_field, under_field) = (input(), input());
            Stacked {
                over: TabScope::modal(cx),
                under: if under_is_modal {
                    TabScope::modal(cx)
                } else {
                    TabScope::new(cx)
                },
                over_field,
                under_field,
            }
        });
        refresh_and_draw(cx);
        let over = |app: &App| handle_of(&view.read(app).over_field, app);
        press(cx, "tab");
        assert_focused(cx, over, "the dialog on top");
        for keys in ["tab", "shift-tab"] {
            focus(cx, |app| handle_of(&view.read(app).under_field, app));
            press(cx, keys);
            assert_focused(cx, over, "the dialog on top, from the form under it");
        }
    }
}

/// Dialogs make a modal scope each time they open. The ones that closed are
/// forgotten as the next registers.
#[gpui::test]
fn closed_modal_forms_are_forgotten(cx: &mut TestAppContext) {
    cx.update(|app| {
        let kept = TabScope::modal(app);
        for _ in 0..50 {
            drop(TabScope::modal(app));
        }
        let live = app.global::<ModalScopes>().0.len();
        assert!(live <= 2, "{live} registered for one live dialog");
        drop(kept);
    });
}
