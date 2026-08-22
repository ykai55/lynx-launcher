#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${DISPLAY:-}" ]] || die "the teardown stress test requires DISPLAY"
require_command grep
require_command mktemp
require_command timeout

host_kind="${LYNX_LAUNCHER_TEARDOWN_HOST:-rust}"
if [[ "${host_kind}" == both ]]; then
  LYNX_LAUNCHER_TEARDOWN_HOST=cpp "$0"
  LYNX_LAUNCHER_TEARDOWN_HOST=rust "$0"
  exit 0
fi

case "${host_kind}" in
  cpp)
    selected_host_binary="${cpp_host_binary}"
    first_frame_marker='[host] first GL frame presented'
    ;;
  rust)
    selected_host_binary="${rust_host_binary}"
    first_frame_marker='[host-rs] first GL frame presented'
    ;;
  *) die "LYNX_LAUNCHER_TEARDOWN_HOST must be cpp, rust, or both" ;;
esac

[[ -x "${selected_host_binary}" ]] ||
  die "${host_kind} host is not built; run scripts/build.sh first"

iterations="${LYNX_LAUNCHER_TEARDOWN_ITERATIONS:-10}"
timeout_value="${LYNX_LAUNCHER_TEARDOWN_TIMEOUT:-30s}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_TEARDOWN_ITERATIONS must be a positive integer"
[[ "${timeout_value}" =~ ^[0-9]+([.][0-9]+)?[smh]?$ ]] ||
  die "invalid LYNX_LAUNCHER_TEARDOWN_TIMEOUT: ${timeout_value}"

log_root="${repo_root}/.logs/ticket-07/teardown-stress/${host_kind}"
mkdir -p -- "${log_root}"
log_dir="$(mktemp -d "${log_root}/run.XXXXXX")"
summary_file="${log_dir}/summary.txt"
stress_status="failed"

layout_parent_count=0
target_missing_count=0

write_summary() {
  printf 'status=%s host=%s bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
    "${stress_status}" "${host_kind}" "${iterations}" "${iterations}" \
    "${layout_parent_count}" "${target_missing_count}" >"${summary_file}"
  printf 'Logs: %s\n' "${log_dir}"
}
trap write_summary EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run_case() {
  local kind="$1" iteration="$2" log="$3"
  shift 3

  if ! timeout --foreground -- "${timeout_value}" "${selected_host_binary}" "$@" \
    >"${log}" 2>&1; then
    printf 'teardown stress %s iteration %s failed\n' "${kind}" "${iteration}" >&2
    return 1
  fi
  grep -Fq -- "${first_frame_marker}" "${log}" ||
    die "teardown stress ${kind} iteration ${iteration} did not present a frame"
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

if [[ "${host_kind}" == rust ]]; then
  printf 'Running Rust pending-launch immediate-defocus teardown (%s iterations)\n' "${iterations}"
  LYNX_LAUNCHER_E2E_HOST=rust \
    LYNX_LAUNCHER_E2E_SCENARIO=immediate-defocus \
    LYNX_LAUNCHER_E2E_IMMEDIATE_DEFOCUS_ITERATIONS="${iterations}" \
    "${scripts_dir}/e2e-launch.sh"

  printf 'Running Rust cursor/clipboard desktop teardown (%s iterations)\n' "${iterations}"
  LYNX_LAUNCHER_RUST_SHELL_ITERATIONS="${iterations}" \
    "${scripts_dir}/rust-shell-smoke.sh"
fi

printf '%s teardown stress passed: bounded=%s first-frame=%s layout_parent=%s target_missing=%s\n' \
  "${host_kind}" "${iterations}" "${iterations}" "${layout_parent_count}" "${target_missing_count}"
stress_status="passed"
