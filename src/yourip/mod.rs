mod rip;

mod ffmpeg;

mod executor;
pub use executor::{Executor, Media};

/// Stops Windows from flashing a console window each time Rippy runs a tool.
fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
