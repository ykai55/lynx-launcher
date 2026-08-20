#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${DISPLAY:-}" ]] || die "the Rust shell smoke test requires DISPLAY"
require_command grep
require_command mktemp
require_command readlink
require_command setsid
require_command timeout

click_driver="${host_build_dir}/x11_click_test_driver"
[[ -x "${rust_host_binary}" ]] ||
  die "Rust host is not built; run scripts/build.sh first"
[[ -x "${click_driver}" ]] ||
  die "X11 test driver is not built; run scripts/build.sh first"

iterations="${LYNX_LAUNCHER_RUST_SHELL_ITERATIONS:-1}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_RUST_SHELL_ITERATIONS must be a positive integer"

log_root="${repo_root}/.logs/rust-shell-smoke"
mkdir -p -- "${log_root}"
run_directory="$(mktemp -d "${log_root}/run.XXXXXX")"
host_pid=""
host_identity=""
host_log=""
expected_host="$(readlink -f -- "${rust_host_binary}")"

process_identity() {
  local pid="$1" stat remainder
  local -a fields
  [[ -r "/proc/${pid}/stat" ]] || return 1
  stat="$(<"/proc/${pid}/stat")"
  remainder="${stat#*) }"
  read -r -a fields <<<"${remainder}"
  [[ "${#fields[@]}" -gt 19 ]] || return 1
  printf '%s:%s:%s\n' \
    "$(readlink -f -- "/proc/${pid}/exe")" "${fields[2]}" "${fields[19]}"
}

capture_host_identity() {
  local identity deadline=$((SECONDS + 2))
  while ((SECONDS < deadline)); do
    identity="$(process_identity "${host_pid}" 2>/dev/null || true)"
    if [[ "${identity}" == "${expected_host}:${host_pid}:"* ]]; then
      printf '%s\n' "${identity}"
      return 0
    fi
    kill -0 "${host_pid}" 2>/dev/null || return 1
    sleep 0.02
  done
  return 1
}

stop_host() {
  local current_identity
  [[ -n "${host_pid}" ]] || return 0
  current_identity="$(process_identity "${host_pid}" 2>/dev/null || true)"
  if [[ -n "${current_identity}" && "${current_identity}" == "${host_identity}" &&
    "${host_identity}" == "${expected_host}:${host_pid}:"* ]]; then
    kill -TERM -- "-${host_pid}" 2>/dev/null || true
  elif kill -0 "${host_pid}" 2>/dev/null; then
    printf 'warning: refusing to signal PID %s because its identity changed\n' \
      "${host_pid}" >&2
  fi
  wait "${host_pid}" 2>/dev/null || true
  host_pid=""
  host_identity=""
}

trap stop_host EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

dump_log() {
  local path="$1"
  [[ -f "${path}" ]] || return 0
  printf '%s\n' "----- ${path} -----" >&2
  while IFS= read -r line; do
    printf '%s\n' "${line}" >&2
  done <"${path}"
  printf '%s\n' '----- end log -----' >&2
}

wait_for_log() {
  local expected="$1" deadline=$((SECONDS + 8))
  while ((SECONDS < deadline)); do
    if [[ -f "${host_log}" ]] && grep -Fq -- "${expected}" "${host_log}"; then
      return 0
    fi
    [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]] ||
      return 1
    sleep 0.05
  done
  return 1
}

wait_for_host_exit() {
  local deadline=$((SECONDS + 5)) status=0
  while ((SECONDS < deadline)); do
    if [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" != "${host_identity}" ]]; then
      set +e
      wait "${host_pid}"
      status=$?
      set -e
      host_pid=""
      host_identity=""
      return "${status}"
    fi
    sleep 0.05
  done
  return 1
}

assert_clean_host_log() {
  local path="$1" label="$2"
  if grep -Eq -- 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error' "${path}"; then
    dump_log "${path}"
    die "${label} emitted a forbidden lifecycle or Lynx error"
  fi
}

first_frame_log="${run_directory}/first-frame.log"
if ! timeout --foreground 10s "${rust_host_binary}" --exit-after-first-frame \
  >"${first_frame_log}" 2>&1; then
  dump_log "${first_frame_log}"
  die "Rust shell first-frame process failed"
fi
assert_clean_host_log "${first_frame_log}" "Rust shell first-frame process"
grep -Fq -- '[host-rs] first shell GL frame presented' "${first_frame_log}" || {
  dump_log "${first_frame_log}"
  die "Rust shell first-frame marker was not emitted"
}
grep -Fq -- '[host-rs] auto-exit after first shell frame' "${first_frame_log}" || {
  dump_log "${first_frame_log}"
  die "Rust shell did not auto-exit after its first frame"
}
grep -Fq -- '[host-rs] runtime core initialized' "${first_frame_log}" || {
  dump_log "${first_frame_log}"
  die "Rust shell first-frame process did not initialize the runtime core"
}
grep -Fq -- '[host-rs] runtime core shutdown complete' "${first_frame_log}" || {
  dump_log "${first_frame_log}"
  die "Rust shell first-frame process did not release the runtime core"
}

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  host_log="${run_directory}/bounded-${iteration}.log"
  setsid "${rust_host_binary}" --run-for 12 >"${host_log}" 2>&1 &
  host_pid=$!
  host_identity="$(capture_host_identity)" ||
    die "could not capture the Rust shell process identity"
  [[ "${host_identity}" == "${expected_host}:${host_pid}:"* ]] ||
    die "Rust shell did not create the expected process group"

  if ! wait_for_log '[host-rs] first shell GL frame presented'; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not present its shell frame"
  fi

  "${click_driver}" --pid "${host_pid}" expect-popup
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 100 --y 100 --width 64 --height 64 \
    --red 9 --green 10 --blue 11 --tolerance 4 \
    --minimum-matches 4000 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" defocus

  if ! wait_for_host_exit; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not exit cleanly after focus loss"
  fi
  assert_clean_host_log "${host_log}" "Rust shell iteration ${iteration}"
  grep -Fq -- '[host-rs] window lost focus; exiting' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not handle focus loss"
  }
  grep -Fq -- '[host-rs] runtime core initialized' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not initialize the runtime core"
  }
  grep -Fq -- '[host-rs] runtime core shutdown complete' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not release the runtime core"
  }
  printf 'Rust shell smoke iteration %s/%s passed.\n' "${iteration}" "${iterations}"
done

printf 'Rust shell smoke passed. Logs: %s\n' "${run_directory}"
