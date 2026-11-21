//! A note for the next launch when dj-gui crashes or stops responding inside a core
//! (.agents/_archive/ISOLATION_PLAN.md, step 4): which core, segment and frame, and which libretro call
//! it was in. A comparison core runs in the same process, so the note also names it and the
//! call it was in, if it was running. A `dj parity` child writes the same note, which its
//! supervisor reads to tell which of the two cores died (`isolate::supervise`).
//!
//! The crash handler (Unix: SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT) only writes the note
//! from buffers filled beforehand, with async-signal-safe calls, then lets the signal take
//! the process down as it would have. The watchdog is a thread that writes a "stopped
//! responding" note when the GUI's frame hasn't come round for 10 s, and removes it if the
//! GUI comes back.

use std::cell::UnsafeCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// `.local/crash-gui-<pid>.txt`.
pub fn note_path(root: &Path, pid: u32) -> PathBuf {
    root.join(".local").join(format!("crash-gui-{pid}.txt"))
}

/// `.local/crash-batch-<pid>.txt`: the note of a batch child (`dj parity`), for its supervisor.
pub fn batch_note_path(root: &Path, pid: u32) -> PathBuf {
    root.join(".local").join(format!("crash-batch-{pid}.txt"))
}

/// The line of a note that names the call a comparison core was in. It is there only when
/// the comparison core was running.
pub const COMPARE_IN: &str = "comparison core in: ";

/// From a note's text: the call the comparison core was in, if it was the one running.
pub fn compare_call(note: &str) -> Option<&str> {
    note.lines().find_map(|l| l.strip_prefix(COMPARE_IN)).map(str::trim).filter(|c| !c.is_empty())
}

const CAP: usize = 1024;

/// A byte buffer the crash handler can read: written by normal code (length set last), read
/// in the handler without locks. A write torn by the crash only garbles the note.
struct Buf {
    bytes: UnsafeCell<[u8; CAP]>,
    len: AtomicUsize,
}

unsafe impl Sync for Buf {}

impl Buf {
    const fn new() -> Buf {
        Buf { bytes: UnsafeCell::new([0; CAP]), len: AtomicUsize::new(0) }
    }

    fn set(&self, s: &[u8]) {
        let n = s.len().min(CAP - 1);
        self.len.store(0, Ordering::SeqCst);
        unsafe {
            let b = &mut *self.bytes.get();
            b[..n].copy_from_slice(&s[..n]);
            b[n] = 0; // NUL-terminated, for the path
        }
        self.len.store(n, Ordering::SeqCst);
    }

    fn get(&self) -> &[u8] {
        let n = self.len.load(Ordering::SeqCst);
        unsafe { &(&*self.bytes.get())[..n] }
    }
}

static NOTE: Buf = Buf::new();
static PROGRAM: Buf = Buf::new();
static CORE: Buf = Buf::new();
static COMPARE: Buf = Buf::new();
static SEGMENT: Buf = Buf::new();
static FRAME: AtomicU64 = AtomicU64::new(u64::MAX);
static CALL: AtomicUsize = AtomicUsize::new(0);
static COMPARE_CALL: AtomicUsize = AtomicUsize::new(0);

/// The calls the note can name ([`enter`], [`enter_compare`]): libretro's, then a comparison
/// core's own.
pub const CALLS: [&str; 11] = ["", "retro_run", "retro_serialize", "retro_unserialize", "retro_load_game", "retro_reset", "retro_init", "dj_parity_diffs", "dj_handle_event", "dj_handle_write", "dj_breakpoints"];
pub const CALL_DJ_PARITY_DIFFS: usize = 7;
pub const CALL_DJ_HANDLE_EVENT: usize = 8;
pub const CALL_DJ_HANDLE_WRITE: usize = 9;
pub const CALL_DJ_BREAKPOINTS: usize = 10;

/// Where to write the note (the project's `.local/crash-gui-<pid>.txt`); the handler is
/// installed the first time.
pub fn set_note_path(root: &Path) {
    set_note_file(&note_path(root, std::process::id()), "dj-gui");
}

/// A batch child's note (`.local/crash-batch-<pid>.txt`), for its supervisor.
pub fn set_batch_note_path(root: &Path) {
    let path = batch_note_path(root, std::process::id());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    set_note_file(&path, "dj");
}

fn set_note_file(path: &Path, program: &str) {
    PROGRAM.set(program.as_bytes());
    NOTE.set(path.as_os_str().as_encoded_bytes());
    #[cfg(unix)]
    {
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(unix::install);
    }
}

/// The comparison core loaded beside the baseline (its name and library), or "" for none.
pub fn set_compare(what: &str) {
    COMPARE.set(what.as_bytes());
    COMPARE_CALL.store(0, Ordering::Relaxed);
}

/// The comparison core is entering call `CALLS[i]`; 0 when it is back in dj.
pub fn enter_compare(call: usize) {
    COMPARE_CALL.store(call, Ordering::Relaxed);
}

/// Names the comparison core's call in progress, until dropped.
pub struct CompareCall;

impl CompareCall {
    pub fn enter(call: usize) -> CompareCall {
        enter_compare(call);
        CompareCall
    }
}

impl Drop for CompareCall {
    fn drop(&mut self) {
        enter_compare(0);
    }
}

/// The baseline core in use (its library path).
pub fn set_core(core: &str) {
    CORE.set(core.as_bytes());
}

/// Where playback is: segment `id`, about to run `frame`.
pub fn set_place(id: &str, frame: u64) {
    if SEGMENT.get() != id.as_bytes() {
        SEGMENT.set(id.as_bytes());
    }
    FRAME.store(frame, Ordering::Relaxed);
}

/// Entering libretro call `CALLS[i]`; 0 when back in dj.
pub fn enter(call: usize) {
    CALL.store(call, Ordering::Relaxed);
}

fn push_num(out: &mut [u8], at: &mut usize, mut n: u64) {
    let mut digits = [0u8; 20];
    let mut k = 0;
    loop {
        digits[k] = b'0' + (n % 10) as u8;
        n /= 10;
        k += 1;
        if n == 0 {
            break;
        }
    }
    while k > 0 && *at < out.len() {
        k -= 1;
        out[*at] = digits[k];
        *at += 1;
    }
}

fn push(out: &mut [u8], at: &mut usize, s: &[u8]) {
    let n = s.len().min(out.len() - *at);
    out[*at..*at + n].copy_from_slice(&s[..n]);
    *at += n;
}

/// The note's text, without allocating (the crash handler builds it too).
fn compose(out: &mut [u8], headline: &[u8], signal: Option<i32>) -> usize {
    let mut at = 0;
    push(out, &mut at, headline);
    if let Some(sig) = signal {
        push(out, &mut at, b" (signal ");
        push_num(out, &mut at, sig as u64);
        push(out, &mut at, b")");
    }
    push(out, &mut at, b"\ncore: ");
    push(out, &mut at, CORE.get());
    push(out, &mut at, b"\nsegment: ");
    push(out, &mut at, SEGMENT.get());
    let frame = FRAME.load(Ordering::Relaxed);
    if frame != u64::MAX {
        push(out, &mut at, b"\nframe: ");
        push_num(out, &mut at, frame);
    }
    let call = CALLS.get(CALL.load(Ordering::Relaxed)).copied().unwrap_or("");
    push(out, &mut at, b"\nin: ");
    push(out, &mut at, if call.is_empty() { b"dj itself, not a libretro call" } else { call.as_bytes() });
    if !COMPARE.get().is_empty() {
        push(out, &mut at, b"\ncomparison core: ");
        push(out, &mut at, COMPARE.get());
        // Only when it was running: then the fault is its, whatever the baseline was in.
        let call = CALLS.get(COMPARE_CALL.load(Ordering::Relaxed)).copied().unwrap_or("");
        if !call.is_empty() {
            push(out, &mut at, b"\n");
            push(out, &mut at, COMPARE_IN.as_bytes());
            push(out, &mut at, call.as_bytes());
        }
    }
    push(out, &mut at, b"\n");
    at
}

/// "dj-gui crashed", or "dj crashed" in a batch child.
fn headline(out: &mut [u8], what: &[u8]) -> usize {
    let mut at = 0;
    let program = PROGRAM.get();
    push(out, &mut at, if program.is_empty() { b"dj-gui" } else { program });
    push(out, &mut at, what);
    at
}

/// The note as it would be written now (for tests and the watchdog).
pub fn text(headline: &str) -> String {
    let mut buf = [0u8; 4 * CAP];
    let n = compose(&mut buf, headline.as_bytes(), None);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The GUI's frame came round: the watchdog's heartbeat.
pub fn heartbeat() {
    BEAT.store(epoch_ms(), Ordering::Relaxed);
}

static BEAT: AtomicU64 = AtomicU64::new(0);

fn epoch_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Starts the watchdog (once): after `after` without a heartbeat it writes a "stopped
/// responding" note, and removes it when heartbeats come back.
pub fn start_watchdog(after: Duration) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        heartbeat();
        std::thread::spawn(move || {
            let mut written: Option<PathBuf> = None;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let stalled = epoch_ms().saturating_sub(BEAT.load(Ordering::Relaxed)) > after.as_millis() as u64;
                let path = PathBuf::from(std::ffi::OsStr::new(&String::from_utf8_lossy(NOTE.get()).into_owned()));
                match (stalled, &written) {
                    (true, None) if !NOTE.get().is_empty() => {
                        let mut who = [0u8; 64];
                        let n = headline(&mut who, b"");
                        let _ = std::fs::write(&path, text(&format!("{} stopped responding for over {} s", String::from_utf8_lossy(&who[..n]), after.as_secs())));
                        written = Some(path);
                    }
                    (false, Some(p)) => {
                        let _ = std::fs::remove_file(p);
                        written = None;
                    }
                    _ => {}
                }
            }
        });
    });
}

#[cfg(unix)]
mod unix {
    use super::*;

    const SIGNALS: [libc::c_int; 5] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT];

    extern "C" fn handler(sig: libc::c_int) {
        let path = NOTE.get();
        if !path.is_empty() {
            let mut buf = [0u8; 4 * CAP];
            let mut head = [0u8; 64];
            let h = headline(&mut head, b" crashed");
            let n = compose(&mut buf, &head[..h], Some(sig));
            unsafe {
                // NOTE's buffer ends in a NUL (Buf::set).
                let fd = libc::open(path.as_ptr() as *const libc::c_char, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o644);
                if fd >= 0 {
                    libc::write(fd, buf.as_ptr() as *const libc::c_void, n);
                    libc::close(fd);
                }
            }
        }
        // SA_RESETHAND put the default action back: returning re-raises the fault (or abort()
        // raises again), and the process ends as it would have without this handler.
    }

    pub fn install() {
        for sig in SIGNALS {
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = handler as extern "C" fn(libc::c_int) as usize;
                sa.sa_flags = libc::SA_RESETHAND | libc::SA_NODEFER;
                libc::sigemptyset(&mut sa.sa_mask);
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_names_the_core_segment_frame_and_call() {
        set_core("cores/mesen_libretro.dylib");
        set_place("d0c14220-1332", 120);
        enter(1);
        let t = text("dj-gui crashed");
        assert!(t.contains("core: cores/mesen_libretro.dylib") && t.contains("segment: d0c14220-1332") && t.contains("frame: 120") && t.contains("in: retro_run"), "{t}");
        enter(0);
        assert!(text("x").contains("in: dj itself"));
        // A comparison core is named when one is loaded, and its call only while it runs.
        assert!(!text("x").contains("comparison core"));
        set_compare("decomp (cores/decomp_libretro.dylib)");
        let idle = text("x");
        assert!(idle.contains("comparison core: decomp (cores/decomp_libretro.dylib)") && compare_call(&idle).is_none(), "{idle}");
        {
            let _c = CompareCall::enter(CALL_DJ_HANDLE_EVENT);
            assert_eq!(compare_call(&text("x")), Some("dj_handle_event"));
        }
        assert!(compare_call(&text("x")).is_none());
        set_compare("");
        assert!(!text("x").contains("comparison core"));
    }
}
