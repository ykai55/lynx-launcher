# 09 — Remove the retired C++ host

**What to build:** Remove the post-cutover C++ host and adapters only after the Rust default has proven stable, leaving CMake to own the verified SDK, pinned native dependencies, runtime staging, and native tests.

**Blocked by:** 08 — Cut over the default host.

**Status:** resolved

- [x] The retired C++ host, host-only support adapter, and obsolete fallback routing are removed without deleting shared X11 tests or CMake staging.
- [x] Remaining consumers of the platform C ABI are reviewed before any ABI removal, with version and ownership contracts updated together if removal is safe.
- [x] Generated artifacts and obsolete dependencies are absent from source and release dependency checks.
- [x] Full no-display, graphical smoke, application E2E, teardown stress, provenance, and release linkage gates pass.

## Comments

Removed in ticket 09:

- Deleted `host/src/main.cc`, `host/src/support.cc`, `host/src/support.h`,
  `host/tests/support_test.cc`, `platform/src/ffi.rs`,
  `platform/tests/ffi.rs`, and `platform/include/lynx_launcher.h`.
- Inlined `XSettingsWindowScale` into the anonymous namespace of
  `host/tests/x11_click_test_driver.cc` and dropped the `support.h` include; the
  driver's PPM/niri/grim/selftest behavior is unchanged.
- `platform` is a pure rlib: `crate-type = ["rlib"]` only, with the C ABI module
  and header removed. `host-rs` continues to consume the direct `Launcher` API.
- `host/CMakeLists.txt` no longer builds a C++ executable, `host_support`, the
  imported `launcher_platform` static library, or the platform staticlib Cargo
  command; C++-only CTest cases were removed. CMake keeps the shim, GLFW,
  runtime resources, Rust workspace/staging, the X11 driver, and the
  lynx-sys ABI/Rust ABI and Rust-host resource/provenance tests.
- Scripts route exclusively to the Rust host; `cpp`/`both` selections die as
  retired, and stale `host/build/lynx-launcher-cpp` artifacts are removed at
  build time.
- README/ARCHITECTURE/LYNX_SDK_SIZE reflect the removed C++ host and C ABI;
  the migration spec keeps its historical problem statement labeled as such.
- Logs: `.logs/ticket-09/`.
