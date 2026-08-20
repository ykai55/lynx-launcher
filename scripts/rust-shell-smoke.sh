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

log_root="${repo_root}/.logs/ticket-03/rust-shell-smoke"
mkdir -p -- "${log_root}"
run_directory="$(mktemp -d "${log_root}/run.XXXXXX")"
fixture_root="${run_directory}/fixture"
data_home="${fixture_root}/data"
mkdir -p -- \
  "${data_home}/applications" \
  "${data_home}/icons/hicolor/64x64/apps" \
  "${fixture_root}/empty-data"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Alpha Resolved' \
  'Icon=lynx-rust-resolved' \
  'Exec=/bin/true' \
  >"${data_home}/applications/00-resolved.desktop"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Bravo Missing' \
  'Icon=lynx-rust-missing' \
  'Exec=/bin/true' \
  >"${data_home}/applications/10-missing.desktop"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Charlie Third' \
  'Exec=/bin/true' \
  >"${data_home}/applications/20-third.desktop"
printf '%s\n' \
  '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">' \
  '  <rect width="64" height="64" fill="#00ff00"/>' \
  '</svg>' \
  >"${data_home}/icons/hicolor/64x64/apps/lynx-rust-resolved.svg"
fixture_environment=(
  "XDG_DATA_HOME=${data_home}"
  "XDG_DATA_DIRS=${fixture_root}/empty-data"
  'XDG_CURRENT_DESKTOP=LYNX_RUST_E2E'
  'LC_ALL=C'
  'LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE=1'
)
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
  if grep -Eq -- 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error|\[host-rs\] fatal:' "${path}"; then
    dump_log "${path}"
    die "${label} emitted a forbidden lifecycle or Lynx error"
  fi
}

assert_snapshot_trace() {
  local path="$1" label="$2" count
  local -a expected=(
    '[host-rs] snapshot index=0 id=00-resolved.desktop name=Alpha Resolved icon=present'
    '[host-rs] snapshot index=1 id=10-missing.desktop name=Bravo Missing icon=missing'
    '[host-rs] snapshot index=2 id=20-third.desktop name=Charlie Third icon=missing'
  )
  count="$(grep -Fc -- '[host-rs] snapshot index=' "${path}" || true)"
  if [[ "${count}" != 3 ]]; then
    dump_log "${path}"
    die "${label} did not emit exactly three snapshot entries"
  fi
  for entry in "${expected[@]}"; do
    if ! grep -Fxq -- "${entry}" "${path}"; then
      dump_log "${path}"
      die "${label} emitted an unexpected application snapshot"
    fi
  done
}

assert_success_log() {
  local path="$1" label="$2"
  assert_clean_host_log "${path}" "${label}"
  for marker in \
    '[host-rs] first shell GL frame presented' \
    '[host-rs] first screen layout completed' \
    '[host-rs] first GL frame presented' \
    '[host-rs] Launcher.getApplications resolved 3 applications' \
    '[host-rs] runtime core initialized' \
    '[host-rs] runtime core shutdown complete'; do
    if ! grep -Fq -- "${marker}" "${path}"; then
      dump_log "${path}"
      die "${label} did not emit ${marker}"
    fi
  done
  assert_snapshot_trace "${path}" "${label}"
}

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  first_frame_log="${run_directory}/first-frame-${iteration}.log"
  if ! timeout --foreground 10s env "${fixture_environment[@]}" \
    "${rust_host_binary}" --exit-after-first-frame \
    >"${first_frame_log}" 2>&1; then
    dump_log "${first_frame_log}"
    die "Rust first-frame iteration ${iteration} failed"
  fi
  assert_success_log "${first_frame_log}" "Rust first-frame iteration ${iteration}"
  grep -Fq -- '[host-rs] auto-exit after first rendered frame' "${first_frame_log}" || {
    dump_log "${first_frame_log}"
    die "Rust first-frame iteration ${iteration} did not wait for layout and present"
  }

  host_log="${run_directory}/bounded-${iteration}.log"
  setsid env "${fixture_environment[@]}" \
    "${rust_host_binary}" --run-for 12 >"${host_log}" 2>&1 &
  host_pid=$!
  host_identity="$(capture_host_identity)" ||
    die "could not capture the Rust shell process identity"
  [[ "${host_identity}" == "${expected_host}:${host_pid}:"* ]] ||
    die "Rust shell did not create the expected process group"

  if ! wait_for_log '[host-rs] first shell GL frame presented' ||
    ! wait_for_log '[host-rs] first screen layout completed' ||
    ! wait_for_log '[host-rs] first GL frame presented' ||
    ! wait_for_log '[host-rs] Launcher.getApplications resolved 3 applications' ||
    ! wait_for_log '[host-rs] snapshot index=2 id=20-third.desktop name=Charlie Third icon=missing'; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not render the real launcher"
  fi

  "${click_driver}" --pid "${host_pid}" expect-popup
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 40 --y 265 --width 76 --height 76 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 512 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 550 --y 250 --width 100 --height 120 \
    --red 232 --green 189 --blue 108 --tolerance 8 \
    --minimum-matches 800 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" defocus

  if ! wait_for_host_exit; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not exit cleanly after focus loss"
  fi
  assert_success_log "${host_log}" "Rust bounded iteration ${iteration}"
  grep -Fq -- '[host-rs] window lost focus; exiting' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not handle focus loss"
  }
  printf 'Rust first-frame and bounded iteration %s/%s passed.\n' \
    "${iteration}" "${iterations}"
done

host_log="${run_directory}/native-rejection.log"
setsid env "${fixture_environment[@]}" \
  LYNX_LAUNCHER_E2E_FORCE_APPLICATIONS_ERROR=1 \
  "${rust_host_binary}" --run-for 12 >"${host_log}" 2>&1 &
host_pid=$!
host_identity="$(capture_host_identity)" ||
  die "could not capture the Rust rejection process identity"
if ! wait_for_log \
  '[host-rs] Launcher.getApplications rejected: injected native application snapshot failure' ||
  ! wait_for_log '[host-rs] first screen layout completed' ||
  ! wait_for_log '[host-rs] first GL frame presented'; then
  dump_log "${host_log}"
  die "Rust rejection process did not render the native error state"
fi
"${click_driver}" --pid "${host_pid}" expect-popup
"${click_driver}" --pid "${host_pid}" expect-pixel \
  --x 510 --y 300 --width 100 --height 120 \
  --red 255 --green 107 --blue 61 --tolerance 8 \
  --minimum-matches 800 --timeout-ms 3000
"${click_driver}" --pid "${host_pid}" defocus
if ! wait_for_host_exit; then
  dump_log "${host_log}"
  die "Rust rejection process did not exit cleanly"
fi
assert_clean_host_log "${host_log}" "Rust rejection process"
grep -Fq -- '[host-rs] runtime core shutdown complete' "${host_log}" || {
  dump_log "${host_log}"
  die "Rust rejection process did not shut down cleanly"
}
if grep -Fq -- '[host-rs] Launcher.getApplications resolved ' "${host_log}"; then
  dump_log "${host_log}"
  die "Rust rejection process unexpectedly resolved the application Promise"
fi
printf 'Rust native Promise rejection smoke passed.\n'

printf 'Rust shell smoke passed. Logs: %s\n' "${run_directory}"
