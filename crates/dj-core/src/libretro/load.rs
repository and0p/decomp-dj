//! Finding a core's library and loading it into a slot.

use super::*;

/// The core library a configured `path` names (docs/config.md#core-paths). A file that
/// exists as written is used as is, and so is a path ending in `.so`, `.dylib` or `.dll`
/// (missing or not, so errors name it). Otherwise this platform's library suffix is added
/// to the whole name: `cores/mesen_libretro` is `cores/mesen_libretro.dylib` on macOS, `.so`
/// on Linux and other Unixes, `.dll` on Windows, and `cores/foo.v2` is `cores/foo.v2.dylib`.
pub fn library_path(path: &Path) -> PathBuf {
    library_path_for(path, std::env::consts::DLL_EXTENSION)
}

fn library_path_for(path: &Path, suffix: &str) -> PathBuf {
    let is_lib = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["so", "dylib", "dll"].iter().any(|l| e.eq_ignore_ascii_case(l)));
    if is_lib || path.is_file() || path.file_name().is_none() {
        return path.to_path_buf();
    }
    let mut s = path.as_os_str().to_os_string();
    s.push(".");
    s.push(suffix);
    PathBuf::from(s)
}

impl Core {
    /// Loads the project's core, into the baseline slot.
    pub fn load(core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        Self::load_in(BASELINE, core_path, work_dir, options)
    }

    /// Loads a core into `slot`. Two slots must not hold the same library file: a library
    /// is only loaded once per process, so the second would share the first one's globals.
    pub fn load_in(slot: usize, core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        if slot >= SLOTS {
            bail!("no core slot {slot}");
        }
        if CORE_LOADED[slot].swap(true, Ordering::SeqCst) {
            bail!("a libretro core is already loaded in slot {slot}");
        }
        let r = Self::load_unchecked(slot, core_path, work_dir, options);
        if r.is_err() {
            HOST.with(|h| h.borrow_mut()[slot] = None);
            CORE_LOADED[slot].store(false, Ordering::SeqCst);
            CORE_RUN[slot].store(0, Ordering::SeqCst);
        }
        r
    }

    fn load_unchecked(slot: usize, core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        let system_dir = work_dir.join("system");
        let save_dir = work_dir.join("saves");
        std::fs::create_dir_all(&system_dir)?;
        std::fs::create_dir_all(&save_dir)?;
        HOST.with(|h| {
            h.borrow_mut()[slot] = Some(Host {
                system_dir: CString::new(system_dir.to_string_lossy().as_bytes()).unwrap(),
                save_dir: CString::new(save_dir.to_string_lossy().as_bytes()).unwrap(),
                pixel_format: PixelFormat::Rgb1555,
                option_defs: BTreeMap::new(),
                option_overrides: options.clone(),
                option_cstrings: HashMap::new(),
                options_dirty: false,
                inputs: HashMap::new(),
                capture_next_frame: false,
                last_frame: None,
                proc_iface: None,
                bus: vec![],
                av_info: SystemAvInfo::default(),
                hw: None,
                hw_refused: None,
                memory: None,
                video_hint: false,
                input_names: vec![],
            })
        });

        unsafe {
            let lib = libloading::Library::new(core_path).with_context(|| format!("loading core {}", core_path.display()))?;
            // One library instance must never serve two slots: its callbacks and globals
            // would be shared. macOS hands back an already-loaded library when the path asked
            // for is that library's install name, which a copy of a file built with its
            // absolute path as install name (`cc -o /abs/lib.dylib`) has. Checked before
            // any of its functions is called, so the other core isn't touched.
            {
                let run = lib.get::<unsafe extern "C" fn()>(b"retro_run\0").map(|f| *f as usize).unwrap_or(0);
                if let Some(other) = (0..SLOTS).find(|o| *o != slot && run != 0 && CORE_RUN[*o].load(Ordering::SeqCst) == run) {
                    bail!(
                        "{} is the same loaded library as the core in slot {other}, so the two would share one instance. On macOS this happens when the library's install name (otool -D) is the path being opened and a copy of it is already loaded; rebuild it with a relative install name (-install_name @rpath/NAME), or load the original first",
                        core_path.display()
                    );
                }
                CORE_RUN[slot].store(run, Ordering::SeqCst);
            }
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).with_context(|| format!("core is missing {}", $name))?
                };
            }
            let set_environment: unsafe extern "C" fn(EnvFn) = sym!("retro_set_environment");
            let set_video: unsafe extern "C" fn(VideoFn) = sym!("retro_set_video_refresh");
            let set_audio: unsafe extern "C" fn(AudioFn) = sym!("retro_set_audio_sample");
            let set_audio_batch: unsafe extern "C" fn(AudioBatchFn) = sym!("retro_set_audio_sample_batch");
            let set_poll: unsafe extern "C" fn(PollFn) = sym!("retro_set_input_poll");
            let set_state: unsafe extern "C" fn(StateFn) = sym!("retro_set_input_state");
            let init: unsafe extern "C" fn() = sym!("retro_init");
            let get_system_info: unsafe extern "C" fn(*mut SystemInfo) = sym!("retro_get_system_info");
            let api = Api {
                deinit: sym!("retro_deinit"),
                run: sym!("retro_run"),
                reset: sym!("retro_reset"),
                serialize_size: sym!("retro_serialize_size"),
                serialize: sym!("retro_serialize"),
                unserialize: sym!("retro_unserialize"),
                load_game: sym!("retro_load_game"),
                unload_game: sym!("retro_unload_game"),
                get_memory_data: sym!("retro_get_memory_data"),
                get_memory_size: sym!("retro_get_memory_size"),
                set_controller_port_device: sym!("retro_set_controller_port_device"),
                get_system_av_info: sym!("retro_get_system_av_info"),
            };

            let mut info: SystemInfo = std::mem::zeroed();
            get_system_info(&mut info);
            let cstr = |p: *const c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
            let library_name = cstr(info.library_name);
            let library_version = cstr(info.library_version);

            // Same order as RetroArch: some cores (Mesen) only build their state in retro_init.
            // Each slot has its own copies of the callbacks, which reach its own Host.
            let (env, video, state): (EnvFn, VideoFn, StateFn) = match slot {
                BASELINE => (env_cb::<0>, video_cb::<0>, input_state_cb::<0>),
                _ => (env_cb::<1>, video_cb::<1>, input_state_cb::<1>),
            };
            set_environment(env);
            init();
            set_video(video);
            set_audio(audio_cb);
            set_audio_batch(audio_batch_cb);
            set_poll(input_poll_cb);
            set_state(state);

            let unknown: Vec<String> = with_host(slot, |h| {
                h.option_overrides.keys().filter(|k| !h.option_defs.contains_key(*k)).cloned().collect()
            })
            .unwrap_or_default();
            if !unknown.is_empty() {
                crate::console::warn("dj", &format!("core does not declare these options: {}", unknown.join(", ")));
            }

            let connector = with_host(slot, |h| h.proc_iface).flatten().and_then(|gpa| {
                let get = |name: &str| -> ProcAddress {
                    let c = CString::new(name).unwrap();
                    gpa(c.as_ptr())
                };
                let version: unsafe extern "C" fn() -> u32 = std::mem::transmute(get("dj_api_version")?);
                if version() != DJ_API_VERSION {
                    crate::console::warn("dj", &format!("this core's coverage connector speaks API version {}, but dj needs version {DJ_API_VERSION}: rebuild the core (connectors/README.md). Playing without coverage.", version()));
                    return None;
                }
                // Optional: core options the connector needs and forces while dj drives the
                // core (`key=value` lines). Taking them as dj's own values keeps the recorded
                // options true to what runs.
                if let Some(f) = get("dj_forced_options") {
                    let forced: unsafe extern "C" fn() -> *const c_char = std::mem::transmute(f);
                    let text = forced();
                    if !text.is_null() {
                        let text = CStr::from_ptr(text).to_string_lossy().into_owned();
                        with_host(slot, |h| {
                            for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
                                if h.option_overrides.get(k).is_some_and(|old| old != v) {
                                    crate::console::note("dj", &format!("the coverage connector needs {k} = {v}; using that instead of {}", h.option_overrides[k]));
                                }
                                h.option_overrides.insert(k.to_string(), v.to_string());
                            }
                        });
                    }
                }
                Some(Connector {
                    region_count: std::mem::transmute(get("dj_region_count")?),
                    get_region: std::mem::transmute(get("dj_get_region")?),
                    clear: std::mem::transmute(get("dj_clear")?),
                    set_breakpoints: std::mem::transmute(get("dj_set_breakpoints")?),
                    registers: get("dj_read_registers").map(|f| std::mem::transmute(f)),
                    trace: (|| {
                        Some(TraceFns {
                            start: std::mem::transmute(get("dj_trace_start")?),
                            stop: std::mem::transmute(get("dj_trace_stop")?),
                            read: std::mem::transmute(get("dj_trace_read")?),
                        })
                    })(),
                })
            });

            with_host(slot, |h| h.memory = Some((api.get_memory_data, api.get_memory_size)));
            Ok(Core { _lib: lib, slot, api, library_name, library_version, connector, game_loaded: false, state_now: None, port_devices: BTreeMap::new() })
        }
    }
}

#[cfg(test)]
mod library_path_tests {
    use super::library_path_for;
    use std::path::{Path, PathBuf};

    #[test]
    fn adds_the_suffix_to_a_bare_name() {
        assert_eq!(library_path_for(Path::new("cores/mesen_libretro"), "dylib"), PathBuf::from("cores/mesen_libretro.dylib"));
        assert_eq!(library_path_for(Path::new("cores/mesen_libretro"), "so"), PathBuf::from("cores/mesen_libretro.so"));
        // Another dot isn't a library extension: the suffix goes on the whole name.
        assert_eq!(library_path_for(Path::new("cores/foo.v2"), "dll"), PathBuf::from("cores/foo.v2.dll"));
    }

    #[test]
    fn keeps_a_library_extension() {
        for p in ["cores/foo.so", "cores/foo.dylib", "C:/cores/foo.DLL", "foo_libretro.so"] {
            assert_eq!(library_path_for(Path::new(p), "dylib"), PathBuf::from(p));
        }
    }

    #[test]
    fn keeps_a_file_that_exists() {
        let dir = std::env::temp_dir().join(format!("dj_library_path_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let odd = dir.join("core_without_suffix");
        std::fs::write(&odd, b"").unwrap();
        assert_eq!(library_path_for(&odd, "dylib"), odd);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
