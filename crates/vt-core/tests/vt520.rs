//! VT510/VT520 features (milestone M4). Reference: VT520/VT525 Video Terminal
//! Programmer Information (EK-VT520-RM), chapter 5.

use vt_core::dump::row_text;
use vt_core::{Config, CursorStyle, Event, Model, Terminal};

fn vt520() -> Terminal {
    Terminal::new(Config {
        model: Model::Vt520,
        ..Config::default()
    })
}

fn row(t: &Terminal, r: usize) -> String {
    row_text(t, r - 1)
}

fn cursor(t: &Terminal) -> (usize, usize) {
    (t.cursor().row + 1, t.cursor().col + 1)
}

fn reply(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

// ------------------------------------------------------- cursor motion

#[test]
fn cha_hpa_hpr_vpa_vpr() {
    let mut t = vt520();
    t.advance(b"\x1b[5;5H\x1b[10G");
    assert_eq!(cursor(&t), (5, 10));
    t.advance(b"\x1b[20`");
    assert_eq!(cursor(&t), (5, 20));
    t.advance(b"\x1b[3a");
    assert_eq!(cursor(&t), (5, 23));
    t.advance(b"\x1b[999a");
    assert_eq!(cursor(&t), (5, 80));
    t.advance(b"\x1b[12d");
    assert_eq!(cursor(&t), (12, 80));
    t.advance(b"\x1b[2e");
    assert_eq!(cursor(&t), (14, 80));
    t.advance(b"\x1b[99e");
    assert_eq!(cursor(&t), (24, 80));
}

#[test]
fn cha_hpa_vpa_follow_origin_mode() {
    let mut t = vt520();
    t.advance(b"\x1b[5;10r\x1b[?69h\x1b[10;20s\x1b[?6h\x1b[3G");
    assert_eq!(cursor(&t), (5, 12));
    t.advance(b"\x1b[4`");
    assert_eq!(cursor(&t), (5, 13));
    t.advance(b"\x1b[2d");
    assert_eq!(cursor(&t), (6, 13));
    t.advance(b"\x1b[99d\x1b[99`");
    assert_eq!(cursor(&t), (10, 20), "clamped to the margins");
    t.advance(b"\x1b[?6l\x1b[1;1H\x1b[30`\x1b[20d");
    assert_eq!(
        cursor(&t),
        (20, 30),
        "margins do not apply outside origin mode"
    );
}

#[test]
fn cbt_stops_at_the_left_margin_only_in_origin_mode() {
    let mut t = vt520();
    t.advance(b"\x1b[?69h\x1b[5;30s\x1b[7;9H\x1b[5Z");
    assert_eq!(cursor(&t), (7, 1));
    t.advance(b"\x1b[?6h\x1b[1;20H\x1b[5Z");
    assert_eq!(cursor(&t), (1, 5));
}

#[test]
fn cnl_cpl_stop_at_margins() {
    let mut t = vt520();
    t.advance(b"\x1b[5;10r\x1b[7;9H\x1b[9E");
    assert_eq!(cursor(&t), (10, 1));
    t.advance(b"\x1b[3;9H\x1b[9F");
    assert_eq!(cursor(&t), (1, 1));
}

#[test]
fn cht_cbt_and_decst8c() {
    let mut t = vt520();
    t.advance(b"\x1b[3g\x1b[1;5H\x1bH\x1b[1;30H\x1bH\x1b[1;1H\x1b[2I");
    assert_eq!(cursor(&t), (1, 30));
    t.advance(b"\x1b[Z");
    assert_eq!(cursor(&t), (1, 5));
    t.advance(b"\x1b[5Z");
    assert_eq!(cursor(&t), (1, 1));
    t.advance(b"\x1b[?5W\x1b[1;1H\x1b[3I");
    assert_eq!(cursor(&t), (1, 25));
}

#[test]
fn vt520_cursor_functions_need_level_5() {
    let mut t = Terminal::new(Config::default());
    t.advance(b"\x1b[5;5H\x1b[10G\x1b[3d");
    assert_eq!(cursor(&t), (5, 5));
}

// --------------------------------------------------------------- modes

#[test]
fn decscusr_selects_cursor_style() {
    let mut t = vt520();
    assert_eq!(t.cursor_style(), CursorStyle::BlinkingBlock);
    t.advance(b"\x1b[4 q");
    assert_eq!(t.cursor_style(), CursorStyle::SteadyUnderline);
    t.advance(b"\x1bP$q q\x1b\\");
    assert_eq!(reply(&mut t), "\x1bP1$r4 q\x1b\\");
    t.advance(b"\x1b[9 q");
    assert_eq!(
        t.cursor_style(),
        CursorStyle::SteadyUnderline,
        "invalid style ignored"
    );
    t.advance(b"\x1b[ q");
    assert_eq!(t.cursor_style(), CursorStyle::BlinkingBlock);
}

#[test]
fn decncsm_keeps_screen_on_column_change() {
    let mut t = vt520();
    t.advance(b"keep\x1b[?95h\x1b[?3h");
    assert_eq!((t.grid().cols(), row(&t, 1).as_str()), (132, "keep"));
    t.advance(b"\x1b[?95l\x1b[?3l");
    assert_eq!((t.grid().cols(), row(&t, 1).as_str()), (80, ""));
}

#[test]
fn vt500_modes_are_stored_and_reported() {
    let mut t = vt520();
    t.advance(b"\x1b[?102$p\x1b[?117$p\x1b[?117h\x1b[?117$p\x1b[?108$p\x1b[?999$p");
    assert_eq!(
        reply(&mut t),
        "\x1b[?102;1$y\x1b[?117;2$y\x1b[?117;1$y\x1b[?108;0$y\x1b[?999;0$y"
    );
}

#[test]
fn decstr_resets_key_position_and_direction() {
    let mut t = vt520();
    t.advance(b"\x1b[?81h\x1b[?34h\x1b[!p\x1b[?34$p");
    assert_eq!(reply(&mut t), "\x1b[?34;2$y");
}

// ----------------------------------------------------- Set-Up selections

#[test]
fn setup_selections_report_factory_values() {
    let mut t = vt520();
    for (request, expected) in [
        (" r", "5 r"),
        (" t", "5 t"),
        (" u", "1 u"),
        (" p", "1 p"),
        (" v", "1 v"),
        ("-p", "30-p"),
        ("-q", "15-q"),
        ("-r", "15-r"),
        (",{", "1,{"),
        ("$s", "1$s"),
        ("*p", "437*p"),
        ("(p", "1(p"),
        ("$q", "3$q"),
        ("p", "1p"),
        ("*u", "0;1*u"),
        ("*r", "1;6*r"),
        ("*s", "1;3;1;1*s"),
        ("+w", "1;1;1;1+w"),
        ("\"u", "1;1\"u"),
        (" ~", "1 ~"),
    ] {
        t.advance(format!("\x1bP$q{request}\x1b\\").as_bytes());
        assert_eq!(
            reply(&mut t),
            format!("\x1bP1$r{expected}\x1b\\"),
            "{request}"
        );
    }
}

#[test]
fn setup_selections_change_and_reject_bad_values() {
    let mut t = vt520();
    t.advance(b"\x1b[2 r\x1b[60-q\x1b[45-q\x1b[850*p\x1b[1;7*r\x1b[2;2;3;1*s\x1b[3;2\"u");
    for (request, expected) in [
        (" r", "2 r"),
        ("-q", "60-q"),
        ("*p", "850*p"),
        ("*r", "1;7*r"),
        ("\"u", "1;1\"u"),
    ] {
        t.advance(format!("\x1bP$q{request}\x1b\\").as_bytes());
        assert_eq!(
            reply(&mut t),
            format!("\x1bP1$r{expected}\x1b\\"),
            "{request}"
        );
    }
}

#[test]
fn setup_selections_are_vt500_only() {
    let mut t = Terminal::new(Config::default());
    t.advance(b"\x1bP$q q\x1b\\");
    assert_eq!(reply(&mut t), "\x1bP0$r\x1b\\");
}

// ------------------------------------------------ identity and resets

#[test]
fn dectid_selects_the_da1_identity() {
    let mut t = vt520();
    t.advance(b"\x1b[c");
    assert!(reply(&mut t).starts_with("\x1b[?65;"));
    t.advance(b"\x1b[9,q\x1b[c");
    assert_eq!(reply(&mut t), "\x1b[?64;1;2;7;8;9;15;18;21c");
    t.advance(b"\x1b[2,q\x1b[c");
    assert_eq!(reply(&mut t), "\x1b[?6c");
}

#[test]
fn dectme_switches_between_vt500_vt100_and_vt52() {
    let mut t = vt520();
    t.advance(b"\x1b[2 ~\x1bP$q\"p\x1b\\");
    assert_eq!(reply(&mut t), "", "DECRQSS is not available in VT100 mode");
    assert_eq!(t.level(), 1);
    t.advance(b"\x1b[1 ~");
    assert_eq!(t.level(), 5);
    t.advance(b"\x1b[3 ~");
    assert!(!t.modes().ansi);
}

#[test]
fn decsr_resets_and_confirms() {
    let mut t = vt520();
    t.advance(b"text\x1b[?7h\x1b[1234+p");
    assert_eq!(row(&t, 1), "");
    assert!(!t.modes().autowrap);
    assert_eq!(reply(&mut t), "\x1b[1234*q");
    t.advance(b"\x1b[+p");
    assert_eq!(reply(&mut t), "");
}

#[test]
fn replies_before_a_reset_are_kept() {
    let mut t = vt520();
    t.advance(b"\x1b[5n\x1bc");
    assert_eq!(reply(&mut t), "\x1b[0n");
}

#[test]
fn declans_loads_the_answerback() {
    let mut t = vt520();
    t.advance(b"\x1bP1v56415820\x1b\\\x05");
    assert_eq!(reply(&mut t), "VAX ");
    t.advance(b"\x1bP2v41\x1b\\\x05");
    assert_eq!(reply(&mut t), "VAX ", "only hex encoding is accepted");
}

#[test]
fn decswt_names_the_session() {
    let mut t = vt520();
    t.advance(b"\x1b]21;VMS1 System Manager\x1b\\\x1b]2L;VMS1\x1b\\\x1b]2;xterm\x07");
    assert_eq!(
        t.take_events(),
        [
            Event::TitleChanged("VMS1 System Manager".into()),
            Event::IconNameChanged("VMS1".into())
        ]
    );
}

// ------------------------------------------------------------ VT525 colour

mod color {
    use super::*;
    use vt_core::cell::{Attrs, Color, Flags};
    use vt_core::color::{ColorMode, DEFAULT_MAP};

    fn vt525() -> Terminal {
        Terminal::new(Config {
            model: Model::Vt525,
            ..Config::default()
        })
    }

    #[test]
    fn sgr_colours_are_vt525_only() {
        let mut t = vt525();
        t.advance(b"\x1b[31;44mA");
        let a = t.grid().line(0).cells()[0].attrs;
        assert_eq!((a.fg, a.bg), (Color::Indexed(1), Color::Indexed(4)));
        assert!(vt520().colors().is_none());
        let mut mono = vt520();
        mono.advance(b"\x1b[31mA");
        assert_eq!(mono.grid().line(0).cells()[0].attrs.fg, Color::Default);
    }

    #[test]
    fn decac_decatc_decstglt_set_and_report() {
        let mut t = vt525();
        t.advance(b"\x1b[1;14;4,|\x1b[2;9;1,|\x1b[5;12;3,}\x1b[1){");
        let (table, _) = t.colors().unwrap();
        assert_eq!((table.normal, table.frame), ((14, 4), (9, 1)));
        assert_eq!(table.alternate[5], (12, 3));
        assert_eq!(table.mode, ColorMode::Alternate);
        for (request, expected) in [
            ("1,|", "1;14;4,|"),
            ("2,|", "2;9;1,|"),
            (",|", "1;14;4,|"),
            ("5,}", "5;12;3,}"),
            ("0,}", "0;7;0,}"),
            ("){", "1){"),
        ] {
            t.advance(format!("\x1bP$q{request}\x1b\\").as_bytes());
            assert_eq!(
                reply(&mut t),
                format!("\x1bP1$r{expected}\x1b\\"),
                "{request}"
            );
        }
        t.advance(b"\x1b[3;16;0,|\x1b[16;1;1,}\x1b[7){");
        let (table, _) = t.colors().unwrap();
        assert_eq!((table.normal, table.mode), ((14, 4), ColorMode::Alternate));
    }

    #[test]
    fn color_table_report_and_restore() {
        let mut t = vt525();
        t.advance(b"\x1b[2;2$u");
        let report = reply(&mut t);
        assert!(
            report.starts_with("\x1bP2$s0;2;0;0;0/1;2;67;0;0/"),
            "{report:?}"
        );
        assert!(report.ends_with("15;2;100;100;100\x1b\\"));
        t.advance(b"\x1bP2$p1;2;10;20;30/4;1;120;50;100\x1b\\");
        let (table, _) = t.colors().unwrap();
        assert_eq!(table.map[1], [10, 20, 30]);
        assert_eq!(table.map[4], [100, 0, 0], "HLS hue 120 is red");
        assert_eq!(
            table.map[2], DEFAULT_MAP[2],
            "unlisted entries are unchanged"
        );
        t.advance(b"\x1b[2;1$u");
        assert!(reply(&mut t).contains("/4;1;120;50;100/"));
    }

    #[test]
    fn erase_colour_mode() {
        let mut t = vt525();
        t.advance(b"\x1b[44m\x1b[2J");
        assert_eq!(t.grid().line(0).cells()[0].attrs.bg, Color::Indexed(4));
        t.advance(b"\x1b[?117h\x1b[2J");
        assert_eq!(t.grid().line(0).cells()[0].attrs.bg, Color::Default);
    }

    #[test]
    fn alternate_colours_are_fixed_when_written() {
        let mut t = vt525();
        t.advance(b"\x1b[6;10;2,}\x1b[1){\x1b[?114h\x1b[31;1;4mA\x1b[3){B");
        let (table, options) = t.colors().unwrap();
        let cells = t.grid().line(0).cells();
        let a = table.resolve(cells[0].attrs, options, true);
        assert_eq!(
            (a.fg, a.bg, a.underline),
            (DEFAULT_MAP[10], DEFAULT_MAP[2], true)
        );
        let b = table.resolve(cells[1].attrs, options, true);
        assert_eq!(
            (b.fg, b.bg),
            (DEFAULT_MAP[9], DEFAULT_MAP[0]),
            "SGR colours again"
        );
        let _ = Attrs::default();
        let _ = Flags::NONE;
    }
}

// ------------------------------------------------------ character sets

mod charsets {
    use super::*;

    #[test]
    fn vt500_supplemental_sets() {
        let mut t = vt520();
        // DEC Greek into G2 via GR, ISO Cyrillic into G3 via SS3.
        t.advance(b"\x1b*\"?\x1b}\xc1\xe1\x1b/L\x1bO\x40\x1bO\x6f");
        assert_eq!(row(&t, 1), "ΑαРя");
        t.advance(b"\x1b[2;1H\x1b*%0\xae\xbe\x1b*&4\xc1\xe1");
        assert_eq!(row(&t, 2), "İıаА");
    }

    #[test]
    fn vt500_national_sets_need_nrc_mode() {
        let mut t = vt520();
        t.advance(b"\x1b(&5ab");
        assert_eq!(row(&t, 1), "ab", "not designated outside NRC mode");
        t.advance(b"\x1b[?42h\x1b[2;1H\x1b(&5ab\x1b(%3@[\x1b(%2&@");
        assert_eq!(row(&t, 2), "АБŽŠğİ");
    }

    #[test]
    fn vt500_sets_are_level_5_only() {
        let mut t = Terminal::new(Config::default());
        t.advance(b"\x1b*\"?\x1b}\xc1");
        assert_ne!(row(&t, 1), "Α");
    }

    #[test]
    fn decaupss_with_vt500_sets() {
        let mut t = vt520();
        t.advance(b"\x1bP0!u\"?\x1b\\\x1b[&u");
        assert_eq!(reply(&mut t), "\x1bP0!u\"?\x1b\\");
        t.advance(b"\x1bP1!uL\x1b\\\x1b[&u\x1b-<\x1b~\xe1");
        assert_eq!(reply(&mut t), "\x1bP1!uL\x1b\\");
        assert_eq!(row(&t, 1), "\u{0441}");
        t.type_text("д");
        assert_eq!(t.take_output(), [0xD4]);
    }
}

// ------------------------------------------------- keyboard and sessions

mod keyboard {
    use super::*;
    use vt_core::{Key, LocalKeyAction};

    #[test]
    fn deckbd_selects_the_reported_keyboard() {
        let mut t = vt520();
        t.advance(b"\x1b[?26n");
        assert_eq!(reply(&mut t), "\x1b[?27;1;0;4n");
        t.advance(b"\x1b[2;39 }\x1b[?26n");
        assert_eq!(
            reply(&mut t),
            "\x1b[?27;39;0;5n",
            "Russian, enhanced PC keyboard"
        );
        t.advance(b"\x1b[1;17 }\x1b[?26n");
        assert_eq!(reply(&mut t), "\x1b[?27;39;0;5n", "17 is not a language");
    }

    #[test]
    fn declfkc_reassigns_local_function_keys() {
        let mut t = vt520();
        assert_eq!(t.local_function_key(1), LocalKeyAction::Local);
        t.advance(b"\x1b[1;2;3;3*}");
        assert_eq!(t.local_function_key(1), LocalKeyAction::SendToHost);
        assert_eq!(t.local_function_key(3), LocalKeyAction::Disabled);
        t.key(Key::Function(1));
        assert_eq!(reply(&mut t), "\x1b[11~");
        t.advance(b"\x1b[0;0*}");
        assert_eq!(t.local_function_key(3), LocalKeyAction::Local);
    }

    #[test]
    fn decelf_disables_copy_and_paste_keys() {
        let mut t = vt520();
        t.advance(b"\x1b[1;2+q");
        assert!(!t.copy_paste_keys_enabled());
        t.advance(b"\x1b[0;1+q");
        assert!(t.copy_paste_keys_enabled());
    }

    #[test]
    fn deces_decus_and_decps() {
        let mut t = vt520();
        t.advance(b"\x1b[&x\x1b[4;16;13,~\x1b[9;1;1,~");
        assert_eq!(
            t.take_events(),
            [
                Event::SessionActivated,
                Event::PlaySound {
                    volume: 4,
                    duration_ms: 500,
                    note: 13
                }
            ]
        );
        t.advance(b"\x1bP$q,y\x1b\\\x1b[3,y\x1bP$q,y\x1b\\\x1bP$q,x\x1b\\");
        assert_eq!(
            reply(&mut t),
            "\x1bP1$r2,y\x1b\\\x1bP1$r3,y\x1b\\\x1bP1$r3;0;0;0,x\x1b\\"
        );
    }
}

#[test]
fn vt500_page_and_screen_sizes() {
    let mut t = vt520();
    assert_eq!(t.page(), (0, 3));
    t.advance(b"\x1b[30t");
    assert_eq!((t.grid().rows(), t.page().1), (36, 2), "next higher length");
    t.advance(b"\x1b[99t");
    assert_eq!((t.grid().rows(), t.page().1), (72, 1));
    t.advance(b"\x1b[30*|");
    assert_eq!(
        t.window().1,
        41,
        "42 data lines less the indicator status line"
    );
    t.advance(b"\x1b[0$~\x1b[53*|");
    assert_eq!(t.window().1, 53);
}

#[test]
fn multiple_session_status_report() {
    let mut t = vt520();
    t.advance(b"\x1b[?85n");
    assert_eq!(reply(&mut t), "\x1b[?83n");
    t.set_sessions(2);
    t.advance(b"\x1b[?85n");
    assert_eq!(reply(&mut t), "\x1b[?87n", "sessions on separate lines");
}
