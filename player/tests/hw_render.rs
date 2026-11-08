//! Cores that draw with OpenGL (SET_HW_RENDER), with the small GL core in tests/testcore
//! (built here with `cc`). Skipped where there's no C compiler or no OpenGL without a window.

use decomp_dj::libretro::{Core, Frame, BASELINE, COMPARISON, MEMORY_SYSTEM_RAM};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn build(dir: &Path, name: &str) -> Option<PathBuf> {
    let lib = dir.join(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/glcore.c");
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).status().ok()?.success();
    ok.then_some(lib)
}

fn pixel(f: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * f.width as usize + x) * 3;
    [f.rgb[i], f.rgb[i + 1], f.rgb[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 2)
}

/// The colour the core clears to on the frame where its counter is `n`.
fn clear_colour(n: u8) -> [u8; 3] {
    [((n % 8) as f32 / 7.0 * 255.0).round() as u8, 64, 128]
}

fn opts(profile: &str, origin: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("glcore_profile".to_string(), profile.to_string()), ("glcore_origin".to_string(), origin.to_string())])
}

fn check_frame(f: &Frame, n: u8, what: &str) {
    assert_eq!((f.width, f.height), (64, 48), "{what}");
    assert!(near(pixel(f, 4, 4), [255, 255, 255]), "{what}: top-left should be white, got {:?}", pixel(f, 4, 4));
    assert!(near(pixel(f, 60, 44), clear_colour(n)), "{what}: bottom-right should be {:?}, got {:?}", clear_colour(n), pixel(f, 60, 44));
    assert!(near(pixel(f, 60, 4), clear_colour(n)), "{what}: top-right should be {:?}", clear_colour(n));
}

#[test]
fn opengl_cores() {
    let probe = decomp_dj::gl::Request { core_profile: true, major: 3, minor: 2, depth: true, stencil: true, debug: false };
    if let Some(why) = decomp_dj::gl::unsupported(&probe) {
        eprintln!("skipping: {why}");
        return;
    }
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("hw-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(lib), Some(lib2)) = (build(&tmp, "glcore.so"), build(&tmp, "glcore2.so")) else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let rom = tmp.join("rom.bin");
    std::fs::write(&rom, b"rom").unwrap();

    // Both profiles, and both origins: the picture comes out the right way up.
    for (profile, origin) in [("core", "bottom"), ("core", "top"), ("compat", "bottom")] {
        let what = format!("{profile} profile, {origin} origin");
        let mut core = match Core::load_in(BASELINE, &lib, &tmp.join("work"), &opts(profile, origin)) {
            Ok(c) => c,
            Err(e) => panic!("{what}: {e:#}"),
        };
        if let Err(e) = core.load_game(&rom) {
            if profile == "compat" {
                // Some drivers have no compatibility profile.
                eprintln!("{what}: {e:#}");
                continue;
            }
            panic!("{what}: {e:#}");
        }
        assert!(core.gpu().is_some(), "{what}: no GL context");
        let resets: extern "C" fn() -> i32 = unsafe { std::mem::transmute(core.proc("glcore_resets")) };
        assert_eq!(resets(), 1, "{what}: context_reset once");
        for n in 1..=3u8 {
            let f = core.run_frame(true).expect("a frame");
            check_frame(&f, n, &what);
        }
        // Frames not captured aren't read back, and still run.
        assert!(core.run_frame(false).is_none());
        let state = core.serialize().unwrap();
        core.run_frame(false);
        core.unserialize(&state).unwrap();
        let f = core.run_frame(true).unwrap();
        check_frame(&f, 5, &format!("{what}, after loading a state"));
    }

    // Two GL cores at once, one per slot, each with its own context.
    let mut a = Core::load_in(BASELINE, &lib, &tmp.join("a"), &opts("core", "bottom")).unwrap();
    a.load_game(&rom).unwrap();
    let mut b = Core::load_in(COMPARISON, &lib2, &tmp.join("b"), &opts("core", "top")).unwrap();
    b.load_game(&rom).unwrap();
    for n in 1..=4u8 {
        let fa = a.run_frame(true).unwrap();
        check_frame(&fa, n, "baseline slot");
        // The comparison core runs twice as often: their counters must stay apart.
        b.run_frame(false);
        let fb = b.run_frame(true).unwrap();
        check_frame(&fb, n * 2, "comparison slot");
    }
    assert_eq!(a.memory(MEMORY_SYSTEM_RAM).unwrap()[0], 4);
    assert_eq!(b.memory(MEMORY_SYSTEM_RAM).unwrap()[0], 8);
    drop(b);
    // The baseline keeps working with the other context gone.
    check_frame(&a.run_frame(true).unwrap(), 5, "baseline after the comparison unloaded");
    drop(a);
    let _ = std::fs::remove_dir_all(&tmp);
}
