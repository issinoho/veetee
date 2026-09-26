//! `vt-headless` — drives the veetee emulator without a GUI.

use std::process::ExitCode;

mod golden;
mod kermit;
mod lat;
mod replay;
mod script;
mod trace;

const USAGE: &str = "\
usage:
  vt-headless trace [--7bit | --8bit | --no-c1 | --utf8] [--vt52] [FILE]
      Print each parser action for FILE (or stdin), one per line.

  vt-headless run SCRIPT [--golden DIR] [--bless] [--record FILE]
      Run a session script against the emulator on a PTY. Snapshots are
      compared with DIR/NAME.screen; --bless rewrites them.

  vt-headless lat INTERFACE [SECONDS] [--connect NODE] [--type TEXT]
      Print the LAT messages heard on INTERFACE; --connect asks a node for a
      circuit once it has announced itself, and --type sends a line to the
      session it opens. Linux only, and needs
      CAP_NET_RAW: LAT is raw Ethernet rather than IP.

  vt-headless kermit receive [--into DIR] CONNECTION [OPTIONS]
  vt-headless kermit send FILE... CONNECTION [OPTIONS]
      Transfer files with Kermit over --command, --telnet, --ssh or --serial.
      `vt-headless kermit` alone lists the options.

  vt-headless replay FILE.vtrec [--golden DIR] [--bless]
      Play a veetee session recording through the emulator and compare the
      screen at each checkpoint, and at the end as `final`, with DIR (default:
      the recording's name without .vtrec).

  vt-headless -v | --version
  vt-headless -h | --help";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("trace") => trace::trace(args).map(|()| true),
        Some("run") => script::run(args),
        Some("lat") => lat::lat(args).map(|()| true),
        Some("replay") => replay::replay(args),
        Some("kermit") => kermit::kermit(args).map_err(std::io::Error::other),
        Some("-v" | "--version") => {
            println!("vt-headless {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
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

#[cfg(test)]
mod tests {
    /// The manual page describes every option the help texts list.
    #[test]
    fn the_manual_page_has_every_option() {
        let page = include_str!("../../../data/vt-headless.1").replace("\\-", "-");
        for usage in [super::USAGE, super::kermit::USAGE] {
            for word in usage.split_whitespace() {
                let option = word
                    .trim_matches(|c| matches!(c, '[' | ']' | ',' | '|' | '.' | '"' | '(' | ')'));
                if option.starts_with('-') && option.len() > 1 {
                    assert!(
                        page.contains(option),
                        "{option} is not in data/vt-headless.1"
                    );
                }
            }
        }
    }
}
