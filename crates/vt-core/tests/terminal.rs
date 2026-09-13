//! Behaviour of individual control functions. References: DEC STD 070,
//! VT100 User Guide (EK-VT100-UG), VT102 User Guide, VT220/VT420 Programmer
//! References (EK-VT220-RM, EK-VT420-RM).

use vt_core::cell::Flags;
use vt_core::dump::row_text;
use vt_core::grid::LineSize;
use vt_core::{Config, Event, Key, Model, Terminal};

fn term(model: Model) -> Terminal {
    Terminal::new(Config {
        model,
        ..Config::default()
    })
}

fn small(model: Model, rows: usize, cols: usize) -> Terminal {
    Terminal::new(Config {
        model,
        rows,
        cols,
        ..Config::default()
    })
}

fn row(t: &Terminal, r: usize) -> String {
    row_text(t, r - 1)
}

fn cursor(t: &Terminal) -> (usize, usize) {
    (t.cursor().row + 1, t.cursor().col + 1)
}

// ------------------------------------------------------------ defaults

#[test]
fn power_up_uses_dec_factory_setup() {
    let t = Terminal::new(Config::default());
    assert_eq!(t.config().model, Model::Vt420);
    assert!(!t.modes().autowrap, "DEC factory Set-Up: no auto wrap");
    assert!(!t.modes().new_line);
    assert!(t.modes().send_receive, "no local echo");
    assert!(t.modes().ansi);
    assert!(t.modes().cursor_visible);
    assert!(!t.eight_bit_replies(), "7-bit controls transmitted");
    assert_eq!((t.grid().rows(), t.grid().cols()), (24, 80));
    assert_eq!(t.level(), 4);
}

// --------------------------------------------------------------- autowrap

#[test]
fn last_column_flag_defers_wrap() {
    let mut t = small(Model::Vt102, 3, 5);
    t.advance(b"\x1b[?7hABCDE");
    assert_eq!(cursor(&t), (1, 5));
    assert!(t.cursor().pending_wrap);
    t.advance(b"F");
    assert_eq!(row(&t, 1), "ABCDE");
    assert_eq!(row(&t, 2), "F");
    assert!(t.grid().line(0).wrapped);
}

#[test]
fn no_autowrap_overwrites_last_column() {
    let mut t = small(Model::Vt102, 3, 5);
    t.advance(b"ABCDEFG");
    assert_eq!(row(&t, 1), "ABCDG");
    assert_eq!(cursor(&t), (1, 5));
}

#[test]
fn backspace_from_last_column_flag_moves_left() {
    let mut t = small(Model::Vt102, 3, 5);
    t.advance(b"\x1b[?7hABCDE\x08X");
    assert_eq!(row(&t, 1), "ABCXE");
}

#[test]
fn cursor_motion_clears_last_column_flag() {
    let mut t = small(Model::Vt102, 3, 5);
    t.advance(b"\x1b[?7hABCDE\x1b[1;5HZ");
    assert_eq!(row(&t, 1), "ABCDZ");
    assert_eq!(row(&t, 2), "");
}

// --------------------------------------------------------- cursor motion

#[test]
fn cup_defaults_and_clamping() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[5;10H");
    assert_eq!(cursor(&t), (5, 10));
    t.advance(b"\x1b[H");
    assert_eq!(cursor(&t), (1, 1));
    t.advance(b"\x1b[0;0f");
    assert_eq!(cursor(&t), (1, 1));
    t.advance(b"\x1b[99;999H");
    assert_eq!(cursor(&t), (24, 80));
}

#[test]
fn cursor_movement_stops_at_margins() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[5;20r\x1b[10;1H\x1b[99A");
    assert_eq!(cursor(&t), (5, 1), "CUU stops at top margin");
    t.advance(b"\x1b[99B");
    assert_eq!(cursor(&t), (20, 1), "CUD stops at bottom margin");
    t.advance(b"\x1b[2;1H\x1b[99B");
    assert_eq!(
        cursor(&t),
        (20, 1),
        "from above the region CUD stops at the bottom margin"
    );
    t.advance(b"\x1b[22;1H\x1b[99A");
    assert_eq!(
        cursor(&t),
        (5, 1),
        "from below the region CUU stops at the top margin"
    );
    t.advance(b"\x1b[24;1H\x1b[99B");
    assert_eq!(cursor(&t), (24, 1));
}

#[test]
fn origin_mode_addresses_relative_to_margins() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[5;10r\x1b[?6h");
    assert_eq!(cursor(&t), (5, 1), "DECOM homes to the top margin");
    t.advance(b"\x1b[3;4H");
    assert_eq!(cursor(&t), (7, 4));
    t.advance(b"\x1b[99;1H");
    assert_eq!(cursor(&t), (10, 1), "clamped to the bottom margin");
    t.advance(b"\x1b[6n");
    assert_eq!(
        t.take_output(),
        b"\x1b[6;1R",
        "CPR is relative in origin mode"
    );
}

#[test]
fn invalid_decstbm_is_ignored() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[3;7H\x1b[10;10r");
    assert_eq!(t.margins(), (0, 23));
    assert_eq!(cursor(&t), (3, 7), "ignored DECSTBM does not home");
}

#[test]
fn tabs_default_and_set_clear() {
    let mut t = term(Model::Vt102);
    t.advance(b"\tA");
    assert_eq!(cursor(&t), (1, 10));
    t.advance(b"\x1b[3g\x1b[1;5H\x1bH\x1b[1;1H\tB");
    assert_eq!(cursor(&t), (1, 6));
    t.advance(b"\t\t");
    assert_eq!(
        cursor(&t),
        (1, 80),
        "HT with no further stops goes to the right margin"
    );
}

// ------------------------------------------------------------- scrolling

#[test]
fn index_scrolls_region_and_feeds_scrollback_only_from_top() {
    let mut t = small(Model::Vt102, 4, 10);
    t.advance(b"1\r\n2\r\n3\r\n4\r\n5");
    assert_eq!(row(&t, 1), "2");
    assert_eq!(row(&t, 4), "5");
    assert_eq!(t.scrollback().len(), 1);

    t.advance(b"\x1b[2;3r\x1b[3;1H\n");
    assert_eq!(
        (row(&t, 1), row(&t, 2), row(&t, 3), row(&t, 4)),
        ("2".into(), "4".into(), "".into(), "5".into())
    );
    assert_eq!(
        t.scrollback().len(),
        1,
        "partial-region scroll does not feed scrollback"
    );
}

#[test]
fn reverse_index_scrolls_down_at_top_margin() {
    let mut t = small(Model::Vt102, 3, 10);
    t.advance(b"a\r\nb\r\nc\x1b[1;1H\x1bM");
    assert_eq!(
        (row(&t, 1), row(&t, 2), row(&t, 3)),
        ("".into(), "a".into(), "b".into())
    );
}

#[test]
fn line_feed_below_region_does_not_scroll() {
    let mut t = small(Model::Vt102, 5, 10);
    t.advance(b"top\x1b[1;3r\x1b[5;1Hx\n\n");
    assert_eq!(row(&t, 1), "top");
    assert_eq!(cursor(&t), (5, 2));
}

#[test]
fn new_line_mode_affects_lf() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[20hab\ncd");
    assert_eq!(row(&t, 2), "cd");
}

// ---------------------------------------------------------------- erasing

#[test]
fn erase_in_display_and_line() {
    let mut t = small(Model::Vt102, 3, 6);
    t.advance(b"\x1b#8\x1b[2;3H\x1b[K");
    assert_eq!(row(&t, 2), "EE");
    t.advance(b"\x1b[1K");
    assert_eq!(row(&t, 2), "");
    t.advance(b"\x1b[1;4H\x1b[1J");
    assert_eq!(row(&t, 1), "    EE");
    t.advance(b"\x1b[3;2H\x1b[J");
    assert_eq!(row(&t, 3), "E");
    t.advance(b"\x1b[2J");
    assert!((1..=3).all(|r| row(&t, r).is_empty()));
}

#[test]
fn ed_resets_line_size_of_completely_erased_lines() {
    // VT510 RM, ED: "When you erase complete lines, they become single-height,
    // single-width lines".
    let mut t = small(Model::Vt102, 3, 10);
    t.advance(b"\x1b#6\x1b[2;1H\x1b#3\x1b[3;1H\x1b#6\x1b[2;2H\x1b[J");
    assert_eq!(t.grid().line(0).size, LineSize::DoubleWidth);
    assert_eq!(
        t.grid().line(1).size,
        LineSize::DoubleHeightTop,
        "partly erased line keeps its size"
    );
    assert_eq!(t.grid().line(2).size, LineSize::Single);
    t.advance(b"\x1b[2;1H\x1b[J");
    assert_eq!(
        t.grid().line(1).size,
        LineSize::Single,
        "erased from column 1: complete"
    );
}

// ------------------------------------------------------ VT102 editing

#[test]
fn insert_and_delete_lines_within_region() {
    let mut t = small(Model::Vt102, 5, 10);
    t.advance(b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[3;5H\x1b[L");
    let rows: Vec<String> = (1..=5).map(|r| row(&t, r)).collect();
    assert_eq!(rows, ["1", "2", "", "3", "5"]);
    assert_eq!(cursor(&t), (3, 1), "IL moves to the left margin");
    t.advance(b"\x1b[2M");
    let rows: Vec<String> = (1..=5).map(|r| row(&t, r)).collect();
    assert_eq!(rows, ["1", "2", "", "", "5"]);
    t.advance(b"\x1b[5;1H\x1b[L");
    assert_eq!(row(&t, 5), "5", "IL outside the region is ignored");
}

#[test]
fn delete_character_and_insert_mode() {
    let mut t = small(Model::Vt102, 2, 8);
    t.advance(b"ABCDEFGH\x1b[1;3H\x1b[2P");
    assert_eq!(row(&t, 1), "ABEFGH");
    t.advance(b"\x1b[4hxy\x1b[4l");
    assert_eq!(row(&t, 1), "ABxyEFGH");
}

#[test]
fn ich_is_a_vt220_function() {
    let mut t = small(Model::Vt102, 2, 8);
    t.advance(b"ABCD\x1b[1;2H\x1b[2@");
    assert_eq!(row(&t, 1), "ABCD", "VT102 has no ICH");
    let mut t = small(Model::Vt220, 2, 8);
    t.advance(b"ABCD\x1b[1;2H\x1b[2@");
    assert_eq!(row(&t, 1), "A  BCD");
}

#[test]
fn vt100_ignores_vt102_editing() {
    let mut t = small(Model::Vt100, 3, 8);
    t.advance(b"ABCD\x1b[1;1H\x1b[P\x1b[L");
    assert_eq!(row(&t, 1), "ABCD");
}

// -------------------------------------------------------------- DECCOLM

#[test]
fn deccolm_clears_homes_resets_margins_keeps_tabs() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[3g\x1b[1;5H\x1bHtext\x1b[5;10r\x1b[12;12H\x1b[?3h");
    assert_eq!(t.grid().cols(), 132);
    assert_eq!(row(&t, 1), "");
    assert_eq!(cursor(&t), (1, 1));
    assert_eq!(t.margins(), (0, 23));
    assert_eq!(t.take_events(), vec![Event::ColumnsChanged(132)]);
    t.advance(b"\t");
    assert_eq!(cursor(&t), (1, 5), "tab stops survive DECCOLM");
}

// ---------------------------------------------------- line attributes

#[test]
fn double_width_line_loses_right_half_and_halves_width() {
    let mut t = small(Model::Vt102, 2, 10);
    t.advance(b"0123456789\x1b#6");
    assert_eq!(row(&t, 1), "01234");
    t.advance(b"\x1b[1;10H");
    assert_eq!(cursor(&t), (1, 5));
    t.advance(b"\x1b#5\x1b[1;10H");
    assert_eq!(cursor(&t), (1, 10));
}

#[test]
fn decaln_fills_resets_margins_and_homes() {
    let mut t = small(Model::Vt102, 3, 4);
    t.advance(b"\x1b[2;3r\x1b[3;3H\x1b#8");
    assert!((1..=3).all(|r| row(&t, r) == "EEEE"));
    assert_eq!(cursor(&t), (1, 1));
    assert_eq!(t.margins(), (0, 2));
}

// -------------------------------------------------------- save/restore

#[test]
fn decsc_decrc_restore_position_attributes_and_charsets() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[3;4H\x1b[1;7m\x1b(0\x1b7\x1b[H\x1b[m\x1b(B\x1b8q");
    assert_eq!(cursor(&t), (3, 5));
    let cell = t.grid().line(2).cells()[3];
    assert_eq!(cell.ch, '─');
    assert!(cell.attrs.flags.contains(Flags::BOLD | Flags::REVERSE));
}

#[test]
fn decrc_without_save_homes_and_resets() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[5;5H\x1b[1m\x1b8x");
    assert_eq!(cursor(&t), (1, 2));
    assert!(t.grid().line(0).cells()[0].attrs.flags.is_empty());
}

// ------------------------------------------------------------------ SGR

#[test]
fn sgr_vt100_attributes() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[1;4;5;7mA\x1b[0mB\x1b[1;4;;5mC");
    let cells = t.grid().line(0).cells();
    assert!(
        cells[0]
            .attrs
            .flags
            .contains(Flags::BOLD | Flags::UNDERLINE | Flags::BLINK | Flags::REVERSE)
    );
    assert!(cells[1].attrs.flags.is_empty());
    assert_eq!(cells[2].attrs.flags, Flags::BLINK, "empty parameter resets");
}

#[test]
fn sgr_vt220_additions_are_level_gated() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[1m\x1b[22mA\x1b[8mB");
    let cells = t.grid().line(0).cells();
    assert!(
        cells[0].attrs.flags.contains(Flags::BOLD),
        "VT102 ignores SGR 22"
    );
    assert!(!cells[1].attrs.flags.contains(Flags::INVISIBLE));
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b[1m\x1b[22mA\x1b[8mB");
    let cells = t.grid().line(0).cells();
    assert!(!cells[0].attrs.flags.contains(Flags::BOLD));
    assert!(cells[1].attrs.flags.contains(Flags::INVISIBLE));
}

#[test]
fn colour_requires_vt525_or_extension() {
    use vt_core::cell::Color;
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[31mA");
    assert_eq!(t.grid().line(0).cells()[0].attrs.fg, Color::Default);
    let mut t = term(Model::Vt525);
    t.advance(b"\x1b[31;44mA");
    let a = t.grid().line(0).cells()[0].attrs;
    assert_eq!((a.fg, a.bg), (Color::Indexed(1), Color::Indexed(4)));
}

#[test]
fn sequences_with_colon_subparameters_are_ignored_by_default() {
    let mut t = term(Model::Vt525);
    t.advance(b"\x1b[4:3mA");
    assert!(t.grid().line(0).cells()[0].attrs.flags.is_empty());
}

// ------------------------------------------------------------ charsets

#[test]
fn shift_in_shift_out_and_line_drawing() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b)0lqk\x0elqk\x0flqk");
    assert_eq!(row(&t, 1), "lqk┌─┐lqk");
}

#[test]
fn vt220_gr_is_dec_supplemental() {
    let mut t = term(Model::Vt220);
    t.advance(b"\xe9\xd7");
    assert_eq!(row(&t, 1), "éŒ");
}

#[test]
fn vt100_models_are_seven_bit() {
    let mut t = term(Model::Vt102);
    t.advance(b"\xc1\x9b[D\xc2");
    assert_eq!(row(&t, 1), "B", "bit 8 stripped: 0xC1 is A, 0x9B is ESC");
}

#[test]
fn single_shift_and_locking_shifts_need_vt220() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b+0\x1bOqq\x1boq\x1bnq");
    assert_eq!(
        row(&t, 1),
        "─q─ñ",
        "SS3 and LS3 use G3; LS2 invokes DEC Supplemental"
    );
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b+0\x1bOq");
    assert_eq!(row(&t, 1), "q", "VT102 has no G3 or single shifts");
}

#[test]
fn sub_displays_error_character() {
    let mut t = term(Model::Vt102);
    t.advance(b"A\x1aB");
    assert_eq!(row(&t, 1), "A▒B");
    let mut t = term(Model::Vt220);
    t.advance(b"A\x1b[1\x1aB");
    assert_eq!(row(&t, 1), "A\u{2426}B");
}

// ------------------------------------------------------------------ VT52

#[test]
fn vt52_mode_round_trip() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b[?2l");
    assert!(!t.modes().ansi);
    t.advance(b"\x1bY%*A\x1bFq\x1bGq\x1bZ");
    assert_eq!(cursor(&t), (6, 14));
    assert_eq!(row(&t, 6), "          A─q");
    assert_eq!(t.take_output(), b"\x1b/Z");
    t.advance(b"\x1b[2J");
    assert_eq!(
        row(&t, 6),
        "          A─q2J",
        "no CSI in VT52 mode: ESC [ is an escape pair"
    );
    t.advance(b"\x1b<");
    assert!(t.modes().ansi);
    assert_eq!(t.level(), 1, "leaving VT52 mode enters VT100 mode");
}

#[test]
fn vt52_direct_cursor_address_out_of_range_keeps_coordinate() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[?2l\x1bY%*");
    t.advance(b"\x1bY\x7f\x21");
    assert_eq!(
        cursor(&t),
        (6, 2),
        "line out of range: only the column changes"
    );
    t.advance(b"\x1bY\x22\x7f");
    assert_eq!(
        cursor(&t),
        (3, 2),
        "column out of range: only the line changes"
    );
}

// ------------------------------------------------------------- reports

#[test]
fn device_attributes_by_model() {
    for (model, primary, secondary) in [
        (Model::Vt100, &b"\x1b[?1;2c"[..], None),
        (Model::Vt102, b"\x1b[?6c", None),
        (
            Model::Vt220,
            b"\x1b[?62;1;2;6;7;8;9c",
            Some(&b"\x1b[>1;10;0c"[..]),
        ),
        (
            Model::Vt420,
            b"\x1b[?64;1;2;6;7;8;9;15;18;21c",
            Some(b"\x1b[>41;10;0c"),
        ),
    ] {
        let mut t = term(model);
        t.advance(b"\x1b[c");
        assert_eq!(t.take_output(), primary, "{model:?} DA1");
        t.advance(b"\x1bZ");
        assert_eq!(t.take_output(), primary, "{model:?} DECID");
        t.advance(b"\x1b[>c");
        assert_eq!(
            t.take_output(),
            secondary.unwrap_or_default(),
            "{model:?} DA2"
        );
    }
}

#[test]
fn s8c1t_selects_eight_bit_replies() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b G\x1b[5n");
    assert_eq!(t.take_output(), b"\x9b0n");
    t.advance(b"\x1b F\x1b[5n");
    assert_eq!(t.take_output(), b"\x1b[0n");
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b G\x1b[5n");
    assert_eq!(t.take_output(), b"\x1b[0n", "VT102 has no S8C1T");
}

#[test]
fn decreqtparm_only_through_vt320() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[x\x1b[1x");
    assert_eq!(
        t.take_output(),
        b"\x1b[2;1;1;112;112;1;0x\x1b[3;1;1;112;112;1;0x"
    );
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[x");
    assert!(t.take_output().is_empty());
}

#[test]
fn enq_sends_answerback() {
    let mut t = Terminal::new(Config {
        answerback: b"VEETEE".to_vec(),
        ..Config::default()
    });
    t.advance(b"\x05");
    assert_eq!(t.take_output(), b"VEETEE");
}

#[test]
fn decll_leds() {
    let mut t = term(Model::Vt102);
    t.advance(b"\x1b[1;3q");
    assert_eq!(t.leds(), 0b0101);
    t.advance(b"\x1b[0;2q");
    assert_eq!(t.leds(), 0b0010);
}

// ---------------------------------------------------------------- reset

#[test]
fn ris_restores_power_up_state() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b[?7h\x1b[5;10r\x1b[1mtext\x1b[?3h\x1b G\x1bc");
    assert_eq!(row(&t, 1), "");
    assert_eq!(t.grid().cols(), 80);
    assert_eq!(t.margins(), (0, 23));
    assert!(!t.modes().autowrap);
    assert!(!t.eight_bit_replies());
    assert_eq!(cursor(&t), (1, 1));
}

#[test]
fn decstr_soft_reset() {
    let mut t = term(Model::Vt220);
    t.advance(b"\x1b[?7h\x1b[?6h\x1b[4h\x1b[5;10r\x1b[?25l\x1b[1m\x1b[!p");
    let m = t.modes();
    assert!(!m.autowrap && !m.origin && !m.insert && m.cursor_visible);
    assert_eq!(t.margins(), (0, 23));
}

// ------------------------------------------------------------- keyboard

#[test]
fn keys_follow_terminal_modes() {
    let mut t = term(Model::Vt420);
    t.key(Key::Up);
    t.advance(b"\x1b[?1h\x1b=");
    t.key(Key::Up);
    t.key(Key::Keypad(5));
    t.key(Key::Function(16));
    assert_eq!(t.take_output(), b"\x1b[A\x1bOA\x1bOu\x1b[29~");
}

#[test]
fn keyboard_action_mode_locks_keyboard() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[2h");
    t.key(Key::Return);
    t.type_text("x");
    assert!(t.take_output().is_empty());
}

#[test]
fn local_echo_when_srm_reset() {
    let mut t = term(Model::Vt420);
    t.advance(b"\x1b[12l");
    t.type_text("hi");
    assert_eq!(t.take_output(), b"hi");
    assert_eq!(row(&t, 1), "hi");
}

#[test]
fn typed_text_uses_dec_multinational() {
    let mut t = term(Model::Vt420);
    t.type_text("aé€");
    assert_eq!(t.take_output(), b"a\xe9");
}

// ------------------------------------------------------------- resizing

#[test]
fn shrinking_below_cursor_pushes_lines_to_scrollback() {
    let mut t = small(Model::Vt420, 5, 10);
    t.advance(b"1\r\n2\r\n3\r\n4\r\n5");
    t.resize(3, 10);
    assert_eq!((row(&t, 1), row(&t, 3)), ("3".into(), "5".into()));
    assert_eq!(cursor(&t), (3, 2));
    assert_eq!(t.scrollback().len(), 2);
}

// ------------------------------------------------------------ selection

mod selection {
    use super::*;
    use vt_core::{Point, Selection};

    fn sel(r0: usize, c0: usize, r1: usize, c1: usize) -> Selection {
        Selection::new(Point { row: r0, col: c0 }, Point { row: r1, col: c1 })
    }

    #[test]
    fn text_across_lines_trims_and_joins() {
        let mut t = small(Model::Vt420, 4, 10);
        t.advance(b"$ DIR\r\nLOGIN.COM\r\nMAIL.MAI");
        assert_eq!(
            t.selection_text(&sel(0, 0, 2, 9)),
            "$ DIR\nLOGIN.COM\nMAIL.MAI"
        );
        assert_eq!(
            t.selection_text(&sel(2, 3, 0, 2)),
            "DIR\nLOGIN.COM\nMAIL",
            "either direction"
        );
        assert_eq!(t.selection_text(&sel(1, 2, 1, 4)), "GIN");
    }

    #[test]
    fn autowrapped_lines_join_without_newline() {
        let mut t = small(Model::Vt420, 3, 5);
        t.advance(b"\x1b[?7hABCDEFG");
        assert_eq!(t.selection_text(&sel(0, 0, 1, 4)), "ABCDEFG");
    }

    #[test]
    fn line_drawing_and_double_width() {
        let mut t = small(Model::Vt420, 3, 10);
        t.advance(b"\x1b(0lqk\x1b(B\r\n\x1b#6WIDE");
        assert_eq!(t.selection_text(&sel(0, 0, 1, 9)), "┌─┐\nWIDE");
    }

    #[test]
    fn double_click_selects_vms_file_specification() {
        let mut t = small(Model::Vt420, 2, 60);
        t.advance(b"$ TYPE DKA0:[SYS0.SYSCOMMON]LOGIN.COM;1, \"quoted\"");
        let w = t.word_at(Point { row: 0, col: 20 });
        assert_eq!(t.selection_text(&w), "DKA0:[SYS0.SYSCOMMON]LOGIN.COM;1");
        let q = t.word_at(Point { row: 0, col: 45 });
        assert_eq!(t.selection_text(&q), "quoted");
        let line = t.line_at(Point { row: 0, col: 3 });
        assert_eq!(
            t.selection_text(&line),
            "$ TYPE DKA0:[SYS0.SYSCOMMON]LOGIN.COM;1, \"quoted\""
        );
    }
}
