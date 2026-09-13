# DEC compatibility matrix

Status of each control function, the models it applies to, and where the
behaviour comes from. veetee follows DEC documentation first; where DEC and
xterm differ, DEC behaviour is the default and xterm behaviour is an opt-in
profile setting.

Legend: ✅ implemented and tested · 🟡 partial · ⬜ not yet · 🔎 needs checking against a manual or real hardware

Sources: **STD070** DEC STD 070 Video Systems Reference Manual · **UG100** VT100 User Guide (EK-VT100-UG) ·
**UG102** VT102 User Guide · **RM220** VT220 Programmer Reference (EK-VT220-RM) · **RM420** VT420 Programmer
Reference (EK-VT420-RM) · **RM510** VT510 Video Terminal Programmer Information (EK-VT510-RM) · **vttest**
conformance scripts in `tests/conformance/vttest`.

## Parser (vt-parser)

| Area | Status | Notes |
|------|--------|-------|
| DEC ANSI state machine (Williams) | ✅ | CAN/SUB abort; C0 executed inside sequences |
| 7-bit / 8-bit / no-C1 / UTF-8 input | ✅ | VT100-class models and VT52 mode strip bit 8 (UG100) |
| VT52 escape parsing, `ESC Y` | ✅ | |
| Colon sub-parameters | ✅ | Sequences containing `:` are ignored unless the xterm SGR extension is on (DEC terminals ignore them) |
| Parameter limits | ✅ | 32 kept (STD070 requires ≥ 16), values saturate |

## VT100 / VT102 (M1)

| Function | Seq | Status | Behaviour notes / source |
|----------|-----|--------|--------------------------|
| CUU CUD CUF CUB | `CSI A–D` | ✅ | Stop at margins when inside the region (STD070) |
| CUP HVP | `CSI H`, `CSI f` | ✅ | DECOM relative & clamped to region |
| IND NEL RI | `ESC D E M`, C1 | ✅ | |
| HTS TBC HT | `ESC H`, `CSI g` | ✅ | HT stops at right margin |
| ED EL | `CSI J`, `CSI K` | ✅ | Completely erased lines, including the cursor line when erased entirely, revert to single width (RM510 ED). ED/EL ignore DECSCA |
| DECSTBM | `CSI r` | ✅ | Invalid (top ≥ bottom) ignored without homing |
| DECSC DECRC | `ESC 7 8` | ✅ | Saves position, SGR, DECSCA, G-sets, GL/GR & single shifts, DECOM and **DECAWM** ("wrap flag (autowrap or no autowrap)", RM510) |
| DECALN | `ESC # 8` | ✅ | Resets margins, homes cursor |
| DECDHL DECDWL DECSWL | `ESC # 3 4 5 6` | ✅ | Right half of line is lost on becoming double width (RM510) |
| SGR 0 1 4 5 7 | `CSI m` | ✅ | |
| SCS G0/G1 `B A 0 1 2` | `ESC ( )` | ✅ | Alternate ROM sets map to ASCII / special graphics |
| SI SO | | ✅ | |
| SM/RM KAM IRM SRM LNM | `CSI h/l` | ✅ | SRM reset = local echo of keyboard output |
| DECCKM DECANM DECCOLM DECSCLM DECSCNM DECOM DECAWM DECARM DECPFF DECPEX | `CSI ? h/l` | ✅ | DECCOLM clears page, resets margins, homes; **tab stops preserved** (STD070) |
| DECKPAM DECKPNM | `ESC = >` | ✅ | |
| DA, DECID | `CSI c`, `ESC Z` | ✅ | VT100 `?1;2c`, VT102 `?6c` |
| DSR 5 / 6 (CPR) | `CSI n` | ✅ | CPR relative in origin mode |
| DECREQTPARM | `CSI x` | ✅ | VT100–VT320 only (not in RM420) |
| DECLL | `CSI q` | ✅ | |
| DECTST | `CSI 4;Ps y` | 🟡 | Performs RIS; no visual self-test |
| RIS | `ESC c` | ✅ | Returns to Set-Up defaults; scrollback kept |
| ENQ answerback | | ✅ | Ctrl+Break also transmits it |
| SUB error character | | ✅ | ▒ on VT100-class; reversed question mark U+2426 on VT220+. 🔎 CAN display on VT100 |
| IL DL DCH | `CSI L M P` | ✅ | VT102 and later; ignored outside scroll region |
| ICH | `CSI @` | ✅ | **VT220 and later** — the VT102 does not have it (vttest, UG102) |
| VT52 mode | `ESC A–K Y Z = > < F G` | ✅ | Out-of-range `ESC Y` line **or column** leaves that coordinate unchanged (VT100 family; a real VT52 clamps) |
| VT52 printer functions | `ESC ^ _ W X ] V` | ⬜ | Post-1.0 printing |

## VT220 / VT320 / VT420 (M2)

| Function | Seq | Status | Behaviour notes / source |
|----------|-----|--------|--------------------------|
| DECSCA | `CSI Ps " q` | ✅ | VT220+. SGR 0 does not clear protection (RM510 DECSCA) |
| DECSED DECSEL | `CSI ? Ps J/K` | ✅ | Erase only unprotected characters; ED, EL, ECH, ICH, DCH ignore protection (vttest 11.1.2.4) |
| DECSCL | `CSI 6n ; Pc " p` | ✅ | **Performs a hard reset** (RM510), then selects level; Pc 1 = 7-bit, 0/2/omitted = 8-bit. Clamped to the model's highest level |
| DECSTR | `CSI ! p` | ✅ | Per RM510 table 5-9: also resets DECNRCM, DECSCA, UPSS, DECSASD and the saved cursor; DECSCNM, DECCOLM, tabs unchanged |
| S7C1T S8C1T | `ESC SP F/G` | ✅ | |
| DECTCEM | `CSI ? 25 h/l` | ✅ | VT220+ |
| SCS national sets | `ESC ( A…=`, `%6` | ✅ | Tables from EK-VT220-RM 2-5…2-15. Accepted only in NRC mode (DECNRCM) at level ≥ 2; British also in VT100 mode. 🔎 Dutch 7/13 transcription |
| SCS DEC Supplemental / Technical / UPSS | `%5`, `>`, `<` | ✅ | `<` is DEC Supplemental on the VT220, the user-preferred set on VT320+ |
| ISO Latin-1 (96) | `ESC - A` etc. | ✅ | VT320+ |
| Locking/single shifts | LS2 LS3 LS1R LS2R LS3R SS2 SS3 | ✅ | VT220+ |
| DECNRCM | `CSI ? 42 h/l` | ✅ | 7-bit: GR unavailable; keyboard uses the Set-Up national layout |
| DECAUPSS DECRQUPSS | `DCS Pn ! u`, `CSI & u` | ✅ | DEC Supplemental or ISO Latin-1 |
| DECDLD | `DCS … {` | ✅ | Two font buffers, 80/132-column renditions, erase modes 0–2; rendered from a soft glyph atlas. 🔎 VT220/VT320 default matrix sizes |
| DECUDK | `DCS Pc;Pl \|` | ✅ | Shifted F6–F20; default Pl locks; 804-byte capacity; invalid definition ends loading |
| DECSSDT DECSASD | `CSI Ps $ ~`, `CSI Ps $ }` | ✅ | VT320+. Status line: only column positioning applies; vertical motion ignored; RIS/DECSTR/DECSCL/DECCOLM exit it. Factory default: indicator |
| DA2 / DA3 | `CSI > c`, `CSI = c` | ✅ | DA3 (VT420+) reports unit ID `00000000` |
| DSR printer / UDK / keyboard | `CSI ? 15/25/26 n` | ✅ | No printer; keyboard type 1 = LK401 on VT420, 4 = LK411/LK450 on VT5xx (EK-VT420-RM p.277) |
| DECXCPR | `CSI ? 6 n` | ✅ | VT420+ |
| DSR data integrity / sessions | `CSI ? 75/85 n` | ✅ | VT420+ |
| DECRQM / DECRPM | `CSI [?] Ps $ p` | ✅ | VT320+. ISO modes 1,3,5,7,10,11,13–19 permanently reset |
| DECRQSS / DECRPSS | `DCS $ q … ST` | ✅ | SGR, DECSTBM, DECSCL, DECSCA, DECSASD, DECSSDT, DECSCPP/DECSLPP/DECSNLS (VT420). **Reply digit 1 = valid** as real VT420/VT520 terminals send (the VT510 manual has it reversed) |
| DECRQPSR DECCIR DECTABSR | `CSI 1/2 $ w` | ✅ | DECCIR reports the actual set designator (`%5`), as in DEC's own example; vttest expects `<` for UPSS |
| DECRSPS | `DCS 1/2 $ t` | ✅ | Invalid data stops the restore partway, as DEC documents |
| DECRQTSR / DECRSTS | `CSI 1 $ u`, `DCS 1 $ p` | ✅ | VT420+. RM420 leaves the data format to the implementation: veetee sends `VT1;modes;margins;page size;screen lines;status;DECSACE;C1;tabs` and restores exactly that |
| CHA HPA VPA CNL CPL CHT CBT | | ✅ | VT500 level only: the VT420 has none of them (RM420 chapter index). See the VT500 section |

## VT420 (M3)

| Function | Seq | Status | Behaviour notes / source |
|----------|-----|--------|--------------------------|
| DECLRMM DECSLRM | `CSI ? 69 h/l`, `CSI Pl ; Pr s` | ✅ | `CSI s` sets margins only while DECLRMM is set; homes the cursor. Resetting DECLRMM clears the margins; DECSTR resets it (DEC STD 070). Autowrap, IND/RI/LF, IL/DL, ICH/DCH, CR stay inside the margins; outside the margins nothing scrolls |
| DECIC DECDC | `CSI Pn ' }`, `CSI Pn ' ~` | ✅ | No effect with the cursor outside the margins (RM420) |
| DECBI DECFI | `ESC 6`, `ESC 9` | ✅ | Shift the region at the left/right margin |
| DECCRA | `CSI Pts;Pls;Pbs;Prs;Pps;Ptd;Pld;Ppd $ v` | ✅ | Characters and renditions copied between pages; lines keep their own size; clipped at the page edge; overlapping copies use the source as it was |
| DECERA DECSERA | `CSI Pt;Pl;Pb;Pr $ z`, `$ {` | ✅ | DECERA ignores protection; DECSERA keeps protected characters and all renditions |
| DECFRA | `CSI Pch;Pt;Pl;Pb;Pr $ x` | ✅ | Pch 32–126 / 160–255 through the in-use GL/GR sets, current SGR |
| DECCARA DECRARA DECSACE | `$ r`, `$ t`, `CSI Ps * x` | ✅ | DECCARA 0,1,4,5,7,22,24,25,27; DECRARA toggles 0,1,4,5,7. Stream extent by default, rectangle with DECSACE 2 |
| Rectangle coordinates | | ✅ | Relative to the origin in DECOM, not limited by margins, clamped to the page; top > bottom ignored |
| DECRQCRA / DECCKSR | `CSI Pid;Pp;Pt;Pl;Pb;Pr * y` | ✅ | Hardware checksum as measured on VT520s (vttest): code + attribute bits + colour index, negated; erased cells count 0. Pp 0 = all pages. **xterm compatibility**: plain sum of character codes on the current page (as esctest expects) |
| Page memory | NP PP PPA PPR PPB `CSI U/V`, `CSI SP P/Q/R` | ✅ | Six 24-line pages (RM420: 144 lines of memory); NP/PP home the cursor, PPA/PPR/PPB keep the position |
| DECSLPP DECSCPP | `CSI Pn t`, `CSI Pn $ \|` | ✅ | 24/25/36/48/72/144 lines → 6/5/4/3/2/1 pages; DECSCPP 80/132 keeps page contents (DECCOLM erases) |
| DECSNLS | `CSI Pn * \|` | ✅ | 24, 36 or 48 screen lines (next supported value up); screen lines beyond the page stay blank |
| SU SD | `CSI Pn S/T` | ✅ | **Pan the user window** through page memory (RM420); no effect when the page fits the screen. xterm compatibility scrolls instead. 🔎 hardware |
| DECVCCM DECPCCM | `CSI ? 61/64 h/l` | ✅ | Both set by default (RM420): the window follows the cursor and the display follows the cursor's page |
| DECRQDE | `CSI " v` | ✅ | `CSI lines;cols;1;top;page " w` |
| DECDMAC DECINVM | `DCS Pid;Pdt;Pen ! z`, `CSI Pid * z` | ✅ | 64 macros in 6 KB, text or hex with `!Pn;…;` repeats; invoked as host input (nesting capped at 16); RIS clears, DECSTR keeps |
| DECMSR / memory checksum | `CSI ? 62 n`, `CSI ? 63 ; Pid n` | ✅ | Free bytes ÷ 16; checksum of macro memory |
| Indicator status line | | 🟡 | Reverse video with printer state, Hold Screen, keyboard lock, page and cursor position. 🔎 field layout against hardware |
| xterm compatibility extras | `CSI 18 t`, `CSI s`, `CSI u` | ✅ | Text area size report and SCOSC/SCORC, only with `xterm_compat` |

## VT510 / VT520 / VT525 (M4)

**RM520** VT520/VT525 Video Terminal Programmer Information (EK-VT520-RM).

| Function | Seq | Status | Behaviour notes / source |
|----------|-----|--------|--------------------------|
| CHA HPA VPA | `CSI Pn G`, `` CSI Pn ` ``, `CSI Pn d` | ✅ | Honour origin mode (relative to the margins, clamped to them) but otherwise ignore margins (vttest, from a VT510; RM520 is silent) |
| HPR VPR | `CSI Pn a`, `CSI Pn e` | ✅ | Relative moves stopping at the page edge |
| CNL CPL | `CSI Pn E/F` | ✅ | CUD/CUU (stopping at the margins from inside) then CR |
| CHT CBT | `CSI Pn I/Z` | ✅ | CHT stops at the right margin from inside; CBT stops at the left margin only in origin mode (vttest) |
| DECST8C | `CSI ? 5 W` | ✅ | Clears all stops, then every 8 columns from column 9 |
| DECSCUSR | `CSI Ps SP q` | ✅ | Blinking/steady block or underline; xterm's bar styles (5, 6) are ignored |
| DECNCSM | `CSI ? 95 h/l` | ✅ | DECCOLM keeps page memory; margins still reset and the cursor homes |
| VT500 private modes | `CSI ? 34…117 h/l` | 🟡 | All modes of RM520 table 5-3 are stored and reported with factory defaults; DECNCSM, DECECM, DECBBSM, DECATCUM/BM and DECKPM (reset by DECSTR) have effect so far |
| Set-Up selections | DECSKCV DECSWBV DECSMBV DECSSCLS DECSLCK DECARR DECCRTST DECSEST DECSZS DECSPRTT DECSPPCS DECSDPT DECSDDT DECSSL DECSCP DECSCS DECSFC DECSPP DECSTRL DECSRFR | 🟡 | Validated, stored and reported with DECRQSS (factory values from RM520). Bells, click, scroll speed and zero style are not rendered yet (M7). DECSRFR is VT510 only |
| DECTID | `CSI Ps , q` | ✅ | Selects the DA1 identity (VT100 … VT520) |
| DECTME | `CSI Ps SP ~` | 🟡 | VT500, VT100 and VT52 operation with a soft reset; Wyse/TVI/ADDS/SCO emulations are not provided |
| DECSR / DECSRC | `CSI Pr + p`, `CSI Pr * q` | ✅ | Reset to power-up without disconnecting; confirmation when Pr is given |
| DECLANS DECLBAN DECLTOD | `DCS 1 v`, `DCS Ps r`, `CSI Ph;Pm , p` | ✅ | Answerback from hex pairs (30 bytes), banner and time of day stored |
| DECSWT DECSIN | `OSC 21 ; name ST`, `OSC 2L ; name ST` | ✅ | Session name becomes the window title (30 characters); icon name stored |
| DA1 | `CSI c` | ✅ | VT510 `?64;1;2;7;8;9;12;15;18;21;23;24;42`, VT520 `?65;1;2;7;9;12;18;21;23;24;42`, VT525 adds 22. 19 sessions, 44 PCTerm, 45 soft key mapping and 46 ASCII emulation are added when implemented |
| Page and screen sizes | DECSLPP DECSNLS | ✅ | VT500 page lengths 24–72 (next higher value; 3/2/1 pages), screens of 26, 42 or 53 lines, one fewer with a status line. 🔎 RM520 also mentions 6 pages of 24 lines |
| Colour (VT525) | SGR 30–37 39 40–47 49 | ✅ | Level 5 VT525 only. Bold adds 8 to the foreground index (and background with DECBBSM); blink alternates with a dimmer shade |
| DECAC DECATC DECSTGLT | `CSI Ps1;Ps2;Ps3 , \|`, `, }`, `CSI Ps ) {` | ✅ | Normal text/window frame colours, alternate text colours per rendition combination, colour mode. The mode in effect when a character is written fixes its colours (vttest DECATC test). Reported with DECRQSS (`Ps1,\|`, `Ps1,}`, `){`). 🔎 factory map and alternate colours are not tabulated in RM520 |
| DECCTR / DECRSTS 2 | `CSI 2 ; Pu $ u`, `DCS 2 $ p` | ✅ | 16-entry map in RGB or DEC HLS (blue at 0°) |
| DECECM | `CSI ? 117 h/l` | ✅ | Erase to text background (factory) or screen background |
| VT500 character sets | SCS `"?` `"4` `%0` `&4`, 96-sets `B` `F` `H` `L` `M`, NRCS `">` `%=` `%2` `%3` `&5` | 🟡 | DEC Greek, Hebrew, Turkish, Cyrillic; ISO Latin-2, Greek, Hebrew, Latin-Cyrillic, Latin-5; Greek, Hebrew, Turkish, Serbo-Croatian and Russian NRCS (NRC mode only). Tables from xterm's transcription of the RM520 figures (see THIRD-PARTY.md). DECAUPSS accepts them. Glyphs for Greek, Hebrew and Cyrillic arrive with the fonts (M7) |
| DECKBD | `CSI Ps1;Ps2 SP }` | ✅ | Layout and language reported by DSR ?26 (type 4 LK411, 5 PC) |
| DECELF DECLFKC DECSMKR | `+q`, `*}`, `+r` | 🟡 | DECLFKC F1–F4 local, sent to the host (🔎 `CSI 11~`–`14~`) or disabled; DECELF group 1 disables copy/paste keys; DECSMKR stored (key position mode is M6) |
| Dual sessions | F4, `--sessions 2` | 🟡 | Two sessions per window, each with its own connection and terminal state, like sessions on separate comm lines (RM420 chapter 14, RM520 2.5). The window splits horizontally with a title bar per session (session name from DECSWT); F4 moves the keyboard. Each session is scaled to its half of the window rather than showing fewer lines. TD/SMP multiplexing over one connection is not provided |
| DECES DECUS DECSPMA | `CSI & x`, `CSI Ps , y`, `CSI Pn;… , x` | 🟡 | DECES makes the session active (keyboard focus, window raised); DSR ?85 reports sessions on separate lines when two are open. DECUS stored and reported (inactive sessions always update). DECSPMA reported; 🔎 each session keeps its own full page memory |
| DECPS | `CSI Pv;Pd;Pn , ~` | 🟡 | Parsed into an event; sound output arrives with bell and keyclick (M7) |
| Key programming, PCTerm | DECPFK DECPAK DECPKA DECRQKD DECRPFK DECRPAK DECPKFMR DECRQPKFM DECEKBD DECRQKT DECPCTERM | ⬜ | M6 (keyboard) |

## Keyboard (vt-core `Key`)

| Keys | Status | Notes |
|------|--------|-------|
| Cursor keys (ANSI / application / VT52) | ✅ | 7-bit or 8-bit SS3/CSI per S8C1T |
| Numeric keypad, PF1–PF4 (numeric / application / VT52) | ✅ | |
| Return (LNM), `<X]` (DECBKM) | ✅ | DEC default `<X]` sends DEL |
| Editing keypad `CSI 1~`–`6~` | ✅ | VT220+ only |
| F6–F20, Help, Do | ✅ | VT100 mode: F11 ESC, F12 BS, F13 LF |
| Typed text | ✅ | DEC Multinational or ISO Latin-1 (per UPSS) in 8-bit modes; national set in NRC mode |
| UDKs | ✅ | `Key::UserDefined(6..=20)`; PC: Ctrl+F6–F12, Ctrl+Shift+F1–F10 |

## vttest coverage (VT102 model)

| Menu | Status |
|------|--------|
| 1 Cursor movements | ✅ |
| 2 Screen features | ✅ |
| 3 Character sets | ✅ |
| 4 Double-sized characters | ✅ |
| 5 Keyboard | ✅ except 5.2 auto-repeat (GUI behaviour) |
| 6 Terminal reports | ✅ |
| 7 VT52 mode | ✅ |
| 8 VT102 insert/delete | ✅ |
| 9–10 | ⬜ |

## vttest coverage (VT420 model)

| Menu | Status |
|------|--------|
| 3 Character sets (VT100 sets, SI/SO, locking and single shifts) | ✅ |
| 11.1 VT220: DSR, SRM, DECTCEM, ECH, DECSCA, S8C1T, DECSTR, DECUDK | ✅ (printer post-1.0; DECDLD test needs a font file) |
| 11.2 VT320: SU/SD, DECXCPR, DECCIR, DECTABSR, DECRPM, DECRSPS, DECRQSS, DECRQUPSS, status line | ✅ (SU/SD pan, so the VT320 scroll pictures stay put) |
| 11.3.2 VT420 cursor movement: DECBI, DECFI, movement within margins, with and without DECLRMM/DECOM | ✅ |
| 11.3.3 VT420 editing: DECIC/DECDC, IND/RI, IL/DL, ICH/DCH, BS/CR/TAB within margins | ✅ |
| 11.3.4 keyboard control, 11.3.5 macros | ⬜ keyboard in M6; vttest has no macro test |
| 11.3.6 rectangles: DECCARA, DECCRA, DECERA, DECFRA, DECRARA, DECSERA, with and without DECOM | ✅ |
| 11.3.7 reports: DECRPM, DECRQSS (DECSACE, DECSLRM, DECSNLS), DECMSR, memory checksum, DECRQCRA GL/GR, DECXCPR | ✅ |
| 11.3.8 DECSNLS | ✅ |
| 11.4 VT520 | ⬜ M4 |

## vttest coverage (VT520/VT525 models)

| Menu | Status |
|------|--------|
| 11.4.2 VT520 cursor movement: HPA, CBT, CHA, CHT, HPR, VPA, CNL, CPL, VPR, with and without margins and origin mode | ✅ |
| 11.4.5.2 DECRPM (VT500 modes) and DECRQSS for the VT510 and VT520 selections | ✅ (DECSRFR is VT510-only, so a VT520 rejects it) |
| 11.4.6 DECNCSM, DECSCUSR, DECATC (VT525) | ✅ |
| 11.4.3 editing, 11.4.4 keyboard | vttest has no tests |

## esctest2 (VT525 model, xterm compatibility)

`cargo xtask esctest` runs the pinned esctest2 at `--max-vt-level=5` against a VT525: 368 tests pass. The
189 failures are listed in `tests/conformance/esctest/expected-failures.txt`, each with its DEC reason — mostly
xterm-only features (window operations, colour setting, alternate screen, reverse wrap), xterm's habit of
checksumming some erased cells as spaces, and VT420/VT500 modes esctest expects xterm to lack. Any other
failure, or a listed test that starts passing, fails the task.
