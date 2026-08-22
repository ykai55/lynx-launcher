# 08 — Cut over the default host

**What to build:** Make the parity-proven Rust executable the default launcher through a small reversible routing change while retaining the C++ binary as an explicit fallback for the cleanup window.

**Blocked by:** 07 — Prove Rust-host lifecycle parity.

**Status:** resolved

- [x] Build, run, smoke, E2E, and teardown entry points select the Rust host by default and can select the C++ fallback explicitly.
- [x] Runtime staging, `$ORIGIN` dependency checks, pinned GLFW linkage, and CMake ownership remain unchanged.
- [x] Full no-display and repeated graphical parity gates pass through the default entry points.
- [x] Architecture and contributor documentation describe the new default and the temporary fallback boundary.
