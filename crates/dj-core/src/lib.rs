//! decomp-dj: play trees of libretro input segments, check state hashes and measure ROM coverage.
//! The `dj` command-line tool and the GUI are both built on this library.

pub mod analysis;
pub mod arm;
pub mod annotations;
pub mod cache;
pub mod collection;
pub mod compare;
pub mod coverage;
pub mod decode;
pub mod edit;
pub mod format;
pub mod gl;
pub mod hydrate;
pub mod isolate;
pub mod libretro;
pub mod parity;
pub mod player;
pub mod query;
pub mod scaffold;
pub mod tas;
pub mod condition;
pub mod console;
pub mod controls;
pub mod crash;
pub mod recovery;
pub mod style;
