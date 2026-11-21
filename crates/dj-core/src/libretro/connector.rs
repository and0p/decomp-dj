//! The decomp-dj connector a core may export (docs/connectors.md): coverage flags and
//! regions, the tick count, and the trace of instructions and memory accesses.

use super::*;

pub struct Connector {
    pub(super) region_count: unsafe extern "C" fn() -> usize,
    pub(super) get_region: unsafe extern "C" fn(usize, *mut DjRegionC) -> bool,
    pub(super) clear: unsafe extern "C" fn(),
    pub(super) set_breakpoints: unsafe extern "C" fn(*const DjBreakC, usize, Option<BreakFn>, *mut c_void),
    /// `dj_read_registers` (optional).
    pub(super) registers: Option<unsafe extern "C" fn(*mut c_void, usize) -> usize>,
    /// The optional trace part of the API (dj_trace_*).
    pub(super) trace: Option<TraceFns>,
}

pub(super) struct TraceFns {
    pub(super) start: unsafe extern "C" fn(usize),
    pub(super) stop: unsafe extern "C" fn(),
    pub(super) read: unsafe extern "C" fn(*mut *const TraceEvent, *mut u64) -> usize,
}

pub const TRACE_EXEC: u8 = 1;
pub const TRACE_READ: u8 = 2;
pub const TRACE_WRITE: u8 = 3;

/// One traced instruction or memory access (`struct dj_trace_event`, docs/connectors.md).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TraceEvent {
    pub cycle: u64,
    /// Offset into `region`, or -1.
    pub offset: i64,
    /// CPU address.
    pub addr: u16,
    pub kind: u8,
    /// Region index as in the coverage regions, or -1.
    pub region: i8,
    pub value: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub operand_count: u8,
    pub operands: [u8; 3],
    /// The CPU address's high 16 bits, on a CPU with 32-bit addresses (0 on the 6502).
    pub addr_high: u16,
    pub reserved: [u8; 2],
}

impl TraceEvent {
    /// The whole CPU address.
    pub fn address(&self) -> u32 {
        (self.addr_high as u32) << 16 | self.addr as u32
    }
}

pub struct CoverageRegion {
    pub name: String,
    pub flags: Vec<u8>,
}

impl Core {
    pub fn coverage_clear(&mut self) {
        if let Some(c) = &self.connector {
            unsafe { (c.clear)() }
        }
    }

    pub fn coverage_regions(&mut self) -> Vec<CoverageRegion> {
        let Some(c) = &self.connector else { return vec![] };
        let mut out = vec![];
        unsafe {
            for i in 0..(c.region_count)() {
                let mut r = DjRegionC::empty();
                if !(c.get_region)(i, &mut r) {
                    continue;
                }
                let name = if r.name.is_null() { format!("R{i}") } else { CStr::from_ptr(r.name).to_string_lossy().into_owned() };
                let flags = if r.flags.is_null() { vec![] } else { std::slice::from_raw_parts(r.flags, r.size).to_vec() };
                out.push(CoverageRegion { name, flags });
            }
        }
        out
    }

    /// What the connector reports about each region (docs/connectors.md), without the flags.
    pub fn region_facts(&mut self) -> Vec<crate::analysis::RegionFacts> {
        let Some(c) = &self.connector else { return vec![] };
        let mut out = vec![];
        unsafe {
            for i in 0..(c.region_count)() {
                let mut r = DjRegionC::empty();
                if !(c.get_region)(i, &mut r) {
                    continue;
                }
                let s = |p: *const c_char| (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned());
                out.push(crate::analysis::RegionFacts {
                    name: s(r.name).unwrap_or_else(|| format!("R{i}")),
                    size: r.size,
                    cpu: s(r.cpu),
                    has_data: !r.data.is_null(),
                    file_offset: u64::try_from(r.file_offset).ok(),
                });
            }
        }
        out
    }

    /// The bytes of region `i` as the connector hands them out (the CPU's byte order).
    pub fn region_data(&mut self, i: usize) -> Option<&[u8]> {
        let c = self.connector.as_ref()?;
        unsafe {
            let mut r = DjRegionC::empty();
            if !(c.get_region)(i as _, &mut r) || r.data.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(r.data, r.size))
        }
    }

    /// The index of the region coverage rolls up into: the first one from the ROM file
    /// whose instruction set has a decoder (`analysis::code_region`).
    pub fn code_region_index(&mut self) -> Option<usize> {
        let facts = self.region_facts();
        let name = crate::analysis::code_region(&facts);
        facts.iter().position(|f| f.name == name)
    }

    /// The connector's live flags for region `i`, without copying (for per-frame scans).
    pub fn coverage_slice(&mut self, i: usize) -> Option<&[u8]> {
        let c = self.connector.as_ref()?;
        unsafe {
            let mut r = DjRegionC::empty();
            if !(c.get_region)(i as _, &mut r) || r.flags.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(r.flags, r.size))
        }
    }

    pub fn coverage_region_index(&mut self, name: &str) -> Option<usize> {
        let Some(c) = &self.connector else { return None };
        unsafe {
            for i in 0..(c.region_count)() {
                let mut r = DjRegionC::empty();
                if (c.get_region)(i, &mut r) && !r.name.is_null() && CStr::from_ptr(r.name).to_string_lossy() == name {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Sets (or, with no region, turns off) the place whose runs count as logic ticks, and
    /// starts the count again. dj counts them itself, at a breakpoint.
    pub fn set_tick_exec(&mut self, region: Option<usize>, offset: u64) {
        BREAKS.with(|b| {
            let b = &mut b.borrow_mut()[self.slot];
            b.tick = region.map(|r| (r as i32, offset as i64));
            b.ticks = 0;
        });
        self.arm_breakpoints();
    }

    /// Starts the tick count again (a segment starts).
    pub fn tick_reset(&mut self) {
        BREAKS.with(|b| b.borrow_mut()[self.slot].ticks = 0);
    }

    pub fn can_trace(&self) -> bool {
        self.connector.as_ref().is_some_and(|c| c.trace.is_some())
    }

    /// Starts recording a trace of up to `capacity` events (instructions and data accesses).
    pub fn trace_start(&mut self, capacity: usize) -> Result<()> {
        let t = self.connector.as_ref().and_then(|c| c.trace.as_ref()).ok_or_else(|| anyhow!("this core's connector has no trace hook (dj_trace_start)"))?;
        unsafe { (t.start)(capacity) };
        Ok(())
    }

    /// Events recorded so far.
    pub fn trace_len(&self) -> usize {
        let Some(t) = self.connector.as_ref().and_then(|c| c.trace.as_ref()) else { return 0 };
        unsafe { (t.read)(std::ptr::null_mut(), std::ptr::null_mut()) }
    }

    /// Stops recording and returns the events, and how many were dropped past the capacity.
    pub fn trace_take(&mut self) -> (Vec<TraceEvent>, u64) {
        let Some(t) = self.connector.as_ref().and_then(|c| c.trace.as_ref()) else { return (vec![], 0) };
        unsafe {
            (t.stop)();
            let mut p: *const TraceEvent = std::ptr::null();
            let mut dropped = 0u64;
            let n = (t.read)(&mut p, &mut dropped);
            let v = if p.is_null() { vec![] } else { std::slice::from_raw_parts(p, n).to_vec() };
            (t.start)(0);
            (v, dropped)
        }
    }

    /// The connector's live flags for region `i`, writable: lets a caller clear a byte to see
    /// whether it runs again.
    pub fn coverage_slice_mut(&mut self, i: usize) -> Option<&mut [u8]> {
        let c = self.connector.as_ref()?;
        unsafe {
            let mut r = DjRegionC::empty();
            if !(c.get_region)(i as _, &mut r) || r.flags.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts_mut(r.flags, r.size))
        }
    }

    /// Ticks since the count was last started. Clearing coverage flags doesn't touch it.
    pub fn tick_count(&mut self) -> Option<u64> {
        self.connector.as_ref().map(|_| BREAKS.with(|b| b.borrow()[self.slot].ticks))
    }
}
