# An Android port

A plan, not work in progress. Written 2026-09-18 against veetee 1.1.2; check the crate layout
before acting on it.

## What carries over

- **`vt-parser`, `vt-core`, `vt-fonts`** have no GTK or OS dependencies and should cross-compile to
  `aarch64-linux-android` unchanged: emulation, character sets, recording and fonts.
- **`vt-render`** already has an OpenGL ES 3.0 path (`crates/vt-render/src/gl.rs` emits
  `#version 300 es`), which Android supports. The glyph atlas, scene and post-processing should
  run on an Android GL surface through `glow`.
- **`vt-keyboard`**: the keymap logic is reused; only the mapping from Android key codes is new.
- **`vt-lat`** is pure message code and compiles, but see LAT below.
- **Telnet** is plain sockets and works as it is.

## What has to be new

### Front end

The `veetee` crate (about 7,000 lines of GTK 4 and libadwaita) does not port. Two options:

- **Kotlin app with the Rust core over JNI (preferred).** A `GLSurfaceView` terminal view calling
  `vt-render`; native Android screens for connections, profiles and Set-Up. The Rust side is a small
  `vt-android` crate exposing a session API (feed bytes, send key, resize, draw). This handles
  background running, the input method and storage best.
- **All Rust** (`android-activity` + `winit`, egui or similar for menus). One language, but
  soft-keyboard and input-method support is weak and the menus would never feel native.

### Transports

| Transport | Android |
|---|---|
| Telnet | Works. |
| SSH | No system `ssh`. Embed a Rust client such as `russh` (Apache-2.0, passes `cargo deny`). Loses `~/.ssh/config`, the agent and ProxyJump, so it needs its own key storage (Android Keystore), known-hosts handling and probably a ProxyJump option. The largest new piece, and a departure from the "system OpenSSH" project decision that should be recorded as a deliberate platform exception. |
| Serial | No `/dev/ttyUSB*` without root. Use the USB Host API, e.g. usb-serial-for-android (MIT) on the Java side, passing bytes to Rust. Worth doing for people with real DEC hardware. |
| Local PTY | Possible (Termux does it) but of little use; drop it. |
| LAT | Needs `AF_PACKET` and `CAP_NET_RAW`, so root only. Drop it, or offer it unsupported on rooted devices. |

### Keyboard: the main UX problem

EDT and most of OpenVMS rely on the LK401 editing keypad and numeric keypad (PF1–PF4,
Find/Select/Do, F6–F20). A phone soft keyboard has none of them. Needed:

- an extra-keys bar (Ctrl, Esc, arrows, PF1–PF4, Compose);
- a switchable on-screen EDT keypad overlay, laid out like the LK401 keypad, possibly driven from
  `vt-keyboard` keymaps;
- `InputConnection` handling, with composed characters mapped into DEC Supplemental;
- a mapping from Android key codes for Bluetooth and USB keyboards, which make it a genuinely
  usable terminal, especially on tablets.

LK401 semantics stay the default (the backarrow key sends DEL), as on the desktop.

### Screen and lifecycle

- 80×24 fits a phone in landscape. 132 columns needs pinch-zoom and panning, and works properly
  only on a tablet. Keep the pixel fonts at integer or near-integer scales so they stay crisp.
- Android suspends or kills background apps, so live sessions need a foreground service with a
  notification (as ConnectBot and Termux do).
- Profiles and Set-Up (TOML) live in app storage, with import and export.
- The bell: `cpal` supports Android (AAudio/Oboe).

## Build and distribution

- `cargo-ndk` + NDK + Gradle; targets `aarch64-linux-android` (devices) and `x86_64-linux-android`
  (the emulator). The Release workflow needs an APK/AAB job and an Android signing key.
- New dependencies (`jni`, `russh`, …) must pass `cargo deny`; regenerate
  `packaging/flatpak/cargo-sources.json` only if desktop dependencies change.
- Distribution:
  - Google Play needs a developer account and a privacy-policy URL, which runs into the same
    problem as winget and Flathub: issinoho.com does not serve.
  - Google's developer verification for sideloaded apps was being rolled out in some countries
    around the time of writing; check where it stands.
  - F-Droid builds from source and fits the licensing well; probably the easiest first channel.

## Phases

1. **Spike**: Kotlin shell app plus JNI to `vt-core` and `vt-render`, Telnet only, hardware
   keyboard. Proves the renderer and the build chain; check first that the core crates
   cross-compile cleanly for `aarch64-linux-android`.
2. Extra-keys bar, EDT keypad overlay and the foreground service.
3. Embedded SSH with key management.
4. USB serial.
5. Distribution, F-Droid first.
