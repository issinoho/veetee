//! Windows pseudo consoles (ConPTY, Windows 10 1809 and later). The child
//! runs attached to a pseudo console fed through two pipes; a thread reads
//! its output, and another closes the console when the child exits so the
//! reader sees the end of the output.
#![allow(unsafe_code)]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

/// The pseudo console, closed once (by whichever of the exit watcher and
/// `Drop` gets there first).
#[derive(Debug)]
struct Console(Mutex<Option<HPCON>>);

impl Console {
    fn close(&self) {
        if let Some(hpc) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // SAFETY: `hpc` came from CreatePseudoConsole and is closed only here.
            unsafe { ClosePseudoConsole(hpc) };
        }
    }
}

/// A running child process attached to a pseudo console.
#[derive(Debug)]
pub struct Pty {
    input: File,
    output: Receiver<io::Result<Vec<u8>>>,
    pending: Vec<u8>,
    process: Arc<OwnedHandle>,
    console: Arc<Console>,
    description: String,
}

impl Pty {
    /// Spawns `program` with `args` on a new pseudo console of the given
    /// size. `TERM` is set to `term`; the rest of the environment is inherited.
    pub fn spawn<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        rows: u16,
        cols: u16,
        term: &str,
    ) -> io::Result<Pty> {
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let mut hpc: HPCON = 0;
        // SAFETY: the pipe handles are valid; `hpc` receives the console.
        let hr = unsafe {
            CreatePseudoConsole(
                size(rows, cols),
                input_read.as_raw_handle() as HANDLE,
                output_write.as_raw_handle() as HANDLE,
                0,
                &mut hpc,
            )
        };
        if hr < 0 {
            return Err(io::Error::from_raw_os_error(hr));
        }
        let console = Arc::new(Console(Mutex::new(Some(hpc))));
        // The console holds its own copies of its ends of the pipes.
        drop((input_read, output_write));

        let process = match start(program, args, term, hpc) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                console.close();
                return Err(e);
            }
        };

        let (tx, output) = channel();
        let mut reader = File::from(output_read);
        std::thread::Builder::new()
            .name("conpty-reader".into())
            .spawn(move || {
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(Ok(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        // The pipe breaks when the console closes.
                        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => break,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
            })?;
        {
            let (process, console) = (process.clone(), console.clone());
            std::thread::Builder::new()
                .name("conpty-exit".into())
                .spawn(move || {
                    // SAFETY: the process handle outlives this thread (Arc).
                    unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE) };
                    console.close();
                })?;
        }

        let mut description = program.to_string();
        for arg in args {
            description.push(' ');
            description.push_str(&arg.as_ref().to_string_lossy());
        }
        Ok(Pty {
            input: File::from(input_write),
            output,
            pending: Vec::new(),
            process,
            console,
            description,
        })
    }

    /// Informs the child of a new console size.
    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        if let Some(hpc) = *self.console.0.lock().unwrap_or_else(|e| e.into_inner()) {
            // SAFETY: the console is open while the lock is held.
            let hr = unsafe { ResizePseudoConsole(hpc, size(rows, cols)) };
            if hr < 0 {
                return Err(io::Error::from_raw_os_error(hr));
            }
        }
        Ok(())
    }

    /// Waits up to `timeout` for output. Returns `Ok(0)` on timeout and
    /// `Err(UnexpectedEof)` once the child has gone and its output is read.
    pub fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        if self.pending.is_empty() {
            match self.output.recv_timeout(timeout) {
                Ok(Ok(data)) => self.pending = data,
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => return Ok(0),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.input.write_all(bytes)
    }

    /// Returns the exit status if the child has finished.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let handle = self.process.as_raw_handle() as HANDLE;
        // SAFETY: the process handle is valid for the life of `self`.
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code = 0u32;
        // SAFETY: as above; `code` receives the exit code.
        if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(ExitStatus::from_raw(code)))
    }

    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the process handle is valid for the life of `self`.
        if unsafe { TerminateProcess(self.process.as_raw_handle() as HANDLE, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl crate::Transport for Pty {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        Pty::read_timeout(self, buf, timeout)
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(PtyWriter(self.input.try_clone()?)))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        Pty::resize(self, rows, cols)
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

struct PtyWriter(File);

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl crate::TransportWriter for PtyWriter {}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Ok(None) = self.try_wait() {
            let _ = self.kill();
        }
        self.console.close();
    }
}

fn size(rows: u16, cols: u16) -> COORD {
    COORD {
        X: cols.min(i16::MAX as u16) as i16,
        Y: rows.min(i16::MAX as u16) as i16,
    }
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: both out-pointers are valid; no security attributes.
    if unsafe { CreatePipe(&mut read, &mut write, null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreatePipe returned two new handles we now own.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read as _),
            OwnedHandle::from_raw_handle(write as _),
        )
    })
}

/// Starts the child attached to the pseudo console `hpc`.
fn start<S: AsRef<OsStr>>(
    program: &str,
    args: &[S],
    term: &str,
    hpc: HPCON,
) -> io::Result<OwnedHandle> {
    let mut command_line = wide(&command_line(program, args));
    let environment = environment(term);

    let mut list_size = 0usize;
    // SAFETY: the first call only reports the size the list needs.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut list_size) };
    let mut list = vec![0usize; list_size.div_ceil(size_of::<usize>())];
    let list_ptr = list.as_mut_ptr().cast::<c_void>();
    // SAFETY: `list` is at least `list_size` bytes.
    if unsafe { InitializeProcThreadAttributeList(list_ptr, 1, 0, &mut list_size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        // SAFETY: the attribute value is the HPCON itself, passed by value
        // as documented for PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.
        if unsafe {
            UpdateProcThreadAttribute(
                list_ptr,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                hpc as *const c_void,
                size_of::<HPCON>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        // Without this a child of a process with redirected standard
        // handles would use those instead of the console.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup.lpAttributeList = list_ptr;
        let mut info = PROCESS_INFORMATION {
            hProcess: null_mut(),
            hThread: null_mut(),
            dwProcessId: 0,
            dwThreadId: 0,
        };
        // SAFETY: all pointers refer to live, correctly sized buffers; the
        // command line is mutable as CreateProcessW requires.
        let ok = unsafe {
            CreateProcessW(
                null(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                null(),
                &startup.StartupInfo,
                &mut info,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateProcessW returned handles we own.
        unsafe {
            drop(OwnedHandle::from_raw_handle(info.hThread as _));
            Ok(OwnedHandle::from_raw_handle(info.hProcess as _))
        }
    })();
    // SAFETY: the list was initialised above.
    unsafe { DeleteProcThreadAttributeList(list_ptr) };
    result
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// A command line in the quoting `CommandLineToArgvW` undoes.
fn command_line<S: AsRef<OsStr>>(program: &str, args: &[S]) -> std::ffi::OsString {
    let mut line = std::ffi::OsString::new();
    quote(OsStr::new(program), &mut line);
    for arg in args {
        line.push(" ");
        quote(arg.as_ref(), &mut line);
    }
    line
}

fn quote(arg: &OsStr, out: &mut std::ffi::OsString) {
    let text = arg.to_string_lossy();
    if !text.is_empty() && !text.contains([' ', '\t', '"']) {
        out.push(arg);
        return;
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for c in text.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
                continue;
            }
            _ => {}
        }
        if c != '\\' {
            quoted.extend(std::iter::repeat_n('\\', backslashes));
            backslashes = 0;
            quoted.push(c);
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    out.push(quoted);
}

/// This process's environment with `TERM` replaced, as a sorted
/// UTF-16 environment block.
fn environment(term: &str) -> Vec<u16> {
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("TERM"))
        .collect();
    vars.push(("TERM".into(), term.into()));
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_like_command_line_to_argv() {
        let line = command_line(
            "cmd.exe",
            &[
                "/C",
                "echo a b",
                r#"say "hi""#,
                r"C:\dir\",
                r"C:\my dir\",
                "",
            ],
        );
        assert_eq!(
            line.to_string_lossy(),
            r#"cmd.exe /C "echo a b" "say \"hi\"" C:\dir\ "C:\my dir\\" """#
        );
    }

    #[test]
    fn child_output_arrives() {
        let mut pty =
            Pty::spawn("cmd.exe", &["/C", "echo veetee %TERM%"], 24, 80, "vt420").expect("spawn");
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match pty.read_timeout(&mut buf, Duration::from_secs(10)) {
                Ok(0) => panic!("timed out; got {:?}", String::from_utf8_lossy(&out)),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => panic!("{e}"),
            }
        }
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("veetee vt420"), "{text:?}");
    }
}
