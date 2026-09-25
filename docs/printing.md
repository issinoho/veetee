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

Released in 1.5.0 (P1 to P3); what follows is how it stood before, and now.

| Part | Before 1.5.0 | From 1.5.0 |
|---|---|---|
| DSR printer status | Answered "no printer" (`CSI ? 13 n`) | "Ready" (`CSI ? 10 n`) while a folder or printer is chosen, "no printer" where not |
| DECPFF, DECPEX | Kept and reported, acted on by nothing | A form feed after Print Screen; the page rather than the scrolling region |
| Printer Set-Up screens and VT500 Printer menu | Settings kept and saved, acting on nothing | Unchanged: print mode, extent and terminator come from the host's sequences |
| Print Screen key (F2) | Showed "Printing is not available yet" | Prints the screen |
| `CSI … i` (Media Copy) | **Ignored, so printer controller data appeared on the screen**: `CSI 5 i FOR THE PRINTER CSI 4 i` painted `FOR THE PRINTER` (checked 25 September 2026; the roadmap had said print data was swallowed) | Print screen, print cursor line, auto print and printer controller, each a print job; controller data never reaches the screen |
| VT52 printer functions | Ignored, with the same result for `ESC W` | As the ANSI forms |

Each print job goes to a PDF in a folder, or to a real printer chosen once (CUPS through `lp` on
Linux, GTK printing on Windows).

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

**Done** (25 September 2026), in `veetee/src/printing.rs`. Each job is a PDF in the printer
folder — Documents until *Print to Folder…* chooses another, kept in `printer.conf` — named for
the session and the time and never overwriting; a message says where it went, with *Open*. The
host is told a printer is ready while there is a folder. A job from the host is read as a plain
printer reads it: line feeds, form feeds, carriage returns that overprint, tabs every eight, the
upper half as Latin-1, and escape sequences for a particular printer passed over. Drawn with
cairo's own text in a monospaced font at six lines to the inch on the locale's paper, landscape
past 80 columns. F2 and *Print Screen* in the window menu print the screen, and the indicator
status line says *Printer: Ready*. Seen working in the window: a controller job and a print
screen each became a PDF holding what they should; the message and its *Open* button were not
seen, the check being made from outside the window.

A job becomes a PDF, drawn with cairo — already part of the GTK stack veetee uses, so no new
dependency — in a monospaced font, at 66 lines a page, form feeds starting a new page, and 132
columns in landscape. Each job is a file in a folder the user picks (Documents to begin with),
named for the session and the time, never overwriting. A toast says where it went, with a button
to open it.

The Print Screen key and a *Print Screen* menu item work from here, and the indicator status
line shows the printer as it does on the VT420.

### P3. Printing to a real printer

**Done** (25 September 2026), and seen on paper: the test page, Print Screen, and a job printed
by OpenVMS on the terminal's printer, on a Canon MX470 over CUPS. *Print to Printer…* shows the
system print dialog once, with a test page, and keeps the chosen printer; every job after goes
straight to it as a PDF through `lp`, with no dialog, across restarts. On Windows, GTK's print
operation does the same, compiled and not yet run. What printing on real paper found:

- **The desktop's print portal asks for every job.** GTK's `PrintDialog` hands over a setup the
  portal will honour once: the job after it showed the dialog again, and printed the host's job
  as a blank page. So the dialog only chooses the printer, and `lp` does the printing.
- **Bypassing the portal, GTK printed to whatever printer it found** when the one named was not
  there: a test meant for *Print to File* came out on the Canon. Jobs therefore go only to the
  printer chosen in the dialog, by its CUPS name.
- **A job has to name its paper.** Left to the printer's defaults, which here were 4×6 photo
  paper, every page stopped with a paper size error. Each job now says its size — the one the PDF
  was drawn at, from the locale — and plain paper.

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

On MYI64, run by the user. Done so far (25 September 2026):

- A line printed by DCL in printer controller mode came out on paper and not on the screen.
- A whole file, printed with `LPRINT.COM` — `TYPE/NOPAGE` wrapped in `ESC [5i` … `ESC [4i`, the
  terminal set `/NOWRAP/NOBROADCAST` around it — the procedure on the wiki's Printing page.
- Not veetee, but asked alongside: an OpenVMS **print queue** printing on the same printer, by
  TCP/IP Services' LPD client to `cups-lpd` and CUPS. It needs `:sh:` in the printcap and
  `-o job-sheets=none -o media=A4 -o media-type=stationery` on `cups-lpd`, or the printer feeds
  a blank sheet — for CUPS's own banner page, and for a job that asks for no paper and gets the
  printer's photo paper. Written up on the wiki's Printing page.

Still to do:

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
