//! BizHawk's `.bk2` movies: a zip of text files. `Header.txt` holds `key value` lines,
//! `Input Log.txt` a `LogKey:` line naming every column and one `|...|` line per frame
//! (https://tasvideos.org/Bizhawk/BK2Format, and BizHawk's `Bk2LogEntryGenerator.cs`).
//!
//! Reading follows the LogKey, whatever the core. Writing lays the columns out the way
//! BizHawk's controller definitions order them: the console's own buttons first, then each
//! player, axes before buttons. That layout comes from BizHawk's source; check a written
//! movie in BizHawk before relying on it.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{column_index, one_line, read_zip, rom_check_hex, text, write_zip, Frame, Meta, Movie, Start};

const HEADER: &str = "Header.txt";
const INPUT: &str = "Input Log.txt";
const COMMENTS: &str = "Comments.txt";
const SUBTITLES: &str = "Subtitles.txt";
const SAVE_RAM: &str = "MovieSaveRam.bin";
const SYNC_SETTINGS: &str = "SyncSettings.json";
const CORE_STATE: [&str; 2] = ["Core.bin", "CoreText.txt"];

/// One column of a log line, as the LogKey names it.
struct Col {
    name: String,
    group: usize,
}

/// Whether the movie's NES sync settings plug a Four Score into the left port.
fn nes_four_score(files: &BTreeMap<String, Vec<u8>>) -> bool {
    files
        .get(SYNC_SETTINGS)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
        .is_some_and(|v| v["o"]["Controls"]["NesLeftPort"] == "FourScore")
}

/// Swaps P2 and P3 in a column name: BizHawk's Four Score numbering to the console's, and
/// back.
fn four_score_player(name: &str) -> String {
    match name.split_once(' ') {
        Some(("P2", rest)) => format!("P3 {rest}"),
        Some(("P3", rest)) => format!("P2 {rest}"),
        _ => name.to_string(),
    }
}

/// NesHawk's sync settings with the given ports ("ControllerNES", "FourScore").
fn nes_sync_settings(left: &str, right: &str) -> String {
    serde_json::json!({ "o": {
        "$type": "BizHawk.Emulation.Cores.Nintendo.NES.NES+NESSyncSettings, BizHawk.Emulation.Cores",
        "BoardProperties": {},
        "RegionOverride": 0,
        "Controls": { "Famicom": false, "NesLeftPort": left, "NesRightPort": right, "FamicomExpPort": "UnpluggedFam" },
        "InitialWRamStatePattern": null,
        "VSDipswitches": (1..=8).map(|i| (format!("Dip_Switch_{i}"), serde_json::Value::Bool(false))).collect::<serde_json::Map<_, _>>(),
    }})
    .to_string()
}

/// Where a log column goes.
#[derive(Clone, Copy)]
enum Slot {
    Column(usize),
    Reset,
    Power,
    Skip,
}

/// `P1 A Up` and so on.
fn is_n64_stick_helper(name: &str) -> bool {
    name.split_once(' ').is_some_and(|(p, rest)| p.starts_with('P') && matches!(rest, "A Up" | "A Down" | "A Left" | "A Right"))
}

fn parse_log_key(key: &str) -> Vec<Col> {
    let mut cols = vec![];
    for (g, group) in key.split('#').filter(|g| !g.is_empty()).enumerate() {
        for name in group.split('|').filter(|n| !n.is_empty()) {
            cols.push(Col { name: name.to_string(), group: g });
        }
    }
    cols
}

/// An axis field at the start of `s` ("  -12,"): its value and length.
fn axis_field(s: &str) -> Option<(i16, usize)> {
    let end = s.find(',')?;
    if end > 7 {
        return None;
    }
    let v = s[..end].trim();
    if v.is_empty() || !v.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((v.parse::<i32>().ok()?.clamp(i16::MIN as i32, i16::MAX as i32) as i16, end + 1))
}

pub fn parse(bytes: &[u8]) -> Result<Movie> {
    // BizHawk may store any member zstd-compressed, as `<name>.zst` (`MovieSaveRam.bin.zst`).
    let mut files = BTreeMap::new();
    for (name, data) in read_zip(bytes)? {
        match name.strip_suffix(".zst") {
            Some(plain) => {
                let data = zstd::decode_all(&data[..]).with_context(|| format!("unpacking {name}"))?;
                files.insert(plain.to_string(), data);
            }
            None => {
                files.insert(name, data);
            }
        }
    }
    let header_text = text(&files, HEADER).ok_or_else(|| anyhow!("not a .bk2: it has no {HEADER}"))?;
    let header: BTreeMap<String, String> = header_text
        .lines()
        .filter_map(|l| {
            let l = l.trim_end_matches('\r');
            let (k, v) = l.split_once(' ').unwrap_or((l, ""));
            (!k.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect();
    let get = |k: &str| header.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let platform = get("Platform").ok_or_else(|| anyhow!("{HEADER} names no Platform"))?.to_string();
    let core = get("Core").unwrap_or("");
    let mut meta = Meta {
        emulator: ["BizHawk", get("emuVersion").unwrap_or("").trim_start_matches("Version "), core].iter().map(|w| one_line(w)).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "),
        author: get("Author").map(String::from),
        rerecords: get("rerecordCount").and_then(|v| v.parse().ok()),
        game: get("GameName").map(String::from),
        pal: get("PAL").is_some_and(|v| v.eq_ignore_ascii_case("true")),
        ..Default::default()
    };
    for k in ["SHA1", "SHA256", "MD5", "CRC32"] {
        if let Some(c) = get(k).and_then(rom_check_hex) {
            meta.rom.push(c);
        }
    }
    if let Some(c) = text(&files, COMMENTS) {
        meta.comments.extend(c.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from));
    }
    let truthy = |k: &str| get(k).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let start = if CORE_STATE.iter().any(|n| files.contains_key(*n)) || truthy("StartsFromSavestate") {
        Start::Savestate("BizHawk".into())
    } else if truthy("StartsFromSaveRam") {
        Start::SaveRam(files.get(SAVE_RAM).cloned().ok_or_else(|| anyhow!("the movie starts from save RAM but has no {SAVE_RAM}"))?)
    } else {
        Start::PowerOn
    };

    let log = text(&files, INPUT).ok_or_else(|| anyhow!("the movie has no {INPUT}"))?;
    let key = log.lines().find_map(|l| l.trim().strip_prefix("LogKey:")).ok_or_else(|| anyhow!("{INPUT} has no LogKey line, so its columns are unknown"))?;
    let cols = parse_log_key(key);
    let groups = cols.iter().map(|c| c.group).max().map_or(0, |g| g + 1);
    // Columns other than the console's Reset and Power become the movie's, except BizHawk
    // 2.6 to 2.11's N64 `A Up` and the like: on-screen helpers for the stick that the core
    // never reads (the axes already hold the stick).
    let mut columns = vec![];
    let mut slot = vec![];
    for c in &cols {
        match c.name.as_str() {
            "Reset" => slot.push(Slot::Reset),
            "Power" => slot.push(Slot::Power),
            n if platform == "N64" && is_n64_stick_helper(n) => slot.push(Slot::Skip),
            n => {
                slot.push(Slot::Column(columns.len()));
                columns.push(n.to_string());
            }
        }
    }
    if platform == "NES" && nes_four_score(&files) {
        // BizHawk numbers a Four Score's pads by port: P2 is the left port's second pad,
        // which the console reads as player 3. Columns are named by the console's players.
        for c in &mut columns {
            *c = four_score_player(c);
        }
    }
    if columns.iter().any(|c| c == "Input Length") {
        meta.warnings.push("the movie uses BizHawk's subframe input (Input Length), which dj can't play: each frame is taken whole".into());
    }
    let mut frames = vec![];
    for (n, line) in log.lines().map(|l| l.trim_end_matches('\r')).filter(|l| l.starts_with('|')).enumerate() {
        let parts: Vec<&str> = line[1..].split('|').collect();
        if parts.len() < groups {
            bail!("frame {n}: it has {} groups and the LogKey {groups}: `{line}`", parts.len());
        }
        let mut frame = Frame { values: vec![0; columns.len()], ..Default::default() };
        let mut ci = 0;
        for (g, part) in parts.iter().take(groups).enumerate() {
            let mut rest = *part;
            while ci < cols.len() && cols[ci].group == g {
                let value = match axis_field(rest) {
                    Some((v, len)) => {
                        rest = &rest[len..];
                        v
                    }
                    None => {
                        let c = rest.chars().next().ok_or_else(|| anyhow!("frame {n}: `{}` is missing from `{line}`", cols[ci].name))?;
                        rest = &rest[c.len_utf8()..];
                        i16::from(c != '.')
                    }
                };
                match slot[ci] {
                    Slot::Column(i) => frame.values[i] = value,
                    Slot::Reset => frame.reset = value != 0,
                    Slot::Power => frame.power = value != 0,
                    Slot::Skip => {}
                }
                ci += 1;
            }
        }
        frames.push(frame);
    }
    let mut markers = vec![];
    if let Some(s) = text(&files, SUBTITLES) {
        for l in s.lines() {
            // subtitle <frame> <x> <y> <length> <ARGB> <text>
            let w: Vec<&str> = l.trim().splitn(7, ' ').collect();
            if w.len() == 7 && w[0] == "subtitle" {
                if let Ok(f) = w[1].parse() {
                    markers.push((f, w[6].trim().to_string()));
                }
            }
        }
        markers.sort_by_key(|(f, _)| *f);
    }
    if meta.comments.iter().any(|c| c.starts_with("Exported by decomp-dj")) {
        meta.emulator = "decomp-dj".into();
    }
    Ok(Movie { platform, columns, frames, start, meta, markers })
}

/// A BizHawk core's column layout: groups of (name, mnemonic), axes written as numbers.
struct Layout {
    core: &'static str,
    groups: Vec<Vec<(String, char, bool)>>,
    /// The NES ports, as `SyncSettings.json` names them; BizHawk's defaults elsewhere.
    sync_settings: Option<String>,
    /// NES with a Four Score: BizHawk's player numbers differ from the console's.
    four_score: bool,
}

fn group(prefix: &str, buttons: &[(&str, char)], axes: &[&str]) -> Vec<(String, char, bool)> {
    let name = |n: &str| if prefix.is_empty() { n.to_string() } else { format!("{prefix} {n}") };
    axes.iter().map(|a| (name(a), ' ', true)).chain(buttons.iter().map(|(b, m)| (name(b), *m, false))).collect()
}

fn layout(platform: &str, players: usize) -> Result<Layout> {
    let console = || vec![("Reset".to_string(), 'r', false), ("Power".to_string(), 'P', false)];
    // NesHawk writes Power first.
    let nes_console = || vec![("Power".to_string(), 'P', false), ("Reset".to_string(), 'r', false)];
    const NES: [(&str, char); 8] = [("Up", 'U'), ("Down", 'D'), ("Left", 'L'), ("Right", 'R'), ("Start", 'S'), ("Select", 's'), ("B", 'B'), ("A", 'A')];
    // A Four Score's pads keep the order the hardware sends them in.
    const FOUR_SCORE: [(&str, char); 8] = [("A", 'A'), ("B", 'B'), ("Select", 's'), ("Start", 'S'), ("Up", 'U'), ("Down", 'D'), ("Left", 'L'), ("Right", 'R')];
    const SNES: [(&str, char); 12] = [("Up", 'U'), ("Down", 'D'), ("Left", 'L'), ("Right", 'R'), ("Select", 's'), ("Start", 'S'), ("Y", 'Y'), ("B", 'B'), ("X", 'X'), ("A", 'A'), ("L", 'l'), ("R", 'r')];
    const N64: [(&str, char); 14] = [
        ("DPad U", 'U'),
        ("DPad D", 'D'),
        ("DPad L", 'L'),
        ("DPad R", 'R'),
        ("Start", 'S'),
        ("Z", 'Z'),
        ("B", 'B'),
        ("A", 'A'),
        ("C Up", 'u'),
        ("C Down", 'd'),
        ("C Left", 'l'),
        ("C Right", 'r'),
        ("L", 'L'),
        ("R", 'R'),
    ];
    let plain = |core, groups| Layout { core, groups, sync_settings: None, four_score: false };
    Ok(match platform {
        "NES" if players > 2 => Layout {
            core: "NesHawk",
            groups: std::iter::once(nes_console()).chain((1..=4).map(|p| group(&format!("P{p}"), &FOUR_SCORE, &[]))).collect(),
            sync_settings: Some(nes_sync_settings("FourScore", "FourScore")),
            four_score: true,
        },
        "NES" => Layout {
            core: "NesHawk",
            groups: std::iter::once(nes_console()).chain((1..=2).map(|p| group(&format!("P{p}"), &NES, &[]))).collect(),
            sync_settings: Some(nes_sync_settings("ControllerNES", "ControllerNES")),
            four_score: false,
        },
        "SNES" => plain("BSNESv115+", std::iter::once(console()).chain((1..=players.max(2)).map(|p| group(&format!("P{p}"), &SNES, &[]))).collect()),
        "N64" => plain("Mupen64Plus", std::iter::once(console()).chain((1..=players.max(1)).map(|p| group(&format!("P{p}"), &N64, &["X Axis", "Y Axis"]))).collect()),
        "GBA" => plain(
            "mGBA",
            vec![group("", &[("Up", 'U'), ("Down", 'D'), ("Left", 'L'), ("Right", 'R'), ("Start", 'S'), ("Select", 's'), ("B", 'B'), ("A", 'A'), ("L", 'L'), ("R", 'R'), ("Power", 'P')], &["Tilt X", "Tilt Y", "Tilt Z", "Light Sensor"])],
        ),
        "GB" | "GBC" => plain("Gambatte", vec![group("", &[("Up", 'U'), ("Down", 'D'), ("Left", 'L'), ("Right", 'R'), ("Start", 'S'), ("Select", 's'), ("B", 'B'), ("A", 'A'), ("Power", 'P')], &[])]),
        p => bail!("a .bk2 for {p} isn't supported"),
    })
}

/// The highest player number with any input in the movie.
fn players_used(movie: &Movie) -> usize {
    let mut max = 0;
    for (i, c) in movie.columns.iter().enumerate() {
        if let Some(p) = c.strip_prefix('P').and_then(|r| r.split(' ').next()).and_then(|n| n.parse::<usize>().ok()) {
            if movie.frames.iter().any(|f| f.values[i] != 0) {
                max = max.max(p);
            }
        }
    }
    max
}

pub fn write(movie: &Movie, rom: Option<&[u8]>) -> Result<Vec<u8>> {
    let lay = layout(&movie.platform, players_used(movie))?;
    let idx = column_index(movie);
    // The movie column each BizHawk column holds.
    let source = |n: &str| if lay.four_score { four_score_player(n) } else { n.to_string() };
    let laid: std::collections::BTreeSet<String> = lay.groups.iter().flatten().map(|(n, _, _)| source(n)).collect();
    if let Some(c) = movie.columns.iter().enumerate().find(|(i, c)| !laid.contains(c.as_str()) && movie.frames.iter().any(|f| f.values[*i] != 0)) {
        bail!("a {} .bk2 has no `{}` (a multitap isn't supported)", movie.platform, c.1);
    }
    let rom = rom.ok_or_else(|| anyhow!("a .bk2 names its ROM's hash: set the project's ROM (--rom)"))?;
    // BizHawk names the ROM by the SHA-1 of the whole file, an iNES header included (a
    // published Super Painter movie matches the file this way and not without the header).
    let hashed = rom;
    let mut h = String::new();
    let _ = writeln!(h, "MovieVersion BizHawk v2.0.0");
    let _ = writeln!(h, "emuVersion {}", movie.meta.emulator);
    let _ = writeln!(h, "Platform {}", movie.platform);
    let _ = writeln!(h, "GameName {}", one_line(movie.meta.game.as_deref().unwrap_or("game")));
    let _ = writeln!(h, "Author {}", one_line(movie.meta.author.as_deref().unwrap_or("")));
    let _ = writeln!(h, "rerecordCount {}", movie.meta.rerecords.unwrap_or(0));
    let _ = writeln!(h, "SHA1 {}", super::hex(&super::sha1(hashed)).to_ascii_uppercase());
    let _ = writeln!(h, "Core {}", lay.core);
    let _ = writeln!(h, "StartsFromSavestate False");
    let _ = writeln!(h, "StartsFromSaveRam {}", if matches!(movie.start, Start::SaveRam(_)) { "True" } else { "False" });
    if movie.meta.pal {
        let _ = writeln!(h, "PAL True");
    }
    let mut log = String::from("[Input]\nLogKey:");
    for g in &lay.groups {
        log.push('#');
        for (n, _, _) in g {
            log.push_str(n);
            log.push('|');
        }
    }
    log.push('\n');
    // Where each BizHawk column's value comes from.
    enum From {
        Reset,
        Power,
        Column(Option<usize>),
    }
    let from: Vec<Vec<From>> = lay
        .groups
        .iter()
        .map(|g| {
            g.iter()
                .map(|(n, _, _)| match n.as_str() {
                    "Reset" => From::Reset,
                    "Power" => From::Power,
                    n => From::Column(idx.get(source(n).as_str()).copied()),
                })
                .collect()
        })
        .collect();
    for f in &movie.frames {
        log.push('|');
        for (g, from) in lay.groups.iter().zip(&from) {
            for ((_, m, axis), from) in g.iter().zip(from) {
                let v = match from {
                    From::Reset => f.reset as i16,
                    From::Power => f.power as i16,
                    From::Column(i) => i.map_or(0, |i| f.values[i]),
                };
                if *axis {
                    let _ = write!(log, "{v:>5},");
                } else {
                    log.push(if v != 0 { *m } else { '.' });
                }
            }
            log.push('|');
        }
        log.push('\n');
    }
    log.push_str("[/Input]\n");
    let mut files = vec![(HEADER, h.into_bytes()), (INPUT, log.into_bytes()), (COMMENTS, movie.meta.comments.join("\n").into_bytes())];
    let subs: String = movie.markers.iter().map(|(f, t)| format!("subtitle {f} 0 0 120 FFFFFFFF {}\n", one_line(t))).collect();
    files.push((SUBTITLES, subs.into_bytes()));
    if let Some(sync) = &lay.sync_settings {
        files.push((SYNC_SETTINGS, sync.clone().into_bytes()));
    }
    if let Start::SaveRam(data) = &movie.start {
        files.push((SAVE_RAM, data.clone()));
    }
    write_zip(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n64_movie() -> Movie {
        let p = crate::tas::profile::for_core("mupen64plus-next", "N64", &Default::default()).unwrap();
        let mut frames = vec![];
        for i in 0..5i16 {
            let mut v = vec![0i16; p.columns.len()];
            v[p.column("P1 A").unwrap()] = (i % 2) as i16;
            v[p.column("P1 X Axis").unwrap()] = i * 10 - 20;
            v[p.column("P1 Y Axis").unwrap()] = 127 - i;
            frames.push(Frame { values: v, reset: i == 3, power: false });
        }
        Movie { platform: "N64".into(), columns: p.columns.clone(), frames, start: Start::PowerOn, meta: Meta { comments: vec!["Exported by decomp-dj test".into()], ..Default::default() }, markers: vec![(2, "hi there".into())] }
    }

    #[test]
    fn n64_round_trip() {
        let m = n64_movie();
        let back = parse(&write(&m, Some(b"rom")).unwrap()).unwrap();
        assert_eq!(back.platform, "N64");
        assert_eq!(back.frames.len(), 5);
        assert_eq!(back.markers, m.markers);
        assert!(back.frames[3].reset);
        let bi = column_index(&back);
        for (a, b) in m.frames.iter().zip(&back.frames) {
            for (i, c) in m.columns.iter().enumerate() {
                if let Some(&j) = bi.get(c.as_str()) {
                    assert_eq!(a.values[i], b.values[j], "{c}");
                } else {
                    assert_eq!(a.values[i], 0, "{c}");
                }
            }
        }
    }

    #[test]
    fn reads_a_hand_written_nes_log() {
        let header = "MovieVersion BizHawk v2.0.0\nPlatform NES\nCore NesHawk\nSHA1 EA343F4E445A9050D4B4FBAC2C77D0693B1D0922\nAuthor someone\n";
        let log = "[Input]\nLogKey:#Reset|Power|#P1 Up|P1 Down|P1 Left|P1 Right|P1 Start|P1 Select|P1 B|P1 A|\n|..|........|\n|r.|....S..A|\n[/Input]\n";
        let z = write_zip(&[(HEADER, header.into()), (INPUT, log.into())]).unwrap();
        let m = parse(&z).unwrap();
        assert_eq!(m.columns.len(), 8);
        assert!(m.frames[1].reset);
        assert_eq!(m.frames[1].values, vec![0, 0, 0, 0, 1, 0, 0, 1]);
        assert_eq!(m.meta.author.as_deref(), Some("someone"));
        assert!(matches!(m.meta.rom[0], crate::tas::RomCheck::Sha1(_)));
    }

    #[test]
    fn four_score_numbers_players_as_bizhawk_does() {
        let p = crate::tas::profile::for_core("mesen", "NES", &Default::default()).unwrap();
        let at = |c: &str| p.columns.iter().position(|x| x == c).unwrap();
        let mut f = Frame { values: vec![0; p.columns.len()], ..Default::default() };
        f.values[at("P3 A")] = 1;
        f.values[at("P2 Start")] = 1;
        let m = Movie { platform: "NES".into(), columns: p.columns.clone(), frames: vec![f], start: Start::PowerOn, meta: Meta::default(), markers: vec![] };
        let files = read_zip(&write(&m, Some(b"NES\x1a")).unwrap()).unwrap();
        let log = text(&files, INPUT).unwrap();
        // Console player 3 is BizHawk's P2 (the left port's second pad), and 2 is its P3.
        assert!(log.contains("LogKey:#Power|Reset|#P1 A|P1 B|P1 Select|P1 Start|P1 Up|"), "{log}");
        assert!(log.contains("\n|..|........|A.......|...S....|........|\n"), "{log}");
        assert!(text(&files, SYNC_SETTINGS).unwrap().contains("\"NesLeftPort\":\"FourScore\""));
        let back = parse(&write(&m, Some(b"NES\x1a")).unwrap()).unwrap();
        let get = |c: &str| back.frames[0].values[back.columns.iter().position(|x| x == c).unwrap()];
        assert_eq!((get("P3 A"), get("P2 Start"), get("P2 A")), (1, 1, 0));
    }
}
