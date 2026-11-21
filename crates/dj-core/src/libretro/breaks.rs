//! Breakpoints inside a frame: what is armed in each slot, the callback the connector
//! calls when one is hit, and the hook that hears about it.

use super::*;

/// A breakpoint hit more often than this in one frame is in a loop (a wait that spins, or
/// other code at that address): it is switched off until the frame ends, with a warning.
pub const BREAK_CAP: u32 = 1024;

/// dj's side of a slot's breakpoints. The connector only calls back; the counting is here,
/// so nothing that clears coverage flags touches it.
pub(super) struct Breaks {
    /// The project's `[tick]`: (region index, offset).
    pub(super) tick: Option<(i32, i64)>,
    pub(super) ticks: u64,
    /// Hits in the frame being run, per breakpoint.
    pub(super) hits: Vec<((i32, i64), u32)>,
    /// Breakpoints the cap switched off this frame.
    pub(super) capped: Vec<(i32, i64)>,
    /// A state is being saved or loaded: a core may run instructions meanwhile (Mupen64Plus
    /// does before a load), and those are not the game's play.
    pub(super) busy: bool,
    /// A comparison core's breakpoints in this core (`set_core_breakpoints`).
    pub(super) core_breaks: Vec<(i32, i64)>,
    /// `[WRITE]` lines of the frame being run that name an instruction: each is made the
    /// first time that instruction is about to run, then leaves this list.
    pub(super) writes: Vec<ArmedWrite>,
}

/// A `[WRITE]` line with an instruction address, waiting for that instruction.
#[derive(Clone)]
pub(super) struct ArmedWrite {
    pub(super) at: (i32, i64),
    /// The memory region written, as `[WRITE]` names it ("RAM", "BUS").
    pub(super) region: String,
    pub(super) address: u64,
    pub(super) bytes: Vec<u8>,
    /// The line, for the warning if its instruction doesn't run.
    pub(super) label: String,
}

impl Breaks {
    pub(super) const fn new() -> Breaks {
        Breaks { tick: None, ticks: 0, hits: Vec::new(), capped: Vec::new(), busy: false, core_breaks: Vec::new(), writes: Vec::new() }
    }

    /// Whether anything still wants to hear of `at`.
    pub(super) fn wants(&self, at: (i32, i64)) -> bool {
        self.tick == Some(at) || self.core_breaks.contains(&at) || self.writes.iter().any(|w| w.at == at)
    }
}

thread_local! {
    pub(super) static BREAKS: RefCell<[Breaks; SLOTS]> = const { RefCell::new([Breaks::new(), Breaks::new()]) };
    /// What to tell while the baseline is stopped (`set_frame_hook`).
    pub(super) static FRAME_HOOK: RefCell<Option<Box<dyn FnMut(Stop)>>> = const { RefCell::new(None) };
}

/// A moment at which the baseline is stopped and may be read: what a frame hook is told.
pub enum Stop<'a> {
    /// The frame's lines are applied and `retro_run` is about to be called.
    FrameStart,
    /// The baseline is inside `retro_run`, about to run the instruction at one of the
    /// comparison core's breakpoints (region index, offset).
    Break { region: i32, offset: i64 },
    /// The baseline is inside `retro_run`: a `[WRITE]` line that names this instruction has
    /// just been made.
    Write { at_region: i32, at_offset: i64, region: &'a str, address: u64, bytes: &'a [u8] },
}

/// Sets (or, with None, removes) the function told of each [`Stop`] of the baseline core.
/// It is called from inside the core's `retro_run`, on this thread. It may read the
/// baseline's memory and registers, and run the comparison core; it must not run, save,
/// load or reset the baseline.
pub fn set_frame_hook(hook: Option<Box<dyn FnMut(Stop)>>) {
    FRAME_HOOK.with(|h| *h.borrow_mut() = hook);
}

/// Tells the frame hook, if there is one. The hook is taken out while it runs, so it may
/// replace itself.
pub(super) fn tell_hook(stop: Stop) {
    let Some(mut hook) = FRAME_HOOK.with(|h| h.borrow_mut().take()) else { return };
    hook(stop);
    FRAME_HOOK.with(|h| {
        let mut h = h.borrow_mut();
        if h.is_none() {
            *h = Some(hook);
        }
    });
}

/// The address of byte `addr` of the emulated bus in slot `slot`, through the core's memory
/// maps.
pub(super) fn bus_ptr(slot: usize, addr: usize) -> Option<*mut u8> {
    with_host(slot, |h| {
        for d in &h.bus {
            if d.ptr == 0 {
                continue;
            }
            let hit = if d.select != 0 { (addr & d.select) == (d.start & d.select) } else { addr >= d.start && addr < d.start + d.len.max(1) };
            if !hit {
                continue;
            }
            let mut a = (addr - d.start) & !d.disconnect;
            if d.len > 0 {
                a %= d.len;
            }
            return Some((d.ptr + d.offset + a) as *mut u8);
        }
        None
    })
    .flatten()
}

/// Makes an armed write, with the core stopped inside its frame. False: the region isn't
/// there, or the write runs past its end.
pub(super) fn make_write(slot: usize, w: &ArmedWrite) -> bool {
    if w.region == "BUS" {
        let ptrs: Option<Vec<*mut u8>> = (0..w.bytes.len()).map(|i| bus_ptr(slot, w.address as usize + i)).collect();
        let Some(ptrs) = ptrs else { return false };
        for (p, b) in ptrs.iter().zip(&w.bytes) {
            unsafe { **p = *b };
        }
        return true;
    }
    let Some(id) = region_memory_id(&w.region) else { return false };
    let Some((ptr, len)) = with_host(slot, |h| h.memory).flatten().and_then(|(data, size)| unsafe {
        let p = data(id) as *mut u8;
        let n = size(id);
        (!p.is_null() && n > 0).then_some((p, n))
    }) else {
        return false;
    };
    let a = w.address as usize;
    if a + w.bytes.len() > len {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(w.bytes.as_ptr(), ptr.add(a), w.bytes.len()) };
    true
}

pub(super) unsafe extern "C" fn break_cb<const S: usize>(_user: *mut c_void, region: i32, offset: i64) -> i32 {
    let at = (region, offset);
    // Counted and sorted out with the state borrowed; the writes and the hook come after,
    // since the hook runs other code (the comparison core) that may come back here.
    let (off, writes, core_break) = BREAKS.with(|b| {
        let mut b = b.borrow_mut();
        let b = &mut b[S];
        if b.busy {
            return (0, vec![], false);
        }
        let n = match b.hits.iter_mut().find(|(k, _)| *k == at) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                b.hits.push((at, 1));
                1
            }
        };
        if n > BREAK_CAP {
            b.capped.push(at);
            crate::console::count_breakpoint_capped();
            let what = if b.tick == Some(at) { "the [tick] address" } else { "a breakpoint" };
            crate::console::warn("dj", &format!("{what} (region {region}, offset {offset:#X}) ran more than {BREAK_CAP} times in one frame, so it is in a loop: it is off until the frame ends, and those runs aren't counted"));
            return (1, vec![], false);
        }
        if b.tick == Some(at) {
            b.ticks += 1;
        }
        // The writes waiting for this instruction are made once, and leave the list.
        let mut writes = vec![];
        b.writes.retain(|w| {
            if w.at == at {
                writes.push(w.clone());
                false
            } else {
                true
            }
        });
        // Nothing left that wants this place: the connector may stop calling for it.
        ((!b.wants(at)) as i32, writes, b.core_breaks.contains(&at))
    });
    for w in &writes {
        if !make_write(S, w) {
            crate::console::count_write_not_made();
            crate::console::warn("dj", &format!("{}: the write couldn't be made (the region isn't there, or the write runs past its end)", w.label));
            continue;
        }
        if S == BASELINE {
            tell_hook(Stop::Write { at_region: region, at_offset: offset, region: &w.region, address: w.address, bytes: &w.bytes });
        }
    }
    if core_break && S == BASELINE {
        tell_hook(Stop::Break { region, offset });
    }
    off
}
