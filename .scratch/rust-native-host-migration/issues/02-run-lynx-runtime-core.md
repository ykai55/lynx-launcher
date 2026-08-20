# 02 — Run the Lynx runtime core in the Rust process

**What to build:** Make a bounded Rust-host run initialize the one-per-process UI runner and GLDirect renderer, execute accepted UI and renderer tasks through the GLFW loop, and shut the empty runtime down deterministically while retaining the distinct shell-frame marker.

**Blocked by:** 01 — Establish the strict Lynx ABI seam.

**Status:** resolved

- [x] Absolute UI deadlines and relative renderer intervals are converted without epoch drift and saturate explicitly on overflow.
- [x] Equal-deadline tasks run FIFO, accepted tokens run exactly once, stopped queues reject new work, and final drains cannot re-enter.
- [x] Cross-thread posts wake GLFW and only the first render thread may acquire, present, or clear the OpenGL context.
- [x] Every new native callback contains panics and one active host is enforced explicitly.
- [x] Deterministic scheduler, callback, ownership, bounded-process, graphical, E2E, and teardown gates pass as applicable.

Ticket 03 supplied the missing executable evidence: the real view drove SDK UI
work and GLDirect present callbacks, and repeated graphical shutdown completed
without forbidden lifecycle logs.

The ticket was reopened after review found that the UI draining lease ended
before asynchronous fetcher finalization was proven quiescent.

The review finding is closed: deterministic tests hold the draining generation
through delayed fetcher finalization and latch timeout as fatal process cleanup.
Ten repeated Rust first-frame and bounded runs completed with clean shutdown.
