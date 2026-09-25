//! The printer port: what reaches the printer, and what does not reach the
//! screen.

use vt_core::{Config, Event, PrintJob, Terminal};

fn terminal() -> Terminal {
    Terminal::new(Config::default())
}

fn screen(term: &Terminal) -> Vec<String> {
    (0..term.grid().rows())
        .map(|row| vt_core::dump::row_text(term, row))
        .filter(|line| !line.is_empty())
        .collect()
}

fn jobs(term: &mut Terminal) -> Vec<PrintJob> {
    term.take_events()
        .into_iter()
        .filter_map(|event| match event {
            Event::Print(job) => Some(job),
            _ => None,
        })
        .collect()
}

/// The one text job the last thing asked for.
fn one_text(term: &mut Terminal) -> String {
    match jobs(term).as_slice() {
        [PrintJob::Text(text)] => text.clone(),
        other => panic!("one text job, not {other:?}"),
    }
}

#[test]
fn printer_controller_data_goes_to_the_printer_and_not_the_screen() {
    // Until 25 September 2026 this painted "FOR THE PRINTER" on the screen.
    let mut term = terminal();
    term.advance(b"before\r\n\x1b[5iFOR THE PRINTER\r\n\x1b[4iafter");
    assert_eq!(screen(&term), ["before", "after"]);
    assert_eq!(
        jobs(&mut term),
        [PrintJob::Controller(b"FOR THE PRINTER\r\n".to_vec())]
    );
}

#[test]
fn the_terminator_is_found_however_the_reads_fall() {
    let stream = b"\x1b[5iline one\r\nline two\r\n\x1b[4iback";
    for split in 1..stream.len() {
        let mut term = terminal();
        term.advance(&stream[..split]);
        term.advance(&stream[split..]);
        assert_eq!(screen(&term), ["back"], "split at {split}");
        assert_eq!(
            jobs(&mut term),
            [PrintJob::Controller(b"line one\r\nline two\r\n".to_vec())],
            "split at {split}"
        );
    }
    // And a byte at a time.
    let mut term = terminal();
    for &byte in stream {
        term.advance(&[byte]);
    }
    assert_eq!(screen(&term), ["back"]);
    assert_eq!(jobs(&mut term).len(), 1);
}

#[test]
fn escape_sequences_meant_for_the_printer_are_passed_on_whole() {
    // Bold on an LA-series printer, and near misses of the terminator.
    let mut term = terminal();
    term.advance(b"\x1b[5i\x1b[1mBold\x1b[0m \x1b[4x \x1b[45i \x1b[4\x1b[4i");
    assert_eq!(
        jobs(&mut term),
        [PrintJob::Controller(
            b"\x1b[1mBold\x1b[0m \x1b[4x \x1b[45i \x1b[4".to_vec()
        )]
    );
    assert!(screen(&term).is_empty(), "none of it on the screen");
}

#[test]
fn the_eight_bit_terminator_ends_it_too() {
    let mut term = terminal();
    term.advance(b"\x1b[5iprinted\x9b4ishown");
    assert_eq!(screen(&term), ["shown"]);
    assert_eq!(jobs(&mut term), [PrintJob::Controller(b"printed".to_vec())]);
}

#[test]
fn print_screen_prints_the_scrolling_region_or_the_whole_page() {
    let mut term = terminal();
    // Lines 1 to 5, then a scrolling region of lines 2 to 4.
    term.advance(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\x1b[2;4r");
    term.advance(b"\x1b[i");
    assert_eq!(
        jobs(&mut term),
        [PrintJob::Text("two\nthree\nfour\n".into())]
    );

    // DECPEX: the whole page, blank lines included; DECPFF: a form feed.
    term.advance(b"\x1b[?19h\x1b[?18h\x1b[0i");
    let page = one_text(&mut term);
    assert!(
        page.starts_with("one\ntwo\nthree\nfour\nfive\n"),
        "{page:?}"
    );
    assert_eq!(
        page.trim_end_matches('\x0c').lines().count(),
        24,
        "every line of the page"
    );
    assert!(page.ends_with("\n\x0c"), "and a form feed");
}

#[test]
fn line_drawing_is_printed_as_the_characters_it_shows() {
    let mut term = terminal();
    term.advance(b"\x1b(0lqqk\r\nx  x\r\nmqqj\x1b(B\x1b[?19h\x1b[i");
    let page = one_text(&mut term);
    assert!(page.starts_with("┌──┐\n│  │\n└──┘\n"), "{page:?}");
}

#[test]
fn print_cursor_line_prints_the_line_the_cursor_is_on() {
    let mut term = terminal();
    term.advance(b"first\r\nsecond\x1b[?1i");
    assert_eq!(jobs(&mut term), [PrintJob::Text("second\n".into())]);
}

#[test]
fn auto_print_prints_each_line_as_the_cursor_leaves_it() {
    let mut term = terminal();
    term.advance(b"\x1b[?5ione\r\ntwo\r\n");
    assert!(jobs(&mut term).is_empty(), "gathered until auto print ends");
    // A line that wraps is left too.
    term.advance(&[b'x'; 85]);
    term.advance(b"\r\n\x1b[?4i");
    assert_eq!(
        jobs(&mut term),
        [PrintJob::Text(format!(
            "one\ntwo\n{}\n{}\n",
            "x".repeat(80),
            "x".repeat(5)
        ))]
    );
    // Off means off.
    term.advance(b"three\r\n");
    assert!(jobs(&mut term).is_empty());
    assert!(
        screen(&term).contains(&"three".to_string()),
        "shown as ever"
    );
}

#[test]
fn the_vt52_forms_do_the_same() {
    let mut term = terminal();
    term.advance(b"\x1b[?2l"); // VT52 mode
    term.advance(b"\x1bWnot shown\x1bXshown");
    assert_eq!(screen(&term), ["shown"]);
    assert_eq!(
        jobs(&mut term),
        [PrintJob::Controller(b"not shown".to_vec())]
    );
    term.advance(b"\x1bV");
    assert_eq!(jobs(&mut term), [PrintJob::Text("shown\n".into())]);
    term.advance(b"\x1b^\r\nmore\r\n\x1b_");
    assert_eq!(jobs(&mut term), [PrintJob::Text("shown\nmore\n".into())]);
}

#[test]
fn the_host_is_told_whether_there_is_a_printer() {
    let mut term = terminal();
    term.advance(b"\x1b[?15n");
    assert_eq!(term.take_output(), b"\x1b[?13n", "none, by default");
    let mut term = Terminal::new(Config {
        printer: true,
        ..Config::default()
    });
    term.advance(b"\x1b[?15n");
    assert_eq!(
        term.take_output(),
        b"\x1b[?10n",
        "ready, where one stands in"
    );
}
