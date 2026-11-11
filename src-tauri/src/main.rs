// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same binary is the `pipescope` command when it's given arguments.
    if let Some(code) = pipescope_lib::cli::run_if_cli() {
        std::process::exit(code);
    }
    pipescope_lib::run()
}
