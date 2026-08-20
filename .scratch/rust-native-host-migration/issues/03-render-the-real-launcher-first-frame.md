# 03 — Render the real launcher first frame

**What to build:** Load the staged core resource and application bundle into a Rust-owned Lynx view, expose the Rust platform application snapshot through a real Promise, and report readiness only after both first-screen completion and renderer present.

**Blocked by:** 02 — Run the Lynx runtime core in the Rust process.

**Status:** resolved

- [x] The resource fetcher serves only the staged Lynx core resource and preserves non-UTF-8 local path bytes.
- [x] Builder, renderer, view, client, load metadata, and application snapshot Promise have stable ownership and panic-contained callbacks.
- [x] Installed applications, names, ordering, icons, deterministic fallback, and native errors retain the existing UI contract through direct Rust platform calls.
- [x] Shell presentation and real Lynx readiness remain distinct in process logs and smoke assertions.
- [x] No-display, real-first-frame, application E2E, and teardown gates pass side-by-side with the C++ baseline.

## Comments

The Rust tracer now emits real `Launcher.getApplications`, first-screen, and
GLDirect present evidence before first-frame auto-exit. Final validation passed
the complete no-display suite, graphical C++ and Rust first-frame smoke, the
application E2E, ten repeated Rust bounded runs, and the C++ 10 bounded plus 10
first-frame teardown stress gate.

Review reopened this ticket for fetcher finalizer lease ordering, isolated XDG
fixtures, Promise rejection and settlement evidence, and repeated Rust
first-frame teardown coverage.

All review findings are closed. The final Rust black-box gate used an isolated
three-application XDG fixture, verified exact snapshot order and icon presence,
checked resolved and fallback pixels, exercised native Promise rejection and the
UI error state, and repeated both first-frame and bounded shutdown ten times.
Final logs are under `.logs/ticket-03/`, with per-process Rust logs under
`.logs/ticket-03/rust-shell-smoke/run.X9be7k/`.

The ticket was reopened for the final P1 review finding: explicitly model N-API
deferred settlement ownership so post-settlement panic handling cannot reuse a
consumed deferred.

The final P1 is closed. A single deferred owner now removes the handle only on a
successful resolve or reject, panic fallback checks that pending ownership, and
every callback returns its original Promise. Deterministic tests cover
post-resolve panic, resolve-failure rejection, and pre-settle panic rejection.
P1 logs are `.logs/ticket-03/15-p1-fmt.log` through
`.logs/ticket-03/22-p1-git-status.log`; graphical process logs are under
`.logs/ticket-03/rust-shell-smoke/run.5w9eku/`.

The ticket was reopened because a failed `napi_reject_deferred` still left a
pending Promise eligible for reuse and return.

The reject-failure gap is closed. Deferred ownership now has explicit pending,
settled, and fatal-settlement-failure states. A failed reject poisons and drops
access to the deferred, attempts one static synchronous throw, and makes the
callback return NULL; resolve failure may still fall back to one reject. Logs
are `.logs/ticket-03/23-fatal-settlement-fmt.log` through
`.logs/ticket-03/30-fatal-settlement-git-status.log`, with graphical process
logs under `.logs/ticket-03/rust-shell-smoke/run.rX12qs/`.
