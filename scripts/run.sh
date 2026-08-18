#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

required_runtime=(
  "${host_binary}"
  "${host_build_dir}/liblynx.so"
  "${host_build_dir}/lynx_core.js"
  "${host_build_dir}/resources/icudtl.dat"
  "${host_build_dir}/resources/lynx_core.js"
  "${host_build_dir}/resources/main.lynx.bundle"
)
for path in "${required_runtime[@]}"; do
  [[ -e "${path}" ]] || die "built runtime is missing ${path}; run scripts/build.sh first"
done
[[ -x "${host_binary}" ]] || die "host is not executable; run scripts/build.sh first"

exec "${host_binary}" "$@"
