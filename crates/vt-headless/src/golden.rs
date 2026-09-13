//! Golden screen files: compare a screen dump with `DIR/NAME.screen`.

use std::fs;
use std::io;
use std::path::Path;

/// Compares `actual` with the golden file. With `bless` a differing or
/// missing file is rewritten. Returns whether the screen matched (or was
/// blessed); on a difference the actual screen is saved as `NAME.screen.new`.
pub fn compare(dir: &Path, name: &str, actual: &str, bless: bool) -> io::Result<bool> {
    let path = dir.join(format!("{name}.screen"));
    let expected = fs::read_to_string(&path).ok();
    if expected.as_deref() == Some(actual) {
        return Ok(true);
    }
    if bless {
        fs::create_dir_all(dir)?;
        fs::write(&path, actual)?;
        eprintln!("blessed {}", path.display());
        return Ok(true);
    }
    match expected {
        None => eprintln!(
            "MISSING {} (run with --bless to create)\n{actual}",
            path.display()
        ),
        Some(expected) => {
            eprintln!("DIFFERS {}", path.display());
            for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
                if e != a {
                    eprintln!("  line {:3} expected: {e}\n             actual: {a}", i + 1);
                }
            }
            let new = path.with_extension("screen.new");
            fs::write(&new, actual)?;
        }
    }
    Ok(false)
}
