# Four sessions

A plan, written 27 September 2026, for the VT520's four sessions. From the VT520/VT525 Video
Terminal Programmer Information (EK-VT520-RM, sections 2.5 and 2.6) and the VT520 Installation
and Operating Information (EK-VT520-IN, chapter 2).

## What a VT520 does

- **Up to four sessions**, S1 to S4: "Multiple sessions extend the VT520 to act like four
  terminals in one" (EK-VT520-IN 2.1). Each is a virtual terminal "that maintains the full
  keyboard and display state of a real physical terminal but shares a single keyboard and display
  with other virtual terminals".
- **Each session is assigned to a comm port.** The VT520 has three, so three sessions can each
  have a line of their own; a fourth, or two on one port, needs **TD/SMP** (Terminal Device/Session
  Management Protocol), which multiplexes sessions over one line. The other end must speak it: a
  terminal server (`SET PORT MULTI ENABLE`), or a host running the Session Support Utility
  (`$ MCR SSU` on OpenVMS). Assigning two sessions to one port turns TD/SMP on (RM 2.5.6); a session
  with "Comm = session off" is disabled and dimmed (RM 2.6.1).
- **At most two sessions are on the screen at once**: "You can display data from two sessions at
  the same time by dividing the screen into two windows" (IN 2.3); Ctrl+Session changes the window
  configuration, and with Auto resize, two windows set 48 lines per screen (RM 2.8.1.1).
- **Switching**: the Session key (F4) cycles through the enabled sessions; Caps Lock with keypad
  1–4 goes to one directly (RM 2.5.4). The current session shows on the Set-Up summary line
  (`S1=comm1`), and at the lower left of the screen.
- **Framed windows** (Display Set-Up): a title bar per window with the session name (30
  characters), and an icon per session whose name is the first 12 characters; an icon blinks when
  its session has new data the user has not seen, where updating in the background is allowed
  (DECUS, RM 2.6.7).
- **The Session menu** (RM 2.6): Select session, Session name, Pages per session (8 pages of 25
  lines shared between the sessions), Soft char sets per session, Save and Restore settings for
  all, Copy settings from, Update session. Most settings are kept per session and saved per
  session.

## Where veetee stands

| Part | State |
|---|---|
| Sessions in a window | Two (`--sessions 2`, *New Session* in the window menu), each with its own connection and terminal state |
| Session 2's connection | The same as session 1's: a second login to the same host |
| Layout | The window split horizontally, both sessions shown, a title bar each (DECSWT names it) |
| Switching | F4 moves the keyboard to the other session; DECES from the host does too |
| Close Session | In the window menu; the other session takes the whole window |
| Set-Up | Saved per model and session (`setup-vt520-session2.conf`) |
| Session menu (VT500) | Session 3 and 4 dimmed; Pages per session and the rest stored or dimmed |
| DECUS, DECSPMA, DSR ?85 | Stored and reported; inactive sessions always update |
| TD/SMP, SSU | Not provided; Enable/Disable Sessions say "Sessions not selected" |
| Models | Two sessions on every model, the VT510 included, which had one |

## The plan

### FS1. Four sessions, each with its own connection

What a VT520 does with a line per session, which is how veetee already treats two. The window
holds up to four sessions on the VT520 and VT525; *New Session* and F4 as now, and a direct way to
a session (decision 4). Session 3 and 4 get their own saved Set-Up, as session 2 does. The Session
menu's Select session offers S1 to S4 for the sessions open.

### FS2. Two windows, as the terminal has

One window or two, whatever the number of sessions (decision 2). With two windows, the active
session is in one and the session last active before it in the other; switching to a session not
on the screen replaces the one not active. Ctrl+F4 toggles one window and two, as Ctrl+Session
does. A session not on the screen goes on receiving from its host.

### FS3. Framed windows and session icons

The Display menu's *Framed windows*: a title bar per window with the session name, and a row of
session icons, one per session, which blinks when its session has had new data unseen, where
DECUS allows it. veetee's title bars already show the name; the icons are new.

### FS4. Where each session connects

A new session asks where to connect (decision 3), from the saved connections or a new one, rather
than logging in again to session 1's host. `--sessions N` still opens N to the window's connection.

### FS5. TD/SMP, only if a host speaks it

Several sessions over one Telnet, SSH, LAT or serial connection, with SSU on the host. It needs
the protocol, which DEC documented but veetee has no copy of (🔎), and a host with SSU to prove it
against; whether OpenVMS V8.4 still ships SSU is not known (decision 1). Not started until both are
in hand.

### FS6. Acceptance

Four sessions to MYI64 over different connections — Telnet, SSH, LAT and the serial line —
switching with F4 and directly, one window and two, a session in the background receiving output
(`MONITOR SYSTEM`) and its icon blinking, and each session's Set-Up saved and restored on its own.
Run by the user.

## Decisions

Settled on 27 September 2026, each as recommended below.

1. **TD/SMP.** *Recommended*: leave it (FS5) until a host with SSU is to hand. Four sessions each
   with its own connection gives everything else, and is what a VT520 with a line per session did.
   Worth checking on MYI64: `$ DIRECTORY SYS$SYSTEM:SSU*.*` and `$ MCR SSU`.
2. **How many on the screen.** The VT520 shows one window or two. *Recommended*: the same — the
   DEC way, and two sessions of 24 lines is what the display was built for. The alternative, four
   tiles at once, is a thing a PC window could do and no VT520 did; it could be an option later.
3. **Where a new session connects.** *Recommended*: ask, with the Connections window, defaulting
   to the window's own connection; a VT520's sessions were on different ports as often as not, and
   a serial session cannot open its port twice.
4. **Going to a session directly.** The VT520 uses Caps Lock with keypad 1–4, which a PC's Caps
   Lock, a toggle, does badly. *Recommended*: Alt+1 to Alt+4, as new local functions in the keymap
   so it can be changed; F4 cycles as ever, and Ctrl+F4 is Ctrl+Session, the window toggle
   (FS2). Ctrl+F2 and Ctrl+F5 are already auto print and answerback.
5. **Which models.** EK-VT520-RM gives the VT520 and VT525 four; the VT420 has two (RM420
   chapter 14); the VT510 had one. *Recommended*: four on the VT520 and VT525, two on the VT420
   and VT510 as now — taking the VT510 down to one would take away something that works, for no
   one's gain — and one below the VT420.
6. **Page memory.** A VT520 shares 8 pages between its sessions (Pages per session). veetee gives
   each session its own page memory. *Recommended*: keep that, and show the Pages per session
   dialog with each session's pages; sharing 8 would only take pages away.
