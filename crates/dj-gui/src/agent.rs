//! Working alongside agents (docs/mcp.md#the-gui): their drafts in the tree, following
//! their sessions, and sharing what the person is looking at. All through files in
//! `.local/`, so `dj mcp` and the GUI need no connection between them.

use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// An agent session, as `dj mcp` publishes it.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSession {
    pub session: String,
    pub draft: String,
    pub frame: u64,
    pub frames: u64,
    pub pid: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Modification times and sizes of the draft files, to notice changes cheaply.
pub fn drafts_fingerprint(root: &Path) -> u128 {
    dir_fingerprint(&root.join(".local").join("drafts"))
}

/// The same for segments/, to notice segment files that other tools changed.
pub fn segments_fingerprint(root: &Path) -> u128 {
    dir_fingerprint(&root.join("segments"))
}

fn dir_fingerprint(dir: &Path) -> u128 {
    let mut h: u128 = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut v: Vec<(String, u128)> = rd
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
                Some((e.file_name().to_string_lossy().into_owned(), t ^ m.len() as u128))
            })
            .collect();
        v.sort();
        for (n, t) in v {
            for b in n.bytes() {
                h = h.wrapping_mul(131).wrapping_add(b as u128);
            }
            h = h.wrapping_mul(1_000_003).wrapping_add(t);
        }
    }
    h
}

/// Every session that running `dj mcp` servers publish.
pub fn sessions(root: &Path) -> Vec<AgentSession> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(root.join(".local").join("mcp")) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("sessions-") && name.ends_with(".json")) {
            continue;
        }
        // A live server touches its file every 5 seconds; an older one was killed.
        let fresh = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() < 20);
        if !fresh {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let pid = v["pid"].as_u64().unwrap_or(0);
        for s in v["sessions"].as_array().into_iter().flatten() {
            out.push(AgentSession {
                session: s["session"].as_str().unwrap_or("").to_string(),
                draft: s["draft"].as_str().unwrap_or("").to_string(),
                frame: s["frame"].as_u64().unwrap_or(0),
                frames: s["frames"].as_u64().unwrap_or(0),
                pid,
            });
        }
    }
    out.sort_by(|a, b| (a.pid, &a.session).cmp(&(b.pid, &b.session)));
    out
}

/// Writes what the person is looking at for `gui_view`, or removes it when not sharing.
pub fn share_view(root: &Path, view: Option<(&str, u64, &str, bool)>) {
    let p = root.join(".local").join("mcp").join("gui.json");
    match view {
        None => {
            let _ = std::fs::remove_file(p);
        }
        Some((id, frame, title, draft)) => {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let v = json!({"segment": id, "title": title, "frame": frame, "draft": draft, "updated": now(), "pid": std::process::id()});
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, v.to_string()).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}
