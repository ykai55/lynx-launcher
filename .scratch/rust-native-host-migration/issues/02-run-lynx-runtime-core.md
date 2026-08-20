# 02 — Run the Lynx runtime core in the Rust process

**What to build:** Make a bounded Rust-host run initialize the one-per-process UI runner and GLDirect renderer, execute accepted UI and renderer tasks through the GLFW loop, and shut the empty runtime down deterministically while retaining the distinct shell-frame marker.

**Blocked by:** 01 — Establish the strict Lynx ABI seam.

**Status:** ready-for-agent

- [ ] Absolute UI deadlines and relative renderer intervals are converted without epoch drift and saturate explicitly on overflow.
- [ ] Equal-deadline tasks run FIFO, accepted tokens run exactly once, stopped queues reject new work, and final drains cannot re-enter.
- [ ] Cross-thread posts wake GLFW and only the first render thread may acquire, present, or clear the OpenGL context.
- [ ] Every new native callback contains panics and one active host is enforced explicitly.
- [ ] Deterministic scheduler, callback, ownership, bounded-process, graphical, E2E, and teardown gates pass as applicable.
