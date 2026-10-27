use std::path::Path;

pub(crate) fn write_executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, text).expect("write test script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make test script executable");
}
