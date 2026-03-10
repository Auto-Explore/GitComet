use super::*;

fn desktop_entry_exec_path_arg(exe: &std::path::Path) -> Result<String, String> {
    let Some(exe) = exe.to_str() else {
        return Err(format!(
            "Desktop install failed: executable path is not valid Unicode: {exe:?}"
        ));
    };
    let mut escaped = String::with_capacity(exe.len() + 16);
    escaped.push('"');
    for ch in exe.chars() {
        match ch {
            // Desktop entry field code escape.
            '%' => escaped.push_str("%%"),
            // Exec strings are unescaped once as generic strings, then again as command args.
            '\\' => escaped.push_str("\\\\\\\\"),
            '$' => escaped.push_str("\\\\$"),
            '"' => escaped.push_str("\\\""),
            '`' => escaped.push_str("\\`"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

fn should_auto_install_linux_desktop_integration(
    no_desktop_install_flag_present: bool,
    _xdg_current_desktop: Option<&str>,
) -> bool {
    // `.desktop` entries follow the FreeDesktop spec, so installation is not GNOME-specific.
    !no_desktop_install_flag_present
}

impl GitCometView {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(in crate::view) fn maybe_auto_install_linux_desktop_integration(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        use std::path::PathBuf;
        const ICON_SIZES: &[u32] = &[32, 48, 128, 256, 512];

        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        if !should_auto_install_linux_desktop_integration(
            std::env::var_os("GITCOMET_NO_DESKTOP_INSTALL").is_some(),
            desktop.as_deref(),
        ) {
            return;
        }

        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        let Some(data_home) = data_home else {
            return;
        };

        let desktop_path = data_home.join("applications/gitcomet.desktop");
        let all_icons_exist = ICON_SIZES.iter().all(|size| {
            data_home
                .join(format!("icons/hicolor/{size}x{size}/apps/gitcomet.png"))
                .exists()
        });
        if desktop_path.exists() && all_icons_exist {
            return;
        }

        self.install_linux_desktop_integration(cx);
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(in crate::view) fn install_linux_desktop_integration(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let result: Result<(std::path::PathBuf, std::path::PathBuf), String> =
                    smol::unblock(|| {
                        use std::fs;
                        use std::path::PathBuf;
                        use std::process::Command;

                        const DESKTOP_TEMPLATE: &str = include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/gitcomet.desktop"
                        ));
                        const ICON_32_PNG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/hicolor/32x32/apps/gitcomet.png"
                        ));
                        const ICON_48_PNG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/hicolor/48x48/apps/gitcomet.png"
                        ));
                        const ICON_128_PNG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/hicolor/128x128/apps/gitcomet.png"
                        ));
                        const ICON_256_PNG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/hicolor/256x256/apps/gitcomet.png"
                        ));
                        const ICON_512_PNG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/linux/hicolor/512x512/apps/gitcomet.png"
                        ));
                        const ICON_ASSETS: &[(u32, &[u8])] = &[
                            (32, ICON_32_PNG),
                            (48, ICON_48_PNG),
                            (128, ICON_128_PNG),
                            (256, ICON_256_PNG),
                            (512, ICON_512_PNG),
                        ];

                        let exe = std::env::current_exe().map_err(|_| {
                            "Desktop install failed: could not resolve executable path".to_string()
                        })?;

                        let home = std::env::var_os("HOME").map(PathBuf::from);
                        let data_home = std::env::var_os("XDG_DATA_HOME")
                            .map(PathBuf::from)
                            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
                        let data_home = data_home.ok_or_else(|| {
                            "Desktop install failed: HOME/XDG_DATA_HOME not set".to_string()
                        })?;

                        let applications_dir = data_home.join("applications");
                        let icons_root = data_home.join("icons/hicolor");
                        let desktop_path = applications_dir.join("gitcomet.desktop");
                        let icon_path = icons_root.join("512x512/apps/gitcomet.png");

                        fs::create_dir_all(&applications_dir)
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

                        use std::fmt::Write as _;

                        let mut desktop_out = String::with_capacity(DESKTOP_TEMPLATE.len() + 128);
                        for line in DESKTOP_TEMPLATE.lines() {
                            if line.starts_with("Exec=") {
                                desktop_out.push_str("Exec=");
                                let _ = writeln!(
                                    &mut desktop_out,
                                    "{}",
                                    desktop_entry_exec_path_arg(&exe)?
                                );
                            } else {
                                desktop_out.push_str(line);
                                desktop_out.push('\n');
                            }
                        }

                        fs::write(&desktop_path, desktop_out.as_bytes())
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

                        for (size, icon_bytes) in ICON_ASSETS {
                            let icon_dir = icons_root.join(format!("{size}x{size}/apps"));
                            let icon_file = icon_dir.join("gitcomet.png");
                            fs::create_dir_all(&icon_dir)
                                .and_then(|_| fs::write(&icon_file, icon_bytes))
                                .map_err(|e| format!("Desktop install failed: {e}"))?;
                        }

                        let _ = Command::new("update-desktop-database")
                            .arg(&applications_dir)
                            .output();
                        let _ = Command::new("gtk-update-icon-cache")
                            .arg(data_home.join("icons/hicolor"))
                            .output();

                        Ok::<_, String>((desktop_path, icon_path))
                    })
                    .await;

                let _ = view.update(cx, |this, cx| match result {
                    Ok((desktop_path, icon_path)) => {
                        this.push_toast(
                            components::ToastKind::Success,
                            format!(
                                "Installed desktop entry + icon to:\n{}\n{}\n\nIf GNOME still shows a generic icon, log out/in (or restart GNOME Shell).",
                                desktop_path.display(),
                                icon_path.display()
                            ),
                            cx,
                        );
                    }
                    Err(message) => {
                        this.push_toast(components::ToastKind::Error, message, cx);
                    }
                });
            },
        )
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{desktop_entry_exec_path_arg, should_auto_install_linux_desktop_integration};
    use std::path::Path;

    #[test]
    fn desktop_exec_path_is_quoted() {
        let escaped =
            desktop_entry_exec_path_arg(Path::new("/usr/local/bin/gitcomet")).expect("path");
        assert_eq!(escaped, "\"/usr/local/bin/gitcomet\"");
    }

    #[test]
    fn desktop_exec_path_escapes_percent_and_spaces() {
        let escaped = desktop_entry_exec_path_arg(Path::new("/opt/Git Comet/git%f")).expect("path");
        assert_eq!(escaped, "\"/opt/Git Comet/git%%f\"");
    }

    #[test]
    fn desktop_exec_path_escapes_special_exec_chars() {
        let escaped = desktop_entry_exec_path_arg(Path::new("/tmp/a\"b\\c$d`e")).expect("path");
        assert_eq!(escaped, "\"/tmp/a\\\"b\\\\\\\\c\\\\$d\\`e\"");
    }

    #[test]
    fn auto_install_is_not_limited_to_gnome() {
        for desktop in ["GNOME", "KDE", "XFCE", "sway", ""] {
            assert!(
                should_auto_install_linux_desktop_integration(false, Some(desktop)),
                "expected desktop '{desktop}' to allow auto install"
            );
        }
        assert!(should_auto_install_linux_desktop_integration(false, None));
    }

    #[test]
    fn auto_install_respects_opt_out_flag() {
        assert!(!should_auto_install_linux_desktop_integration(
            true,
            Some("GNOME")
        ));
        assert!(!should_auto_install_linux_desktop_integration(
            true,
            Some("KDE")
        ));
    }
}
