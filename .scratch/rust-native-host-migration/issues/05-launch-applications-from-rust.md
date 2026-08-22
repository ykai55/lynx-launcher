# 05 — Launch applications from Rust Promises

**What to build:** Make application activation in the Rust host call the Rust platform launcher directly and settle the existing asynchronous UI contract with the same success and failure behavior.

**Blocked by:** none (04 and 10 resolved).

**Status:** resolved

- [x] The pinned weak N-API symbols are bound by their actual exported names and `launchApplication(id)` returns a real Promise.
- [x] Successful spawn resolves asynchronously without waiting for application exit.
- [x] Invalid identifiers and spawn failures reject with native details and drive the existing UI error state.
- [x] Search, click, exact application identity, child process effects, and cleanup pass through the existing E2E seam for both hosts.
- [x] No-display, graphical smoke, application E2E, and teardown gates pass.

The Rust host now binds the verified weak N-API async-work symbols, parses the
single UTF-8 application ID, keeps `ViewState` alive across execute/completion,
calls the Rust `Launcher` only on the worker, and settles the deferred exactly
once on the JS thread. Rust E2E verifies both the target process marker and a
real spawn failure with native error detail; the C++ path remains the default.

Shutdown closes launch registration before releasing the view and waits for all
registered work to settle and delete its N-API handle. Timeout or deletion
failure latches process-fatal callback ownership and preserves the runtime for
OS cleanup. E2E separately proves early Promise resolution, spawn failure,
unknown-ID rejection, and immediate click-to-defocus quiescence. Earlier
ActionError diagnosis logs remain under `.logs/ticket-05/`; ticket 10 records
the corrected conclusion that synthetic capture wakeups invalidated the old
pixel gate rather than exposing a launch or React scheduling defect.

Final review evidence is `.logs/ticket-05/17-full-test.log` through
`.logs/ticket-05/26-final-full-test.log`. Rust smoke passed 10/10; Rust E2E
passed success, spawn-failure, unknown-ID, and immediate-defocus 10/10 each;
the pure-observation failure gate passed 32/32 for both C++ and Rust.

The final P2/P3 review adds a strictly bounded, disabled-by-default E2E worker
delay. Its immediate-defocus gate passed 10/10 while asserting the live host
identity and `worker pending -> shutdown waiting -> Promise resolved -> work
quiesced -> clean exit` ordering. Evidence is in the ticket05 logs numbered 33
through 43.
