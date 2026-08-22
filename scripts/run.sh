#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

host_kind="${LYNX_LAUNCHER_HOST:-rust}"
if [[ "${1:-}" == --host ]]; then
  [[ "$#" -ge 2 ]] || die "--host requires rust or cpp"
  host_kind="$2"
  shift 2
elif [[ "${1:-}" == --host=* ]]; then
  host_kind="${1#--host=}"
  shift
fi
case "${host_kind}" in
  rust) selected_host_binary="${rust_host_binary}" ;;
  cpp) selected_host_binary="${cpp_host_binary}" ;;
  *) die "LYNX_LAUNCHER_HOST/--host must be rust or cpp" ;;
esac

required_runtime=(
  "${selected_host_binary}"
  "${host_build_dir}/liblynx.so"
  "${host_build_dir}/lynx_core.js"
  "${host_build_dir}/resources/icudtl.dat"
  "${host_build_dir}/resources/lynx_core.js"
  "${host_build_dir}/resources/main.lynx.bundle"
)
for path in "${required_runtime[@]}"; do
  [[ -e "${path}" ]] || die "built runtime is missing ${path}; run scripts/build.sh first"
done
[[ -x "${selected_host_binary}" ]] || die "host is not executable; run scripts/build.sh first"

exec "${selected_host_binary}" "$@"
