//! A neutral product built on unmodified GitComet crates.
//!
//! It proves the downstream boundary: everything here goes through public
//! upstream interfaces, never private UI modules or source patches.

use gitcomet_core::identity::{ProductIdentity, ProductLinks};

/// The example product's name, used for its windows, paths, and Git tools.
pub const DISPLAY_NAME: &str = "Comet Example";
/// Its command, directory, desktop app id, and Git tool name.
pub const EXECUTABLE_NAME: &str = "comet-example";

pub fn identity() -> ProductIdentity {
    ProductIdentity::builder(DISPLAY_NAME, EXECUTABLE_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .macos_bundle_id("com.example.comet-example")
        .links(ProductLinks::default())
        .build()
        .expect("the example identity is valid")
}

#[cfg(test)]
mod tests {
    #[test]
    fn identity_is_neutral() {
        let identity = super::identity();
        assert_eq!(identity.display_name(), "Comet Example");
        assert_eq!(identity.directory_name(), "comet-example");
        assert_eq!(identity.git_tool_name(), "comet-example");
        assert!(identity.links().repository.is_none());
    }
}
