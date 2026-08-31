//! Platform probing shared by the CLI launch guard and the UI's GUI-environment
//! detection.

/// Detects WSL from explicit environment signals or the Linux kernel release.
///
/// Takes the two environment markers as booleans so the caller decides which
/// environment variables count as signals, plus the kernel release string when
/// one was read.
pub fn detect_is_wsl(
    has_wsl_distro_name: bool,
    has_wsl_interop: bool,
    osrelease: Option<&str>,
) -> bool {
    has_wsl_distro_name || has_wsl_interop || osrelease_mentions_microsoft(osrelease)
}

/// Whether a Linux kernel release string identifies a Microsoft (WSL) kernel.
pub fn osrelease_mentions_microsoft(osrelease: Option<&str>) -> bool {
    osrelease
        .map(|value| value.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// Reads the running Linux kernel release from `/proc/sys/kernel/osrelease`;
/// `None` off Linux or when the read fails.
pub fn read_linux_osrelease() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_is_wsl_accepts_environment_and_kernel_markers() {
        assert!(detect_is_wsl(true, false, None));
        assert!(detect_is_wsl(false, true, None));
        assert!(detect_is_wsl(
            false,
            false,
            Some("5.15.90.1-microsoft-standard-WSL2")
        ));
        assert!(detect_is_wsl(
            false,
            false,
            Some("5.15.153.1-Microsoft-standard")
        ));
        assert!(!detect_is_wsl(false, false, Some("6.8.0-generic")));
    }

    #[test]
    fn osrelease_match_is_case_insensitive() {
        assert!(osrelease_mentions_microsoft(Some("MICROSOFT WSL2")));
        assert!(osrelease_mentions_microsoft(Some("foo microsoft bar")));
        assert!(!osrelease_mentions_microsoft(Some("linux 6.8")));
        assert!(!osrelease_mentions_microsoft(None));
    }
}
