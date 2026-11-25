//! Making a new project (`dj init`): `propose` works out what the project would be from a
//! ROM and a core and writes nothing into the project's folder; `InitPlan::write` makes it.
//! Front ends show the plan in between.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::analysis::RegionFacts;
use crate::collection::{sha256_file, Collection, Config, CoreConfig, HashConfig, LocalConfig, PortConfig, RomConfig, SymbolsConfig, DJ_VERSION, PROJECT_FORMAT};
use crate::libretro;

/// What the user gave. Everything but the ROM and the core has a default.
#[derive(Clone, Debug, Default)]
pub struct InitRequest {
    /// The folder to make the project in (default: one named after the ROM file, without
    /// its extension, in the current folder).
    pub dir: Option<PathBuf>,
    /// The project's name (default: the folder's name when `dir` is given and has one,
    /// else the ROM file's name without its extension).
    pub name: Option<String>,
    pub rom: PathBuf,
    /// The core library, with or without its suffix (`libretro::library_path`).
    pub core: PathBuf,
    /// The id of the core's build to record (default: what the core reports).
    pub core_build: Option<String>,
    /// Core options to set before the core's own values are recorded.
    pub options: BTreeMap<String, String>,
    /// A symbol map to copy into `symbols/`.
    pub symbols: Option<PathBuf>,
}

/// One of the connector's regions, as `dj init` reports it.
#[derive(Clone, Debug)]
pub struct RegionNote {
    pub facts: RegionFacts,
    /// Whether dj decodes the region's instruction set itself.
    pub decoder: bool,
    /// Whether coverage rolls up into this region (`analysis::code_region`).
    pub rolls_up: bool,
    /// The region says it comes from the ROM file, and its bytes aren't the file's there.
    pub differs_from_rom: bool,
}

/// A project that `write` would make.
#[derive(Clone, Debug)]
pub struct InitPlan {
    pub dir: PathBuf,
    pub rom: PathBuf,
    /// The core library's file.
    pub core: PathBuf,
    /// The symbol map to copy, which `config.symbols` already names.
    pub symbols: Option<PathBuf>,
    pub config: Config,
    /// Whether the core has a coverage connector. Without one, segments play but record
    /// no coverage.
    pub connector: bool,
    /// The connector's regions (none without a connector).
    pub regions: Vec<RegionNote>,
}

fn has_project(dir: &Path) -> bool {
    dir.join("config.toml").exists() || dir.join("collection.toml").exists()
}

fn stem(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
}

/// Loads the core with the ROM to learn what the project pins (the core's build and every
/// option, the regions to hash, the connector's regions) and returns the project it would
/// make. The core runs in a folder of its own under the system's temporary folder, so
/// nothing is written into the project's.
pub fn propose(req: &InitRequest) -> Result<InitPlan> {
    if !req.rom.is_file() {
        bail!("the ROM {} doesn't exist", req.rom.display());
    }
    let core_path = libretro::library_path(&req.core);
    if !core_path.is_file() {
        bail!("the core library {} doesn't exist", core_path.display());
    }
    let rom_stem = stem(&req.rom);
    let dir = match &req.dir {
        Some(d) => d.clone(),
        None => PathBuf::from(rom_stem.clone().ok_or_else(|| anyhow!("can't name a folder after {}: say which folder to make the project in", req.rom.display()))?),
    };
    if has_project(&dir) {
        bail!("{} already has a project", dir.display());
    }
    let name = match &req.name {
        Some(n) => n.clone(),
        // `.` and `..` have no name of their own.
        None => req.dir.as_deref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).or(rom_stem).ok_or_else(|| anyhow!("can't name the project after {}: pass a name", req.rom.display()))?,
    };
    let mut syms = vec![];
    if let Some(s) = &req.symbols {
        if !s.is_file() {
            bail!("the symbol map {} doesn't exist", s.display());
        }
        let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
        let format = crate::analysis::guess_format(s);
        syms.push(SymbolsConfig { file: format!("symbols/{}", fname.to_string_lossy()), format: format.into(), rom_header: None, region: None, cpu_base: None });
    }

    let work = std::env::temp_dir().join(format!("dj-init-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
    let probed = probe(&core_path, &req.rom, &work, &req.options);
    let _ = std::fs::remove_dir_all(&work);
    let p = probed?;

    let config = Config {
        decoders: vec![],
        dj: Some(DJ_VERSION.to_string()),
        format: PROJECT_FORMAT,
        name,
        rom: RomConfig { sha256: sha256_file(&req.rom)?, file: req.rom.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), shareable: None },
        core: CoreConfig { name: p.library_name.to_lowercase(), build: req.core_build.clone().unwrap_or(p.build_id), sha256: String::new(), options: p.options },
        ports: vec![PortConfig { port: 1, device: "JOYPAD".into(), labels: Default::default(), hide: vec![] }],
        hash: HashConfig { algorithm: "blake3".into(), regions: p.hash_regions, exclude: vec![] },
        tick: None,
        symbols: syms,
        cache: None,
        compare: vec![],
    };
    Ok(InitPlan { dir, rom: req.rom.clone(), core: core_path, symbols: req.symbols.clone(), config, connector: p.connector, regions: p.regions })
}

struct Probed {
    library_name: String,
    build_id: String,
    options: BTreeMap<String, String>,
    hash_regions: Vec<String>,
    connector: bool,
    regions: Vec<RegionNote>,
}

fn probe(core_path: &Path, rom: &Path, work: &Path, options: &BTreeMap<String, String>) -> Result<Probed> {
    let mut core = libretro::Core::load(core_path, work, options)?;
    core.load_game(rom)?;
    // Some cores set up their memory on the first frame (Mupen64Plus-Next): run one so
    // the RAM regions to hash exist when they're probed.
    core.run_frame(false);
    let mut hash_regions = vec![];
    for r in ["RAM", "SRAM"] {
        if core.memory_view(libretro::region_memory_id(r).unwrap()).is_some() {
            hash_regions.push(r.to_string());
        }
    }
    let connector = core.has_connector();
    let mut regions = vec![];
    if connector {
        let file = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
        let facts = core.region_facts();
        let code = crate::analysis::code_region(&facts);
        for (i, f) in facts.into_iter().enumerate() {
            let differs_from_rom = match (f.file_offset, core.region_data(i)) {
                (Some(off), Some(data)) => usize::try_from(off).ok().and_then(|o| file.get(o..o.checked_add(data.len())?)) != Some(data),
                _ => false,
            };
            regions.push(RegionNote { decoder: f.cpu.as_deref().is_some_and(crate::analysis::has_decoder), rolls_up: f.name == code, differs_from_rom, facts: f });
        }
    }
    Ok(Probed { library_name: core.library_name.clone(), build_id: core.build_id(), options: core.effective_options(), hash_regions, connector, regions })
}

impl InitPlan {
    /// Makes the project: its folders, `config.toml`, the symbol map's copy, and
    /// `.local/local.toml` with where the ROM and the core are on this machine. Returns
    /// the path of `config.toml`.
    pub fn write(&self) -> Result<PathBuf> {
        let dir = &self.dir;
        if has_project(dir) {
            bail!("{} already has a project", dir.display());
        }
        std::fs::create_dir_all(dir.join("segments"))?;
        std::fs::create_dir_all(dir.join("blobs"))?;
        if let Some(s) = &self.symbols {
            let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
            std::fs::create_dir_all(dir.join("symbols"))?;
            std::fs::copy(s, dir.join("symbols").join(fname)).with_context(|| format!("copying {}", s.display()))?;
        }
        let path = dir.join("config.toml");
        std::fs::write(&path, toml::to_string_pretty(&self.config)?)?;
        let col = Collection::open(dir)?;
        col.save_local(&LocalConfig { rom: Some(std::fs::canonicalize(&self.rom)?), core: Some(std::fs::canonicalize(&self.core)?), ..Default::default() })?;
        Ok(path)
    }
}
