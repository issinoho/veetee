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
      `vt102/menu2`. --bless rewrites golden snapshots.
  esctest [--update] [REGEX]
      Fetch the pinned esctest2 and run it against a VT525 with xterm
      compatibility enabled. Failures must match
      tests/conformance/esctest/expected-failures.txt, where every entry
      gives the DEC reason for the difference. --update rewrites the list,
      keeping existing reasons.
  openvms [--bless] [NAME ...]
      Replay the session recordings in tests/conformance/openvms and compare
      each checkpoint screen with NAME/CHECKPOINT.screen.
  dist [--no-deb]
      Build release binaries and package them under target/dist: a
      veetee-VERSION-x86_64-linux.tar.gz, a Debian package (needs cargo-deb)
      and SHA256SUMS.";

/// Pinned vttest release. Update both together.
const VTTEST_VERSION: &str = "20251205";
const VTTEST_SHA256: &str = "cd6886f9aefe6a3f6c566fa61271a55710901a71849c630bf5376aa984bf77cc";

/// Pinned esctest2 commit (GPL-2.0; fetched and run as an external tool only).
const ESCTEST_REPO: &str = "https://github.com/ThomasDickey/esctest2.git";
const ESCTEST_COMMIT: &str = "664be3cf2c1e3f06bc93a8bafb48a0db83c607db";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("vttest") => vttest(&args[1..]),
        Some("esctest") => esctest(&args[1..]),
        Some("dist") => dist(&args[1..]),
        Some("openvms") => openvms(&args[1..]),
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

fn esctest(args: &[String]) -> Result<()> {
    let update = args.iter().any(|a| a == "--update");
    let include = args.iter().find(|a| !a.starts_with("--"));

    let esctest = fetch_esctest()?;
    run(Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "vt-headless"])
        .current_dir(root()))?;
    let work = root().join("target/conformance");
    let log = work.join("esctest.log");
    let _ = fs::remove_file(&log);

    let mut command = format!(
        "spawn python3 {} --expected-terminal=xterm --max-vt-level=5 \
         --xterm-checksum=279 --options disableWideChars --timeout=0.5 --v=2 \
         --no-print-logs --logfile={}",
        esctest.join("esctest/esctest.py").display(),
        log.display()
    );
    if let Some(regex) = include {
        command.push_str(&format!(" --include={regex}"));
    }
    let script = work.join("esctest.vtscript");
    fs::write(
        &script,
        format!(
            "# Generated by cargo xtask esctest.\n\
             model vt525\nsize 24 80\nextensions xterm-compat\ntimeout 1800\n\
             {command}\nexpect-exit\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    run(Command::new(root().join("target/debug/vt-headless"))
        .arg("run")
        .arg(&script))?;

    let text = fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let summary = text
        .lines()
        .find(|l| l.contains(" passed, ") && l.contains("known bug"))
        .ok_or("esctest did not finish; see target/conformance/esctest.log")?;
    println!(
        "esctest: {}",
        summary.trim_matches(|c| c == '*' || c == ' ')
    );
    let failures: Vec<String> = text
        .lines()
        .filter_map(|l| l.split("*** TEST ").nth(1))
        .filter_map(|rest| rest.strip_suffix(" FAILED:"))
        .map(str::to_string)
        .collect();

    let list_path = root().join("tests/conformance/esctest/expected-failures.txt");
    let listed = fs::read_to_string(&list_path).unwrap_or_default();
    let mut expected: Vec<(String, String)> = Vec::new();
    for line in listed.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, reason) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        expected.push((name.to_string(), reason.trim().to_string()));
    }

    let unexpected: Vec<&String> = failures
        .iter()
        .filter(|f| !expected.iter().any(|(n, _)| n == *f))
        .collect();
    // With --include only a subset ran, so absent tests may simply not have run.
    let fixed: Vec<&String> = if include.is_some() {
        Vec::new()
    } else {
        expected
            .iter()
            .map(|(n, _)| n)
            .filter(|n| !failures.contains(n))
            .collect()
    };

    if update && include.is_none() {
        let header: String = listed
            .lines()
            .take_while(|l| l.starts_with('#') || l.trim().is_empty())
            .map(|l| format!("{l}\n"))
            .collect();
        let mut body = String::new();
        for name in &failures {
            let reason = expected
                .iter()
                .find(|(n, _)| n == name)
                .map_or("TODO: triage", |(_, r)| r.as_str());
            body.push_str(&format!("{name}  {reason}\n"));
        }
        fs::write(&list_path, header + &body).map_err(|e| e.to_string())?;
        println!("updated {}", list_path.display());
        return Ok(());
    }

    for name in &unexpected {
        println!("UNEXPECTED FAILURE {name}");
    }
    for name in &fixed {
        println!("NOW PASSING {name} (remove it from the expected failures)");
    }
    if let Some((name, _)) = expected
        .iter()
        .find(|(_, r)| r.is_empty() || r.starts_with("TODO"))
    {
        return Err(format!("expected failure {name} needs a reason"));
    }
    if unexpected.is_empty() && fixed.is_empty() {
        Ok(())
    } else {
        Err("esctest results differ from expected-failures.txt; details in target/conformance/esctest.log".into())
    }
}

/// Clones esctest2 at the pinned commit under target/conformance.
fn fetch_esctest() -> Result<PathBuf> {
    let dir = root().join(format!(
        "target/conformance/esctest2-{}",
        &ESCTEST_COMMIT[..12]
    ));
    if dir.join("esctest/esctest.py").exists() {
        return Ok(dir);
    }
    let _ = fs::remove_dir_all(&dir);
    eprintln!("fetching {ESCTEST_REPO} at {ESCTEST_COMMIT}");
    run(Command::new("git").args(["init", "--quiet"]).arg(&dir))?;
    run(Command::new("git")
        .args([
            "fetch",
            "--quiet",
            "--depth",
            "1",
            ESCTEST_REPO,
            ESCTEST_COMMIT,
        ])
        .current_dir(&dir))?;
    run(Command::new("git")
        .args(["checkout", "--quiet", "FETCH_HEAD"])
        .current_dir(&dir))?;
    Ok(dir)
}

fn dist(args: &[String]) -> Result<()> {
    let deb = !args.iter().any(|a| a == "--no-deb");
    let version = env!("CARGO_PKG_VERSION");
    let arch = env::consts::ARCH;
    // Release artifacts carry no debug information.
    let release_env = [
        ("CARGO_PROFILE_RELEASE_DEBUG", "0"),
        ("CARGO_PROFILE_RELEASE_STRIP", "symbols"),
    ];
    run(Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "veetee",
            "-p",
            "vt-headless",
        ])
        .envs(release_env)
        .current_dir(root()))?;

    let dist = root().join("target/dist");
    let name = format!("veetee-{version}-{arch}-linux");
    let stage = dist.join(&name);
    let _ = fs::remove_dir_all(&dist);
    let copy = |from: &str, to: &str| -> Result<()> {
        let dest = stage.join(to);
        fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::copy(root().join(from), &dest).map_err(|e| format!("{from}: {e}"))?;
        Ok(())
    };
    copy("target/release/veetee", "bin/veetee")?;
    copy("target/release/vt-headless", "bin/vt-headless")?;
    copy(
        "data/com.issinoho.Veetee.desktop",
        "share/applications/com.issinoho.Veetee.desktop",
    )?;
    for doc in [
        "README.md",
        "CHANGELOG.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "THIRD-PARTY.md",
    ] {
        copy(doc, doc)?;
    }
    copy("crates/vt-fonts/fonts/OFL.txt", "fonts-OFL.txt")?;

    let tarball = format!("{name}.tar.gz");
    run(Command::new("tar")
        .args(["--owner=0", "--group=0", "-czf", &tarball, &name])
        .current_dir(&dist))?;
    fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    let mut files = vec![tarball];

    if deb {
        run(Command::new(env!("CARGO"))
            .args(["deb", "--no-build", "-p", "veetee", "--output"])
            .arg(&dist)
            .current_dir(root()))
        .map_err(|e| {
            format!("{e}\n(install cargo-deb with `cargo install cargo-deb`, or pass --no-deb)")
        })?;
        for entry in read_dir_sorted(&dist)? {
            if entry.extension().is_some_and(|e| e == "deb") {
                files.push(entry.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }

    let sums = Command::new("sha256sum")
        .args(&files)
        .current_dir(&dist)
        .output()
        .map_err(|e| e.to_string())?;
    if !sums.status.success() {
        return Err("sha256sum failed".into());
    }
    fs::write(dist.join("SHA256SUMS"), &sums.stdout).map_err(|e| e.to_string())?;
    for file in files
        .iter()
        .chain(std::iter::once(&"SHA256SUMS".to_string()))
    {
        println!("{}", dist.join(file).display());
    }
    Ok(())
}

fn openvms(args: &[String]) -> Result<()> {
    let bless = args.iter().any(|a| a == "--bless");
    let filters: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    run(Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "vt-headless"])
        .current_dir(root()))?;
    let headless = root().join("target/debug/vt-headless");
    let suite = root().join("tests/conformance/openvms");
    let mut failed = Vec::new();
    let mut count = 0;
    for recording in read_dir_sorted(&suite)? {
        if !recording.extension().is_some_and(|e| e == "vtrec") {
            continue;
        }
        let name = recording
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f)) {
            continue;
        }
        count += 1;
        let mut cmd = Command::new(&headless);
        cmd.arg("replay").arg(&recording);
        if bless {
            cmd.arg("--bless");
        }
        if !cmd.status().map_err(|e| e.to_string())?.success() {
            failed.push(name);
        }
    }
    println!("openvms: {count} recordings, {} failed", failed.len());
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("recording replay failures: {}", failed.join(", ")))
    }
}
