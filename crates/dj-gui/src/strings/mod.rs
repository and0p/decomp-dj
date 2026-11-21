//! The text dj-gui shows, by name: `t::OPEN_PROJECT` rather than "Open project…" in the code
//! that draws it, so the words are in one place and can be translated.
//!
//! English (en.rs) is the only language and is used directly. To add one: write a file
//! with the same constants, and choose between them here.
//!
//! Text with values in it (`format!("Deleted {n} segments")`) is still where it is used:
//! it needs a function or a macro per message rather than a constant.

mod en;

pub(crate) use en::*;
