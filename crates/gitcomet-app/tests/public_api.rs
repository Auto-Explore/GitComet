use gitcomet_app::AppMode;
use gitcomet_app::cli::Cli;

#[test]
fn browser_launch_types_keep_their_original_public_fields() {
    let cli = Cli {
        command: None,
        path: Some("example-repository".into()),
    };
    let mode = AppMode::Browser { path: cli.path };
    match mode {
        AppMode::Browser { path } => {
            assert_eq!(path.unwrap(), std::path::Path::new("example-repository"));
        }
        _ => panic!("expected browser mode"),
    }
}
