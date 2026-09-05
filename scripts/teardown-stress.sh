#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_command grep
require_command mktemp
require_command timeout

window_backend="${LYNX_LAUNCHER_WINDOW_BACKEND-x11}"
case "${window_backend}" in
  x11)
    [[ -n "${DISPLAY:-}" ]] || die "the teardown stress test requires DISPLAY"
    selected_host_binary="${rust_host_binary}"
    backend_arguments=()
    log_backend="rust"
    ;;
  wayland)
    [[ -n "${WAYLAND_DISPLAY:-}" ]] ||
      die "the native Wayland teardown stress test requires WAYLAND_DISPLAY"
    selected_host_binary="${wayland_host_binary}"
    backend_arguments=(--window-backend wayland)
    log_backend="wayland"
    ;;
  *) die "LYNX_LAUNCHER_WINDOW_BACKEND must be x11 or wayland" ;;
esac

host_kind="${LYNX_LAUNCHER_TEARDOWN_HOST:-rust}"
case "${host_kind}" in
  rust) ;;
  cpp|both)
    die "the C++ host has been retired; LYNX_LAUNCHER_TEARDOWN_HOST only supports rust"
    ;;
  *) die "LYNX_LAUNCHER_TEARDOWN_HOST must be rust" ;;
esac

first_frame_marker='[host-rs] first GL frame presented'

if [[ ! -x "${selected_host_binary}" ]]; then
  if [[ "${window_backend}" == wayland ]]; then
    die "native Wayland host is not built; run LYNX_LAUNCHER_BUILD_WAYLAND=1 ./scripts/build.sh first"
  fi
  die "Rust host is not built; run scripts/build.sh first"
fi

iterations="${LYNX_LAUNCHER_TEARDOWN_ITERATIONS:-10}"
timeout_value="${LYNX_LAUNCHER_TEARDOWN_TIMEOUT:-30s}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_TEARDOWN_ITERATIONS must be a positive integer"
[[ "${timeout_value}" =~ ^[0-9]+([.][0-9]+)?[smh]?$ ]] ||
  die "invalid LYNX_LAUNCHER_TEARDOWN_TIMEOUT: ${timeout_value}"

log_root="${repo_root}/.logs/ticket-07/teardown-stress/${log_backend}"
mkdir -p -- "${log_root}"
log_dir="$(mktemp -d "${log_root}/run.XXXXXX")"
summary_file="${log_dir}/summary.txt"
stress_status="failed"

layout_parent_count=0
target_missing_count=0

write_summary() {
  if [[ "${window_backend}" == x11 ]]; then
    printf 'status=%s host=%s bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
      "${stress_status}" "${host_kind}" "${iterations}" "${iterations}" \
      "${layout_parent_count}" "${target_missing_count}" >"${summary_file}"
  else
    printf 'status=%s host=%s backend=%s bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
      "${stress_status}" "${host_kind}" "${window_backend}" "${iterations}" "${iterations}" \
      "${layout_parent_count}" "${target_missing_count}" >"${summary_file}"
  fi
  printf 'Logs: %s\n' "${log_dir}"
}
trap write_summary EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run_case() {
  local kind="$1" iteration="$2" log="$3"
  shift 3

  local -a command
  if [[ "${window_backend}" == wayland ]]; then
    command=(
      env
      -u DISPLAY
      -u LYNX_LAUNCHER_BUNDLE
      -u LYNX_LAUNCHER_E2E_IGNORE_FOCUS_LOSS
      -u LYNX_LAUNCHER_E2E_STARTUP_FOCUS_WAIT
      -u LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_OUTPUT
      -u LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_PROBE
      -u LYNX_LAUNCHER_HOST
      -u LYNX_LAUNCHER_ICU
      -u LYNX_LAUNCHER_LYNX_CORE
      -u LYNX_LAUNCHER_RESOURCE_HOST
      -u LYNX_LAUNCHER_WINDOW_BACKEND
      timeout --foreground -- "${timeout_value}"
      "${selected_host_binary}" "${backend_arguments[@]}"
    )
  else
    command=(timeout --foreground -- "${timeout_value}" "${selected_host_binary}")
  fi

  if ! "${command[@]}" "$@" >"${log}" 2>&1; then
    printf 'teardown stress %s iteration %s failed\n' "${kind}" "${iteration}" >&2
    return 1
  fi
  grep -Fq -- "${first_frame_marker}" "${log}" ||
    die "teardown stress ${kind} iteration ${iteration} did not present a frame"
  if [[ "${window_backend}" == wayland ]]; then
    grep -Fq -- '[host-rs] runtime core shutdown complete' "${log}" ||
      die "native Wayland teardown stress ${kind} iteration ${iteration} did not shut down cleanly"
    if [[ "${kind}" == first-frame ]]; then
      grep -Fq -- '[host-rs] first screen layout completed' "${log}" ||
        die "native Wayland teardown stress ${kind} iteration ${iteration} missed first layout"
      grep -Fq -- '[host-rs] auto-exit after first rendered frame' "${log}" ||
        die "native Wayland teardown stress ${kind} iteration ${iteration} missed first-frame auto-exit"
    fi
  fi
  if grep -Eq -- 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error|\[host(-rs)?\] fatal:' "${log}"; then
    printf 'teardown stress %s iteration %s emitted a forbidden teardown/resource error\n' \
      "${kind}" "${iteration}" >&2
    return 1
  fi

  layout_parent_count=$((layout_parent_count + $(grep -Fc -- 'DestroyLayoutNodeBeforeRemoveFromParent' "${log}" || true)))
  target_missing_count=$((target_missing_count + $(grep -Ec -- 'target view: .* not found' "${log}" || true)))
  printf 'teardown stress %s iteration %s/%s passed\n' \
    "${kind}" "${iteration}" "${iterations}"
}

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  printf -v suffix '%02d' "${iteration}"
  run_case bounded "${iteration}" "${log_dir}/bounded-${suffix}.log" --run-for 1
done

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  printf -v suffix '%02d' "${iteration}"
  run_case first-frame "${iteration}" "${log_dir}/first-frame-${suffix}.log" \
    --exit-after-first-frame
done

if [[ "${window_backend}" == x11 ]]; then
  printf 'Running Rust pending-launch immediate-defocus teardown (%s iterations)\n' "${iterations}"
  LYNX_LAUNCHER_E2E_HOST=rust \
    LYNX_LAUNCHER_E2E_SCENARIO=immediate-defocus \
    LYNX_LAUNCHER_E2E_IMMEDIATE_DEFOCUS_ITERATIONS="${iterations}" \
    "${scripts_dir}/e2e-launch.sh"

  printf 'Running Rust cursor/clipboard desktop teardown (%s iterations)\n' "${iterations}"
  LYNX_LAUNCHER_RUST_SHELL_ITERATIONS="${iterations}" \
    "${scripts_dir}/rust-shell-smoke.sh"
else
  printf 'Skipping X11 immediate-defocus E2E and cursor/clipboard shell in native Wayland mode.\n'
fi

if [[ "${window_backend}" == x11 ]]; then
  printf '%s teardown stress passed: bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
    "${host_kind}" "${iterations}" "${iterations}" "${layout_parent_count}" "${target_missing_count}"
else
  printf '%s/%s teardown stress passed: bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
    "${host_kind}" "${window_backend}" "${iterations}" "${iterations}" "${layout_parent_count}" "${target_missing_count}"
fi
stress_status="passed"
