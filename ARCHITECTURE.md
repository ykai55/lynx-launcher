# Architecture

## System shape

Lynx Launcher is split into three application layers around a pinned Lynx SDK:

```text
ReactLynx UI (TypeScript)
        |
        | NativeModules.Launcher promises
        v
C++ host (N-API + Lynx C API + GLFW/OpenGL)
        |
        | lynx_launcher.h C ABI
        v
Rust platform layer (XDG discovery, icons, process launch)
```

The boundaries are deliberate. The UI has no browser DOM dependency, the C++
host contains embedder and graphics concerns, and Rust owns operating-system
policy and parsing. `third_party/lynx` is an implementation dependency rather
than an application layer.

`host-rs/` owns the default native host. It implements the executable CLI and
pure host support behavior, directly
uses the Rust platform interface for discovery, and verifies that its linked
`lynx_log_init` symbol resolves to the staged `$ORIGIN/liblynx.so`. Its windowed
path owns a popup-like pinned-GLFW X11 window, OpenGL 3.3 context, static shell
frame, focus exit, bounded event loop, process-global UI runner, deadline queues,
GLDirect renderer, restricted core-resource fetcher, builder, view, client, and
load metadata with deterministic teardown. It loads the staged ReactLynx bundle
and exposes a Promise-based application snapshot through the SDK's verified weak
N-API symbols. Pointer, wheel, keyboard, character, focus, metric, and async
application-launch, clipboard, cursor, and lifecycle parity are implemented.
The C++ binary remains available as an explicit fallback until post-cutover
cleanup completes.

## Layers

### Rust platform

`platform/src/lib.rs` discovers Linux desktop files in XDG priority order,
filters non-launchable entries, expands the supported `Exec` syntax, resolves
icons, and starts child processes. Platform-specific behavior is selected with
Rust `cfg` gates; unsupported systems return `UnsupportedPlatform` rather than
silently emulating Linux.

`platform/src/ffi.rs` exports the stable interface in
`platform/include/lynx_launcher.h`. The ABI uses opaque handles, explicit status
codes, byte slices instead of NUL-terminated strings, matching destroy functions,
and panic containment. This keeps Rust layout and allocator details out of C++.

### Native host

`host/src/main.cc` is the temporary C++ fallback. It owns its GLFW window,
OpenGL context, Lynx view, renderer callbacks, task queues, input translation,
resource loading, and `Launcher` N-API module while the cutover remains
reversible. It links the Rust platform static library and pinned Lynx shared
library.

`host/src/support.cc` contains independently testable path, file, URI, and UTF-8
operations. CMake copies `liblynx.so`, ICU data, `lynx_core.js`, and the UI bundle
into `host/build`, allowing runtime paths to be resolved relative to the
executable rather than the caller's current directory. Core JS is copied both to
`resources/lynx_core.js` for the host's explicit path and to `$ORIGIN/lynx_core.js`
for the engine preloader's default lookup.

`host-rs/src/support.rs` preserves those pure path, file, URI, XSettings, scroll,
UTF-8, key-mapping, input-bookkeeping, and window-metric semantics. `lynx-sys/`
contains the reviewed runtime, renderer, view/resource, and weak N-API ABI,
strict by-value view wrappers, and dynamic-loader path probe. The Rust runtime
tracer configures the process-global UI runner once, executes absolute UI
deadlines and relative renderer intervals through the GLFW loop, and binds the
GLDirect callbacks. Its renderer callback userdata has a stable heap address;
a process registry acquires an `Arc` before state access and remains installed
through renderer release. View, client, native-module, and fetcher userdata share
an `Arc`-owned state that outlives SDK callbacks, while the fetcher owns one
explicit reference consumed by its finalizer.

The binary-private Rust window module keeps `RuntimeCore`, metrics, and input in
a stable boxed userdata object. GLFW callbacks recover only a shared reference;
short `RefCell` borrows serialize platform-thread mutation rather than creating a
long-lived arbitrary `&mut`. Every callback contains panics, atomically latches
failure, wakes the loop, and requests close. Input follows the C++ USB HID and
logical-key mappings, tracks down/repeat/up, synthesizes missing key-up and
pointer cancel/remove events, and gates UTF-8 character events on Lynx text-input
activation. Focus loss and normal shutdown first close a one-way input-acceptance
gate, then cancel active state. Later GLFW cursor, pointer, key, character, and
synthetic release callbacks return before borrowing state or dispatching, so no
input can follow the view's background transition.

Before runtime userdata exists, a separate panic-contained focus callback is
installed ahead of show/focus and the initial GLFW event pump. It latches any
FocusOut and requests close. Formal callback registration replaces it only after
the stable window state exists, then reconciles the latched close flag with
GLFW's current focus attribute so an early loss backgrounds and shuts down the
new view instead of being forgotten.

The Rust fetcher returns only the staged core bytes for the Lynx-core resource
type and rejects every other request. Bundle file URIs percent-encode raw Linux
path bytes, and the ICU C path is constructed directly from `OsStr` bytes rather
than a lossy UTF-8 conversion. The view owns one Rust `Launcher` and prefetches
its ordered application and icon snapshot. `Launcher.getApplications()` resolves
that snapshot as a real Promise or rejects with its native discovery error.
`launchApplication()` validates one UTF-8 ID, returns its Promise immediately,
and queues weak N-API async work. Execute calls the same launcher's direct Rust
interface on a worker; completion resolves with `undefined` or rejects with the
native detail on the JS thread. The queued allocation retains an `Arc<ViewState>`
but never accesses its window wake from execute or completion, and completion
deletes the async-work handle before releasing the allocation.

Shutdown backgrounds and releases the view and client while queues still accept
release work, then stops and drains renderer and UI work before releasing the
renderer and fetcher. The process-global UI generation remains in a draining
lease through renderer release, registry removal, fetcher release, and a bounded
`Mutex`/`Condvar` wait for the fetcher finalizer. Renderer/fetcher finalizer,
registry, and old-generation health results are sampled before the lease is
finished, so another host cannot activate or reset health early. If native
callbacks cannot be proven quiescent, the lease remains draining and the window
path forgets the runtime, window, and GLFW owners before delegating cleanup to
process exit. All Rust callbacks contain panics. The binary-private window
module supplies raw GLFW, X11, and OpenGL operations and translates GLFW input
into the reviewed Lynx pointer/key ABI.

### ReactLynx UI

`ui/src/platform.ts` is the UI's native boundary. It resolves the registered
`NativeModules.Launcher`, awaits its promises, and validates application data
before the view consumes it. `ui/src/App.tsx` owns loading, filtering, launch,
and error state. Rspeedy produces `ui/dist/main.lynx.bundle` for the host.

## Data flow

### Discover and render

1. The C++ host creates a Rust `LynxLauncher` while initializing the Lynx view.
2. ReactLynx calls `Launcher.getApplications()` through N-API.
3. C++ requests an owned Rust `LynxAppList` snapshot through the C ABI.
4. C++ reads each borrowed `LynxApplicationView`, resolves its optional icon,
   and constructs JavaScript objects.
5. C++ resolves a real N-API promise with the array and destroys all temporary
   Rust handles.
6. TypeScript validates IDs, names, icon URIs, and uniqueness before updating
   ReactLynx state.

The side-by-side Rust tracer performs the same platform discovery and icon
resolution directly before creating its view, stores an owned snapshot, and
resolves it from `Launcher.getApplications()` without crossing the platform C
ABI. This alternate path is not yet the default host.

### Launch

1. The UI calls `Launcher.launchApplication(id)` and awaits the returned promise.
2. The active host validates one JavaScript string as a length-delimited UTF-8 ID.
3. The C++ host crosses the platform C ABI; the Rust host queues N-API async work
   whose execute callback calls the retained Rust `Launcher` directly.
4. Rust finds the previously discovered application, builds a process command
   without a shell, and starts a named reaper thread. Launch returns after the
   reaper reports the `spawn` result; only the reaper waits for child exit.
5. The C++ callback or Rust async completion resolves with `undefined` or rejects
   with native detail; the UI clears pending state or displays the error.

The test-only `LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS` gate is disabled by
default, strictly accepts 0 through 5000 milliseconds, and delays only the Rust
worker before it calls `Launcher::launch`. It exists solely to make pending-work
shutdown ownership deterministic in E2E tests.

## Threads and ownership

- The process/platform thread creates GLFW and the host, pumps GLFW events, and
  runs the process-global Lynx UI task runner.
- Lynx posts UI work into a mutex-protected deadline queue. `glfwPostEmptyEvent`
  wakes the platform loop when new work arrives.
- Renderer work uses a separate synchronized queue. The first render thread to
  acquire the OpenGL context becomes its stable owner; callbacks reject access
  from another render thread. Context ownership is also tracked per thread.
- Shutdown cancels input, closes Rust launch-work registration, and waits with a
  fixed bound for every registered async launch to settle and successfully
  delete its N-API work handle. Only then does it background/release the view and
  client while renderer/UI queues still accept release work, drain those queues,
  and release renderer and fetcher. Launch-work timeout, async-work deletion
  failure, and fetcher/renderer quiescence failure preserve the live runtime and
  delegate native cleanup to process exit rather than freeing callback userdata.
- Focus loss stops accepting input, cancels active keys and pointers, backgrounds
  the Rust view, and closes the popup. The terminal gate ignores later synthetic
  GLFW releases and focus gains. Normal shutdown also closes the gate before
  cancellation and view background/release.
- Rust `LynxLauncher`, list, icon, and error values are opaque heap handles.
  Every successful allocation has one matching destroy call. Slices borrowed
  from a handle are valid only until that handle is destroyed.
- The application list returned over the ABI is a byte-owning snapshot, so it
  remains valid independently of the launcher's internal vector.
- Rust panics are caught at the ABI boundary. Status-returning functions expose
  a panic status instead of unwinding through C++.
- The Rust shutdown gate cancels input and launch work before closing desktop
  callbacks. It then backgrounds/releases the view and client while renderer
  and UI queues can service release work, stops and drains those queues, and
  releases renderer, fetcher, cursor, and GLFW owners in that order. Repeated
  bounded, real-first-frame, pending-launch, cursor, and clipboard runs exercise
  this same path; fatal callback/context failures preserve native owners for
  process-exit cleanup rather than masking the original error.

## Windowless API and XWayland

The Lynx windowless C API is used because this application is an embedder, not a
fork of Lynx Explorer. It exposes the necessary stable seams for task scheduling,
view lifecycle, graphics callbacks, input, clipboard, cursors, and resource
fetching while allowing the launcher to own its native window and event loop.
This also keeps application policy out of the Lynx submodule.

The current host deliberately configures vendored GLFW with
`GLFW_USE_WAYLAND=OFF`. The tested GLDirect path creates an X11 OpenGL 3.3
context, and GLFW supplies the window/input/clipboard integration expected by
the host. On a Wayland desktop this runs through XWayland and therefore requires
`DISPLAY`. Enabling native Wayland is more than a build switch: clipboard, text
input/IME, cursor, scale, and context behavior must be validated before the host
can claim that backend.

Graphical pixel gates are observation-only. The X11 driver first attempts root
readback; under niri/XWayland it validates compositor geometry and uses bounded
direct `niri`/`grim` capture. Capture polling never sends Expose or wake events,
and every capture subprocess shares the caller's assertion deadline.

The Rust shell links the same CMake `glfw` static target directly rather than a
system or crate-provided GLFW. Its `[host-rs] first shell GL frame presented`
marker proves only the static clear/swap path. Real Rust-host readiness requires
both `[host-rs] first screen layout completed` from the view client and
`[host-rs] first GL frame presented` from the GLDirect renderer. While the shell
event loop is active it uses an X11 background matching the verified shell clear,
so Expose handling preserves that color without reacquiring the GL context after
renderer startup.
If owner-thread context rollback or detach verification fails, the Rust tracer
marks the context stranded and exits without calling `glfwDestroyWindow` or
`glfwTerminate`; only this fatal path delegates native cleanup to process exit,
because the platform thread cannot safely repair another thread's GL state.

At startup the host combines GLFW's X11 content scale with the XSettings
`Gdk/WindowScalingFactor`. It creates a correspondingly larger physical window
while keeping the Lynx viewport in logical pixels, passes the scale as the Lynx
device pixel ratio, and converts pointer coordinates separately through the
window-to-framebuffer ratio. XSettings scale changes are not observed while the
process is running.

## Cross-platform extension points

- Add operating-system discovery and launch implementations behind the Rust
  platform `cfg` boundary while preserving the C ABI.
- Add native host backends that implement the same windowless renderer, task,
  input, and runtime-resource responsibilities. The ReactLynx bundle and N-API
  module contract can remain unchanged.
- Replace the GLFW/OpenGL adapter with another window and graphics backend
  without moving application discovery into C++ or UI code.
- Add native module methods by extending the C ABI first, implementing explicit
  ownership/status semantics, then adapting them to promises at the N-API edge.
- Package target-specific SDK/runtime resources through CMake without teaching
  the UI about filesystem layout.

## Dependency locking

The build treats every dependency boundary as an explicit lock:

- `third_party/lynx` is a Git submodule pinned by the parent repository's
  gitlink. Bootstrap checks out and verifies that exact object; it never uses
  `--remote` or follows `develop`.
- `patches/lynx` contains an explicit allowlist of Linux windowless integration
  patches. Bootstrap rejects unknown patch files, applies the trusted set only
  around the SDK build, and reverses it from an exit trap. The teardown,
  fontconfig fallback, and launcher size profile are integration boundaries,
  not a business fork; the submodule remains at its pinned, clean source state
  outside bootstrap.
- The pinned Lynx checkout supplies Habitat `0.3.149` and its DEPS revisions.
  Habitat runs with `.build-home/` as HOME so its multi-gigabyte cache cannot
  enter Git or depend on unrelated user cache state. Bootstrap serializes the
  entire submodule/cache/log critical section with the stable
  `.build-home/bootstrap.lock`, including reverse-patch restoration.
- A successful SDK build records a source key composed of the pinned gitlink SHA
  and a deterministic hash of the ordered patch set. Build and test reject a
  missing, incomplete, or stale keyed archive, verify its SHA256, and extract it
  to `.build-home/sdk/<pinned-sha>-<patch-set-sha256>/`. Only the ZIP's native
  `lib/`, `include/`, `data/icudtl.dat`, and `lynx_core.js` paths are passed to
  CMake. All extracted entries are compared byte-for-byte with the archive on
  reuse; unverified loose files under `out/Default` are never linked or copied.
- `rust-toolchain.toml` pins Rust `1.93.0`; the root `Cargo.lock` covers
  `platform`, `lynx-sys`, and `host-rs` and is used with locked workspace
  commands. Serial CMake Cargo rules build the platform static library before the
  Rust tracer, avoiding concurrent writers to the shared Cargo target directory.
- `.nvmrc` pins Node.js `22.14.0`. `ui/package.json` pins pnpm `10.34.5` and every
  direct package version; `ui/pnpm-lock.yaml` locks the full graph. Scripts ask
  Corepack for that exact `packageManager` value and use frozen installs.
- GLFW is not fetched independently by the launcher. CMake builds the copy
  materialized by the pinned Lynx DEPS graph, keeping host headers and SDK
  behavior aligned.
- The patched Linux SDK links the system `libfontconfig.so.1`. Source and SDK
  provenance remain locked, while the selected fallback typefaces intentionally
  follow the host's fontconfig configuration and installed font set.
- The Linux launcher SDK uses hidden-by-default visibility, keeps its static
  LLVM unwinder local, and excludes Inspector, runtime Lepus compilation, Wuffs,
  and Skottie. `LYNX_SDK_SIZE.md` records the measured impact and retained
  runtime capabilities.
- Runtime resources are refreshed by an always-run target whose operations are
  content-aware `copy_if_different` calls. Correctness does not depend on source
  mtimes, while equal files avoid writes and preserve incremental efficiency.
- Both executables use an exact `$ORIGIN` runtime search path, depend on one
  staged `liblynx.so`, and never dynamically link GLFW. Rust check and windowed
  startup resolve bundle, core JavaScript, ICU, and Lynx relative to the
  executable, verify the actually loaded Lynx path, and reject an injected
  `LD_LIBRARY_PATH` copy even when launched from an unrelated working directory.

Changing a lock or integration patch is an intentional source change. A Lynx
update must update the gitlink, revalidate or remove the integration patch,
rebuild the SDK, run the resource/ABI and teardown checks, and complete the
first-frame smoke before the new source key is accepted.
