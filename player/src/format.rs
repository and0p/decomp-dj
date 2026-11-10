//! Parser and writer for `.segment` files (docs/segments.md).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

pub const MAGIC: &str = "dj-replay 1";
/// The first line before the rename to decomp-dj. Still read; files are written with MAGIC.
const OLD_MAGIC: &str = "emucov-replay 1";

/// Hashes are the first 32 hex digits of a BLAKE3 digest (docs/playback.md, "State hashes").
pub const HASH_HEX: usize = 32;

/// A BLAKE3 digest as the format writes it.
pub fn hash_hex(h: &blake3::Hash) -> String {
    h.to_hex()[..HASH_HEX].to_string()
}

/// Reads a hash in the current form or the older `blake3-<64 hex>` one.
pub fn normalize_hash(v: &str) -> String {
    let v = v.trim();
    let v = v.strip_prefix("blake3-").unwrap_or(v);
    if v.len() > HASH_HEX && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        v[..HASH_HEX].to_string()
    } else {
        v.to_string()
    }
}

/// Header fields whose values are hashes (`checkpoint` holds `<frame> <hash>`).
const HASH_FIELDS: [&str; 4] = ["start-hash", "end-hash", "snapshot", "sram"];

/// Header field names from before a rename, and what they are called now.
fn current_key(k: &str) -> &str {
    match k {
        "save" => "snapshot",
        _ => k,
    }
}

fn normalize_field(k: &str, v: &str) -> String {
    if HASH_FIELDS.contains(&k) {
        return normalize_hash(v);
    }
    if k == "checkpoint" {
        if let Some((f, h)) = v.split_once(char::is_whitespace) {
            return format!("{} {}", f.trim(), normalize_hash(h));
        }
    }
    v.to_string()
}

/// Rewrites a file from before the rename into the current form: the first line and
/// the hashes in the header. Everything else stays byte for byte.
pub fn upgrade_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_header = true;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let (body, cr) = match body.strip_suffix('\r') {
            Some(b) => (b, "\r"),
            None => (body, ""),
        };
        let new = if i == 0 && body.trim() == OLD_MAGIC {
            MAGIC.to_string()
        } else if in_header && body.trim() == "---" {
            in_header = false;
            body.to_string()
        } else if in_header && i > 0 {
            match body.split_once(':') {
                Some((k, v)) if !body.trim_start().starts_with('#') => {
                    let key = current_key(k.trim());
                    let nv = normalize_field(key, v.trim());
                    if nv == v.trim() && key == k.trim() { body.to_string() } else { format!("{key}: {nv}") }
                }
                _ => body.to_string(),
            }
        } else {
            body.to_string()
        };
        out.push_str(&new);
        out.push_str(cr);
        out.push_str(nl);
    }
    out
}

// libretro device ids (libretro.h).
pub const DEVICE_NONE: u32 = 0;
pub const DEVICE_JOYPAD: u32 = 1;
pub const DEVICE_MOUSE: u32 = 2;
pub const DEVICE_KEYBOARD: u32 = 3;
pub const DEVICE_LIGHTGUN: u32 = 4;
pub const DEVICE_ANALOG: u32 = 5;
pub const DEVICE_POINTER: u32 = 6;

/// One input value as libretro addresses it: `input_state(port, device, index, id)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct InputKey {
    pub port: u32,
    pub device: u32,
    pub index: u32,
    pub id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Sets a held value. `impulse` values last one frame only (mouse motion).
    Input { key: InputKey, value: i16, impulse: bool },
    Device { port: u32, device: u32 },
    Write { region: String, address: u64, bytes: Vec<u8> },
    Event { text: String, pos: Option<(i32, i32)> },
    Reset,
    Screenshot,
    Rng { name: String, count: u64 },
}

#[derive(Clone, Debug)]
pub struct Line {
    pub frame: u64,
    pub command: Command,
    /// Its line number in the file when it was read: what errors cite, and what names the
    /// line to `line_command_text` and `replace_line`. Header edits that don't reparse (a
    /// title, a hash) move the file's lines but not this.
    pub line_no: usize,
    /// Index into `body_text`, which header edits don't move.
    body: usize,
}

#[derive(Clone, Debug)]
enum HeaderLine {
    Field(String, String),
    Other(String),
}

#[derive(Clone, Debug)]
pub struct Replay {
    header: Vec<HeaderLine>,
    body_text: Vec<String>,
    pub lines: Vec<Line>,
}

impl Replay {
    pub fn new(id: &str, parent: Option<&str>, title: &str, frames: u64) -> Replay {
        let mut r = Replay { header: vec![], body_text: vec![], lines: vec![] };
        r.set("id", id);
        r.set("parent", parent.unwrap_or("none"));
        if !title.is_empty() {
            r.set("title", title);
        }
        r.set("frames", &frames.to_string());
        r
    }

    pub fn load(path: &Path) -> Result<Replay> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Replay::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Replay> {
        let mut it = text.lines().enumerate();
        let first = it.next().map(|(_, l)| l.trim()).unwrap_or("");
        if first != MAGIC && first != OLD_MAGIC {
            bail!("first line must be `{MAGIC}`");
        }
        let mut header = vec![];
        let mut in_body = false;
        let mut body_text = vec![];
        let mut lines = vec![];
        let mut last_frame = 0u64;
        for (n, raw) in it {
            let line_no = n + 1;
            let l = raw.trim_end_matches('\r');
            if !in_body {
                if l.trim() == "---" {
                    in_body = true;
                    continue;
                }
                let t = l.trim();
                if t.is_empty() || t.starts_with('#') {
                    header.push(HeaderLine::Other(l.to_string()));
                    continue;
                }
                let (k, v) = t.split_once(':').ok_or_else(|| anyhow!("line {line_no}: expected `key: value`"))?;
                let key = current_key(k.trim());
                header.push(HeaderLine::Field(key.to_string(), normalize_field(key, v.trim())));
            } else {
                body_text.push(l.to_string());
                let body = body_text.len() - 1;
                let t = l.trim();
                if t.is_empty() || t.starts_with('#') {
                    continue;
                }
                let (frame, command) = parse_body_line(t).with_context(|| format!("line {line_no}: `{t}`"))?;
                if frame < last_frame {
                    bail!("line {line_no}: frame {frame} comes after frame {last_frame}");
                }
                last_frame = frame;
                lines.push(Line { frame, command, line_no, body });
            }
        }
        let r = Replay { header, body_text, lines };
        r.id()?;
        r.frames()?;
        if r.get("parent").is_none() {
            bail!("missing `parent` (use `parent: none` for a trunk)");
        }
        if let Some(last) = r.lines.last() {
            if last.frame >= r.frames()? {
                bail!("line {}: frame {} is past the end (frames: {})", last.line_no, last.frame, r.frames()?);
            }
        }
        Ok(r)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.header.iter().find_map(|h| match h {
            HeaderLine::Field(k, v) if k == key => Some(v.as_str()),
            _ => None,
        })
    }

    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.header
            .iter()
            .filter_map(|h| match h {
                HeaderLine::Field(k, v) if k == key => Some(v.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Sets a header field, keeping its position if it exists.
    pub fn set(&mut self, key: &str, value: &str) {
        for h in self.header.iter_mut() {
            if let HeaderLine::Field(k, v) = h {
                if k == key {
                    *v = value.to_string();
                    return;
                }
            }
        }
        self.header.push(HeaderLine::Field(key.to_string(), value.to_string()));
    }

    pub fn header_keys(&self) -> Vec<String> {
        self.header
            .iter()
            .filter_map(|h| match h {
                HeaderLine::Field(k, _) => Some(k.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn remove_all(&mut self, key: &str) {
        self.header.retain(|h| !matches!(h, HeaderLine::Field(k, _) if k == key));
    }

    pub fn add(&mut self, key: &str, value: &str) {
        self.header.push(HeaderLine::Field(key.to_string(), value.to_string()));
    }

    pub fn id(&self) -> Result<&str> {
        self.get("id").ok_or_else(|| anyhow!("missing `id`"))
    }

    pub fn parent(&self) -> Option<&str> {
        match self.get("parent") {
            None | Some("none") | Some("") => None,
            Some(p) => Some(p),
        }
    }

    pub fn title(&self) -> &str {
        self.get("title").unwrap_or("")
    }

    pub fn frames(&self) -> Result<u64> {
        let f = self.get("frames").ok_or_else(|| anyhow!("missing `frames`"))?;
        f.parse().map_err(|_| anyhow!("`frames` is not a number: {f}"))
    }

    pub fn checkpoints(&self) -> Result<Vec<(u64, String)>> {
        self.get_all("checkpoint")
            .into_iter()
            .map(|v| {
                let (f, h) = v.split_once(char::is_whitespace).ok_or_else(|| anyhow!("checkpoint needs `<frame> <hash>`"))?;
                Ok((f.trim().parse()?, h.trim().to_string()))
            })
            .collect()
    }

    pub fn is_poked(&self) -> bool {
        self.lines.iter().any(|l| matches!(l.command, Command::Write { .. }))
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(MAGIC);
        s.push('\n');
        for h in &self.header {
            match h {
                HeaderLine::Field(k, v) => {
                    let _ = writeln!(s, "{k}: {v}");
                }
                HeaderLine::Other(o) => {
                    s.push_str(o);
                    s.push('\n');
                }
            }
        }
        s.push_str("---\n");
        for b in &self.body_text {
            s.push_str(b);
            s.push('\n');
        }
        s
    }

    /// Appends body lines (already formatted, frame first).
    /// Replaces every input line for `key` with the given held spans
    /// (`start..end`, end exclusive), keeping all other lines. Used by editors.
    pub fn set_binary_spans(&mut self, key: InputKey, spans: &[(u64, u64)]) -> Result<()> {
        self.set_binary_spans_many(&[(key, spans.to_vec())])
    }

    /// `set_binary_spans` for several inputs at once, re-parsing the file only once.
    pub fn set_binary_spans_many(&mut self, edits: &[(InputKey, Vec<(u64, u64)>)]) -> Result<()> {
        if edits.is_empty() {
            return Ok(());
        }
        let frames = self.frames()?;
        let mut kept: Vec<(u64, String)> = vec![];
        let mut comments: Vec<String> = vec![];
        for b in &self.body_text {
            let t = b.trim();
            if t.is_empty() {
                continue;
            }
            if t.starts_with('#') {
                comments.push(b.clone());
                continue;
            }
            let (f, cmd) = parse_body_line(t)?;
            if let Command::Input { key: k, .. } = cmd {
                if edits.iter().any(|(key, _)| *key == k) {
                    continue;
                }
            }
            kept.push((f, b.clone()));
        }
        for (key, spans) in edits {
            let key = *key;
            let mut merged: Vec<(u64, u64)> = vec![];
            let mut sorted: Vec<(u64, u64)> = spans.iter().copied().filter(|(a, b)| b > a && *a < frames).collect();
            sorted.sort();
            for (a, b) in sorted {
                match merged.last_mut() {
                    Some(last) if a <= last.1 => last.1 = last.1.max(b),
                    _ => merged.push((a, b)),
                }
            }
            for (a, b) in merged {
                kept.push((a, format!("{a} {}", input_text(key, 1))));
                if b < frames {
                    kept.push((b, format!("{b} {}", input_text(key, 0))));
                }
            }
        }
        kept.sort_by_key(|(f, _)| *f);
        let mut body = comments;
        body.extend(kept.into_iter().map(|(_, t)| t));
        let mut text = String::new();
        // Re-parse the whole file so line numbers and parsed commands stay in step.
        self.body_text = body;
        text.push_str(&self.to_text());
        *self = Replay::parse(&text)?;
        Ok(())
    }

    /// Held spans (`start..end`) of a binary input, worked out from the lines.
    pub fn binary_spans(&self, key: InputKey) -> Vec<(u64, u64)> {
        let frames = self.frames().unwrap_or(0);
        let mut out = vec![];
        let mut open: Option<u64> = None;
        for l in &self.lines {
            if let Command::Input { key: k, value, .. } = l.command {
                if k != key {
                    continue;
                }
                match (open, value != 0) {
                    (None, true) => open = Some(l.frame),
                    (Some(a), false) => {
                        if l.frame > a {
                            out.push((a, l.frame));
                        }
                        open = None;
                    }
                    _ => {}
                }
            }
        }
        if let Some(a) = open {
            out.push((a, frames));
        }
        out
    }

    fn reparse(&mut self) -> Result<()> {
        *self = Replay::parse(&self.to_text())?;
        Ok(())
    }

    /// Index into `body_text` of the line read from file line `line_no`. Not worked out from
    /// the header's length: `set`, `add` and `remove_all` change that without reparsing.
    fn body_index(&self, line_no: usize) -> Option<usize> {
        let i = self.lines.iter().find(|l| l.line_no == line_no)?.body;
        (i < self.body_text.len()).then_some(i)
    }

    /// The command part of a body line (everything after the frame number).
    pub fn line_command_text(&self, line_no: usize) -> Option<String> {
        let t = self.body_text[self.body_index(line_no)?].trim();
        t.split_once(char::is_whitespace).map(|(_, rest)| rest.trim().to_string())
    }

    /// Adds `<frame> <command>` after any lines already on that frame.
    pub fn insert_line(&mut self, frame: u64, command: &str) -> Result<()> {
        let text = format!("{frame} {}", command.trim());
        parse_body_line(&text)?;
        let mut at = None;
        for (i, b) in self.body_text.iter().enumerate() {
            let t = b.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let f = parse_body_line(t)?.0;
            if f <= frame {
                at = Some(i + 1);
            } else if at.is_none() {
                at = Some(i);
                break;
            } else {
                break;
            }
        }
        let at = at.unwrap_or(self.body_text.len());
        let old = self.clone();
        self.body_text.insert(at, text);
        if let Err(e) = self.reparse() {
            *self = old;
            return Err(e);
        }
        Ok(())
    }

    /// Replaces a body line's command (keeping its frame), or removes the line when `command` is None.
    pub fn replace_line(&mut self, line_no: usize, command: Option<&str>) -> Result<()> {
        let i = self.body_index(line_no).ok_or_else(|| anyhow!("no body line {line_no}"))?;
        let old = self.clone();
        match command {
            None => {
                self.body_text.remove(i);
            }
            Some(c) => {
                let frame = parse_body_line(self.body_text[i].trim())?.0;
                let text = format!("{frame} {}", c.trim());
                parse_body_line(&text)?;
                self.body_text[i] = text;
            }
        }
        if let Err(e) = self.reparse() {
            *self = old;
            return Err(e);
        }
        Ok(())
    }

    /// Splits this replay at `frame`. The downstream part (the rest, renumbered from 0)
    /// keeps this id, title and children; the upstream part (frames `0..frame`) gets
    /// `upstream_id`, takes this replay's place under its parent, and becomes the
    /// downstream part's parent. Inputs held across the split are pressed again at frame 0
    /// of the downstream part, since every replay starts with all inputs released.
    /// Hashes: the downstream part keeps the old end-hash and the later checkpoints; the
    /// upstream part keeps the earlier ones. The caller records the hash at the split
    /// as the upstream end-hash and the downstream start-hash.
    pub fn split_at(&self, frame: u64, upstream_id: &str) -> Result<(Replay, Replay)> {
        let frames = self.frames()?;
        if frame == 0 || frame >= frames {
            bail!("can only split inside the replay (1..{frames})");
        }
        let mut held: BTreeMap<InputKey, i16> = BTreeMap::new();
        for l in self.lines.iter().take_while(|l| l.frame < frame) {
            if let Command::Input { key, value, impulse: false } = l.command {
                if value == 0 {
                    held.remove(&key);
                } else {
                    held.insert(key, value);
                }
            }
        }
        let (mut up_body, mut down_body) = (vec![], vec![]);
        for (k, v) in &held {
            down_body.push(format!("0 {}", input_text(*k, *v)));
        }
        for b in &self.body_text {
            let t = b.trim();
            if t.is_empty() || t.starts_with('#') {
                up_body.push(b.clone());
                continue;
            }
            let (f, _) = parse_body_line(t)?;
            if f < frame {
                up_body.push(b.clone());
            } else {
                let rest = t.split_once(char::is_whitespace).map(|(_, r)| r.trim()).unwrap_or("");
                down_body.push(format!("{} {rest}", f - frame));
            }
        }
        let cps = self.checkpoints()?;
        let mut up = self.clone();
        up.body_text = up_body;
        up.set("id", upstream_id);
        up.set("title", &format!("{} (first {frame} frames)", self.title()));
        up.set("frames", &frame.to_string());
        up.remove_all("end-hash");
        up.remove_all("checkpoint");
        for (f, h) in cps.iter().filter(|(f, _)| *f < frame) {
            up.add("checkpoint", &format!("{f} {h}"));
        }
        let mut down = self.clone();
        down.body_text = down_body;
        down.set("parent", upstream_id);
        down.set("frames", &(frames - frame).to_string());
        // Only a root segment starts from blobs; those stay with the upstream part.
        down.remove_all("snapshot");
        down.remove_all("sram");
        down.remove_all("start-hash");
        match self.get("start-frame").and_then(|s| s.parse::<u64>().ok()) {
            Some(sf) => down.set("start-frame", &(sf + frame).to_string()),
            None => down.remove_all("start-frame"),
        }
        down.remove_all("checkpoint");
        for (f, h) in cps.iter().filter(|(f, _)| *f > frame) {
            down.add("checkpoint", &format!("{} {h}", f - frame));
        }
        up.reparse()?;
        down.reparse()?;
        Ok((up, down))
    }

    /// Cuts the replay back to its first `frame` frames: later lines and checkpoints go,
    /// and so does the end hash.
    pub fn truncate_at(&mut self, frame: u64) -> Result<()> {
        let frames = self.frames()?;
        if frame >= frames {
            return Ok(());
        }
        let mut body = vec![];
        for b in &self.body_text {
            let t = b.trim();
            if t.is_empty() || t.starts_with('#') || parse_body_line(t)?.0 < frame {
                body.push(b.clone());
            }
        }
        let cps = self.checkpoints()?;
        self.body_text = body;
        self.set("frames", &frame.to_string());
        self.remove_all("end-hash");
        self.remove_all("checkpoint");
        for (f, h) in cps.iter().filter(|(f, _)| *f < frame) {
            self.add("checkpoint", &format!("{f} {h}"));
        }
        self.reparse()
    }

    /// Adds `frames` frames at the end, with body lines whose frame numbers count from the
    /// start of the chunk. Errors name the chunk's own line numbers.
    pub fn append_chunk(&mut self, text: &str, frames: u64) -> Result<()> {
        let start = self.frames()?;
        let mut added = vec![];
        for (n, raw) in text.lines().enumerate() {
            let t = raw.trim();
            if t.is_empty() {
                continue;
            }
            if t.starts_with('#') {
                added.push(t.to_string());
                continue;
            }
            let (f, _) = parse_body_line(t).with_context(|| format!("line {}: `{t}`", n + 1))?;
            if f >= frames {
                bail!("line {}: frame {f} is past the end of the chunk ({frames} frames)", n + 1);
            }
            let rest = t.split_once(char::is_whitespace).map(|(_, r)| r.trim()).unwrap_or("");
            added.push(format!("{} {rest}", start + f));
        }
        let old = self.clone();
        self.body_text.extend(added);
        self.set("frames", &(start + frames).to_string());
        self.remove_all("end-hash");
        if let Err(e) = self.reparse() {
            *self = old;
            return Err(e);
        }
        Ok(())
    }

    /// Sets the frame count without touching the lines (which must all be before it).
    pub fn set_frames(&mut self, frames: u64) -> Result<()> {
        if let Some(l) = self.lines.last() {
            if l.frame >= frames {
                bail!("a line at frame {} is past {frames} frames", l.frame);
            }
        }
        self.set("frames", &frames.to_string());
        self.remove_all("end-hash");
        Ok(())
    }

    /// The body text (everything after `---`).
    pub fn body(&self) -> String {
        self.body_text.join("\n")
    }

    #[allow(dead_code)]
    pub fn push_body(&mut self, text: &str) -> Result<()> {
        let (frame, command) = parse_body_line(text.trim())?;
        let line_no = self.lines.iter().map(|l| l.line_no).max().unwrap_or(0) + 1;
        self.lines.push(Line { frame, command, line_no, body: self.body_text.len() });
        self.body_text.push(text.to_string());
        Ok(())
    }
}

fn parse_num(s: &str) -> Result<u64> {
    let s = s.trim();
    let (digits, radix) = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => (h, 16),
        None => (s, 10),
    };
    u64::from_str_radix(digits, radix).map_err(|_| anyhow!("not a number: {s}"))
}

fn parse_hex(s: &str) -> Result<u64> {
    let s = s.trim();
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(h, 16).map_err(|_| anyhow!("not a hex number: {s}"))
}

fn parse_value(s: &str) -> Result<i16> {
    let v: i64 = if let Some(h) = s.strip_prefix("0x") { i64::from_str_radix(h, 16)? } else { s.parse()? };
    if !(i16::MIN as i64..=i16::MAX as i64).contains(&v) {
        bail!("value {v} is outside -32768..32767");
    }
    Ok(v as i16)
}

pub fn parse_body_line(t: &str) -> Result<(u64, Command)> {
    let (frame_s, rest) = t.split_once(char::is_whitespace).ok_or_else(|| anyhow!("expected `<frame> <command>`"))?;
    let frame = parse_num(frame_s)?;
    let rest = rest.trim();
    if let Some(after) = rest.strip_prefix('[') {
        let (tag, args) = after.split_once(']').ok_or_else(|| anyhow!("unclosed `[`"))?;
        let args = args.trim();
        let mut tag_words = tag.split_whitespace();
        let name = tag_words.next().unwrap_or("").to_ascii_uppercase();
        let cmd = match name.as_str() {
            "WRITE" => {
                let mut w = args.split_whitespace();
                let region = w.next().ok_or_else(|| anyhow!("[WRITE] needs a region"))?.to_ascii_uppercase();
                let address = parse_hex(w.next().ok_or_else(|| anyhow!("[WRITE] needs an address"))?)?;
                let bytes = w.map(|b| parse_hex(b).and_then(|v| u8::try_from(v).map_err(|_| anyhow!("byte {b} > 0xFF")))).collect::<Result<Vec<u8>>>()?;
                if bytes.is_empty() {
                    bail!("[WRITE] needs at least one byte");
                }
                Command::Write { region, address, bytes }
            }
            "EVENT" => {
                let pos = match tag_words.next() {
                    Some(p) => {
                        let p = p.strip_prefix('@').ok_or_else(|| anyhow!("event position must look like @x,y"))?;
                        let (x, y) = p.split_once(',').ok_or_else(|| anyhow!("event position must look like @x,y"))?;
                        Some((x.parse()?, y.parse()?))
                    }
                    None => None,
                };
                Command::Event { text: args.to_string(), pos }
            }
            "DEVICE" => {
                let mut w = args.split_whitespace();
                let port = parse_port(w.next().ok_or_else(|| anyhow!("[DEVICE] needs a port"))?)?;
                let dev = w.next().ok_or_else(|| anyhow!("[DEVICE] needs a device"))?;
                Command::Device { port, device: device_from_name(dev)? }
            }
            "RESET" => Command::Reset,
            "SCREENSHOT" => Command::Screenshot,
            "RNG" => {
                let mut w = args.split_whitespace();
                let name = w.next().ok_or_else(|| anyhow!("[RNG] needs a name"))?.to_string();
                let count = parse_num(w.next().unwrap_or("1"))?;
                Command::Rng { name, count }
            }
            other => bail!("unknown command [{other}]"),
        };
        return Ok((frame, cmd));
    }
    Ok((frame, parse_input(rest)?))
}

fn parse_port(s: &str) -> Result<u32> {
    let n = s.strip_prefix('P').or_else(|| s.strip_prefix('p')).ok_or_else(|| anyhow!("port must look like P1"))?;
    let n: u32 = n.parse()?;
    if n == 0 {
        bail!("ports start at P1");
    }
    Ok(n - 1)
}

pub fn device_from_name(s: &str) -> Result<u32> {
    Ok(match s.to_ascii_uppercase().as_str() {
        "NONE" => DEVICE_NONE,
        "JOYPAD" => DEVICE_JOYPAD,
        "MOUSE" => DEVICE_MOUSE,
        "KEYBOARD" => DEVICE_KEYBOARD,
        "LIGHTGUN" => DEVICE_LIGHTGUN,
        "ANALOG" => DEVICE_ANALOG,
        "POINTER" => DEVICE_POINTER,
        other => other.parse().map_err(|_| anyhow!("unknown device {other}"))?,
    })
}

pub const JOYPAD: [&str; 16] = ["B", "Y", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT", "A", "X", "L", "R", "L2", "R2", "L3", "R3"];

/// Formats an input line (without the frame number), the inverse of the parser.
pub fn input_text(key: InputKey, value: i16) -> String {
    let p = key.port + 1;
    match key.device {
        DEVICE_JOYPAD if (key.id as usize) < JOYPAD.len() && key.index == 0 => format!("P{p} JOYPAD {} {value}", JOYPAD[key.id as usize]),
        DEVICE_MOUSE => match mouse_name(key.id) {
            Some(n) => format!("P{p} MOUSE {n} {value}"),
            None => format!("P{p} RAW {} {} {} {value}", key.device, key.index, key.id),
        },
        _ => format!("P{p} RAW {} {} {} {value}", key.device, key.index, key.id),
    }
}

pub fn mouse_name(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "X",
        1 => "Y",
        2 => "LEFT",
        3 => "RIGHT",
        4 => "WHEELUP",
        5 => "WHEELDOWN",
        6 => "MIDDLE",
        7 => "HORIZ_WHEELUP",
        8 => "HORIZ_WHEELDOWN",
        9 => "BUTTON_4",
        10 => "BUTTON_5",
        _ => return None,
    })
}

/// True for inputs that are on/off (buttons and keys), as opposed to axes and positions.
pub fn is_binary(key: InputKey) -> bool {
    match key.device {
        DEVICE_JOYPAD => key.id < 16,
        DEVICE_MOUSE => matches!(key.id, 2 | 3 | 6 | 9 | 10),
        DEVICE_KEYBOARD => true,
        DEVICE_LIGHTGUN => matches!(key.id, 2..=12 | 16),
        _ => false,
    }
}

fn joypad_id(s: &str) -> Option<u32> {
    JOYPAD.iter().position(|b| *b == s).map(|i| i as u32)
}

fn lightgun_id(s: &str) -> Option<u32> {
    Some(match s {
        "X" => 0,
        "Y" => 1,
        "TRIGGER" => 2,
        "AUX_A" => 3,
        "AUX_B" => 4,
        "START" => 6,
        "SELECT" => 7,
        "AUX_C" => 8,
        "DPAD_UP" => 9,
        "DPAD_DOWN" => 10,
        "DPAD_LEFT" => 11,
        "DPAD_RIGHT" => 12,
        "SCREEN_X" => 13,
        "SCREEN_Y" => 14,
        "IS_OFFSCREEN" => 15,
        "RELOAD" => 16,
        _ => return None,
    })
}

fn mouse_id(s: &str) -> Option<(u32, bool)> {
    Some(match s {
        "X" => (0, true),
        "Y" => (1, true),
        "LEFT" => (2, false),
        "RIGHT" => (3, false),
        "WHEELUP" => (4, true),
        "WHEELDOWN" => (5, true),
        "MIDDLE" => (6, false),
        "HORIZ_WHEELUP" => (7, true),
        "HORIZ_WHEELDOWN" => (8, true),
        "BUTTON_4" => (9, false),
        "BUTTON_5" => (10, false),
        _ => return None,
    })
}

fn pointer_id(s: &str) -> Option<u32> {
    Some(match s {
        "X" => 0,
        "Y" => 1,
        "PRESSED" => 2,
        "COUNT" => 3,
        "IS_OFFSCREEN" => 15,
        _ => return None,
    })
}

/// RETROK_ key codes for the keys replays are most likely to use.
fn key_id(s: &str) -> Option<u32> {
    if s.len() == 1 {
        let c = s.as_bytes()[0];
        if c.is_ascii_uppercase() {
            return Some((c.to_ascii_lowercase()) as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = s.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=15).contains(&n) {
            return Some(281 + n);
        }
    }
    Some(match s {
        "BACKSPACE" => 8,
        "TAB" => 9,
        "RETURN" => 13,
        "ESCAPE" => 27,
        "SPACE" => 32,
        "DELETE" => 127,
        "UP" => 273,
        "DOWN" => 274,
        "RIGHT" => 275,
        "LEFT" => 276,
        "INSERT" => 277,
        "HOME" => 278,
        "END" => 279,
        "PAGEUP" => 280,
        "PAGEDOWN" => 281,
        "RSHIFT" => 303,
        "LSHIFT" => 304,
        "RCTRL" => 305,
        "LCTRL" => 306,
        "RALT" => 307,
        "LALT" => 308,
        _ => return None,
    })
}

fn parse_input(rest: &str) -> Result<Command> {
    let w: Vec<&str> = rest.split_whitespace().collect();
    if w.len() < 3 {
        bail!("input lines look like `P1 JOYPAD A 1`");
    }
    let port = parse_port(w[0])?;
    let dev = w[1].to_ascii_uppercase();
    let up = |i: usize| w.get(i).map(|s| s.to_ascii_uppercase()).ok_or_else(|| anyhow!("missing field"));
    let val = |i: usize| parse_value(w.get(i).ok_or_else(|| anyhow!("missing value"))?);
    let (device, index, id, value, impulse) = match dev.as_str() {
        "JOYPAD" => {
            let b = up(2)?;
            (DEVICE_JOYPAD, 0, joypad_id(&b).ok_or_else(|| anyhow!("unknown joypad button {b}"))?, val(3)?, false)
        }
        "ANALOG" => {
            let stick = up(2)?;
            match stick.as_str() {
                "LEFT" | "RIGHT" => {
                    let axis = up(3)?;
                    let id = match axis.as_str() {
                        "X" => 0,
                        "Y" => 1,
                        _ => bail!("analog axis must be X or Y"),
                    };
                    (DEVICE_ANALOG, if stick == "LEFT" { 0 } else { 1 }, id, val(4)?, false)
                }
                "BUTTON" => {
                    let b = up(3)?;
                    (DEVICE_ANALOG, 2, joypad_id(&b).ok_or_else(|| anyhow!("unknown button {b}"))?, val(4)?, false)
                }
                _ => bail!("analog control must be LEFT, RIGHT or BUTTON"),
            }
        }
        "LIGHTGUN" => {
            let c = up(2)?;
            let id = lightgun_id(&c).ok_or_else(|| anyhow!("unknown lightgun control {c}"))?;
            // X/Y (ids 0,1) are deprecated relative motion: impulses.
            (DEVICE_LIGHTGUN, 0, id, val(3)?, id <= 1)
        }
        "MOUSE" => {
            let c = up(2)?;
            let (id, impulse) = mouse_id(&c).ok_or_else(|| anyhow!("unknown mouse control {c}"))?;
            (DEVICE_MOUSE, 0, id, val(3)?, impulse)
        }
        "POINTER" => {
            // P1 POINTER X 0  or  P1 POINTER 1 X 0
            let (index, at) = match w.get(2).and_then(|s| s.parse::<u32>().ok()) {
                Some(i) => (i, 3),
                None => (0, 2),
            };
            let c = up(at)?;
            (DEVICE_POINTER, index, pointer_id(&c).ok_or_else(|| anyhow!("unknown pointer control {c}"))?, val(at + 1)?, false)
        }
        "KEYBOARD" => {
            let k = up(2)?;
            (DEVICE_KEYBOARD, 0, key_id(&k).ok_or_else(|| anyhow!("unknown key {k}; use RAW for others"))?, val(3)?, false)
        }
        "RAW" => {
            if w.len() < 6 {
                bail!("RAW lines look like `P1 RAW <device> <index> <id> <value>`");
            }
            (parse_num(w[2])? as u32, parse_num(w[3])? as u32, parse_num(w[4])? as u32, val(5)?, false)
        }
        other => bail!("unknown device {other}"),
    };
    Ok(Command::Input { key: InputKey { port, device, index, id }, value, impulse })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_append_and_truncate() {
        let mut r = Replay::new("d", Some("p"), "t", 0);
        r.append_chunk("0 P1 JOYPAD A 1\n# comment\n10 P1 JOYPAD A 0", 20).unwrap();
        r.append_chunk("5 P1 JOYPAD RIGHT 1", 10).unwrap();
        assert_eq!(r.frames().unwrap(), 30);
        let frames: Vec<u64> = r.lines.iter().map(|l| l.frame).collect();
        assert_eq!(frames, vec![0, 10, 25]);
        let e = r.append_chunk("3 P1 JOYPAD B 1\n12 P1 JOYPAD B 0", 10).unwrap_err();
        assert!(format!("{e:#}").contains("line 2"), "{e:#}");
        assert_eq!(r.frames().unwrap(), 30, "a failed chunk changes nothing");
        r.truncate_at(12).unwrap();
        assert_eq!(r.frames().unwrap(), 12);
        assert_eq!(r.lines.len(), 2);
        r.truncate_at(40).unwrap();
        assert_eq!(r.frames().unwrap(), 12);
    }

    const SAMPLE: &str = "dj-replay 1
id: 3b8f0f2c-1d44-4c1e-9e55-0d7a3c1b2e90
parent: none
title: Boot
frames: 100
# a comment
---
0 P1 JOYPAD START 1
4 P1 JOYPAD START 0
5 P2 MOUSE X -3
6 P1 ANALOG LEFT X -32768
7 [WRITE] RAM 0x12 13 ff
8 [EVENT @10,20] Title visible
9 [DEVICE] P2 MOUSE
10 P1 RAW 1 0 8 1
";

    #[test]
    fn parses_and_round_trips() {
        let r = Replay::parse(SAMPLE).unwrap();
        assert_eq!(r.id().unwrap(), "3b8f0f2c-1d44-4c1e-9e55-0d7a3c1b2e90");
        assert_eq!(r.parent(), None);
        assert_eq!(r.frames().unwrap(), 100);
        assert_eq!(r.lines.len(), 8);
        assert!(r.is_poked());
        assert_eq!(
            r.lines[0].command,
            Command::Input { key: InputKey { port: 0, device: 1, index: 0, id: 3 }, value: 1, impulse: false }
        );
        assert_eq!(
            r.lines[2].command,
            Command::Input { key: InputKey { port: 1, device: 2, index: 0, id: 0 }, value: -3, impulse: true }
        );
        assert_eq!(r.lines[4].command, Command::Write { region: "RAM".into(), address: 0x12, bytes: vec![0x13, 0xff] });
        assert_eq!(r.lines[5].command, Command::Event { text: "Title visible".into(), pos: Some((10, 20)) });
        assert_eq!(Replay::parse(&r.to_text()).unwrap().to_text(), r.to_text());
    }

    #[test]
    fn line_text_survives_header_edits() {
        let text = "dj-replay 1\nid: a\nparent: none\nframes: 100\nend-hash: x\n---\n0 P1 JOYPAD DOWN 1\n40 [WRITE] RAM 0x73 00 00 03\n50 P1 JOYPAD DOWN 0\n";
        let mut r = Replay::parse(text).unwrap();
        let write = r.lines[1].line_no;
        // Fewer header lines (an edit drops the end hash), then more (a title, a checkpoint).
        r.remove_all("end-hash");
        assert_eq!(r.line_command_text(write).as_deref(), Some("[WRITE] RAM 0x73 00 00 03"));
        r.set("title", "t");
        r.add("checkpoint", "20 c");
        r.add("checkpoint", "30 d");
        assert_eq!(r.line_command_text(write).as_deref(), Some("[WRITE] RAM 0x73 00 00 03"));
        assert_eq!(r.line_command_text(r.lines[0].line_no).as_deref(), Some("P1 JOYPAD DOWN 1"));
        r.replace_line(write, Some("[WRITE] RAM 0x73 09")).unwrap();
        assert!(r.to_text().contains("\n40 [WRITE] RAM 0x73 09\n50 P1 JOYPAD DOWN 0\n"), "{}", r.to_text());
        assert!(r.to_text().contains("checkpoint: 30 d"));
    }

    #[test]
    fn splits_and_edits_lines() {
        let text = "dj-replay 1\nid: a\nparent: none\nframes: 100\nend-hash: x\ncheckpoint: 20 c20\ncheckpoint: 80 c80\n---\n# note\n0 P1 JOYPAD A 1\n10 P1 JOYPAD RIGHT 1\n30 [EVENT] middle\n60 P1 JOYPAD RIGHT 0\n70 [WRITE] RAM 0x10 01 02\n";
        let r = Replay::parse(text).unwrap();
        let (up, down) = r.split_at(50, "b").unwrap();
        assert_eq!(up.id().unwrap(), "b");
        assert_eq!(up.parent(), None);
        assert_eq!(down.id().unwrap(), "a");
        assert_eq!(up.frames().unwrap(), 50);
        assert_eq!(up.get("end-hash"), None);
        assert_eq!(up.checkpoints().unwrap(), vec![(20, "c20".to_string())]);
        assert_eq!(up.lines.len(), 3);
        assert_eq!(down.parent(), Some("b"));
        assert_eq!(down.frames().unwrap(), 50);
        assert_eq!(down.get("end-hash"), Some("x"));
        assert_eq!(down.checkpoints().unwrap(), vec![(30, "c80".to_string())]);
        // A and RIGHT were held across the split, so they are pressed again at 0.
        let k = |id| InputKey { port: 0, device: DEVICE_JOYPAD, index: 0, id };
        assert_eq!(down.binary_spans(k(8)), vec![(0, 50)]);
        assert_eq!(down.binary_spans(k(7)), vec![(0, 10)]);
        assert!(matches!(down.lines.last().unwrap().command, Command::Write { .. }));
        assert_eq!(down.lines.last().unwrap().frame, 20);

        let mut r = r;
        r.insert_line(30, "[EVENT] second").unwrap();
        let evs: Vec<_> = r.lines.iter().filter(|l| l.frame == 30).collect();
        assert_eq!(evs.len(), 2);
        let ln = evs[1].line_no;
        assert_eq!(r.line_command_text(ln).as_deref(), Some("[EVENT] second"));
        r.replace_line(ln, Some("[EVENT] renamed")).unwrap();
        assert!(r.insert_line(5, "[BOGUS]").is_err());
        let ln = r.lines.iter().find(|l| matches!(&l.command, Command::Event { text, .. } if text == "renamed")).unwrap().line_no;
        r.replace_line(ln, None).unwrap();
        assert_eq!(r.lines.iter().filter(|l| l.frame == 30).count(), 1);
    }

    #[test]
    fn edits_binary_spans() {
        let mut r = Replay::parse(SAMPLE).unwrap();
        let start = InputKey { port: 0, device: DEVICE_JOYPAD, index: 0, id: 3 };
        assert_eq!(r.binary_spans(start), vec![(0, 4)]);
        r.set_binary_spans(start, &[(2, 6), (5, 9), (90, 120)]).unwrap();
        assert_eq!(r.binary_spans(start), vec![(2, 9), (90, 100)]);
        // Other lines survive.
        assert_eq!(r.lines.iter().filter(|l| matches!(l.command, Command::Write { .. })).count(), 1);
        assert!(r.to_text().contains("2 P1 JOYPAD START 1\n"));
        assert!(!r.to_text().contains("100 P1 JOYPAD START 0"));
    }

    #[test]
    fn rejects_bad_files() {
        assert!(Replay::parse("nope").is_err());
        let past_end = SAMPLE.replace("frames: 100", "frames: 5");
        assert!(Replay::parse(&past_end).is_err());
        let unordered = SAMPLE.replace("4 P1 JOYPAD START 0", "0 P1 JOYPAD START 0").replace("0 P1 JOYPAD START 1", "3 P1 JOYPAD START 1");
        assert!(Replay::parse(&unordered).is_err());
        let unknown = SAMPLE.replace("[RESETX]", "").replace("9 [DEVICE] P2 MOUSE", "9 [TELEPORT] P2");
        assert!(Replay::parse(&unknown).is_err());
    }

    #[test]
    fn upgrades_old_files() {
        let h64 = "2f4d".repeat(16);
        let old = format!("emucov-replay 1\r\nid: a\nparent: none\nframes: 10\nend-hash: blake3-{h64}\ncheckpoint: 5 blake3-{h64}\n# end-hash: blake3-keep\n---\n# blake3-{h64}\n");
        let new = upgrade_text(&old);
        let h32 = &h64[..32];
        assert_eq!(new, format!("dj-replay 1\r\nid: a\nparent: none\nframes: 10\nend-hash: {h32}\ncheckpoint: 5 {h32}\n# end-hash: blake3-keep\n---\n# blake3-{h64}\n"));
        // The parser reads the old form the same way.
        let r = Replay::parse(&old).unwrap();
        assert_eq!(r.get("end-hash"), Some(h32));
        assert_eq!(r.checkpoints().unwrap(), vec![(5, h32.to_string())]);
    }

    #[test]
    fn renames_save_to_snapshot() {
        let h = "ab".repeat(16);
        let old = format!("dj-replay 1\nid: a\nparent: none\nframes: 10\nsave: blake3-{h}{h}\n---\n");
        assert_eq!(upgrade_text(&old), format!("dj-replay 1\nid: a\nparent: none\nframes: 10\nsnapshot: {h}\n---\n"));
        let r = Replay::parse(&old).unwrap();
        assert_eq!(r.get("snapshot"), Some(h.as_str()));
        assert!(r.to_text().contains(&format!("snapshot: {h}\n")));
    }
}
