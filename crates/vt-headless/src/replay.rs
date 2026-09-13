//! `replay` subcommand: plays a `.vtrec` session recording through the
//! emulator and compares the screen at each checkpoint and at the end.

use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;

use vt_core::Terminal;
use vt_core::dump::dump;
use vt_core::recording::{self, Record};

use crate::golden;

pub fn replay(mut args: impl Iterator<Item = String>) -> io::Result<bool> {
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidInput, msg);
    let mut file = None;
    let mut golden_dir = None;
    let mut bless = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--golden" => {
                golden_dir =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        invalid("--golden needs a directory".into())
                    })?))
            }
            "--bless" => bless = true,
            _ if file.is_none() && !arg.starts_with("--") => file = Some(PathBuf::from(arg)),
            _ => return Err(invalid(format!("unexpected argument {arg:?}"))),
        }
    }
    let file = file.ok_or_else(|| invalid("replay needs a .vtrec file".into()))?;
    // By default snapshots live next to the recording: session.vtrec -> session/.
    let golden_dir = golden_dir.unwrap_or_else(|| file.with_extension(""));

    let (config, records) = recording::read(BufReader::new(File::open(&file)?))?;
    let mut term = Terminal::new(config);
    let mut ok = true;
    let mut marks = 0;
    for record in &records {
        match record {
            Record::Host { data, .. } => {
                term.advance(data);
                // Replies are what the terminal would send; they are not checked.
                term.take_output();
            }
            Record::Mark { name, .. } => {
                marks += 1;
                ok &= golden::compare(&golden_dir, name, &dump(&term), bless)?;
            }
            Record::Reply { .. } | Record::Keys { .. } => {}
        }
    }
    ok &= golden::compare(&golden_dir, "final", &dump(&term), bless)?;
    eprintln!(
        "{} {} ({} records, {marks} checkpoints)",
        if ok { "PASS" } else { "FAIL" },
        file.display(),
        records.len()
    );
    Ok(ok)
}
