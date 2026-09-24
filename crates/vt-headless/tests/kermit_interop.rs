//! veetee's Kermit against real ones: C-Kermit and G-Kermit, each run on a
//! pty by `vt-headless kermit --command`, sending to veetee and receiving
//! from it, in text and binary, with the one-character check and the CRC.
//!
//! Both are GPL and are only ever run, never read or linked: they are the far
//! end of the line and nothing else. Where one is not installed its tests
//! pass with a note, unless `VEETEE_REQUIRE_KERMITS` is set, as CI sets it,
//! in which case its absence fails them.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug)]
enum Peer {
    CKermit,
    GKermit,
}

impl Peer {
    fn program(self) -> &'static str {
        match self {
            Peer::CKermit => "kermit",
            Peer::GKermit => "gkermit",
        }
    }

    /// The command line for this peer. `-Y` keeps C-Kermit from reading an
    /// init file; each is told the mode, since without an attribute packet
    /// from veetee a receiving Kermit cannot know it and assumes binary.
    fn command(self, mode: &str, action: &str) -> String {
        match self {
            Peer::CKermit => format!("kermit -Y {mode} {action}"),
            Peer::GKermit => format!("gkermit {mode} {action}"),
        }
    }

    fn available(self) -> bool {
        let found = Command::new("sh")
            .args(["-c", &format!("command -v {}", self.program())])
            .output()
            .is_ok_and(|o| o.status.success());
        if !found {
            assert!(
                std::env::var_os("VEETEE_REQUIRE_KERMITS").is_none(),
                "{} is not installed, and VEETEE_REQUIRE_KERMITS says it must be",
                self.program()
            );
            eprintln!("{} is not installed; skipping", self.program());
        }
        found
    }
}

/// A scratch directory holding `src`, `in` and `out`, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("veetee-kermit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["src", "in", "out"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        Scratch(dir)
    }
    fn src(&self) -> PathBuf {
        self.0.join("src")
    }
    fn incoming(&self) -> PathBuf {
        self.0.join("in")
    }
    fn out(&self) -> PathBuf {
        self.0.join("out")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Files of every awkward shape for binary, and text with both endings.
fn write_files(dir: &Path) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut state = 0x5eed_u64;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
    };
    let files: [(&str, Vec<u8>); 7] = [
        ("every.dat", (0..=255u8).cycle().take(1024).collect()),
        ("noise.dat", (0..30_000).map(|_| noise()).collect()),
        ("empty.dat", Vec::new()),
        ("one.dat", vec![b'x']),
        ("runs.dat", {
            let mut v = vec![0u8; 500];
            v.extend([b'~'; 200]);
            v.extend([b'#'; 3]);
            v.extend([0xff; 97]);
            v.extend([b'&'; 50]);
            v
        }),
        ("lf.txt", b"one\ntwo\n\nthree\n".repeat(300)),
        ("crlf.txt", b"one\r\ntwo\r\n".repeat(300)),
    ];
    for (name, data) in &files {
        std::fs::write(dir.join(name), data).unwrap();
    }
    (
        vec!["every.dat", "noise.dat", "empty.dat", "one.dat", "runs.dat"],
        vec!["lf.txt", "crlf.txt"],
    )
}

fn vt_headless(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_vt-headless"))
        .arg("kermit")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("vt-headless runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// What a text file is once one of these Kermits has received it in text
/// mode: they drop every carriage return, which veetee's CR LF lines lose
/// nothing to.
fn as_unix_text(data: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(data)
        .replace("\r\n", "\n")
        .into_bytes()
}

fn exchange(peer: Peer, check: &str) {
    if !peer.available() {
        return;
    }
    let scratch = Scratch::new(&format!("{peer:?}-{check}"));
    let (binary, text) = write_files(&scratch.src());
    let src = scratch.src();
    let quote = |p: &Path| p.display().to_string();

    // The peer sends, veetee receives: binary, then text.
    for (mode, flag, files) in [("-i", "--binary", &binary), ("-T", "", &text)] {
        let into = scratch.incoming();
        let cmd = format!(
            "cd '{}' && {}",
            quote(&src),
            peer.command(mode, &format!("-s {}", files.join(" ")))
        );
        let mut args = vec!["receive", "--check", check, "--into"];
        let into_s = quote(&into);
        args.push(&into_s);
        args.extend(["--command", &cmd]);
        if !flag.is_empty() {
            args.push(flag);
        }
        let (ok, log) = vt_headless(&args, &src);
        assert!(ok, "{peer:?} {mode} check {check} to veetee:\n{log}");
        for name in files.iter() {
            let sent = std::fs::read(src.join(name)).unwrap();
            let got = std::fs::read(into.join(name))
                .unwrap_or_else(|e| panic!("{peer:?} {name}: {e}\n{log}"));
            // Text as well as binary arrives exactly as it was. A Unix Kermit
            // sending a file that already has CR LF sends the CR as part of
            // the line, so CR CR LF; veetee turns the line ending back into a
            // line feed and keeps the other CR, which is the file's own.
            assert!(got == sent, "{peer:?} {mode} check {check}: {name} differs");
        }
    }

    // veetee sends, the peer receives.
    for (mode, flag, files) in [("-i", "--binary", &binary), ("-T", "", &text)] {
        let out = scratch.out();
        let cmd = format!("cd '{}' && {}", quote(&out), peer.command(mode, "-r"));
        let mut args = vec!["send"];
        args.extend(files.iter().copied());
        args.extend(["--check", check, "--command", &cmd]);
        if !flag.is_empty() {
            args.push(flag);
        }
        let (ok, log) = vt_headless(&args, &src);
        assert!(ok, "veetee to {peer:?} {mode} check {check}:\n{log}");
        for name in files.iter() {
            let sent = std::fs::read(src.join(name)).unwrap();
            let got = std::fs::read(out.join(name))
                .unwrap_or_else(|e| panic!("{peer:?} {name}: {e}\n{log}"));
            let expected = if mode == "-T" {
                as_unix_text(&sent)
            } else {
                sent
            };
            assert!(
                got == expected,
                "veetee to {peer:?} {mode} check {check}: {name} differs"
            );
        }
    }
}

#[test]
fn with_c_kermit_and_the_one_character_check() {
    exchange(Peer::CKermit, "1");
}

#[test]
fn with_c_kermit_and_the_crc() {
    exchange(Peer::CKermit, "3");
}

#[test]
fn with_g_kermit_and_the_one_character_check() {
    exchange(Peer::GKermit, "1");
}

#[test]
fn with_g_kermit_and_the_crc() {
    exchange(Peer::GKermit, "3");
}

/// With attribute packets, the host's Kermit needs telling nothing: veetee
/// says whether each file is text, and the receiving Kermit, left at its own
/// default of binary, follows. On OpenVMS that is the difference between a
/// user setting the mode at both ends and at one.
fn told_only_by_veetee(peer: Peer) {
    if !peer.available() {
        return;
    }
    let scratch = Scratch::new(&format!("{peer:?}-attributes"));
    let (binary, text) = write_files(&scratch.src());
    let src = scratch.src();
    let out = scratch.out();
    for (flag, files) in [("--binary", &binary), ("", &text)] {
        let cmd = format!("cd '{}' && {}", out.display(), peer.command("", "-r"));
        let mut args = vec!["send"];
        args.extend(files.iter().copied());
        args.extend(["--command", &cmd]);
        if !flag.is_empty() {
            args.push(flag);
        }
        let (ok, log) = vt_headless(&args, &src);
        assert!(ok, "veetee to {peer:?} {flag}:\n{log}");
        for name in files.iter() {
            let sent = std::fs::read(src.join(name)).unwrap();
            let got = std::fs::read(out.join(name)).unwrap();
            let expected = if flag.is_empty() {
                as_unix_text(&sent)
            } else {
                sent
            };
            assert!(got == expected, "veetee to {peer:?} {flag}: {name} differs");
        }
    }
}

/// And the other way: C-Kermit, left to itself, looks at each file and says
/// which it is, so veetee — set to binary — still receives text as text.
#[test]
fn c_kermit_left_to_itself_says_which_files_are_text() {
    let peer = Peer::CKermit;
    if !peer.available() {
        return;
    }
    let scratch = Scratch::new("ckermit-auto");
    let (binary, text) = write_files(&scratch.src());
    let src = scratch.src();
    let into = scratch.incoming();
    let all: Vec<&str> = binary.iter().chain(&text).copied().collect();
    let cmd = format!(
        "cd '{}' && {}",
        src.display(),
        peer.command("", &format!("-s {}", all.join(" ")))
    );
    let into_s = into.display().to_string();
    let (ok, log) = vt_headless(
        &["receive", "--binary", "--into", &into_s, "--command", &cmd],
        &src,
    );
    assert!(ok, "{log}");
    for name in &all {
        let sent = std::fs::read(src.join(name)).unwrap();
        let got = std::fs::read(into.join(name)).unwrap();
        assert!(got == sent, "{name} differs");
    }
}

#[test]
fn c_kermit_is_told_the_mode_by_veetee_alone() {
    told_only_by_veetee(Peer::CKermit);
}

#[test]
fn g_kermit_is_told_the_mode_by_veetee_alone() {
    told_only_by_veetee(Peer::GKermit);
}
