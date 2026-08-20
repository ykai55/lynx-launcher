# 03 — Render the real launcher first frame

**What to build:** Load the staged core resource and application bundle into a Rust-owned Lynx view, expose the Rust platform application snapshot through a real Promise, and report readiness only after both first-screen completion and renderer present.

**Blocked by:** 02 — Run the Lynx runtime core in the Rust process.

**Status:** ready-for-agent

- [ ] The resource fetcher serves only the staged Lynx core resource and preserves non-UTF-8 local path bytes.
- [ ] Builder, renderer, view, client, load metadata, and application snapshot Promise have stable ownership and panic-contained callbacks.
- [ ] Installed applications, names, ordering, icons, deterministic fallback, and native errors retain the existing UI contract through direct Rust platform calls.
- [ ] Shell presentation and real Lynx readiness remain distinct in process logs and smoke assertions.
- [ ] No-display, real-first-frame, application E2E, and teardown gates pass side-by-side with the C++ baseline.
