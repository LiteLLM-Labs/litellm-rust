use std::{ffi::OsString, io};

use super::{claude::run_claude_wizard, parser::parse_claude_args, ui::print_tool_selector};

pub fn run_tool_selector() -> Result<i32, Box<dyn std::error::Error>> {
    print_tool_selector();

    let mut selection = String::new();
    io::stdin().read_line(&mut selection)?;
    match selection.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "claude" => {
            let args = parse_claude_args(std::iter::empty::<OsString>())?;
            run_claude_wizard(args)
        }
        other => Err(format!("unknown AI tool `{other}`; available tool: claude").into()),
    }
}
