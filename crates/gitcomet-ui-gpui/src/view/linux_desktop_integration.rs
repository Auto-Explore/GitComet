use super::*;

impl GitCometView {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(in crate::view) fn maybe_auto_install_linux_desktop_integration(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        use std::path::PathBuf;

        if std::env::var_os("GITCOMET_NO_DESKTOP_INSTALL").is_some() {
            return;
        }

        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        const GNOME: &[u8] = b"gnome";
        if !desktop
            .as_bytes()
            .windows(GNOME.len())
            .any(|window| window.eq_ignore_ascii_case(GNOME))
        {
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
        let icon_path = data_home.join("icons/hicolor/scalable/apps/gitcomet-512.svg");
        if desktop_path.exists() && icon_path.exists() {
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
                        const ICON_SVG: &[u8] = include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../assets/gitcomet-512.svg"
                        ));

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
                        let icons_dir = data_home.join("icons/hicolor/scalable/apps");
                        let desktop_path = applications_dir.join("gitcomet.desktop");
                        let icon_path = icons_dir.join("gitcomet-512.svg");

                        fs::create_dir_all(&applications_dir)
                            .and_then(|_| fs::create_dir_all(&icons_dir))
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

                        use std::fmt::Write as _;

                        let mut desktop_out = String::with_capacity(DESKTOP_TEMPLATE.len() + 128);
                        for line in DESKTOP_TEMPLATE.lines() {
                            if line.starts_with("Exec=") {
                                desktop_out.push_str("Exec=");
                                let _ = writeln!(&mut desktop_out, "{}", exe.display());
                            } else {
                                desktop_out.push_str(line);
                                desktop_out.push('\n');
                            }
                        }

                        fs::write(&desktop_path, desktop_out.as_bytes())
                            .and_then(|_| fs::write(&icon_path, ICON_SVG))
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

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
