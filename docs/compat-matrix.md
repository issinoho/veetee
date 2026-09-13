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
| DECRQTSR / DECRSTS | | ⬜ | M3 |
| CHA HPA VPA CNL CPL CHT CBT | | 🟡 | Currently VT500 level only. 🔎 which of these the VT420 has |

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
| 11.2 VT320: SU/SD, DECXCPR, DECCIR, DECTABSR, DECRPM, DECRSPS, DECRQSS, DECRQUPSS, status line | ✅ (page format/movement, DECRQTSR, DECRPDE: M3) |
| 11.3–11.4 VT420/VT520 | ⬜ M3/M4 |
