// These snapshots and the patch come from bd8b4a04 and its first parent.
// Keep the original contents: the regressions assert exact source line numbers.
// Embedding them makes the tests independent of Git history and checkout depth.
pub(super) const COMMIT_PATCH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test_data/commit-bd8b4a04.patch"
));

pub(super) struct YamlFixture {
    pub path: &'static str,
    pub old_text: &'static str,
    pub new_text: &'static str,
}

impl YamlFixture {
    pub fn unified_diff(&self) -> &'static str {
        let header = format!("diff --git a/{} b/{}", self.path, self.path);
        let start = COMMIT_PATCH
            .find(&format!("{header}\n"))
            .or_else(|| COMMIT_PATCH.find(&format!("{header}\r\n")))
            .expect("YAML fixture must have a section in the commit patch");
        let section = &COMMIT_PATCH[start..];
        let end = section
            .find("\ndiff --git ")
            .map_or(section.len(), |offset| offset + 1);
        &section[..end]
    }
}

pub(super) const DEPLOYMENT_CI: YamlFixture = YamlFixture {
    path: ".github/workflows/deployment-ci.yml",
    old_text: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test_data/commit-bd8b4a04/deployment-ci.before.yml"
    )),
    new_text: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test_data/commit-bd8b4a04/deployment-ci.after.yml"
    )),
};

pub(super) const BUILD_RELEASE_ARTIFACTS: YamlFixture = YamlFixture {
    path: ".github/workflows/build-release-artifacts.yml",
    old_text: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test_data/commit-bd8b4a04/build-release-artifacts.before.yml"
    )),
    new_text: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test_data/commit-bd8b4a04/build-release-artifacts.after.yml"
    )),
};
