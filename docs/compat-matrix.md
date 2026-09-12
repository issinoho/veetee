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
| ED EL | `CSI J`, `CSI K` | ✅ | Completely erased lines below/above revert to single width (UG100) |
| DECSTBM | `CSI r` | ✅ | Invalid (top ≥ bottom) ignored without homing |
| DECSC DECRC | `ESC 7 8` | ✅ | Saves position, SGR, G-sets & shifts, DECOM, last-column flag. 🔎 whether DECAWM belongs in "wrap flag" |
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
| SUB error character | | ✅ | ▒ on VT100-class, ⸮ on VT220+. 🔎 CAN display on VT100 |
| IL DL DCH | `CSI L M P` | ✅ | VT102 and later; ignored outside scroll region |
| ICH | `CSI @` | ✅ | **VT220 and later** — the VT102 does not have it (vttest, UG102) |
| VT52 mode | `ESC A–K Y Z = > < F G` | ✅ | Out-of-range `ESC Y` line **or column** leaves that coordinate unchanged (VT100 family; a real VT52 clamps) |
| VT52 printer functions | `ESC ^ _ W X ] V` | ⬜ | Post-1.0 printing |

## Keyboard (vt-core `Key`)

| Keys | Status | Notes |
|------|--------|-------|
| Cursor keys (ANSI / application / VT52) | ✅ | 7-bit or 8-bit SS3/CSI per S8C1T |
| Numeric keypad, PF1–PF4 (numeric / application / VT52) | ✅ | |
| Return (LNM), `<X]` (DECBKM) | ✅ | DEC default `<X]` sends DEL |
| Editing keypad `CSI 1~`–`6~` | ✅ | VT220+ only |
| F6–F20, Help, Do | ✅ | VT100 mode: F11 ESC, F12 BS, F13 LF |
| Typed text | ✅ | DEC Multinational in 8-bit modes; NRCS encoding ⬜ (M2) |
| UDKs | ⬜ | M2 |

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
| 9–11 | ⬜ M2+ |
