//! VT220/VT320/VT420 features (milestone M2). References: VT510 Video
//! Terminal Programmer Information (EK-VT510-RM) chapter 5, VT220 Programmer
//! Reference (EK-VT220-RM) chapter 2.

use vt_core::cell::Flags;
use vt_core::charset::{ERROR_CHARACTER, Nrc, SOFT_BASE};
use vt_core::dump::row_text;
use vt_core::{Config, Key, Model, StatusDisplay, Terminal};

fn term(model: Model) -> Terminal {
    Terminal::new(Config {
        model,
        ..Config::default()
    })
}

fn row(t: &Terminal, r: usize) -> String {
    row_text(t, r - 1)
}

fn cursor(t: &Terminal) -> (usize, usize) {
    (t.cursor().row + 1, t.cursor().col + 1)
}

// ------------------------------------------------- selective erase

#[test]
fn decsca_protects_from_decsed_and_decsel_only() {
    let mut t = term(Model::Vt220);
    t.advance(b"ab\x1b[1\"qPROT\x1b[0\"qcd\r\n\x1b[1\"qLINE2\x1b[2\"qxy");
    t.advance(b"\x1b[1;1H\x1b[?2K");
    assert_eq!(row(&t, 1), "  PROT");
    t.advance(b"\x1b[?J");
    assert_eq!(row(&t, 1), "  PROT");
    assert_eq!(row(&t, 2), "LINE2");
    t.advance(b"\x1b[2K");
    assert_eq!(row(&t, 1), "", "EL erases protected characters too");
}

#[test]
fn sgr_does_not_change_protection() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b[1\"q\x1b[0mX\x1b[mY");
    let cells = t.grid().line(0).cells();
    assert!(cells[0].attrs.flags.contains(Flags::PROTECTED));
    assert!(cells[1].attrs.flags.contains(Flags::PROTECTED));
}

#[test]
fn vt102_has_no_selective_erase() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[1\"qAB\x1b[1;1H\x1b[?2K");
    assert_eq!(row(&t, 1), "AB");
    assert!(
        !t.grid().line(0).cells()[0]
            .attrs
            .flags
            .contains(Flags::PROTECTED)
    );
}

// ------------------------------------------------ save and reset

#[test]
fn decsc_saves_autowrap_mode_and_protection() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[?7h\x1b[1\"q\x1b7\x1b[?7l\x1b[0\"q\x1b8X");
    assert!(t.modes().autowrap);
    assert!(
        t.grid().line(0).cells()[0]
            .attrs
            .flags
            .contains(Flags::PROTECTED)
    );
}

#[test]
fn decstr_resets_per_vt510_table() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[?42h\x1b[1\"q\x1b[5;10r\x1b[?5h\x1b[3;3H\x1b7\x1b[!p\x1b8");
    assert!(!t.modes().national);
    assert!(t.modes().reverse_screen, "DECSCNM is not reset by DECSTR");
    assert_eq!(cursor(&t), (1, 1), "saved cursor state is reset to home");
    t.advance(b"X");
    assert!(
        !t.grid().line(0).cells()[0]
            .attrs
            .flags
            .contains(Flags::PROTECTED)
    );
}

#[test]
fn decscl_selects_level_and_hard_resets() {
    let mut t = term(Model::Vt420);
    t.advance(b"text\x1b[62\"p");
    assert_eq!(t.level(), 2);
    assert!(t.eight_bit_replies(), "8-bit controls by default");
    assert_eq!(row(&t, 1), "", "hard reset");
    t.advance(b"\x1b[63;1\"p");
    assert_eq!((t.level(), t.eight_bit_replies()), (3, false));
    t.advance(b"\x1b[61\"p");
    assert_eq!(t.level(), 1);
    t.advance(b"\x1b[c");
    assert_eq!(
        t.take_output(),
        b"\x1b[?64;1;2;6;7;8;9;15;18;21c",
        "VT100 mode keeps the Terminal ID (EK-VT510-RM 2.6.2)"
    );
    t.advance(b"\x1b[65;1\"p");
    assert_eq!(t.level(), 4, "clamped to the model's highest level");
    let mut vt102 = term(Model::Vt102);
    vt102.advance(b"x\x1b[62\"p");
    assert_eq!((vt102.level(), row(&vt102, 1)), (1, "x".into()));
}

// ------------------------------------------------------ reports

fn reply(t: &mut Terminal, seq: &[u8]) -> String {
    t.advance(seq);
    String::from_utf8_lossy(&t.take_output()).replace('\x1b', "ESC ")
}

#[test]
fn dsr_variants() {
    let mut t = term(Model::Vt420);
    assert_eq!(reply(&mut t, b"\x1b[?15n"), "ESC [?13n");
    assert_eq!(reply(&mut t, b"\x1b[?25n"), "ESC [?20n");
    assert_eq!(reply(&mut t, b"\x1b[?26n"), "ESC [?27;1;0;1n");
    assert_eq!(reply(&mut t, b"\x1b[5;7H\x1b[?6n"), "ESC [?5;7;1R");
    assert_eq!(reply(&mut t, b"\x1b[?75n"), "ESC [?70n");
    let mut vt220 = term(Model::Vt220);
    assert_eq!(reply(&mut vt220, b"\x1b[?26n"), "ESC [?27;1n");
    assert_eq!(
        reply(&mut vt220, b"\x1b[?6n"),
        "",
        "DECXCPR is a VT420 report"
    );
}

#[test]
fn decrqm_reports_modes() {
    let mut t = term(Model::Vt420);
    assert_eq!(
        reply(&mut t, b"\x1b[?7$p"),
        "ESC [?7;1$y",
        "auto wrap is on at power-up"
    );
    assert_eq!(reply(&mut t, b"\x1b[?7l\x1b[?7$p"), "ESC [?7;2$y");
    assert_eq!(reply(&mut t, b"\x1b[20$p"), "ESC [20;2$y");
    assert_eq!(
        reply(&mut t, b"\x1b[13$p"),
        "ESC [13;4$y",
        "permanently reset ISO mode"
    );
    assert_eq!(reply(&mut t, b"\x1b[?9999$p"), "ESC [?9999;0$y");
    assert_eq!(
        reply(&mut term(Model::Vt220), b"\x1b[?7$p"),
        "",
        "DECRQM is VT320+"
    );
}

#[test]
fn decrqss_reports_settings() {
    let mut t = term(Model::Vt420);
    assert_eq!(
        reply(&mut t, b"\x1b[1;4;7m\x1bP$qm\x1b\\"),
        "ESC P1$r0;1;4;7mESC \\"
    );
    assert_eq!(
        reply(&mut t, b"\x1b[3;20r\x1bP$qr\x1b\\"),
        "ESC P1$r3;20rESC \\"
    );
    assert_eq!(reply(&mut t, b"\x1bP$q\"p\x1b\\"), "ESC P1$r64;1\"pESC \\");
    assert_eq!(
        reply(&mut t, b"\x1b[1\"q\x1bP$q\"q\x1b\\"),
        "ESC P1$r1\"qESC \\"
    );
    assert_eq!(reply(&mut t, b"\x1bP$q$~\x1b\\"), "ESC P1$r1$~ESC \\");
    assert_eq!(reply(&mut t, b"\x1bP$qt\x1b\\"), "ESC P1$r24tESC \\");
    assert_eq!(reply(&mut t, b"\x1bP$qz\x1b\\"), "ESC P0$rESC \\");
}

#[test]
fn deccir_and_dectabsr_round_trip() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[3g\x1b[1;9H\x1bH\x1b[1;17H\x1bH");
    t.advance(b"\x1b[5;12H\x1b[1;4m\x1b[1\"q\x1b*0\x1bn\x1b[?6h\x1b[2;23r");
    t.advance(b"\x1b[5;12H");
    let cir = reply(&mut t, b"\x1b[1$w");
    assert_eq!(cir, "ESC P1$u6;12;1;C;A;A;2;2;@;BB0%5ESC \\");
    let tabs = reply(&mut t, b"\x1b[2$w");
    assert_eq!(tabs, "ESC P2$u9/17ESC \\");

    // Restore both into a fresh terminal.
    let mut u = term(Model::Vt420);
    u.advance(b"\x1bP1$t6;12;1;C;A;A;2;2;@;BB0%5\x1b\\\x1bP2$t9/17\x1b\\");
    assert_eq!(cursor(&u), (6, 12));
    assert!(u.modes().origin);
    let restored = reply(&mut u, b"\x1b[1$w");
    assert_eq!(restored, cir);
    assert_eq!(reply(&mut u, b"\x1b[2$w"), tabs);
}

#[test]
fn da3_and_upss() {
    let mut t = term(Model::Vt420);
    assert_eq!(reply(&mut t, b"\x1b[=c"), "ESC P!|00000000ESC \\");
    assert_eq!(reply(&mut t, b"\x1b[&u"), "ESC P0!u%5ESC \\");
    assert_eq!(reply(&mut t, b"\x1bP1!uA\x1b\\\x1b[&u"), "ESC P1!uAESC \\");
    // With Latin-1 preferred, typed characters use Latin-1 codes.
    t.type_text("¤");
    assert_eq!(t.take_output(), b"\xa4");
}

// ------------------------------------------------------ charsets

#[test]
fn national_sets_need_nrc_mode() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b(K[");
    assert_eq!(row(&t, 1), "[", "ignored in multinational mode");
    t.advance(b"\x1b[?42h\x1b(K[{~");
    assert_eq!(row(&t, 1), "[Ääß");
}

#[test]
fn national_mode_keyboard() {
    let mut t = Terminal::new(Config {
        keyboard_language: Some(Nrc::German),
        national_mode: true,
        ..Config::default()
    });
    t.type_text("Grüße [x]");
    assert_eq!(t.take_output(), b"Gr}~e x");
}

#[test]
fn dec_technical_and_supplemental_designators() {
    let mut t = term(Model::Vt320);
    t.advance(b"\x1b*>\x1bnD\x1b+%5\x1bo\x57");
    assert_eq!(row(&t, 1), "ΔŒ");
    let mut vt220 = term(Model::Vt220);
    vt220.advance(b"\x1b(>D");
    assert_eq!(row(&vt220, 1), "D", "no DEC Technical on the VT220");
}

// ------------------------------------------------- status line

#[test]
fn host_writable_status_line() {
    let mut t = term(Model::Vt420);
    assert_eq!(t.status_display(), StatusDisplay::Indicator);
    t.advance(b"\x1b[5;5Hmain\x1b[2$~\x1b[1$}\x1b[1;30HSTATUS\n\x1b[2;3Hxx");
    assert!(t.status_active());
    assert_eq!(row_text(&t, 4), "    main");
    let status: String = t.status_line().cells().iter().map(|c| c.ch).collect();
    assert_eq!(status.trim_end(), "  xx                         STATUS");
    t.advance(b"\x1b[0$}!");
    assert!(!t.status_active());
    assert_eq!(
        row_text(&t, 4),
        "    main!",
        "cursor returns to the main display"
    );
    t.advance(b"\x1b[0$~");
    assert_eq!(t.status_display(), StatusDisplay::None);
}

#[test]
fn status_line_needs_vt320() {
    let mut t = term(Model::Vt220);
    assert_eq!(t.status_display(), StatusDisplay::None);
    t.advance(b"\x1b[2$~\x1b[1$}X");
    assert!(!t.status_active());
    assert_eq!(row(&t, 1), "X");
}

// ------------------------------------------------ user-defined keys

#[test]
fn decudk_defines_shifted_function_keys() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1bP0;1|17/444952;29/48454C500D\x1b\\");
    t.key(Key::UserDefined(6));
    t.key(Key::UserDefined(16));
    t.key(Key::UserDefined(7));
    assert_eq!(t.take_output(), b"DIRHELP\r");
    assert_eq!(reply(&mut t, b"\x1b[?25n"), "ESC [?20n");
    // Default Pl locks the keys; later definitions are then ignored.
    t.advance(b"\x1bP1|18/41\x1b\\\x1bP1;1|19/42\x1b\\");
    assert_eq!(reply(&mut t, b"\x1b[?25n"), "ESC [?21n");
    t.key(Key::UserDefined(7));
    t.key(Key::UserDefined(8));
    assert_eq!(t.take_output(), b"A");
    t.unlock_user_keys();
    t.advance(b"\x1bP1;1|19/42\x1b\\");
    t.key(Key::UserDefined(8));
    assert_eq!(t.take_output(), b"B");
}

// ------------------------------------------------- soft fonts

#[test]
fn decdld_soft_set_is_selected_by_its_designator() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1bP1;1;1;10;0;0;16;0{ @~~~~~~~~~~/~~~~~~~~~~/????????~~\x1b\\");
    t.advance(b"\x1b( @!\"\x1b(B!");
    let cells = t.grid().line(0).cells();
    let soft = cells[0].ch;
    assert_eq!(u32::from(soft), SOFT_BASE + 0x21);
    let glyph = t.soft_glyph(soft).expect("glyph loaded");
    assert_eq!((glyph.width, glyph.height), (10, 16));
    assert_eq!(glyph.rows[0], 0x3FF);
    assert_eq!(glyph.rows[12], 0x300);
    assert!(
        t.soft_glyph(cells[1].ch).is_none(),
        "position 2/2 was not loaded"
    );
    assert_eq!(cells[2].ch, '!');
    assert!(t.soft_font_generation() > 0);
    let _ = ERROR_CHARACTER;
}

#[test]
fn column_switch_never_sets_a_tab_stop_in_column_one() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[?3h\x1b[?3l");
    assert_eq!(
        reply(&mut t, b"\x1b[2$w"),
        "ESC P2$u9/17/25/33/41/49/57/65/73ESC \\"
    );
}
