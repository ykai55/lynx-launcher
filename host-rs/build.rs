use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    if let Some(archive) = env::var_os("LYNX_LAUNCHER_GLFW_ARCHIVE") {
        let archive = PathBuf::from(archive);
        assert!(
            archive.is_file() && archive.extension().is_some_and(|value| value == "a"),
            "LYNX_LAUNCHER_GLFW_ARCHIVE must name the pinned GLFW static archive"
        );
        let archive = archive
            .to_str()
            .expect("the pinned GLFW archive path must be UTF-8 for Cargo linker directives");
        for argument in [
            archive,
            "-lX11",
            "-lGLX",
            "-lOpenGL",
            "-lpthread",
            "-ldl",
            "-lm",
            "-lrt",
        ] {
            println!("cargo:rustc-link-arg-bin=lynx-launcher-rs={argument}");
        }
        println!("cargo:rerun-if-changed={archive}");
    }
    println!("cargo:rerun-if-env-changed=LYNX_LAUNCHER_GLFW_ARCHIVE");
    if env::var_os("CARGO_FEATURE_NATIVE_WAYLAND").is_some() {
        pkg_config::Config::new()
            .atleast_version("1.20")
            .probe("wayland-client")
            .expect("native Wayland requires wayland-client >= 1.20");
        pkg_config::Config::new()
            .probe("wayland-egl")
            .expect("native Wayland requires wayland-egl");
        pkg_config::Config::new()
            .probe("egl")
            .expect("native Wayland requires EGL");
        pkg_config::Config::new()
            .atleast_version("1.0")
            .probe("xkbcommon")
            .expect("native Wayland requires xkbcommon >= 1.0");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
