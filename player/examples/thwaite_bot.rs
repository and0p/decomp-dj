//! Scratch tool for the Thwaite decomp (.agents/_archive/THWAITE_DECOMP_PLAN.md): plays the original
//! from the start of an empty segment, choosing the joypad itself from what is in RAM, and
//! writes what it pressed into that segment's file. Used to record segments that reach
//! balloons, MIRVs and the end of a level, which take aimed shots.
//!
//! Each step of the plan is one of:
//!   click:X,Y   move player 1's crosshair to (X, Y) and press A
//!   press:BTN   press a joypad button for 4 frames (START, SELECT, ...)
//!   wait:N      do nothing for N frames
//!   defend      shoot down enemy missiles until the level ends (gameState goes back to 0)
//!   defend:N    the same for at most N frames
//!   defend:N:T  ... and let the missiles aimed at building T through (2, 9: the silos;
//!               several: T,U)
//! The segment's frame count is set to where the plan ends, plus 30 frames.
//!
//! Usage: thwaite_bot <project> <rom> <core> <segment> <step> [<step>...]
use anyhow::{anyhow, bail, Result};
use decomp_dj::collection::Collection;
use decomp_dj::format::InputKey;
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::player::Session;
use std::collections::BTreeMap;
use std::path::Path;

const JOYPAD: [&str; 9] = ["B", "Y", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT", "A"];

const GAME_STATE: usize = 0x38;
const HOUSES: usize = 0x3CC;
const SILO_LEFT: usize = 0x3E6;
const ENEMY_LEFT: usize = 0x305;
const M_XHI: usize = 0x3EA;
const M_YHI: usize = 0x412;
const M_DXHI: usize = 0x43A;
const M_DXLO: usize = 0x44E;
const M_DYHI: usize = 0x462;
const M_DYLO: usize = 0x476;
const M_TYPE: usize = 0x49E;
const CH_X: usize = 0x4E0;
const CH_Y: usize = 0x4E4;
const CH_DXHI: usize = 0x4E8;
const CH_DXLO: usize = 0x4E6;
const CH_DYHI: usize = 0x4EC;
const CH_DYLO: usize = 0x4EA;

fn s16(hi: u8, lo: u8) -> f32 {
    (((hi as u16) << 8 | lo as u16) as i16) as f32 / 256.0
}

/// Which way to push one axis of the crosshair to stop at `d` pixels away, given velocity
/// `v` (pixels a frame): toward it until the braking distance reaches it, then nothing
/// (letting go brakes, at 160/256 px a frame per frame).
fn push(d: f32, v: f32) -> i32 {
    let brake = 160.0 / 256.0;
    let stop = v * v.abs() / (2.0 * brake);
    let left = d - stop;
    if left.abs() < 2.0 {
        0
    } else if left > 0.0 {
        1
    } else {
        -1
    }
}

struct Bot {
    held: BTreeMap<&'static str, bool>,
    lines: Vec<String>,
}

impl Bot {
    fn set(&mut self, s: &mut Session, frame: u64, b: &'static str, on: bool) {
        if self.held.get(b).copied().unwrap_or(false) == on {
            return;
        }
        self.held.insert(b, on);
        let id = JOYPAD.iter().position(|x| *x == b).unwrap() as u32;
        s.core.set_input(InputKey { port: 0, device: 1, index: 0, id }, on as i16);
        self.lines.push(format!("{frame} P1 JOYPAD {b} {}", on as u8));
    }
    fn steer(&mut self, s: &mut Session, frame: u64, ram: &[u8], tx: f32, ty: f32) -> bool {
        let (x, y) = (ram[CH_X] as f32, ram[CH_Y] as f32);
        let px = push(tx - x, s16(ram[CH_DXHI], ram[CH_DXLO]));
        let py = push(ty - y, s16(ram[CH_DYHI], ram[CH_DYLO]));
        self.set(s, frame, "RIGHT", px > 0);
        self.set(s, frame, "LEFT", px < 0);
        self.set(s, frame, "DOWN", py > 0);
        self.set(s, frame, "UP", py < 0);
        (tx - x).abs() < 4.0 && (ty - y).abs() < 4.0
    }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        bail!("usage: thwaite_bot <project> <rom> <core> <segment> <step>...");
    }
    let mut col = Collection::open(Path::new(&a[1]))?;
    let id = col.resolve(&a[4])?.to_string();
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    s.enter_replay(&col, &id)?;
    let mut bot = Bot { held: BTreeMap::new(), lines: vec![] };
    let mut frame = 0u64;
    let ram = |s: &mut Session| -> Result<Vec<u8>> { Ok(s.core.memory(MEMORY_SYSTEM_RAM).ok_or_else(|| anyhow!("no RAM"))?.to_vec()) };
    let mut seen_types = [0u32; 3];
    let mut cooldown: [u64; 20] = [0; 20];
    let mut shots: Vec<(f32, f32, u64)> = vec![];
    for step in &a[5..] {
        let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match verb {
            "wait" => {
                for _ in 0..arg.parse::<u64>()? {
                    s.core.run_frame(false);
                    frame += 1;
                }
            }
            "press" => {
                let b = *JOYPAD.iter().find(|x| **x == arg).ok_or_else(|| anyhow!("no button {arg}"))?;
                for k in 0..8 {
                    bot.set(&mut s, frame, b, k < 4);
                    s.core.run_frame(false);
                    frame += 1;
                }
            }
            "click" => {
                let (x, y) = arg.split_once(',').ok_or_else(|| anyhow!("click:X,Y"))?;
                let (tx, ty): (f32, f32) = (x.parse()?, y.parse()?);
                let mut settled = 0;
                for _ in 0..400 {
                    let r = ram(&mut s)?;
                    let there = bot.steer(&mut s, frame, &r, tx, ty);
                    let still = s16(r[CH_DXHI], r[CH_DXLO]) == 0.0 && s16(r[CH_DYHI], r[CH_DYLO]) == 0.0;
                    settled = if there && still { settled + 1 } else { 0 };
                    s.core.run_frame(false);
                    frame += 1;
                    if settled >= 3 {
                        break;
                    }
                }
                for b in ["RIGHT", "LEFT", "UP", "DOWN"] {
                    bot.set(&mut s, frame, b, false);
                }
                for k in 0..8 {
                    bot.set(&mut s, frame, "A", k < 4);
                    s.core.run_frame(false);
                    frame += 1;
                }
                let r = ram(&mut s)?;
                println!("frame {frame}: clicked at ({}, {})", r[CH_X], r[CH_Y]);
            }
            "defend" => {
                let (lim, spare) = arg.split_once(':').unwrap_or((arg, ""));
                let limit: u64 = if lim.is_empty() { 20000 } else { lim.parse()? };
                let spare: Vec<u8> = spare.split(',').filter(|v| !v.is_empty()).map(|v| v.parse()).collect::<std::result::Result<_, _>>()?;
                let mut was_active = false;
                for k in 0..limit {
                    let r = ram(&mut s)?;
                    let state = r[GAME_STATE];
                    if state != 0 {
                        was_active = true;
                    } else if was_active {
                        println!("frame {frame}: gameState is 0 again");
                        break;
                    }
                    for m in 4..20 {
                        if r[M_YHI + m] != 0 {
                            seen_types[(r[M_TYPE + m] as usize).min(2)] += 1;
                        }
                    }
                    // the lowest enemy missile that is low enough, and not shot at lately
                    let target = (4..20)
                        .filter(|&m| r[M_YHI + m] >= 96 && cooldown[m] <= frame && !spare.contains(&r[0x4B2 + m]))
                        .filter(|&m| {
                            // already covered: near an explosion or a shot still on its way
                            let (mx, my) = (r[M_XHI + m] as f32, r[M_YHI + m] as f32);
                            let near = |x: f32, y: f32, d: f32| (mx - x).hypot(my - y) < d;
                            let boom = (0..8).any(|e| r[0x500 + e] != 0 && near(r[0x4F0 + e] as f32, r[0x4F8 + e] as f32, 20.0));
                            let shot = shots.iter().any(|&(x, y, f)| frame < f + 70 && near(x, y, 22.0));
                            !boom && !shot
                        })
                        .max_by_key(|&m| r[M_YHI + m]);
                    let mut fire = None;
                    if let Some(m) = target {
                        let lead = 22.0;
                        let tx = r[M_XHI + m] as f32 + s16(r[M_DXHI + m], r[M_DXLO + m]) * lead;
                        let ty = r[M_YHI + m] as f32 + s16(r[M_DYHI + m], r[M_DYLO + m]) * lead;
                        let (tx, ty) = (tx.clamp(8.0, 247.0), ty.clamp(16.0, 175.0));
                        bot.steer(&mut s, frame, &r, tx, ty);
                        let (dx, dy) = (tx - r[CH_X] as f32, ty - r[CH_Y] as f32);
                        if dx.abs() < 8.0 && dy.abs() < 8.0 {
                            let left_ok = r[SILO_LEFT] > 0 && r[HOUSES + 2] != 0;
                            let right_ok = r[SILO_LEFT + 1] > 0 && r[HOUSES + 9] != 0;
                            fire = match (tx < 128.0, left_ok, right_ok) {
                                (true, true, _) | (false, true, false) => Some("B"),
                                (_, _, true) => Some("A"),
                                _ => None,
                            };
                            if fire.is_some() {
                                shots.push((tx, ty, frame));
                                cooldown[m] = frame + 50;
                            }
                        }
                    } else {
                        // park in the middle
                        bot.steer(&mut s, frame, &r, 128.0, 110.0);
                    }
                    let held_b = bot.held.get("B").copied().unwrap_or(false);
                    let held_a = bot.held.get("A").copied().unwrap_or(false);
                    bot.set(&mut s, frame, "B", fire == Some("B") && !held_b);
                    bot.set(&mut s, frame, "A", fire == Some("A") && !held_a);
                    if k % 300 == 0 {
                        let houses: String = r[HOUSES..HOUSES + 12].iter().map(|h| if *h != 0 { '#' } else { '.' }).collect();
                        println!("frame {frame}: state {state} enemies left {} silos {}/{} houses {houses}", r[ENEMY_LEFT], r[SILO_LEFT], r[SILO_LEFT + 1]);
                    }
                    s.core.run_frame(false);
                    frame += 1;
                }
                for b in ["RIGHT", "LEFT", "UP", "DOWN", "A", "B"] {
                    bot.set(&mut s, frame, b, false);
                }
                println!("frame {frame}: missile-frames seen: normal {} MIRV {} balloon {}", seen_types[0], seen_types[1], seen_types[2]);
            }
            _ => bail!("unknown step {step}"),
        }
    }
    let r = ram(&mut s)?;
    let houses: String = r[HOUSES..HOUSES + 12].iter().map(|h| if *h != 0 { '#' } else { '.' }).collect();
    println!("end, frame {frame}: state {} houses {houses} score {:02X}{:02X}", r[GAME_STATE], r[0x3D9], r[0x3DA]);

    // write the lines into the segment's file, after its header
    let path = col.paths.get(&id).cloned().ok_or_else(|| anyhow!("no file for {id}"))?;
    let text = std::fs::read_to_string(&path)?;
    let (head, _) = text.split_once("\n---\n").ok_or_else(|| anyhow!("no --- in {}", path.display()))?;
    let head: Vec<String> = head.lines().map(|l| if l.starts_with("frames:") { format!("frames: {}", frame + 30) } else { l.to_string() }).collect();
    std::fs::write(&path, format!("{}\n---\n{}\n", head.join("\n"), bot.lines.join("\n")))?;
    println!("wrote {} lines to {}", bot.lines.len(), path.display());
    let _ = &mut col;
    Ok(())
}
