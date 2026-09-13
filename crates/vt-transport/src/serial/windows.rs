//! Serial ports through the Win32 communications API. The port is opened
//! for overlapped I/O so a read waiting for the host never holds up keys
//! being sent from the other thread.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::ptr::null;
use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::Devices::Communication::{
    COMMTIMEOUTS, ClearCommBreak, DCB, EVENPARITY, GetCommState, MARKPARITY, NOPARITY, ODDPARITY,
    ONESTOPBIT, SPACEPARITY, SetCommBreak, SetCommState, SetCommTimeouts, TWOSTOPBITS,
};
use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, ReadFile, WriteFile};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

use super::{FlowControl, Parity, SerialConfig, invalid};

/// An open serial port.
#[derive(Debug)]
pub struct Serial {
    port: Arc<File>,
    config: SerialConfig,
    event: OwnedHandle,
    /// The read timeout currently set on the port, in milliseconds.
    timeout_ms: Option<u32>,
}

impl Serial {
    /// Opens and configures the port (`COM3`, or a `\\.\` device path) for
    /// exclusive use.
    pub fn open(config: SerialConfig) -> io::Result<Serial> {
        if !(5..=8).contains(&config.data_bits) {
            return Err(invalid("data bits must be 5 to 8"));
        }
        if !(1..=2).contains(&config.stop_bits) {
            return Err(invalid("stop bits must be 1 or 2"));
        }
        let port = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .open(device_path(&config.device))
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", config.device.display())))?;
        let handle = port.as_raw_handle() as HANDLE;

        let mut dcb = DCB {
            DCBlength: size_of::<DCB>() as u32,
            ..DCB::default()
        };
        // SAFETY: `handle` is an open port and `dcb` is correctly sized.
        if unsafe { GetCommState(handle, &mut dcb) } == 0 {
            return Err(io::Error::last_os_error());
        }
        apply_line_settings(&config, &mut dcb);
        // SAFETY: as above.
        if unsafe { SetCommState(handle, &dcb) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{}: {}",
                    config.device.display(),
                    io::Error::last_os_error()
                ),
            ));
        }
        Ok(Serial {
            port: Arc::new(port),
            config,
            event: event()?,
            timeout_ms: None,
        })
    }

    pub fn config(&self) -> &SerialConfig {
        &self.config
    }

    fn set_timeout(&mut self, ms: u32) -> io::Result<()> {
        if self.timeout_ms == Some(ms) {
            return Ok(());
        }
        // Return as soon as any byte arrives, or after `ms` with none.
        let timeouts = COMMTIMEOUTS {
            ReadIntervalTimeout: u32::MAX,
            ReadTotalTimeoutMultiplier: u32::MAX,
            ReadTotalTimeoutConstant: ms,
            WriteTotalTimeoutMultiplier: 0,
            WriteTotalTimeoutConstant: 0,
        };
        // SAFETY: the port is open.
        if unsafe { SetCommTimeouts(self.port.as_raw_handle() as HANDLE, &timeouts) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.timeout_ms = Some(ms);
        Ok(())
    }
}

/// `COM10` and above need the device namespace prefix; it works for all.
fn device_path(device: &std::path::Path) -> PathBuf {
    let text = device.to_string_lossy();
    if text.starts_with(r"\\") {
        device.to_path_buf()
    } else {
        PathBuf::from(format!(r"\\.\{text}"))
    }
}

/// Sets binary mode and the line parameters in `dcb`.
fn apply_line_settings(config: &SerialConfig, dcb: &mut DCB) {
    // DCB bit fields, least significant first.
    const BINARY: u32 = 1 << 0;
    const PARITY_CHECK: u32 = 1 << 1;
    const OUTX_CTS_FLOW: u32 = 1 << 2;
    const OUTX_DSR_FLOW: u32 = 1 << 3;
    const DTR_CONTROL_ENABLE: u32 = 1 << 4;
    const DTR_CONTROL_MASK: u32 = 0b11 << 4;
    const DSR_SENSITIVITY: u32 = 1 << 6;
    const TX_CONTINUE_ON_XOFF: u32 = 1 << 7;
    const OUT_X: u32 = 1 << 8;
    const IN_X: u32 = 1 << 9;
    const ERROR_CHAR: u32 = 1 << 10;
    const NULL_STRIP: u32 = 1 << 11;
    const RTS_CONTROL_ENABLE: u32 = 1 << 12;
    const RTS_CONTROL_HANDSHAKE: u32 = 2 << 12;
    const RTS_CONTROL_MASK: u32 = 0b11 << 12;
    const ABORT_ON_ERROR: u32 = 1 << 14;

    dcb.BaudRate = config.baud;
    dcb.ByteSize = config.data_bits;
    dcb.Parity = match config.parity {
        Parity::None => NOPARITY,
        Parity::Even => EVENPARITY,
        Parity::Odd => ODDPARITY,
        Parity::Mark => MARKPARITY,
        Parity::Space => SPACEPARITY,
    };
    dcb.StopBits = if config.stop_bits == 2 {
        TWOSTOPBITS
    } else {
        ONESTOPBIT
    };
    let mut bits = dcb._bitfield;
    bits &= !(PARITY_CHECK
        | OUTX_CTS_FLOW
        | OUTX_DSR_FLOW
        | DTR_CONTROL_MASK
        | DSR_SENSITIVITY
        | OUT_X
        | IN_X
        | ERROR_CHAR
        | NULL_STRIP
        | RTS_CONTROL_MASK
        | ABORT_ON_ERROR);
    bits |= BINARY | DTR_CONTROL_ENABLE | TX_CONTINUE_ON_XOFF;
    if config.parity != Parity::None {
        bits |= PARITY_CHECK;
    }
    match config.flow {
        FlowControl::None => bits |= RTS_CONTROL_ENABLE,
        FlowControl::XonXoff => bits |= RTS_CONTROL_ENABLE | OUT_X | IN_X,
        FlowControl::RtsCts => bits |= OUTX_CTS_FLOW | RTS_CONTROL_HANDSHAKE,
    }
    dcb._bitfield = bits;
    dcb.XonChar = 0x11;
    dcb.XoffChar = 0x13;
    dcb.XonLim = 256;
    dcb.XoffLim = 256;
}

fn event() -> io::Result<OwnedHandle> {
    // SAFETY: an unnamed manual-reset event with default security.
    let handle = unsafe { CreateEventW(null(), 1, 0, null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateEventW returned a new handle we own.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as _) })
}

/// Runs one overlapped read or write, waiting up to `wait_ms` for it.
fn overlapped(
    port: &File,
    event: &OwnedHandle,
    wait_ms: u32,
    start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32,
) -> io::Result<usize> {
    let handle = port.as_raw_handle() as HANDLE;
    let mut ov = OVERLAPPED {
        hEvent: event.as_raw_handle() as HANDLE,
        ..OVERLAPPED::default()
    };
    let mut done = 0u32;
    if start(handle, &mut ov) == 0 {
        // SAFETY: plain error query.
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        // SAFETY: `ov` stays alive until the operation completes or is cancelled.
        if unsafe { WaitForSingleObject(ov.hEvent, wait_ms) } != WAIT_OBJECT_0 {
            // SAFETY: cancels only this operation.
            unsafe { CancelIoEx(handle, &ov) };
        }
    }
    // SAFETY: waits for the (possibly cancelled) operation to finish.
    if unsafe { GetOverlappedResult(handle, &ov, &mut done, 1) } == 0 {
        // SAFETY: plain error query.
        let error = unsafe { GetLastError() };
        if error != ERROR_OPERATION_ABORTED {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
    }
    Ok(done as usize)
}

impl crate::Transport for Serial {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let ms = timeout.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32;
        self.set_timeout(ms)?;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        // The port times the read out itself; the wait is only a backstop.
        overlapped(&self.port, &self.event, ms.saturating_add(1000), |h, ov| {
            // SAFETY: `buf` outlives the operation, which completes before returning.
            unsafe { ReadFile(h, ptr, len, std::ptr::null_mut(), ov) }
        })
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(SerialWriter {
            port: self.port.clone(),
            event: event()?,
        }))
    }

    fn description(&self) -> String {
        self.config.to_string()
    }
}

struct SerialWriter {
    port: Arc<File>,
    event: OwnedHandle,
}

impl Write for SerialWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_ptr();
        overlapped(&self.port, &self.event, INFINITE, |h, ov| {
            // SAFETY: `buf` outlives the operation, which completes before returning.
            unsafe { WriteFile(h, ptr, len, std::ptr::null_mut(), ov) }
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl crate::TransportWriter for SerialWriter {
    fn send_break(&mut self) -> io::Result<()> {
        let handle = self.port.as_raw_handle() as HANDLE;
        // SAFETY: the port is open.
        if unsafe { SetCommBreak(handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // A DEC break is a quarter-second space condition.
        std::thread::sleep(Duration::from_millis(250));
        // SAFETY: as above.
        if unsafe { ClearCommBreak(handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_settings_fill_the_dcb() {
        let mut dcb = DCB::default();
        let config = SerialConfig {
            baud: 19200,
            data_bits: 7,
            parity: Parity::Even,
            stop_bits: 2,
            flow: FlowControl::RtsCts,
            ..SerialConfig::new("COM3")
        };
        apply_line_settings(&config, &mut dcb);
        assert_eq!((dcb.BaudRate, dcb.ByteSize), (19200, 7));
        assert_eq!((dcb.Parity, dcb.StopBits), (EVENPARITY, TWOSTOPBITS));
        assert_eq!(
            dcb._bitfield & 0b111,
            0b111,
            "binary, parity check, CTS flow"
        );
        assert_eq!(dcb._bitfield >> 12 & 0b11, 2, "RTS handshake");
        apply_line_settings(&SerialConfig::new("COM3"), &mut dcb);
        assert_ne!(dcb._bitfield & (1 << 8), 0, "XON/XOFF by default");
        assert_eq!(config.to_string(), "COM3 19200 7E2 RTS/CTS");
    }

    #[test]
    fn device_names_get_the_device_namespace() {
        assert_eq!(device_path("COM12".as_ref()), PathBuf::from(r"\\.\COM12"));
        assert_eq!(
            device_path(r"\\.\COM1".as_ref()),
            PathBuf::from(r"\\.\COM1")
        );
    }

    #[test]
    fn missing_port_is_reported_by_name() {
        let err = Serial::open(SerialConfig::new("COM250")).unwrap_err();
        assert!(err.to_string().contains("COM250"), "{err}");
    }
}
