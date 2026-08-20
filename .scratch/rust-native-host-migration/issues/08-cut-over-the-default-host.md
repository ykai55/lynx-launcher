# 08 — Cut over the default host

**What to build:** Make the parity-proven Rust executable the default launcher through a small reversible routing change while retaining the C++ binary as an explicit fallback for the cleanup window.

**Blocked by:** 07 — Prove Rust-host lifecycle parity.

**Status:** ready-for-agent

- [ ] Build, run, smoke, E2E, and teardown entry points select the Rust host by default and can select the C++ fallback explicitly.
- [ ] Runtime staging, `$ORIGIN` dependency checks, pinned GLFW linkage, and CMake ownership remain unchanged.
- [ ] Full no-display and repeated graphical parity gates pass through the default entry points.
- [ ] Architecture and contributor documentation describe the new default and the temporary fallback boundary.
