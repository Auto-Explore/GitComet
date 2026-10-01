use super::*;

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
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

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn should_auto_install_linux_desktop_integration(
    no_desktop_install_flag_present: bool,
    system_desktop_entry_present: bool,
    _xdg_current_desktop: Option<&str>,
) -> bool {
    // `.desktop` entries follow the FreeDesktop spec, so installation is not GNOME-specific.
    // A user copy would shadow a packaged entry and outlive the package.
    !no_desktop_install_flag_present && !system_desktop_entry_present
}

/// The launcher entry for `identity`: GitComet's template with the product's
/// name, icon, window class, and the running executable.
#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn desktop_entry_for_identity(
    template: &str,
    identity: &gitcomet_core::identity::ProductIdentity,
    exec_arg: &str,
) -> String {
    let mut out = String::with_capacity(template.len() + 128);
    for line in template.lines() {
        let replacement = match line.split_once('=').map(|(key, _)| key) {
            Some("Exec") => Some(exec_arg),
            Some("Name") => Some(identity.display_name()),
            Some("Icon" | "StartupWMClass") => Some(identity.app_id()),
            _ => None,
        };
        match replacement {
            Some(value) => {
                out.push_str(line.split_once('=').map_or(line, |(key, _)| key));
                out.push('=');
                out.push_str(value);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Whether a package (deb, rpm, distro) installed the product's launcher in `$XDG_DATA_DIRS`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn system_desktop_entry_present(
    xdg_data_dirs: Option<&std::ffi::OsStr>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> bool {
    // The spec's default applies when the variable is unset or empty; relative entries are invalid.
    let dirs = xdg_data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("/usr/local/share:/usr/share"));
    let entry = gitcomet_core::identity::current().desktop_file_name();
    std::env::split_paths(dirs)
        .filter(|dir| dir.is_absolute())
        .any(|dir| exists(&dir.join("applications").join(&entry)))
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
            system_desktop_entry_present(std::env::var_os("XDG_DATA_DIRS").as_deref(), |path| {
                path.exists()
            }),
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

        let identity = gitcomet_core::identity::current();
        let desktop_path = data_home
            .join("applications")
            .join(identity.desktop_file_name());
        let icon_file = format!("{}.png", identity.app_id());
        // Custom branding ships one large icon; GitComet ships every size.
        let sizes: &[u32] = if identity.branding().app_icon_png.is_some() {
            &[512]
        } else {
            ICON_SIZES
        };
        let all_icons_exist = sizes.iter().all(|size| {
            data_home
                .join(format!("icons/hicolor/{size}x{size}/apps"))
                .join(&icon_file)
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
        // UI tests use GPUI's deterministic scheduler. A real blocking worker
        // cannot wake its foreground tasks, and tests must not install desktop
        // files into the developer's or CI runner's data directory.
        #[cfg(test)]
        let _ = cx;

        #[cfg(not(test))]
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

                        let identity = gitcomet_core::identity::current();
                        let branded_icon = identity.branding().app_icon_png.map(|png| [(512, png)]);
                        let icon_assets: &[(u32, &[u8])] = match &branded_icon {
                            Some(icon) => icon,
                            None => ICON_ASSETS,
                        };
                        let icon_file_name = format!("{}.png", identity.app_id());

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
                        let desktop_path = applications_dir.join(identity.desktop_file_name());
                        let icon_path = icons_root.join("512x512/apps").join(&icon_file_name);

                        fs::create_dir_all(&applications_dir)
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

                        let desktop_out = desktop_entry_for_identity(
                            DESKTOP_TEMPLATE,
                            identity,
                            &desktop_entry_exec_path_arg(&exe)?,
                        );

                        fs::write(&desktop_path, desktop_out.as_bytes())
                            .map_err(|e| format!("Desktop install failed: {e}"))?;

                        for (size, icon_bytes) in icon_assets {
                            let icon_dir = icons_root.join(format!("{size}x{size}/apps"));
                            let icon_file = icon_dir.join(&icon_file_name);
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
    #[test]
    fn desktop_entry_names_the_installed_product() {
        let template = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/linux/gitcomet.desktop"
        ));
        let gitcomet = gitcomet_core::identity::ProductIdentity::gitcomet();
        let entry = super::desktop_entry_for_identity(template, &gitcomet, "\"/usr/bin/gitcomet\"");
        assert!(entry.contains("\nName=GitComet\n"), "{entry}");
        assert!(entry.contains("\nExec=\"/usr/bin/gitcomet\"\n"), "{entry}");
        assert!(entry.contains("\nIcon=gitcomet\n"), "{entry}");
        assert!(entry.contains("\nStartupWMClass=gitcomet\n"), "{entry}");

        let pro = gitcomet_core::identity::ProductIdentity::builder("Comet Pro", "comet-pro")
            .build()
            .unwrap();
        let entry = super::desktop_entry_for_identity(template, &pro, "\"/opt/pro\"");
        assert!(entry.contains("\nName=Comet Pro\n"), "{entry}");
        assert!(entry.contains("\nIcon=comet-pro\n"), "{entry}");
        assert!(entry.contains("\nStartupWMClass=comet-pro\n"), "{entry}");
        assert!(
            entry.contains("\nCategories=Development;RevisionControl;\n"),
            "{entry}"
        );
        assert!(!entry.contains("gitcomet"), "{entry}");
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    use super::system_desktop_entry_present;
    use super::{desktop_entry_exec_path_arg, should_auto_install_linux_desktop_integration};
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    use std::ffi::OsStr;
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
                should_auto_install_linux_desktop_integration(false, false, Some(desktop)),
                "expected desktop '{desktop}' to allow auto install"
            );
        }
        assert!(should_auto_install_linux_desktop_integration(
            false, false, None
        ));
    }

    #[test]
    fn auto_install_respects_opt_out_flag() {
        assert!(!should_auto_install_linux_desktop_integration(
            true,
            false,
            Some("GNOME")
        ));
        assert!(!should_auto_install_linux_desktop_integration(
            true,
            false,
            Some("KDE")
        ));
    }

    #[test]
    fn auto_install_skips_when_a_package_installed_the_entry() {
        assert!(!should_auto_install_linux_desktop_integration(
            false,
            true,
            Some("GNOME")
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn system_entry_is_found_in_xdg_data_dirs() {
        let exists = |path: &Path| path == Path::new("/opt/share/applications/gitcomet.desktop");
        assert!(system_desktop_entry_present(
            Some(OsStr::new("/usr/share:/opt/share")),
            exists
        ));
        assert!(!system_desktop_entry_present(
            Some(OsStr::new("/usr/share")),
            exists
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn system_entry_lookup_defaults_when_xdg_data_dirs_is_unset_or_empty() {
        let exists = |path: &Path| path == Path::new("/usr/share/applications/gitcomet.desktop");
        assert!(system_desktop_entry_present(None, exists));
        assert!(system_desktop_entry_present(Some(OsStr::new("")), exists));
        assert!(!system_desktop_entry_present(
            Some(OsStr::new("/opt/share")),
            exists
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn system_entry_lookup_ignores_relative_and_empty_entries() {
        let exists = |path: &Path| path.ends_with("applications/gitcomet.desktop");
        assert!(!system_desktop_entry_present(
            Some(OsStr::new("share::relative/dir")),
            exists
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn window_tests_do_not_install_desktop_integration() {
        // Run in a subprocess so an empty XDG directory and the absent opt-out
        // apply to startup without changing the environment of parallel tests.
        // Existing desktop files otherwise hide the CI-only auto-install path.
        let data_home = tempfile::tempdir().expect("temporary desktop data directory");
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "app::tests::close_button_closes_only_the_clicked_window_after_opening_a_new_window",
            ])
            .env("XDG_DATA_HOME", data_home.path())
            .env_remove("GITCOMET_NO_DESKTOP_INSTALL")
            .output()
            .expect("run window test with no installed desktop files");
        assert!(
            output.status.success(),
            "window test failed with an empty desktop data directory:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            std::fs::read_dir(data_home.path())
                .expect("read desktop data directory")
                .next()
                .is_none(),
            "UI tests must leave the desktop data directory untouched",
        );
    }
}
