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

`host/src/main.cc` owns the GLFW window, OpenGL context, Lynx view, windowless
renderer callbacks, task queues, input translation, resource loading, and the
`Launcher` N-API module. The host links the Rust static library and the pinned
Lynx shared library.

`host/src/support.cc` contains independently testable path, file, URI, and UTF-8
operations. CMake copies `liblynx.so`, ICU data, `lynx_core.js`, and the UI bundle
into `host/build`, allowing runtime paths to be resolved relative to the
executable rather than the caller's current directory. Core JS is copied both to
`resources/lynx_core.js` for the host's explicit path and to `$ORIGIN/lynx_core.js`
for the engine preloader's default lookup.

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

### Launch

1. The UI calls `Launcher.launchApplication(id)` and awaits the returned promise.
2. C++ converts the JavaScript string to a length-delimited UTF-8 `LynxSlice`.
3. Rust finds the previously discovered application and builds a process command
   without invoking a shell.
4. Rust starts a named reaper thread, reports whether `spawn` succeeded, and
   waits for the child off the UI thread.
5. C++ resolves or rejects the N-API promise; the UI clears its pending state or
   displays the error.

## Threads and ownership

- The process/platform thread creates GLFW and the host, pumps GLFW events, and
  runs the process-global Lynx UI task runner.
- Lynx posts UI work into a mutex-protected deadline queue. `glfwPostEmptyEvent`
  wakes the platform loop when new work arrives.
- Renderer work uses a separate synchronized queue. The first render thread to
  acquire the OpenGL context becomes its stable owner; callbacks reject access
  from another render thread. Context ownership is also tracked per thread.
- Shutdown cancels input, backgrounds/releases the view and client while their
  renderer/UI queues still accept release work, then stops and drains those
  queues before releasing renderer, fetcher, Rust, cursor, and GLFW resources.
- Rust `LynxLauncher`, list, icon, and error values are opaque heap handles.
  Every successful allocation has one matching destroy call. Slices borrowed
  from a handle are valid only until that handle is destroyed.
- The application list returned over the ABI is a byte-owning snapshot, so it
  remains valid independently of the launcher's internal vector.
- Rust panics are caught at the ABI boundary. Status-returning functions expose
  a panic status instead of unwinding through C++.

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
  around the SDK build, and reverses it from an exit trap. The teardown and
  fontconfig fixes are integration boundaries, not a business fork; the
  submodule remains at its pinned, clean source state outside bootstrap.
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
- `rust-toolchain.toml` pins Rust `1.93.0`; `platform/Cargo.lock` is source and is
  used with locked Cargo commands. The CMake Rust static-library rule depends on
  both files, so changing the pinned toolchain invalidates that build output.
- `.nvmrc` pins Node.js `22.14.0`. `ui/package.json` pins pnpm `10.34.5` and every
  direct package version; `ui/pnpm-lock.yaml` locks the full graph. Scripts ask
  Corepack for that exact `packageManager` value and use frozen installs.
- GLFW is not fetched independently by the launcher. CMake builds the copy
  materialized by the pinned Lynx DEPS graph, keeping host headers and SDK
  behavior aligned.
- The patched Linux SDK links the system `libfontconfig.so.1`. Source and SDK
  provenance remain locked, while the selected fallback typefaces intentionally
  follow the host's fontconfig configuration and installed font set.
- Runtime resources are refreshed by an always-run target whose operations are
  content-aware `copy_if_different` calls. Correctness does not depend on source
  mtimes, while equal files avoid writes and preserve incremental efficiency.

Changing a lock or integration patch is an intentional source change. A Lynx
update must update the gitlink, revalidate or remove the integration patch,
rebuild the SDK, run the resource/ABI and teardown checks, and complete the
first-frame smoke before the new source key is accepted.
