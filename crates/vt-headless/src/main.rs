//! `vt-headless` — drives the veetee emulator without a GUI.

use std::process::ExitCode;

mod script;
mod trace;

const USAGE: &str = "\
usage:
  vt-headless trace [--7bit | --8bit | --no-c1 | --utf8] [--vt52] [FILE]
      Print each parser action for FILE (or stdin), one per line.

  vt-headless run SCRIPT [--golden DIR] [--bless] [--record FILE]
      Run a session script against the emulator on a PTY. Snapshots are
      compared with DIR/NAME.screen; --bless rewrites them.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("trace") => trace::trace(args).map(|()| true),
        Some("run") => script::run(args),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("vt-headless: {e}");
            ExitCode::FAILURE
        }
    }
}
