//! Edits to a project's tree of segments: joining a segment into its parent, giving
//! segments another parent, duplicating, and splitting one in two.
//!
//! These work on the segments in memory (`Collection::segments`). Writing the files is the
//! caller's (`Collection::save_segment`, `delete_segment_file`), as is anything it keeps
//! about the segments it shows: dj-gui has its undo steps and its list of unsaved edits.

use crate::collection::Collection;
use crate::format::{input_text, Command, InputKey, Segment};
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, HashMap};

/// An id for a new segment.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// `id`'s child, if it has exactly one.
pub fn only_child<'a>(col: &'a Collection, id: &str) -> Option<&'a str> {
    let kids = col.children(Some(id));
    (kids.len() == 1).then(|| kids[0])
}

/// The only children below `top`, down to the first segment with none or several.
pub fn only_child_run(col: &Collection, top: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = top;
    while let Some(c) = only_child(col, cur) {
        out.push(c.to_string());
        cur = c;
    }
    out
}

/// A segment can be joined into its parent when it is the parent's only child.
pub fn can_join(col: &Collection, id: &str) -> bool {
    col.segments.get(id).and_then(|r| r.parent()).is_some_and(|p| col.children(Some(p)).len() == 1)
}

/// A segment joined into its parent, worked out but not yet done (`apply`): the parent
/// grows by the segment's frames and takes its children; the segment goes.
pub struct Join {
    /// The segment joined, which goes.
    pub id: String,
    /// Its parent, which stays.
    pub parent: String,
    /// The segment's children, which become the parent's.
    pub children: Vec<String>,
    /// The frame of the parent where the segment's frames start: the parent's old length.
    pub at: u64,
    merged: Segment,
}

impl Join {
    /// Works out joining `id` into its parent, leaving the project as it is. `marker`: the
    /// text of an event to leave at the join.
    pub fn plan(col: &Collection, id: &str, marker: Option<&str>) -> Result<Join> {
        if !can_join(col, id) {
            return Err(anyhow!("only a segment whose parent has no other children can be joined"));
        }
        let child = &col.segments[id];
        let pid = child.parent().unwrap().to_string();
        let kids: Vec<String> = col.children(Some(id)).into_iter().map(String::from).collect();
        let mut p = col.segments[&pid].clone();
        let (pf, cf) = (p.frames()?, child.frames()?);
        // Every input is released when a segment starts, so release what the parent still
        // holds at its end; then the child's own lines follow, shifted by the parent's length.
        let mut held: BTreeMap<InputKey, i16> = BTreeMap::new();
        for l in &p.lines {
            if let Command::Input { key, value, impulse: false } = &l.command {
                held.insert(*key, *value);
            }
        }
        p.set("frames", &(pf + cf).to_string());
        for (key, v) in held {
            if v != 0 {
                p.insert_line(pf, &input_text(key, 0))?;
            }
        }
        if let Some(m) = marker.map(str::trim).filter(|m| !m.is_empty()) {
            p.insert_line(pf, &format!("[EVENT] {m}"))?;
        }
        for l in &child.lines {
            let t = child.line_command_text(l.line_no).ok_or_else(|| anyhow!("line {} of the child", l.line_no))?;
            p.insert_line(pf + l.frame, &t)?;
        }
        // Hashes: the child's start hash becomes a checkpoint at the join, its end hash the new end.
        p.remove_all("end-hash");
        if let Some(h) = child.get("end-hash") {
            p.set("end-hash", h);
        }
        if let Some(h) = child.get("start-hash") {
            p.add("checkpoint", &format!("{pf} {h}"));
        }
        for (f, h) in child.checkpoints()? {
            p.add("checkpoint", &format!("{} {h}", pf + f));
        }
        Ok(Join { id: id.to_string(), parent: pid, children: kids, at: pf, merged: p })
    }

    /// The segments the join changes: the parent, the segment, and its children.
    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = vec![self.parent.as_str(), self.id.as_str()];
        ids.extend(self.children.iter().map(String::as_str));
        ids
    }

    /// Does the join.
    pub fn apply(self, col: &mut Collection) {
        col.segments.insert(self.parent.clone(), self.merged);
        col.segments.remove(&self.id);
        for k in &self.children {
            if let Some(r) = col.segments.get_mut(k) {
                r.set("parent", &self.parent);
            }
        }
    }
}

/// Gives the segments `ids` another parent (`to`; None: they become root segments, starting
/// from power-on). They start from somewhere else now, so their hashes go; under a parent,
/// so do a save file and a snapshot, which only a root segment has. Returns the ones found.
pub fn reparent(col: &mut Collection, ids: &[String], to: Option<&str>) -> Vec<String> {
    let mut done = vec![];
    for id in ids {
        let Some(r) = col.segments.get_mut(id) else { continue };
        match to {
            Some(p) => {
                r.set("parent", p);
                r.remove_all("sram");
                r.remove_all("snapshot");
            }
            None => r.set("parent", "none"),
        }
        r.remove_all("start-hash");
        r.remove_all("checkpoint");
        r.remove_all("end-hash");
        done.push(id.clone());
    }
    done
}

/// Copies of `id` and, with `children`, of everything below it, under new ids and not yet
/// in the project. The first is the copy of `id` itself: titled "<title> (copy)", with the
/// same parent. The rest hang below it as the originals do.
pub fn duplicate(col: &Collection, id: &str, children: bool) -> Vec<Segment> {
    if !col.segments.contains_key(id) {
        return vec![];
    }
    let olds: Vec<&str> = if children { col.subtree(id) } else { vec![id] };
    let map: HashMap<&str, String> = olds.iter().map(|o| (*o, new_id())).collect();
    olds.iter()
        .map(|o| {
            let mut r = col.segments[*o].clone();
            r.set("id", &map[o]);
            if *o == id {
                r.set("title", &format!("{} (copy)", r.title()));
            } else if let Some(p) = r.parent().and_then(|p| map.get(p)).cloned() {
                r.set("parent", &p);
            }
            r
        })
        .collect()
}

/// Splits `id` in two before frame `frame`: the frames before it become a new segment
/// `up_id`, the parent of `id`, which keeps the rest, its children and its end hash. `hash`
/// is the state at the cut: the new segment's end hash and `id`'s start hash. Returns
/// whether `id` had a checkpoint at `frame` that says otherwise.
pub fn split(col: &mut Collection, id: &str, frame: u64, hash: &str, up_id: &str) -> Result<bool> {
    let whole = col.segments.get(id).ok_or_else(|| anyhow!("no segment {id}"))?;
    let (mut up, mut down) = whole.split_at(frame, up_id)?;
    let differs = whole.checkpoints()?.into_iter().any(|(f, h)| f == frame && h != hash);
    up.set("end-hash", hash);
    down.set("start-hash", hash);
    col.segments.insert(up_id.to_string(), up);
    col.segments.insert(id.to_string(), down);
    Ok(differs)
}

/// The first frame at which two versions of a segment's commands differ.
pub fn first_change(a: &Segment, b: &Segment) -> Option<u64> {
    let n = a.lines.len().min(b.lines.len());
    for i in 0..n {
        let (x, y) = (&a.lines[i], &b.lines[i]);
        if x.frame != y.frame || x.command != y.command {
            return Some(x.frame.min(y.frame));
        }
    }
    match (a.lines.get(n), b.lines.get(n)) {
        (Some(l), _) | (_, Some(l)) => Some(l.frame),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a (10 frames) > b (5) > c (5), and d (5) beside nothing: a second root.
    fn project(name: &str) -> Collection {
        let dir = std::env::temp_dir().join(format!("dj-edit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        std::fs::write(dir.join("config.toml"), CONFIG).unwrap();
        let mut col = Collection::open(&dir).unwrap();
        let mut a = Segment::new("a", None, "root", 10);
        a.set("end-hash", "A");
        let mut b = Segment::new("b", Some("a"), "middle", 5);
        b.set("start-hash", "A");
        b.add("checkpoint", "2 B2");
        b.set("end-hash", "B");
        b.insert_line(1, "[EVENT] in b").unwrap();
        let mut c = Segment::new("c", Some("b"), "leaf", 5);
        c.set("start-hash", "B");
        c.set("end-hash", "C");
        let mut d = Segment::new("d", None, "other root", 5);
        d.set("sram", "blob");
        for r in [a, b, c, d] {
            col.segments.insert(r.id().unwrap().to_string(), r);
        }
        col
    }

    const CONFIG: &str = "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\nfile = \"t.nes\"\n[core]\nname = \"c\"\nbuild = \"c\"\nsha256 = \"\"\n[[ports]]\nport = 1\ndevice = \"JOYPAD\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n";

    #[test]
    fn subtree_and_only_children() {
        let col = project("subtree");
        assert_eq!(col.subtree("a"), ["a", "b", "c"]);
        assert_eq!(col.subtree("nope"), Vec::<&str>::new());
        assert_eq!(only_child(&col, "a"), Some("b"));
        assert_eq!(only_child(&col, "c"), None);
        assert_eq!(only_child_run(&col, "a"), ["b", "c"]);
    }

    #[test]
    fn join_moves_frames_hashes_and_children_into_the_parent() {
        let mut col = project("join");
        let join = Join::plan(&col, "b", Some(" joined here ")).unwrap();
        assert_eq!(join.ids(), ["a", "b", "c"]);
        assert_eq!(join.at, 10);
        join.apply(&mut col);
        assert!(!col.segments.contains_key("b"));
        assert_eq!(col.segments["c"].parent(), Some("a"));
        let a = &col.segments["a"];
        assert_eq!(a.frames().unwrap(), 15);
        assert_eq!(a.get("end-hash"), Some("B"));
        assert_eq!(a.checkpoints().unwrap(), [(10, "A".to_string()), (12, "B2".to_string())]);
        let frames: Vec<u64> = a.lines.iter().map(|l| l.frame).collect();
        assert_eq!(frames, [10, 11], "the marker at the join, then b's event shifted by a's length");
        assert!(a.body().contains("joined here") && a.body().contains("in b"));
    }

    #[test]
    fn only_an_only_child_joins() {
        let mut col = project("join-no");
        assert!(can_join(&col, "b") && !can_join(&col, "a"));
        col.segments.insert("e".into(), Segment::new("e", Some("a"), "sibling", 5));
        assert!(!can_join(&col, "b"));
        assert!(Join::plan(&col, "b", None).is_err());
        assert!(col.segments.contains_key("b"));
    }

    #[test]
    fn reparent_drops_hashes_and_under_a_parent_the_save_file() {
        let mut col = project("reparent");
        let done = reparent(&mut col, &["d".to_string(), "nope".to_string()], Some("c"));
        assert_eq!(done, ["d"]);
        let d = &col.segments["d"];
        assert_eq!((d.parent(), d.get("sram")), (Some("c"), None));
        reparent(&mut col, &["b".to_string()], None);
        let b = &col.segments["b"];
        assert_eq!(b.parent(), None);
        assert_eq!((b.get("start-hash"), b.get("end-hash")), (None, None));
        assert!(b.checkpoints().unwrap().is_empty());
        assert_eq!(col.segments["c"].get("end-hash"), Some("C"), "segments below keep theirs");
    }

    #[test]
    fn duplicate_copies_under_new_ids() {
        let col = project("dup");
        let one = duplicate(&col, "b", false);
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].title(), one[0].parent()), ("middle (copy)", Some("a")));
        assert_ne!(one[0].id().unwrap(), "b");
        let all = duplicate(&col, "b", true);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].title(), "leaf");
        assert_eq!(all[1].parent(), Some(all[0].id().unwrap()), "the copy of c hangs below the copy of b");
        assert_eq!(all[1].get("end-hash"), Some("C"));
        assert!(duplicate(&col, "nope", true).is_empty());
    }

    #[test]
    fn split_puts_the_first_frames_in_a_new_parent() {
        let mut col = project("split");
        assert!(!split(&mut col, "b", 3, "CUT", "up").unwrap());
        let (up, b) = (&col.segments["up"], &col.segments["b"]);
        assert_eq!((up.parent(), b.parent()), (Some("a"), Some("up")));
        assert_eq!((up.frames().unwrap(), b.frames().unwrap()), (3, 2));
        assert_eq!((up.get("end-hash"), b.get("start-hash"), b.get("end-hash")), (Some("CUT"), Some("CUT"), Some("B")));
        assert_eq!(col.segments["c"].parent(), Some("b"));
        // A checkpoint at the cut that says something else is reported.
        let mut col = project("split-differs");
        assert!(split(&mut col, "b", 2, "CUT", "up").unwrap());
        assert!(split(&mut col, "nope", 2, "CUT", "up2").is_err());
    }

    #[test]
    fn first_change_is_the_earliest_differing_frame() {
        let a = Segment::new("a", None, "a", 10);
        let mut b = a.clone();
        assert_eq!(first_change(&a, &b), None);
        b.insert_line(4, "[EVENT] x").unwrap();
        assert_eq!(first_change(&a, &b), Some(4));
        let mut c = b.clone();
        c.insert_line(2, "[EVENT] y").unwrap();
        assert_eq!(first_change(&b, &c), Some(2));
    }
}
