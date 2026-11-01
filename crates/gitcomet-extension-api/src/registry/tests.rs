use super::*;
use crate::contributions::{CommandContext, GateDecision};
use gitcomet_ui_kit::gpui::{App, Window};

struct Fixture {
    id: &'static str,
    register: fn(&mut Registrar),
}

impl Extension for Fixture {
    fn id(&self) -> ExtensionId {
        ExtensionId::new(self.id).unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        (self.register)(registrar)
    }
}

fn command(label: &'static str) -> CommandDescriptor {
    CommandDescriptor {
        label: label.into(),
        category: "Example".into(),
        keywords: "".into(),
        requires_repository: false,
        repository_requirement: gitcomet_core::domain::RepositoryRequirement::Repository,
        run: Rc::new(|_: CommandContext, _: &mut Window, _: &mut App| {}),
    }
}

fn build(fixtures: Vec<Fixture>) -> Result<Registry, Vec<RegistrationError>> {
    let extensions: Vec<Box<dyn Extension>> = fixtures
        .into_iter()
        .map(|fixture| Box::new(fixture) as Box<dyn Extension>)
        .collect();
    Registry::build(extensions)
}

#[test]
fn contributions_keep_registration_order_across_extensions() {
    let registry = build(vec![
        Fixture {
            id: "com.example.a",
            register: |r| {
                r.command("first", command("First"))
                    .command("second", command("Second"))
                    .key_binding("secondary-shift-f", "first", None)
                    .menu_item(MenuLocation::Application, "second")
                    .asset("icons/a.svg", b"<svg/>");
            },
        },
        Fixture {
            id: "com.example.b",
            register: |r| {
                r.command("first", command("B First"))
                    .repository_entry_gate("gate", Rc::new(|_, _| GateDecision::Allow));
            },
        },
    ])
    .expect("valid registrations");
    let labels: Vec<&str> = registry
        .commands()
        .iter()
        .map(|(_, command)| command.label.as_ref())
        .collect();
    assert_eq!(labels, ["First", "Second", "B First"]);
    assert_eq!(
        registry.asset("extensions/com.example.a/icons/a.svg"),
        Some(&b"<svg/>"[..])
    );
    assert_eq!(registry.menu_items(MenuLocation::Application).count(), 1);
    assert_eq!(registry.entry_gates().len(), 1);
    assert!(!registry.is_empty());
    assert!(Registry::default().is_empty());
}

#[test]
fn duplicates_unknown_commands_and_bad_names_are_rejected_before_install() {
    let errors = build(vec![
        Fixture {
            id: "com.example.a",
            register: |r| {
                r.command("run", command("Run"))
                    .command("run", command("Run again"))
                    .key_binding("secondary-shift-x", "missing", None)
                    .key_binding("a-b", "run", None)
                    .menu_item(MenuLocation::RepositoryTab, "missing")
                    .asset("a.svg", b"")
                    .asset("a.svg", b"")
                    .asset("../escape.svg", b"")
                    .command("Bad Name", command("Bad"));
            },
        },
        Fixture {
            id: "com.example.a",
            register: |_| {},
        },
    ])
    .err()
    .expect("invalid registrations");
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    for expected in [
        "registered twice",
        "command com.example.a/run is declared twice",
        "unknown command com.example.a/missing",
        "does not parse",
        "asset extensions/com.example.a/a.svg is declared twice",
        "must be relative",
        "contribution name \"Bad Name\"",
    ] {
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "missing {expected:?} in {messages:#?}"
        );
    }
}

#[test]
fn malformed_key_contexts_are_aggregated_with_other_registration_errors() {
    let errors = build(vec![
        Fixture {
            id: "com.example.a",
            register: |r| {
                r.command("run", command("Run"))
                    .key_binding("secondary-r", "run", Some("(".into()))
                    .key_binding("a-b", "missing", Some("RepositoryView &&".into()));
            },
        },
        Fixture {
            id: "com.example.b",
            register: |r| {
                r.command("run", command("Run")).key_binding(
                    "secondary-r",
                    "run",
                    Some("RepositoryView >".into()),
                );
            },
        },
    ])
    .err()
    .expect("invalid contexts must prevent keymap installation");
    assert_eq!(
        errors,
        [
            RegistrationError {
                extension: "com.example.a".into(),
                message: "key binding context \"(\" does not parse".into(),
            },
            RegistrationError {
                extension: "com.example.a".into(),
                message: "key binding names unknown command com.example.a/missing".into(),
            },
            RegistrationError {
                extension: "com.example.a".into(),
                message: "key binding \"a-b\" does not parse".into(),
            },
            RegistrationError {
                extension: "com.example.a".into(),
                message: "key binding context \"RepositoryView &&\" does not parse".into(),
            },
            RegistrationError {
                extension: "com.example.b".into(),
                message: "key binding context \"RepositoryView >\" does not parse".into(),
            },
        ]
    );
}

#[test]
fn valid_compound_key_contexts_keep_declaration_and_precedence_order() {
    let registry = build(vec![
        Fixture {
            id: "com.example.a",
            register: |r| {
                r.command("run", command("Run"))
                    .key_binding("secondary-r", "run", None)
                    .key_binding(
                        "secondary-r",
                        "run",
                        Some("RepositoryView && (!TextInput || mode == review)".into()),
                    );
            },
        },
        Fixture {
            id: "com.example.b",
            register: |r| {
                r.command("run", command("Run")).key_binding(
                    "secondary-r",
                    "run",
                    Some("Window > RepositoryView".into()),
                );
            },
        },
    ])
    .expect("the GPUI parser accepts compound contexts");
    let declarations: Vec<_> = registry
        .key_bindings()
        .iter()
        .map(|binding| (binding.command.to_string(), binding.context.as_deref()))
        .collect();
    assert_eq!(
        declarations,
        [
            ("com.example.a/run".into(), None),
            (
                "com.example.a/run".into(),
                Some("RepositoryView && (!TextInput || mode == review)"),
            ),
            ("com.example.b/run".into(), Some("Window > RepositoryView"),),
        ]
    );
}

#[test]
fn bar_placement_defaults_and_overrides_are_independent() {
    let registry = build(vec![Fixture {
        id: "com.example.layout",
        register: |registrar| {
            registrar
                .place_bar_item(BuiltinBarItem::Terminal, BarItemLocation::StatusBarEnd)
                .place_bar_item(
                    BuiltinBarItem::DetailsToggle,
                    BarItemLocation::ActionBarStart,
                );
        },
    }])
    .unwrap();
    assert_eq!(
        Registry::default().bar_item_location(BuiltinBarItem::Terminal),
        BarItemLocation::ActionBarEnd
    );
    assert_eq!(
        registry.bar_item_location(BuiltinBarItem::Terminal),
        BarItemLocation::StatusBarEnd
    );
    assert_eq!(
        registry.bar_item_location(BuiltinBarItem::DetailsToggle),
        BarItemLocation::ActionBarStart
    );
    assert_eq!(
        registry.bar_item_location(BuiltinBarItem::Stash),
        BarItemLocation::ActionBarEnd
    );
    assert_eq!(registry.bar_item_locations().len(), 2);
}

#[test]
fn bar_placement_conflicts_are_rejected_with_the_declaring_extension() {
    let errors = build(vec![
        Fixture {
            id: "com.example.a",
            register: |registrar| {
                registrar
                    .place_bar_item(BuiltinBarItem::Terminal, BarItemLocation::StatusBarEnd)
                    .place_bar_item(BuiltinBarItem::Terminal, BarItemLocation::StatusBarStart);
            },
        },
        Fixture {
            id: "com.example.b",
            register: |registrar| {
                registrar.place_bar_item(BuiltinBarItem::Terminal, BarItemLocation::ActionBarEnd);
            },
        },
    ])
    .err()
    .expect("conflicting placements");
    assert_eq!(
        errors,
        [
            RegistrationError {
                extension: "com.example.a".into(),
                message: "bar item Terminal is placed twice".into(),
            },
            RegistrationError {
                extension: "com.example.b".into(),
                message: "bar item Terminal is placed twice".into(),
            },
        ]
    );
}
