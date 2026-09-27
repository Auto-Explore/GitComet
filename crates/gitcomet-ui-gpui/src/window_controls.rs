use gitcomet_state::session;
use gpui::{BorrowAppContext, Decorations, WindowButton, WindowButtonLayout};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum WindowControlsMode {
    #[default]
    System,
    Show,
    Hide,
}

impl WindowControlsMode {
    pub(crate) const ALL: [Self; 3] = [Self::System, Self::Show, Self::Hide];

    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Show => "show",
            Self::Hide => "hide",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(Self::System),
            "show" => Some(Self::Show),
            "hide" => Some(Self::Hide),
            _ => None,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Show => "Always show",
            Self::Hide => "Hide minimize and maximize",
        }
    }

    pub(crate) const fn detail(self) -> &'static str {
        match self {
            Self::System => "Follow the desktop layout and hide controls while tiled.",
            Self::Show => "Show minimize and maximize controls in every window.",
            Self::Hide => "Keep only the close control, useful with tiling window managers.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AppWindowControls {
    pub(crate) mode: WindowControlsMode,
    initialized: bool,
}

impl Default for AppWindowControls {
    fn default() -> Self {
        Self {
            mode: WindowControlsMode::System,
            initialized: false,
        }
    }
}

impl gpui::Global for AppWindowControls {}

pub(crate) fn current<C>(cx: &mut C) -> AppWindowControls
where
    C: BorrowAppContext,
{
    cx.update_default_global::<AppWindowControls, _>(|controls, _cx| *controls)
}

pub(crate) fn current_or_initialize_from_session<C>(
    ui_session: &session::UiSession,
    cx: &mut C,
) -> AppWindowControls
where
    C: BorrowAppContext,
{
    let current = current(cx);
    if current.initialized {
        return current;
    }

    let next = AppWindowControls {
        mode: ui_session
            .window_controls_mode
            .as_deref()
            .and_then(WindowControlsMode::from_key)
            .unwrap_or_default(),
        initialized: true,
    };
    cx.set_global(next);
    next
}

pub(crate) fn set_current<C>(cx: &mut C, mode: WindowControlsMode)
where
    C: BorrowAppContext,
{
    cx.set_global(AppWindowControls {
        mode,
        initialized: true,
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedWindowControls {
    pub(crate) left: [Option<WindowButton>; 3],
    pub(crate) right: [Option<WindowButton>; 3],
}

pub(crate) fn resolve_visibility(
    mode: WindowControlsMode,
    system_layout: Option<WindowButtonLayout>,
    system_layout_supported: bool,
    decorations: Decorations,
    is_maximized: bool,
) -> ResolvedWindowControls {
    // Wayland reports every tiled edge for maximized windows too. Those
    // windows still need their minimize and restore controls.
    let system_window_is_tiled = !is_maximized
        && matches!(decorations, Decorations::Client { tiling }
            if tiling.top || tiling.bottom || tiling.left || tiling.right);
    let trailing = |minimize: bool, maximize: bool| ResolvedWindowControls {
        left: [None; 3],
        right: [
            minimize.then_some(WindowButton::Minimize),
            maximize.then_some(WindowButton::Maximize),
            Some(WindowButton::Close),
        ],
    };
    match mode {
        WindowControlsMode::Show => trailing(true, true),
        WindowControlsMode::Hide => trailing(false, false),
        WindowControlsMode::System if system_layout_supported => {
            let layout = system_layout.unwrap_or(WindowButtonLayout {
                left: [None; 3],
                right: [
                    Some(WindowButton::Minimize),
                    Some(WindowButton::Maximize),
                    Some(WindowButton::Close),
                ],
            });
            let filter = |side: [Option<WindowButton>; 3]| {
                side.map(|button| {
                    button.filter(|button| {
                        !system_window_is_tiled
                            || !matches!(button, WindowButton::Minimize | WindowButton::Maximize)
                    })
                })
            };
            ResolvedWindowControls {
                left: filter(layout.left),
                right: filter(layout.right),
            }
        }
        WindowControlsMode::System => trailing(true, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_EDGES_TILED: Decorations = Decorations::Client {
        tiling: gpui::Tiling {
            top: true,
            bottom: true,
            left: true,
            right: true,
        },
    };

    #[test]
    fn system_mode_uses_linux_desktop_button_layout() {
        let close_only = WindowButtonLayout {
            left: [Some(WindowButton::Close), None, None],
            right: [None; 3],
        };
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                Some(close_only),
                true,
                Decorations::Server,
                false,
            ),
            ResolvedWindowControls {
                left: [Some(WindowButton::Close), None, None],
                right: [None; 3],
            }
        );

        let mixed = WindowButtonLayout {
            left: [Some(WindowButton::Minimize), None, None],
            right: [Some(WindowButton::Close), None, None],
        };
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                Some(mixed),
                true,
                Decorations::Server,
                false,
            ),
            ResolvedWindowControls {
                left: [Some(WindowButton::Minimize), None, None],
                right: [Some(WindowButton::Close), None, None],
            }
        );
    }

    #[test]
    fn explicit_modes_override_the_system_layout() {
        let empty = WindowButtonLayout {
            left: [None; 3],
            right: [None; 3],
        };
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::Show,
                Some(empty),
                true,
                ALL_EDGES_TILED,
                false
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [
                    Some(WindowButton::Minimize),
                    Some(WindowButton::Maximize),
                    Some(WindowButton::Close),
                ],
            }
        );
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::Hide,
                None,
                false,
                Decorations::Server,
                false
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [None, None, Some(WindowButton::Close)],
            }
        );
    }

    #[test]
    fn system_mode_keeps_windows_controls_on_platforms_without_a_layout_api() {
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                None,
                false,
                ALL_EDGES_TILED,
                false
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [
                    Some(WindowButton::Minimize),
                    Some(WindowButton::Maximize),
                    Some(WindowButton::Close),
                ],
            }
        );
    }

    #[test]
    fn system_mode_hides_minimize_and_maximize_for_tiled_linux_windows() {
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                None,
                true,
                ALL_EDGES_TILED,
                false
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [None, None, Some(WindowButton::Close)],
            }
        );
    }

    #[test]
    fn review_regression_maximized_linux_windows_keep_minimize_and_restore_controls() {
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                None,
                true,
                ALL_EDGES_TILED,
                true
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [
                    Some(WindowButton::Minimize),
                    Some(WindowButton::Maximize),
                    Some(WindowButton::Close),
                ],
            }
        );

        let layout = WindowButtonLayout {
            left: [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None,
            ],
            right: [Some(WindowButton::Maximize), None, None],
        };
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::System,
                Some(layout),
                true,
                ALL_EDGES_TILED,
                true
            ),
            ResolvedWindowControls {
                left: layout.left,
                right: layout.right
            }
        );
        assert_eq!(
            resolve_visibility(
                WindowControlsMode::Hide,
                Some(layout),
                true,
                ALL_EDGES_TILED,
                true
            ),
            ResolvedWindowControls {
                left: [None; 3],
                right: [None, None, Some(WindowButton::Close)],
            }
        );
    }

    #[test]
    fn review_regression_system_mode_preserves_button_side_and_order() {
        let close_then_minimize_on_left = WindowButtonLayout {
            left: [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None,
            ],
            right: [None; 3],
        };
        let minimize_then_close_on_left = WindowButtonLayout {
            left: [
                Some(WindowButton::Minimize),
                Some(WindowButton::Close),
                None,
            ],
            right: [None; 3],
        };
        let close_then_minimize_on_right = WindowButtonLayout {
            left: [None; 3],
            right: [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None,
            ],
        };

        let left_close_first = resolve_visibility(
            WindowControlsMode::System,
            Some(close_then_minimize_on_left),
            true,
            Decorations::Server,
            false,
        );
        let left_minimize_first = resolve_visibility(
            WindowControlsMode::System,
            Some(minimize_then_close_on_left),
            true,
            Decorations::Server,
            false,
        );
        let right_close_first = resolve_visibility(
            WindowControlsMode::System,
            Some(close_then_minimize_on_right),
            true,
            Decorations::Server,
            false,
        );

        assert_ne!(
            left_close_first, left_minimize_first,
            "different system button ordering must produce a different render layout"
        );
        assert_ne!(
            left_close_first, right_close_first,
            "moving the system controls to the other side must change the render layout"
        );
    }
}
