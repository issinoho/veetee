//! Telnet (RFC 854) with the options a DEC terminal needs: BINARY (856) for
//! 8-bit controls, ECHO (857), SUPPRESS-GO-AHEAD (858), TERMINAL-TYPE (1091),
//! NAWS window size (1073) and TERMINAL-SPEED (1079).
//!
//! BINARY is offered only when the caller asks for it. DEC's telnet server
//! answers it by putting the terminal in PASSALL, where the driver passes
//! input through untouched and DELETE no longer erases (OpenVMS restores it
//! with SET TERMINAL/INTERACTIVE). A server that proposes BINARY itself is
//! still answered.
//!
//! Option negotiation follows the RFC 1143 rule that prevents loops: we only
//! answer a request that changes an option's state, or that we initiated.

use crate::serial::{FlowControl, Line, Parity};
use socket2::{SockRef, TcpKeepalive};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const BRK: u8 = 243;
const SE: u8 = 240;

const BINARY: u8 = 0;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;
const TSPEED: u8 = 32;
/// RFC 2217 COM Port Control, for terminal servers. Proved against ser2net,
/// which answered every setting with what it was asked for. 🔎 Whether a
/// DECserver answers it at all is untested, the option being from 1997 and
/// DEC's own servers older. See `docs/rfc2217-testing.md`.
const COM_PORT: u8 = 44;

// RFC 2217 client commands. The server answers with the command plus 100,
// reporting what it actually set.
const SET_BAUDRATE: u8 = 1;
const SET_DATASIZE: u8 = 2;
const SET_PARITY: u8 = 3;
const SET_STOPSIZE: u8 = 4;
const SET_CONTROL: u8 = 5;

// SET_CONTROL values (RFC 2217 table 2).
const FLOW_NONE: u8 = 1;
const FLOW_XON_XOFF: u8 = 2;
const FLOW_HARDWARE: u8 = 3;
const BREAK_ON: u8 = 4;
const BREAK_OFF: u8 = 5;
// The same for the inbound direction alone: the server sending XOFF to the
// host when its buffer fills.
const INBOUND_FLOW_NONE: u8 = 14;
const INBOUND_FLOW_XON_XOFF: u8 = 15;

const IS: u8 = 0;
const SEND: u8 = 1;

/// Options we will perform locally (answer DO with WILL).
fn local_supported(option: u8, o: &Options) -> bool {
    match option {
        COM_PORT => o.com_port.is_some(),
        _ => matches!(option, BINARY | SGA | TTYPE | NAWS | TSPEED),
    }
}

/// Options we accept from the server (answer WILL with DO).
fn remote_supported(option: u8) -> bool {
    matches!(option, BINARY | ECHO | SGA)
}

/// The options offered on connect: what a DEC terminal wants, which the
/// server may refuse. BINARY goes out only when `binary` asks for it.
fn hello(options: &mut Options, binary: bool) -> Vec<u8> {
    let mut hello = Vec::new();
    for option in [BINARY, SGA, TTYPE, NAWS] {
        if option == BINARY && !binary {
            continue;
        }
        options.us_pending[usize::from(option)] = true;
        hello.extend_from_slice(&[IAC, WILL, option]);
    }
    if options.com_port.is_some() {
        options.us_pending[usize::from(COM_PORT)] = true;
        hello.extend_from_slice(&[IAC, WILL, COM_PORT]);
    }
    for option in [BINARY, SGA] {
        if option == BINARY && !binary {
            continue;
        }
        options.them_pending[usize::from(option)] = true;
        hello.extend_from_slice(&[IAC, DO, option]);
    }
    hello
}

/// The line settings, sent once the server agrees to COM Port Control.
fn com_port_settings(o: &Options, replies: &mut Vec<u8>) {
    let Some(port) = o.com_port else { return };
    let mut send = |command: u8, data: &[u8]| {
        replies.extend_from_slice(&[IAC, SB, COM_PORT, command]);
        for &b in data {
            // RFC 854: a 255 in subnegotiation data is doubled.
            replies.push(b);
            if b == IAC {
                replies.push(IAC);
            }
        }
        replies.extend_from_slice(&[IAC, SE]);
    };
    send(SET_BAUDRATE, &port.baud.to_be_bytes());
    send(SET_DATASIZE, &[port.data_bits]);
    send(
        SET_PARITY,
        &[match port.parity {
            Parity::None => 1,
            Parity::Odd => 2,
            Parity::Even => 3,
            Parity::Mark => 4,
            Parity::Space => 5,
        }],
    );
    // 1 and 2 stop bits share their numbers with RFC 2217; 3 would be 1.5.
    send(SET_STOPSIZE, &[port.stop_bits]);
    // Outbound, or both ways; a flow control one way only sets outbound and
    // then inbound on its own.
    send(
        SET_CONTROL,
        &[match port.flow {
            FlowControl::None | FlowControl::XonXoffReceive => FLOW_NONE,
            FlowControl::XonXoff | FlowControl::XonXoffTransmit => FLOW_XON_XOFF,
            FlowControl::RtsCts => FLOW_HARDWARE,
        }],
    );
    match port.flow {
        FlowControl::XonXoffTransmit => send(SET_CONTROL, &[INBOUND_FLOW_NONE]),
        FlowControl::XonXoffReceive => send(SET_CONTROL, &[INBOUND_FLOW_XON_XOFF]),
        _ => {}
    }
}

/// Serial line settings for a terminal server, sent with RFC 2217 COM Port
/// Control. The defaults are DEC's factory Set-Up values.
pub type ComPort = Line;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelnetConfig {
    pub host: String,
    pub port: u16,
    /// Sent for TERMINAL-TYPE, e.g. `VT420`.
    pub terminal_type: String,
    pub rows: u16,
    pub cols: u16,
    /// Offer BINARY, for 8-bit controls. Off by default: OpenVMS answers it
    /// by putting the terminal in PASSALL, which loses DCL line editing.
    pub binary: bool,
    /// Serial line settings to ask a terminal server for (RFC 2217). The
    /// option is offered only when this is set: a server that does not know
    /// it refuses, but there is no reason to raise it with an ordinary host.
    pub com_port: Option<ComPort>,
}

impl TelnetConfig {
    pub fn new(host: impl Into<String>, terminal_type: impl Into<String>) -> TelnetConfig {
        TelnetConfig {
            host: host.into(),
            port: 23,
            terminal_type: terminal_type.into(),
            rows: 24,
            cols: 80,
            binary: false,
            com_port: None,
        }
    }
}

/// Negotiated state shared by the reading and writing halves.
#[derive(Debug)]
struct Options {
    us: [bool; 256],
    them: [bool; 256],
    us_pending: [bool; 256],
    them_pending: [bool; 256],
    rows: u16,
    cols: u16,
    com_port: Option<ComPort>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            us: [false; 256],
            them: [false; 256],
            us_pending: [false; 256],
            them_pending: [false; 256],
            rows: 24,
            cols: 80,
            com_port: None,
        }
    }
}

impl Options {
    fn local_binary(&self) -> bool {
        self.us[usize::from(BINARY)]
    }

    fn remote_binary(&self) -> bool {
        self.them[usize::from(BINARY)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Cr,
    Iac,
    Verb(u8),
    Sub,
    SubIac,
}

/// Incoming stream decoder. Separated from the socket so it can be tested.
#[derive(Debug)]
struct Decoder {
    state: State,
    sub: Vec<u8>,
    terminal_type: String,
}

impl Decoder {
    fn new(terminal_type: String) -> Decoder {
        Decoder {
            state: State::Data,
            sub: Vec::new(),
            terminal_type,
        }
    }

    /// Decodes `buf` in place, returning the number of data bytes left at the
    /// front. Negotiation replies are appended to `replies`.
    fn decode(&mut self, buf: &mut [u8], options: &mut Options, replies: &mut Vec<u8>) -> usize {
        let mut out = 0;
        for i in 0..buf.len() {
            let b = buf[i];
            match self.state {
                State::Data | State::Cr => {
                    let after_cr = self.state == State::Cr;
                    self.state = State::Data;
                    if b == IAC {
                        self.state = State::Iac;
                    } else if after_cr && b == 0 && !options.remote_binary() {
                        // NVT: CR NUL means a bare carriage return.
                    } else {
                        buf[out] = b;
                        out += 1;
                        if b == b'\r' {
                            self.state = State::Cr;
                        }
                    }
                }
                State::Iac => {
                    self.state = State::Data;
                    match b {
                        IAC => {
                            buf[out] = IAC;
                            out += 1;
                        }
                        WILL | WONT | DO | DONT => self.state = State::Verb(b),
                        SB => {
                            self.sub.clear();
                            self.state = State::Sub;
                        }
                        // NOP, DM, GA and other commands carry no data for us.
                        _ => {}
                    }
                }
                State::Verb(verb) => {
                    self.state = State::Data;
                    negotiate(verb, b, options, replies);
                }
                State::Sub => {
                    if b == IAC {
                        self.state = State::SubIac;
                    } else if self.sub.len() < 1024 {
                        self.sub.push(b);
                    }
                }
                State::SubIac => match b {
                    IAC => {
                        self.sub.push(IAC);
                        self.state = State::Sub;
                    }
                    SE => {
                        self.state = State::Data;
                        self.subnegotiation(options, replies);
                    }
                    _ => self.state = State::Data,
                },
            }
        }
        out
    }

    fn subnegotiation(&self, options: &Options, replies: &mut Vec<u8>) {
        match self.sub.as_slice() {
            [TTYPE, SEND, ..] if options.us[usize::from(TTYPE)] => {
                replies.extend_from_slice(&[IAC, SB, TTYPE, IS]);
                replies.extend_from_slice(self.terminal_type.as_bytes());
                replies.extend_from_slice(&[IAC, SE]);
            }
            [TSPEED, SEND, ..] if options.us[usize::from(TSPEED)] => {
                replies.extend_from_slice(&[IAC, SB, TSPEED, IS]);
                replies.extend_from_slice(b"9600,9600");
                replies.extend_from_slice(&[IAC, SE]);
            }
            // RFC 2217 replies (the command plus 100) report what the server
            // set. We asked for what we want and have nothing to add.
            [COM_PORT, ..] => {}
            _ => {}
        }
    }
}

fn negotiate(verb: u8, option: u8, o: &mut Options, replies: &mut Vec<u8>) {
    let i = usize::from(option);
    match verb {
        DO => {
            if local_supported(option, o) {
                let was_pending = std::mem::take(&mut o.us_pending[i]);
                if !o.us[i] {
                    o.us[i] = true;
                    if !was_pending {
                        replies.extend_from_slice(&[IAC, WILL, option]);
                    }
                    if option == NAWS {
                        window_size(o, replies);
                    }
                    if option == COM_PORT {
                        com_port_settings(o, replies);
                    }
                }
            } else {
                replies.extend_from_slice(&[IAC, WONT, option]);
            }
        }
        DONT => {
            let was_pending = std::mem::take(&mut o.us_pending[i]);
            if o.us[i] || was_pending {
                if o.us[i] && !was_pending {
                    replies.extend_from_slice(&[IAC, WONT, option]);
                }
                o.us[i] = false;
            }
        }
        WILL => {
            if remote_supported(option) {
                let was_pending = std::mem::take(&mut o.them_pending[i]);
                if !o.them[i] {
                    o.them[i] = true;
                    if !was_pending {
                        replies.extend_from_slice(&[IAC, DO, option]);
                    }
                }
            } else {
                replies.extend_from_slice(&[IAC, DONT, option]);
            }
        }
        WONT => {
            let was_pending = std::mem::take(&mut o.them_pending[i]);
            if o.them[i] && !was_pending {
                replies.extend_from_slice(&[IAC, DONT, option]);
            }
            o.them[i] = false;
        }
        _ => {}
    }
}

/// NAWS subnegotiation with the current page size.
fn window_size(o: &Options, out: &mut Vec<u8>) {
    out.extend_from_slice(&[IAC, SB, NAWS]);
    for value in [o.cols, o.rows] {
        for byte in value.to_be_bytes() {
            out.push(byte);
            if byte == IAC {
                out.push(IAC);
            }
        }
    }
    out.extend_from_slice(&[IAC, SE]);
}

/// Encodes outgoing data: doubles IAC and, outside binary mode, sends a
/// bare CR as CR NUL (RFC 854).
fn encode(data: &[u8], local_binary: bool, out: &mut Vec<u8>) {
    for (i, &b) in data.iter().enumerate() {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        } else if b == b'\r' && !local_binary && data.get(i + 1) != Some(&b'\n') {
            out.push(0);
        }
    }
}

/// A Telnet connection.
#[derive(Debug)]
pub struct Telnet {
    stream: TcpStream,
    options: Arc<Mutex<Options>>,
    decoder: Decoder,
    description: String,
    /// The receive timeout currently set on the socket.
    read_timeout: Option<Duration>,
}

impl Telnet {
    /// Connects and starts negotiation. Tries each resolved address in turn.
    pub fn connect(config: TelnetConfig) -> io::Result<Telnet> {
        let addrs = (config.host.as_str(), config.port)
            .to_socket_addrs()
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", config.host)))?;
        let mut last_error = io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no addresses", config.host),
        );
        let mut stream = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(15)) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => {
                    last_error =
                        io::Error::new(e.kind(), format!("{}:{}: {e}", config.host, config.port))
                }
            }
        }
        let mut stream = stream.ok_or(last_error)?;
        stream.set_nodelay(true)?;
        // A terminal session sits idle for as long as the user is reading, and
        // OpenVMS sends nothing meanwhile, so a firewall or NAT between the two
        // is free to forget the connection: the next keystroke then fails with
        // "an established connection was aborted". Keepalives hold the path
        // open. The times are the BSD defaults rather than the two hours of
        // RFC 1122, which is longer than the idle timeout of most firewalls.
        let keepalive = TcpKeepalive::new()
            .with_time(Duration::from_secs(60))
            .with_interval(Duration::from_secs(15));
        SockRef::from(&stream).set_tcp_keepalive(&keepalive)?;

        let mut options = Options {
            rows: config.rows,
            cols: config.cols,
            com_port: config.com_port,
            ..Options::default()
        };
        stream.write_all(&hello(&mut options, config.binary))?;

        let description = if config.port == 23 {
            format!("telnet {}", config.host)
        } else {
            format!("telnet {} {}", config.host, config.port)
        };
        Ok(Telnet {
            stream,
            options: Arc::new(Mutex::new(options)),
            decoder: Decoder::new(config.terminal_type),
            description,
            read_timeout: None,
        })
    }
}

impl crate::Transport for Telnet {
    fn flow_in_band(&self) -> bool {
        true
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        // A zero timeout would mean "block forever" to the socket.
        let timeout = timeout.max(Duration::from_millis(1));
        if self.read_timeout != Some(timeout) {
            self.stream.set_read_timeout(Some(timeout))?;
            self.read_timeout = Some(timeout);
        }
        let n = match self.stream.read(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed by foreign host",
                ));
            }
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(0);
            }
            Err(e) => return Err(e),
        };
        let mut replies = Vec::new();
        let data = {
            let mut options = self.options.lock().unwrap_or_else(|e| e.into_inner());
            self.decoder
                .decode(&mut buf[..n], &mut options, &mut replies)
        };
        if !replies.is_empty() {
            self.stream.write_all(&replies)?;
        }
        Ok(data)
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(TelnetWriter {
            stream: self.stream.try_clone()?,
            options: self.options.clone(),
        }))
    }

    fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        let mut out = Vec::new();
        {
            let mut o = self.options.lock().unwrap_or_else(|e| e.into_inner());
            o.rows = rows;
            o.cols = cols;
            if o.us[usize::from(NAWS)] {
                window_size(&o, &mut out);
            }
        }
        if !out.is_empty() {
            self.stream.write_all(&out)?;
        }
        Ok(())
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn line(&self) -> Option<Line> {
        self.options
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .com_port
    }
}

struct TelnetWriter {
    stream: TcpStream,
    options: Arc<Mutex<Options>>,
}

impl Write for TelnetWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let binary = self
            .options
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .local_binary();
        let mut out = Vec::with_capacity(data.len() + 8);
        encode(data, binary, &mut out);
        self.stream.write_all(&out)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl crate::TransportWriter for TelnetWriter {
    fn send_break(&mut self) -> io::Result<()> {
        let com_port = {
            let o = self.options.lock().unwrap_or_else(|e| e.into_inner());
            o.us[usize::from(COM_PORT)]
        };
        if com_port {
            // RFC 2217 sends a break as the control on and then off, so the
            // terminal server holds the line for the gap between them.
            self.stream.write_all(&[
                IAC,
                SB,
                COM_PORT,
                SET_CONTROL,
                BREAK_ON,
                IAC,
                SE,
                IAC,
                SB,
                COM_PORT,
                SET_CONTROL,
                BREAK_OFF,
                IAC,
                SE,
            ])
        } else {
            self.stream.write_all(&[IAC, BRK])
        }
    }

    /// Asks the terminal server for new line settings, on a connection that
    /// asked for COM Port Control; if the server has not agreed to it yet,
    /// they go when it does.
    fn set_line(&mut self, line: &Line) -> io::Result<()> {
        let mut out = Vec::new();
        {
            let mut o = self.options.lock().unwrap_or_else(|e| e.into_inner());
            let Some(port) = o.com_port.as_mut() else {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "this connection did not ask for COM Port Control",
                ));
            };
            *port = *line;
            if o.us[usize::from(COM_PORT)] {
                com_port_settings(&o, &mut out);
            }
        }
        self.stream.write_all(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transport;
    use std::net::TcpListener;

    fn decode(decoder: &mut Decoder, options: &mut Options, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut buf = input.to_vec();
        let mut replies = Vec::new();
        let n = decoder.decode(&mut buf, options, &mut replies);
        buf.truncate(n);
        (buf, replies)
    }

    #[test]
    fn data_iac_and_cr_nul() {
        let mut d = Decoder::new("VT420".into());
        let mut o = Options::default();
        let (data, replies) = decode(&mut d, &mut o, b"a\xff\xffb\r\0c\r\nd");
        assert_eq!(data, b"a\xffb\rc\r\nd");
        assert!(replies.is_empty());
        o.them[usize::from(BINARY)] = true;
        let (data, _) = decode(&mut d, &mut o, b"\r\0");
        assert_eq!(data, b"\r\0", "binary mode keeps NUL");
    }

    #[test]
    fn binary_is_offered_only_when_asked_for() {
        let mut options = Options::default();
        assert_eq!(
            hello(&mut options, false),
            [
                IAC, WILL, SGA, IAC, WILL, TTYPE, IAC, WILL, NAWS, IAC, DO, SGA
            ]
        );
        assert!(!options.us_pending[usize::from(BINARY)]);
        // A server that proposes BINARY itself is still answered.
        let mut decoder = Decoder::new("VT420".into());
        let (_, replies) = decode(&mut decoder, &mut options, &[IAC, WILL, BINARY]);
        assert_eq!(replies, [IAC, DO, BINARY]);

        let mut options = Options::default();
        let offered = hello(&mut options, true);
        assert!(offered.starts_with(&[IAC, WILL, BINARY]));
        assert!(offered.windows(3).any(|w| w == [IAC, DO, BINARY]));
    }

    #[test]
    fn commands_split_across_reads() {
        let mut d = Decoder::new("VT420".into());
        let mut o = Options::default();
        let (a, ra) = decode(&mut d, &mut o, b"x\xff");
        let (b, rb) = decode(&mut d, &mut o, b"\xfb");
        let (c, rc) = decode(&mut d, &mut o, b"\x01y");
        assert_eq!([a, b, c].concat(), b"xy");
        assert_eq!([ra, rb, rc].concat(), [IAC, DO, ECHO]);
    }

    #[test]
    fn negotiation_accepts_supported_and_refuses_others() {
        let mut d = Decoder::new("VT420".into());
        let mut o = Options::default();
        let (_, r) = decode(
            &mut d,
            &mut o,
            &[IAC, DO, TTYPE, IAC, DO, 39, IAC, WILL, ECHO, IAC, WILL, 34],
        );
        assert_eq!(
            r,
            [
                IAC, WILL, TTYPE, IAC, WONT, 39, IAC, DO, ECHO, IAC, DONT, 34
            ]
        );
        // Repeating an accepted request must not produce another reply (no loops).
        let (_, r) = decode(&mut d, &mut o, &[IAC, DO, TTYPE, IAC, WILL, ECHO]);
        assert!(r.is_empty());
    }

    #[test]
    fn pending_requests_are_not_acknowledged_twice() {
        let mut d = Decoder::new("VT420".into());
        let mut o = Options::default();
        o.us_pending[usize::from(BINARY)] = true;
        let (_, r) = decode(&mut d, &mut o, &[IAC, DO, BINARY]);
        assert!(r.is_empty());
        assert!(o.local_binary());
    }

    #[test]
    fn terminal_type_and_window_size() {
        let mut d = Decoder::new("VT420".into());
        let mut o = Options {
            rows: 24,
            cols: 255,
            ..Options::default()
        };
        let (_, r) = decode(&mut d, &mut o, &[IAC, DO, NAWS]);
        assert_eq!(
            r,
            [IAC, WILL, NAWS, IAC, SB, NAWS, 0, 255, 255, 0, 24, IAC, SE]
        );
        o.us[usize::from(TTYPE)] = true;
        let (_, r) = decode(&mut d, &mut o, &[IAC, SB, TTYPE, SEND, IAC, SE]);
        assert_eq!(
            r,
            [&[IAC, SB, TTYPE, IS][..], b"VT420", &[IAC, SE]].concat()
        );
    }

    #[test]
    fn outgoing_encoding() {
        let mut out = Vec::new();
        encode(b"a\xff\r", false, &mut out);
        assert_eq!(out, b"a\xff\xff\r\0");
        out.clear();
        encode(b"\r\n\r", true, &mut out);
        assert_eq!(out, b"\r\n\r");
    }

    #[test]
    fn com_port_control_is_offered_only_when_asked_for() {
        let has_will = |hello: &[u8]| hello.windows(3).any(|w| w == [IAC, WILL, COM_PORT]);
        let mut plain = Options::default();
        assert!(
            !has_will(&hello(&mut plain, false)),
            "not raised with an ordinary host"
        );
        let mut asked = Options {
            com_port: Some(ComPort::default()),
            ..Options::default()
        };
        assert!(has_will(&hello(&mut asked, false)));
    }

    #[test]
    fn com_port_settings_go_out_when_the_server_agrees() {
        let mut options = Options {
            com_port: Some(ComPort {
                baud: 19200,
                data_bits: 7,
                parity: Parity::Even,
                stop_bits: 2,
                flow: FlowControl::RtsCts,
            }),
            ..Options::default()
        };
        let mut decoder = Decoder::new("VT420".into());
        let (_, replies) = decode(&mut decoder, &mut options, &[IAC, DO, COM_PORT]);
        let mut expect = vec![IAC, WILL, COM_PORT];
        for (command, data) in [
            (SET_BAUDRATE, &[0, 0, 0x4B, 0][..]), // 19200
            (SET_DATASIZE, &[7][..]),
            (SET_PARITY, &[3][..]), // even
            (SET_STOPSIZE, &[2][..]),
            (SET_CONTROL, &[FLOW_HARDWARE][..]),
        ] {
            expect.extend_from_slice(&[IAC, SB, COM_PORT, command]);
            expect.extend_from_slice(data);
            expect.extend_from_slice(&[IAC, SE]);
        }
        assert_eq!(replies, expect);
    }

    #[test]
    fn flow_control_one_way_sets_inbound_on_its_own() {
        let options = Options {
            com_port: Some(ComPort {
                flow: FlowControl::XonXoffTransmit,
                ..ComPort::default()
            }),
            ..Options::default()
        };
        let mut replies = Vec::new();
        com_port_settings(&options, &mut replies);
        assert!(replies.ends_with(&[
            IAC,
            SB,
            COM_PORT,
            SET_CONTROL,
            FLOW_XON_XOFF,
            IAC,
            SE,
            IAC,
            SB,
            COM_PORT,
            SET_CONTROL,
            INBOUND_FLOW_NONE,
            IAC,
            SE
        ]));
    }

    #[test]
    fn new_line_settings_go_to_the_server_while_connected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || listener.accept().unwrap().0);
        let mut t = Telnet::connect(TelnetConfig {
            port,
            com_port: Some(ComPort::default()),
            ..TelnetConfig::new("127.0.0.1", "VT420")
        })
        .unwrap();
        let mut server = server.join().unwrap();
        // The server agrees to COM Port Control, and gets the factory line.
        server.write_all(&[IAC, DO, COM_PORT]).unwrap();
        let mut buf = [0u8; 64];
        // Negotiation is not data: each read returns none, having acted on it.
        for _ in 0..5 {
            t.read_timeout(&mut buf, Duration::from_millis(50)).unwrap();
        }
        assert_eq!(t.line(), Some(ComPort::default()));
        let line = ComPort {
            baud: 19200,
            ..ComPort::default()
        };
        t.writer().unwrap().set_line(&line).unwrap();
        assert_eq!(t.line(), Some(line));
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut got = Vec::new();
        let wanted = [IAC, SB, COM_PORT, SET_BAUDRATE, 0, 0, 0x4B, 0, IAC, SE];
        while !got.windows(wanted.len()).any(|w| w == wanted) {
            let n = server.read(&mut buf).unwrap();
            assert_ne!(n, 0, "the server saw {got:?}");
            got.extend_from_slice(&buf[..n]);
        }
    }

    #[test]
    fn a_line_cannot_be_set_without_com_port_control() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || listener.accept().unwrap());
        let t = Telnet::connect(TelnetConfig {
            port,
            ..TelnetConfig::new("127.0.0.1", "VT420")
        })
        .unwrap();
        let _server = server.join().unwrap();
        assert_eq!(t.line(), None);
        let err = t
            .writer()
            .unwrap()
            .set_line(&ComPort::default())
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn a_255_in_the_settings_is_doubled() {
        let options = Options {
            com_port: Some(ComPort {
                baud: 255,
                ..ComPort::default()
            }),
            ..Options::default()
        };
        let mut replies = Vec::new();
        com_port_settings(&options, &mut replies);
        assert!(
            replies
                .windows(6)
                .any(|w| w == [COM_PORT, SET_BAUDRATE, 0, 0, 0, IAC]),
            "the 255 of the baud rate is escaped: {replies:?}"
        );
        let (start, _) = replies
            .windows(2)
            .enumerate()
            .find(|(_, w)| *w == [IAC, IAC])
            .expect("a doubled 255");
        assert_eq!(replies[start..start + 4], [IAC, IAC, IAC, SE]);
    }

    #[test]
    fn keeps_the_connection_alive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || listener.accept().unwrap());
        let t = Telnet::connect(TelnetConfig {
            port,
            ..TelnetConfig::new("127.0.0.1", "VT420")
        })
        .unwrap();
        assert!(
            SockRef::from(&t.stream).keepalive().unwrap(),
            "an idle session must survive a firewall that forgets it"
        );
        drop(server.join().unwrap());
    }

    #[test]
    fn talks_to_a_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&[IAC, DO, TTYPE, IAC, WILL, ECHO]).unwrap();
            s.write_all(&[IAC, SB, TTYPE, SEND, IAC, SE]).unwrap();
            s.write_all(b"Username: ").unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 256];
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            while !got.windows(7).any(|w| w == b"SYSTEM\r") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            got
        });
        let mut t = Telnet::connect(TelnetConfig {
            port,
            ..TelnetConfig::new("127.0.0.1", "VT420")
        })
        .unwrap();
        let mut screen = Vec::new();
        let mut buf = [0u8; 256];
        while !screen.ends_with(b"Username: ") {
            let n = t.read_timeout(&mut buf, Duration::from_secs(2)).unwrap();
            screen.extend_from_slice(&buf[..n]);
        }
        let mut w = t.writer().unwrap();
        w.write_all(b"SYSTEM\r").unwrap();
        let got = server.join().unwrap();
        let ttype = [&[IAC, SB, TTYPE, IS][..], b"VT420", &[IAC, SE]].concat();
        assert!(
            got.windows(ttype.len()).any(|win| win == ttype.as_slice()),
            "{got:?}"
        );
        assert!(
            got.ends_with(b"SYSTEM\r\0"),
            "not binary: bare CR sent as CR NUL: {got:?}"
        );
    }
}
