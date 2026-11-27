fn main() {
    // The log callback given to cores: variadic, so written in C (src/libretro/log.c).
    println!("cargo:rerun-if-changed=src/libretro/log.c");
    cc::Build::new().file("src/libretro/log.c").warnings(true).compile("dj_core_log");
}
