use gitcomet_core::platform::dirs;

fn main() {
    let launch = gitcomet_app::AppLaunch::new(gitcomet_extension_example::identity())
        .about("An example product built on unmodified GitComet crates")
        .extension(gitcomet_extension_example::review::ReviewExtension);

    // Lets the integration tests see which directories the identity selects
    // without opening a window.
    if std::env::var_os("COMET_EXAMPLE_PRINT_DIRS").is_some() {
        gitcomet_core::identity::install(gitcomet_extension_example::identity())
            .expect("nothing resolved the identity yet");
        for (label, dir) in [
            ("state", dirs::state_dir()),
            ("data", dirs::data_dir()),
            ("crash", dirs::crash_dir()),
        ] {
            println!(
                "{label}={}",
                dir.map(|d| d.display().to_string()).unwrap_or_default()
            );
        }
        return;
    }

    std::process::exit(launch.run());
}
