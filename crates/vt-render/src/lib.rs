//! Renders a [`vt_core::Terminal`] the way a DEC video terminal draws it.
//!
//! * The page keeps the proportions of a VT screen (4:3 at 24 lines) and is
//!   letterboxed inside the window, so 132-column mode really narrows the
//!   characters instead of shrinking a font.
//! * Glyphs are dot matrices sampled in the fragment shader: dots are
//!   stretched by half a dot (as DEC's video circuits did), each dot row is
//!   two scan lines, and double-width/height lines double the dots.
//!
//! [`layout`] and [`build_instances`] are pure and unit-tested; [`Renderer`]
//! only uploads and draws.

mod gl;
mod scene;
mod theme;

pub use gl::Renderer;
pub use scene::{FrameState, Layout, build_instances, layout};
pub use theme::{Phosphor, Theme};
