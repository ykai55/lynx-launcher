# 06 — Complete Rust desktop integration

**What to build:** Preserve Lynx clipboard and cursor requests in the Rust popup so text interactions and pointer feedback match the production host.

**Blocked by:** 04 — Complete Rust-host launcher input.

**Status:** resolved

- [x] Lynx clipboard reads and writes use the pinned GLFW/X11 desktop clipboard with callback-safe storage.
- [x] Supported Lynx cursor requests map to native cursors and unsupported requests retain the current deterministic fallback.
- [x] Cursor and clipboard resources remain valid through callback use and are released during ordered teardown.
- [x] Shared graphical and teardown tests cover the observable desktop behavior without browser assumptions.
