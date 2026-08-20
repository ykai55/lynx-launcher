# 05 — Launch applications from Rust Promises

**What to build:** Make application activation in the Rust host call the Rust platform launcher directly and settle the existing asynchronous UI contract with the same success and failure behavior.

**Blocked by:** 04 — Complete Rust-host launcher input.

**Status:** ready-for-agent

- [ ] The pinned weak N-API symbols are bound by their actual exported names and `launchApplication(id)` returns a real Promise.
- [ ] Successful spawn resolves asynchronously without waiting for application exit.
- [ ] Invalid identifiers and spawn failures reject with native details and drive the existing UI error state.
- [ ] Search, click, exact application identity, child process effects, and cleanup pass through the existing E2E seam for both hosts.
- [ ] No-display, graphical smoke, application E2E, and teardown gates pass.
