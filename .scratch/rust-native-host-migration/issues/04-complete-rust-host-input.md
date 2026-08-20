# 04 — Complete Rust-host launcher input

**What to build:** Let users operate the real Rust-rendered launcher with pointer, wheel, keyboard, character, focus, and scaling behavior matching the C++ baseline.

**Blocked by:** 03 — Render the real launcher first frame.

**Status:** ready-for-agent

- [ ] Physical and logical keys, character input, pointer buttons, movement, and wheel deltas match existing behavior.
- [ ] Focus transitions foreground and background the view, loss of focus closes the popup, and shutdown cancels active input.
- [ ] Logical window metrics, framebuffer coordinates, system scale, and XSettings scale remain correct.
- [ ] The shared X11 driver verifies popup properties, search text, filtered pixels, clicks, scaling, and focus exit for both hosts.
- [ ] No-display, graphical smoke, application E2E, and teardown gates pass.
