use gpui::Rgba;
use gpui::WindowAppearance;

#[derive(Clone, Copy)]
pub struct AppTheme {
    pub is_dark: bool,
    pub colors: Colors,
    pub radii: Radii,
}

#[derive(Clone, Copy)]
pub struct Colors {
    pub window_bg: Rgba,
    pub surface_bg: Rgba,
    pub surface_bg_elevated: Rgba,
    pub active_section: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub hover: Rgba,
    pub active: Rgba,
    pub focus_ring: Rgba,
    pub focus_ring_bg: Rgba,
    pub scrollbar_thumb: Rgba,
    pub scrollbar_thumb_hover: Rgba,
    pub scrollbar_thumb_active: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
}

#[derive(Clone, Copy)]
pub struct Radii {
    pub panel: f32,
    pub pill: f32,
    pub row: f32,
}

impl AppTheme {
    pub fn default_for_window_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::zed_one_light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::zed_ayu_dark(),
        }
    }

    /// Zed's "Ayu Dark" theme (ported from `zed/assets/themes/ayu/ayu.json`).
    pub fn zed_ayu_dark() -> Self {
        let accent = gpui::rgba(0x5ac1feff);
        let hover = gpui::rgba(0x2d2f34ff);
        let text_muted = gpui::rgba(0x8a8986ff);
        Self {
            is_dark: true,
            colors: Colors {
                // editor.background
                window_bg: gpui::rgba(0x0d1016ff),
                // surface.background
                surface_bg: gpui::rgba(0x1f2127ff),
                // elevated_surface.background
                surface_bg_elevated: gpui::rgba(0x1f2127ff),
                active_section: hover,
                // border.variant
                border: gpui::rgba(0x2d2f34ff),
                // text
                text: gpui::rgba(0xbfbdb6ff),
                // text.muted
                text_muted,
                // text.accent
                accent,
                // element.hover
                hover,
                active: with_alpha(hover, 0.78),
                focus_ring: with_alpha(accent, 0.60),
                focus_ring_bg: with_alpha(accent, 0.16),
                scrollbar_thumb: with_alpha(text_muted, 0.30),
                scrollbar_thumb_hover: with_alpha(text_muted, 0.42),
                scrollbar_thumb_active: with_alpha(text_muted, 0.52),
                // terminal.ansi.red
                danger: gpui::rgba(0xef7177ff),
                // terminal.ansi.yellow
                warning: gpui::rgba(0xfeb454ff),
                // terminal.ansi.green
                success: gpui::rgba(0xaad84cff),
            },
            radii: Radii {
                panel: 2.0,
                pill: 2.0,
                row: 2.0,
            },
        }
    }

    /// Zed's "One Light" theme (ported from `zed/assets/themes/one/one.json`).
    pub fn zed_one_light() -> Self {
        let accent = gpui::rgba(0x5c78e2ff);
        let hover = gpui::rgba(0xdfdfe0ff);
        let text_muted = gpui::rgba(0x58585aff);
        Self {
            is_dark: false,
            colors: Colors {
                // editor.background
                window_bg: gpui::rgba(0xfafafaff),
                // surface.background
                surface_bg: gpui::rgba(0xebebecff),
                // elevated_surface.background
                surface_bg_elevated: gpui::rgba(0xebebecff),
                active_section: gpui::rgba(0xfafafaff),
                // border.variant
                border: gpui::rgba(0xdfdfe0ff),
                // text
                text: gpui::rgba(0x242529ff),
                // text.muted
                text_muted,
                // text.accent
                accent,
                // element.hover
                hover,
                active: with_alpha(hover, 0.88),
                focus_ring: with_alpha(accent, 0.52),
                focus_ring_bg: with_alpha(accent, 0.12),
                scrollbar_thumb: with_alpha(text_muted, 0.26),
                scrollbar_thumb_hover: with_alpha(text_muted, 0.36),
                scrollbar_thumb_active: with_alpha(text_muted, 0.46),
                // terminal.ansi.red
                danger: gpui::rgba(0xde3e35ff),
                // terminal.ansi.yellow
                warning: gpui::rgba(0xd2b67cff),
                // terminal.ansi.green
                success: gpui::rgba(0x3f953aff),
            },
            radii: Radii {
                panel: 2.0,
                pill: 2.0,
                row: 2.0,
            },
        }
    }
}

pub(crate) fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

#[cfg(test)]
mod tests {
    use super::{Rgba, with_alpha};

    #[test]
    fn with_alpha_preserves_rgb_and_overwrites_alpha() {
        let color = Rgba {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        };

        let adjusted = with_alpha(color, 0.75);

        assert_eq!(adjusted.r, color.r);
        assert_eq!(adjusted.g, color.g);
        assert_eq!(adjusted.b, color.b);
        assert_eq!(adjusted.a, 0.75);
    }
}
