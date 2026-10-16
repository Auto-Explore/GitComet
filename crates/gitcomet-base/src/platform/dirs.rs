//! Per-user directories of the running product.
//!
//! Every directory is named after [`ProductIdentity::directory_name`], so two
//! products never share state. Environment values are read as `OsString` and
//! joined without conversion: a non-UTF-8 home directory still works. An empty
//! variable counts as unset.
//! `GITCOMET_PROFILE_ROOT` isolates state/data/crashes for diagnostic launches
//! on every platform, without changing the process's home directory.
//!
//! | | state | data | crashes |
//! |---|---|---|---|
//! | Linux | `$XDG_STATE_HOME/<dir>`, `~/.local/state/<dir>` | `$XDG_DATA_HOME/<dir>`, `~/.local/share/<dir>` | `<state>/crashes` |
//! | macOS | `~/Library/Application Support/<dir>` | same as state | `~/Library/Logs/<dir>/crashes` |
//! | Windows | `%LOCALAPPDATA%\<dir>`, `%APPDATA%\<dir>` | same as state | `<state>\crashes` |
//! | other | `~/.<dir>` | same as state | `~/<dir>/crashes` |

use crate::identity::{self, ProductIdentity};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Settings, session, and runtime rendezvous files.
pub fn state_dir() -> Option<PathBuf> {
    ProductDirs::current().state_dir()
}

/// User-provided content such as custom themes.
pub fn data_dir() -> Option<PathBuf> {
    ProductDirs::current().data_dir()
}

/// Crash logs, recovery markers, and crash-time diagnostics.
pub fn crash_dir() -> Option<PathBuf> {
    ProductDirs::current().crash_dir()
}

/// Directory resolution for one product, platform, and environment.
pub struct ProductDirs<'a> {
    directory_name: &'a str,
    platform: Platform,
    env: &'a dyn Fn(&str) -> Option<OsString>,
}

fn process_env(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

impl ProductDirs<'static> {
    /// The installed identity on this platform with the process environment.
    pub fn current() -> Self {
        Self::new(identity::current(), Platform::current(), &process_env)
    }
}

impl<'a> ProductDirs<'a> {
    pub fn new(
        identity: &'a ProductIdentity,
        platform: Platform,
        env: &'a dyn Fn(&str) -> Option<OsString>,
    ) -> Self {
        Self {
            directory_name: identity.directory_name(),
            platform,
            env,
        }
    }

    fn var(&self, key: &str) -> Option<PathBuf> {
        non_empty_path((self.env)(key).as_deref())
    }

    fn home(&self) -> Option<PathBuf> {
        self.var("HOME")
    }

    fn windows_app_data(&self) -> Option<PathBuf> {
        self.var("LOCALAPPDATA").or_else(|| self.var("APPDATA"))
    }

    pub fn state_dir(&self) -> Option<PathBuf> {
        if let Some(root) = self.var("GITCOMET_PROFILE_ROOT") {
            return Some(root.join("state").join(self.directory_name));
        }
        let name = self.directory_name;
        match self.platform {
            Platform::Linux => self
                .var("XDG_STATE_HOME")
                .map(|base| base.join(name))
                .or_else(|| {
                    self.home()
                        .map(|home| home.join(".local").join("state").join(name))
                }),
            Platform::MacOs => self
                .home()
                .map(|home| home.join("Library").join("Application Support").join(name)),
            Platform::Windows => self.windows_app_data().map(|base| base.join(name)),
            Platform::Other => self.home().map(|home| home.join(format!(".{name}"))),
        }
    }

    pub fn data_dir(&self) -> Option<PathBuf> {
        if let Some(root) = self.var("GITCOMET_PROFILE_ROOT") {
            return Some(root.join("data").join(self.directory_name));
        }
        match self.platform {
            Platform::Linux => self
                .var("XDG_DATA_HOME")
                .map(|base| base.join(self.directory_name))
                .or_else(|| {
                    self.home()
                        .map(|home| home.join(".local").join("share").join(self.directory_name))
                }),
            Platform::MacOs | Platform::Windows | Platform::Other => self.state_dir(),
        }
    }

    pub fn crash_dir(&self) -> Option<PathBuf> {
        if self.var("GITCOMET_PROFILE_ROOT").is_some() {
            return Some(self.state_dir()?.join("crashes"));
        }
        let base = match self.platform {
            Platform::Linux | Platform::Windows => return Some(self.state_dir()?.join("crashes")),
            Platform::MacOs => self.home()?.join("Library").join("Logs"),
            Platform::Other => self.home()?,
        };
        Some(base.join(self.directory_name).join("crashes"))
    }
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    // Whitespace alone is as good as unset; any real path is kept byte-exact.
    if value.to_str().is_some_and(|text| text.trim().is_empty()) {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve(
        identity: &ProductIdentity,
        platform: Platform,
        vars: &[(&str, &str)],
    ) -> [Option<PathBuf>; 3] {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(key, value)| (key.to_string(), OsString::from(value)))
            .collect();
        let env = move |key: &str| vars.get(key).cloned();
        let dirs = ProductDirs::new(identity, platform, &env);
        [dirs.state_dir(), dirs.data_dir(), dirs.crash_dir()]
    }

    fn paths(values: [&str; 3]) -> [Option<PathBuf>; 3] {
        values.map(|value| Some(PathBuf::from(value)))
    }

    #[test]
    fn diagnostic_profiles_isolate_all_platforms_without_replacing_home() {
        for platform in [
            Platform::Linux,
            Platform::MacOs,
            Platform::Windows,
            Platform::Other,
        ] {
            assert_eq!(
                resolve(
                    &ProductIdentity::gitcomet(),
                    platform,
                    &[("HOME", "/user"), ("GITCOMET_PROFILE_ROOT", "/capture")]
                ),
                paths([
                    "/capture/state/gitcomet",
                    "/capture/data/gitcomet",
                    "/capture/state/gitcomet/crashes"
                ])
            );
        }
    }

    #[test]
    fn gitcomet_keeps_its_historical_locations_on_every_platform() {
        let gitcomet = ProductIdentity::gitcomet();
        assert_eq!(
            resolve(&gitcomet, Platform::Linux, &[("HOME", "/home/u")]),
            paths([
                "/home/u/.local/state/gitcomet",
                "/home/u/.local/share/gitcomet",
                "/home/u/.local/state/gitcomet/crashes",
            ])
        );
        assert_eq!(
            resolve(
                &gitcomet,
                Platform::Linux,
                &[
                    ("HOME", "/home/u"),
                    ("XDG_STATE_HOME", "/s"),
                    ("XDG_DATA_HOME", "/d")
                ]
            ),
            paths(["/s/gitcomet", "/d/gitcomet", "/s/gitcomet/crashes"])
        );
        assert_eq!(
            resolve(&gitcomet, Platform::MacOs, &[("HOME", "/Users/u")]),
            paths([
                "/Users/u/Library/Application Support/gitcomet",
                "/Users/u/Library/Application Support/gitcomet",
                "/Users/u/Library/Logs/gitcomet/crashes",
            ])
        );
        let local = PathBuf::from("C:/Users/u/AppData/Local");
        assert_eq!(
            resolve(
                &gitcomet,
                Platform::Windows,
                &[
                    ("LOCALAPPDATA", "C:/Users/u/AppData/Local"),
                    ("APPDATA", "C:/Roaming")
                ]
            ),
            [
                Some(local.join("gitcomet")),
                Some(local.join("gitcomet")),
                Some(local.join("gitcomet").join("crashes")),
            ]
        );
        assert_eq!(
            resolve(&gitcomet, Platform::Windows, &[("APPDATA", "C:/Roaming")])[0],
            Some(PathBuf::from("C:/Roaming").join("gitcomet"))
        );
        assert_eq!(
            resolve(&gitcomet, Platform::Other, &[("HOME", "/h")]),
            paths(["/h/.gitcomet", "/h/.gitcomet", "/h/gitcomet/crashes"])
        );
    }

    #[test]
    fn another_product_gets_its_own_directories() {
        let pro = ProductIdentity::builder("Pro", "pro").build().unwrap();
        assert_eq!(
            resolve(&pro, Platform::Linux, &[("HOME", "/home/u")]),
            paths([
                "/home/u/.local/state/pro",
                "/home/u/.local/share/pro",
                "/home/u/.local/state/pro/crashes",
            ])
        );
    }

    #[test]
    fn empty_or_blank_variables_count_as_unset() {
        let gitcomet = ProductIdentity::gitcomet();
        assert_eq!(
            resolve(
                &gitcomet,
                Platform::Linux,
                &[("XDG_STATE_HOME", ""), ("HOME", "/h")]
            )[0],
            Some(PathBuf::from("/h/.local/state/gitcomet"))
        );
        assert_eq!(
            resolve(&gitcomet, Platform::Linux, &[("HOME", "  ")]),
            [None, None, None]
        );
        assert_eq!(
            resolve(&gitcomet, Platform::Windows, &[("LOCALAPPDATA", "")]),
            [None, None, None]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_homes_are_joined_losslessly() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let home = OsString::from_vec(b"/home/\xffuser".to_vec());
        let env = |key: &str| (key == "HOME").then(|| home.clone());
        let gitcomet = ProductIdentity::gitcomet();
        let dirs = ProductDirs::new(&gitcomet, Platform::Linux, &env);
        assert_eq!(
            dirs.state_dir().unwrap().as_os_str().as_bytes(),
            b"/home/\xffuser/.local/state/gitcomet"
        );
    }
}
