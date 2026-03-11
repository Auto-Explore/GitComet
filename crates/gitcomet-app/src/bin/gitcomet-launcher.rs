#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
    use std::env;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // Prevent the console-subsystem app from creating a visible terminal window
    // when launched from Start Menu shortcuts.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(current_exe) = env::current_exe() else {
        return;
    };
    let Some(install_dir) = current_exe.parent() else {
        return;
    };
    let app_exe = install_dir.join("gitcomet-app.exe");

    let mut cmd = Command::new(app_exe);
    cmd.current_dir(install_dir);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(env::args_os().skip(1));

    let _ = cmd.spawn();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("gitcomet-launcher is only supported on Windows");
    std::process::exit(1);
}
