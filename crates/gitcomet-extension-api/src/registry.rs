//! Collecting, validating, and freezing extension declarations.

use crate::contributions::{
    BottomPanelDescriptor, ChromeDescriptor, CloseGuard, CommandDescriptor, DetailsTabDescriptor,
    MenuLocation, RepositoryEntryGate, RepositoryViewDescriptor, SettingsPageDescriptor,
    SidebarSectionDescriptor, StatusItemDescriptor, WindowGateDescriptor,
};
use crate::id::{ContributionId, ExtensionId};
use crate::{Extension, HistoryAnnotator};
use gitcomet_ui_kit::gpui::{Keystroke, SharedString};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

/// A declaration the host refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationError {
    pub extension: String,
    pub message: String,
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.extension, self.message)
    }
}

impl std::error::Error for RegistrationError {}

#[derive(Clone)]
pub struct KeyBindingDeclaration {
    pub keystrokes: SharedString,
    pub command: ContributionId,
    /// The GPUI key context the binding applies in; `None` for the window.
    pub context: Option<SharedString>,
}

#[derive(Clone)]
pub struct MenuItemDeclaration {
    pub location: MenuLocation,
    pub command: ContributionId,
}

#[cfg(test)]
mod key_context_contract {
    use gitcomet_ui_kit::gpui::{KeyBindingContextPredicate, KeyContext};

    #[test]
    fn not_excludes_text_input_at_every_depth() {
        let predicate = KeyBindingContextPredicate::parse("RepositoryView && !TextInput").unwrap();
        let view = KeyContext::parse("RepositoryView").unwrap();
        let input = KeyContext::parse("TextInput").unwrap();
        assert!(predicate.eval(std::slice::from_ref(&view)));
        assert!(!predicate.eval(&[view.clone(), input.clone()]));
        assert!(!predicate.eval(&[input, view]));
    }
}

/// Collects one extension's declarations. Invalid names are recorded and
/// reported when the host builds the [`Registry`].
pub struct Registrar {
    extension: ExtensionId,
    repository_views: Vec<(ContributionId, RepositoryViewDescriptor)>,
    bottom_panels: Vec<(ContributionId, BottomPanelDescriptor)>,
    details_tabs: Vec<(ContributionId, DetailsTabDescriptor)>,
    sidebar_sections: Vec<(ContributionId, SidebarSectionDescriptor)>,
    history_annotators: Vec<(ContributionId, HistoryAnnotator)>,
    sidebar_providers: Vec<(ContributionId, crate::SidebarProvider)>,
    edition_strip: Vec<(ContributionId, ChromeDescriptor)>,
    title_bar_brand: Vec<(ContributionId, ChromeDescriptor)>,
    status_items: Vec<(ContributionId, StatusItemDescriptor)>,
    settings_pages: Vec<(ContributionId, SettingsPageDescriptor)>,
    commands: Vec<(ContributionId, CommandDescriptor)>,
    key_bindings: Vec<KeyBindingDeclaration>,
    menu_items: Vec<MenuItemDeclaration>,
    assets: Vec<(String, &'static [u8])>,
    entry_gates: Vec<(ContributionId, RepositoryEntryGate)>,
    window_gates: Vec<(ContributionId, WindowGateDescriptor)>,
    close_guards: Vec<(ContributionId, CloseGuard)>,
    errors: Vec<String>,
}

impl Registrar {
    fn new(extension: ExtensionId) -> Self {
        Self {
            extension,
            repository_views: Vec::new(),
            bottom_panels: Vec::new(),
            details_tabs: Vec::new(),
            sidebar_sections: Vec::new(),
            history_annotators: Vec::new(),
            sidebar_providers: Vec::new(),
            edition_strip: Vec::new(),
            title_bar_brand: Vec::new(),
            status_items: Vec::new(),
            settings_pages: Vec::new(),
            commands: Vec::new(),
            key_bindings: Vec::new(),
            menu_items: Vec::new(),
            assets: Vec::new(),
            entry_gates: Vec::new(),
            window_gates: Vec::new(),
            close_guards: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn extension(&self) -> &ExtensionId {
        &self.extension
    }

    fn id(&mut self, local: impl Into<Cow<'static, str>>) -> Option<ContributionId> {
        match self.extension.contribution(local) {
            Ok(id) => Some(id),
            Err(error) => {
                self.errors.push(error.to_string());
                None
            }
        }
    }

    pub fn window_gate(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: WindowGateDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.window_gates.push((id, descriptor));
        }
        self
    }

    pub fn repository_view(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: RepositoryViewDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.repository_views.push((id, descriptor));
        }
        self
    }

    pub fn bottom_panel(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: BottomPanelDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.bottom_panels.push((id, descriptor));
        }
        self
    }

    pub fn details_tab(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: DetailsTabDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.details_tabs.push((id, descriptor));
        }
        self
    }

    pub fn sidebar_provider(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        provider: crate::SidebarProvider,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.sidebar_providers.push((id, provider));
        }
        self
    }

    pub fn history_annotator(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        provider: HistoryAnnotator,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.history_annotators.push((id, provider));
        }
        self
    }

    pub fn sidebar_section(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: SidebarSectionDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.sidebar_sections.push((id, descriptor));
        }
        self
    }

    pub fn edition_strip(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: ChromeDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.edition_strip.push((id, descriptor));
        }
        self
    }

    pub fn title_bar_brand(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: ChromeDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.title_bar_brand.push((id, descriptor));
        }
        self
    }

    pub fn status_item(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: StatusItemDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.status_items.push((id, descriptor));
        }
        self
    }

    pub fn settings_page(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: SettingsPageDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.settings_pages.push((id, descriptor));
        }
        self
    }

    pub fn command(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        descriptor: CommandDescriptor,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.commands.push((id, descriptor));
        }
        self
    }

    /// Binds `keystrokes` (GPUI syntax, e.g. `secondary-shift-r`) to one of
    /// this extension's commands.
    pub fn key_binding(
        &mut self,
        keystrokes: impl Into<SharedString>,
        command: impl Into<Cow<'static, str>>,
        context: Option<SharedString>,
    ) -> &mut Self {
        if let Some(command) = self.id(command) {
            self.key_bindings.push(KeyBindingDeclaration {
                keystrokes: keystrokes.into(),
                command,
                context,
            });
        }
        self
    }

    /// Lists one of this extension's commands in `location`.
    pub fn menu_item(
        &mut self,
        location: MenuLocation,
        command: impl Into<Cow<'static, str>>,
    ) -> &mut Self {
        if let Some(command) = self.id(command) {
            self.menu_items
                .push(MenuItemDeclaration { location, command });
        }
        self
    }

    /// Serves `bytes` at `extensions/<extension id>/<path>` through the
    /// window's asset source (for `svg()`/`img()` paths).
    pub fn asset(&mut self, path: &str, bytes: &'static [u8]) -> &mut Self {
        let clean = !path.is_empty()
            && !path.starts_with('/')
            && path.split('/').all(|part| !part.is_empty() && part != "..");
        if clean {
            self.assets
                .push((format!("extensions/{}/{path}", self.extension), bytes));
        } else {
            self.errors
                .push(format!("asset path {path:?} must be relative without `..`"));
        }
        self
    }

    pub fn repository_entry_gate(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        gate: RepositoryEntryGate,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.entry_gates.push((id, gate));
        }
        self
    }

    pub fn close_guard(
        &mut self,
        local: impl Into<Cow<'static, str>>,
        guard: CloseGuard,
    ) -> &mut Self {
        if let Some(id) = self.id(local) {
            self.close_guards.push((id, guard));
        }
        self
    }
}

/// The validated, frozen set of every extension's contributions, in
/// registration order.
#[derive(Clone, Default)]
pub struct Registry {
    instances: Vec<Rc<dyn Extension>>,
    extensions: Vec<ExtensionId>,
    repository_views: Vec<(ContributionId, RepositoryViewDescriptor)>,
    bottom_panels: Vec<(ContributionId, BottomPanelDescriptor)>,
    details_tabs: Vec<(ContributionId, DetailsTabDescriptor)>,
    sidebar_sections: Vec<(ContributionId, SidebarSectionDescriptor)>,
    history_annotators: Vec<(ContributionId, HistoryAnnotator)>,
    sidebar_providers: Vec<(ContributionId, crate::SidebarProvider)>,
    edition_strip: Vec<(ContributionId, ChromeDescriptor)>,
    title_bar_brand: Vec<(ContributionId, ChromeDescriptor)>,
    status_items: Vec<(ContributionId, StatusItemDescriptor)>,
    settings_pages: Vec<(ContributionId, SettingsPageDescriptor)>,
    commands: Vec<(ContributionId, CommandDescriptor)>,
    key_bindings: Vec<KeyBindingDeclaration>,
    menu_items: Vec<MenuItemDeclaration>,
    assets: Vec<(String, &'static [u8])>,
    entry_gates: Vec<(ContributionId, RepositoryEntryGate)>,
    window_gates: Vec<(ContributionId, WindowGateDescriptor)>,
    close_guards: Vec<(ContributionId, CloseGuard)>,
}

impl Registry {
    /// Registers `extensions` in order and validates the result: extension
    /// ids, contribution ids per kind, and asset paths must be unique, and key
    /// bindings and menu items must name a declared command.
    pub fn build(extensions: Vec<Box<dyn Extension>>) -> Result<Self, Vec<RegistrationError>> {
        let mut registry = Self::default();
        let mut errors = Vec::new();
        let mut error = |extension: &ExtensionId, message: String| {
            errors.push(RegistrationError {
                extension: extension.to_string(),
                message,
            })
        };
        let mut seen_extensions = BTreeSet::new();
        for extension in extensions {
            let extension: Rc<dyn Extension> = Rc::from(extension);
            let id = extension.id();
            if !seen_extensions.insert(id.clone()) {
                error(&id, "registered twice".to_string());
                continue;
            }
            let mut registrar = Registrar::new(id.clone());
            extension.register(&mut registrar);
            for message in std::mem::take(&mut registrar.errors) {
                error(&id, message);
            }
            registry.extensions.push(id.clone());
            registry.instances.push(extension);
            registry.repository_views.extend(registrar.repository_views);
            registry.bottom_panels.extend(registrar.bottom_panels);
            registry.details_tabs.extend(registrar.details_tabs);
            registry.sidebar_sections.extend(registrar.sidebar_sections);
            registry
                .history_annotators
                .extend(registrar.history_annotators);
            registry
                .sidebar_providers
                .extend(registrar.sidebar_providers);
            registry.edition_strip.extend(registrar.edition_strip);
            registry.title_bar_brand.extend(registrar.title_bar_brand);
            registry.status_items.extend(registrar.status_items);
            registry.settings_pages.extend(registrar.settings_pages);
            registry.commands.extend(registrar.commands);
            registry.key_bindings.extend(registrar.key_bindings);
            registry.menu_items.extend(registrar.menu_items);
            registry.assets.extend(registrar.assets);
            registry.entry_gates.extend(registrar.entry_gates);
            registry.window_gates.extend(registrar.window_gates);
            registry.close_guards.extend(registrar.close_guards);
        }

        fn duplicates<'a>(
            ids: impl Iterator<Item = &'a ContributionId>,
        ) -> Vec<&'a ContributionId> {
            let mut seen = BTreeSet::new();
            ids.filter(|id| !seen.insert(*id)).collect()
        }
        for (kind, dupes) in [
            (
                "window gate",
                duplicates(registry.window_gates.iter().map(|(id, _)| id)),
            ),
            (
                "repository view",
                duplicates(registry.repository_views.iter().map(|(id, _)| id)),
            ),
            (
                "bottom panel",
                duplicates(registry.bottom_panels.iter().map(|(id, _)| id)),
            ),
            (
                "details tab",
                duplicates(registry.details_tabs.iter().map(|(id, _)| id)),
            ),
            (
                "sidebar section",
                duplicates(registry.sidebar_sections.iter().map(|(id, _)| id)),
            ),
            (
                "history annotator",
                duplicates(registry.history_annotators.iter().map(|(id, _)| id)),
            ),
            (
                "sidebar provider",
                duplicates(registry.sidebar_providers.iter().map(|(id, _)| id)),
            ),
            (
                "status item",
                duplicates(registry.status_items.iter().map(|(id, _)| id)),
            ),
            (
                "settings page",
                duplicates(registry.settings_pages.iter().map(|(id, _)| id)),
            ),
            (
                "command",
                duplicates(registry.commands.iter().map(|(id, _)| id)),
            ),
            (
                "entry gate",
                duplicates(registry.entry_gates.iter().map(|(id, _)| id)),
            ),
            (
                "close guard",
                duplicates(registry.close_guards.iter().map(|(id, _)| id)),
            ),
        ] {
            for id in dupes {
                error(id.extension(), format!("{kind} {id} is declared twice"));
            }
        }
        let mut asset_paths = BTreeSet::new();
        for (path, _) in &registry.assets {
            if !asset_paths.insert(path) {
                let extension = registry
                    .extensions
                    .iter()
                    .find(|id| path.starts_with(&format!("extensions/{id}/")))
                    .cloned();
                if let Some(extension) = extension {
                    error(&extension, format!("asset {path} is declared twice"));
                }
            }
        }
        if registry.edition_strip.len() > 1 {
            for (id, _) in registry.edition_strip.iter().skip(1) {
                error(
                    id.extension(),
                    "edition_strip has more than one provider".to_string(),
                );
            }
        }
        if registry.title_bar_brand.len() > 1 {
            for (id, _) in registry.title_bar_brand.iter().skip(1) {
                error(
                    id.extension(),
                    "title_bar_brand has more than one provider".to_string(),
                );
            }
        }
        let commands: BTreeSet<&ContributionId> =
            registry.commands.iter().map(|(id, _)| id).collect();
        for binding in &registry.key_bindings {
            if !commands.contains(&binding.command) {
                error(
                    binding.command.extension(),
                    format!("key binding names unknown command {}", binding.command),
                );
            }
            for keystroke in binding.keystrokes.split_whitespace() {
                if Keystroke::parse(keystroke).is_err() {
                    error(
                        binding.command.extension(),
                        format!("key binding {:?} does not parse", binding.keystrokes),
                    );
                }
            }
        }
        for item in &registry.menu_items {
            if !commands.contains(&item.command) {
                error(
                    item.command.extension(),
                    format!("menu item names unknown command {}", item.command),
                );
            }
        }

        if errors.is_empty() {
            Ok(registry)
        } else {
            Err(errors)
        }
    }

    /// Nothing registered: the host allocates no extension state or tasks.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    pub fn extensions(&self) -> &[ExtensionId] {
        &self.extensions
    }

    #[doc(hidden)]
    pub fn instances(&self) -> &[Rc<dyn Extension>] {
        &self.instances
    }

    pub fn window_gates(&self) -> &[(ContributionId, WindowGateDescriptor)] {
        &self.window_gates
    }

    pub fn repository_views(&self) -> &[(ContributionId, RepositoryViewDescriptor)] {
        &self.repository_views
    }

    pub fn bottom_panels(&self) -> &[(ContributionId, BottomPanelDescriptor)] {
        &self.bottom_panels
    }

    pub fn details_tabs(&self) -> &[(ContributionId, DetailsTabDescriptor)] {
        &self.details_tabs
    }

    pub fn sidebar_providers(&self) -> &[(ContributionId, crate::SidebarProvider)] {
        &self.sidebar_providers
    }

    pub fn history_annotators(&self) -> &[(ContributionId, HistoryAnnotator)] {
        &self.history_annotators
    }

    pub fn sidebar_sections(&self) -> &[(ContributionId, SidebarSectionDescriptor)] {
        &self.sidebar_sections
    }

    pub fn edition_strip(&self) -> Option<&ChromeDescriptor> {
        self.edition_strip.first().map(|(_, descriptor)| descriptor)
    }

    pub fn title_bar_brand(&self) -> Option<&ChromeDescriptor> {
        self.title_bar_brand
            .first()
            .map(|(_, descriptor)| descriptor)
    }

    pub fn status_items(&self) -> &[(ContributionId, StatusItemDescriptor)] {
        &self.status_items
    }

    pub fn settings_pages(&self) -> &[(ContributionId, SettingsPageDescriptor)] {
        &self.settings_pages
    }

    pub fn commands(&self) -> &[(ContributionId, CommandDescriptor)] {
        &self.commands
    }

    pub fn command(&self, id: &ContributionId) -> Option<&CommandDescriptor> {
        self.commands
            .iter()
            .find_map(|(command, descriptor)| (command == id).then_some(descriptor))
    }

    pub fn key_bindings(&self) -> &[KeyBindingDeclaration] {
        &self.key_bindings
    }

    pub fn menu_items(&self, location: MenuLocation) -> impl Iterator<Item = &MenuItemDeclaration> {
        self.menu_items
            .iter()
            .filter(move |item| item.location == location)
    }

    pub fn asset(&self, path: &str) -> Option<&'static [u8]> {
        self.assets
            .iter()
            .find_map(|(asset, bytes)| (asset == path).then_some(*bytes))
    }

    pub fn asset_paths(&self) -> impl Iterator<Item = &str> {
        self.assets.iter().map(|(path, _)| path.as_str())
    }

    /// Every asset as `(path, bytes)`, in registration order.
    pub fn assets(&self) -> &[(String, &'static [u8])] {
        &self.assets
    }

    pub fn entry_gates(&self) -> &[(ContributionId, RepositoryEntryGate)] {
        &self.entry_gates
    }

    pub fn close_guards(&self) -> &[(ContributionId, CloseGuard)] {
        &self.close_guards
    }
}

#[cfg(test)]
mod tests {
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
}
