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
    println!("cargo:rerun-if-changed=build.rs");
}
