# 01 — Establish the strict Lynx ABI seam

**What to build:** Give the side-by-side Rust host a reviewed, link-checked Lynx runtime and renderer ABI plus strict by-value wrappers for the five C++ float-reference calls, without changing product behavior or the default host.

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] Rust exposes only the log, process-global UI runner, GLDirect renderer, task, and opaque view handles required by the next tracer.
- [x] A project-owned C++ shim forwards exactly the five currently used float-reference calls through by-value C functions and contains no host state or policy.
- [x] Automated tests compile the shim header as C, compare callback and value layouts with the verified SDK, and link Rust declarations against the shim and SDK.
- [x] The full no-display project test and `git diff --check` pass without modifying the Lynx submodule or the default executable.
