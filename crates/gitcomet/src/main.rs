// Use the GUI subsystem on Windows for packaged builds so launching
// `gitcomet.exe` from Explorer or Start Menu shortcuts does not create a
// separate console window. Keep debug/test builds attached to the invoking
// console so subprocess-heavy Windows test runs do not pop extra terminals.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

use rustfs_mimalloc::MiMalloc;

/// This covers Rust only. tree-sitter is C and calls `malloc`, which resolves to
/// libc unless it is told otherwise, so it needs pointing at mimalloc separately
/// -- but *not* from here. `gitcomet-ui-gpui` does it from the lazy initialisers
/// that front every parser and query it builds, which covers every binary and
/// every test rather than this one entry point. See
/// `gitcomet_tree_sitter_alloc::install_mimalloc_allocator`.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> ! {
    gitcomet_app::App::new().run()
}
