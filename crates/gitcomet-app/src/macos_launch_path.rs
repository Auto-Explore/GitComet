//! The command search path for an app launchd started (Finder, the Dock,
//! `open`).
//!
//! launchd starts apps with `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. Git config
//! names helper programs by bare name, such as `git-lfs filter-process`,
//! credential helpers and `gpg` for signing, and those live where Homebrew
//! (`/opt/homebrew/bin`, `/usr/local/bin`), MacPorts (`/opt/local/bin`) or an
//! installer (`/usr/local/bin`) put them. gix runs filter drivers through
//! `sh -c` with this process's environment, so under launchd's `PATH` every
//! status walk over an LFS checkout fails with `git-lfs: command not found`,
//! and so do GitComet's own `git` commands that touch LFS files.
//!
//! When `PATH` holds nothing but launchd's system directories, the launch
//! restarts itself in place with the directories a terminal's login shell
//! would have put in front of them: the package-manager prefixes, then what
//! `path_helper` reads from `/etc/paths` and `/etc/paths.d`. Any other `PATH`
//! was chosen by the user (a terminal, `launchctl setenv PATH`) and is kept.
//!
//! Asking the login shell instead (`$SHELL -l -c 'printf %s "$PATH"'`) would
//! run the user's profile scripts inside app startup: that takes from a few
//! milliseconds to seconds, can stop on a prompt, and needs a timeout that
//! leaves the bug in place when it expires. The fixed list costs a few `stat`
//! calls. Directories that only a profile script adds (`~/.cargo/bin`,
//! version managers) stay missing; Git config can name those helpers by
//! absolute path.
//!
//! The process restarts rather than setting the variable, because
//! `std::env::set_var` is `unsafe` (the workspace forbids it) and races with
//! any thread that reads the environment. `exec` keeps the PID, so Git,
//! launchd and LaunchServices still see the process they started.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

/// The directories of the `PATH` launchd gives the apps it starts.
const LAUNCHD_PATH_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// Package-manager prefixes, in the order their shell setup puts them first:
/// Homebrew on Apple Silicon, Homebrew on Intel (also where the git-lfs and
/// Git Credential Manager installers link their programs), then MacPorts.
const PACKAGE_MANAGER_DIRS: [&str; 6] = [
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/local/bin",
    "/opt/local/sbin",
];

/// Restarts this process in place with [`launch_path`] when launchd started
/// it with its bare system `PATH`. Returns when the `PATH` is kept, or when
/// the restart failed, in which case the launch goes on with the old `PATH`.
///
/// `args` are the process arguments, program name first. Call this before
/// the launch has side effects, since they happen again after the restart.
#[cfg(target_os = "macos")]
pub(crate) fn restart_with_package_manager_path(args: &[OsString]) {
    use std::os::unix::process::CommandExt as _;

    let current = std::env::var_os("PATH");
    let Some(path) = launch_path(
        current.as_deref(),
        || path_helper_dirs(Path::new("/etc")),
        Path::is_dir,
    ) else {
        return;
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            eprintln!("Could not find this executable to restart it with a fuller PATH: {err}");
            return;
        }
    };
    let mut restart = std::process::Command::new(exe);
    if let Some((program, rest)) = args.split_first() {
        restart.arg0(program).args(rest);
    }
    restart.env("PATH", path);
    let err = restart.exec();
    eprintln!("Could not restart with a fuller PATH; Git helpers may not be found: {err}");
}

/// The `PATH` to restart with, or `None` to keep the current one.
///
/// Only a `PATH` that is unset, empty, or made of launchd's system
/// directories is replaced, and `path_helper_dirs` is read only then. The
/// replacement puts the package-manager directories and `path_helper_dirs`
/// that `is_dir` accepts in front of the current entries, without
/// duplicates. It is `None` as well when that adds nothing outside launchd's
/// directories, so a replacement is never itself replaced: the restarted
/// process keeps its `PATH` instead of restarting again.
fn launch_path(
    current: Option<&OsStr>,
    path_helper_dirs: impl FnOnce() -> Vec<PathBuf>,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<OsString> {
    let current_dirs = launchd_path_dirs(current)?;
    let added = PACKAGE_MANAGER_DIRS
        .iter()
        .map(PathBuf::from)
        .chain(path_helper_dirs())
        .filter(|dir| is_usable_path_entry(dir) && is_dir(dir));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in added.chain(current_dirs) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    if dirs.iter().all(|dir| is_launchd_path_dir(dir)) {
        return None;
    }
    std::env::join_paths(dirs).ok()
}

/// The entries of `current` when it is launchd's `PATH`: unset or empty
/// (counted as launchd's own directories) or made only of those
/// directories. `None` for any other `PATH`, including one with an empty
/// entry, which means the working directory.
fn launchd_path_dirs(current: Option<&OsStr>) -> Option<Vec<PathBuf>> {
    let Some(current) = current.filter(|path| !path.is_empty()) else {
        return Some(LAUNCHD_PATH_DIRS.iter().map(PathBuf::from).collect());
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(current).collect();
    dirs.iter()
        .all(|dir| is_launchd_path_dir(dir))
        .then_some(dirs)
}

fn is_launchd_path_dir(dir: &Path) -> bool {
    LAUNCHD_PATH_DIRS
        .iter()
        .any(|launchd| dir == Path::new(launchd))
}

/// An absolute directory that `PATH` can hold as one entry.
fn is_usable_path_entry(dir: &Path) -> bool {
    dir.is_absolute() && !dir.as_os_str().as_bytes().contains(&b':')
}

/// The directories `path_helper` puts on a login shell's `PATH`: the lines
/// of `<etc>/paths`, then those of each file in `<etc>/paths.d` in name
/// order. Unreadable files are skipped.
fn path_helper_dirs(etc: &Path) -> Vec<PathBuf> {
    let mut files = vec![etc.join("paths")];
    if let Ok(entries) = std::fs::read_dir(etc.join("paths.d")) {
        let mut drop_ins: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        drop_ins.sort();
        files.extend(drop_ins);
    }
    files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .flat_map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const LAUNCHD_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

    /// `/etc/paths` and `/etc/paths.d` as a stock Apple Silicon Mac with
    /// Homebrew has them.
    fn stock_path_helper_dirs() -> Vec<PathBuf> {
        [
            "/usr/local/bin",
            "/System/Cryptexes/App/usr/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
            "/Library/Apple/usr/bin",
            "/opt/homebrew/bin",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    fn existing(dirs: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |dir: &Path| dirs.iter().any(|existing| dir == Path::new(existing))
    }

    const APPLE_SILICON_DIRS: &[&str] = &[
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/System/Cryptexes/App/usr/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
        "/Library/Apple/usr/bin",
    ];

    fn launch_path_str(
        current: Option<&str>,
        path_helper_dirs: Vec<PathBuf>,
        dirs: &'static [&'static str],
    ) -> Option<String> {
        launch_path(current.map(OsStr::new), || path_helper_dirs, existing(dirs))
            .map(|path| path.into_string().unwrap())
    }

    #[test]
    fn launchd_path_gets_package_manager_and_path_helper_dirs_in_front() {
        assert_eq!(
            launch_path_str(
                Some(LAUNCHD_PATH),
                stock_path_helper_dirs(),
                APPLE_SILICON_DIRS
            )
            .as_deref(),
            Some(
                "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:\
                 /System/Cryptexes/App/usr/bin:/usr/bin:/bin:/usr/sbin:/sbin:\
                 /Library/Apple/usr/bin"
            ),
        );
    }

    #[test]
    fn intel_homebrew_and_installer_dir_come_first_on_intel() {
        assert_eq!(
            launch_path_str(
                Some(LAUNCHD_PATH),
                Vec::new(),
                &["/usr/local/bin", "/usr/local/sbin", "/usr/bin", "/bin"],
            )
            .as_deref(),
            Some("/usr/local/bin:/usr/local/sbin:/usr/bin:/bin:/usr/sbin:/sbin"),
        );
    }

    #[test]
    fn a_path_set_by_a_terminal_or_launchctl_is_kept_without_reading_etc() {
        for current in [
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "/usr/bin:/bin:/usr/sbin:/sbin:/Users/me/bin",
            "/usr/local/bin:/usr/bin:/bin",
            // An empty entry searches the working directory: a deliberate PATH.
            "/usr/bin::/bin",
        ] {
            let read_etc = Cell::new(false);
            let path = launch_path(
                Some(OsStr::new(current)),
                || {
                    read_etc.set(true);
                    stock_path_helper_dirs()
                },
                existing(APPLE_SILICON_DIRS),
            );
            assert_eq!(path, None, "{current}");
            assert!(!read_etc.get(), "{current} must not read /etc");
        }
    }

    #[test]
    fn launchds_dirs_in_any_order_or_subset_count_as_launchds_path() {
        assert_eq!(
            launch_path_str(Some("/bin:/usr/bin"), Vec::new(), &["/opt/homebrew/bin"]).as_deref(),
            Some("/opt/homebrew/bin:/bin:/usr/bin"),
        );
    }

    #[test]
    fn an_unset_or_empty_path_keeps_launchds_dirs_after_the_added_ones() {
        for current in [None, Some("")] {
            assert_eq!(
                launch_path_str(current, Vec::new(), &["/opt/homebrew/bin"]).as_deref(),
                Some("/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
                "{current:?}",
            );
        }
    }

    #[test]
    fn nothing_to_add_keeps_the_path_so_the_launch_does_not_restart() {
        assert_eq!(
            launch_path_str(
                Some(LAUNCHD_PATH),
                ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
                &["/usr/bin", "/bin", "/usr/sbin", "/sbin"],
            ),
            None,
        );
    }

    #[test]
    fn the_restarted_process_keeps_the_path_it_was_given() {
        let restarted = launch_path(
            Some(OsStr::new(LAUNCHD_PATH)),
            stock_path_helper_dirs,
            existing(APPLE_SILICON_DIRS),
        )
        .expect("launchd's PATH is replaced");
        assert_eq!(
            launch_path(
                Some(&restarted),
                stock_path_helper_dirs,
                existing(APPLE_SILICON_DIRS)
            ),
            None,
        );
    }

    #[test]
    fn missing_relative_and_colon_holding_dirs_are_not_added() {
        assert_eq!(
            launch_path_str(
                Some(LAUNCHD_PATH),
                [
                    "relative/bin",
                    "/opt/odd:name/bin",
                    "/missing/bin",
                    "/opt/tool/bin"
                ]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
                &["relative/bin", "/opt/odd:name/bin", "/opt/tool/bin"],
            )
            .as_deref(),
            Some("/opt/tool/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
        );
    }

    #[test]
    fn path_helper_dirs_reads_etc_paths_then_paths_d_in_name_order() {
        let etc = tempfile::tempdir().unwrap();
        std::fs::write(etc.path().join("paths"), "/usr/local/bin\n/usr/bin\n\n").unwrap();
        let paths_d = etc.path().join("paths.d");
        std::fs::create_dir(&paths_d).unwrap();
        std::fs::write(paths_d.join("homebrew"), "/opt/homebrew/bin\n").unwrap();
        std::fs::write(paths_d.join("10-cryptex"), "  /var/run/cryptex/bin  \n").unwrap();
        std::fs::create_dir(paths_d.join("00-not-a-file")).unwrap();

        assert_eq!(
            path_helper_dirs(etc.path()),
            [
                "/usr/local/bin",
                "/usr/bin",
                "/var/run/cryptex/bin",
                "/opt/homebrew/bin"
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        );
        assert_eq!(
            path_helper_dirs(&etc.path().join("missing")),
            Vec::<PathBuf>::new()
        );
    }
}
