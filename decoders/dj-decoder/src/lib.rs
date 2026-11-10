//! The instruction decoder interface of decomp-dj, shared by the player and by decoder
//! plugins (docs/decoders.md).
//!
//! A decoder answers one question: what is the instruction at these bytes? Its length, how
//! control leaves it (on, a jump or branch, a call, a return, a stop), its target and delay
//! slots, the mode the next instruction runs in, and any literal it reads. The player builds
//! instructions, blocks, successors and callers from those answers, the same way for every
//! instruction set.
//!
//! - [`Decoder`], [`Decoded`] and [`Kind`]: the interface, as the player uses it.
//! - [`isa`]: the decoders built into the player (6502, MIPS III, ARMv4T).
//! - [`ffi`]: the C interface plugins export (`decoders/include/dj_decoder.h`), and
//!   [`export_decoders!`], which exports Rust decoders through it.

pub mod ffi;
pub mod isa;

/// How control leaves an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// On to the next instruction.
    Next,
    /// To `target` (None: indirect, such as a jump table). A conditional jump is a branch.
    Jump,
    /// To `target` (None: indirect), coming back after the instruction and its delay slots.
    Call,
    /// Back to the caller.
    Return,
    /// Execution doesn't go on (BRK, KIL, BREAK).
    Stop,
}

/// One decoded instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Bytes. 0: not an instruction.
    pub len: u8,
    pub kind: Kind,
    /// Whether execution may also go on to the next instruction (a branch, a conditional
    /// return or call). Calls always come back, so this only matters for the others.
    pub conditional: bool,
    /// The CPU address control goes to, for jumps and calls whose target the bytes say.
    pub target: Option<u64>,
    /// Instructions that run before control moves (MIPS branches and jumps: 1).
    pub delay_slots: u8,
    /// The mode the next instruction runs in (65816 REP/SEP, ARM/Thumb); 0 where the
    /// instruction set has none.
    pub mode_after: u32,
    /// How a call site through this instruction is listed ("JSR", "JAL", ...), for jumps
    /// and calls.
    pub site: &'static str,
    /// The mode code runs in at `target`, when the jump changes it (ARM `bx pc` from Thumb).
    /// None: the same mode.
    pub target_mode: Option<u32>,
    /// Data the instruction reads at a CPU address its bytes give (a PC-relative literal on
    /// ARM): the address and how many bytes. The sweep doesn't read those bytes as code.
    pub data_ref: Option<(u64, u8)>,
}

impl Decoded {
    /// A plain instruction `len` bytes long (0: not an instruction): control goes on.
    pub fn next(len: u8) -> Decoded {
        Decoded { len, kind: Kind::Next, conditional: false, target: None, delay_slots: 0, mode_after: 0, site: "", target_mode: None, data_ref: None }
    }

    /// Whether this instruction ends a basic block (after its delay slots). Calls don't:
    /// control comes back.
    pub fn ends_block(&self) -> bool {
        !matches!(self.kind, Kind::Next | Kind::Call)
    }
}

/// An instruction set the player can split into instructions and blocks.
///
/// Decoders are pure: the same bytes, address and mode always give the same answer, with no
/// state between calls, and they may be called from several threads at once.
pub trait Decoder: Sync + Send {
    /// The name connectors report for it (`dj_region.cpu`): "6502", "mips3".
    fn cpu(&self) -> &'static str;
    /// Which decoder this is, for reports and caches: "built-in 6502", or a library's path
    /// and hash. Results from different decoders are never mixed up.
    fn id(&self) -> String {
        format!("built-in {}", self.cpu())
    }
    /// The most bytes one instruction takes; the sweep offers this many where it can.
    fn max_len(&self) -> usize;
    /// Every instruction this long, for fixed-width instruction sets. Lets callers skip
    /// decoding lengths.
    fn fixed_len(&self) -> Option<u8> {
        None
    }
    /// Instructions in `mode` start only at multiples of this many bytes (from the
    /// function's start).
    fn alignment(&self, _mode: u32) -> u8 {
        1
    }
    /// The mode an opcode ran in, from the connector's mode bits on its first byte
    /// (`0x04`-`0x20`, defined per instruction set in docs/connectors.md). None: the bits
    /// don't say (no mode bits, or more than one mode).
    fn mode_from_flags(&self, _flags: u8) -> Option<u32> {
        None
    }
    /// Decodes the instruction at the start of `bytes` (in the ROM file's order), which runs
    /// at CPU address `pc` in `mode`. `bytes` may be shorter than `max_len` at the end of the
    /// code; a length past it means the instruction doesn't fit.
    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded;
}
