//! `cargo xtask` — developer tasks for veetee.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  vttest [--bless] [MODEL/MENU ...]
      Fetch and build the pinned vttest, then run the scripted conformance
      suite in tests/conformance/vttest. Arguments select scripts, e.g.
      `vt102/menu2`. --bless rewrites golden snapshots.";

/// Pinned vttest release. Update both together.
const VTTEST_VERSION: &str = "20251205";
const VTTEST_SHA256: &str = "cd6886f9aefe6a3f6c566fa61271a55710901a71849c630bf5376aa984bf77cc";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("vttest") => vttest(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to run {cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed with {status}"))
    }
}

fn vttest(args: &[String]) -> Result<()> {
    let bless = args.iter().any(|a| a == "--bless");
    let filters: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();

    let vttest = build_vttest()?;
    run(Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "vt-headless"])
        .current_dir(root()))?;
    let headless = root().join("target/debug/vt-headless");

    let suite = root().join("tests/conformance/vttest");
    let mut scripts = Vec::new();
    for model in read_dir_sorted(&suite)? {
        if model.is_dir() {
            for script in read_dir_sorted(&model)? {
                if script.extension().is_some_and(|e| e == "vtscript") {
                    scripts.push(script);
                }
            }
        }
    }

    let mut failed = Vec::new();
    for script in &scripts {
        let name = script
            .strip_prefix(&suite)
            .unwrap()
            .with_extension("")
            .display()
            .to_string();
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f)) {
            continue;
        }
        let mut cmd = Command::new(&headless);
        cmd.arg("run").arg(script).env("VTTEST", &vttest);
        if bless {
            cmd.arg("--bless");
        }
        let ok = cmd.status().map_err(|e| e.to_string())?.success();
        println!("{} {name}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failed.push(name);
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "vttest conformance failures: {}",
            failed.join(", ")
        ))
    }
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    Ok(entries)
}

/// Downloads, verifies and builds vttest under target/conformance. Returns the binary path.
fn build_vttest() -> Result<PathBuf> {
    let work = root().join("target/conformance");
    let src = work.join(format!("vttest-{VTTEST_VERSION}"));
    let binary = src.join("vttest");
    if binary.exists() {
        return Ok(binary);
    }
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let tarball = work.join(format!("vttest-{VTTEST_VERSION}.tgz"));
    let url = format!("https://invisible-island.net/archives/vttest/vttest-{VTTEST_VERSION}.tgz");
    eprintln!("fetching {url}");
    run(Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&tarball)
        .arg(&url))?;

    let output = Command::new("sha256sum")
        .arg(&tarball)
        .output()
        .map_err(|e| e.to_string())?;
    let digest = String::from_utf8_lossy(&output.stdout);
    if !digest.starts_with(VTTEST_SHA256) {
        let _ = fs::remove_file(&tarball);
        return Err(format!("checksum mismatch for {url}: got {digest}"));
    }

    run(Command::new("tar")
        .arg("xzf")
        .arg(&tarball)
        .current_dir(&work))?;
    run(Command::new("./configure").arg("--quiet").current_dir(&src))?;
    run(Command::new("make").arg("--silent").current_dir(&src))?;
    Ok(binary)
}
