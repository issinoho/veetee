//! The veetee terminal model: a DEC VT terminal (VT52 through VT525) with no
//! dependency on any user interface.
//!
//! Feed host output to [`Terminal::advance`], send [`Terminal::take_output`]
//! back to the host, and render from [`Terminal::grid`] and
//! [`Terminal::cursor`]. Behaviour follows DEC STD 070 and the DEC programmer
//! reference manuals; defaults are DEC factory Set-Up values.

pub mod cell;
pub mod charset;
mod charset_tables;
pub mod color;
mod config;
pub mod dump;
pub mod grid;
mod keyboard;
pub mod keyprog;
mod modes;
pub mod recording;
mod selection;
pub mod softfont;
mod terminal;
pub mod udk;

pub use config::{Config, Extensions, Model, StatusDisplay, Supplemental};
pub use keyboard::{Key, KeyMods};
pub use modes::Modes;
pub use selection::{Point, Selection};
pub use terminal::{Cursor, CursorStyle, Event, KeyOutcome, LocalKeyAction, Terminal};
