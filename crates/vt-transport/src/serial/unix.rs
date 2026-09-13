//! Serial ports through termios.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::termios::{
    ControlModes, InputModes, OptionalActions, SpecialCodeIndex, Termios, ioctl_tiocexcl,
    tcgetattr, tcsendbreak, tcsetattr,
};

use super::{FlowControl, Parity, SerialConfig, invalid};

/// An open serial port.
#[derive(Debug)]
pub struct Serial {
    port: File,
    config: SerialConfig,
}

impl Serial {
    /// Opens and configures the port. The port is claimed for exclusive use.
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
            .custom_flags(libc_o_noctty())
            .open(&config.device)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    io::Error::new(
                        e.kind(),
                        format!(
                            "{}: permission denied (add your user to the 'dialout' group, then log in again)",
                            config.device.display()
                        ),
                    )
                } else {
                    io::Error::new(e.kind(), format!("{}: {e}", config.device.display()))
                }
            })?;
        let _ = ioctl_tiocexcl(&port);

        let mut t = tcgetattr(&port)?;
        apply_line_settings(&config, &mut t)?;
        tcsetattr(&port, OptionalActions::Now, &t)?;
        Ok(Serial { port, config })
    }

    pub fn config(&self) -> &SerialConfig {
        &self.config
    }
}

/// Sets raw mode and the line parameters in `t`.
fn apply_line_settings(config: &SerialConfig, t: &mut Termios) -> io::Result<()> {
    t.make_raw();
    t.set_speed(config.baud)
        .map_err(|_| invalid("unsupported baud rate"))?;
    let mut c = t.control_modes;
    c.remove(ControlModes::CSIZE | ControlModes::PARENB | ControlModes::PARODD);
    c.remove(ControlModes::CMSPAR | ControlModes::CSTOPB | ControlModes::CRTSCTS);
    c.insert(ControlModes::CREAD | ControlModes::CLOCAL);
    c.insert(match config.data_bits {
        5 => ControlModes::CS5,
        6 => ControlModes::CS6,
        7 => ControlModes::CS7,
        _ => ControlModes::CS8,
    });
    match config.parity {
        Parity::None => {}
        Parity::Even => c.insert(ControlModes::PARENB),
        Parity::Odd => c.insert(ControlModes::PARENB | ControlModes::PARODD),
        Parity::Mark => {
            c.insert(ControlModes::PARENB | ControlModes::PARODD | ControlModes::CMSPAR)
        }
        Parity::Space => c.insert(ControlModes::PARENB | ControlModes::CMSPAR),
    }
    if config.stop_bits == 2 {
        c.insert(ControlModes::CSTOPB);
    }
    let mut i = t.input_modes;
    i.remove(InputModes::IXON | InputModes::IXOFF | InputModes::IXANY);
    match config.flow {
        FlowControl::None => {}
        FlowControl::XonXoff => i.insert(InputModes::IXON | InputModes::IXOFF),
        FlowControl::RtsCts => c.insert(ControlModes::CRTSCTS),
    }
    t.control_modes = c;
    t.input_modes = i;
    t.special_codes[SpecialCodeIndex::VMIN] = 1;
    t.special_codes[SpecialCodeIndex::VTIME] = 0;
    Ok(())
}

/// `O_NOCTTY`, so opening a serial line never makes it our controlling terminal.
fn libc_o_noctty() -> i32 {
    rustix::fs::OFlags::NOCTTY.bits() as i32
}

impl crate::Transport for Serial {
    fn read_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let ts = Timespec {
            tv_sec: timeout.as_secs() as _,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        let mut fds = [PollFd::new(&self.port, PollFlags::IN)];
        if poll(&mut fds, Some(&ts))? == 0 {
            return Ok(0);
        }
        if fds[0].revents().intersects(PollFlags::HUP | PollFlags::ERR) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{} disconnected", self.config.device.display()),
            ));
        }
        match self.port.read(buf) {
            Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
            other => other,
        }
    }

    fn writer(&self) -> io::Result<Box<dyn crate::TransportWriter>> {
        Ok(Box::new(SerialWriter(self.port.try_clone()?)))
    }

    fn description(&self) -> String {
        self.config.to_string()
    }
}

struct SerialWriter(File);

impl Write for SerialWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl crate::TransportWriter for SerialWriter {
    fn send_break(&mut self) -> io::Result<()> {
        tcsendbreak(&self.0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transport;
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    use std::path::PathBuf;

    /// A PTY pair stands in for a null-modem cable: the slave is the "serial port".
    fn loopback() -> (File, PathBuf) {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        (File::from(master), PathBuf::from(name.to_str().unwrap()))
    }

    #[test]
    fn computes_line_settings() {
        // Linux PTYs force CS8 and no parity, so check the computed termios.
        let (_master, path) = loopback();
        let mut t = tcgetattr(File::open(&path).unwrap()).unwrap();
        let config = SerialConfig {
            baud: 19200,
            data_bits: 7,
            parity: Parity::Even,
            stop_bits: 2,
            flow: FlowControl::RtsCts,
            ..SerialConfig::new(&path)
        };
        apply_line_settings(&config, &mut t).unwrap();
        assert_eq!(t.input_speed(), 19200);
        let c = t.control_modes;
        assert!(c.contains(ControlModes::CS7 | ControlModes::PARENB | ControlModes::CSTOPB));
        assert!(c.contains(ControlModes::CRTSCTS | ControlModes::CREAD | ControlModes::CLOCAL));
        assert!(!c.contains(ControlModes::PARODD));
        assert!(!t.input_modes.contains(InputModes::IXON));
        assert_eq!(
            config.to_string(),
            format!("{} 19200 7E2 RTS/CTS", path.display())
        );
        apply_line_settings(
            &SerialConfig {
                parity: Parity::Mark,
                ..config
            },
            &mut t,
        )
        .unwrap();
        assert!(
            t.control_modes
                .contains(ControlModes::CMSPAR | ControlModes::PARODD)
        );
    }

    #[test]
    fn dec_defaults_use_xon_xoff() {
        let (_master, path) = loopback();
        let serial = Serial::open(SerialConfig::new(&path)).unwrap();
        let t = tcgetattr(&serial.port).unwrap();
        assert_eq!(t.input_speed(), 9600);
        assert!(t.input_modes.contains(InputModes::IXON | InputModes::IXOFF));
        assert!(t.control_modes.contains(ControlModes::CS8));
        assert_eq!(serial.description(), format!("{} 9600 8N1", path.display()));
    }

    #[test]
    fn data_flows_both_ways() {
        let (mut master, path) = loopback();
        let mut serial = Serial::open(SerialConfig {
            flow: FlowControl::None,
            ..SerialConfig::new(&path)
        })
        .unwrap();
        master.write_all(b"USERNAME: ").unwrap();
        let mut buf = [0u8; 64];
        let n = serial
            .read_timeout(&mut buf, Duration::from_secs(2))
            .unwrap();
        assert_eq!(&buf[..n], b"USERNAME: ");
        let mut writer = serial.writer().unwrap();
        writer.write_all(b"SYSTEM\r").unwrap();
        let mut got = [0u8; 16];
        let n = master.read(&mut got).unwrap();
        assert_eq!(&got[..n], b"SYSTEM\r");
    }

    #[test]
    fn missing_device_is_reported_with_its_path() {
        let err = Serial::open(SerialConfig::new("/dev/does-not-exist-veetee")).unwrap_err();
        assert!(err.to_string().contains("/dev/does-not-exist-veetee"));
    }
}
