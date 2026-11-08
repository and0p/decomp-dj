//! Offscreen OpenGL for libretro cores that render on the GPU (`RETRO_ENVIRONMENT_SET_HW_RENDER`).
//!
//! Each core that asks for OpenGL gets a private context with no window (CGL on macOS, EGL
//! elsewhere) and a framebuffer object to draw into. Frames are read back into memory only
//! when they are captured, so everything after `video_cb` sees the same RGB frames as a
//! software core.
//!
//! The context is current only while the core runs (`enter` .. `leave`). On macOS leaving
//! puts back the context that was current before (the GUI's); elsewhere it makes no context
//! current, and eframe, finding its own not current, makes it current again before it paints.

use anyhow::{anyhow, bail, Result};
use std::ffi::c_void;

/// What a core asked for.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// A core profile (`RETRO_HW_CONTEXT_OPENGL_CORE`), or compatibility (`..._OPENGL`, GL 2.x).
    pub core_profile: bool,
    pub major: u32,
    pub minor: u32,
    pub depth: bool,
    pub stencil: bool,
    pub debug: bool,
}

impl Request {
    pub fn describe(&self) -> String {
        if self.core_profile {
            format!("OpenGL {}.{} core profile", self.major, self.minor)
        } else {
            "OpenGL (compatibility profile)".to_string()
        }
    }
}

// GL enums.
const FRAMEBUFFER: u32 = 0x8D40;
const RENDERBUFFER: u32 = 0x8D41;
const RGBA8: u32 = 0x8058;
const DEPTH24_STENCIL8: u32 = 0x88F0;
const DEPTH_COMPONENT24: u32 = 0x81A6;
const COLOR_ATTACHMENT0: u32 = 0x8CE0;
const DEPTH_ATTACHMENT: u32 = 0x8D00;
const STENCIL_ATTACHMENT: u32 = 0x8D20;
const FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const PIXEL_PACK_BUFFER: u32 = 0x88EB;
const PIXEL_PACK_BUFFER_BINDING: u32 = 0x88ED;
const PACK_ROW_LENGTH: u32 = 0x0D02;
const PACK_SKIP_ROWS: u32 = 0x0D03;
const PACK_SKIP_PIXELS: u32 = 0x0D04;
const PACK_ALIGNMENT: u32 = 0x0D05;
const READ_BUFFER: u32 = 0x0C02;
const RGBA: u32 = 0x1908;
const UNSIGNED_BYTE: u32 = 0x1401;
const VERSION: u32 = 0x1F02;

/// A function pointer from a non-null `*const c_void`.
unsafe fn cast<F: Copy>(p: *const c_void) -> F {
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const c_void>());
    std::mem::transmute_copy(&p)
}

/// The few GL functions the frontend itself needs.
struct Fns {
    gen_framebuffers: unsafe extern "system" fn(i32, *mut u32),
    delete_framebuffers: unsafe extern "system" fn(i32, *const u32),
    bind_framebuffer: unsafe extern "system" fn(u32, u32),
    gen_renderbuffers: unsafe extern "system" fn(i32, *mut u32),
    delete_renderbuffers: unsafe extern "system" fn(i32, *const u32),
    bind_renderbuffer: unsafe extern "system" fn(u32, u32),
    renderbuffer_storage: unsafe extern "system" fn(u32, u32, i32, i32),
    framebuffer_renderbuffer: unsafe extern "system" fn(u32, u32, u32, u32),
    check_framebuffer_status: unsafe extern "system" fn(u32) -> u32,
    read_pixels: unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void),
    read_buffer: unsafe extern "system" fn(u32),
    get_integerv: unsafe extern "system" fn(u32, *mut i32),
    pixel_storei: unsafe extern "system" fn(u32, i32),
    bind_buffer: unsafe extern "system" fn(u32, u32),
    get_string: unsafe extern "system" fn(u32) -> *const u8,
}

impl Fns {
    fn load(plat: &Platform) -> Result<Fns> {
        // Core names first, then the EXT/ARB ones a legacy context may only have.
        let get = |names: &[&str]| -> Result<*const c_void> {
            for n in names {
                let p = plat.proc(n);
                if !p.is_null() {
                    return Ok(p);
                }
            }
            Err(anyhow!("the OpenGL driver has no {}", names[0]))
        };
        macro_rules! f {
            ($($n:literal),+) => {
                unsafe { cast(get(&[$($n),+])?) }
            };
        }
        Ok(Fns {
            gen_framebuffers: f!("glGenFramebuffers", "glGenFramebuffersEXT"),
            delete_framebuffers: f!("glDeleteFramebuffers", "glDeleteFramebuffersEXT"),
            bind_framebuffer: f!("glBindFramebuffer", "glBindFramebufferEXT"),
            gen_renderbuffers: f!("glGenRenderbuffers", "glGenRenderbuffersEXT"),
            delete_renderbuffers: f!("glDeleteRenderbuffers", "glDeleteRenderbuffersEXT"),
            bind_renderbuffer: f!("glBindRenderbuffer", "glBindRenderbufferEXT"),
            renderbuffer_storage: f!("glRenderbufferStorage", "glRenderbufferStorageEXT"),
            framebuffer_renderbuffer: f!("glFramebufferRenderbuffer", "glFramebufferRenderbufferEXT"),
            check_framebuffer_status: f!("glCheckFramebufferStatus", "glCheckFramebufferStatusEXT"),
            read_pixels: f!("glReadPixels"),
            read_buffer: f!("glReadBuffer"),
            get_integerv: f!("glGetIntegerv"),
            pixel_storei: f!("glPixelStorei"),
            bind_buffer: f!("glBindBuffer", "glBindBufferARB"),
            get_string: f!("glGetString"),
        })
    }
}

/// A core's GL context and the framebuffer it draws into.
pub struct GlContext {
    plat: Platform,
    f: Fns,
    fbo: u32,
    color: u32,
    depth: u32,
    req: Request,
    /// The framebuffer's size.
    pub width: u32,
    pub height: u32,
    pub version: String,
}

impl GlContext {
    /// Creates the context and a `width` x `height` framebuffer. Leaves the context current.
    pub fn new(req: Request, width: u32, height: u32) -> Result<GlContext> {
        let plat = Platform::new(&req)?;
        plat.make_current()?;
        let f = match Fns::load(&plat) {
            Ok(f) => f,
            Err(e) => {
                plat.release();
                return Err(e);
            }
        };
        let version = unsafe {
            let p = (f.get_string)(VERSION);
            if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p as *const _).to_string_lossy().into_owned() }
        };
        let mut gl = GlContext { plat, f, fbo: 0, color: 0, depth: 0, req, width: 0, height: 0, version };
        if let Err(e) = gl.make_framebuffer(width.max(1), height.max(1)) {
            gl.plat.release();
            return Err(e);
        }
        Ok(gl)
    }

    fn make_framebuffer(&mut self, width: u32, height: u32) -> Result<()> {
        let f = &self.f;
        unsafe {
            if self.fbo == 0 {
                (f.gen_framebuffers)(1, &mut self.fbo);
                (f.gen_renderbuffers)(1, &mut self.color);
                if self.req.depth || self.req.stencil {
                    (f.gen_renderbuffers)(1, &mut self.depth);
                }
            }
            (f.bind_framebuffer)(FRAMEBUFFER, self.fbo);
            (f.bind_renderbuffer)(RENDERBUFFER, self.color);
            (f.renderbuffer_storage)(RENDERBUFFER, RGBA8, width as i32, height as i32);
            (f.framebuffer_renderbuffer)(FRAMEBUFFER, COLOR_ATTACHMENT0, RENDERBUFFER, self.color);
            if self.depth != 0 {
                (f.bind_renderbuffer)(RENDERBUFFER, self.depth);
                let format = if self.req.stencil { DEPTH24_STENCIL8 } else { DEPTH_COMPONENT24 };
                (f.renderbuffer_storage)(RENDERBUFFER, format, width as i32, height as i32);
                (f.framebuffer_renderbuffer)(FRAMEBUFFER, DEPTH_ATTACHMENT, RENDERBUFFER, self.depth);
                if self.req.stencil {
                    (f.framebuffer_renderbuffer)(FRAMEBUFFER, STENCIL_ATTACHMENT, RENDERBUFFER, self.depth);
                }
            }
            (f.bind_renderbuffer)(RENDERBUFFER, 0);
            let status = (f.check_framebuffer_status)(FRAMEBUFFER);
            if status != FRAMEBUFFER_COMPLETE {
                bail!("the core's {width}x{height} framebuffer is incomplete (GL status 0x{status:x})");
            }
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// The framebuffer the core draws into (`get_current_framebuffer`).
    pub fn framebuffer(&self) -> u32 {
        self.fbo
    }

    /// A GL function, for the core's `get_proc_address`.
    pub fn proc(&self, name: &str) -> *const c_void {
        self.plat.proc(name)
    }

    /// Makes the context current, growing the framebuffer to `width` x `height` if it is
    /// smaller.
    pub fn enter(&mut self, width: u32, height: u32) -> Result<()> {
        self.plat.make_current()?;
        if width > self.width || height > self.height {
            let (w, h) = (width.max(self.width), height.max(self.height));
            self.make_framebuffer(w, h)?;
        }
        Ok(())
    }

    /// Makes no context current.
    pub fn leave(&self) {
        self.plat.release();
    }

    /// Reads the bottom-left `width` x `height` of the framebuffer (where the core drew this
    /// frame) as RGB, top row first. `bottom_left_origin`: the core drew with GL's usual
    /// origin, so the image's top is the framebuffer's last row. The core's pack state and
    /// bindings are left as they were.
    pub fn read_rgb(&self, width: u32, height: u32, bottom_left_origin: bool) -> Vec<u8> {
        let (w, h) = (width.min(self.width) as usize, height.min(self.height) as usize);
        let mut rgba = vec![0u8; w * h * 4];
        let f = &self.f;
        unsafe {
            let get = |e: u32| {
                let mut v = 0i32;
                (f.get_integerv)(e, &mut v);
                v
            };
            let (fb, pbo, read_buf) = (get(FRAMEBUFFER_BINDING), get(PIXEL_PACK_BUFFER_BINDING), get(READ_BUFFER));
            let pack = [PACK_ROW_LENGTH, PACK_SKIP_ROWS, PACK_SKIP_PIXELS, PACK_ALIGNMENT].map(|e| (e, get(e)));
            (f.bind_framebuffer)(FRAMEBUFFER, self.fbo);
            (f.read_buffer)(COLOR_ATTACHMENT0);
            if pbo != 0 {
                (f.bind_buffer)(PIXEL_PACK_BUFFER, 0);
            }
            for (e, _) in pack {
                (f.pixel_storei)(e, if e == PACK_ALIGNMENT { 1 } else { 0 });
            }
            (f.read_pixels)(0, 0, w as i32, h as i32, RGBA, UNSIGNED_BYTE, rgba.as_mut_ptr() as *mut c_void);
            for (e, v) in pack {
                (f.pixel_storei)(e, v);
            }
            if pbo != 0 {
                (f.bind_buffer)(PIXEL_PACK_BUFFER, pbo as u32);
            }
            (f.read_buffer)(read_buf as u32);
            (f.bind_framebuffer)(FRAMEBUFFER, fb as u32);
        }
        // glReadPixels gives the framebuffer's bottom row first.
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            let row = if bottom_left_origin { h - 1 - y } else { y };
            for px in rgba[row * w * 4..(row + 1) * w * 4].chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
            }
        }
        rgb
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        if self.plat.make_current().is_ok() {
            unsafe {
                (self.f.bind_framebuffer)(FRAMEBUFFER, 0);
                (self.f.delete_framebuffers)(1, &self.fbo);
                (self.f.delete_renderbuffers)(1, &self.color);
                if self.depth != 0 {
                    (self.f.delete_renderbuffers)(1, &self.depth);
                }
            }
            self.plat.release();
        }
        self.plat.destroy();
    }
}

/// Why a request can't be met on this machine, if it can't: checked before telling a core
/// yes, so it can fall back to software rendering.
pub fn unsupported(req: &Request) -> Option<String> {
    if cfg!(target_os = "macos") && req.core_profile && (req.major, req.minor) > (4, 1) {
        return Some(format!("{} (macOS has OpenGL up to 4.1)", req.describe()));
    }
    if cfg!(windows) {
        return Some("OpenGL for cores isn't supported on Windows yet".into());
    }
    if let Err(e) = Platform::library() {
        return Some(format!("{e:#}"));
    }
    None
}

// ---- macOS: CGL ----------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    use super::Request;
    use anyhow::{anyhow, bail, Result};
    use std::ffi::{c_void, CString};
    use std::sync::OnceLock;

    type Obj = *mut c_void;
    const PFA_ACCELERATED: i32 = 73;
    const PFA_ALLOW_OFFLINE_RENDERERS: i32 = 96;
    const PFA_OPENGL_PROFILE: i32 = 99;
    const PFA_COLOR_SIZE: i32 = 8;
    const PFA_DEPTH_SIZE: i32 = 12;
    const PFA_STENCIL_SIZE: i32 = 13;
    const PROFILE_LEGACY: i32 = 0x1000;
    const PROFILE_GL4_CORE: i32 = 0x4100;
    const PROFILE_GL3_CORE: i32 = 0x3200;

    struct Cgl {
        lib: libloading::Library,
        choose: unsafe extern "C" fn(*const i32, *mut Obj, *mut i32) -> i32,
        destroy_pix: unsafe extern "C" fn(Obj) -> i32,
        create: unsafe extern "C" fn(Obj, Obj, *mut Obj) -> i32,
        destroy: unsafe extern "C" fn(Obj) -> i32,
        set_current: unsafe extern "C" fn(Obj) -> i32,
        get_current: unsafe extern "C" fn() -> Obj,
    }
    unsafe impl Send for Cgl {}
    unsafe impl Sync for Cgl {}

    fn cgl() -> Result<&'static Cgl> {
        static CGL: OnceLock<std::result::Result<Cgl, String>> = OnceLock::new();
        CGL.get_or_init(|| unsafe {
            let lib = libloading::Library::new("/System/Library/Frameworks/OpenGL.framework/OpenGL").map_err(|e| format!("loading OpenGL.framework: {e}"))?;
            macro_rules! s {
                ($n:literal) => {
                    *lib.get(concat!($n, "\0").as_bytes()).map_err(|e| format!("OpenGL.framework has no {}: {e}", $n))?
                };
            }
            Ok(Cgl {
                choose: s!("CGLChoosePixelFormat"),
                destroy_pix: s!("CGLDestroyPixelFormat"),
                create: s!("CGLCreateContext"),
                destroy: s!("CGLDestroyContext"),
                set_current: s!("CGLSetCurrentContext"),
                get_current: s!("CGLGetCurrentContext"),
                lib,
            })
        })
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
    }

    pub struct Platform {
        ctx: Obj,
        /// The context that was current before ours (a GUI's), put back on release: AppKit
        /// keeps its own idea of the current NSOpenGLContext, so leave CGL as it was.
        prev: std::cell::Cell<Obj>,
    }

    impl Platform {
        pub fn library() -> Result<()> {
            cgl().map(|_| ())
        }

        pub fn new(req: &Request) -> Result<Platform> {
            let c = cgl()?;
            let profiles: &[i32] = if req.core_profile { &[PROFILE_GL4_CORE, PROFILE_GL3_CORE] } else { &[PROFILE_LEGACY] };
            for &profile in profiles {
                let mut attrs = vec![PFA_OPENGL_PROFILE, profile, PFA_ACCELERATED, PFA_ALLOW_OFFLINE_RENDERERS, PFA_COLOR_SIZE, 24];
                if req.depth {
                    attrs.extend([PFA_DEPTH_SIZE, 24]);
                }
                if req.stencil {
                    attrs.extend([PFA_STENCIL_SIZE, 8]);
                }
                attrs.push(0);
                unsafe {
                    let mut pix: Obj = std::ptr::null_mut();
                    let mut n = 0;
                    if (c.choose)(attrs.as_ptr(), &mut pix, &mut n) != 0 || pix.is_null() {
                        continue;
                    }
                    let mut ctx: Obj = std::ptr::null_mut();
                    let err = (c.create)(pix, std::ptr::null_mut(), &mut ctx);
                    (c.destroy_pix)(pix);
                    if err == 0 && !ctx.is_null() {
                        return Ok(Platform { ctx, prev: std::cell::Cell::new(std::ptr::null_mut()) });
                    }
                }
            }
            bail!("macOS couldn't make a {} context", req.describe())
        }

        pub fn make_current(&self) -> Result<()> {
            let c = cgl()?;
            let cur = unsafe { (c.get_current)() };
            if cur != self.ctx {
                self.prev.set(cur);
            }
            let err = unsafe { (c.set_current)(self.ctx) };
            if err != 0 {
                bail!("CGLSetCurrentContext failed (error {err})");
            }
            Ok(())
        }

        pub fn release(&self) {
            if let Ok(c) = cgl() {
                unsafe { (c.set_current)(self.prev.replace(std::ptr::null_mut())) };
            }
        }

        pub fn proc(&self, name: &str) -> *const c_void {
            let (Ok(c), Ok(n)) = (cgl(), CString::new(name)) else { return std::ptr::null() };
            unsafe { c.lib.get::<*const c_void>(n.as_bytes_with_nul()).map(|s| *s).unwrap_or(std::ptr::null()) }
        }

        pub fn destroy(&mut self) {
            if let Ok(c) = cgl() {
                if !self.ctx.is_null() {
                    unsafe { (c.destroy)(self.ctx) };
                    self.ctx = std::ptr::null_mut();
                }
            }
        }
    }
}

// ---- Linux and other Unixes: EGL -----------------------------------------------

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Request;
    use anyhow::{anyhow, bail, Result};
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::OnceLock;

    type Ptr = *mut c_void;
    const NONE: i32 = 0x3038;
    const EXTENSIONS: i32 = 0x3055;
    const OPENGL_API: u32 = 0x30A2;
    const SURFACE_TYPE: i32 = 0x3033;
    const RENDERABLE_TYPE: i32 = 0x3040;
    const OPENGL_BIT: i32 = 0x0008;
    const RED_SIZE: i32 = 0x3024;
    const GREEN_SIZE: i32 = 0x3023;
    const BLUE_SIZE: i32 = 0x3022;
    const CONTEXT_MAJOR_VERSION: i32 = 0x3098;
    const CONTEXT_MINOR_VERSION: i32 = 0x30FB;
    const CONTEXT_OPENGL_PROFILE_MASK: i32 = 0x30FD;
    const CONTEXT_OPENGL_CORE_PROFILE_BIT: i32 = 0x1;
    const CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT: i32 = 0x2;
    const CONTEXT_OPENGL_DEBUG: i32 = 0x31B0;
    const PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
    const PLATFORM_DEVICE_EXT: u32 = 0x313F;

    #[allow(clippy::type_complexity)]
    struct Egl {
        _lib: libloading::Library,
        get_proc: unsafe extern "C" fn(*const c_char) -> *const c_void,
        query_string: unsafe extern "C" fn(Ptr, i32) -> *const c_char,
        get_display: unsafe extern "C" fn(Ptr) -> Ptr,
        initialize: unsafe extern "C" fn(Ptr, *mut i32, *mut i32) -> u32,
        bind_api: unsafe extern "C" fn(u32) -> u32,
        choose_config: unsafe extern "C" fn(Ptr, *const i32, *mut Ptr, i32, *mut i32) -> u32,
        create_context: unsafe extern "C" fn(Ptr, Ptr, Ptr, *const i32) -> Ptr,
        destroy_context: unsafe extern "C" fn(Ptr, Ptr) -> u32,
        make_current: unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32,
        get_error: unsafe extern "C" fn() -> i32,
        /// libGL/libOpenGL, for GL 1.x functions some EGLs don't hand out.
        gl: Option<libloading::Library>,
        /// GLX, to let go of a GUI's GLX context (one thread can't have GLX and EGL current).
        glx: Option<(unsafe extern "C" fn() -> Ptr, unsafe extern "C" fn() -> Ptr, unsafe extern "C" fn(Ptr, u64, Ptr) -> i32)>,
    }
    unsafe impl Send for Egl {}
    unsafe impl Sync for Egl {}

    fn egl() -> Result<&'static Egl> {
        static EGL: OnceLock<std::result::Result<Egl, String>> = OnceLock::new();
        EGL.get_or_init(|| unsafe {
            let lib = libloading::Library::new("libEGL.so.1").or_else(|_| libloading::Library::new("libEGL.so")).map_err(|e| format!("cores that draw with OpenGL need EGL (libEGL.so.1): {e}"))?;
            macro_rules! s {
                ($n:literal) => {
                    *lib.get(concat!($n, "\0").as_bytes()).map_err(|e| format!("libEGL has no {}: {e}", $n))?
                };
            }
            let gl = libloading::Library::new("libOpenGL.so.0").or_else(|_| libloading::Library::new("libGL.so.1")).ok();
            let glx = libloading::Library::new("libGLX.so.0").or_else(|_| libloading::Library::new("libGL.so.1")).ok().and_then(|l| {
                let r = (|| -> Option<_> {
                    Some((*l.get(b"glXGetCurrentContext\0").ok()?, *l.get(b"glXGetCurrentDisplay\0").ok()?, *l.get(b"glXMakeCurrent\0").ok()?))
                })();
                std::mem::forget(l);
                r
            });
            Ok(Egl {
                get_proc: s!("eglGetProcAddress"),
                query_string: s!("eglQueryString"),
                get_display: s!("eglGetDisplay"),
                initialize: s!("eglInitialize"),
                bind_api: s!("eglBindAPI"),
                choose_config: s!("eglChooseConfig"),
                create_context: s!("eglCreateContext"),
                destroy_context: s!("eglDestroyContext"),
                make_current: s!("eglMakeCurrent"),
                get_error: s!("eglGetError"),
                _lib: lib,
                gl,
                glx,
            })
        })
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
    }

    /// A display with no window system: Mesa's surfaceless platform, else the first GPU
    /// device, else the default display.
    unsafe fn display(e: &Egl) -> Result<Ptr> {
        static DISPLAY: OnceLock<usize> = OnceLock::new();
        if let Some(d) = DISPLAY.get() {
            return Ok(*d as Ptr);
        }
        let ext = {
            let p = (e.query_string)(std::ptr::null_mut(), EXTENSIONS);
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let get_platform: Option<unsafe extern "C" fn(u32, Ptr, *const i32) -> Ptr> = {
            let p = (e.get_proc)(c"eglGetPlatformDisplayEXT".as_ptr());
            (!p.is_null()).then(|| std::mem::transmute(p))
        };
        let mut candidates = vec![];
        if let Some(gp) = get_platform {
            if ext.contains("EGL_MESA_platform_surfaceless") {
                candidates.push(gp(PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null()));
            }
            if ext.contains("EGL_EXT_platform_device") {
                let q = (e.get_proc)(c"eglQueryDevicesEXT".as_ptr());
                if !q.is_null() {
                    let q: unsafe extern "C" fn(i32, *mut Ptr, *mut i32) -> u32 = std::mem::transmute(q);
                    let mut devs = [std::ptr::null_mut(); 8];
                    let mut n = 0;
                    if q(8, devs.as_mut_ptr(), &mut n) != 0 {
                        for d in &devs[..n.max(0) as usize] {
                            candidates.push(gp(PLATFORM_DEVICE_EXT, *d, std::ptr::null()));
                        }
                    }
                }
            }
        }
        candidates.push((e.get_display)(std::ptr::null_mut()));
        for d in candidates {
            let (mut maj, mut min) = (0, 0);
            if !d.is_null() && (e.initialize)(d, &mut maj, &mut min) != 0 {
                let _ = DISPLAY.set(d as usize);
                return Ok(d);
            }
        }
        bail!("couldn't open an EGL display without a window (EGL error 0x{:x})", (e.get_error)())
    }

    pub struct Platform {
        dpy: Ptr,
        ctx: Ptr,
    }

    impl Platform {
        pub fn library() -> Result<()> {
            egl().map(|_| ())
        }

        pub fn new(req: &Request) -> Result<Platform> {
            let e = egl()?;
            unsafe {
                let dpy = display(e)?;
                if (e.bind_api)(OPENGL_API) == 0 {
                    bail!("EGL here has no desktop OpenGL");
                }
                // Any surface type: the context only ever draws into its own framebuffer.
                let attrs = [SURFACE_TYPE, 0, RENDERABLE_TYPE, OPENGL_BIT, RED_SIZE, 8, GREEN_SIZE, 8, BLUE_SIZE, 8, NONE];
                let mut config: Ptr = std::ptr::null_mut();
                let mut n = 0;
                if (e.choose_config)(dpy, attrs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
                    // EGL_KHR_no_config_context.
                    config = std::ptr::null_mut();
                }
                let mut cattrs = vec![];
                if req.core_profile {
                    cattrs.extend([CONTEXT_MAJOR_VERSION, req.major as i32, CONTEXT_MINOR_VERSION, req.minor as i32, CONTEXT_OPENGL_PROFILE_MASK, CONTEXT_OPENGL_CORE_PROFILE_BIT]);
                } else {
                    cattrs.extend([CONTEXT_OPENGL_PROFILE_MASK, CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT]);
                }
                if req.debug {
                    cattrs.extend([CONTEXT_OPENGL_DEBUG, 1]);
                }
                cattrs.push(NONE);
                let ctx = (e.create_context)(dpy, config, std::ptr::null_mut(), cattrs.as_ptr());
                if ctx.is_null() {
                    bail!("EGL couldn't make a {} context (error 0x{:x})", req.describe(), (e.get_error)());
                }
                Ok(Platform { dpy, ctx })
            }
        }

        pub fn make_current(&self) -> Result<()> {
            let e = egl()?;
            unsafe {
                // A GUI's GLX context must let go first (one thread, one API).
                if let Some((cur, cur_dpy, glx_make_current)) = e.glx {
                    if !cur().is_null() {
                        glx_make_current(cur_dpy(), 0, std::ptr::null_mut());
                    }
                }
                if (e.make_current)(self.dpy, std::ptr::null_mut(), std::ptr::null_mut(), self.ctx) == 0 {
                    bail!("eglMakeCurrent failed (error 0x{:x})", (e.get_error)());
                }
            }
            Ok(())
        }

        pub fn release(&self) {
            if let Ok(e) = egl() {
                unsafe { (e.make_current)(self.dpy, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            }
        }

        pub fn proc(&self, name: &str) -> *const c_void {
            let (Ok(e), Ok(n)) = (egl(), CString::new(name)) else { return std::ptr::null() };
            unsafe {
                let p = (e.get_proc)(n.as_ptr());
                if !p.is_null() {
                    return p;
                }
                e.gl.as_ref().and_then(|l| l.get::<*const c_void>(n.as_bytes_with_nul()).ok().map(|s| *s)).unwrap_or(std::ptr::null())
            }
        }

        pub fn destroy(&mut self) {
            if let Ok(e) = egl() {
                if !self.ctx.is_null() {
                    unsafe { (e.destroy_context)(self.dpy, self.ctx) };
                    self.ctx = std::ptr::null_mut();
                }
            }
        }
    }
}

use platform::Platform;
