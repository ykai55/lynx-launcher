# 04 — Complete Rust-host launcher input

**What to build:** Let users operate the real Rust-rendered launcher with pointer, wheel, keyboard, character, focus, and scaling behavior matching the C++ baseline.

**Blocked by:** none (03 and 10 resolved).

**Status:** resolved

- [x] Physical and logical keys, character input, pointer buttons, movement, and wheel deltas match existing behavior.
- [x] Focus transitions foreground and background the view, loss of focus closes the popup, and shutdown cancels active input.
- [x] Logical window metrics, framebuffer coordinates, system scale, and XSettings scale remain correct.
- [x] The shared X11 driver verifies popup properties, search text, filtered pixels, clicks, scaling, and focus exit for both hosts.
- [x] No-display, graphical smoke, application E2E, and teardown gates pass.

Implemented and reviewed against the C++ host and verified SDK headers. The Rust
window now uses stable boxed userdata with shared callback access and short
interior-mutability borrows, contains every GLFW callback panic, and atomically
latches failures while waking and closing the event loop. Input preserves the
C++ USB HID/logical key maps, repeat and synthesized-up bookkeeping, pointer
phase/button/scroll/timestamp semantics, text-input gating, focus ordering, and
logical/framebuffer metrics. Metric/frame calls use the existing by-value shim.

The shared X11 driver now supports scaled negative pixel assertions, scroll,
repeat, held-input, and strictly increasing X server timestamps. Rust smoke uses
a 1.25 observable scale, waits for the target SVG, enters `cobalt`, verifies real
key down/up events and the sole filtered target, clicks it, verifies the stub's
rejected Promise through the existing `ActionError`, exercises
scaled scroll and repeat, then exits through focus loss while checking synthesized
key-up, pointer cancel/remove, and background dispatch. Final logs are
`.logs/ticket-04/01-fmt.log` through `.logs/ticket-04/12-git-status.log`; the
10-iteration Rust process logs are under
`.logs/ticket-04/rust-input-smoke/run.BtECKZ/`.

The review pass also installs a panic-contained focus callback before show/focus
and the first event pump, reconciles any latched startup focus loss when stable
window userdata is installed, and emits foreground/background and input traces
only under the E2E trace flag. Ticket 05 replaces the former launch stub with
quiesced async work. ActionError assertions use ticket 10's compositor-visible,
pure-observation capture seam; polling sends no wake or Expose event.

The final review pass makes input acceptance terminal: focus loss and normal
shutdown close the gate before cancellation, and all later GLFW input callbacks
are ignored before state borrow or dispatch. Smoke logs reject any pointer, key,
or character dispatch after background. The X11 driver compares 32-bit server
times by signed modular delta and runs a no-display wrap self-test across
`UINT32_MAX`.
