//! Binds glow to the GL context GTK created, via libepoxy (which GTK itself
//! uses to dispatch GL calls).
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::OnceLock;

static EPOXY: OnceLock<Result<libloading::Library, String>> = OnceLock::new();

#[cfg(windows)]
const EPOXY_LIBRARY: &str = "libepoxy-0.dll";
#[cfg(not(windows))]
const EPOXY_LIBRARY: &str = "libepoxy.so.0";

/// Creates a glow context for the GL context that is current on this thread.
pub fn glow_context() -> Result<glow::Context, String> {
    let lib = EPOXY
        .get_or_init(|| {
            // SAFETY: libepoxy has no initialisation side effects beyond symbol resolution.
            unsafe { libloading::Library::new(EPOXY_LIBRARY) }
                .map_err(|e| format!("cannot load libepoxy: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    // SAFETY: libepoxy exports each GL entry point as a function-pointer
    // variable named `epoxy_<name>`; we read the variable's current value.
    let ctx = unsafe {
        glow::Context::from_loader_function(|name| {
            let symbol = format!("epoxy_{name}");
            match lib.get::<*const *const c_void>(symbol.as_bytes()) {
                Ok(var) => **var,
                Err(_) => std::ptr::null(),
            }
        })
    };
    Ok(ctx)
}
