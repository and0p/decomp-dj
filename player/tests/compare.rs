//! Comparison cores end to end, with the tiny libretro core in tests/testcore (built here
//! with the system C compiler; the test is skipped when there is none).

use decomp_dj::collection::Collection;
use decomp_dj::compare::{Comparison, SyncOutcome};
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::player::Session;
use std::path::{Path, PathBuf};
use std::process::Command;

fn build(out: &Path, name: &str, defs: &[&str]) -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = out.join(name);
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).args(defs).status().ok()?.success();
    ok.then_some(lib)
}

const SEGMENT: &str = "dj-replay 1\nid: seg\nparent: none\nframes: 20\n---\n2 P1 JOYPAD B 1\n6 P1 JOYPAD B 0\n8 P1 JOYPAD Y 1\n12 [WRITE] RAM 0x20 ab cd\n";

fn project(dir: &Path, core: &Path, nodj: &Path) -> Collection {
    std::fs::create_dir_all(dir.join("segments")).unwrap();
    std::fs::create_dir_all(dir.join(".local")).unwrap();
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    std::fs::write(dir.join("segments/seg.segment"), SEGMENT).unwrap();
    let c = core.display();
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"1\"\npath = \"{c}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n\
             [[compare]]\nname = \"state\"\npath = \"{c}\"\nsync = \"state\"\n\
             [[compare]]\nname = \"dj\"\npath = \"{c}\"\n\
             [[compare]]\nname = \"dj-fail\"\npath = \"{c}\"\n[compare.options]\ntestcore_sync = \"fail\"\n\
             [[compare]]\nname = \"nodj\"\npath = \"{}\"\n",
            nodj.display()
        ),
    )
    .unwrap();
    Collection::open(dir).unwrap()
}

fn ram(core: &mut decomp_dj::libretro::Core) -> Vec<u8> {
    core.memory(MEMORY_SYSTEM_RAM).unwrap().to_vec()
}

#[test]
fn comparison_cores() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    let col = project(&dir, &core, &nodj);
    let rom = dir.join("rom.bin");
    let mut s = Session::open_with(&col, &rom, &core, false).unwrap();
    s.enter_replay(&col, "seg").unwrap();

    // The same library as the baseline: loaded from a private copy, with its own state.
    let mut c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    for f in 0..5 {
        s.step(&col, "seg", f, false).unwrap();
    }
    assert_ne!(ram(&mut s.core), ram(&mut c.core));
    assert_eq!(c.sync(&mut s.core, &col, "seg", 5, 5, false).unwrap(), SyncOutcome::Synced);
    assert_eq!(ram(&mut s.core), ram(&mut c.core));
    for f in 5..20 {
        s.step(&col, "seg", f, false).unwrap();
        let st = c.step(&s.core, &col, "seg", f, f == 19).unwrap();
        assert!(!st.needs_sync);
        assert_eq!(ram(&mut s.core), ram(&mut c.core), "frame {f}");
    }
    assert_eq!(&ram(&mut c.core)[0x20..0x22], &[0xab, 0xcd]);
    drop(c);

    // dj_sync: told it's a boot at frame 0, then follows inputs and writes (dj_write).
    let mut s2 = {
        drop(s);
        Session::open_with(&col, &rom, &core, false).unwrap()
    };
    s2.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    assert_eq!(c.sync(&mut s2.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
    assert_eq!(ram(&mut c.core)[63], 1);
    for f in 0..20 {
        s2.step(&col, "seg", f, false).unwrap();
        c.step(&s2.core, &col, "seg", f, false).unwrap();
        assert_eq!(ram(&mut s2.core)[..63], ram(&mut c.core)[..63], "frame {f}");
    }
    drop(c);

    // A sync the core declines, with its reason.
    let mut c = Comparison::open(&col, "dj-fail", &core, Some(&rom)).unwrap();
    match c.sync(&mut s2.core, &col, "seg", 20, 20, false).unwrap() {
        SyncOutcome::Failed(r) => assert!(r.contains("told to fail at frame 20"), "{r}"),
        o => panic!("{o:?}"),
    }
    drop(c);

    // sync = "dj_sync" (the default) needs the function.
    let e = Comparison::open(&col, "nodj", &core, Some(&rom)).err().expect("loading without dj_sync must fail");
    assert!(format!("{e:#}").contains("dj_sync"), "{e:#}");
    // ...and the failed load left the slot free.
    let c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    drop(c);
    drop(s2);
    let _ = std::fs::remove_dir_all(&tmp);
}
