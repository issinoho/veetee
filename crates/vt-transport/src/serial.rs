//! Serial line connections (RS-232, USB serial adapters).
//!
//! Defaults follow the DEC VT420 factory Set-Up: 9600 baud, 8 data bits,
//! no parity, 1 stop bit and XON/XOFF flow control.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::Serial;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Serial;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
    Mark,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    None,
    /// Both ways: stops sending at the host's XOFF, and sends XOFF when the
    /// port's buffer fills.
    XonXoff,
    /// Stops sending at the host's XOFF but sends none: a VT420 whose
    /// Communications Set-Up says No XOFF, which still honours the host's.
    XonXoffTransmit,
    /// Sends XOFF when the buffer fills but does not stop at the host's (a
    /// VT500 with transmit flow control off and receive XON/XOFF).
    XonXoffReceive,
    RtsCts,
}

/// The settings of a line — speed, data format and flow control — apart
/// from the port they are set on. The default is DEC's factory Set-Up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow: FlowControl,
}

impl Default for Line {
    fn default() -> Line {
        Line {
            baud: 9600,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            flow: FlowControl::XonXoff,
        }
    }
}

impl Line {
    /// Refuses a data format no port has.
    pub fn check(&self) -> io::Result<()> {
        if !(5..=8).contains(&self.data_bits) {
            return Err(invalid("data bits must be 5 to 8"));
        }
        if !(1..=2).contains(&self.stop_bits) {
            return Err(invalid("stop bits must be 1 or 2"));
        }
        Ok(())
    }
}

impl fmt::Display for Line {
    /// `9600 8N1`, plus the flow control when not XON/XOFF.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parity = match self.parity {
            Parity::None => 'N',
            Parity::Even => 'E',
            Parity::Odd => 'O',
            Parity::Mark => 'M',
            Parity::Space => 'S',
        };
        write!(
            f,
            "{} {}{}{}",
            self.baud, self.data_bits, parity, self.stop_bits
        )?;
        match self.flow {
            FlowControl::XonXoff => Ok(()),
            FlowControl::XonXoffTransmit => f.write_str(" XON/XOFF transmit only"),
            FlowControl::XonXoffReceive => f.write_str(" XON/XOFF receive only"),
            FlowControl::None => f.write_str(" no flow control"),
            FlowControl::RtsCts => f.write_str(" RTS/CTS"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    pub device: PathBuf,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow: FlowControl,
}

impl SerialConfig {
    /// A port with DEC factory settings.
    pub fn new(device: impl Into<PathBuf>) -> SerialConfig {
        let line = Line::default();
        SerialConfig {
            device: device.into(),
            baud: line.baud,
            data_bits: line.data_bits,
            parity: line.parity,
            stop_bits: line.stop_bits,
            flow: line.flow,
        }
    }

    pub fn line(&self) -> Line {
        Line {
            baud: self.baud,
            data_bits: self.data_bits,
            parity: self.parity,
            stop_bits: self.stop_bits,
            flow: self.flow,
        }
    }

    pub fn set_line(&mut self, line: Line) {
        self.baud = line.baud;
        self.data_bits = line.data_bits;
        self.parity = line.parity;
        self.stop_bits = line.stop_bits;
        self.flow = line.flow;
    }
}

impl fmt::Display for SerialConfig {
    /// `/dev/ttyUSB0 9600 8N1`, plus the flow control when not XON/XOFF.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.device.display(), self.line())
    }
}

impl FromStr for Parity {
    type Err = String;
    /// picocom style: `n`, `e`, `o`, `m`, `s` (or the full word).
    fn from_str(s: &str) -> Result<Parity, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "n" | "none" => Parity::None,
            "e" | "even" => Parity::Even,
            "o" | "odd" => Parity::Odd,
            "m" | "mark" => Parity::Mark,
            "s" | "space" => Parity::Space,
            _ => return Err(format!("unknown parity {s:?} (use n, e, o, m or s)")),
        })
    }
}

impl FromStr for FlowControl {
    type Err = String;
    /// picocom style: `n` none, `x` XON/XOFF, `h` RTS/CTS hardware; and
    /// `xt` and `xr` for XON/XOFF one way only, as Set-Up can choose.
    fn from_str(s: &str) -> Result<FlowControl, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "n" | "none" => FlowControl::None,
            "x" | "xon" | "xonxoff" | "xoff" => FlowControl::XonXoff,
            "xt" => FlowControl::XonXoffTransmit,
            "xr" => FlowControl::XonXoffReceive,
            "h" | "rtscts" | "hardware" => FlowControl::RtsCts,
            _ => return Err(format!("unknown flow control {s:?} (use n, x or h)")),
        })
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_picocom_style_options() {
        assert_eq!("e".parse::<Parity>(), Ok(Parity::Even));
        assert_eq!("n".parse::<FlowControl>(), Ok(FlowControl::None));
        assert_eq!("h".parse::<FlowControl>(), Ok(FlowControl::RtsCts));
        assert!("q".parse::<Parity>().is_err());
    }
}
