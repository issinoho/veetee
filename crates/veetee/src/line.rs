//! The serial line as Communications Set-Up describes it, and back
//! (docs/serial-setup.md). On a DEC terminal Set-Up is where the line is set:
//! leaving it puts the speed, data format and flow control on the line, and
//! on the VT500 series the host can set them too (DECSCS, DECSPP, DECSFC).

use vt_core::Model;
use vt_core::setup::{Features, Parity as SetupParity};
use vt_transport::serial::{FlowControl, Line, Parity};

/// The line that Set-Up's `features` describe.
///
/// The receive speed and the XOFF threshold are not part of it: few ports
/// can receive at one speed and send at another, and the port's driver, not
/// the terminal, decides when its buffer is full enough to send XOFF.
pub fn of(model: Model, features: &Features) -> Line {
    let (transmit, receive) = flows(model, features);
    let flow = if (transmit | receive) & 2 != 0 {
        // DSR and DTR flow control: a PC's serial port and USB adapters
        // carry RTS and CTS, and Linux has no other hardware flow control.
        FlowControl::RtsCts
    } else {
        match (transmit & 1 != 0, receive & 1 != 0) {
            (true, true) => FlowControl::XonXoff,
            (true, false) => FlowControl::XonXoffTransmit,
            (false, true) => FlowControl::XonXoffReceive,
            (false, false) => FlowControl::None,
        }
    };
    Line {
        baud: features.transmit_speed,
        data_bits: if features.seven_bit_data { 7 } else { 8 },
        parity: match features.parity {
            SetupParity::None => Parity::None,
            // Unchecked parity is sent all the same.
            SetupParity::Even | SetupParity::EvenUnchecked => Parity::Even,
            SetupParity::Odd | SetupParity::OddUnchecked => Parity::Odd,
            SetupParity::Mark => Parity::Mark,
            SetupParity::Space => Parity::Space,
        },
        stop_bits: if features.two_stop_bits { 2 } else { 1 },
        flow,
    }
}

/// Transmit and receive flow control as the VT500 codes them: 0 none,
/// 1 XON/XOFF, 2 DSR or DTR, 3 both. A VT420 has no choice of transmit flow
/// control — it always stops at the host's XOFF — and chooses receive flow
/// control with XOFF at 64 or 128, or No XOFF (Communications Set-Up).
fn flows(model: Model, features: &Features) -> (u8, u8) {
    if model.max_level() >= 5 {
        (features.transmit_flow, features.receive_flow)
    } else {
        (features.transmit_flow, u8::from(features.xoff.is_some()))
    }
}

/// Puts `line` into `features`, so that Set-Up shows what the line is doing.
/// What Set-Up cannot show — a speed not in its list, 5 or 6 data bits —
/// comes as near as it can; the line keeps the real value.
pub fn put(line: &Line, features: &mut Features) {
    features.transmit_speed = line.baud;
    features.seven_bit_data = line.data_bits < 8;
    features.parity = match line.parity {
        Parity::None => SetupParity::None,
        Parity::Even => SetupParity::Even,
        Parity::Odd => SetupParity::Odd,
        Parity::Mark => SetupParity::Mark,
        Parity::Space => SetupParity::Space,
    };
    features.two_stop_bits = line.stop_bits == 2;
    let (transmit, receive) = match line.flow {
        FlowControl::None => (0, 0),
        FlowControl::XonXoff => (1, 1),
        FlowControl::XonXoffTransmit => (1, 0),
        FlowControl::XonXoffReceive => (0, 1),
        FlowControl::RtsCts => (2, 2),
    };
    features.transmit_flow = transmit;
    features.receive_flow = receive;
    features.xoff = if receive & 1 != 0 {
        features.xoff.or(Some(64))
    } else {
        None
    };
}

/// The line to set when Set-Up's line has gone from `before` to `after`:
/// `current` with what changed in Set-Up, and only that. A setting Set-Up
/// cannot show, given on the command line, survives a change to another.
pub fn changed(before: &Line, after: &Line, current: &Line) -> Line {
    fn pick<T: PartialEq + Copy>(before: T, after: T, current: T) -> T {
        if before == after { current } else { after }
    }
    Line {
        baud: pick(before.baud, after.baud, current.baud),
        data_bits: pick(before.data_bits, after.data_bits, current.data_bits),
        parity: pick(before.parity, after.parity, current.parity),
        stop_bits: pick(before.stop_bits, after.stop_bits, current.stop_bits),
        flow: pick(before.flow, after.flow, current.flow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_set_up_is_the_factory_line() {
        for model in [Model::Vt420, Model::Vt520] {
            assert_eq!(of(model, &Features::factory(model)), Line::default());
        }
    }

    #[test]
    fn set_up_describes_the_line() {
        let mut f = Features::factory(Model::Vt420);
        f.transmit_speed = 19200;
        f.seven_bit_data = true;
        f.parity = SetupParity::EvenUnchecked;
        f.two_stop_bits = true;
        f.xoff = None;
        assert_eq!(
            of(Model::Vt420, &f),
            Line {
                baud: 19200,
                data_bits: 7,
                parity: Parity::Even,
                stop_bits: 2,
                // No XOFF: the VT420 still stops at the host's.
                flow: FlowControl::XonXoffTransmit,
            }
        );
    }

    #[test]
    fn vt500_flow_control_both_ways() {
        let model = Model::Vt520;
        let mut f = Features::factory(model);
        f.transmit_flow = 0;
        f.receive_flow = 1;
        assert_eq!(of(model, &f).flow, FlowControl::XonXoffReceive);
        f.transmit_flow = 2;
        assert_eq!(of(model, &f).flow, FlowControl::RtsCts, "DSR");
        f.transmit_flow = 0;
        f.receive_flow = 0;
        assert_eq!(of(model, &f).flow, FlowControl::None);
    }

    #[test]
    fn a_line_put_into_set_up_reads_back() {
        let lines = [
            Line::default(),
            Line {
                baud: 1200,
                data_bits: 7,
                parity: Parity::Odd,
                stop_bits: 2,
                flow: FlowControl::None,
            },
            Line {
                flow: FlowControl::XonXoffTransmit,
                ..Line::default()
            },
            Line {
                flow: FlowControl::RtsCts,
                parity: Parity::Mark,
                ..Line::default()
            },
        ];
        for model in [Model::Vt420, Model::Vt520] {
            for line in lines {
                let mut f = Features::factory(model);
                put(&line, &mut f);
                assert_eq!(of(model, &f), line, "{model:?}");
            }
        }
    }

    #[test]
    fn only_what_changed_in_set_up_changes_the_line() {
        // 250000 baud from the command line, which Set-Up shows as 9600.
        let current = Line {
            baud: 250_000,
            ..Line::default()
        };
        let before = Line::default();
        let after = Line {
            parity: Parity::Even,
            ..before
        };
        assert_eq!(
            changed(&before, &after, &current),
            Line {
                baud: 250_000,
                parity: Parity::Even,
                ..Line::default()
            }
        );
        let faster = Line {
            baud: 19200,
            ..before
        };
        assert_eq!(changed(&before, &faster, &current).baud, 19200);
    }
}
