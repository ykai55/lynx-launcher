# 09 — Remove the retired C++ host

**What to build:** Remove the post-cutover C++ host and adapters only after the Rust default has proven stable, leaving CMake to own the verified SDK, pinned native dependencies, runtime staging, and native tests.

**Blocked by:** 08 — Cut over the default host.

**Status:** ready-for-agent

- [ ] The retired C++ host, host-only support adapter, and obsolete fallback routing are removed without deleting shared X11 tests or CMake staging.
- [ ] Remaining consumers of the platform C ABI are reviewed before any ABI removal, with version and ownership contracts updated together if removal is safe.
- [ ] Generated artifacts and obsolete dependencies are absent from source and release dependency checks.
- [ ] Full no-display, graphical smoke, application E2E, teardown stress, provenance, and release linkage gates pass.
