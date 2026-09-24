//! The name a received file is given here.
//!
//! The far end chooses the name, so it is untrusted input: a host that sends
//! `../../.profile` or `/etc/passwd` must not get to write there. Only the
//! last part of the name is kept, whatever system it came from — a Unix path,
//! a Windows one, or an OpenVMS `DKA0:[SYSMGR]LOGIN.COM;3` — and a name with
//! nothing usable left is refused rather than invented. Where the file then
//! goes, and what happens if the name is taken, is the caller's to decide.

/// How a received name is written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Names {
    /// What Kermit calls converted: an OpenVMS version number is dropped and
    /// the name is lower-cased, so `LOGIN.COM;3` arrives as `login.com`.
    #[default]
    Converted,
    /// The last part of the name exactly as it was sent, version and case
    /// included.
    AsSent,
}

/// A name for a received file, safe to create in whatever directory the user
/// chose, or `None` if nothing in it can be used.
#[must_use]
pub fn local_name(sent: &str, names: Names) -> Option<String> {
    // The directory and device, in any of the forms a far end might use, are
    // places on its own system and mean nothing here.
    let last = sent.rsplit(['/', '\\', ']', '>', ':']).next().unwrap_or("");
    let name = match names {
        Names::Converted => without_version(last).to_lowercase(),
        Names::AsSent => last.to_string(),
    };
    let name: String = name
        .chars()
        .map(|c| {
            // What Windows refuses in a name, and control characters
            // everywhere.
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // An OpenVMS file with no type is `NAME.`, and Windows will not keep a
    // trailing dot or space anyway. This also leaves `.` and `..` empty.
    let name = name.trim().trim_end_matches(['.', ' ']);
    if name.is_empty() {
        return None;
    }
    // A device name is a device on Windows whatever follows the dot.
    let stem = name.split('.').next().unwrap_or("");
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    Some(if reserved {
        format!("_{name}")
    } else {
        name.to_string()
    })
}

/// `LOGIN.COM;3` without its `;3`.
fn without_version(name: &str) -> &str {
    match name.rsplit_once(';') {
        Some((base, version)) if version.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converted(sent: &str) -> Option<String> {
        local_name(sent, Names::Converted)
    }

    #[test]
    fn an_openvms_name_arrives_without_its_version_or_its_capitals() {
        assert_eq!(converted("LOGIN.COM;3").as_deref(), Some("login.com"));
        assert_eq!(converted("LOGIN.COM;").as_deref(), Some("login.com"));
        assert_eq!(converted("LOGIN.COM").as_deref(), Some("login.com"));
        assert_eq!(
            converted("DKA0:[SYSMGR]LOGIN.COM;12").as_deref(),
            Some("login.com")
        );
        assert_eq!(
            converted("SYS$LOGIN:NOTES.TXT;1").as_deref(),
            Some("notes.txt")
        );
        assert_eq!(converted("<DIR>FILE.TXT").as_deref(), Some("file.txt"));
        assert_eq!(converted("README.;1").as_deref(), Some("readme"), "no type");
    }

    #[test]
    fn as_sent_keeps_the_name_but_not_the_path() {
        let as_sent = |s| local_name(s, Names::AsSent);
        assert_eq!(as_sent("LOGIN.COM;3").as_deref(), Some("LOGIN.COM;3"));
        assert_eq!(
            as_sent("[SYSMGR]LOGIN.COM;3").as_deref(),
            Some("LOGIN.COM;3")
        );
        assert_eq!(as_sent("../../x").as_deref(), Some("x"));
    }

    #[test]
    fn a_far_end_does_not_choose_where_a_file_lands() {
        for sent in [
            "../../.profile",
            "/etc/passwd",
            "..\\..\\autoexec.bat",
            "C:\\Windows\\win.ini",
            "~/.ssh/authorized_keys",
        ] {
            let name = converted(sent).unwrap();
            assert!(
                !name.contains(['/', '\\', ':']) && name != ".." && name != ".",
                "{sent:?} became {name:?}"
            );
        }
        assert_eq!(converted("../../.profile").as_deref(), Some(".profile"));
    }

    #[test]
    fn a_name_with_nothing_left_is_refused() {
        for sent in ["", ".", "..", "/", "[DIR]", "DKA0:", "   ", "...", "a/.."] {
            assert_eq!(converted(sent), None, "{sent:?}");
        }
    }

    #[test]
    fn what_a_filesystem_would_refuse_is_made_harmless() {
        assert_eq!(converted("a*b?.txt").as_deref(), Some("a_b_.txt"));
        assert_eq!(converted("bell\x07.txt").as_deref(), Some("bell_.txt"));
        assert_eq!(converted("NUL.TXT").as_deref(), Some("_nul.txt"));
        assert_eq!(converted("con").as_deref(), Some("_con"));
        assert_eq!(converted("console.txt").as_deref(), Some("console.txt"));
    }
}
