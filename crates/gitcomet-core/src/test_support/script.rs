use std::path::Path;

pub fn write_executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, text).expect("write test script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make test script executable");
}

pub fn install_hook(repo: &Path, name: &str, script: &str) {
    write_executable(&repo.join(".git").join("hooks").join(name), script);
}
