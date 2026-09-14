/// Phosphor colours of DEC monochrome monitors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phosphor {
    /// P4 white (VT220, VT320, VT420 standard monitors).
    #[default]
    White,
    /// P1 green.
    Green,
    /// P3 amber.
    Amber,
}

/// Optional CRT effects, chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    /// A soft halo of light around lit dots.
    pub glow: bool,
    /// Lit phosphor fades out rather than switching off at once.
    pub afterglow: bool,
    /// The picture bends like the face of a tube, darker at the corners.
    pub curvature: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            glow: true,
            afterglow: true,
            curvature: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Area outside the page.
    pub bezel: [f32; 3],
    /// Unlit screen.
    pub background: [f32; 3],
    /// Fully lit phosphor (bold).
    pub foreground: [f32; 3],
    /// Brightness of normal-intensity text relative to bold (DEC bold is brighter, not heavier).
    pub normal_intensity: f32,
    /// Darkening between scan lines, 0 = none.
    pub scanlines: f32,
    /// Stretch each dot by half a dot to the right.
    pub dot_stretch: bool,
    /// Strength of the glow around lit dots, 0 = none.
    pub glow: f32,
    /// Phosphor decay time constant in milliseconds, 0 = none.
    pub afterglow_ms: f32,
    /// Barrel distortion of the picture, 0 = flat.
    pub curvature: f32,
}

impl Theme {
    /// A phosphor with the default effects.
    pub fn phosphor(p: Phosphor) -> Theme {
        Theme::with_effects(p, Effects::default())
    }

    pub fn with_effects(p: Phosphor, effects: Effects) -> Theme {
        // P4 is a short-persistence phosphor, P1 medium and P3 long; the
        // visible afterglow is kept subtle.
        let persistence = match p {
            Phosphor::White => 25.0,
            Phosphor::Green => 45.0,
            Phosphor::Amber => 90.0,
        };
        let foreground = match p {
            Phosphor::White => [0.92, 0.94, 0.96],
            Phosphor::Green => [0.35, 1.00, 0.45],
            Phosphor::Amber => [1.00, 0.70, 0.18],
        };
        Theme {
            bezel: [0.055, 0.058, 0.064],
            background: [0.018, 0.020, 0.024],
            foreground,
            normal_intensity: 0.78,
            scanlines: 0.30,
            dot_stretch: true,
            glow: if effects.glow { 0.35 } else { 0.0 },
            afterglow_ms: if effects.afterglow { persistence } else { 0.0 },
            curvature: if effects.curvature { 0.06 } else { 0.0 },
        }
    }

    /// True when the picture needs the post-processing pass.
    pub fn has_effects(&self) -> bool {
        self.glow > 0.0 || self.afterglow_ms > 0.0 || self.curvature > 0.0
    }

    /// Colour for an indexed or direct colour attribute.
    pub fn color(&self, color: vt_core::cell::Color, fallback: [f32; 3]) -> [f32; 3] {
        use vt_core::cell::Color;
        match color {
            Color::Default => fallback,
            Color::Indexed(i) => indexed(i),
            Color::Rgb(r, g, b) => [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            ],
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::phosphor(Phosphor::White)
    }
}

/// ANSI colour table (VT525 order), with the xterm 256-colour extension.
fn indexed(i: u8) -> [f32; 3] {
    const BASE: [[f32; 3]; 16] = [
        [0.00, 0.00, 0.00],
        [0.80, 0.14, 0.14],
        [0.18, 0.72, 0.22],
        [0.80, 0.72, 0.16],
        [0.20, 0.35, 0.85],
        [0.72, 0.25, 0.75],
        [0.18, 0.70, 0.75],
        [0.78, 0.78, 0.78],
        [0.40, 0.40, 0.40],
        [1.00, 0.35, 0.35],
        [0.40, 0.95, 0.40],
        [1.00, 0.95, 0.40],
        [0.45, 0.55, 1.00],
        [1.00, 0.45, 1.00],
        [0.40, 0.95, 1.00],
        [1.00, 1.00, 1.00],
    ];
    match i {
        0..=15 => BASE[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| {
                if v == 0 {
                    0.0
                } else {
                    (55.0 + 40.0 * f32::from(v)) / 255.0
                }
            };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        _ => {
            let v = (8.0 + 10.0 * f32::from(i - 232)) / 255.0;
            [v, v, v]
        }
    }
}
