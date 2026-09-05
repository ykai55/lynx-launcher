# Native Wayland Backend Contract

## Purpose

The native Wayland backend is an explicit launcher backend for Lynx windowless
GLDirect rendering and input through wlr-layer-shell and EGL. It is not the default
backend and does not replace unsupported X11/XWayland integration gates.

## Build Boundary

- Default CMake builds leave `LYNX_LAUNCHER_NATIVE_WAYLAND=OFF`, produce
  `host/build/lynx-launcher`, and neither discover nor directly link Wayland/EGL.
- The Wayland build sets `LYNX_LAUNCHER_NATIVE_WAYLAND=ON`, enables Cargo feature
  `native-wayland`, and produces `host/build-wayland/lynx-launcher-wayland`.
- `LYNX_LAUNCHER_BUILD_WAYLAND=1 ./scripts/build.sh` builds that runtime in addition
  to the unchanged default X11 runtime. `LYNX_LAUNCHER_WINDOW_BACKEND=wayland
  ./scripts/run.sh` selects it and injects `--window-backend wayland`; invalid
  selectors and missing artifacts fail without fallback.
- `--window-backend wayland` on a default binary fails clearly instead of changing
  backend implicitly.

## Protocol And Rendering

- Create one overlay layer surface with `output=NULL`, no anchors, requested logical
  size 1120x760, on-demand keyboard interactivity, and exclusive zone -1 so it is
  positioned relative to the complete output instead of avoiding other surfaces'
  positive exclusive zones. This is Fuzzel-style unanchored centering; overlay is
  the launcher's own layer choice. On-demand focus lets the compositor focus the
  launcher initially while allowing a normal window to take focus and deliver the
  `wl_keyboard.leave` that starts clean shutdown.
- Commit the surface without a buffer, wait for the initial configure, acknowledge
  its serial, and only then create the EGL window surface.
- Report first frame only after EGL buffer swap succeeds.
- Poll the Wayland display fd and runtime `eventfd` together. Interrupted wake writes
  retry, a saturated counter is already awake, and permanent write failures latch
  runtime health failure.
- Drop EGL surface/context/display before destroying Wayland protocol objects.

## Lifecycle And Scope

- A keyboard enter followed by leave during startup remains a focus-loss event after
  runtime event delivery activates. Removing the seat keyboard capability drops the
  keyboard proxy and applies the same focus-loss policy; restoring it creates a new
  proxy. The backend binds only `wl_seat` v3+, sends `wl_keyboard.release` only for
  v3+ proxies, and handles registry seat removal without retaining dead proxies.
- Pointer motion/buttons, frame-aggregated wheel events, xkbcommon keymap/modifiers,
  compositor-configured repeat, UTF-8 key text, cursor-shape-v1, focus, metrics, a
  real Lynx first frame, and EGL lifecycle are supported. Seat capabilities can be
  removed and recreated with version-correct proxy release.
- Clipboard selection transfer, IME composition, resize/reconfigure, dynamic output
  migration, and dynamic scale changes are unsupported. Initial
  `wl_surface.enter`/`leave` events record every overlapping output identity. After
  event delivery activation, entering a new identity, any leave, removal of a mapped
  output global, every layer configure, and every preferred scale change closes the
  backend with a health failure, including migration between equal-scale outputs.

## Verification

`./scripts/wayland-smoke.sh` runs feature-specific Clippy/tests and the opt-in CMake
build/CTest with `DISPLAY` unset. Its first real process uses a strictly validated,
E2E-only output name to present only a uniquely colored, bounded EGL shell frame on an
output large enough for the assertion; normal backend launches retain `output=NULL`. The
script parses the selected output's P6 PPM capture, Niri
logical geometry, scale, current mode, and transform, then verifies a connected
1120x760-logical-pixel bounding box centered on both axes. Its marker cannot satisfy Lynx
readiness. A second `--exit-after-first-frame` process requires both first screen layout
and successful GL present before auto-exit, exit zero, clean runtime shutdown, and no
forbidden lifecycle marker. Both invocations must remove their Niri layer after exit.
