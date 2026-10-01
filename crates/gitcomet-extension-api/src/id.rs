//! Namespaced identifiers.

use std::borrow::Cow;
use std::fmt;

/// An extension's namespace: at least two dot-separated labels of lower-case
/// ASCII letters, digits, `-`, or `_`, such as `com.example.review`.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExtensionId(Cow<'static, str>);

/// A contribution's id: its extension's namespace and a local name, written
/// `<extension>/<local>`.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContributionId {
    extension: ExtensionId,
    local: Cow<'static, str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdError(String);

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdError {}

fn is_label_char(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
}

impl ExtensionId {
    pub fn new(id: impl Into<Cow<'static, str>>) -> Result<Self, IdError> {
        let id = id.into();
        let labels: Vec<&str> = id.split('.').collect();
        let valid = labels.len() >= 2
            && labels
                .iter()
                .all(|label| !label.is_empty() && label.bytes().all(is_label_char));
        if valid {
            Ok(Self(id))
        } else {
            Err(IdError(format!(
                "extension id {id:?} must be dot-separated lower-case labels, like com.example.name"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A contribution id in this namespace.
    pub fn contribution(
        &self,
        local: impl Into<Cow<'static, str>>,
    ) -> Result<ContributionId, IdError> {
        let local = local.into();
        if local.is_empty()
            || !local
                .bytes()
                .all(|byte| is_label_char(byte) || byte == b'.')
        {
            return Err(IdError(format!(
                "contribution name {local:?} must be lower-case letters, digits, '-', '_' or '.'"
            )));
        }
        Ok(ContributionId {
            extension: self.clone(),
            local,
        })
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl ContributionId {
    pub fn extension(&self) -> &ExtensionId {
        &self.extension
    }

    pub fn local(&self) -> &str {
        &self.local
    }
}

impl fmt::Display for ContributionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.extension, self.local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_ids_are_namespaced_lower_case_labels() {
        assert!(ExtensionId::new("com.example.review").is_ok());
        assert!(ExtensionId::new("example.tool_2").is_ok());
        for bad in [
            "",
            "review",
            "Com.Example",
            "com..example",
            "com.example/x",
            "com. example",
        ] {
            assert!(ExtensionId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn contribution_ids_display_with_their_namespace() {
        let ext = ExtensionId::new("com.example.review").unwrap();
        let id = ext.contribution("panel").unwrap();
        assert_eq!(id.to_string(), "com.example.review/panel");
        assert!(ext.contribution("Bad Name").is_err());
        assert!(ext.contribution("").is_err());
    }
}
