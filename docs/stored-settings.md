# The Set-Up settings that are stored only

A plan, written 27 September 2026, for the settings veetee keeps, saves and reports but does not
act on (compat-matrix.md, *Stored only*). From the VT520/VT525 Video Terminal Programmer
Information (EK-VT520-RM, chapter 5), each setting's own page.

## The settings

| Setting | Control | What EK-VT520-RM says |
|---|---|---|
| Zero style | DECSZS `CSI Ps , {` | 1 oval zero (default), 2 zero with a slash, 3 zero with a dot |
| Transmit rate limit | DECXRLM `CSI ? 73 h`, DECSTRL `CSI Ps1 ; Ps2 " u` | "The terminal limits the rate at which it transmits the answerback, CPR, DA, TSI, and DSR messages, as well as the keyboard keys, and any other characters": 150, 50 or 30 characters a second, for all keys or separately for graphic and function keys |
| Modem control | DECMCM `CSI ? 99 h`, DECSDDT `CSI Ps $ q` | "No data is transmitted or received unless DSR is asserted … Loss of DSR causes a disconnect"; losing carrier (RLSD) disconnects after 2 seconds, 60 ms, or not at all |
| Host wake-up | DECHWUM `CSI ? 113 h` | "Any character received from the host will also restore the display" from the CRT or energy saver — only when set |
| Overscan | DECOSCNM `CSI ? 106 h` | The monochrome VT520 only: overscan on or off (default off) |
| Energy saver | DECSEST `CSI Ps - r` | "Switches the monitor to suspend mode after the specified interval of CRT saver active" |
| Keyboard options | Set-Up only | The Compose, Alt and F5 keys and the `,<` `.>` and `<>` keys of the VT520's PC keyboard |

## The plan

### Z1. Host wake-up

Host output wakes the screen from the CRT saver only with DECHWUM set, as the Set-Up factory
default has it; cleared, only a key does. Today host output always wakes it.

**Done** (27 September 2026). DECHWUM's factory setting was wrong in veetee — cleared, where the
factory-defaults table (EK-VT520-RM 2.18, table 2-10) has *Host wake-up* ticked — and is now set,
so nothing changes until it is turned off; the VT520 mode-report snapshot in vttest changes with
it. Earlier models, with no such setting, wake on host output.

### Z2. Transmit rate limit

With DECXRLM set, everything the session sends — keys, pastes, reports, answerback — goes at the
DECSTRL rate, the function-key rate for what a function key sends. The paced sending lives with
the XOFF queue in the session, so the two agree. Kermit's packets are left at full speed: a
transfer is not typing (decision 2).

**Done** (27 September 2026). A key that sends more than one byte — a function, editing, cursor
or keypad key — goes at the function-key rate where one is set; typing, pastes and the
terminal's replies at the other. While anything is still going out paced, what follows queues
behind it, so nothing overtakes; XON and XOFF still stop and start it.

### Z3. Zero style

DECSZS 2 and 3 draw the zero slashed or dotted, in every font veetee has: 80 and 132 columns,
each model's cell, double width and height. The slash and dot are added to the existing zero by
rule rather than drawn anew per font, so every size gets them.

### Z4. Modem control, on a serial line

With DECMCM set on a serial line: the port's DSR and carrier are read as data flows; while DSR is
off nothing is sent and what arrives is dropped; DSR going off ends the session, and carrier
going off ends it after the DECSDDT delay (2 seconds, 60 ms, or never). Closing the session drops
DTR, as the terminal's disconnect does. Other connections have no modem and are unaffected.
Tested on the PL2303 adapter, whose cable decides what DSR and carrier do (decision 3).

### Z5. Overscan

On the monochrome VT520, the screen's background colour fills the window to its edges instead
of stopping at the picture's border.

### Z6. Energy saver

After the CRT saver has run the DECSEST time, veetee stops drawing the screen altogether until
woken, which is what suspend would save on a laptop; nothing changes to the eye (decision 4).

## Decisions

Settled on 27 September 2026, each as recommended below.

1. **Keyboard options.** *Recommended*: leave them stored. They describe the VT520's own PC
   keyboard; veetee's keymap and its editor already decide what every PC key does, and two
   places deciding would contradict each other.
2. **Rate limit and Kermit.** *Recommended*: Kermit packets go at full speed. DEC's limit is for
   what the terminal transmits of itself and the user's typing; a transfer paced to 150 characters
   a second would take minutes per megabyte for no reason a host gives.
3. **Modem control's test.** The adapter's cable may not carry DSR or carrier at all, in which case
   the test is that nothing gets through with modem control on, and all of it with it off.
   *Recommended*: that, and a note of which lines the cable carries.
4. **Energy saver.** *Recommended*: stop drawing, as in Z6 — the one effect a window can have.
   The alternative is leaving it stored.
