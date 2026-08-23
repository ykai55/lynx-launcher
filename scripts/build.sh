#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_command cmake
require_command cargo
ensure_sdk
prepare_ui_toolchain

printf 'Installing locked UI dependencies with %s\n' "${UI_PNPM[*]}"
(
  cd -- "${ui_dir}"
  "${UI_PNPM[@]}" install --frozen-lockfile
  "${UI_PNPM[@]}" run build
)

printf 'Building the default Rust host\n'
cmake -S "${host_dir}" -B "${host_build_dir}" \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo \
  -DBUILD_TESTING=ON \
  -DLYNX_SDK_DIR="${verified_sdk_dir}"
cmake --build "${host_build_dir}" --parallel

# Remove stale artifacts from the retired C++ host and older Rust staging names.
rm -f -- \
  "${host_build_dir}/lynx-launcher-cpp" \
  "${host_build_dir}/lynx-launcher-rs" \
  "${host_build_dir}/libhost_support.a"

printf 'Build complete: %s (default Rust host)\n' "${host_binary}"
