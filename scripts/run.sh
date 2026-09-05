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
  rust) ;;
  cpp|both)
    die "the C++ host has been retired; run.sh only supports the Rust host"
    ;;
  *) die "LYNX_LAUNCHER_HOST/--host must be rust" ;;
esac

window_backend="${LYNX_LAUNCHER_WINDOW_BACKEND-x11}"
case "${window_backend}" in
  x11)
    selected_build_dir="${host_build_dir}"
    selected_host_binary="${rust_host_binary}"
    backend_arguments=(--window-backend x11)
    build_hint="./scripts/build.sh"
    ;;
  wayland)
    selected_build_dir="${host_wayland_build_dir}"
    selected_host_binary="${wayland_host_binary}"
    backend_arguments=(--window-backend wayland)
    build_hint="LYNX_LAUNCHER_BUILD_WAYLAND=1 ./scripts/build.sh"
    ;;
  *) die "LYNX_LAUNCHER_WINDOW_BACKEND must be x11 or wayland" ;;
esac

for argument in "$@"; do
  case "${argument}" in
    --window-backend|--window-backend=*)
      die "select the window backend only with LYNX_LAUNCHER_WINDOW_BACKEND"
      ;;
  esac
done

required_runtime=(
  "${selected_host_binary}"
  "${selected_build_dir}/liblynx.so"
  "${selected_build_dir}/lynx_core.js"
  "${selected_build_dir}/resources/icudtl.dat"
  "${selected_build_dir}/resources/lynx_core.js"
  "${selected_build_dir}/resources/main.lynx.bundle"
)
for path in "${required_runtime[@]}"; do
  [[ -e "${path}" ]] || die "built ${window_backend} runtime is missing ${path}; run ${build_hint} first"
done
[[ -x "${selected_host_binary}" ]] || die "${window_backend} host is not executable; run ${build_hint} first"

exec "${selected_host_binary}" "${backend_arguments[@]}" "$@"
