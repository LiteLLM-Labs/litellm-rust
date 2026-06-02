use std::process::Command;

pub(crate) fn disable_terminal_echo() -> bool {
    Command::new("stty")
        .arg("-echo")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn enable_terminal_echo() -> bool {
    Command::new("stty")
        .arg("echo")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
