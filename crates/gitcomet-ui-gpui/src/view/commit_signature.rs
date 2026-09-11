//! Presentation for commit signature badges.
//!
//! Shared by the details pane (a chip) and the history rows (an icon painted on
//! the canvas), so both agree on colour, glyph and wording.

use crate::theme::{AppTheme, StatusColorSet};
use gitcomet_core::domain::{CommitSignature, SignatureStatus};
use gpui::SharedString;

pub(in crate::view) const SHIELD_CHECK_ICON_PATH: &str = "icons/shield_check.svg";
pub(in crate::view) const SHIELD_ALERT_ICON_PATH: &str = "icons/shield_alert.svg";

pub(in crate::view) struct SignatureBadge {
    pub icon: &'static str,
    pub palette: StatusColorSet,
    pub label: SharedString,
    pub tooltip: SharedString,
}

fn status_label(status: SignatureStatus) -> &'static str {
    match status {
        SignatureStatus::Good => "Verified",
        SignatureStatus::GoodUncertified => "Untrusted key",
        SignatureStatus::Expired => "Expired",
        SignatureStatus::ExpiredKey => "Expired key",
        SignatureStatus::Bad => "Bad signature",
        SignatureStatus::Revoked => "Revoked key",
    }
}

fn status_palette(theme: AppTheme, status: SignatureStatus) -> StatusColorSet {
    match status {
        SignatureStatus::Good => theme.colors.status.info,
        SignatureStatus::GoodUncertified => theme.colors.status.warning,
        SignatureStatus::Expired | SignatureStatus::ExpiredKey => theme.colors.status.warning,
        SignatureStatus::Bad | SignatureStatus::Revoked => theme.colors.status.danger,
    }
}

fn tooltip_text(signature: &CommitSignature) -> String {
    let mut lines = vec![format!(
        "{} {} signature",
        if signature.status == SignatureStatus::Bad {
            "Bad"
        } else {
            status_label(signature.status)
        },
        signature.format.label()
    )];
    if let Some(signer) = &signature.signer {
        lines.push(format!("Signed by {signer}"));
    }
    if let Some(key_id) = &signature.key_id {
        lines.push(format!("Key {key_id}"));
    }
    if signature.status == SignatureStatus::GoodUncertified {
        // Without this the badge overstates itself: the signature is good, but
        // nothing says the key belongs to who it claims to.
        lines
            .push("The signing key is not trusted by your verification configuration.".to_string());
    }
    lines.join("\n")
}

/// Just what painting needs. Split out from [`signature_badge`] because the
/// history canvas draws the icon for every signed row on every frame and must
/// not build the label and tooltip strings it never shows.
pub(in crate::view) fn signature_glyph(
    theme: AppTheme,
    signature: &CommitSignature,
) -> (&'static str, StatusColorSet) {
    let icon = if signature.status.is_verified() {
        SHIELD_CHECK_ICON_PATH
    } else {
        SHIELD_ALERT_ICON_PATH
    };
    (icon, status_palette(theme, signature.status))
}

/// The hover text. Built on demand, once per hover transition.
pub(in crate::view) fn signature_tooltip(signature: &CommitSignature) -> SharedString {
    tooltip_text(signature).into()
}

pub(in crate::view) fn signature_badge(
    theme: AppTheme,
    signature: &CommitSignature,
) -> SignatureBadge {
    let (icon, palette) = signature_glyph(theme, signature);
    SignatureBadge {
        icon,
        palette,
        label: status_label(signature.status).into(),
        tooltip: signature_tooltip(signature),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::SignatureFormat;
    use std::sync::Arc;

    fn signature(status: SignatureStatus) -> CommitSignature {
        CommitSignature {
            status,
            format: SignatureFormat::OpenPgp,
            signer: Some(Arc::from("Ada <ada@example.com>")),
            key_id: Some(Arc::from("DEADBEEF")),
        }
    }

    #[test]
    fn verified_statuses_use_the_shield_check_and_the_info_palette() {
        let theme = AppTheme::gitcomet_dark();
        for status in [SignatureStatus::Good] {
            let badge = signature_badge(theme, &signature(status));
            assert_eq!(badge.icon, SHIELD_CHECK_ICON_PATH);
            assert_eq!(badge.palette, theme.colors.status.info);
            assert_eq!(badge.label, "Verified");
        }
    }

    #[test]
    fn failed_statuses_warn_with_their_own_palettes() {
        let theme = AppTheme::gitcomet_dark();
        for (status, expected) in [
            (SignatureStatus::Expired, theme.colors.status.warning),
            (SignatureStatus::ExpiredKey, theme.colors.status.warning),
            (SignatureStatus::Bad, theme.colors.status.danger),
            (SignatureStatus::Revoked, theme.colors.status.danger),
        ] {
            let badge = signature_badge(theme, &signature(status));
            assert_eq!(badge.icon, SHIELD_ALERT_ICON_PATH);
            assert_eq!(badge.palette, expected);
        }
    }

    #[test]
    fn an_uncertified_key_says_so_in_the_tooltip() {
        let theme = AppTheme::gitcomet_dark();
        let certified = signature_badge(theme, &signature(SignatureStatus::Good));
        let uncertified = signature_badge(theme, &signature(SignatureStatus::GoodUncertified));

        assert!(certified.tooltip.contains("Signed by Ada"));
        assert!(!certified.tooltip.contains("not trusted"));
        assert!(uncertified.tooltip.contains("not trusted"));
    }

    #[test]
    fn an_untrusted_key_has_its_own_warning_badge() {
        let theme = AppTheme::gitcomet_dark();
        let badge = signature_badge(theme, &signature(SignatureStatus::GoodUncertified));
        assert_eq!(badge.label, "Untrusted key");
        assert_eq!(badge.icon, SHIELD_ALERT_ICON_PATH);
        assert_eq!(badge.palette, theme.colors.status.warning);
    }

    #[test]
    fn a_bad_signature_tooltip_does_not_repeat_signature() {
        let text = signature_tooltip(&signature(SignatureStatus::Bad));
        assert_eq!(text.lines().next(), Some("Bad GPG signature"));
    }
}
