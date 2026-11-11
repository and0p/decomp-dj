//! A log of what dj says while it runs: warnings and notes from the library, the cores' own
//! log messages (libretro's log interface), what worker processes print, and the GUI's status
//! messages. dj-gui shows it in its debug console (View > Debug console); everything still goes to
//! stderr as before, for the command line.
//!
//! The last `CAP` lines are kept, in memory only.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

/// How much a line matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// One line of the log.
#[derive(Clone, Debug)]
pub struct Line {
    /// Its number, counting from 0 for the first line logged.
    pub seq: u64,
    /// Seconds since the log started.
    pub at: f64,
    pub level: Level,
    /// Where it came from: "dj", "core", "dj-gui", a worker ("compare", "decode").
    pub source: String,
    pub text: String,
}

const CAP: usize = 5000;

struct Log {
    lines: VecDeque<Line>,
    next: u64,
    start: Option<Instant>,
}

static LOG: Mutex<Log> = Mutex::new(Log { lines: VecDeque::new(), next: 0, start: None });

/// Adds a line (or several, one per line of `text`).
pub fn push(level: Level, source: &str, text: &str) {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let start = *log.start.get_or_insert_with(Instant::now);
    for t in text.trim_end().lines() {
        let seq = log.next;
        log.next += 1;
        log.lines.push_back(Line { seq, at: start.elapsed().as_secs_f64(), level, source: source.to_string(), text: t.to_string() });
        if log.lines.len() > CAP {
            log.lines.pop_front();
        }
    }
}

/// A warning: written to stderr as `warning: ...` and logged.
pub fn warn(source: &str, text: &str) {
    eprintln!("warning: {text}");
    push(Level::Warn, source, text);
}

/// A note: written to stderr as `note: ...` and logged.
pub fn note(source: &str, text: &str) {
    eprintln!("note: {text}");
    push(Level::Info, source, text);
}

/// A line some other program printed (a worker's stderr): its level read from how it starts.
pub fn push_printed(source: &str, text: &str) {
    let t = text.trim_start();
    let level = if t.starts_with("error") || t.contains(" error:") {
        Level::Error
    } else if t.starts_with("warning") || t.contains(" warning:") {
        Level::Warn
    } else {
        Level::Info
    };
    push(level, source, text);
}

/// The lines kept, oldest first.
pub fn lines() -> Vec<Line> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).lines.iter().cloned().collect()
}

/// How many lines have been logged so far (it grows when one is).
pub fn count() -> u64 {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).next
}

/// Forgets the lines kept so far.
pub fn clear() {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).lines.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_kept_with_their_level_and_source() {
        let before = count();
        push(Level::Info, "dj-gui", "opened\nsecond line");
        push_printed("compare", "warning: core does not declare these options: x");
        push_printed("compare", "core error: oops");
        assert_eq!(count(), before + 4);
        let got: Vec<(Level, String, String)> = lines().into_iter().filter(|l| l.seq >= before).map(|l| (l.level, l.source, l.text)).collect();
        assert_eq!(got[0], (Level::Info, "dj-gui".into(), "opened".into()));
        assert_eq!(got[1].2, "second line");
        assert_eq!(got[2].0, Level::Warn);
        assert_eq!(got[3].0, Level::Error);
    }
}
