# 10 — Make visible-pixel capture observational

**What to build:** Make E2E pixel assertions observe compositor-visible output
without sending input, Expose, or any other event that can advance the host.

**Blocked by:** none.

**Blocks:** none (04 and 05 resolved).

**Status:** resolved

- [x] Validate the target is visible, unobscured, and owns the focus chain.
- [x] Prefer native X11 root readback when it is available.
- [x] Use bounded niri/grim compositor capture only for the niri/XWayland path.
- [x] Execute capture tools directly, with caller-deadline process cleanup.
- [x] Keep all pixel polling purely observational.
- [x] Pass 32/32 C++ and 32/32 Rust launch-failure assertions without rescue input.

The final root cause was the test observation seam. Under the tested
niri/XWayland session, `XGetImage(target window)` intermittently returned stale
window contents even though GL readback and compositor capture contained the
`ActionError` banner. Synthetic Expose events made the old assertion appear more
reliable by advancing host work, so those events could not be part of a valid
gate.

The driver now tries X11 root readback first. If niri/XWayland requires
compositor capture, it validates focused floating-window and output geometry,
runs `niri` and `grim` directly, parses bounded PPM output, and shares the
assertion's timeout deadline with both child processes. Native X11 root capture
does not require either tool. Timeout, read failure, or size overflow kills and
reaps the capture child. Selftests cover parsing, scaling, geometry bounds,
deadline cleanup, and output limits.

Historical diagnosis under `.logs/issue10/` includes rejected React, Clay,
frame-scheduler, microtask, and GL hypotheses. None is retained in source or the
Lynx patch set; the pinned submodule remains clean. Final 32/32 evidence is in
`.logs/ticket-05/23-targeted-cpp-failure32-scale2.log` and
`.logs/ticket-05/24-targeted-rust-failure32.log`.
