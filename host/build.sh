#!/usr/bin/env bash
set -euo pipefail

host_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
launcher_root="$(cd -- "${host_dir}/.." && pwd)"
# shellcheck source=scripts/_common.sh
source "${launcher_root}/scripts/_common.sh"

build_dir="${LYNX_LAUNCHER_HOST_BUILD_DIR:-${host_dir}/build}"

require_command cmake
require_command cargo
require_command ctest
ensure_sdk

cmake -S "${host_dir}" -B "${build_dir}" \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo \
  -DBUILD_TESTING=ON \
  "$@" \
  -DLYNX_SDK_DIR="${verified_sdk_dir}"
cmake --build "${build_dir}" --parallel
ctest --test-dir "${build_dir}" --build-config RelWithDebInfo --output-on-failure
