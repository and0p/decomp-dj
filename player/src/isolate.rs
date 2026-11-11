//! Workers: native code dj doesn't trust (decoder plugins; cores, later) run in a child
//! process that dj watches, so a crash or a hang there can't take dj down
//! (.agents/ISOLATION_PLAN.md).
//!
//! A worker is the same program started again with a hidden first argument,
//! `__dj-worker <kind>`; `dj` and `dj-gui` both call [`run_if_worker`] before anything
//! else. `DJ_WORKER_EXE` names another program to start instead (tests point it at `dj`).
//!
//! One request per worker process: a JSON line on stdin, then the raw bytes its `bulk`
//! lengths announce. The worker answers with progress lines (`{"at": ...}`), then
//! `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`. The parent reads on a
//! thread and kills the worker at a deadline.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

/// The hidden first argument that starts a worker.
pub const WORKER_ARG: &str = "__dj-worker";

/// If this process was started as a worker, runs it and exits; otherwise returns at once.
pub fn run_if_worker() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(WORKER_ARG) {
        return;
    }
    let kind = args.next().unwrap_or_default();
    let code = match kind.as_str() {
        "decode" => serve(crate::decode::worker_request),
        "compare" => serve_session(crate::compare::CompareWorker::default(), crate::compare::CompareWorker::request),
        _ => {
            eprintln!("dj: unknown worker kind {kind:?}");
            2
        }
    };
    std::process::exit(code);
}

/// A request as a worker receives it.
pub struct Request {
    pub args: Value,
    /// The bytes announced in the request's `bulk`, in order.
    pub bulk: Vec<Vec<u8>>,
}

/// Lets a worker report progress (`{"at": ...}`): what it's working on, named in the
/// parent's message if the worker crashes or hangs there.
pub struct Progress<'a>(&'a mut dyn Write);

impl Progress<'_> {
    pub fn at(&mut self, what: Value) {
        let _ = writeln!(self.0, "{}", json!({ "at": what }));
        let _ = self.0.flush();
    }
}

/// Serves one request on stdin and stdout; the exit code.
fn serve(handler: fn(&str, Request, &mut Progress) -> Result<Value>) -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    if input.read_line(&mut line).unwrap_or(0) == 0 {
        return 2;
    }
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let reply = (|| -> Result<Value> {
        let req: Value = serde_json::from_str(&line).context("reading the request")?;
        let mut bulk = vec![];
        for n in req["bulk"].as_array().cloned().unwrap_or_default() {
            let mut buf = vec![0u8; n.as_u64().unwrap_or(0) as usize];
            input.read_exact(&mut buf).context("reading the request's bytes")?;
            bulk.push(buf);
        }
        let cmd = req["cmd"].as_str().unwrap_or_default().to_string();
        handler(&cmd, Request { args: req["args"].clone(), bulk }, &mut Progress(&mut out))
    })();
    let msg = match reply {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
    };
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
    0
}

/// Passes what a worker prints on stderr on to dj's stderr, and into the console log under
/// the worker's kind (a comparison core's log, a decoder plugin's messages).
fn forward_stderr(child: &mut std::process::Child, kind: &str) {
    let Some(err) = child.stderr.take() else { return };
    let kind = kind.to_string();
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines() {
            let Ok(line) = line else { break };
            eprintln!("{line}");
            crate::console::push_printed(&kind, &line);
        }
    });
}

/// Why a worker gave no answer.
#[derive(Debug)]
pub struct WorkerFailure {
    /// What happened: "crashed (signal 11, SIGSEGV)", "didn't finish within 10 s", or the
    /// error the worker reported.
    pub what: String,
    /// The worker's last progress report, if any: where it was.
    pub at: Option<Value>,
    /// True when the worker answered with an error, rather than crashing or hanging.
    pub reported: bool,
}

impl std::fmt::Display for WorkerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.what)
    }
}

/// The program workers run as.
fn worker_exe() -> Result<PathBuf> {
    match std::env::var_os("DJ_WORKER_EXE") {
        Some(p) => Ok(PathBuf::from(p)),
        None => std::env::current_exe().context("finding dj's own program to start a worker"),
    }
}

/// Scales worker deadlines: `DJ_WORKER_TIMEOUT_SCALE` (a number; tests use small ones).
pub fn timeout(base: Duration) -> Duration {
    let scale = std::env::var("DJ_WORKER_TIMEOUT_SCALE").ok().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0).unwrap_or(1.0);
    base.mul_f64(scale)
}

/// Runs one request in a new worker of `kind`, with `bulk` bytes after it, waiting at most
/// `deadline`. Err(Ok(failure)) when the worker crashed, hung or reported an error;
/// Err(Err(e)) when it couldn't be started.
pub fn call(kind: &str, cmd: &str, args: Value, bulk: &[&[u8]], deadline: Duration) -> std::result::Result<Value, std::result::Result<WorkerFailure, anyhow::Error>> {
    let exe = worker_exe().map_err(Err)?;
    let mut child = Command::new(&exe)
        .arg(WORKER_ARG)
        .arg(kind)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting a {kind} worker ({})", exe.display()))
        .map_err(Err)?;
    forward_stderr(&mut child, kind);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    // Write on a thread: a worker that dies early mustn't leave dj blocked on a full pipe.
    let header = json!({ "cmd": cmd, "args": args, "bulk": bulk.iter().map(|b| b.len()).collect::<Vec<_>>() }).to_string();
    let payload: Vec<Vec<u8>> = bulk.iter().map(|b| b.to_vec()).collect();
    let writer = std::thread::spawn(move || {
        let _ = writeln!(stdin, "{header}");
        for b in payload {
            if stdin.write_all(&b).is_err() {
                break;
            }
        }
        let _ = stdin.flush();
    });
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let end = Instant::now() + deadline;
    let mut at = None;
    let failure = loop {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if let Some(a) = v.get("at") {
                    at = Some(a.clone());
                } else if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    let _ = child.wait();
                    let _ = writer.join();
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                } else if v.get("ok").is_some() {
                    let _ = child.wait();
                    let what = v.get("error").and_then(Value::as_str).unwrap_or("the worker failed").to_string();
                    break WorkerFailure { what, at, reported: true };
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                break WorkerFailure { what: format!("didn't finish within {:.0} s", deadline.as_secs_f64().max(1.0)), at, reported: false };
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = child.wait().ok();
                break WorkerFailure { what: describe_exit(status), at, reported: false };
            }
        }
    };
    let _ = writer.join();
    Err(Ok(failure))
}

fn describe_exit(status: Option<std::process::ExitStatus>) -> String {
    let Some(status) = status else { return "ended without answering".into() };
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            let name = match sig {
                4 => ", SIGILL",
                6 => ", SIGABRT",
                7 | 10 => ", SIGBUS",
                8 => ", SIGFPE",
                11 => ", SIGSEGV",
                _ => "",
            };
            return format!("crashed (signal {sig}{name})");
        }
    }
    match status.code() {
        // Windows: an unhandled exception ends a process with its NTSTATUS code.
        Some(c) if (c as u32) >= 0xC000_0000 => {
            let name = match c as u32 {
                0xC000_0005 => ", access violation",
                0xC000_001D => ", illegal instruction",
                0xC000_0094 => ", integer division by zero",
                0xC000_00FD => ", stack overflow",
                0xC000_0409 => ", abort",
                _ => "",
            };
            format!("crashed (exception {:#010X}{name})", c as u32)
        }
        Some(c) => format!("ended without answering (exit code {c:#x})"),
        None => "ended without answering".into(),
    }
}

/// A worker failure as one line: "crashed (signal 11) decoding `on_a`".
pub fn failure_text(f: &WorkerFailure, at: impl Fn(&Value) -> Option<String>) -> String {
    match f.at.as_ref().and_then(at) {
        Some(where_) => format!("{} {where_}", f.what),
        None => f.what.clone(),
    }
}

/// Turns the outer error of [`call`] into an anyhow error, with the failure's text.
pub fn flatten(e: std::result::Result<WorkerFailure, anyhow::Error>) -> anyhow::Error {
    match e {
        Ok(f) => anyhow!("{f}"),
        Err(e) => e,
    }
}

// ---- long-lived workers -------------------------------------------------------------

/// What a long-lived worker's handler returns: the result, and bytes to send after it.
pub type Reply = Result<(Value, Vec<Vec<u8>>)>;

/// Serves requests until stdin closes, keeping `state` between them. Each reply is a JSON
/// line (`{"ok": ..., "result": ..., "bulk": [lengths]}`) and then those bytes.
fn serve_session<S>(mut state: S, handler: fn(&mut S, &str, Request) -> Reply) -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            return 0;
        }
        let reply = (|| -> Reply {
            let req: Value = serde_json::from_str(&line).context("reading the request")?;
            let mut bulk = vec![];
            for n in req["bulk"].as_array().cloned().unwrap_or_default() {
                let mut buf = vec![0u8; n.as_u64().unwrap_or(0) as usize];
                input.read_exact(&mut buf).context("reading the request's bytes")?;
                bulk.push(buf);
            }
            let cmd = req["cmd"].as_str().unwrap_or_default().to_string();
            handler(&mut state, &cmd, Request { args: req["args"].clone(), bulk })
        })();
        let (msg, bytes) = match reply {
            Ok((result, bytes)) => (json!({ "ok": true, "result": result, "bulk": bytes.iter().map(Vec::len).collect::<Vec<_>>() }), bytes),
            Err(e) => (json!({ "ok": false, "error": format!("{e:#}") }), vec![]),
        };
        let _ = writeln!(out, "{msg}");
        for b in &bytes {
            let _ = out.write_all(b);
        }
        if out.flush().is_err() {
            return 0;
        }
    }
}

/// A long-lived worker: one process serving requests one after another, killed when it
/// takes too long, and done for once it has crashed or hung (start another).
pub struct Worker {
    kind: String,
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    replies: mpsc::Receiver<std::result::Result<(Value, Vec<Vec<u8>>), String>>,
    /// Why it's gone: set once it crashed, hung or went away.
    dead: Option<String>,
}

impl Worker {
    pub fn spawn(kind: &str) -> Result<Worker> {
        let exe = worker_exe()?;
        let mut child = Command::new(&exe)
            .arg(WORKER_ARG)
            .arg(kind)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting a {kind} worker ({})", exe.display()))?;
        forward_stderr(&mut child, kind);
        let stdin = child.stdin.take();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, replies) = mpsc::channel();
        // Reads replies, with their bytes, as they come.
        std::thread::spawn(move || loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let msg = (|| -> std::result::Result<(Value, Vec<Vec<u8>>), String> {
                let v: Value = serde_json::from_str(&line).map_err(|e| format!("a garbled reply from the worker: {e}"))?;
                let mut bulk = vec![];
                for n in v["bulk"].as_array().cloned().unwrap_or_default() {
                    let mut buf = vec![0u8; n.as_u64().unwrap_or(0) as usize];
                    stdout.read_exact(&mut buf).map_err(|e| format!("reading the worker's reply: {e}"))?;
                    bulk.push(buf);
                }
                Ok((v, bulk))
            })();
            if tx.send(msg).is_err() {
                break;
            }
        });
        Ok(Worker { kind: kind.to_string(), child, stdin, replies, dead: None })
    }

    /// Why the worker is gone, once it is.
    pub fn dead(&self) -> Option<&str> {
        self.dead.as_deref()
    }

    /// Sends one request with `bulk` bytes and waits at most `deadline` for the answer.
    /// `what` says what the request does, for messages ("running a frame"). An Err is a
    /// sentence: the worker's own error, or that it crashed or hung (then it's dead).
    pub fn request(&mut self, cmd: &str, args: Value, bulk: &[&[u8]], deadline: Duration, what: &str) -> std::result::Result<(Value, Vec<Vec<u8>>), String> {
        if let Some(why) = &self.dead {
            return Err(format!("the {} worker has stopped ({why})", self.kind));
        }
        let header = json!({ "cmd": cmd, "args": args, "bulk": bulk.iter().map(|b| b.len()).collect::<Vec<_>>() });
        let sent = (|| -> std::io::Result<()> {
            let stdin = self.stdin.as_mut().ok_or(std::io::ErrorKind::BrokenPipe)?;
            writeln!(stdin, "{header}")?;
            for b in bulk {
                stdin.write_all(b)?;
            }
            stdin.flush()
        })();
        let answer = match sent {
            Err(_) => Err(mpsc::RecvTimeoutError::Disconnected),
            Ok(()) => self.replies.recv_timeout(deadline),
        };
        match answer {
            Ok(Ok((v, bulk))) if v["ok"].as_bool() == Some(true) => Ok((v["result"].clone(), bulk)),
            Ok(Ok((v, _))) => Err(v["error"].as_str().unwrap_or("the worker failed").to_string()),
            Ok(Err(garbled)) => Err(self.die(garbled)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                Err(self.die(format!("didn't finish {what} within {:.0} s", deadline.as_secs_f64().max(1.0))))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = self.child.wait().ok();
                Err(self.die(format!("{} {what}", describe_exit(status))))
            }
        }
    }

    fn die(&mut self, why: String) -> String {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.dead = Some(why.clone());
        why
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing stdin ends it; don't wait long for a worker that doesn't listen.
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---- batch commands ---------------------------------------------------------------------

/// Set in a batch child: the command runs here, reporting progress to its supervisor.
const BATCH_CHILD_ENV: &str = "DJ_BATCH_CHILD";
/// Segments the batch child leaves out (comma-separated ids): ones that crashed the core
/// or hung in an earlier child.
const BATCH_SKIP_ENV: &str = "DJ_BATCH_SKIP";
/// Runs a batch command in dj's own process, without a supervisor (for debugging).
const IN_PROCESS_ENV: &str = "DJ_IN_PROCESS";
/// How a batch child's progress lines start on stderr.
const PROGRESS_MARK: &str = "\u{1e}dj-progress ";

/// Whether this process is a batch child.
pub fn batch_child() -> bool {
    static CHILD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHILD.get_or_init(|| std::env::var_os(BATCH_CHILD_ENV).is_some())
}

fn batch_skip() -> &'static std::collections::HashSet<String> {
    static SKIP: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    SKIP.get_or_init(|| std::env::var(BATCH_SKIP_ENV).unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect())
}

/// Whether a batch child leaves segment `id` out: "crashed" (it crashed the core or hung in
/// an earlier child), "blocked" (one above it did), or None.
pub fn batch_skipped(col: &crate::collection::Collection, id: &str) -> Option<&'static str> {
    let skip = batch_skip();
    if skip.is_empty() {
        return None;
    }
    if skip.contains(id) {
        return Some("crashed");
    }
    col.chain(id).ok()?.iter().any(|a| skip.contains(*a)).then_some("blocked")
}

/// In a batch child: segment `id` is starting `frame`. Reported to the supervisor every
/// frame (a short line on stderr), so a crash or hang names its frame exactly.
pub fn batch_progress(id: &str, frame: u64) {
    if !batch_child() {
        return;
    }
    let stderr = std::io::stderr();
    let mut e = stderr.lock();
    let _ = writeln!(e, "{PROGRESS_MARK}{frame} {id}");
}

/// How long a batch child may go without progress: while it starts (loading the core, the
/// local database), and between reports.
const BATCH_START_TIMEOUT: Duration = Duration::from_secs(60);
const BATCH_HANG_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs this batch command (`dj run`, `clean`, `hydrate`) in a child `dj` that plays the
/// segments, so a core that crashes or hangs costs that segment, not the command
/// (.agents/ISOLATION_PLAN.md, step 3). A crash, or no progress for 30 s, ends the child;
/// the segment it was playing is reported, and a new child runs the command again without
/// it and the segments below it. Returns the exit code to end with, or None when this
/// process should run the command itself (it's the child, or `DJ_IN_PROCESS` is set).
pub fn supervise(dir: &std::path::Path) -> Result<Option<i32>> {
    if batch_child() || std::env::var_os(IN_PROCESS_ENV).is_some() {
        return Ok(None);
    }
    let exe = worker_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let titles = crate::collection::Collection::open(dir).ok();
    let title = |id: &str| titles.as_ref().and_then(|c| c.replays.get(id)).map(|r| r.title().to_string()).unwrap_or_default();
    let mut skip: Vec<String> = vec![];
    let mut failures: Vec<(String, u64, String)> = vec![];
    loop {
        let mut child = Command::new(&exe)
            .args(&args)
            .env(BATCH_CHILD_ENV, "1")
            .env(BATCH_SKIP_ENV, skip.join(","))
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting dj ({})", exe.display()))?;
        // Progress lines go to the channel; everything else the child says goes on to stderr.
        let (tx, rx) = mpsc::channel::<(String, u64)>();
        let stderr = child.stderr.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                match line.strip_prefix(PROGRESS_MARK) {
                    Some(p) => {
                        let (frame, id) = p.split_once(' ').unwrap_or(("0", p));
                        let _ = tx.send((id.to_string(), frame.parse().unwrap_or(0)));
                    }
                    None => eprintln!("{line}"),
                }
            }
        });
        let mut at: Option<(String, u64)> = None;
        let mut last = Instant::now();
        let hung = loop {
            if let Ok(Some(_)) = child.try_wait() {
                break None;
            }
            let limit = timeout(if at.is_none() { BATCH_START_TIMEOUT } else { BATCH_HANG_TIMEOUT });
            if last.elapsed() >= limit {
                let _ = child.kill();
                break Some(limit);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(p) => {
                    at = Some(p);
                    last = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let status = child.wait()?;
        let _ = reader.join();
        while let Ok(p) = rx.try_recv() {
            at = Some(p);
        }
        let what = match hung {
            Some(limit) => Some(format!("made no progress for {:.0} s", limit.as_secs_f64())),
            None if crashed(&status) => Some(describe_exit(Some(status))),
            None => None,
        };
        let Some(what) = what else {
            // Finished, or ended with its own error (already printed).
            if !failures.is_empty() {
                eprintln!("{} segment(s) crashed or hung the core, and were left out with the segments below them:", failures.len());
                for (id, frame, what) in &failures {
                    eprintln!("  {} {} (frame {frame}): the core {what}", &id[..8.min(id.len())], title(id));
                }
            }
            let code = status.code().unwrap_or(1);
            return Ok(Some(if code == 0 && !failures.is_empty() { 1 } else { code }));
        };
        let Some((id, frame)) = at.filter(|(id, _)| !id.is_empty()) else {
            bail!("dj {what} before playing any segment");
        };
        if skip.contains(&id) {
            bail!("dj {what} again in segment {} after leaving it out", &id[..8.min(id.len())]);
        }
        eprintln!("crashed  {} {} at frame {frame}: the core {what}; going on without it and the segments below it", &id[..8.min(id.len())], title(&id));
        failures.push((id.clone(), frame, what));
        skip.push(id);
    }
}

/// Whether a process ended by crashing rather than exiting.
fn crashed(status: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal().is_some() {
            return true;
        }
    }
    // Windows: an unhandled exception ends with an NTSTATUS error code (0xC0000005 and so on).
    status.code().is_some_and(|c| (c as u32) >= 0xC000_0000)
}
