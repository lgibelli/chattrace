// SPDX-License-Identifier: GPL-3.0-or-later
//! `chattrace`: the desktop app. With no arguments it opens the window; with a
//! command (`chattrace export …`) it behaves like `chattrace-cli` (on Windows
//! use `chattrace-cli.exe` for that, because this exe has no console).
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    chattrace::cli::main(true)
}
