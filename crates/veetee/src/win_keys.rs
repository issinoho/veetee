//! The extended-key flag of Windows key messages, which GDK does not pass on:
//! without it the numeric keypad with NumLock off, and keypad Enter, look
//! like the cursor keys and Return (see `vt_keyboard::windows_keypad_keysym`).
//! A keyboard hook on the GTK thread records the flag of each virtual key as
//! its message is taken from the queue, before GDK makes it a key event.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{CallNextHookEx, SetWindowsHookExW, WH_KEYBOARD};

/// Whether the last message for each virtual key had the extended flag.
static EXTENDED: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
/// Whether the hook is in place; without it no flag is known.
static INSTALLED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0
        && let Some(flag) = EXTENDED.get(wparam)
    {
        flag.store(lparam & (1 << 24) != 0, Ordering::Relaxed);
    }
    // SAFETY: passes the hook's own arguments on, as the hook chain requires.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Installs the hook for the calling thread, which must be the GTK thread.
/// Without it (if installing fails) the keypad behaves as before.
pub fn install() {
    // SAFETY: a thread hook needs no module handle; `hook` lives for the
    // whole program and the hook is never removed.
    let hhook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD,
            Some(hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    INSTALLED.store(!hhook.is_null(), Ordering::Relaxed);
}

/// The keysym for a key press: GDK's, unless the virtual key `keycode` is a
/// numeric keypad key GDK took for another.
pub fn keysym(sym: u32, keycode: u32) -> u32 {
    if !INSTALLED.load(Ordering::Relaxed) {
        return sym;
    }
    let extended = EXTENDED
        .get(keycode as usize)
        .is_some_and(|f| f.load(Ordering::Relaxed));
    vt_keyboard::windows_keypad_keysym(keycode, extended).unwrap_or(sym)
}
