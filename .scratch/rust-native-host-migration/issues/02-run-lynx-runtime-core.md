# 02 — Run the Lynx runtime core in the Rust process

**What to build:** Make a bounded Rust-host run initialize the one-per-process UI runner and GLDirect renderer, execute accepted UI and renderer tasks through the GLFW loop, and shut the empty runtime down deterministically while retaining the distinct shell-frame marker.

**Blocked by:** 01 — Establish the strict Lynx ABI seam.

**Status:** claimed

- [x] Absolute UI deadlines and relative renderer intervals are converted without epoch drift and saturate explicitly on overflow.
- [x] Equal-deadline tasks run FIFO, accepted tokens run exactly once, stopped queues reject new work, and final drains cannot re-enter.
- [x] Cross-thread posts wake GLFW and only the first render thread may acquire, present, or clear the OpenGL context.
- [x] Every new native callback contains panics and one active host is enforced explicitly.
- [ ] Deterministic scheduler, callback, ownership, bounded-process, graphical, E2E, and teardown gates pass as applicable.

Remaining evidence: the empty renderer in this stage does not make the SDK post
real tasks or invoke GL callbacks. Ticket 03's real view must close the executable
native-callback evidence gap; deterministic callback-seam tests are not a
substitute for that evidence.
