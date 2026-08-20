use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(lynx_sys_cmake_link)");

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo did not provide target OS");
    let target_arch =
        env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo did not provide target architecture");
    assert!(
        target_os == "linux" && target_arch == "x86_64",
        "lynx-sys supports only the verified Linux x86_64 SDK"
    );

    let sdk = PathBuf::from(
        env::var_os("LYNX_SDK_DIR")
            .expect("LYNX_SDK_DIR must point to the verified Lynx SDK extraction"),
    );
    let library = sdk.join("lib/liblynx.so");
    assert!(
        library.is_file(),
        "verified Lynx shared library is missing: {}",
        library.display()
    );

    let library_directory = sdk.join("lib");
    let library_directory = library_directory
        .to_str()
        .expect("LYNX_SDK_DIR must be UTF-8 for Cargo linker directives");
    println!("cargo:rustc-link-search=native={library_directory}");

    if let Some(archive) = env::var_os("LYNX_LAUNCHER_LYNX_SHIM_ARCHIVE") {
        let archive = PathBuf::from(archive);
        assert!(
            archive.is_file()
                && archive
                    .file_name()
                    .is_some_and(|value| value == "liblynx_sys_shim.a"),
            "LYNX_LAUNCHER_LYNX_SHIM_ARCHIVE must name the CMake-built static archive"
        );
        let archive_directory = archive
            .parent()
            .and_then(|directory| directory.to_str())
            .expect("the Lynx shim archive path must be UTF-8 for Cargo linker directives");
        println!("cargo:rustc-link-search=native={archive_directory}");
        println!("cargo:rustc-link-lib=static=lynx_sys_shim");
        println!("cargo:rustc-cfg=lynx_sys_cmake_link");
        println!("cargo:rerun-if-changed={}", archive.display());
    }

    println!("cargo:rustc-link-lib=dylib=lynx");
    println!("cargo:rerun-if-env-changed=LYNX_SDK_DIR");
    println!("cargo:rerun-if-env-changed=LYNX_LAUNCHER_LYNX_SHIM_ARCHIVE");
    println!("cargo:rerun-if-changed=build.rs");
}
