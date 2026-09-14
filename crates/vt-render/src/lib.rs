//! Renders a [`vt_core::Terminal`] the way a DEC video terminal draws it.
//!
//! * The page keeps the proportions of the model's screen (a VT220 is 4:3 at
//!   24 lines; a VT420 or VT500 is an 800×400 raster of 1:1.4 pixels) and is
//!   letterboxed inside the window, so 132-column mode really narrows the
//!   characters instead of shrinking a font.
//! * Each screen size is drawn with its own face (10×16 and 6×16 dots at 24
//!   lines on a VT420, 10 and 8 dots high at 36 and 48 lines; see
//!   [`vt_fonts::FontSet`]).
//! * Glyphs are dot matrices sampled in the fragment shader: dots are
//!   stretched by half a dot (as DEC's video circuits did), each dot row is
//!   two scan lines on a VT220 and one on a VT420, and double-width/height
//!   lines double the dots.
//!
//! [`layout`], [`page_layout`] and [`build_instances`] are pure and unit-tested; [`Renderer`]
//! only uploads and draws.

mod gl;
mod postfx;
mod scene;
mod theme;

pub use gl::Renderer;
pub use scene::{
    Clip, FrameState, Layout, ScrollFrame, SoftAtlas, build_instances, face_offset, family, layout,
    page_layout, page_rows,
};
pub use theme::{Effects, Phosphor, Theme};
