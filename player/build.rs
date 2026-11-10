//! The Windows executable's icon (dj.exe, as Explorer and the taskbar show it), from
//! assets/icon_cli.ico (made by scripts/make_icons.py). Building for any other system, this does
//! nothing: macOS and Linux show no icon for an executable file.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../assets/icon_cli.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // A resource script naming the icon by its full path, so it doesn't depend on the
    // directory the resource compiler runs in.
    let icon = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..").join("assets").join("icon_cli.ico");
    let rc = Path::new(&std::env::var("OUT_DIR").unwrap()).join("dj.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", icon.display().to_string().replace('\\', "\\\\"))).unwrap();
    match embed_resource::compile_for(&rc, ["dj"], embed_resource::NONE) {
        embed_resource::CompilationResult::Ok => {}
        embed_resource::CompilationResult::NotAttempted(why) => println!("cargo:warning=dj.exe gets no icon: {why} (needs a resource compiler: rc.exe from the Windows SDK, or windres)"),
        e => panic!("compiling dj.exe's icon: {e}"),
    }
}
