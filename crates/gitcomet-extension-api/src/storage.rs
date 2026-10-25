//! Limits on the state an extension saves in the session file.
//!
//! Each extension owns one namespace in the session and one in each saved
//! workspace. The host keeps namespaces verbatim through every ordinary
//! session update, so an extension's data survives releases that do not load
//! it, and an unreadable namespace never hides the host's own settings.

use gitcomet_state::session::ExtensionNamespaceError;
use std::fmt;

/// The largest serialized value one namespace may hold.
pub use gitcomet_state::session::MAX_EXTENSION_NAMESPACE_BYTES as MAX_NAMESPACE_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageError {
    /// The value serializes to more than [`MAX_NAMESPACE_BYTES`].
    TooLarge { bytes: usize },
    /// The session could not be written.
    Write(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "extension state is {bytes} bytes; the limit is {MAX_NAMESPACE_BYTES}"
            ),
            Self::Write(error) => write!(f, "could not save extension state: {error}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<ExtensionNamespaceError> for StorageError {
    fn from(error: ExtensionNamespaceError) -> Self {
        match error {
            ExtensionNamespaceError::TooLarge { bytes } => Self::TooLarge { bytes },
            ExtensionNamespaceError::Io(error) => Self::Write(error.to_string()),
        }
    }
}

/// Checks `value` against the namespace limit.
pub fn check_namespace_size(value: &serde_json::Value) -> Result<(), StorageError> {
    gitcomet_state::session::check_extension_namespace_size(value).map_err(StorageError::from)
}

/// The extension's session-wide state.
pub fn load(extension: &crate::ExtensionId) -> Option<serde_json::Value> {
    gitcomet_state::session::extension_namespace(extension.as_str())
}

/// An extension's private data directory. The caller creates it when needed.
pub fn storage_dir(extension: &crate::ExtensionId) -> Option<std::path::PathBuf> {
    Some(
        gitcomet_core::platform::dirs::data_dir()?
            .join("extensions")
            .join(extension.as_str()),
    )
}

/// Saves (`Some`) or clears (`None`) the extension's session-wide state.
pub fn save(
    extension: &crate::ExtensionId,
    value: Option<serde_json::Value>,
) -> Result<(), StorageError> {
    gitcomet_state::session::persist_extension_namespace(extension.as_str(), value)
        .map_err(StorageError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_up_to_the_limit_are_accepted() {
        let small = serde_json::json!({ "open": true });
        assert_eq!(check_namespace_size(&small), Ok(()));
        let big = serde_json::Value::String("x".repeat(MAX_NAMESPACE_BYTES));
        assert!(matches!(
            check_namespace_size(&big),
            Err(StorageError::TooLarge { .. })
        ));
    }
}
