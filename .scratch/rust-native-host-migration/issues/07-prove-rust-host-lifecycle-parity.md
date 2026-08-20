# 07 — Prove Rust-host lifecycle parity

**What to build:** Apply the full black-box smoke, application, provenance, and repeated teardown surface to the side-by-side Rust host and make every bounded and first-frame run exit cleanly.

**Blocked by:** 05 — Launch applications from Rust Promises; 06 — Complete Rust desktop integration.

**Status:** ready-for-agent

- [ ] Shutdown cancels input, backgrounds and releases view/client, stops renderer acceptance, drains renderer and UI work, then releases owners and GLFW resources in the proven order.
- [ ] Both check and windowed modes reject injected Lynx libraries and use only executable-relative staged resources and dependencies.
- [ ] Repeated bounded and real-first-frame runs retain all timeout, first-frame, Lynx-error, nonzero-exit, and forbidden-string failures.
- [ ] The existing X11 driver, application E2E, and teardown stress scripts select and exercise either host without weakened assertions.
- [ ] Parity gates pass repeatedly and the C++ executable remains the default.
