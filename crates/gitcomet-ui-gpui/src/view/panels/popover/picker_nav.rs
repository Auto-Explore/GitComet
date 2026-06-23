use super::components;

pub(super) struct PickerNavKeys {
    pub escape: bool,
    pub arrow_up: bool,
    pub arrow_down: bool,
    pub tab: bool,
    pub shift_tab: bool,
    pub enter: bool,
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
pub(super) fn handle_picker_nav(
    keys: &PickerNavKeys,
    selected: &mut Option<usize>,
    count: usize,
) -> PickerNavOutcome {
    if keys.escape {
        return PickerNavOutcome::Escape;
    }
    if keys.arrow_up || keys.shift_tab {
        *selected = Some(match *selected {
            Some(ix) if ix > 0 => ix - 1,
            _ if count > 0 => count - 1,
            _ => return PickerNavOutcome::Idle,
        });
        return PickerNavOutcome::Navigated;
    }
    if keys.arrow_down || keys.tab {
        *selected = Some(match *selected {
            Some(ix) if ix + 1 < count => ix + 1,
            _ if count > 0 => 0,
            _ => return PickerNavOutcome::Idle,
        });
        return PickerNavOutcome::Navigated;
    }
    if keys.enter {
        return PickerNavOutcome::Enter;
    }
    PickerNavOutcome::Idle
}
