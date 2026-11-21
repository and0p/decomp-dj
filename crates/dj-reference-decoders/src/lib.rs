//! decomp-dj's built-in decoders, exported as a decoder plugin (docs/decoders.md).
//!
//! The whole of a Rust plugin: decoders that implement `dj_decoder::Decoder` and `Default`,
//! and one `export_decoders!` line. Build it with `cargo build --release`; the library is
//! `target/release/libdj_reference_decoders.{so,dylib}` (`dj_reference_decoders.dll` on
//! Windows), for a project's `[[decoders]]`.
//!
//! The player's tests load it and check that it decodes exactly as the built-in decoders do.

use dj_decoder::isa::{Armv4t, Mips3, Mos6502};

dj_decoder::export_decoders!(Mos6502, Mips3, Armv4t);
