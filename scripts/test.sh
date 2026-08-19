#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_command cargo
require_command cmake
require_command ctest
ensure_sdk
prepare_ui_toolchain

printf 'Checking and testing the Rust workspace\n'
(
  cd -- "${repo_root}"
  cargo fmt --all -- --check
  LYNX_SDK_DIR="${verified_sdk_dir}" \
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  LYNX_SDK_DIR="${verified_sdk_dir}" \
  LD_LIBRARY_PATH="${verified_sdk_dir}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    cargo test --workspace --all-targets --all-features --locked
)

printf 'Testing, type-checking, and building the ReactLynx UI\n'
(
  cd -- "${ui_dir}"
  "${UI_PNPM[@]}" install --frozen-lockfile
  "${UI_PNPM[@]}" run test
  "${UI_PNPM[@]}" run typecheck
  "${UI_PNPM[@]}" run build
)

printf 'Building and testing the desktop host\n'
cmake -S "${host_dir}" -B "${host_build_dir}" \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo \
  -DBUILD_TESTING=ON \
  -DLYNX_SDK_DIR="${verified_sdk_dir}"
cmake --build "${host_build_dir}" --parallel
ctest --test-dir "${host_build_dir}" --build-config RelWithDebInfo --output-on-failure
"${host_binary}" --check-resources
"${rust_host_binary}" --check-resources

case "${LYNX_LAUNCHER_SMOKE:-0}" in
  0|false|no|'')
    printf 'Graphical first-frame smoke skipped (set LYNX_LAUNCHER_SMOKE=1 to enable).\n'
    ;;
  1|true|yes)
    [[ -n "${DISPLAY:-}" ]] || die "LYNX_LAUNCHER_SMOKE requires DISPLAY (X11 or XWayland)"
    require_command timeout
    smoke_timeout="${LYNX_LAUNCHER_SMOKE_TIMEOUT:-30s}"
    [[ "${smoke_timeout}" =~ ^[0-9]+([.][0-9]+)?[smh]?$ ]] ||
      die "invalid LYNX_LAUNCHER_SMOKE_TIMEOUT: ${smoke_timeout}"
    printf 'Running graphical first-frame smoke with timeout %s\n' "${smoke_timeout}"
    timeout --foreground -- "${smoke_timeout}" "${host_binary}" --exit-after-first-frame
    ;;
  *)
    die "LYNX_LAUNCHER_SMOKE must be 0/false/no or 1/true/yes"
    ;;
esac

printf 'All tests passed.\n'
