//! Unsaved edits kept on disk while dj-gui runs, so a crash or a hang in a core (or a
//! force-quit) doesn't lose them (.agents/ISOLATION_PLAN.md, step 4).
//!
//! dj-gui writes `.local/recovery/gui-<pid>.json` whenever its unsaved edits change: each
//! edited segment's text, or null for one it deleted. It removes the file when nothing is
//! unsaved and when it closes the project normally. A file left by a dj-gui that is no
//! longer running is offered back the next time the project opens, with the crash note
//! that GUI left (`crash::note_path`), if any.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// What a dj-gui had unsaved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub pid: u32,
    /// Seconds since the Unix epoch.
    pub saved_at: i64,
    /// Each segment with unsaved edits: its text, or None when it was deleted.
    pub segments: BTreeMap<String, Option<String>>,
}

fn dir(root: &Path) -> PathBuf {
    root.join(".local").join("recovery")
}

fn path_for(root: &Path, pid: u32) -> PathBuf {
    dir(root).join(format!("gui-{pid}.json"))
}

/// Writes this process's snapshot of `segments` (removing it when there are none).
pub fn write(root: &Path, segments: BTreeMap<String, Option<String>>) -> Result<()> {
    let path = path_for(root, std::process::id());
    if segments.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let snap = Snapshot { pid: std::process::id(), saved_at: crate::parity::now(), segments };
    std::fs::create_dir_all(dir(root))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&snap)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Removes this process's snapshot and crash note (a normal close).
pub fn clear(root: &Path) {
    let _ = std::fs::remove_file(path_for(root, std::process::id()));
    let _ = std::fs::remove_file(crate::crash::note_path(root, std::process::id()));
}

/// What a dj-gui that's gone left: its snapshot (if any) and its crash note (if any).
#[derive(Clone, Debug)]
pub struct Left {
    pub pid: u32,
    pub snapshot: Option<Snapshot>,
    pub note: Option<String>,
}

/// Snapshots and crash notes left by dj-gui processes that aren't running any more.
pub fn left_behind(root: &Path) -> Vec<Left> {
    let mut pids: BTreeMap<u32, Left> = BTreeMap::new();
    let read_pid = |name: &str, prefix: &str, suffix: &str| name.strip_prefix(prefix).and_then(|s| s.strip_suffix(suffix)).and_then(|s| s.parse::<u32>().ok());
    let mut found = vec![];
    for e in std::fs::read_dir(dir(root)).into_iter().flatten().flatten() {
        if let Some(pid) = read_pid(&e.file_name().to_string_lossy(), "gui-", ".json") {
            found.push((pid, e.path(), true));
        }
    }
    for e in std::fs::read_dir(root.join(".local")).into_iter().flatten().flatten() {
        if let Some(pid) = read_pid(&e.file_name().to_string_lossy(), "crash-gui-", ".txt") {
            found.push((pid, e.path(), false));
        }
    }
    for (pid, path, is_snapshot) in found {
        if pid == std::process::id() || alive(root, pid) {
            continue;
        }
        let l = pids.entry(pid).or_insert(Left { pid, snapshot: None, note: None });
        if is_snapshot {
            l.snapshot = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        } else {
            l.note = std::fs::read_to_string(&path).ok();
        }
    }
    pids.into_values().filter(|l| l.snapshot.as_ref().is_some_and(|s| !s.segments.is_empty()) || l.note.is_some()).collect()
}

/// Removes what `pid` left behind (after it was restored or discarded).
pub fn forget(root: &Path, pid: u32) {
    let _ = std::fs::remove_file(path_for(root, pid));
    let _ = std::fs::remove_file(crate::crash::note_path(root, pid));
}

/// Whether dj-gui process `pid` is still running.
fn alive(root: &Path, pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks the process exists; EPERM means it does, as another user's.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        let _ = root;
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        // Its open-project mark, refreshed every minute while it runs.
        let mark = root.join(".local").join(format!("gui-{pid}.open"));
        mark.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() < crate::cache::GUI_MARK_STALE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_left_by_a_process_that_has_gone_are_found() {
        let root = std::env::temp_dir().join(format!("dj-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // One of ours: never offered back to ourselves.
        write(&root, BTreeMap::from([("a".to_string(), Some("text".to_string()))])).unwrap();
        assert!(left_behind(&root).is_empty());
        // One left by a process that doesn't exist, with its crash note.
        let gone = 0x7FFF_FFF0u32;
        let snap = Snapshot { pid: gone, saved_at: 1, segments: BTreeMap::from([("b".to_string(), None), ("c".to_string(), Some("c text".to_string()))]) };
        std::fs::write(path_for(&root, gone), serde_json::to_vec(&snap).unwrap()).unwrap();
        std::fs::write(crate::crash::note_path(&root, gone), "crashed").unwrap();
        let left = left_behind(&root);
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].pid, left[0].snapshot.as_ref(), left[0].note.as_deref()), (gone, Some(&snap), Some("crashed")));
        forget(&root, gone);
        assert!(left_behind(&root).is_empty());
        // Nothing unsaved: our file goes.
        write(&root, BTreeMap::new()).unwrap();
        assert!(!path_for(&root, std::process::id()).exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
