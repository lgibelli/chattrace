// SPDX-License-Identifier: GPL-3.0-or-later
//! `chattrace-cli`: the command-line tool (same commands as `chattrace`, but a
//! console program on every platform; prints help when run without a command).

fn main() -> std::process::ExitCode {
    chattrace::cli::main(false)
}
