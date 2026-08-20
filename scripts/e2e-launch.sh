#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${DISPLAY:-}" ]] || die "the launch E2E test requires DISPLAY"
require_command mktemp
require_command readlink
require_command setsid

click_driver="${host_build_dir}/x11_click_test_driver"
[[ -x "${host_binary}" ]] || die "host is not built; run scripts/build.sh first"
[[ -x "${click_driver}" ]] || die "X11 test driver is not built; run scripts/build.sh first"

iterations="${LYNX_LAUNCHER_E2E_ITERATIONS:-1}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_E2E_ITERATIONS must be a positive integer"

host_pid=""
host_identity=""
temporary=""
host_log=""
expected_host="$(readlink -f -- "${host_binary}")"

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

run_iteration() {
  local iteration="$1"
  temporary="$(mktemp -d "${TMPDIR:-/tmp}/lynx-launcher-e2e.XXXXXX")"
  host_log="${temporary}/host.log"
  local data_home="${temporary}/data"
  local decoy_marker="${temporary}/decoy-launched.marker"
  local target_marker="${temporary}/target-launched.marker"
  local decoy_helper="${temporary}/launch-decoy.sh"
  local target_helper="${temporary}/launch-target.sh"
  mkdir -p -- \
    "${data_home}/applications" \
    "${data_home}/icons/hicolor/64x64/apps" \
    "${temporary}/empty-data"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ": >\"${decoy_marker}\"" >"${decoy_helper}"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
    ": >\"${target_marker}\"" >"${target_helper}"
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

  setsid env \
    XDG_DATA_HOME="${data_home}" \
    XDG_DATA_DIRS="${temporary}/empty-data" \
    XDG_CURRENT_DESKTOP=LYNX_E2E \
    LC_ALL=C \
    "${host_binary}" --run-for 12 >"${host_log}" 2>&1 &
  host_pid=$!
  host_identity="$(capture_host_identity)" ||
    die "could not capture the launched host process identity"
  [[ "${host_identity}" == "${expected_host}:${host_pid}:"* ]] ||
    die "launched host did not create the expected process group"

  if ! wait_for_log '[host] Launcher.getApplications resolved 3 applications' ||
    ! wait_for_log '[host] first GL frame presented'; then
    printf 'launch E2E iteration %s: host did not become ready\n' "${iteration}" >&2
    dump_log
    return 1
  fi
  sleep 0.25

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
  "${click_driver}" --pid "${host_pid}" click --x 200 --y 305

  local deadline=$((SECONDS + 3))
  while [[ ! -e "${target_marker}" ]] && ((SECONDS < deadline)); do
    sleep 0.02
  done
  if [[ ! -e "${target_marker}" ]]; then
    if [[ -e "${decoy_marker}" ]]; then
      printf 'launch E2E iteration %s: non-target launched instead of target\n' \
        "${iteration}" >&2
    else
      printf 'launch E2E iteration %s: target marker was not created after click\n' \
        "${iteration}" >&2
    fi
    dump_log
    return 1
  fi
  sleep 0.1
  if [[ -e "${decoy_marker}" ]]; then
    printf 'launch E2E iteration %s: non-target marker was unexpectedly created\n' \
      "${iteration}" >&2
    dump_log
    return 1
  fi

  "${click_driver}" --pid "${host_pid}" defocus
  local exit_deadline=$((SECONDS + 3))
  while [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]] &&
    ((SECONDS < exit_deadline)); do
    sleep 0.02
  done
  if [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]]; then
    printf 'launch E2E iteration %s: host did not exit after focus loss\n' \
      "${iteration}" >&2
    dump_log
    return 1
  fi
  if ! wait "${host_pid}"; then
    printf 'launch E2E iteration %s: host failed while exiting after focus loss\n' \
      "${iteration}" >&2
    dump_log
    host_pid=""
    host_identity=""
    return 1
  fi
  host_pid=""
  host_identity=""
  if ! grep -Fq -- '[host] window lost focus; exiting' "${host_log}"; then
    printf 'launch E2E iteration %s: host did not report focus-loss exit\n' \
      "${iteration}" >&2
    dump_log
    return 1
  fi
  rm -rf -- "${temporary}"
  temporary=""
  printf 'launch E2E iteration %s/%s passed\n' \
    "${iteration}" "${iterations}"
}

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  run_iteration "${iteration}"
done
