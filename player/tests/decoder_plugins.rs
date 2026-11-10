//! Decoder plugins (docs/decoders.md): the reference plugin (decoders/reference, the built-in
//! decoders exported through dj-decoder's C interface) must decode exactly as the built-in
//! decoders do; a plugin written in C (tests/testdecoder/toy.c) checks the header's layout,
//! the player's checks on answers, `[[decoders]]` loading and decoders exported by a core.

use std::path::{Path, PathBuf};
use std::process::Command;

use decomp_dj::analysis::{self, Function};
use decomp_dj::decode::{self, Decoder, PluginDecoder};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn dll(dir: &Path, stem: &str) -> PathBuf {
    let (pre, ext) = (std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    dir.join(format!("{pre}{stem}.{ext}"))
}

/// Builds decoders/reference with cargo; the library's path.
fn reference_plugin() -> PathBuf {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reference-decoders");
    let ok = Command::new(env!("CARGO"))
        .args(["build", "--release", "--quiet", "--manifest-path"])
        .arg(repo().join("decoders/reference/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("running cargo")
        .success();
    assert!(ok, "building decoders/reference");
    dll(&target.join("release"), "dj_reference_decoders")
}

/// Builds tests/testdecoder/toy.c with `cc` (and `defines`); None where there's no C compiler.
fn toy_plugin_with(name: &str, defines: &[&str]) -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("toy-decoder");
    std::fs::create_dir_all(&dir).ok()?;
    let lib = dll(&dir, name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdecoder/toy.c");
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1"]).args(defines).arg("-o").arg(&lib).arg(&src).status().ok()?.success();
    ok.then_some(lib)
}

fn toy_plugin(name: &str) -> Option<PathBuf> {
    toy_plugin_with(name, &[])
}

/// Plugins run in workers: point them at the `dj` program (the test binary isn't one).
fn use_dj_workers() {
    std::env::set_var("DJ_WORKER_EXE", env!("CARGO_BIN_EXE_dj"));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn the_reference_plugin_decodes_as_the_built_in_decoders_do() {
    let plugins = PluginDecoder::load(&reference_plugin(), "decoder plugin").unwrap();
    let cpus: Vec<&str> = plugins.iter().map(|p| p.cpu()).collect();
    assert_eq!(cpus, ["6502", "mips3", "armv4t"]);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for p in &plugins {
        let b = decode::decoder_for(p.cpu()).unwrap();
        assert!(p.id().contains("sha256"));
        assert_eq!((p.max_len(), p.fixed_len()), (b.max_len(), b.fixed_len()), "{}", p.cpu());
        for mode in 0..2 {
            assert_eq!(p.alignment(mode), b.alignment(mode), "{}", p.cpu());
        }
        for flags in 0..=255u8 {
            assert_eq!(p.mode_from_flags(flags), b.mode_from_flags(flags), "{} flags {flags:#x}", p.cpu());
        }
        for _ in 0..200_000 {
            let bytes = rng.next().to_le_bytes();
            let avail = (rng.next() % 5) as usize; // short slices too, as at the end of code
            let pc = rng.next() & 0xFFFF_FFFF;
            let mode = (rng.next() & 1) as u32;
            let s = &bytes[..avail.min(p.max_len())];
            assert_eq!(p.decode(s, pc, mode), b.decode(s, pc, mode), "{} {s:02x?} at {pc:#x} mode {mode}", p.cpu());
        }
        assert_eq!(p.invalid_answers(), 0);
    }

    // Whole functions of the example projects, where they've been built.
    let ex = repo().join("examples");
    let sweeps: Vec<(&str, PathBuf, Box<dyn Fn() -> Option<Vec<Function>>>, u64)> = vec![
        ("6502", ex.join("thwaite/build/thwaite.nes"), Box::new(|| analysis::load_ca65_dbg(&ex.join("thwaite/build/thwaite.dbg"), 16).ok()), 16),
        ("6502", ex.join("tetris/build/tetris_usa.nes"), Box::new(|| analysis::load_csv(&ex.join("tetris/build/ref/tetris_symbols.csv")).ok()), 16),
        ("armv4t", ex.join("gba-test/build/gba-test.gba"), Box::new(|| analysis::load_elf(&ex.join("gba-test/symbols/gba-test.elf"), 0x0800_0000, "ROM").ok()), 0),
    ];
    for (cpu, path, funcs, header) in sweeps {
        let (Ok(rom), Some(funcs)) = (std::fs::read(&path), funcs()) else {
            eprintln!("skipping {}: not built", path.display());
            continue;
        };
        let code = &rom[header as usize..];
        let p = plugins.iter().find(|p| p.cpu() == cpu).unwrap();
        let by_plugin = decode::analyze(p, &funcs, code, &[], |f| Some(f.offset));
        let built_in = decode::analyze(decode::decoder_for(cpu).unwrap(), &funcs, code, &[], |f| Some(f.offset));
        assert_eq!(format!("{by_plugin:?}"), format!("{built_in:?}"), "{}", path.display());
        let n: usize = built_in.iter().map(|f| f.instructions.len()).sum();
        assert!(n > 0);
        eprintln!("{}: {n} instructions decoded the same", path.display());
    }
}

#[test]
fn a_c_plugin_is_read_through_the_header() {
    let Some(lib) = toy_plugin("toy_decoder") else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let p = PluginDecoder::load(&lib, "decoder plugin").unwrap().pop().unwrap();
    assert_eq!((p.cpu(), p.max_len(), p.fixed_len(), p.alignment(0), p.mode_from_flags(0x04)), ("toy8", 3, None, 1, None));
    // 1000: NOP; BEQ 1006; CALL 2000; 1006: LDL 1009; RET; 1009: 2 bytes of data; 100B: HALT
    let code = [0x00, 0x10, 0x03, 0x20, 0x00, 0x20, 0x60, 0x01, 0x40, 0xAA, 0xBB, 0x50];
    let f = Function { name: "f".into(), region: "ROM".into(), offset: 0, size: code.len() as u64, cpu_addr: Some(0x1000), file_offset: None, mode: 0 };
    let a = &decode::analyze(&p, std::slice::from_ref(&f), &code, &[], |f| Some(f.offset))[0];
    let at: Vec<u64> = a.instructions.iter().map(|i| i.offset).collect();
    assert_eq!(at, [0, 1, 3, 6, 8, 11], "the literal at 9 and 10 isn't code");
    let blocks: Vec<u64> = a.blocks.iter().map(|b| b.offset).collect();
    assert_eq!(blocks, [0, 3, 6, 11]);
    assert_eq!(a.successors, [vec![3, 6], vec![6], vec![], vec![]]);
    let calls: Vec<(u64, u64, &str)> = a.calls.iter().map(|c| (c.at, c.target, c.kind)).collect();
    assert_eq!(calls, [(3, 0x2000, "CALL")]);
    // Bytes that aren't an instruction, and a broken answer, which is refused and counted.
    assert_eq!(p.decode(&[0xFF], 0, 0).len, 0);
    assert_eq!(p.decode(&[0xEE], 0, 0).len, 0);
    assert_eq!(p.invalid_answers(), 1);
}

/// A project folder with the GBA example's config, and `extra` added to config.toml.
fn project(name: &str, extra: &str, local: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".local")).unwrap();
    let config = std::fs::read_to_string(repo().join("examples/gba-test/config.toml")).unwrap();
    // Without its [[symbols]] (the last table): these projects have no ELF.
    let config = &config[..config.find("[[symbols]]").unwrap_or(config.len())];
    std::fs::write(dir.join("config.toml"), format!("{config}\n{extra}")).unwrap();
    std::fs::write(dir.join(".local/local.toml"), local).unwrap();
    dir
}

#[test]
fn projects_load_decoders_from_config_and_from_their_core() {
    use_dj_workers();
    let Some(lib) = toy_plugin("toy_for_projects") else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let path = lib.to_str().unwrap().replace('\\', "/");
    // [[decoders]], with the platform's suffix left out, and pinned.
    let sha = decomp_dj::collection::sha256_file(&lib).unwrap();
    let stem = path.trim_end_matches(&format!(".{}", std::env::consts::DLL_EXTENSION));
    let dir = project("plugin-config", &format!("[[decoders]]\npath = \"{stem}\"\nsha256 = \"{sha}\"\n"), "");
    let col = decomp_dj::collection::Collection::open(&dir).unwrap();
    assert_eq!(col.decoders.get("toy8").map(|d| d.cpu()), Some("toy8"));
    assert!(col.decoders.get("toy8").unwrap().id().contains(&sha[..12]));
    assert_eq!(col.decoders.get("6502").map(|d| d.id()), Some("built-in 6502".to_string()));
    // A wrong pin, or a missing file, stops the project from opening, naming the entry.
    let dir = project("plugin-bad-pin", &format!("[[decoders]]\npath = \"{path}\"\nsha256 = \"00\"\n"), "");
    let err = format!("{:#}", decomp_dj::collection::Collection::open(&dir).err().unwrap());
    assert!(err.contains("pinned to 00"), "{err}");
    let dir = project("plugin-missing", "[[decoders]]\npath = \"no_such_decoder\"\n", "");
    let err = format!("{:#}", decomp_dj::collection::Collection::open(&dir).err().unwrap());
    assert!(err.contains("doesn't exist"), "{err}");
    // A core that exports dj_decoders: its decoders are used for CPUs nothing else decodes.
    let dir = project("plugin-core", "", &format!("core = \"{path}\"\n"));
    let col = decomp_dj::collection::Collection::open(&dir).unwrap();
    assert_eq!(col.decoders.get("toy8").map(|d| d.cpu()), Some("toy8"));
    assert!(col.decoders.get("z80").is_none());
}

/// The toy plugin as a project's `[[decoders]]`, and a function `name` holding `code`.
fn toy_project(name: &str, lib: &Path) -> decomp_dj::collection::Collection {
    use_dj_workers();
    let dir = project(name, &format!("[[decoders]]\npath = \"{}\"\n", lib.to_str().unwrap().replace('\\', "/")), "");
    decomp_dj::collection::Collection::open(&dir).unwrap()
}

fn toy_function(name: &str, size: u64) -> Vec<(Function, Option<u64>)> {
    vec![
        (Function { name: "fine".into(), region: "ROM".into(), offset: 0, size: 4, cpu_addr: Some(0x1000), file_offset: None, mode: 0 }, Some(0)),
        (Function { name: name.into(), region: "ROM".into(), offset: 4, size, cpu_addr: Some(0x1004), file_offset: None, mode: 0 }, Some(4)),
    ]
}

#[test]
fn a_plugin_that_crashes_costs_the_decoding_not_dj() {
    let Some(lib) = toy_plugin("toy_crash") else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let col = toy_project("plugin-crash", &lib);
    let dec = col.decoders.get("toy8").unwrap();
    // Through the worker, the toy decodes as it does in process.
    let (ok, invalid) = dec.analyze(&toy_function("also_fine", 4), &[0, 0, 0, 0x40, 0x00, 0xEE, 0, 0x40], &[]).unwrap();
    assert_eq!((ok.len(), invalid), (2, 1));
    // 0xCC crashes it: dj is still here, and says where.
    let err = dec.analyze(&toy_function("boom", 4), &[0, 0, 0, 0x40, 0x00, 0xCC, 0, 0x40], &[]).unwrap_err();
    assert!(err.contains("crashed") && err.contains("decoding `boom`"), "{err}");
}

#[test]
fn a_plugin_that_hangs_is_stopped() {
    let Some(lib) = toy_plugin("toy_hang") else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let col = toy_project("plugin-hang", &lib);
    let dec = col.decoders.get("toy8").unwrap();
    std::env::set_var("DJ_WORKER_TIMEOUT_SCALE", "0.1"); // 1 s instead of 10
    let started = std::time::Instant::now();
    let err = dec.analyze(&toy_function("spin", 4), &[0, 0, 0, 0x40, 0xDD, 0, 0, 0x40], &[]).unwrap_err();
    assert!(err.contains("didn't finish") && err.contains("decoding `spin`"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(8), "{:?}", started.elapsed());
}

#[test]
fn a_plugin_that_crashes_while_its_tables_are_read_names_the_entry() {
    use_dj_workers();
    let Some(lib) = toy_plugin_with("toy_crash_in_tables", &["-DTOY_CRASH_IN_TABLES"]) else {
        eprintln!("skipping: no C compiler");
        return;
    };
    let dir = project("plugin-crash-tables", &format!("[[decoders]]\npath = \"{}\"\n", lib.to_str().unwrap().replace('\\', "/")), "");
    let err = format!("{:#}", decomp_dj::collection::Collection::open(&dir).err().unwrap());
    assert!(err.contains("[[decoders]]") && err.contains("crashed") && err.contains("while its decoders were read"), "{err}");
}
