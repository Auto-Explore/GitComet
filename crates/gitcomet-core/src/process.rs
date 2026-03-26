use std::ffi::OsStr;
use std::process::Command;

/// Create a background subprocess command preconfigured to avoid creating a
/// visible console window on Windows.
pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    configure_background_command(&mut command);
    command
}

/// Configure a background subprocess so it does not create a visible console
/// window on Windows when GitComet is running as a GUI-subsystem app.
pub fn configure_background_command(command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}
