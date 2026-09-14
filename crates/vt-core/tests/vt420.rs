//! VT420 features (milestone M3). Reference: VT420 Programmer Reference
//! Manual (EK-VT420-RM-002), chapters 5, 6 and 9, and DEC STD 070.

use vt_core::cell::{Color, Flags};
use vt_core::dump::row_text;
use vt_core::{Config, Event, Extensions, Model, Terminal};

fn vt420() -> Terminal {
    Terminal::new(Config::default())
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

/// Fills rows 1..=n with a distinct letter per row, repeated `width` times.
fn letters(t: &mut Terminal, n: usize, width: usize) {
    for r in 0..n {
        let ch = (b'A' + r as u8) as char;
        t.advance(format!("\x1b[{};1H{}", r + 1, ch.to_string().repeat(width)).as_bytes());
    }
}

// --------------------------------------------------- left/right margins

#[test]
fn decslrm_needs_declrmm_and_homes_cursor() {
    let mut t = vt420();
    t.advance(b"\x1b[5;5H\x1b[10;20s");
    assert_eq!(t.lr_margins(), (0, 79), "without DECLRMM, CSI s is SCOSC");
    t.advance(b"\x1b[?69h\x1b[10;20s");
    assert_eq!(t.lr_margins(), (9, 19));
    assert_eq!(cursor(&t), (1, 1));
    t.advance(b"\x1b[?6h");
    assert_eq!(cursor(&t), (1, 10), "origin mode homes to the margins");
    t.advance(b"\x1b[?69l");
    assert_eq!(
        t.lr_margins(),
        (0, 79),
        "resetting DECLRMM clears the margins"
    );
}

#[test]
fn autowrap_and_scrolling_stay_inside_lr_margins() {
    let mut t = vt420();
    letters(&mut t, 4, 10);
    t.advance(b"\x1b[?7h\x1b[?69h\x1b[1;3r\x1b[3;6s\x1b[1;3H1234x");
    assert_eq!(row(&t, 1), "AA1234AAAA");
    assert_eq!(row(&t, 2), "BBxBBBBBBB", "wraps to the left margin");
    t.advance(b"\x1b[3;3H\n");
    assert_eq!(
        row(&t, 1),
        "AAxBBBAAAA",
        "IND scrolls only the margin columns"
    );
    assert_eq!(row(&t, 3), "CC    CCCC");
    assert_eq!(row(&t, 4), "DDDDDDDDDD");
}

#[test]
fn ich_dch_il_dl_respect_lr_margins() {
    let mut t = vt420();
    letters(&mut t, 3, 10);
    t.advance(b"\x1b[?69h\x1b[3;6s\x1b[1;4H\x1b[@");
    assert_eq!(row(&t, 1), "AAA AAAAAA");
    t.advance(b"\x1b[1;4H\x1b[2P");
    assert_eq!(row(&t, 1), "AAAA  AAAA");
    t.advance(b"\x1b[2;4H\x1b[L");
    assert_eq!(row(&t, 2), "BB    BBBB");
    assert_eq!(row(&t, 3), "CCBBBBCCCC");
    t.advance(b"\x1b[2;4H\x1b[M");
    assert_eq!(row(&t, 2), "BBBBBBBBBB");
    t.advance(b"\x1b[1;1H\x1b[@");
    assert_eq!(row(&t, 1), "AAAA  AAAA", "no effect outside the margins");
}

#[test]
fn decic_decdc_edit_columns_in_the_region() {
    let mut t = vt420();
    letters(&mut t, 3, 6);
    t.advance(b"\x1b[1;2r\x1b[1;2H\x1b['}");
    assert_eq!(row(&t, 1), "A AAAAA");
    assert_eq!(row(&t, 2), "B BBBBB");
    assert_eq!(row(&t, 3), "CCCCCC", "outside the top/bottom margins");
    t.advance(b"\x1b[1;1H\x1b[2'~");
    assert_eq!(row(&t, 1), "AAAAA");
    assert_eq!(row(&t, 2), "BBBBB");
}

#[test]
fn decbi_decfi_shift_at_margins() {
    let mut t = vt420();
    t.advance(b"\x1b[?69h\x1b[2;5sABCD\x1b[1;2H\x1b6");
    assert_eq!(row(&t, 1), "A BCD");
    t.advance(b"\x1b[1;5H\x1b9");
    assert_eq!(row(&t, 1), "ABCD");
    t.advance(b"\x1b[1;3H\x1b9");
    assert_eq!(cursor(&t), (1, 4));
}

#[test]
fn cpr_is_relative_to_margins_in_origin_mode() {
    let mut t = vt420();
    t.advance(b"\x1b[?69h\x1b[5;20r\x1b[10;30s\x1b[?6h\x1b[3;4H\x1b[6n");
    assert_eq!(reply(&mut t), "\x1b[3;4R");
    t.advance(b"\x1b[?6n");
    assert_eq!(reply(&mut t), "\x1b[?3;4;1R");
}

// ------------------------------------------------------- rectangles

#[test]
fn decfra_fills_with_current_rendition() {
    let mut t = vt420();
    t.advance(b"\x1b[1m\x1b[42;2;3;3;5$x");
    assert_eq!(row(&t, 1), "");
    assert_eq!(row(&t, 2), "  ***");
    assert_eq!(row(&t, 3), "  ***");
    assert!(
        t.grid().line(1).cells()[2]
            .attrs
            .flags
            .contains(Flags::BOLD)
    );
    t.advance(b"\x1b[31;1;1;1;1$x");
    assert_eq!(row(&t, 1), "", "control characters are not filled");
}

#[test]
fn decfra_uses_gl_and_gr_sets() {
    let mut t = vt420();
    t.advance(b"\x1b(0\x1b[113;1;1;1;3$x");
    assert_eq!(row(&t, 1), "───");
}

#[test]
fn rectangles_clamp_and_follow_origin_mode() {
    let mut t = vt420();
    t.advance(b"\x1b[88;23;78;99;99$x");
    assert_eq!(row(&t, 24), format!("{}XXX", " ".repeat(77)));
    t.advance(b"\x1b[10;12r\x1b[?6h\x1b[65;1;1;1;1$x\x1b[?6l");
    assert_eq!(row(&t, 10), "A");
    t.advance(b"\x1b[66;5;5;4;4$x");
    assert_eq!(row(&t, 4), "", "top > bottom is ignored");
}

#[test]
fn decera_erases_attributes_and_ignores_protection() {
    let mut t = vt420();
    t.advance(b"\x1b[1\"q\x1b[7mABCDE\x1b[1;2;1;4$z");
    assert_eq!(row(&t, 1), "A   E");
    let c = t.grid().line(0).cells()[1];
    assert!(c.attrs.flags.is_empty());
}

#[test]
fn decsera_keeps_protected_and_renditions() {
    let mut t = vt420();
    t.advance(b"\x1b[4mAB\x1b[1\"qCD\x1b[0\"qEF\x1b[${");
    assert_eq!(row(&t, 1), "  CD");
    assert!(
        t.grid().line(0).cells()[0]
            .attrs
            .flags
            .contains(Flags::UNDERLINE)
    );
}

#[test]
fn deccra_copies_with_attributes_and_clips() {
    let mut t = vt420();
    t.advance(b"\x1b[7mAB\x1b[m\r\nCD\x1b[1;1;2;2;1;23;79;1$v");
    assert_eq!(row(&t, 23), format!("{}AB", " ".repeat(78)));
    assert_eq!(row(&t, 24), format!("{}CD", " ".repeat(78)));
    assert!(
        t.grid().line(22).cells()[78]
            .attrs
            .flags
            .contains(Flags::REVERSE)
    );
    t.advance(b"\x1b[1;1;2;2;1;24;80;1$v");
    assert_eq!(
        row(&t, 24),
        format!("{}CA", " ".repeat(78)),
        "clipped at the page edge"
    );
}

#[test]
fn deccra_overlapping_copy_uses_source_before_change() {
    let mut t = vt420();
    t.advance(b"ABCDEF\x1b[1;1;1;5;1;1;2;1$v");
    assert_eq!(row(&t, 1), "AABCDE");
}

#[test]
fn deccara_stream_and_rectangle_extents() {
    let mut t = vt420();
    letters(&mut t, 3, 10);
    t.advance(b"\x1b[1;5;3;6;1;4$r");
    let bold = |t: &Terminal, r: usize, c: usize| {
        t.grid().line(r - 1).cells()[c - 1]
            .attrs
            .flags
            .contains(Flags::BOLD)
    };
    assert!(bold(&t, 1, 5) && bold(&t, 1, 10) && bold(&t, 2, 1) && bold(&t, 3, 6));
    assert!(!bold(&t, 1, 4) && !bold(&t, 3, 7));
    t.advance(b"\x1b[1;1;3;10;0$r\x1b[2*x\x1b[1;5;3;6;1$r");
    assert!(bold(&t, 2, 5) && !bold(&t, 2, 1) && !bold(&t, 1, 10));
    t.advance(b"\x1b[1;5;3;6;22$r");
    assert!(!bold(&t, 2, 5));
}

#[test]
fn decrara_toggles_attributes() {
    let mut t = vt420();
    t.advance(b"\x1b[2*x\x1b[4mAB\x1b[mCD\x1b[1;1;1;4;4$t");
    let flags: Vec<bool> = t.grid().line(0).cells()[..4]
        .iter()
        .map(|c| c.attrs.flags.contains(Flags::UNDERLINE))
        .collect();
    assert_eq!(flags, [false, false, true, true]);
}

#[test]
fn deccara_ignores_colour_on_a_monochrome_vt420() {
    let mut t = vt420();
    t.advance(b"AB\x1b[1;1;1;2;31$r");
    assert_eq!(t.grid().line(0).cells()[0].attrs.fg, Color::Default);
}

#[test]
fn rectangle_ops_are_vt420_only() {
    let mut t = Terminal::new(Config {
        model: Model::Vt320,
        ..Config::default()
    });
    t.advance(b"\x1b[65;1;1;1;1$x");
    assert_eq!(row(&t, 1), "");
}

// --------------------------------------------------------- checksums

#[test]
fn decrqcra_matches_hardware_checksum() {
    let mut t = vt420();
    t.advance(b"\x1b[1;1;1;1;1;1*y");
    assert_eq!(
        reply(&mut t),
        "\x1bP1!~0000\x1b\\",
        "erased cells count as zero"
    );
    t.advance(b"A\x1b[4mB\x1b[m\x1b[2;1;1;1;1;2*y");
    // -(0x41 + 0x42 + 0x10) & 0xFFFF
    assert_eq!(reply(&mut t), "\x1bP2!~FF6D\x1b\\");
}

#[test]
fn decrqcra_page_zero_sums_all_pages() {
    let mut t = vt420();
    t.advance(b"A\x1b[U");
    t.advance(b"A\x1b[1*y");
    assert_eq!(reply(&mut t), "\x1bP1!~FF7E\x1b\\");
    t.advance(b"\x1b[1;2*y");
    assert_eq!(reply(&mut t), "\x1bP1!~FFBF\x1b\\");
}

// ------------------------------------------------------------- pages

#[test]
fn np_pp_move_between_pages_and_home() {
    let mut t = vt420();
    assert_eq!(t.page(), (0, 6));
    t.advance(b"page1\x1b[5;5H\x1b[U");
    assert_eq!(t.page().0, 1);
    assert_eq!(cursor(&t), (1, 1));
    assert_eq!(row(&t, 1), "");
    t.advance(b"page2\x1b[9U");
    assert_eq!(t.page().0, 5, "clamped to the last page");
    t.advance(b"\x1b[3;3H\x1b[4 R");
    assert_eq!(
        (t.page().0, cursor(&t)),
        (1, (3, 3)),
        "PPB keeps the position"
    );
    assert_eq!(row(&t, 1), "page2");
    t.advance(b"\x1b[1 P");
    assert_eq!(row(&t, 1), "page1");
    t.advance(b"\x1b[2 Q\x1b[V");
    assert_eq!(t.page().0, 1);
}

#[test]
fn page_coupling_off_keeps_display() {
    let mut t = vt420();
    t.advance(b"shown\x1b[?64l\x1b[Uhidden");
    assert_eq!(t.page().0, 1);
    assert!(!t.cursor_on_display());
    assert_eq!(row_of(t.display_grid(), 0), "shown");
    t.advance(b"\x1b[?64h");
    t.advance(b"\x1b[ Q");
    assert!(t.cursor_on_display());
}

fn row_of(g: &vt_core::grid::Grid, r: usize) -> String {
    let s: String = g.line(r).cells().iter().map(|c| c.ch).collect();
    s.trim_end().to_string()
}

#[test]
fn decslpp_sets_page_length_and_page_count() {
    let mut t = vt420();
    t.advance(b"\x1b[72t");
    assert_eq!((t.grid().rows(), t.page().1), (72, 2));
    assert_eq!(t.take_events(), [Event::LinesChanged(72)]);
    assert_eq!(t.margins(), (0, 71), "default margins follow the page");
    t.advance(b"\x1b[5;30r\x1b[48t");
    assert_eq!(t.margins(), (4, 29), "set margins are kept");
    t.advance(b"\x1b[24t");
    assert_eq!(t.margins(), (0, 23), "margins beyond the page are reset");
    t.take_events();
    t.advance(b"\x1b[30t");
    assert_eq!(t.grid().rows(), 24, "unsupported lengths are ignored");
    t.advance(b"\x1b[$|");
    assert!(t.take_events().is_empty());
}

#[test]
fn decscpp_changes_width_without_clearing() {
    let mut t = vt420();
    t.advance(b"keep\x1b[132$|");
    assert_eq!((t.grid().cols(), row(&t, 1).as_str()), (132, "keep"));
    t.advance(b"\x1b[?3l");
    assert_eq!(
        (t.grid().cols(), row(&t, 1).as_str()),
        (80, ""),
        "DECCOLM clears"
    );
}

#[test]
fn vertical_coupling_pans_to_the_cursor() {
    let mut t = vt420();
    t.advance(b"\x1b[72t");
    assert_eq!(t.window(), (0, 24));
    t.advance(b"\x1b[40;1H");
    assert_eq!(t.window(), (16, 24));
    t.advance(b"\x1b[?61l\x1b[1;1H");
    assert_eq!(t.window(), (16, 24), "DECVCCM reset leaves the window");
    t.advance(b"\x1b[3T");
    assert_eq!(t.window(), (13, 24), "SD pans up");
    t.advance(b"\x1b[100S");
    assert_eq!(t.window(), (48, 24), "SU pans down to the end of the page");
}

#[test]
fn su_scrolls_with_xterm_compat() {
    let mut t = Terminal::new(Config {
        extensions: Extensions {
            xterm_compat: true,
            ..Extensions::default()
        },
        ..Config::default()
    });
    t.advance(b"one\r\ntwo\x1b[S");
    assert_eq!(row(&t, 1), "two");
}

#[test]
fn decsnls_and_decrqde() {
    let mut t = vt420();
    t.advance(b"\x1b[48t\x1b[36*|");
    assert_eq!(t.window(), (0, 36));
    t.advance(b"\x1b[40;1H\x1b[\"v");
    assert_eq!(reply(&mut t), "\x1b[36;80;1;5;1\"w");
}

// ------------------------------------------------------------ macros

#[test]
fn decdmac_and_decinvm() {
    let mut t = vt420();
    t.advance(b"\x1bP5;0;0!zHello\x1b[1m\x1b\\\x1b[5*zX");
    assert_eq!(row(&t, 1), "HelloX");
    assert!(
        t.grid().line(0).cells()[5]
            .attrs
            .flags
            .contains(Flags::BOLD)
    );
}

#[test]
fn decdmac_hex_with_repeats() {
    let mut t = vt420();
    t.advance(b"\x1bP1;0;1!z41!3;42;43\x1b\\\x1b[1*z");
    assert_eq!(row(&t, 1), "ABBBC");
}

#[test]
fn recursive_macros_terminate() {
    let mut t = vt420();
    t.advance(b"\x1bP0;0;0!zx\x1b[0*z\x1b\\\x1b[0*z");
    assert!(row(&t, 1).len() <= 20);
}

#[test]
fn decdmac_delete_all_and_space_report() {
    let mut t = vt420();
    t.advance(b"\x1b[?62n");
    assert_eq!(reply(&mut t), "\x1b[384*{");
    t.advance(b"\x1bP1;0;0!z0123456789ABCDEF\x1b\\\x1b[?62n");
    assert_eq!(reply(&mut t), "\x1b[383*{");
    t.advance(b"\x1bP2;1;0!z\x1b\\\x1b[?62n\x1b[1*z");
    assert_eq!(reply(&mut t), "\x1b[384*{");
    assert_eq!(row(&t, 1), "");
}

#[test]
fn memory_checksum_report() {
    let mut t = vt420();
    t.advance(b"\x1b[?63;7n");
    assert_eq!(reply(&mut t), "\x1bP7!~0000\x1b\\");
    t.advance(b"\x1bP1;0;0!zA\x1b\\\x1b[?63;7n");
    assert_eq!(reply(&mut t), "\x1bP7!~FFBF\x1b\\");
}

#[test]
fn ris_clears_macros_decstr_does_not() {
    let mut t = vt420();
    t.advance(b"\x1bP1;0;0!zM\x1b\\\x1b[!p\x1b[1*z");
    assert_eq!(row(&t, 1), "M");
    t.advance(b"\x1bc\x1b[1*z");
    assert_eq!(row(&t, 1), "");
}

// ------------------------------------------------------ state reports

#[test]
fn dectsr_round_trips_through_decrsts() {
    let mut t = vt420();
    t.advance(b"\x1b[?69h\x1b[5;10s\x1b[3;20r\x1b[2*x\x1b[?7l\x1b[3g\x1b[1;7H\x1bH\x1b[1$u");
    let report = reply(&mut t);
    assert!(
        report.starts_with("\x1bP1$s") && report.ends_with("\x1b\\"),
        "{report:?}"
    );
    let body = &report[2..report.len() - 2];
    let mut u = vt420();
    u.advance(format!("\x1bP1$p{}\x1b\\", &body[3..]).as_bytes());
    assert_eq!(u.lr_margins(), (4, 9));
    assert_eq!(u.margins(), (2, 19));
    assert!(u.modes().lr_margins && !u.modes().autowrap);
    u.advance(b"\x1b[1;1H\t");
    assert_eq!(cursor(&u), (1, 7));
    u.advance(b"\x1b[1$u");
    assert_eq!(reply(&mut u), report);
}

#[test]
fn decrqm_and_decrqss_report_vt420_state() {
    let mut t = vt420();
    t.advance(b"\x1b[?61$p\x1b[?69$p");
    assert_eq!(reply(&mut t), "\x1b[?61;1$y\x1b[?69;2$y");
    t.advance(b"\x1b[?69h\x1b[3;40s\x1bP$qs\x1b\\");
    assert_eq!(reply(&mut t), "\x1bP1$r3;40s\x1b\\");
}

// -------------------------------------------------- xterm compatibility

fn xterm() -> Terminal {
    Terminal::new(Config {
        extensions: Extensions {
            xterm_compat: true,
            ..Extensions::default()
        },
        ..Config::default()
    })
}

#[test]
fn xterm_compat_adds_size_report_scosc_and_plain_checksums() {
    let mut t = vt420();
    t.advance(b"\x1b[18t\x1b[5;5H\x1b[s\x1b[H\x1b[u");
    assert_eq!(reply(&mut t), "");
    assert_eq!(cursor(&t), (1, 1), "DEC terminals have no SCOSC/SCORC");

    let mut t = xterm();
    t.advance(b"\x1b[18t");
    assert_eq!(reply(&mut t), "\x1b[8;24;80t");
    t.advance(b"\x1b[5;5H\x1b[s\x1b[H\x1b[u");
    assert_eq!(cursor(&t), (5, 5));
    t.advance(b"\x1b[H\x1b[4mA\x1b[1;0;1;1;1;1*y");
    assert_eq!(reply(&mut t), "\x1bP1!~0041\x1b\\");
}

#[test]
fn decstr_resets_left_right_margin_mode() {
    let mut t = vt420();
    t.advance(b"\x1b[?69h\x1b[5;10s\x1b[!p");
    assert!(!t.modes().lr_margins);
    assert_eq!(t.lr_margins(), (0, 79));
}

/// DECSCLM: a VT420 powers up in smooth scroll (EK-VT420-RM 11), and paced
/// processing stops after each line that scrolls.
#[test]
fn smooth_scroll_paces_each_scrolled_line() {
    let mut t = Terminal::new(Config::default());
    assert_eq!(t.smooth_scroll_rate(), Some(9));
    t.advance(b"\x1b[24;1Htop");
    let data = b"\nb\nc";
    let n = t.advance_paced(data);
    assert_eq!(n, 1, "stops after the line feed that scrolled");
    let s = t.take_smooth_scroll().expect("a smooth scroll");
    assert!(s.up);
    assert_eq!((s.top, s.bottom, s.left, s.right), (0, 23, 0, 79));
    assert_eq!(t.advance_paced(&data[n..]), 2);
    assert!(t.take_smooth_scroll().is_some());
    assert_eq!(t.advance_paced(b"x"), 1);
    assert!(t.take_smooth_scroll().is_none());

    // Jump scroll does not pace.
    t.advance(b"\x1b[?4l");
    assert_eq!(t.smooth_scroll_rate(), None);
    assert_eq!(t.advance_paced(b"\n\n\n"), 3);
    assert!(t.take_smooth_scroll().is_none());

    // DECSSCLS (VT500): 4 is Smooth 4, 9 is jump.
    let mut vt510 = Terminal::new(Config {
        model: Model::Vt510,
        ..Config::default()
    });
    assert_eq!(vt510.smooth_scroll_rate(), Some(9));
    vt510.advance(b"\x1b[4 p");
    assert_eq!(vt510.smooth_scroll_rate(), Some(18));
    vt510.advance(b"\x1b[9 p");
    assert_eq!(vt510.smooth_scroll_rate(), None);

    let vt520 = Terminal::new(Config {
        model: Model::Vt520,
        ..Config::default()
    });
    assert_eq!(vt520.smooth_scroll_rate(), None, "VT520 factory: jump");
}
