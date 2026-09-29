//! Who the running application is: names, identifiers, links, and branding.
//!
//! An application built on these crates installs its identity once, before
//! crash logging, the browser-instance broker, or any directory lookup. Every
//! other caller reads [`current`]; without an installed identity the process is
//! GitComet. Behavior such as history filtering is not identity: it belongs in
//! the options of the service it changes.

use std::borrow::Cow;
use std::fmt;
use std::sync::OnceLock;

type Text = Cow<'static, str>;

/// Names, identifiers, links, and branding for one application.
///
/// Fields are private so later additions do not break downstream builders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductIdentity {
    display_name: Text,
    executable_name: Text,
    directory_name: Text,
    app_id: Text,
    macos_bundle_id: Text,
    version: Text,
    git_tool_name: Text,
    links: ProductLinks,
    update_source: UpdateSource,
    branding: ProductBranding,
}

/// Web pages the application points users at. `None` hides the entry point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductLinks {
    pub website: Option<Text>,
    /// The page comparing editions, linked from About and the status bar.
    pub editions: Option<Text>,
    /// Community chat.
    pub community: Option<Text>,
    pub repository: Option<Text>,
    /// Where a prefilled crash report is filed; the query string is appended.
    pub new_issue: Option<Text>,
    pub releases: Option<Text>,
    pub license: Option<NamedLink>,
    /// Base of the user documentation; guides are `<base>/<page>.md`.
    pub documentation: Option<Text>,
    pub survey: Option<SurveyLink>,
}

impl ProductLinks {
    /// A documentation page, e.g. `themes` or `commit-signatures`.
    pub fn documentation_page(&self, page: &str) -> Option<String> {
        let base = self.documentation.as_deref()?.trim_end_matches('/');
        Some(format!("{base}/{page}.md"))
    }
}

/// A link shown by name, such as a license.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedLink {
    pub name: Text,
    pub url: Text,
}

/// A one-off user survey, prompted once per `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurveyLink {
    pub id: Text,
    pub message: Text,
    pub url: Text,
}

/// Where the update check looks for newer releases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateSource {
    Disabled,
    GitHubReleases { owner: Text, repo: Text },
}

/// Product artwork. `None` uses GitComet's artwork, which the UI embeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProductBranding {
    /// Application icon (PNG, at least 512 px): app bundles and launchers.
    pub app_icon_png: Option<&'static [u8]>,
    /// Small window icon (PNG).
    pub window_icon_png: Option<&'static [u8]>,
    /// Logo drawn on the splash and home screens (SVG).
    pub logo_svg: Option<&'static [u8]>,
}

/// The kinds of top-level window, each with its own desktop app id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    FocusedMergetool,
    FocusedDiff,
    Settings,
}

/// Why a builder was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityError(String);

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdentityError {}

const GITCOMET_REPOSITORY: &str = "https://github.com/Auto-Explore/GitComet";

impl ProductIdentity {
    /// GitComet's own identity.
    pub fn gitcomet() -> Self {
        Self {
            display_name: Cow::Borrowed("GitComet"),
            executable_name: Cow::Borrowed("gitcomet"),
            directory_name: Cow::Borrowed("gitcomet"),
            app_id: Cow::Borrowed("gitcomet"),
            macos_bundle_id: Cow::Borrowed("ai.autoexplore.gitcomet"),
            version: Cow::Borrowed(env!("CARGO_PKG_VERSION")),
            git_tool_name: Cow::Borrowed("gitcomet"),
            links: ProductLinks {
                website: Some(Cow::Borrowed("https://gitcomet.dev")),
                editions: Some(Cow::Borrowed("https://gitcomet.dev/#editions")),
                community: Some(Cow::Borrowed("https://discord.com/invite/2ufDGP8RnA")),
                repository: Some(Cow::Borrowed(GITCOMET_REPOSITORY)),
                license: Some(NamedLink {
                    name: Cow::Borrowed("AGPL-3.0"),
                    url: Cow::Borrowed(concat!(
                        "https://github.com/Auto-Explore/GitComet",
                        "/blob/main/LICENSE-AGPL-3.0"
                    )),
                }),
                documentation: Some(Cow::Borrowed(concat!(
                    "https://github.com/Auto-Explore/GitComet",
                    "/blob/main/docs"
                ))),
                new_issue: Some(Cow::Borrowed(concat!(
                    "https://github.com/Auto-Explore/GitComet",
                    "/issues/new"
                ))),
                releases: Some(Cow::Borrowed(concat!(
                    "https://github.com/Auto-Explore/GitComet",
                    "/releases"
                ))),
                survey: Some(SurveyLink {
                    id: Cow::Borrowed("gitcomet_user_survey_2026_04"),
                    message: Cow::Borrowed("Help shape GitComet by taking a short user survey."),
                    url: Cow::Borrowed(
                        "https://docs.google.com/forms/d/e/1FAIpQLSd8DKIl222UomSXrpv1q9rWodRlBSQo9pJDD62GbZEANTgD1A/viewform",
                    ),
                }),
            },
            update_source: UpdateSource::GitHubReleases {
                owner: Cow::Borrowed("Auto-Explore"),
                repo: Cow::Borrowed("GitComet"),
            },
            branding: ProductBranding::default(),
        }
    }

    /// Starts a new identity. `executable_name` also becomes the default
    /// directory name, desktop app id, and Git tool name; links, updates, and
    /// the survey start disabled.
    pub fn builder(
        display_name: impl Into<Text>,
        executable_name: impl Into<Text>,
    ) -> ProductIdentityBuilder {
        let executable_name = executable_name.into();
        ProductIdentityBuilder {
            identity: Self {
                display_name: display_name.into(),
                directory_name: executable_name.clone(),
                app_id: executable_name.clone(),
                macos_bundle_id: Cow::Owned(String::new()),
                version: Cow::Borrowed("0.0.0"),
                git_tool_name: executable_name.clone(),
                executable_name,
                links: ProductLinks::default(),
                update_source: UpdateSource::Disabled,
                branding: ProductBranding::default(),
            },
        }
    }

    /// Product name in window titles, menus, and messages.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Command name: CLI help, launchers, and the macOS bundle executable.
    pub fn executable_name(&self) -> &str {
        &self.executable_name
    }

    /// Per-user state, data, and crash directories are named after this.
    pub fn directory_name(&self) -> &str {
        &self.directory_name
    }

    /// Desktop app id of the main window, also the launcher and icon name.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn window_app_id(&self, kind: WindowKind) -> String {
        match kind {
            WindowKind::Main => self.app_id.to_string(),
            WindowKind::FocusedMergetool => format!("{}-mergetool", self.app_id),
            WindowKind::FocusedDiff => format!("{}-diff", self.app_id),
            WindowKind::Settings => format!("{}-settings", self.app_id),
        }
    }

    /// File name of the Linux launcher entry.
    pub fn desktop_file_name(&self) -> String {
        format!("{}.desktop", self.app_id)
    }

    pub fn macos_bundle_id(&self) -> &str {
        &self.macos_bundle_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// `git difftool`/`git mergetool` name of the terminal tools.
    pub fn git_tool_name(&self) -> &str {
        &self.git_tool_name
    }

    /// Tool name of the windowed variant, selected by `guiDefault=auto`.
    pub fn git_gui_tool_name(&self) -> String {
        format!("{}-gui", self.git_tool_name)
    }

    pub fn links(&self) -> &ProductLinks {
        &self.links
    }

    pub fn update_source(&self) -> &UpdateSource {
        &self.update_source
    }

    pub fn branding(&self) -> ProductBranding {
        self.branding
    }
}

/// Builds a [`ProductIdentity`]; [`build`](Self::build) validates the names.
#[derive(Clone, Debug)]
pub struct ProductIdentityBuilder {
    identity: ProductIdentity,
}

impl ProductIdentityBuilder {
    pub fn directory_name(mut self, name: impl Into<Text>) -> Self {
        self.identity.directory_name = name.into();
        self
    }

    pub fn app_id(mut self, app_id: impl Into<Text>) -> Self {
        self.identity.app_id = app_id.into();
        self
    }

    pub fn macos_bundle_id(mut self, bundle_id: impl Into<Text>) -> Self {
        self.identity.macos_bundle_id = bundle_id.into();
        self
    }

    pub fn version(mut self, version: impl Into<Text>) -> Self {
        self.identity.version = version.into();
        self
    }

    pub fn git_tool_name(mut self, name: impl Into<Text>) -> Self {
        self.identity.git_tool_name = name.into();
        self
    }

    pub fn links(mut self, links: ProductLinks) -> Self {
        self.identity.links = links;
        self
    }

    pub fn update_source(mut self, source: UpdateSource) -> Self {
        self.identity.update_source = source;
        self
    }

    pub fn branding(mut self, branding: ProductBranding) -> Self {
        self.identity.branding = branding;
        self
    }

    pub fn build(self) -> Result<ProductIdentity, IdentityError> {
        let mut identity = self.identity;
        if identity.display_name.trim().is_empty() {
            return Err(IdentityError("display name is empty".into()));
        }
        for (what, value) in [
            ("executable name", &identity.executable_name),
            ("directory name", &identity.directory_name),
            ("app id", &identity.app_id),
            ("Git tool name", &identity.git_tool_name),
        ] {
            if !is_plain_name(value) {
                return Err(IdentityError(format!(
                    "{what} {value:?} must be non-empty ASCII letters, digits, '-', '_' or '.', \
                     not starting with '.' or '-'"
                )));
            }
        }
        if identity.macos_bundle_id.is_empty() {
            let id = identity.app_id.replace('_', "-");
            identity.macos_bundle_id = Cow::Owned(format!("app.{id}"));
        } else if !is_reverse_dns(&identity.macos_bundle_id) {
            return Err(IdentityError(format!(
                "macOS bundle id {:?} must be reverse-DNS",
                identity.macos_bundle_id
            )));
        }
        Ok(identity)
    }
}

/// Safe as a path component, a desktop id, and a Git config subsection.
fn is_plain_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['.', '-'])
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_reverse_dns(value: &str) -> bool {
    let mut labels = 0;
    for label in value.split('.') {
        if label.is_empty()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return false;
        }
        labels += 1;
    }
    labels >= 2
}

/// A write-once identity slot; reading it first freezes the default.
struct IdentitySlot(OnceLock<ProductIdentity>);

impl IdentitySlot {
    const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn install(&self, identity: ProductIdentity) -> Result<(), Box<ProductIdentity>> {
        self.0.set(identity).map_err(Box::new)
    }

    fn current(&self) -> &ProductIdentity {
        self.0.get_or_init(ProductIdentity::gitcomet)
    }
}

static IDENTITY: IdentitySlot = IdentitySlot::new();

/// Installs the process identity. Fails, returning `identity`, when one was
/// installed already or [`current`] was read first: anything resolved before
/// the install would disagree with it.
pub fn install(identity: ProductIdentity) -> Result<(), Box<ProductIdentity>> {
    IDENTITY.install(identity)
}

/// The installed identity, or GitComet's.
pub fn current() -> &'static ProductIdentity {
    IDENTITY.current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitcomet_identity_keeps_the_historical_names() {
        let identity = ProductIdentity::gitcomet();
        assert_eq!(identity.display_name(), "GitComet");
        assert_eq!(identity.executable_name(), "gitcomet");
        assert_eq!(identity.directory_name(), "gitcomet");
        assert_eq!(identity.window_app_id(WindowKind::Main), "gitcomet");
        assert_eq!(
            identity.window_app_id(WindowKind::FocusedMergetool),
            "gitcomet-mergetool"
        );
        assert_eq!(
            identity.window_app_id(WindowKind::FocusedDiff),
            "gitcomet-diff"
        );
        assert_eq!(
            identity.window_app_id(WindowKind::Settings),
            "gitcomet-settings"
        );
        assert_eq!(identity.desktop_file_name(), "gitcomet.desktop");
        assert_eq!(identity.git_tool_name(), "gitcomet");
        assert_eq!(identity.git_gui_tool_name(), "gitcomet-gui");
        assert_eq!(identity.macos_bundle_id(), "ai.autoexplore.gitcomet");
        assert_eq!(identity.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            identity.links().new_issue.as_deref(),
            Some(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        );
        assert_eq!(
            identity.links().repository.as_deref(),
            Some(env!("CARGO_PKG_REPOSITORY"))
        );
        assert_eq!(
            identity.links().documentation_page("themes").as_deref(),
            Some("https://github.com/Auto-Explore/GitComet/blob/main/docs/themes.md")
        );
    }

    #[test]
    fn builder_derives_names_from_the_executable_and_disables_links() {
        let identity = ProductIdentity::builder("Comet Pro", "comet-pro")
            .version("1.2.3")
            .build()
            .unwrap();
        assert_eq!(identity.directory_name(), "comet-pro");
        assert_eq!(identity.app_id(), "comet-pro");
        assert_eq!(identity.git_gui_tool_name(), "comet-pro-gui");
        assert_eq!(identity.macos_bundle_id(), "app.comet-pro");
        assert_eq!(identity.version(), "1.2.3");
        assert_eq!(identity.links(), &ProductLinks::default());
        assert_eq!(identity.update_source(), &UpdateSource::Disabled);
    }

    #[test]
    fn builder_rejects_names_that_cannot_be_paths_or_config_keys() {
        for bad in ["", "a/b", "..", ".hidden", "-flag", "has space", "é"] {
            assert!(
                ProductIdentity::builder("X", bad).build().is_err(),
                "{bad:?} must be rejected"
            );
        }
        assert!(
            ProductIdentity::builder(" ", "x").build().is_err(),
            "empty display name"
        );
        assert!(
            ProductIdentity::builder("X", "x")
                .macos_bundle_id("nodots")
                .build()
                .is_err()
        );
        assert!(
            ProductIdentity::builder("X", "x")
                .macos_bundle_id("com.example.x")
                .build()
                .is_ok()
        );
    }

    #[test]
    fn a_slot_accepts_one_install_and_none_after_it_is_read() {
        let installed = IdentitySlot::new();
        let pro = ProductIdentity::builder("Pro", "pro").build().unwrap();
        assert!(installed.install(pro.clone()).is_ok());
        assert_eq!(installed.current(), &pro);
        assert!(installed.install(ProductIdentity::gitcomet()).is_err());

        let read_first = IdentitySlot::new();
        assert_eq!(read_first.current(), &ProductIdentity::gitcomet());
        assert_eq!(read_first.install(pro.clone()), Err(Box::new(pro)));
    }
}
