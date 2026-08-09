use super::{PickerPrompt, PickerPromptItem};
use crate::kit::TextInput;
use crate::theme::AppTheme;
use crate::view::tooltip_host::TooltipHost;
use gpui::{ClickEvent, Entity, ScrollHandle, SharedString, WeakEntity, Window, px};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BranchRefKind {
    Head,
    Branch,
    Tag,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchRefPickerItem {
    name: String,
    kind: BranchRefKind,
}

impl BranchRefPickerItem {
    pub fn head() -> Self {
        Self {
            name: "HEAD".to_string(),
            kind: BranchRefKind::Head,
        }
    }

    pub fn branch(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: BranchRefKind::Branch,
        }
    }

    pub fn tag(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: BranchRefKind::Tag,
        }
    }

    fn icon(&self) -> &'static str {
        match self.kind {
            BranchRefKind::Head | BranchRefKind::Branch => "icons/git_branch.svg",
            BranchRefKind::Tag => "icons/tag.svg",
        }
    }
}

pub struct BranchRefPicker {
    query_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    items: Vec<BranchRefPickerItem>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    empty_text: SharedString,
    max_height: gpui::Pixels,
    selected_index: Option<usize>,
    marked_name: Option<String>,
    select_on_mouse_down: bool,
}

impl BranchRefPicker {
    pub fn new(
        query_input: Entity<TextInput>,
        scroll_handle: ScrollHandle,
        items: Vec<BranchRefPickerItem>,
    ) -> Self {
        Self {
            query_input,
            scroll_handle,
            items,
            tooltip_host: None,
            empty_text: "No matches".into(),
            max_height: px(240.0),
            selected_index: None,
            marked_name: None,
            select_on_mouse_down: false,
        }
    }

    pub fn tooltip_host(mut self, tooltip_host: WeakEntity<TooltipHost>) -> Self {
        self.tooltip_host = Some(tooltip_host);
        self
    }

    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty_text = text.into();
        self
    }

    pub fn max_height(mut self, height: gpui::Pixels) -> Self {
        self.max_height = height;
        self
    }

    pub fn selected_index(mut self, selected_index: Option<usize>) -> Self {
        self.selected_index = selected_index;
        self
    }

    pub fn marked_name(mut self, name: Option<impl Into<String>>) -> Self {
        self.marked_name = name.map(Into::into);
        self
    }

    pub fn select_on_mouse_down(mut self) -> Self {
        self.select_on_mouse_down = true;
        self
    }

    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<crate::ui_scale::UiScale>,
        cx: &mut gpui::Context<V>,
        on_select: impl Fn(&mut V, String, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> gpui::Div {
        self.query_input.update(cx, |input, cx| {
            input.set_chromeless(true, cx);
            input.set_leading_icon(Some("icons/git_branch.svg"), cx);
        });

        let marked_index = self.marked_name.as_ref().and_then(|marked| {
            self.items
                .iter()
                .position(|item| item.name.as_str() == marked)
        });
        let prompt_items = self
            .items
            .iter()
            .map(|item| PickerPromptItem::plain(item.name.clone()).icon(item.icon()))
            .collect::<Vec<_>>();
        let items = self.items;

        let mut picker = PickerPrompt::new(self.query_input, self.scroll_handle)
            .items(prompt_items)
            .empty_text(self.empty_text)
            .max_height(self.max_height)
            .selected_index(self.selected_index)
            .marked_index(marked_index)
            .leading_icon("icons/git_branch.svg")
            .selected_hint("Enter")
            .accent_selection()
            .attached_list_surface();
        if let Some(tooltip_host) = self.tooltip_host {
            picker = picker.tooltip_host(tooltip_host);
        }
        if self.select_on_mouse_down {
            picker = picker.select_on_mouse_down();
        }

        picker.render(theme, ui_scale, cx, move |this, ix, event, window, cx| {
            if let Some(item) = items.get(ix) {
                on_select(this, item.name.clone(), event, window, cx);
            }
        })
    }
}
