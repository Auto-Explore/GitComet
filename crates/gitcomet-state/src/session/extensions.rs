//! Extension-owned data in the session file and in each workspace record.
//!
//! The data is kept verbatim: any JSON loads, so a malformed namespace never
//! makes ordinary settings unreadable, and every ordinary session update
//! carries the namespaces through untouched (see `update_session_file`).
//! Only explicit namespace writes are validated.

use super::{SessionUpdate, default_session_file_path, load_file, update_session_file};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

/// The largest serialized value one namespace may hold.
pub const MAX_EXTENSION_NAMESPACE_BYTES: usize = 64 * 1024;

/// One JSON value per extension id.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ExtensionNamespaces(Option<serde_json::Value>);

#[derive(Debug)]
pub enum ExtensionNamespaceError {
    /// The value serializes to more than [`MAX_EXTENSION_NAMESPACE_BYTES`].
    TooLarge {
        bytes: usize,
    },
    Io(io::Error),
}

impl fmt::Display for ExtensionNamespaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "extension state is {bytes} bytes; the limit is {MAX_EXTENSION_NAMESPACE_BYTES}"
            ),
            Self::Io(error) => write!(f, "could not save extension state: {error}"),
        }
    }
}

impl std::error::Error for ExtensionNamespaceError {}

impl From<io::Error> for ExtensionNamespaceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Checks `value` against the namespace size limit.
pub fn check_extension_namespace_size(
    value: &serde_json::Value,
) -> Result<(), ExtensionNamespaceError> {
    let bytes = serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_EXTENSION_NAMESPACE_BYTES {
        Err(ExtensionNamespaceError::TooLarge { bytes })
    } else {
        Ok(())
    }
}

impl ExtensionNamespaces {
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            None => true,
            Some(serde_json::Value::Object(map)) => map.is_empty(),
            Some(_) => false,
        }
    }

    /// `namespace`'s value; `None` when absent or when the stored namespaces
    /// are not a JSON object.
    pub fn get(&self, namespace: &str) -> Option<&serde_json::Value> {
        match &self.0 {
            Some(serde_json::Value::Object(map)) => map.get(namespace),
            _ => None,
        }
    }

    /// Sets (`Some`) or removes (`None`) `namespace`. A malformed store is
    /// replaced by an object holding only this namespace: nothing could read
    /// it anyway.
    pub fn set(
        &mut self,
        namespace: &str,
        value: Option<serde_json::Value>,
    ) -> Result<(), ExtensionNamespaceError> {
        if let Some(value) = &value {
            check_extension_namespace_size(value)?;
        }
        if !matches!(self.0, Some(serde_json::Value::Object(_))) {
            self.0 = Some(serde_json::Value::Object(serde_json::Map::new()));
        }
        let Some(serde_json::Value::Object(map)) = &mut self.0 else {
            unreachable!("replaced above");
        };
        match value {
            Some(value) => {
                map.insert(namespace.to_string(), value);
            }
            None => {
                map.remove(namespace);
            }
        }
        if map.is_empty() {
            self.0 = None;
        }
        Ok(())
    }

    pub(super) fn is_absent(&self) -> bool {
        self.0.is_none()
    }
}

impl PartialEq for ExtensionNamespaces {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ExtensionNamespaces {}

impl Hash for ExtensionNamespaces {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0
            .as_ref()
            .map(serde_json::Value::to_string)
            .hash(state);
    }
}

/// An extension's session-wide state.
pub fn extension_namespace(namespace: &str) -> Option<serde_json::Value> {
    let path = default_session_file_path()?;
    extension_namespace_from_path(namespace, &path)
}

pub fn extension_namespace_from_path(namespace: &str, path: &Path) -> Option<serde_json::Value> {
    load_file(path)?.extensions.get(namespace).cloned()
}

/// Saves (`Some`) or clears (`None`) an extension's session-wide state.
pub fn persist_extension_namespace(
    namespace: &str,
    value: Option<serde_json::Value>,
) -> Result<(), ExtensionNamespaceError> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_extension_namespace_to_path(namespace, value, &path)
}

pub fn persist_extension_namespace_to_path(
    namespace: &str,
    value: Option<serde_json::Value>,
    path: &Path,
) -> Result<(), ExtensionNamespaceError> {
    if let Some(value) = &value {
        check_extension_namespace_size(value)?;
    }
    let mut result = Ok(());
    update_session_file(path, |file| match file.extensions.set(namespace, value) {
        Ok(()) => SessionUpdate::Write,
        Err(error) => {
            result = Err(error);
            SessionUpdate::Unchanged
        }
    })?;
    result
}

#[cfg(test)]
mod tests;
