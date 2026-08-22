#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${DISPLAY:-}" ]] || die "the launch E2E test requires DISPLAY"
require_command mktemp
require_command grep
require_command cut
require_command readlink
require_command setsid
require_visible_capture_commands

click_driver="${host_build_dir}/x11_click_test_driver"
host_kind="${LYNX_LAUNCHER_E2E_HOST:-cpp}"
case "${host_kind}" in
  cpp)
    selected_host_binary="${host_binary}"
    host_log_prefix='[host]'
    ;;
  rust)
    selected_host_binary="${rust_host_binary}"
    host_log_prefix='[host-rs]'
    ;;
  *)
    die "LYNX_LAUNCHER_E2E_HOST must be cpp or rust"
    ;;
esac
[[ -x "${selected_host_binary}" ]] ||
  die "${host_kind} host is not built; run scripts/build.sh first"
[[ -x "${click_driver}" ]] || die "X11 test driver is not built; run scripts/build.sh first"

iterations="${LYNX_LAUNCHER_E2E_ITERATIONS:-1}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_E2E_ITERATIONS must be a positive integer"
scenario="${LYNX_LAUNCHER_E2E_SCENARIO:-$([[ "${host_kind}" == rust ]] && printf all || printf success)}"
case "${scenario}" in
  success | spawn-failure | unknown | immediate-defocus | all) ;;
  *) die "LYNX_LAUNCHER_E2E_SCENARIO must be success, spawn-failure, unknown, immediate-defocus, or all" ;;
esac
if [[ "${host_kind}" != rust && ("${scenario}" == unknown || "${scenario}" == immediate-defocus || "${scenario}" == all) ]]; then
  die "${scenario} launch E2E is only supported by the Rust host"
fi

host_pid=""
host_identity=""
temporary=""
host_log=""
expected_host="$(readlink -f -- "${selected_host_binary}")"

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

cleanup() {
  stop_host
  if [[ -n "${temporary}" && "${temporary}" == "${TMPDIR:-/tmp}/lynx-launcher-e2e."* ]]; then
    rm -rf -- "${temporary}"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

dump_log() {
  if [[ -f "${host_log}" ]]; then
    printf '%s\n' '----- host log -----' >&2
    while IFS= read -r line; do
      printf '%s\n' "${line}" >&2
    done <"${host_log}"
    printf '%s\n' '----- end host log -----' >&2
  fi
}

assert_clean_host_log() {
  local label="$1"
  if grep -Eq -- 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error|\[host-rs\] fatal:' "${host_log}"; then
    dump_log
    die "${label}: host log contains a forbidden lifecycle or Lynx error"
  fi
}

assert_log_order() {
  local label="$1"
  shift
  local marker line previous=0
  for marker in "$@"; do
    line="$(grep -Fnm1 -- "${marker}" "${host_log}" | cut -d: -f1 || true)"
    if [[ ! "${line}" =~ ^[1-9][0-9]*$ ]] || ((line <= previous)); then
      dump_log
      die "${label}: log marker is missing or out of order: ${marker}"
    fi
    previous="${line}"
  done
}

wait_for_log() {
  local expected="$1"
  local deadline=$((SECONDS + 8))
  while ((SECONDS < deadline)); do
    if [[ -f "${host_log}" ]] && grep -Fq -- "${expected}" "${host_log}"; then
      return 0
    fi
    if [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" != "${host_identity}" ]]; then
      return 1
    fi
    sleep 0.05
  done
  return 1
}

drive_target_tap() {
  "${click_driver}" --pid "${host_pid}" expect-popup
  "${click_driver}" --pid "${host_pid}" expect-regions-differ \
    --x 125 --y 310 --other-x 140 --other-y 310 --width 15 --height 18 \
    --red 241 --green 234 --blue 217 --tolerance 40 \
    --minimum-differences 10 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" click --x 220 --y 160
  sleep 0.2
  # A synthetic X11 click can be consumed while activating the popup. Repeat
  # the same click once the window is active before testing Lynx input.
  "${click_driver}" --pid "${host_pid}" click --x 220 --y 160
  sleep 0.1
  "${click_driver}" --pid "${host_pid}" type --text cobalt
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 40 --y 265 --width 76 --height 76 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 512 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" expect-no-pixel \
    --x 360 --y 270 --width 700 --height 100 \
    --red 241 --green 234 --blue 217 --tolerance 20 \
    --maximum-matches 20 --timeout-ms 3000
  "${click_driver}" --pid "${host_pid}" click --x 200 --y 305
}

wait_for_focus_loss_exit() {
  local label="$1" exit_deadline
  exit_deadline=$((SECONDS + 3))
  while [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]] &&
    ((SECONDS < exit_deadline)); do
    sleep 0.02
  done
  if [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]]; then
    printf '%s: host did not exit after focus loss\n' "${label}" >&2
    dump_log
    return 1
  fi
  if ! wait "${host_pid}"; then
    printf '%s: host failed while exiting after focus loss\n' "${label}" >&2
    dump_log
    host_pid=""
    host_identity=""
    return 1
  fi
  host_pid=""
  host_identity=""
  if ! grep -Fq -- "${host_log_prefix} window lost focus; exiting" "${host_log}"; then
    printf '%s: host did not report focus-loss exit\n' "${label}" >&2
    dump_log
    return 1
  fi
}

exit_after_focus_loss() {
  local label="$1"
  "${click_driver}" --pid "${host_pid}" defocus
  wait_for_focus_loss_exit "${label}"
}

run_case() {
  local iteration="$1" case_name="$2" case_iterations="$3"
  temporary="$(mktemp -d "${TMPDIR:-/tmp}/lynx-launcher-e2e.XXXXXX")"
  host_log="${temporary}/host.log"
  local data_home="${temporary}/data"
  local decoy_marker="${temporary}/decoy-launched.marker"
  local target_started="${temporary}/target-started.marker"
  local target_exited="${temporary}/target-exited.marker"
  local decoy_helper="${temporary}/launch-decoy.sh"
  local target_helper="${temporary}/launch-target.sh"
  local -a host_environment=(
    "XDG_DATA_HOME=${data_home}"
    "XDG_DATA_DIRS=${temporary}/empty-data"
    "XDG_CURRENT_DESKTOP=LYNX_E2E"
    "LC_ALL=C"
  )
  mkdir -p -- \
    "${data_home}/applications" \
    "${data_home}/icons/hicolor/64x64/apps" \
    "${temporary}/empty-data"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ": >\"${decoy_marker}\"" >"${decoy_helper}"
  if [[ "${case_name}" == immediate-defocus ]]; then
    printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
      ": >\"${target_started}\"" \
      ": >\"${target_exited}\"" >"${target_helper}"
  else
    printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
      ": >\"${target_started}\"" \
      'sleep 5' \
      ": >\"${target_exited}\"" >"${target_helper}"
  fi
  chmod +x -- "${decoy_helper}" "${target_helper}"
  printf '%s\n' \
    '[Desktop Entry]' \
    'Type=Application' \
    'Name=中文应用' \
    'Exec=/bin/true' \
    >"${data_home}/applications/00-chinese.desktop"
  printf '%s\n' \
    '[Desktop Entry]' \
    'Type=Application' \
    'Name=Archive Utility Fixture' \
    "Exec=${decoy_helper}" \
    >"${data_home}/applications/10-decoy.desktop"
  printf '%s\n' \
    '[Desktop Entry]' \
    'Type=Application' \
    'Name=Cobalt Target Fixture' \
    'Icon=lynx-e2e-target' \
    "Exec=${target_helper}" \
    >"${data_home}/applications/20-target.desktop"
  printf '%s\n' \
    '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">' \
    '  <rect width="64" height="64" fill="#00ff00"/>' \
    '</svg>' \
    >"${data_home}/icons/hicolor/64x64/apps/lynx-e2e-target.svg"

  if [[ "${case_name}" == unknown ]]; then
    host_environment+=("LYNX_LAUNCHER_E2E_UNKNOWN_APPLICATION_ID=20-target.desktop")
  elif [[ "${case_name}" == spawn-failure ]]; then
    rm -f -- "${target_helper}"
  elif [[ "${case_name}" == immediate-defocus ]]; then
    host_environment+=("LYNX_LAUNCHER_E2E_LAUNCH_WORK_DELAY_MS=750")
  fi

  setsid env "${host_environment[@]}" \
    "${selected_host_binary}" --run-for 12 >"${host_log}" 2>&1 &
  host_pid=$!
  host_identity="$(capture_host_identity)" ||
    die "could not capture the launched host process identity"
  [[ "${host_identity}" == "${expected_host}:${host_pid}:"* ]] ||
    die "launched host did not create the expected process group"

  if ! wait_for_log "${host_log_prefix} Launcher.getApplications resolved 3 applications" ||
    ! wait_for_log "${host_log_prefix} first GL frame presented"; then
    printf 'launch E2E iteration %s: host did not become ready\n' "${iteration}" >&2
    dump_log
    return 1
  fi
  sleep 0.25

  drive_target_tap

  if [[ "${case_name}" == immediate-defocus ]]; then
    local worker_marker='[host-rs] Launcher.launchApplication worker entered pending delay_ms=750'
    local waiting_marker='[host-rs] shutdown waiting for 1 Launcher async work'
    local resolved_marker='[host-rs] Launcher.launchApplication Promise resolved'
    local quiesced_marker='[host-rs] Launcher async work quiesced during shutdown'
    if ! wait_for_log "${worker_marker}"; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: worker did not enter pending delay"
    fi
    if grep -Fq -- 'Launcher.launchApplication Promise resolved' "${host_log}" ||
      grep -Fq -- 'Launcher.launchApplication Promise rejected' "${host_log}"; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: Promise settled before defocus"
    fi
    "${click_driver}" --pid "${host_pid}" defocus
    sleep 0.2
    if [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" != "${host_identity}" ]]; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: host released the live runtime during worker delay"
    fi
    if ! wait_for_log "${waiting_marker}"; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: shutdown did not wait for pending work"
    fi
    if ! wait_for_log "${resolved_marker}" || ! wait_for_log "${quiesced_marker}"; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: launch work did not complete during shutdown"
    fi
    if ! wait_for_focus_loss_exit "launch E2E immediate-defocus iteration ${iteration}"; then
      dump_log
      die "launch E2E immediate-defocus iteration ${iteration}: host did not exit cleanly"
    fi
    assert_log_order "launch E2E immediate-defocus iteration ${iteration}" \
      "${worker_marker}" "${waiting_marker}" "${resolved_marker}" \
      "${quiesced_marker}" '[host-rs] runtime core shutdown complete'
    printf '%s\n' \
      'verified worker pending -> shutdown waiting -> Promise resolved -> work quiesced -> clean exit'
  elif [[ "${case_name}" == success ]]; then
    local deadline=$((SECONDS + 3))
    while [[ ! -e "${target_started}" ]] && ((SECONDS < deadline)); do
      sleep 0.02
    done
    [[ -e "${target_started}" ]] || {
      dump_log
      die "launch E2E ${case_name} iteration ${iteration}: target did not start"
    }
    if [[ "${host_kind}" == rust ]] &&
      ! wait_for_log "${host_log_prefix} Launcher.launchApplication Promise resolved"; then
      dump_log
      die "launch E2E ${case_name} iteration ${iteration}: Promise did not resolve"
    fi
    [[ ! -e "${target_exited}" ]] || {
      dump_log
      die "launch E2E ${case_name} iteration ${iteration}: Promise waited for child exit"
    }
    deadline=$((SECONDS + 8))
    while [[ ! -e "${target_exited}" ]] && ((SECONDS < deadline)); do
      sleep 0.02
    done
    [[ -e "${target_exited}" ]] || die "launch E2E ${case_name} iteration ${iteration}: target did not exit"
    exit_after_focus_loss "launch E2E success iteration ${iteration}"
  else
    local expected_rejection
    if [[ "${case_name}" == spawn-failure ]]; then
      expected_rejection='No such file or directory (os error 2)'
    else
      expected_rejection='application not found: lynx-launcher-e2e-unknown.desktop'
    fi
    if [[ "${host_kind}" == rust ]] &&
      ! wait_for_log "${host_log_prefix} Launcher.launchApplication Promise rejected: ${expected_rejection}"; then
      dump_log
      die "launch E2E ${case_name} iteration ${iteration}: native rejection detail was missing"
    fi
    "${click_driver}" --pid "${host_pid}" expect-pixel \
      --x 45 --y 205 --width 1030 --height 75 \
      --red 255 --green 107 --blue 61 --tolerance 12 \
      --minimum-matches 10 --timeout-ms 3000
    [[ ! -e "${target_started}" && ! -e "${target_exited}" && ! -e "${decoy_marker}" ]] || {
      dump_log
      die "launch E2E ${case_name} iteration ${iteration}: rejected launch ran a fixture"
    }
    exit_after_focus_loss "launch E2E ${case_name} iteration ${iteration}"
  fi
  assert_clean_host_log "launch E2E ${case_name} iteration ${iteration}"
  rm -rf -- "${temporary}"
  temporary=""
  printf 'launch E2E %s %s iteration %s/%s passed\n' \
    "${host_kind}" "${case_name}" "${iteration}" "${case_iterations}"
}

cases=("${scenario}")
if [[ "${scenario}" == all ]]; then
  cases=(success spawn-failure unknown immediate-defocus)
fi
for case_name in "${cases[@]}"; do
  case "${case_name}" in
    success) case_iterations="${LYNX_LAUNCHER_E2E_SUCCESS_ITERATIONS:-${iterations}}" ;;
    spawn-failure) case_iterations="${LYNX_LAUNCHER_E2E_SPAWN_FAILURE_ITERATIONS:-${iterations}}" ;;
    unknown) case_iterations="${LYNX_LAUNCHER_E2E_UNKNOWN_ITERATIONS:-${iterations}}" ;;
    immediate-defocus) case_iterations="${LYNX_LAUNCHER_E2E_IMMEDIATE_DEFOCUS_ITERATIONS:-${iterations}}" ;;
  esac
  [[ "${case_iterations}" =~ ^[1-9][0-9]*$ ]] ||
    die "iteration count for ${case_name} must be a positive integer"
  for ((iteration = 1; iteration <= case_iterations; iteration += 1)); do
    run_case "${iteration}" "${case_name}" "${case_iterations}"
  done
done
