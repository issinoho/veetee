# Printing

A plan. Written 2026-09-25 against veetee 1.4.0; check the code before acting on it.

## Why, and what a DEC terminal does

Every terminal from the VT102 on has a printer port, and OpenVMS knows it: `SHOW TERMINAL`
lists *Printer port* for veetee today, because `SET TERMINAL/INQUIRE` reads it from the Device
Attributes reply. The host can then print on the terminal's printer, and the user can print what
is on the screen. A DEC terminal does that in four ways:

- **Print Screen** (F2 on the LK401, or `CSI i` from the host): the scrolling region or the whole
  page (DECPEX), then a form feed if DECPFF is set.
- **Print cursor line** (`CSI ? 1 i`).
- **Auto print** (`CSI ? 5 i` … `CSI ? 4 i`): each line goes to the printer as the cursor leaves it
  by a line feed, form feed, vertical tab or wrap.
- **Printer controller** (`CSI 5 i` … `CSI 4 i`): everything the host sends goes to the printer and
  *not* to the screen, until the terminator. This is how an application prints on the user's
  desk: a report, a form, a label.

And the VT52 equivalents: `ESC ]` print screen, `ESC V` print cursor line, `ESC ^`/`ESC _`
auto print on and off, `ESC W`/`ESC X` controller on and off. The host asks after the printer
with `CSI ? 15 n`; the answer is `CSI ? 10 n` ready, `? 11 n` not ready, `? 13 n` none.

🔎 To be checked against the printing chapters of EK-VT420-RM and EK-VT520-RM before building,
and cited in the code: the full list of `CSI ? … i` functions (print composed display, print all
pages, the VT500 printer-to-host session), what controller mode passes through untouched (the
7-bit and 8-bit forms of the terminator, and whether XON/XOFF are still acted on), and what the
Printer Set-Up screens' *print mode*, *print extent*, *print terminator* and *printer type* do.

## Where it stands

| Part | State |
|---|---|
| DSR printer status | Answers "no printer" (`CSI ? 13 n`) |
| DECPFF, DECPEX | Kept and reported as modes, acted on by nothing |
| Printer Set-Up screens and VT500 Printer menu | Settings kept and saved, acting on nothing |
| Print Screen key (F2) | Shows "Printing is not available yet" |
| `CSI … i` (Media Copy) | **Ignored — and so printer controller data appears on the screen.** Checked 25 September 2026: `CSI 5 i FOR THE PRINTER CSI 4 i` paints `FOR THE PRINTER`. The roadmap had said print data was swallowed; it is not. |
| VT52 printer functions | Ignored, with the same result for `ESC W` |

The last two are a fault whatever else is done: a host that prints to the terminal's printer
paints its print job over the screen. Fixing that is the first step.

## The plan

### P1. The printer functions, in `vt-core`

**Done** (25 September 2026), in `vt-core/src/terminal/printer.rs`, tested in
`vt-core/tests/printer.rs`: printer controller mode, print screen, print cursor line and auto
print, in their ANSI and VT52 forms, and the printer status report. Print data no longer reaches
the screen; what is printed is handed out as `Event::Print`, which the window does not yet take
anywhere — that is P2. The 🔎 functions below are ignored until checked.

Headless and testable like the rest of the core. The terminal keeps a print buffer and hands out
**print jobs** as an event, the way it hands out replies: `Event::Print(Job)`, a job being the
text, as the lines and form feeds a printer would get, plus the raw bytes where the host sent
them for a printer (controller mode).

- **Printer controller mode**: from `CSI 5 i` (or `ESC W`) everything goes into the job and nothing
  reaches the parser, until the terminator — watched for in both its 7-bit and 8-bit forms and
  across the boundaries of reads. The display is untouched throughout. This alone fixes the
  fault above, whether or not anything is printed.
- **Print screen and print cursor line**: the page or the scrolling region (DECPEX), as text; DEC
  Special Graphics and the national and supplemental sets translated to Unicode, as copying
  already does; DECPFF's form feed at the end.
- **Auto print**: lines as the cursor leaves them.
- **DSR** answers "ready" when a printer is configured, and "none" when not, as now.

What counts as one job: a controller session is one; a print screen is one; auto print gathers
lines into a job that ends with auto print off. 🔎 A real printer has no jobs, only a stream of
paper; the boundaries are veetee's, chosen so that each thing printed becomes one document.

### P2. Printing to PDF, in the window

A job becomes a PDF, drawn with cairo — already part of the GTK stack veetee uses, so no new
dependency — in a monospaced font, at 66 lines a page, form feeds starting a new page, and 132
columns in landscape. Each job is a file in a folder the user picks (Documents to begin with),
named for the session and the time, never overwriting. A toast says where it went, with a button
to open it.

The Print Screen key and a *Print Screen* menu item work from here, and the indicator status
line shows the printer as it does on the VT420.

### P3. Printing to a real printer

The same drawing, sent through `GtkPrintOperation`, which is CUPS on Linux and the Windows print
system on Windows: the first print asks which printer, and after that jobs go straight to it, as
they would to a printer on the terminal's port. The destination — PDF folder, printer, or none —
is a setting in the window menu.

### P4. Passing a host's printer data through untouched

Printer controller data is often meant for a particular printer: escape sequences for bold,
condensed print, or a form's layout on a DEC LA-series or LN03. Drawn as text those are lost. A
*raw* destination sends the bytes exactly — to a file, or to a CUPS queue as `lp -o raw` — for
anyone who has the printer the host expects. Only after P2 and P3, and only if wanted.

### P5. Acceptance on OpenVMS

On MYI64, run by the user:

- Printer controller from DCL: `WRITE SYS$OUTPUT` with `ESC [5i`, some lines, `ESC [4i` — the
  lines reach the PDF and not the screen.
- Print Screen from a full-screen application: EVE, MONITOR.
- Auto print, and the DSR answer seen by an application that asks.
- Whatever application on the system prints to the terminal's printer, if one does: ALL-IN-1 and
  DECforms are the usual ones. 🔎 Not yet known what MYI64 has.

## Decisions

Settled on 25 September 2026, each as recommended below.

1. **Is a printer there by default?** A DEC terminal reports one only when one is plugged in. With
   PDF as the default destination, veetee could report "ready" out of the box and Print Screen
   would just work; with none, it keeps saying "no printer" until the user picks a destination,
   which is closer to the factory state. *Recommended: PDF, reporting ready* — the host cannot
   tell a PDF from paper, and a Print Screen key that does nothing is the worse surprise.
2. **Text or DEC's own glyphs in the PDF.** Text in a monospaced font is searchable and small;
   veetee's DEC fonts would look like the screen. *Recommended: text*, with the line-drawing and
   technical characters as their Unicode equivalents.
3. **Where P1 stops.** Controller mode fixed so print data no longer reaches the screen, even with
   printing otherwise off, is worth a release on its own.
