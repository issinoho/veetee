//! The terminal's sounds: the warning and margin bells (125 ms beeps), the
//! keyclick (a 2 ms beep) and DECPS notes (EK-VT520-RM section 2.17, DECPS).
//!
//! Sounds are synthesised as a soft square wave, like the keyboard beeper,
//! and mixed on an audio thread. Bells and clicks play at once; DECPS notes
//! queue and play one after another, as the VT520's sound buffer does.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vt_core::setup::Volume;

/// Pitch of the bell and keyclick: C6, the middle of the DECPS range.
// 🔎 The LK401/LK411 beeper pitch is not documented; C6 is a stand-in.
const BEEP_HZ: f32 = 1046.5;
const BELL_MS: u32 = 125;
const CLICK_MS: u32 = 2;

/// A sound to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Bell(Volume),
    Click(Volume),
    /// DECPS: volume 0–7, duration, note 1 (C5) … 25 (C7); note 0 is a rest.
    Note {
        volume: u8,
        duration_ms: u32,
        note: u8,
    },
}

fn gain(volume: Volume) -> f32 {
    match volume {
        Volume::Off => 0.0,
        Volume::Low => 0.08,
        Volume::High => 0.22,
    }
}

/// DECPS volume: 0 off, 1–3 low, 4–7 high.
fn note_gain(volume: u8) -> f32 {
    match volume {
        0 => 0.0,
        1..=3 => gain(Volume::Low),
        _ => gain(Volume::High),
    }
}

/// Equal-tempered frequency of DECPS note `n` (1 = C5).
pub fn note_hz(n: u8) -> f32 {
    523.25 * 2f32.powf(f32::from(n.saturating_sub(1)) / 12.0)
}

#[derive(Debug, Clone)]
struct Voice {
    hz: f32,
    gain: f32,
    phase: f32,
    elapsed: u32,
    length: u32,
}

impl Voice {
    fn new(hz: f32, gain: f32, ms: u32, rate: f32) -> Voice {
        Voice {
            hz,
            gain,
            phase: 0.0,
            elapsed: 0,
            length: ((ms as f32 / 1000.0) * rate).max(1.0) as u32,
        }
    }

    fn done(&self) -> bool {
        self.elapsed >= self.length
    }

    fn next(&mut self, rate: f32) -> f32 {
        // A square wave with its edges softened by the first harmonics only.
        let t = self.phase * std::f32::consts::TAU;
        let wave = t.sin() + (3.0 * t).sin() / 3.0 + (5.0 * t).sin() / 5.0;
        self.phase = (self.phase + self.hz / rate).fract();
        // 1 ms ramps at each end avoid clicks of their own.
        let ramp = (rate / 1000.0).max(1.0);
        let pos = self.elapsed as f32;
        let envelope = (pos / ramp).min(1.0).min((self.length as f32 - pos) / ramp);
        self.elapsed += 1;
        wave * self.gain * envelope.max(0.0)
    }
}

/// Mixes the sounds playing now and the queued DECPS notes.
#[derive(Debug)]
pub struct Mixer {
    rate: f32,
    voices: Vec<Voice>,
    notes: VecDeque<Voice>,
}

impl Mixer {
    pub fn new(rate: f32) -> Mixer {
        Mixer {
            rate,
            voices: Vec::new(),
            notes: VecDeque::new(),
        }
    }

    pub fn play(&mut self, sound: Sound) {
        let rate = self.rate;
        match sound {
            Sound::Bell(v) if v != Volume::Off => {
                self.voices
                    .push(Voice::new(BEEP_HZ, gain(v), BELL_MS, rate))
            }
            Sound::Click(v) if v != Volume::Off => {
                self.voices
                    .push(Voice::new(BEEP_HZ, gain(v), CLICK_MS, rate))
            }
            // The VT520 buffers 16 notes; later ones would wait for room.
            Sound::Note {
                volume,
                duration_ms,
                note,
            } if self.notes.len() < 64 => {
                let g = if note == 0 { 0.0 } else { note_gain(volume) };
                self.notes
                    .push_back(Voice::new(note_hz(note), g, duration_ms, rate));
            }
            _ => {}
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let rate = self.rate;
        let mut sum: f32 = self.voices.iter_mut().map(|v| v.next(rate)).sum();
        self.voices.retain(|v| !v.done());
        if let Some(note) = self.notes.front_mut() {
            sum += note.next(rate);
            if note.done() {
                self.notes.pop_front();
            }
        }
        sum.clamp(-1.0, 1.0)
    }

    pub fn idle(&self) -> bool {
        self.voices.is_empty() && self.notes.is_empty()
    }
}

/// The audio output, opened on first use.
struct Output {
    sounds: Mutex<Option<Sender<Sound>>>,
}

static OUTPUT: OnceLock<Output> = OnceLock::new();

/// Plays `sound`. Returns false when no audio output is available, so the
/// caller can fall back to the desktop's bell.
pub fn play(sound: Sound) -> bool {
    let output = OUTPUT.get_or_init(|| Output {
        sounds: Mutex::new(start()),
    });
    let sender = output.sounds.lock().unwrap_or_else(|e| e.into_inner());
    sender.as_ref().is_some_and(|tx| tx.send(sound).is_ok())
}

/// Opens the audio output ahead of the first sound.
pub fn warm_up() {
    OUTPUT.get_or_init(|| Output {
        sounds: Mutex::new(start()),
    });
}

/// Starts the audio thread, which owns the output stream.
fn start() -> Option<Sender<Sound>> {
    let (tx, rx) = channel();
    let (ready_tx, ready_rx) = channel();
    std::thread::Builder::new()
        .name("veetee-audio".into())
        .spawn(move || run(rx, ready_tx))
        .ok()?;
    match ready_rx.recv() {
        Ok(Ok(())) => Some(tx),
        Ok(Err(e)) => {
            eprintln!("veetee: no sound: {e}");
            None
        }
        Err(_) => None,
    }
}

fn run(rx: Receiver<Sound>, ready: Sender<Result<(), String>>) {
    let stream = match open_stream() {
        Ok((stream, mixer)) => {
            let _ = ready.send(Ok(()));
            (stream, mixer)
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let (_stream, mixer) = stream;
    while let Ok(sound) = rx.recv() {
        mixer.lock().unwrap_or_else(|e| e.into_inner()).play(sound);
    }
}

type SharedMixer = Arc<Mutex<Mixer>>;

fn open_stream() -> Result<(cpal::Stream, SharedMixer), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate as f32)));
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, mixer.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, mixer.clone()),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, mixer.clone()),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, mixer.clone()),
        other => return Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, mixer))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: SharedMixer,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                let mut mixer = mixer.lock().unwrap_or_else(|e| e.into_inner());
                for frame in data.chunks_mut(channels) {
                    let value = T::from_sample(if mixer.idle() {
                        0.0
                    } else {
                        mixer.next_sample()
                    });
                    frame.iter_mut().for_each(|s| *s = value);
                }
            },
            |e| eprintln!("veetee: audio: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(mixer: &mut Mixer, samples: usize) -> Vec<f32> {
        (0..samples).map(|_| mixer.next_sample()).collect()
    }

    #[test]
    fn notes_follow_the_decps_table() {
        assert!((note_hz(1) - 523.25).abs() < 0.01);
        assert!((note_hz(13) - 1046.5).abs() < 0.01);
        assert!((note_hz(25) - 2093.0).abs() < 0.1);
        // The manual's approximate D#5 and G#5.
        assert!((note_hz(4) - 632.0).abs() < 15.0);
        assert!((note_hz(9) - 847.0).abs() < 20.0);
    }

    #[test]
    fn bell_lasts_125_ms_and_click_2_ms() {
        let mut m = Mixer::new(8000.0);
        m.play(Sound::Bell(Volume::High));
        let bell = render(&mut m, 1100);
        assert!(bell[..990].iter().any(|s| s.abs() > 0.1));
        assert!(
            bell[1001..].iter().all(|s| *s == 0.0),
            "silent after 125 ms"
        );
        assert!(m.idle());
        m.play(Sound::Click(Volume::Low));
        render(&mut m, 16);
        assert!(m.idle(), "a click is 2 ms");
        m.play(Sound::Bell(Volume::Off));
        assert!(m.idle());
    }

    #[test]
    fn notes_play_one_after_another() {
        let mut m = Mixer::new(1000.0);
        for note in [1, 5, 8] {
            m.play(Sound::Note {
                volume: 7,
                duration_ms: 100,
                note,
            });
        }
        render(&mut m, 250);
        assert!(!m.idle(), "the third note is still to play");
        render(&mut m, 60);
        assert!(m.idle());
    }
}
