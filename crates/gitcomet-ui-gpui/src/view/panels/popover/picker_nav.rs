use super::components;

/// Resolves only the selected payload, without cloning the filtered list.
pub(super) trait PickerNavRows: 'static {
    type Item: Clone;
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<Self::Item>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Clone + 'static> PickerNavRows for Vec<T> {
    type Item = T;
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, index: usize) -> Option<T> {
        self.as_slice().get(index).cloned()
    }
}

pub(super) struct IndexedNavRows<T> {
    pub len: usize,
    pub resolve: Box<dyn Fn(usize) -> Option<T>>,
}

impl<T: Clone + 'static> PickerNavRows for IndexedNavRows<T> {
    type Item = T;
    fn len(&self) -> usize {
        self.len
    }
    fn get(&self, index: usize) -> Option<T> {
        (self.resolve)(index)
    }
}

#[derive(Default)]
pub(super) struct PickerNavKeys {
    pub escape: bool,
    pub arrow_up: bool,
    pub arrow_down: bool,
    pub tab: bool,
    pub shift_tab: bool,
    pub enter: bool,
    pub home: bool,
    pub end: bool,
    pub page_up: bool,
    pub page_down: bool,
}

impl PickerNavKeys {
    pub(super) fn take(input: &mut components::TextInput) -> Self {
        Self {
            escape: input.take_escape_pressed(),
            arrow_up: input.take_arrow_up_pressed(),
            arrow_down: input.take_arrow_down_pressed(),
            tab: input.take_tab_pressed(),
            shift_tab: input.take_shift_tab_pressed(),
            enter: input.take_enter_pressed(),
            home: input.take_document_home_pressed(),
            end: input.take_document_end_pressed(),
            page_up: input.take_page_up_pressed(),
            page_down: input.take_page_down_pressed(),
        }
    }
}

pub(super) enum PickerNavOutcome {
    Escape,
    Navigated,
    Enter,
    Idle,
}

/// Updates `selected` based on navigation keys and returns what happened.
/// Does not scroll or notify — the caller owns both.
///
/// Input notifications can coalesce, leaving navigation and Enter set at the
/// same time. In that case the selection moves first and Enter wins as the
/// outcome, so the caller submits the row the user just moved to.
pub(super) fn handle_picker_nav(
    keys: &PickerNavKeys,
    selected: &mut Option<usize>,
    count: usize,
    page_rows: usize,
) -> PickerNavOutcome {
    if keys.escape {
        return PickerNavOutcome::Escape;
    }
    if count > 0 && (keys.home || keys.end || keys.page_up || keys.page_down) {
        *selected = Some(if keys.home {
            0
        } else if keys.end {
            count - 1
        } else if keys.page_up {
            selected.map_or(count - 1, |ix| {
                ix.min(count - 1).saturating_sub(page_rows.max(1))
            })
        } else {
            selected.map_or(0, |ix| ix.saturating_add(page_rows.max(1)).min(count - 1))
        });
        return if keys.enter {
            PickerNavOutcome::Enter
        } else {
            PickerNavOutcome::Navigated
        };
    }
    if keys.arrow_up || keys.shift_tab {
        *selected = Some(match *selected {
            Some(ix) if ix > 0 => ix - 1,
            _ if count > 0 => count - 1,
            _ if keys.enter => return PickerNavOutcome::Enter,
            _ => return PickerNavOutcome::Idle,
        });
        return if keys.enter {
            PickerNavOutcome::Enter
        } else {
            PickerNavOutcome::Navigated
        };
    }
    if keys.arrow_down || keys.tab {
        *selected = Some(match *selected {
            Some(ix) if ix + 1 < count => ix + 1,
            _ if count > 0 => 0,
            _ if keys.enter => return PickerNavOutcome::Enter,
            _ => return PickerNavOutcome::Idle,
        });
        return if keys.enter {
            PickerNavOutcome::Enter
        } else {
            PickerNavOutcome::Navigated
        };
    }
    if keys.enter {
        return PickerNavOutcome::Enter;
    }
    PickerNavOutcome::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> PickerNavKeys {
        PickerNavKeys {
            escape: false,
            arrow_up: false,
            arrow_down: false,
            tab: false,
            shift_tab: false,
            enter: false,
            ..Default::default()
        }
    }

    #[test]
    fn coalesced_navigation_and_enter_submits_the_new_selection() {
        let mut keys = keys();
        keys.arrow_down = true;
        keys.enter = true;
        let mut selected = None;

        assert!(matches!(
            handle_picker_nav(&keys, &mut selected, 2, 10),
            PickerNavOutcome::Enter
        ));
        assert_eq!(selected, Some(0));
    }

    #[test]
    fn coalesced_navigation_and_enter_still_submits_an_empty_list() {
        let mut keys = keys();
        keys.arrow_down = true;
        keys.enter = true;
        let mut selected = None;

        assert!(matches!(
            handle_picker_nav(&keys, &mut selected, 0, 10),
            PickerNavOutcome::Enter
        ));
        assert_eq!(selected, None);
    }
    #[test]
    fn page_navigation_clamps_and_home_end_jump() {
        let mut selected = Some(19);
        let page_down = PickerNavKeys {
            page_down: true,
            ..Default::default()
        };
        handle_picker_nav(&page_down, &mut selected, 25, 10);
        assert_eq!(selected, Some(24));
        let page_up = PickerNavKeys {
            page_up: true,
            ..Default::default()
        };
        handle_picker_nav(&page_up, &mut selected, 25, 10);
        assert_eq!(selected, Some(14));
        handle_picker_nav(
            &PickerNavKeys {
                home: true,
                ..Default::default()
            },
            &mut selected,
            25,
            10,
        );
        assert_eq!(selected, Some(0));
        handle_picker_nav(&page_up, &mut selected, 25, 10);
        assert_eq!(selected, Some(0));
        handle_picker_nav(
            &PickerNavKeys {
                end: true,
                enter: true,
                ..Default::default()
            },
            &mut selected,
            25,
            10,
        );
        assert_eq!(selected, Some(24));
        selected = None;
        handle_picker_nav(&page_up, &mut selected, 25, 10);
        assert_eq!(selected, Some(24));
        selected = None;
        handle_picker_nav(&page_down, &mut selected, 25, 10);
        assert_eq!(selected, Some(0));
        selected = None;
        handle_picker_nav(&page_down, &mut selected, 0, 10);
        assert_eq!(selected, None);
    }
}
