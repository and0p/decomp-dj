//! The C interface a decoder plugin exports, as in `decoders/include/dj_decoder.h`
//! (docs/decoders.md), and [`export_decoders!`](crate::export_decoders), which exports Rust
//! decoders through it.
//!
//! A plugin is a shared library (`.so`, `.dylib`, `.dll`) with one exported function,
//! `dj_decoders`, that hands out a table per instruction set. The tables and everything they
//! point to stay valid while the library is loaded; the functions in them are pure and may be
//! called from several threads at once.

use std::ffi::{c_char, c_int, CStr, CString};

use crate::{Decoded, Decoder, Kind};

/// The interface version in `dj_decoder.api_version`.
pub const DJ_DECODER_API: u32 = 1;

/// `enum dj_kind`.
pub const DJ_NEXT: u8 = 0;
pub const DJ_JUMP: u8 = 1;
pub const DJ_CALL: u8 = 2;
pub const DJ_RETURN: u8 = 3;
pub const DJ_STOP: u8 = 4;

/// `struct dj_instruction`: one decoded instruction. The player zeroes it before each call,
/// so a decoder only sets what applies.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DjInstruction {
    /// Bytes; 0: not an instruction.
    pub length: u8,
    /// `DJ_NEXT` ... `DJ_STOP`.
    pub kind: u8,
    /// 1: execution may also go on to the next instruction (a branch, a conditional return).
    pub conditional: u8,
    pub delay_slots: u8,
    /// 1: `target` holds the CPU address control goes to.
    pub has_target: u8,
    /// 1: `target_mode` holds the mode code runs in at `target`.
    pub has_target_mode: u8,
    /// Bytes of data the instruction reads at `data_ref`; 0: none.
    pub data_ref_length: u8,
    pub reserved: u8,
    pub mode_after: u32,
    pub target_mode: u32,
    pub target: u64,
    pub data_ref: u64,
    /// How a call site through this instruction is listed ("BL", "JSR"): a string that stays
    /// valid while the library is loaded, or NULL.
    pub site: *const c_char,
}

impl Default for DjInstruction {
    fn default() -> Self {
        DjInstruction { length: 0, kind: 0, conditional: 0, delay_slots: 0, has_target: 0, has_target_mode: 0, data_ref_length: 0, reserved: 0, mode_after: 0, target_mode: 0, target: 0, data_ref: 0, site: std::ptr::null() }
    }
}

impl DjInstruction {
    /// The C form of a decoded instruction (for exporting a Rust decoder).
    pub fn from_decoded(d: &Decoded) -> Self {
        DjInstruction {
            length: d.len,
            kind: match d.kind {
                Kind::Next => DJ_NEXT,
                Kind::Jump => DJ_JUMP,
                Kind::Call => DJ_CALL,
                Kind::Return => DJ_RETURN,
                Kind::Stop => DJ_STOP,
            },
            conditional: d.conditional as u8,
            delay_slots: d.delay_slots,
            has_target: d.target.is_some() as u8,
            has_target_mode: d.target_mode.is_some() as u8,
            data_ref_length: d.data_ref.map_or(0, |(_, n)| n),
            reserved: 0,
            mode_after: d.mode_after,
            target_mode: d.target_mode.unwrap_or(0),
            target: d.target.unwrap_or(0),
            data_ref: d.data_ref.map_or(0, |(a, _)| a),
            site: if d.site.is_empty() { std::ptr::null() } else { export_support::c_str(d.site) },
        }
    }

    /// Back to the player's form, with `site` already read; Err when a field is out of range.
    pub fn to_decoded(&self, site: &'static str) -> Result<Decoded, String> {
        let kind = match self.kind {
            DJ_NEXT => Kind::Next,
            DJ_JUMP => Kind::Jump,
            DJ_CALL => Kind::Call,
            DJ_RETURN => Kind::Return,
            DJ_STOP => Kind::Stop,
            k => return Err(format!("kind {k} isn't one of DJ_NEXT ... DJ_STOP")),
        };
        Ok(Decoded {
            len: self.length,
            kind,
            conditional: self.conditional != 0,
            target: (self.has_target != 0).then_some(self.target),
            delay_slots: self.delay_slots,
            mode_after: self.mode_after,
            site,
            target_mode: (self.has_target_mode != 0).then_some(self.target_mode),
            data_ref: (self.data_ref_length != 0).then_some((self.data_ref, self.data_ref_length)),
        })
    }
}

/// `struct dj_decoder`: one instruction set.
#[repr(C)]
#[derive(Debug)]
pub struct DjDecoder {
    /// [`DJ_DECODER_API`].
    pub api_version: u32,
    /// `sizeof(struct dj_decoder)` as the plugin was built: later versions add fields at the
    /// end, and the player reads only what's there.
    pub size: u32,
    /// The instruction set's name, as connectors report it (`dj_region.cpu`).
    pub cpu: *const c_char,
    /// The most bytes one instruction takes.
    pub max_length: u8,
    /// Every instruction this long; 0: lengths vary.
    pub fixed_length: u8,
    pub reserved: [u8; 6],
    /// Instructions in `mode` start at multiples of this many bytes. NULL: 1.
    pub alignment: Option<unsafe extern "C" fn(mode: u32) -> u8>,
    /// The mode an opcode ran in, from the connector's mode bits: 1 and `*mode` set when the
    /// bits say, else 0. NULL: never.
    pub mode_from_flags: Option<unsafe extern "C" fn(flags: u8, mode: *mut u32) -> c_int>,
    /// Decodes the instruction at `bytes` (`available` bytes there, maybe fewer than
    /// `max_length` at the end of the code), which runs at CPU address `pc` in `mode`.
    pub decode: Option<unsafe extern "C" fn(bytes: *const u8, available: usize, pc: u64, mode: u32, out: *mut DjInstruction)>,
}

// The tables are read-only and their functions are pure.
unsafe impl Sync for DjDecoder {}
unsafe impl Send for DjDecoder {}

/// The size of a version-1 table: everything up to and including `decode`.
pub const DJ_DECODER_V1_SIZE: usize = std::mem::size_of::<DjDecoder>();

/// `dj_decoders`: the function a plugin exports. Returns the plugin's tables and sets
/// `*count`.
pub type DjDecodersFn = unsafe extern "C" fn(count: *mut usize) -> *const *const DjDecoder;

/// The name of the exported function.
pub const DJ_DECODERS_SYMBOL: &[u8] = b"dj_decoders";

#[doc(hidden)]
pub mod export_support {
    //! What `export_decoders!` expands to calls; not for direct use.
    use super::*;
    use std::collections::HashMap;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{Mutex, OnceLock};

    /// A NUL-terminated copy of a static string, made once and kept for good.
    pub fn c_str(s: &'static str) -> *const c_char {
        static STRINGS: OnceLock<Mutex<HashMap<&'static str, &'static CStr>>> = OnceLock::new();
        let mut map = STRINGS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.entry(s).or_insert_with(|| Box::leak(CString::new(s.replace('\0', "")).unwrap().into_boxed_c_str())).as_ptr()
    }

    pub unsafe extern "C" fn decode<D: Decoder + Default>(bytes: *const u8, available: usize, pc: u64, mode: u32, out: *mut DjInstruction) {
        let d = catch_unwind(AssertUnwindSafe(|| {
            let bytes = if bytes.is_null() || available == 0 { &[][..] } else { std::slice::from_raw_parts(bytes, available) };
            D::default().decode(bytes, pc, mode)
        }))
        .unwrap_or(Decoded::next(0)); // a panic: not an instruction
        if !out.is_null() {
            *out = DjInstruction::from_decoded(&d);
        }
    }

    pub unsafe extern "C" fn alignment<D: Decoder + Default>(mode: u32) -> u8 {
        catch_unwind(|| D::default().alignment(mode)).unwrap_or(1)
    }

    pub unsafe extern "C" fn mode_from_flags<D: Decoder + Default>(flags: u8, mode: *mut u32) -> c_int {
        match catch_unwind(|| D::default().mode_from_flags(flags)) {
            Ok(Some(m)) if !mode.is_null() => {
                *mode = m;
                1
            }
            _ => 0,
        }
    }

    /// The table for decoder `D`, made once and kept for good.
    pub fn table<D: Decoder + Default + 'static>() -> *const DjDecoder {
        let d = D::default();
        Box::leak(Box::new(DjDecoder {
            api_version: DJ_DECODER_API,
            size: std::mem::size_of::<DjDecoder>() as u32,
            cpu: c_str(d.cpu()),
            max_length: d.max_len().min(255) as u8,
            fixed_length: d.fixed_len().unwrap_or(0),
            reserved: [0; 6],
            alignment: Some(alignment::<D>),
            mode_from_flags: Some(mode_from_flags::<D>),
            decode: Some(decode::<D>),
        }))
    }

    /// The tables a plugin hands out.
    pub struct Tables(pub Vec<*const DjDecoder>);
    unsafe impl Sync for Tables {}
    unsafe impl Send for Tables {}
}

/// Exports Rust decoders from a `cdylib` as a decomp-dj decoder plugin: defines the
/// `dj_decoders` function with a table for each type. Each type implements
/// [`Decoder`](crate::Decoder) and `Default` (decoders hold no state, so unit structs).
///
/// ```ignore
/// #[derive(Default)]
/// struct Wdc65816;
/// impl dj_decoder::Decoder for Wdc65816 { /* ... */ }
/// dj_decoder::export_decoders!(Wdc65816);
/// ```
#[macro_export]
macro_rules! export_decoders {
    ($($decoder:ty),+ $(,)?) => {
        /// # Safety
        /// `count` is NULL or points to a `size_t`.
        #[no_mangle]
        pub unsafe extern "C" fn dj_decoders(count: *mut usize) -> *const *const $crate::ffi::DjDecoder {
            static TABLES: ::std::sync::OnceLock<$crate::ffi::export_support::Tables> = ::std::sync::OnceLock::new();
            let tables = TABLES.get_or_init(|| $crate::ffi::export_support::Tables(vec![$($crate::ffi::export_support::table::<$decoder>()),+]));
            if !count.is_null() {
                *count = tables.0.len();
            }
            tables.0.as_ptr()
        }
    };
}
