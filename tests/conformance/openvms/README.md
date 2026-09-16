# OpenVMS acceptance recordings

Each `NAME.vtrec` here is a real session with an OpenVMS host, recorded by
veetee. `cargo xtask openvms` replays every recording through the emulator
and compares the screen at each checkpoint with `NAME/CHECKPOINT.screen`
(and the last screen with `NAME/final.screen`). CI runs it on every push.

## Making a recording

1. Connect with recording on, choosing the model you want to test:

   ```sh
   veetee --model vt420 --record edt-keypad.vtrec --telnet vms1
   ```

2. Log in, then work through the application. Whenever the screen shows
   something worth checking, press **Ctrl+Shift+M** (or choose *Mark
   Checkpoint* in the window menu). Checkpoints are numbered
   `checkpoint-01`, `checkpoint-02`, …
3. Log out and close the window.
4. Look at what the emulator made of it:

   ```sh
   cargo run -p vt-headless -- replay edt-keypad.vtrec --bless
   ```

   This writes `edt-keypad/checkpoint-01.screen` and so on. Read each screen
   and compare it with what the session looked like; only commit screens that
   are right.

Typed keys are not recorded unless you add `--record-keys`, so passwords do
not end up in the file. The host's output is recorded, though: review a
recording (for node names, user names, file contents) before committing it.

## Checklist

| Recording | Application | What to exercise |
|-----------|-------------|------------------|
| `set-terminal-inquire` | DCL | `SET TERMINAL/INQUIRE`, `SHOW TERMINAL`, DCL line editing (arrows, Ctrl/B, Ctrl/E, insert/overstrike) |
| `set-terminal-inquire-ssh` | DCL over SSH | The same over SSH, where OpenVMS never sends the
  inquiry and the terminal must answer nothing |
| `edt-keypad` | EDT | Keypad mode, PF1 Gold functions, Help (PF2), Find, Cut/Paste, scrolling regions |
| `eve-tpu` | EVE/TPU | Two windows, Do commands, Find/Select/Remove/Insert Here, 132 columns (`SET WIDTH 132`) |
| `mail` | MAIL | Directory listing, reading and paging a long message, cursor addressing |
| `fms-forms` | FMS | Form display, protected fields, field navigation, video attributes |
| `decforms` | DECforms | Panels, status line messages, function keys, list boxes |
| `monitor` | MONITOR | `MONITOR SYSTEM` and `MONITOR PROCESSES/TOPCPU` updating in place |
| `smg-demo` | SMG$ | Any SMG-based utility: windows, line drawing, pasteboard updates |
