//! TAS movies: importing one as a root segment, and exporting a chain of segments as one
//! (.agents/TAS_PLAN.md, docs/tas.md).
//!
//! A movie reads into a [`Movie`] and writes from one, and segments convert through it
//! too. BizHawk's `.bk2` is the one format so far (`bk2`). A [`Profile`] maps a console's
//! buttons to the project core's libretro inputs.

pub mod bk2;
pub mod profile;

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::collection::Collection;
use crate::format::{Command, InputKey, Replay};
pub use profile::Profile;

/// A movie in memory: one record per frame, every button in every record.
#[derive(Clone, Debug)]
pub struct Movie {
    /// The console, as profiles name it: "NES", "GBA", "N64", "SNES", "GB", "GBC".
    pub platform: String,
    /// Button and axis names ("P1 A", "P1 X Axis"), as the console's profile names them.
    /// Each frame holds one value per column.
    pub columns: Vec<String>,
    pub frames: Vec<Frame>,
    pub start: Start,
    pub meta: Meta,
    /// Subtitles and other notes, by frame.
    pub markers: Vec<(u64, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// One per column: 1 or 0 for a button, the position for an axis.
    pub values: Vec<i16>,
    /// A soft reset before this frame runs.
    pub reset: bool,
    /// A power cycle before this frame runs.
    pub power: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    PowerOn,
    /// Battery-backed save RAM, loaded before frame 0.
    SaveRam(Vec<u8>),
    /// The recording emulator's own savestate, which dj can't load. Names the emulator.
    Savestate(String),
}

/// A hash a movie names its ROM by.
#[derive(Clone, Debug, PartialEq)]
pub enum RomCheck {
    Md5(Vec<u8>),
    Sha1(Vec<u8>),
    Crc32(u32),
}

#[derive(Clone, Debug, Default)]
pub struct Meta {
    /// The emulator and its version, as the file says: "BizHawk 2.9.1".
    pub emulator: String,
    pub author: Option<String>,
    pub rerecords: Option<u64>,
    /// The game or ROM name the movie gives.
    pub game: Option<String>,
    /// The hashes the movie names its ROM by.
    pub rom: Vec<RomCheck>,
    /// PAL timing.
    pub pal: bool,
    pub comments: Vec<String>,
    /// Anything else the author should know about this movie.
    pub warnings: Vec<String>,
}

/// The consoles a `.bk2` can hold.
const PLATFORMS: [&str; 5] = ["NES", "GBA", "N64", "SNES", "GB"];

/// Whether a movie for `platform` can hold a soft reset (BizHawk's GB and GBA cores have none).
fn carries_reset(platform: &str) -> bool {
    matches!(platform, "NES" | "N64" | "SNES")
}

/// Refuses a path that isn't a `.bk2`.
pub fn check_ext(path: &Path) -> Result<()> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("bk2") => Ok(()),
        Some(ext) => bail!("dj reads and writes BizHawk .bk2 movies, not .{ext}"),
        None => bail!("{} has no extension: dj reads and writes BizHawk .bk2 movies", path.display()),
    }
}

/// What import and export need from the project, beyond the movie.
pub struct Ctx<'a> {
    pub col: &'a Collection,
    /// The project's ROM, to check a movie against and to name in one.
    pub rom: Option<&'a [u8]>,
}

// ---- ROM hashes -------------------------------------------------------------------------

pub fn md5(data: &[u8]) -> Vec<u8> {
    use md5::Digest;
    md5::Md5::digest(data).to_vec()
}

pub fn sha1(data: &[u8]) -> Vec<u8> {
    use sha1::Digest;
    sha1::Sha1::digest(data).to_vec()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

/// A hash given in hex: what kind it is follows from its length (BizHawk's `SHA1` key holds
/// a CRC32, MD5 or SHA1, depending on the core).
pub fn rom_check_hex(s: &str) -> Option<RomCheck> {
    let b = unhex(s)?;
    Some(match b.len() {
        4 => RomCheck::Crc32(u32::from_be_bytes(b.try_into().unwrap())),
        16 => RomCheck::Md5(b),
        20 => RomCheck::Sha1(b),
        _ => return None,
    })
}

/// The NES ROM's PRG and CHR data without the iNES header and trainer: what FCEUX and
/// BizHawk hash. Any other file comes back whole.
pub fn nes_data(rom: &[u8]) -> &[u8] {
    if rom.len() >= 16 && &rom[..4] == b"NES\x1a" {
        let trainer = if rom[6] & 0x04 != 0 { 512 } else { 0 };
        let start = (16 + trainer).min(rom.len());
        let size = rom[4] as usize * 0x4000 + rom[5] as usize * 0x2000;
        &rom[start..(start + size).min(rom.len())]
    } else {
        rom
    }
}

/// The ROM's bytes as hashing tools may see them: whole, without an iNES header, without a
/// 512-byte SNES copier header.
fn rom_variants(rom: &[u8]) -> Vec<&[u8]> {
    let mut v = vec![rom];
    if rom.len() >= 16 && &rom[..4] == b"NES\x1a" {
        v.push(nes_data(rom));
        v.push(&rom[16..]);
    }
    if rom.len() % 1024 == 512 {
        v.push(&rom[512..]);
    }
    v
}

impl RomCheck {
    /// Whether the ROM matches, trying the forms other tools hash.
    pub fn matches(&self, rom: &[u8]) -> bool {
        let any = |f: &dyn Fn(&[u8]) -> bool| rom_variants(rom).into_iter().any(f);
        match self {
            RomCheck::Md5(h) => any(&|d| md5(d) == *h),
            RomCheck::Sha1(h) => any(&|d| sha1(d) == *h),
            RomCheck::Crc32(c) => any(&|d| crc32fast::hash(d) == *c),
        }
    }

    fn describe(&self) -> String {
        match self {
            RomCheck::Md5(h) => format!("MD5 {}", hex(h)),
            RomCheck::Sha1(h) => format!("SHA-1 {}", hex(h)),
            RomCheck::Crc32(c) => format!("CRC32 {c:08x}"),
        }
    }
}

// ---- zip --------------------------------------------------------------------------------

/// Every file in a zip archive, by name.
pub fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("not a zip archive")?;
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let mut data = vec![];
        f.read_to_end(&mut data)?;
        out.insert(f.name().to_string(), data);
    }
    Ok(out)
}

pub fn write_zip(files: &[(&str, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        z.start_file(*name, opts)?;
        z.write_all(data)?;
    }
    Ok(z.finish()?.into_inner())
}

/// A zip member as text.
pub fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> Option<String> {
    files.get(name).map(|b| String::from_utf8_lossy(b).trim_start_matches('\u{feff}').to_string())
}

// ---- import -----------------------------------------------------------------------------

/// A movie turned into a root segment, not saved yet.
pub struct Imported {
    pub replay: Replay,
    pub movie: Movie,
    /// Things the author should know: the ROM didn't match, core options differ, and so on.
    pub warnings: Vec<String>,
}

/// Reads a movie and turns it into a new root segment. `strict` makes a ROM mismatch an
/// error. The segment isn't added to the project: the caller saves it, or keeps it unsaved.
pub fn import(ctx: &Ctx, file: &Path, title: Option<&str>, strict: bool) -> Result<Imported> {
    check_ext(file)?;
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let col = ctx.col;
    let mut movie = bk2::parse(&bytes).with_context(|| format!("reading {}", file.display()))?;
    if movie.meta.emulator.is_empty() {
        movie.meta.emulator = "BizHawk".to_string();
    }
    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("movie").to_string();
    let profile = profile::for_core(&col.config.core.name, &movie.platform, &col.config.core.options)?;
    let mut warnings = profile.check_options(&col.config.core.options);
    warnings.append(&mut movie.meta.warnings);
    if let Some(n) = opposite_directions(&movie) {
        let core = &col.config.core.name;
        if core == "mesen" && col.config.core.options.get("mesen_allow_invalid_input").map(String::as_str) != Some("enabled") {
            warnings.push(format!(
                "the movie holds Left+Right or Up+Down at once on {n} frames, which Mesen reads as neither: set the core option mesen_allow_invalid_input = \"enabled\" for it to play as recorded"
            ));
        }
    }
    if !movie.meta.rom.is_empty() {
        match ctx.rom {
            Some(rom) if !movie.meta.rom.iter().any(|c| c.matches(rom)) => {
                let named = movie.meta.rom.iter().map(RomCheck::describe).collect::<Vec<_>>().join(", ");
                let msg = format!("the movie was made with a different ROM (it names {named}): a different dump or revision desyncs at once");
                if strict {
                    bail!("{msg}");
                }
                warnings.push(msg);
            }
            Some(_) => {}
            None => warnings.push("no ROM path is set for this project, so the movie's ROM wasn't checked".into()),
        }
    } else {
        warnings.push("the movie names no ROM hash, so it wasn't checked".into());
    }
    if movie.meta.pal {
        warnings.push("the movie uses PAL timing: it only plays the same if the core runs the PAL region too".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let title = match title {
        Some(t) => t.to_string(),
        None => {
            let game = movie.meta.game.clone().filter(|g| !g.is_empty()).unwrap_or_else(|| file.file_stem().and_then(|s| s.to_str()).unwrap_or("movie").to_string());
            match &movie.meta.author {
                Some(a) if !a.is_empty() => format!("{game} (TAS by {a})"),
                _ => format!("{game} (TAS)"),
            }
        }
    };
    let (mut replay, mut more) = to_segment(&movie, &profile, &id, &title, &file_name)?;
    warnings.append(&mut more);
    match &movie.start {
        Start::PowerOn => {}
        Start::SaveRam(data) => {
            let blob = col.write_blob(data, "sram")?;
            replay.set("sram", &blob);
        }
        Start::Savestate(_) => unreachable!("to_segment refuses it"),
    }
    Ok(Imported { replay, movie, warnings })
}

/// The segment a movie plays as: a root segment whose lines change inputs only where the
/// movie's records do. It refuses what a segment can't hold, naming the frame.
pub fn to_segment(movie: &Movie, profile: &Profile, id: &str, title: &str, file_name: &str) -> Result<(Replay, Vec<String>)> {
    if let Start::Savestate(who) = &movie.start {
        bail!("the movie starts from {who}'s own savestate, which dj can't load: only movies that start from power-on or from save RAM can be imported");
    }
    if movie.frames.is_empty() {
        bail!("the movie has no frames");
    }
    // The profile column each movie column feeds, if any.
    let cols: Vec<Option<usize>> = movie.columns.iter().map(|c| profile.column(c)).collect();
    let mut unmapped: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lossy = 0usize;
    let mut markers = movie.markers.iter().peekable();
    let mut body = vec![format!("0 [EVENT] Imported from {file_name} ({}); sync not checked", movie.meta.emulator)];
    let mut held: BTreeMap<InputKey, i16> = profile.keys().into_iter().map(|k| (k, 0)).collect();
    let mut values = vec![0i16; profile.columns.len()];
    for (f, frame) in movie.frames.iter().enumerate() {
        let f = f as u64;
        while let Some((_, text)) = markers.next_if(|(mf, _)| *mf <= f) {
            body.push(format!("{f} [EVENT] {}", one_line(text)));
        }
        if frame.power && f > 0 {
            bail!("the movie power-cycles the console at frame {f}, and a segment has no power-cycle command");
        }
        // A reset on frame 0 comes right after power-on, where BizHawk's conversions of
        // FCEUX movies put one. It changes nothing there, but on Mesen it delays the game by
        // 5 frames, so it is left out.
        if frame.reset && f > 0 {
            body.push(format!("{f} [RESET]"));
        }
        values.iter_mut().for_each(|v| *v = 0);
        for (i, &v) in frame.values.iter().enumerate() {
            match cols[i] {
                Some(c) => values[c] = v,
                None if v != 0 => *unmapped.entry(&movie.columns[i]).or_default() += 1,
                None => {}
            }
        }
        let lr = profile.to_libretro(&values);
        lossy += lr.lossy as usize;
        for (key, v) in lr.inputs {
            let h = held.entry(key).or_insert(0);
            if *h != v {
                body.push(format!("{f} {}", crate::format::input_text(key, v)));
                *h = v;
            }
        }
    }
    let mut warnings = vec![];
    if !unmapped.is_empty() {
        let list = unmapped.iter().map(|(c, n)| format!("{c} ({n} frames)")).collect::<Vec<_>>().join(", ");
        warnings.push(format!("left out inputs the project's core has no button for: {list}"));
    }
    if lossy > 0 {
        let stick = movie.columns.iter().enumerate().filter(|(_, c)| c.ends_with(" Axis")).flat_map(|(i, _)| movie.frames.iter().map(move |f| f.values[i].unsigned_abs())).max().unwrap_or(0);
        // Mupen64Plus-Next reaches 80 straight ahead at sensitivity 100, and 120 at 150.
        let why = if stick > 80 {
            format!("the stick goes as far as {stick}, past the 80 Mupen64Plus-Next reaches straight ahead at sensitivity 100 (120 at 150, its highest); or C-Left and C-Right are held at once")
        } else {
            "C-Left and C-Right (or C-Up and C-Down) are held at once, which the right stick can't do, or a stick position falls between what the deadzone and sensitivity reach".to_string()
        };
        warnings.push(format!("on {lossy} frames the core can't read the movie's input exactly: {why}. Those frames may play differently"));
    }
    let mut r = Replay::new(id, None, title, movie.frames.len() as u64);
    r.set("tags", "tas imported");
    r.set("tas-source", &one_line(&format!("bk2 {}", movie.meta.emulator)));
    r.set("tas-file", &one_line(file_name));
    if let Some(a) = &movie.meta.author {
        r.set("tas-author", &one_line(a));
    }
    if let Some(n) = movie.meta.rerecords {
        r.set("tas-rerecords", &n.to_string());
    }
    let mut text = r.to_text();
    for l in body {
        text.push_str(&l);
        text.push('\n');
    }
    Ok((Replay::parse(&text).context("the imported segment didn't read back")?, warnings))
}

/// The frames on which a pad holds Left and Right, or Up and Down, at once, if any.
fn opposite_directions(movie: &Movie) -> Option<usize> {
    let at = |name: String| movie.columns.iter().position(|c| *c == name);
    let mut pairs = vec![];
    for c in &movie.columns {
        if let Some(prefix) = c.strip_suffix("Left") {
            if let (Some(l), Some(r)) = (at(format!("{prefix}Left")), at(format!("{prefix}Right"))) {
                pairs.push((l, r));
            }
            if let (Some(u), Some(d)) = (at(format!("{prefix}Up")), at(format!("{prefix}Down"))) {
                pairs.push((u, d));
            }
        }
    }
    let n = movie.frames.iter().filter(|f| pairs.iter().any(|&(a, b)| f.values[a] != 0 && f.values[b] != 0)).count();
    (n > 0).then_some(n)
}

pub fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ").trim().to_string()
}

// ---- export -----------------------------------------------------------------------------

/// Where an export ends: the segment, and the last of its frames to include.
#[derive(Clone, Copy)]
pub struct ExportEnd<'a> {
    pub segment: &'a str,
    /// The last frame of `segment` to include (the playhead's frame); `None` for its last.
    pub through: Option<u64>,
}

/// An exported movie, ready to write.
pub struct Exported {
    pub bytes: Vec<u8>,
    pub frames: usize,
    pub segments: usize,
    pub warnings: Vec<String>,
}

/// Exports the chain from the root down to `end` as a `.bk2`. What a movie can't carry is
/// left out and summed up in one warning, or with `strict` refused.
pub fn export(ctx: &Ctx, end: ExportEnd, strict: bool, author: Option<&str>) -> Result<Exported> {
    let (mut movie, warnings) = from_chain(ctx.col, end, ctx.rom, strict)?;
    movie.meta.author = author.map(one_line).filter(|a| !a.is_empty());
    let segments = ctx.col.chain(end.segment)?.len();
    let frames = movie.frames.len();
    let bytes = bk2::write(&movie, ctx.rom)?;
    Ok(Exported { bytes, frames, segments, warnings })
}

/// The chain from the root segment down to `end` as one movie: what the core sees on each
/// frame, with every segment starting from all inputs released, as it does when played.
/// `rom` tells a Game Boy game from a GBA one on mGBA.
///
/// What a movie can't carry (a snapshot start, resets on GB and GBA, `[WRITE]`, `[RNG]`,
/// `[DEVICE]`, inputs with no button) is left out, and summed up in one warning; `strict`
/// refuses it.
pub fn from_chain(col: &Collection, end: ExportEnd, rom: Option<&[u8]>, strict: bool) -> Result<(Movie, Vec<String>)> {
    let chain = col.chain(end.segment)?;
    let platform = profile::platform_of(&col.config.core.name, rom).ok_or_else(|| anyhow!("TAS movies aren't supported for the core `{}` yet", col.config.core.name))?;
    if !PLATFORMS.contains(&platform) {
        bail!("a .bk2 can't hold a {platform} game yet ({} only)", PLATFORMS.join(", "));
    }
    let profile = profile::for_core(&col.config.core.name, platform, &col.config.core.options)?;
    let platform = profile.platform.clone();
    let mut warnings = profile.check_options(&col.config.core.options);
    let keys = profile.keys();
    let root = &col.replays[chain[0]];
    let mut left_out = LeftOut { snapshot: root.get("snapshot").is_some(), ..Default::default() };
    let start = match root.get("sram") {
        Some(b) => Start::SaveRam(col.read_blob(b, "sram")?),
        None => Start::PowerOn,
    };
    let carries_reset = carries_reset(&platform);
    let mut frames = vec![];
    let mut markers = vec![];
    let mut dirty = 0;
    for (n, id) in chain.iter().enumerate() {
        let seg = &col.replays[*id];
        let mut len = seg.frames()?;
        if n + 1 == chain.len() {
            if let Some(t) = end.through {
                if t >= len {
                    bail!("frame {t} is past the end of \"{}\" ({len} frames)", seg.title());
                }
                len = t + 1;
            }
        }
        if col.status(id) != crate::collection::Status::Clean {
            dirty += 1;
        }
        let base = frames.len() as u64;
        // Every segment starts with all inputs released (docs/segments.md#frames).
        let mut held: BTreeMap<InputKey, i16> = BTreeMap::new();
        let mut lines = seg.lines.iter().peekable();
        for f in 0..len {
            let mut frame = Frame::default();
            while let Some(l) = lines.next_if(|l| l.frame == f) {
                match &l.command {
                    Command::Input { key, value, .. } => {
                        if keys.contains(key) {
                            held.insert(*key, *value);
                        } else if *value != 0 {
                            left_out.inputs += 1;
                        }
                    }
                    Command::Reset if carries_reset => frame.reset = true,
                    Command::Reset => left_out.resets += 1,
                    Command::Event { text, .. } => markers.push((base + f, text.clone())),
                    Command::Screenshot => {}
                    Command::Write { .. } => left_out.writes += 1,
                    Command::Rng { .. } => left_out.rng += 1,
                    Command::Device { .. } => left_out.devices += 1,
                }
            }
            frame.values = profile.from_libretro(&|k| held.get(&k).copied().unwrap_or(0));
            frames.push(frame);
        }
    }
    if let Some(msg) = left_out.summary(&platform) {
        if strict {
            bail!("not exported (--strict): {msg}");
        }
        warnings.push(msg);
    }
    if dirty > 0 {
        warnings.push(format!("{dirty} of the {} segments are dirty, so nothing has checked that the chain still plays as recorded", chain.len()));
    }
    let last = &col.replays[*chain.last().unwrap()];
    let mut meta = Meta { emulator: format!("decomp-dj {}", env!("CARGO_PKG_VERSION")), game: Some(col.config.name.clone()), ..Default::default() };
    meta.comments.push(format!("Exported by decomp-dj from segment {} \"{}\"{}, {} frames", end.segment, one_line(last.title()), end.through.map(|t| format!(" through frame {t}")).unwrap_or_default(), frames.len()));
    // A Game Boy Color game is a GBC movie.
    let platform = match (platform.as_str(), rom) {
        ("GB", Some(r)) if r.get(0x143).is_some_and(|b| b & 0x80 != 0) => "GBC".to_string(),
        _ => platform,
    };
    let movie = Movie { platform, columns: profile.columns.clone(), frames, start, meta, markers };
    Ok((movie, warnings))
}

/// What an export leaves out because the movie can't carry it.
#[derive(Default)]
struct LeftOut {
    snapshot: bool,
    resets: usize,
    writes: usize,
    rng: usize,
    devices: usize,
    inputs: usize,
}

impl LeftOut {
    /// One message for all of it, or None when nothing was left out.
    fn summary(&self, platform: &str) -> Option<String> {
        let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut items = vec![];
        if self.resets > 0 {
            items.push(count(self.resets, "reset", "resets"));
        }
        if self.writes > 0 {
            items.push(count(self.writes, "memory write", "memory writes"));
        }
        if self.rng > 0 {
            items.push(count(self.rng, "RNG advance", "RNG advances"));
        }
        if self.devices > 0 {
            items.push(count(self.devices, "controller change", "controller changes"));
        }
        if self.inputs > 0 {
            items.push(format!("{} with no {platform} button (turbo, mouse and the like)", count(self.inputs, "input", "inputs")));
        }
        let starts = self.snapshot.then_some("starts from a snapshot");
        let mut msg = String::from("this movie may not play as expected in another emulator, because the chain ");
        match (starts, items.is_empty()) {
            (None, true) => return None,
            (Some(s), true) => msg.push_str(s),
            (s, false) => {
                if let Some(s) = s {
                    msg.push_str(s);
                    msg.push_str(" and ");
                }
                msg.push_str("contains:");
                for i in items {
                    msg.push_str("\n  * ");
                    msg.push_str(&i);
                }
            }
        }
        Some(msg)
    }
}

/// The column index of each name in `names`, for writers that lay columns out their own way.
pub fn column_index(movie: &Movie) -> BTreeMap<&str, usize> {
    movie.columns.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect()
}
