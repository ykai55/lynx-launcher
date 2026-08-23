# Complete the Remaining Native Host Migration to Rust

Status: completed

> **Historical problem statement.** This statement describes the starting point
> of the migration. As of ticket 09 the retired C++ host, the host-only support
> adapter, and the platform C ABI have been removed: `host-rs` is the only host,
> and the `platform` crate is a pure rlib consumed directly through its Rust
> interface. Statements below that reference the C++ fallback or the C ABI
> describe the migration window and are retained for historical context only.

## Problem Statement

Lynx Launcher now has a side-by-side Rust host that owns CLI handling, executable-relative runtime validation, the pinned GLFW/X11 popup window, an OpenGL shell frame, focus exit, and bounded event pumping. However, the default C++ host still owns the behavior that turns that shell into the real launcher: the process-global Lynx UI runner, deadline queues, renderer callbacks, Lynx view and resource lifecycle, input translation, clipboard and cursor integration, the `Launcher` native module, and exact teardown ordering.

From the user's perspective, this leaves two native hosts with different capabilities. The Rust binary proves native integration but cannot render or operate the ReactLynx launcher. The C++ binary remains the only usable launcher, so native ownership is duplicated, lifecycle fixes must still be made in C++, and the project cannot complete its gradual Rust migration or simplify the platform call path.

## Solution

Gradually move the remaining product behavior of the native host into the existing Rust host while preserving every externally observable contract. The Rust host will become capable of loading the packaged Lynx runtime, rendering a real ReactLynx first screen, processing input, exposing the asynchronous `NativeModules.Launcher` interface, launching applications through the Rust platform module, and shutting down without forbidden lifecycle failures.

The migration will remain side-by-side until parity is demonstrated. A narrow project-owned C++ shim will convert the currently used C++ float-reference functions into strict by-value C calls; no host policy or lifecycle behavior will live in that shim. CMake will continue to own the pinned SDK, pinned GLFW, native system libraries, runtime staging, and native test driver. The default executable will switch only after the Rust binary passes the existing black-box test surface without weakened assertions.

## User Stories

1. As a launcher user, I want the Rust host to display the same ReactLynx launcher UI, so that changing native implementation does not change the product I use.
2. As a launcher user, I want installed applications to appear with the same names and ordering, so that the Rust migration does not alter discovery behavior.
3. As a launcher user, I want application icons to render with the same deterministic fallback behavior, so that missing or invalid icons do not degrade the interface.
4. As a launcher user, I want search to remain trimmed and case-insensitive, so that filtering behaves identically after cutover.
5. As a launcher user, I want clicking an application to launch exactly that application, so that the host migration cannot misroute actions.
6. As a launcher user, I want launch failures to be shown through the existing UI error state, so that native errors remain actionable.
7. As a launcher user, I want the popup to remain focusable, undecorated, floating, and absent from taskbar and pager lists, so that desktop behavior remains unchanged.
8. As a launcher user, I want the launcher to close when it loses focus, so that popup interaction remains predictable.
9. As a launcher user, I want system and XSettings scaling to remain correct, so that logical layout and physical rendering remain usable on HiDPI desktops.
10. As a launcher user, I want pointer, wheel, keyboard, and character input to behave as before, so that search and application selection remain responsive.
11. As a launcher user, I want clipboard operations exposed by Lynx to use the native desktop clipboard, so that text interactions continue to work.
12. As a launcher user, I want native cursors to match Lynx cursor requests, so that pointer feedback remains correct.
13. As a launcher user, I want the first visible frame to be a real Lynx-rendered frame, so that a static shell frame is never mistaken for application readiness.
14. As a launcher user, I want startup failures to exit with a clear error rather than hang or show an empty window, so that broken runtime resources are diagnosable.
15. As a launcher user, I want the launcher to exit cleanly after bounded runs and first-frame smoke runs, so that automated and interactive use do not leak processes.
16. As a maintainer, I want the Rust host to use the existing Rust platform interface directly, so that application policy no longer crosses an unnecessary C ABI inside the final host.
17. As a maintainer, I want the existing platform C ABI preserved while the C++ host remains, so that migration does not break the baseline implementation or external consumers prematurely.
18. As a maintainer, I want Lynx raw bindings to stay narrow and reviewed, so that the project exposes only SDK contracts it actually uses.
19. As a maintainer, I want the five currently used float-reference functions wrapped by strict by-value C calls, so that Rust never hard-codes a C++ reference ABI.
20. As a maintainer, I want every native callback to contain Rust panics, so that unwinding never crosses GLFW, X11, Lynx, or N-API boundaries.
21. As a maintainer, I want callback userdata to have stable ownership and lifetime, so that concurrent callbacks cannot access moved or destroyed host state.
22. As a maintainer, I want the process-global UI runner to permit only one active host, so that unsupported concurrent hosts fail explicitly.
23. As a maintainer, I want absolute monotonic UI deadlines preserved, so that task timing does not drift through an incorrect clock epoch conversion.
24. As a maintainer, I want relative renderer intervals preserved, so that renderer scheduling matches the pinned SDK contract.
25. As a maintainer, I want equal-deadline tasks to retain stable FIFO ordering, so that callback execution is deterministic.
26. As a maintainer, I want cross-thread task posts to wake the GLFW event loop, so that the UI does not stall until an unrelated native event occurs.
27. As a maintainer, I want task tokens consumed exactly once, so that native work cannot run twice or leak.
28. As a maintainer, I want the first render thread to become the stable OpenGL owner, so that no other thread makes the context current or presents a frame.
29. As a maintainer, I want renderer calls from the wrong thread to fail safely, so that context ownership violations are visible rather than corrupting state.
30. As a maintainer, I want the resource fetcher restricted to the packaged Lynx core resource, so that migration does not open arbitrary filesystem or network access.
31. As a maintainer, I want non-UTF-8 Linux runtime paths to remain byte-preserving, so that valid local resources are not corrupted by lossy conversion.
32. As a maintainer, I want the real `Launcher` methods to return N-API promises, so that the UI contract remains asynchronous.
33. As a maintainer, I want promise failures to reject with native error details, so that errors are neither swallowed nor converted into synchronous exceptions.
34. As a maintainer, I want application discovery and process launch to remain outside the UI thread when required, so that native work does not block rendering.
35. As a maintainer, I want shutdown to stop accepting new work before final drains, so that release callbacks cannot re-enter destroyed state.
36. As a maintainer, I want view and client release to occur while their queues can still process release work, so that Lynx teardown follows the proven ordering.
37. As a maintainer, I want renderer and UI queues drained before their owners are released, so that pending native tasks cannot outlive resources.
38. As a maintainer, I want GLFW window and process-global resources released last, so that dependent callbacks retain valid native infrastructure during teardown.
39. As a maintainer, I want forbidden lifecycle log strings to remain hard failures, so that the Rust migration cannot normalize leaks or unknown tasks.
40. As a maintainer, I want the C++ host to remain the default until parity is proven, so that incremental work cannot replace the production baseline early.
41. As a maintainer, I want the Rust host selectable side-by-side during migration, so that behavior can be compared against the same fixtures and SDK.
42. As a maintainer, I want the same runtime staging and `$ORIGIN` provenance checks used by both hosts, so that local environment libraries cannot mask packaging defects.
43. As a maintainer, I want the pinned GLFW static artifact shared by both hosts, so that the Rust host cannot silently adopt a different window backend.
44. As a maintainer, I want CMake retained through the migration, so that SDK verification, X11/OpenGL discovery, resource staging, and native tests remain reproducible.
45. As a test author, I want the existing X11 driver to exercise both hosts, so that popup, pixels, focus, input, and click behavior are checked through one black-box seam.
46. As a test author, I want Rust scheduling invariants covered by deterministic tests, so that deadline, FIFO, stop, drain, and overflow behavior can be diagnosed without a graphical race.
47. As a test author, I want first-screen and first-present tracked separately, so that readiness requires both layout and an actual renderer present.
48. As a test author, I want repeated bounded and first-frame shutdown runs, so that intermittent callback and ownership bugs become reproducible.
49. As a release engineer, I want the Rust binary to retain only staged native dependencies, so that release behavior does not depend on the caller's working directory or system GLFW.
50. As a release engineer, I want the final default-binary switch to be a small reversible change after parity, so that rollout risk is isolated from implementation work.
51. As a contributor, I want module ownership documented throughout the migration, so that application policy, host lifecycle, UI state, and SDK adapters do not drift across seams.
52. As a contributor, I want obsolete C++ adapters removed only after cutover, so that cleanup cannot erase a still-required baseline or ABI contract.

## Implementation Decisions

- The migration is staged and side-by-side; it is not a big-bang rewrite.
- The primary product module is the existing Rust host. The C++ host remained
  the default adapter until all parity gates passed and was removed by ticket 09.
- The raw Lynx module will expose only reviewed functions, opaque handles, callback tables, enums, and layouts used by the launcher.
- A tiny project-owned C++ shim will expose by-value wrappers for the five float-reference functions currently used by the host. The shim contains no task scheduling, resource policy, input logic, N-API behavior, or lifecycle state.
- The first implementation stage establishes the strict Lynx ABI surface and compile/link/layout checks needed by subsequent stages.
- The runtime-core stage migrates the process-global UI runner, UI deadline queue, renderer interval queue, event-loop wakeup, task-token ownership, and renderer callback wiring together. Scheduling will not be ported as disconnected unused code.
- Absolute UI deadlines remain monotonic nanosecond targets. Relative renderer delays remain intervals. Saturation and overflow behavior must be explicit.
- Equal-deadline tasks preserve insertion order. Queue stop prevents new work; drain consumes only already accepted work and must not permit re-entry.
- Callback state has a stable heap address for its full registration lifetime. Concurrent callbacks use raw pointers internally with atomics, mutexes, or thread-owned state rather than constructing arbitrary mutable references.
- Every callback entered from GLFW, X11, Lynx, resource fetching, view clients, renderer delegates, or N-API contains panic before returning across FFI.
- The process supports one active Lynx host. Process-global runner configuration and active/draining state are explicit.
- The renderer stage preserves stable OpenGL ownership by the first render thread. The platform thread may prepare the shell context before renderer startup, but no second renderer thread may acquire or present it.
- The view stage creates the packaged resource fetcher, builder, renderer binding, view, and view client; loads the packaged bundle; and distinguishes first-screen completion from first renderer present.
- The resource fetcher serves only the packaged core resource and keeps byte-preserving local file behavior.
- The input stage migrates physical and logical key mapping, pointer, scroll, character input, focus foreground/background, window metrics, clipboard, cursor, and input cancellation without browser assumptions.
- The native module stage binds the pinned SDK's weak N-API symbol names directly and preserves real Promise resolve/reject behavior.
- The Rust host calls the Rust platform module directly for application snapshots, icon resolution, and launch. The platform C ABI was compiled and tested until the C++ host was retired; as of ticket 09 the ABI, its header, and the `staticlib` crate-type are removed, and the platform crate is a pure rlib.
- Shutdown preserves the proven order: cancel input; background and release view/client; stop accepting renderer work; drain renderer and UI work while runners are alive; release renderer, fetcher, platform state, cursors, and window resources last.
- The default executable changed only after the Rust host passed all baseline and parity gates side-by-side. Cleanup of the old host, support adapter, and platform C ABI was completed as ticket 09.
- CMake continues to own the verified Lynx SDK, pinned static GLFW, X11/OpenGL system linkage, `$ORIGIN` runtime layout, resource refresh, CTest, and the native X11 test driver.
- Changes to the pinned SDK, patch set, gitlink, CMake removal, or window backend are separate architectural efforts.

## Testing Decisions

- The primary acceptance seam is the executable process. Tests invoke the side-by-side Rust host through its existing CLI and observe window properties, pixels, input, native module behavior, process exit, logs, and child-process effects. Tests do not assert private Rust struct layout or callback implementation details.
- The existing no-display project test remains the baseline for formatting, Clippy, Rust tests, UI tests and type checking, native build, CTest, resource validation, and ABI checks.
- The existing graphical first-frame workflow remains the readiness seam. A Rust host may claim Lynx readiness only after both first-screen completion and renderer present; the shell-frame marker remains distinct until then.
- The existing X11 driver is reused for popup properties, rendered pixels, focus loss, pointer clicks, keyboard text input, scaling, and application launch behavior.
- The existing search, icon, and launch E2E remains the application interaction seam. Assertions are not weakened to accommodate the Rust implementation.
- The existing teardown stress workflow remains the lifecycle seam. It runs bounded and first-frame modes repeatedly and retains the current forbidden-string contract.
- The existing resource provenance checks are applied to the Rust host in both check and windowed modes, including rejection of an externally injected Lynx library.
- Deterministic module tests cover scheduling facts that are difficult to diagnose solely through the process seam: absolute versus relative deadlines, equal-deadline FIFO order, overflow saturation, wakeup requests, stop-accepting behavior, drain behavior, and exactly-once task ownership.
- ABI tests compile the shim header as C, link the Rust declarations against the verified SDK, and check all project-owned callback and value layouts used across FFI.
- Callback tests use controlled panic injection where a real callback seam exists and assert conversion to a safe error or shutdown signal; they do not mock internal helper calls.
- Renderer ownership tests assert that one render thread succeeds and a different render thread is rejected without acquiring, presenting, or clearing the context.
- Native module tests assert real Promise resolution and rejection shapes through the same interface consumed by the UI.
- Non-UTF-8 path tests use real Linux path bytes and validate resource lookup and file URI behavior without lossy conversion.
- Each migration stage runs the no-display suite. Stages affecting window, renderer, input, native modules, or lifecycle also run graphical smoke, application E2E, and teardown stress.
- Before default cutover, parity runs are repeated rather than relying on one successful invocation. Any timeout, missing real first frame, nonzero host exit, Lynx error marker, test failure, or forbidden lifecycle string is a failure.

## Out of Scope

- Replacing CMake or the verified SDK staging mechanism.
- Replacing the pinned GLFW target with a system GLFW, a crate-bundled GLFW, or another window library.
- Native Wayland support; Wayland desktops continue through XWayland and require `DISPLAY`.
- Upgrading the Lynx gitlink, SDK, Habitat graph, Node.js, pnpm, Rust toolchain, or unrelated native dependencies.
- Editing the Lynx submodule directly or broadening the patch set as part of host migration.
- Hard-coding the Linux C++ reference ABI in Rust to eliminate the narrow C++ shim.
- Moving application discovery, desktop entry policy, icon lookup, or process command construction into host code.
- Moving OS policy into the ReactLynx UI.
- Expanding the resource fetcher to arbitrary filesystem or network resources.
- Adding full IME composition, new input features, installer support, packaging formats, or desktop integration beyond existing behavior.
- Removing the platform C ABI before the C++ baseline is retired and remaining consumers are reviewed. *(Historical guard; the C ABI was removed by ticket 09 after the C++ host was retired.)*
- Deleting the C++ host before side-by-side parity and default cutover are complete. *(Historical guard; the C++ host was removed by ticket 09 after cutover.)*

## Further Notes

- The current Rust host already proves CLI, runtime provenance, popup X11 behavior, pinned GLFW linkage, OpenGL shell presentation, focus exit, and bounded event pumping. The next implementation work begins at the Lynx runtime seam, not at window creation.
- The remaining C++ host behavior is highly lifecycle-coupled. Ticket decomposition should use tracer bullets that end in observable process behavior rather than horizontal tickets for all bindings, all queues, or all callbacks.
- The known SDK currently exposes additional C++ reference APIs beyond the five used by the launcher. This specification covers only the five functions required by current host behavior and does not claim that the full SDK is a strict C ABI.
- Existing upstream warning noise remains governed by the repository's documented allowlist. New warnings or errors must be investigated rather than appended automatically.
- This spec is an umbrella for gradual completion. A ticketing pass should preserve the stage ordering and blocking edges described above.
