//! Windows pseudo consoles (ConPTY, Windows 10 1809 and later). The child
//! runs attached to a pseudo console fed through two pipes; a thread reads
//! its output, and another closes the console when the child exits so the
//! reader sees the end of the output. `Pty::spawn_piped` gives the child
//! pipes for its standard handles as well (see `piped.rs`).
#![allow(unsafe_code)]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

use super::piped::{ConsoleText, Newlines, Routing};

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
    input: PtyWriter,
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
        Pty::open(program, args, rows, cols, term, &[], false)
    }

    /// Spawns `program` as `spawn` does, but with pipes for its standard
    /// input, output and error, so that what it reads and writes there
    /// passes untouched. The pseudo console carries only what the program
    /// reads from and writes to the console itself, such as OpenSSH's
    /// password prompts; typed input goes there until the program's
    /// standard output shows the session has begun (see `piped.rs`).
    // 🔎 A program that takes the window size from its standard output, as
    // OpenSSH does (`GetConsoleScreenBufferInfo` on STD_OUTPUT_HANDLE),
    // finds none, so the host keeps its default size and is not told of resizes.
    pub fn spawn_piped<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        rows: u16,
        cols: u16,
        term: &str,
    ) -> io::Result<Pty> {
        Pty::open(program, args, rows, cols, term, &[], true)
    }

    /// Spawns `program` as `spawn_piped` does, with `env` added to its
    /// environment.
    pub fn spawn_piped_with_env<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        rows: u16,
        cols: u16,
        term: &str,
        env: &[(&str, &str)],
    ) -> io::Result<Pty> {
        Pty::open(program, args, rows, cols, term, env, true)
    }

    fn open<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        rows: u16,
        cols: u16,
        term: &str,
        env: &[(&str, &str)],
        piped: bool,
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

        // Standard input, output and error: (read, write) each.
        let standard = if piped {
            Some([pipe()?, pipe()?, pipe()?])
        } else {
            None
        };
        let child_ends = standard.as_ref().map(|[stdin, stdout, stderr]| {
            [&stdin.0, &stdout.1, &stderr.1].map(|h| h.as_raw_handle() as HANDLE)
        });
        let process = match start(program, args, term, env, hpc, child_ends) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                console.close();
                return Err(e);
            }
        };

        let (tx, output) = channel();
        let console_input = File::from(input_write);
        let input = match standard {
            None => {
                read_into("conpty-reader", output_read, tx, <[u8]>::to_vec)?;
                PtyWriter {
                    console: console_input,
                    piped: None,
                }
            }
            Some(
                [
                    (stdin_read, stdin_write),
                    (stdout_read, stdout_write),
                    (stderr_read, stderr_write),
                ],
            ) => {
                // The child holds its own copies of its ends.
                drop((stdin_read, stdout_write, stderr_write));
                let routing = Arc::new(Routing::default());
                let mut text = ConsoleText::default();
                read_into("conpty-reader", output_read, tx.clone(), move |b| {
                    text.filter(b)
                })?;
                let stdout_routing = routing.clone();
                read_into("conpty-stdout", stdout_read, tx.clone(), move |b| {
                    stdout_routing.standard_output(b);
                    b.to_vec()
                })?;
                let mut newlines = Newlines::default();
                read_into("conpty-stderr", stderr_read, tx, move |b| {
                    newlines.filter(b)
                })?;
                PtyWriter {
                    console: console_input,
                    piped: Some((File::from(stdin_write), routing)),
                }
            }
        };
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
            input,
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
        Ok(Box::new(self.input.try_clone()?))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        Pty::resize(self, rows, cols)
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}

/// Input for the child: the console's, or with pipes, standard input once
/// the session has begun.
#[derive(Debug)]
struct PtyWriter {
    console: File,
    piped: Option<(File, Arc<Routing>)>,
}

impl PtyWriter {
    fn try_clone(&self) -> io::Result<PtyWriter> {
        let piped = match &self.piped {
            Some((stdin, routing)) => Some((stdin.try_clone()?, routing.clone())),
            None => None,
        };
        Ok(PtyWriter {
            console: self.console.try_clone()?,
            piped,
        })
    }
}

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.piped {
            None => self.console.write(buf),
            Some((stdin, routing)) if routing.started() => stdin.write(buf),
            Some((_, routing)) => {
                self.console.write_all(&routing.to_console(buf))?;
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.console.flush()?;
        if let Some((stdin, _)) = &mut self.piped {
            stdin.flush()?;
        }
        Ok(())
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

/// Reads `handle` on a thread of its own, sending what `shape` makes of
/// each read to `tx` until the end of the output.
fn read_into(
    name: &str,
    handle: OwnedHandle,
    tx: Sender<io::Result<Vec<u8>>>,
    mut shape: impl FnMut(&[u8]) -> Vec<u8> + Send + 'static,
) -> io::Result<()> {
    let mut reader = File::from(handle);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = shape(&buf[..n]);
                        if !data.is_empty() && tx.send(Ok(data)).is_err() {
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
    Ok(())
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

/// Starts the child attached to the pseudo console `hpc`, with `standard` as its
/// standard input, output and error if given (or else the console's).
fn start<S: AsRef<OsStr>>(
    program: &str,
    args: &[S],
    term: &str,
    env: &[(&str, &str)],
    hpc: HPCON,
    standard: Option<[HANDLE; 3]>,
) -> io::Result<OwnedHandle> {
    let mut command_line = wide(&command_line(program, args));
    let environment = environment(term, env);

    let attributes = if standard.is_some() { 2 } else { 1 };
    let mut list_size = 0usize;
    // SAFETY: the first call only reports the size the list needs.
    unsafe { InitializeProcThreadAttributeList(null_mut(), attributes, 0, &mut list_size) };
    let mut list = vec![0usize; list_size.div_ceil(size_of::<usize>())];
    let list_ptr = list.as_mut_ptr().cast::<c_void>();
    // SAFETY: `list` is at least `list_size` bytes.
    if unsafe { InitializeProcThreadAttributeList(list_ptr, attributes, 0, &mut list_size) } == 0 {
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
        if let Some(handles) = &standard {
            // The pipes are inherited, and only they: the handle list keeps
            // any other inheritable handle of veetee's out of the child.
            for &h in handles {
                // SAFETY: `h` is an open pipe handle.
                if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            // SAFETY: `handles` outlives the CreateProcessW call below.
            if unsafe {
                UpdateProcThreadAttribute(
                    list_ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    size_of_val(handles),
                    null_mut(),
                    null(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let [stdin, stdout, stderr] = standard.unwrap_or([INVALID_HANDLE_VALUE; 3]);
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        // Without this a child of a process with redirected standard
        // handles would use those instead of the console.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin;
        startup.StartupInfo.hStdOutput = stdout;
        startup.StartupInfo.hStdError = stderr;
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
                i32::from(standard.is_some()),
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

/// This process's environment with `TERM` and the variables in `env`
/// replaced, as a sorted UTF-16 environment block.
fn environment(term: &str, env: &[(&str, &str)]) -> Vec<u16> {
    let replaced = |k: &std::ffi::OsStr| {
        k.eq_ignore_ascii_case("TERM") || env.iter().any(|(e, _)| k.eq_ignore_ascii_case(e))
    };
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> =
        std::env::vars_os().filter(|(k, _)| !replaced(k)).collect();
    vars.push(("TERM".into(), term.into()));
    vars.extend(env.iter().map(|(k, v)| ((*k).into(), (*v).into())));
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

    #[test]
    fn piped_output_arrives_untouched() {
        let mut pty = Pty::spawn_piped("cmd.exe", &["/C", "echo veetee %TERM%"], 24, 80, "vt420")
            .expect("spawn");
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
        // Through the pipe, with none of the console's escape sequences.
        assert!(!out.contains(&0x1B), "{:?}", String::from_utf8_lossy(&out));
        assert!(out.windows(14).any(|w| w == b"veetee vt420\r\n"), "{out:?}");
    }
}
