//! The `heimdall` executable.
//!
//! Two adapters over one core: `shell` for direct commands and the desktop
//! bridge, `mcp` for AI clients. Neither implements domain behavior — both
//! translate into `heimdall-core`.

mod envelope;
mod mcp;
mod shell;

fn main() -> std::process::ExitCode {
    shell::run()
}
