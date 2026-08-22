#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${DISPLAY:-}" ]] || die "the Rust shell smoke test requires DISPLAY"
require_command grep
require_command mktemp
require_command readlink
require_command setsid
require_command sleep
require_command tail
require_command timeout

click_driver="${host_build_dir}/x11_click_test_driver"
[[ -x "${rust_host_binary}" ]] ||
  die "Rust host is not built; run scripts/build.sh first"
[[ -x "${click_driver}" ]] ||
  die "X11 test driver is not built; run scripts/build.sh first"

iterations="${LYNX_LAUNCHER_RUST_SHELL_ITERATIONS:-10}"
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_RUST_SHELL_ITERATIONS must be a positive integer"

log_root="${repo_root}/.logs/ticket-07/rust-desktop-smoke"
mkdir -p -- "${log_root}"
run_directory="$(mktemp -d "${log_root}/run.XXXXXX")"
fixture_root="${run_directory}/fixture"
data_home="${fixture_root}/data"
target_helper="${fixture_root}/launch-target.sh"
mkdir -p -- \
  "${data_home}/applications" \
  "${data_home}/icons/hicolor/64x64/apps" \
  "${fixture_root}/empty-data"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Alpha Decoy' \
  'Exec=/bin/true' \
  >"${data_home}/applications/00-resolved.desktop"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Bravo Decoy' \
  'Icon=lynx-rust-missing' \
  'Exec=/bin/true' \
  >"${data_home}/applications/10-missing.desktop"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Cobalt Target Fixture' \
  'Icon=lynx-rust-target' \
  "Exec=${target_helper}" \
  >"${data_home}/applications/20-third.desktop"
printf '%s\n' \
  '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">' \
  '  <rect width="64" height="64" fill="#00ff00"/>' \
  '</svg>' \
  >"${data_home}/icons/hicolor/64x64/apps/lynx-rust-target.svg"
fixture_environment=(
  "XDG_DATA_HOME=${data_home}"
  "XDG_DATA_DIRS=${fixture_root}/empty-data"
  'XDG_CURRENT_DESKTOP=LYNX_RUST_E2E'
  'LC_ALL=C'
  'LYNX_LAUNCHER_E2E_SNAPSHOT_TRACE=1'
  'LYNX_LAUNCHER_E2E_SYSTEM_SCALE=1.25'
  'LYNX_LAUNCHER_E2E_CURSOR_FALLBACK_PROBE=1'
)
host_pid=""
host_identity=""
host_log=""
clipboard_owner_pid=""
clipboard_owner_identity=""
display_server_pid=""
display_server_identity=""
display_server_expected=""
clipboard_test=false
clipboard_display=""
expected_host="$(readlink -f -- "${rust_host_binary}")"
expected_driver="$(readlink -f -- "${click_driver}")"

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

capture_expected_identity() {
  local pid="$1" expected="$2" identity deadline=$((SECONDS + 2))
  while ((SECONDS < deadline)); do
    identity="$(process_identity "${pid}" 2>/dev/null || true)"
    if [[ "${identity}" == "${expected}:"* ]]; then
      printf '%s\n' "${identity}"
      return 0
    fi
    kill -0 "${pid}" 2>/dev/null || return 1
    sleep 0.02
  done
  return 1
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

stop_clipboard_owner() {
  [[ -n "${clipboard_owner_pid}" ]] || return 0
  stop_verified_process "${clipboard_owner_pid}" "${clipboard_owner_identity}" \
    "clipboard selection owner"
  clipboard_owner_pid=""
  clipboard_owner_identity=""
}

stop_verified_process() {
  local pid="$1" identity="$2" label="$3" current deadline
  current="$(process_identity "${pid}" 2>/dev/null || true)"
  if [[ -z "${current}" ]]; then
    wait "${pid}" 2>/dev/null || true
    return 0
  fi
  if [[ -z "${identity}" || "${current}" != "${identity}" ]]; then
    printf 'warning: refusing to signal %s PID %s because its identity changed\n' \
      "${label}" "${pid}" >&2
    return 1
  fi
  kill -TERM -- "${pid}" 2>/dev/null || true
  deadline=$((SECONDS + 2))
  while [[ "$(process_identity "${pid}" 2>/dev/null || true)" == "${identity}" ]] &&
    ((SECONDS < deadline)); do
    sleep 0.02
  done
  if [[ "$(process_identity "${pid}" 2>/dev/null || true)" == "${identity}" ]]; then
    kill -KILL -- "${pid}" 2>/dev/null || true
    deadline=$((SECONDS + 2))
    while [[ "$(process_identity "${pid}" 2>/dev/null || true)" == "${identity}" ]] &&
      ((SECONDS < deadline)); do
      sleep 0.02
    done
  fi
  wait "${pid}" 2>/dev/null || true
  if [[ "$(process_identity "${pid}" 2>/dev/null || true)" == "${identity}" ]]; then
    printf 'warning: %s PID %s retained its identity after SIGKILL\n' \
      "${label}" "${pid}" >&2
    return 1
  fi
}

verify_cleanup_helper() {
  local pid identity expected
  sleep 30 &
  pid=$!
  expected="$(readlink -f -- "$(command -v sleep)")"
  identity="$(capture_expected_identity "${pid}" "${expected}")" ||
    die "could not verify the cleanup helper fixture identity"
  stop_verified_process "${pid}" "${identity}" "cleanup helper fixture"
  if kill -0 "${pid}" 2>/dev/null; then
    die "cleanup helper left its fixture process alive"
  fi

  bash -c 'trap "" TERM; exec tail -f /dev/null' &
  pid=$!
  expected="$(readlink -f -- "$(command -v tail)")"
  identity="$(capture_expected_identity "${pid}" "${expected}")" ||
    die "could not verify the cleanup escalation fixture identity"
  stop_verified_process "${pid}" "${identity}" "cleanup escalation fixture"
  if kill -0 "${pid}" 2>/dev/null; then
    die "cleanup helper did not SIGKILL its TERM-resistant fixture"
  fi
}

cleanup() {
  stop_host
  stop_clipboard_owner
  if [[ -n "${display_server_pid}" ]]; then
    if ! stop_verified_process "${display_server_pid}" "${display_server_identity}" \
      "isolated X server"; then
      return
    fi
    display_server_pid=""
    display_server_identity=""
  fi
}

prepare_isolated_clipboard_display() {
  if [[ "${LYNX_LAUNCHER_TEST_ISOLATED_DISPLAY:-}" == 1 ]]; then
    clipboard_display="${DISPLAY}"
    clipboard_test=true
    return
  fi
  local display_file="${run_directory}/xwayland.display"
  local server_log="${run_directory}/xwayland.log"
  local deadline display_number server_kind
  exec {xwayland_display_fd}>"${display_file}"
  if [[ -n "${WAYLAND_DISPLAY:-}" ]] && command -v Xwayland >/dev/null 2>&1; then
    server_kind="Xwayland"
    Xwayland -displayfd "${xwayland_display_fd}" -geometry 1600x1200 -noreset \
      >"${server_log}" 2>&1 &
  elif command -v Xvfb >/dev/null 2>&1; then
    server_kind="Xvfb"
    Xvfb -displayfd "${xwayland_display_fd}" -screen 0 1600x1200x24 \
      -nolisten tcp -noreset >"${server_log}" 2>&1 &
  else
    exec {xwayland_display_fd}>&-
    die "isolated clipboard smoke requires standalone Xwayland with WAYLAND_DISPLAY, or Xvfb"
  fi
  display_server_pid=$!
  exec {xwayland_display_fd}>&-
  display_server_expected="$(readlink -f -- "$(command -v "${server_kind}")")"
  display_server_identity="$(capture_expected_identity \
    "${display_server_pid}" "${display_server_expected}")" ||
    die "could not verify the isolated ${server_kind} process identity"
  deadline=$((SECONDS + 3))
  while [[ ! -s "${display_file}" ]] && ((SECONDS < deadline)); do
    kill -0 "${display_server_pid}" 2>/dev/null || break
    sleep 0.02
  done
  [[ -s "${display_file}" ]] || die "isolated Xwayland did not become ready"
  display_number="$(<"${display_file}")"
  [[ "${display_number}" =~ ^[0-9]+$ ]] || die "isolated Xwayland returned an invalid display"
  clipboard_display=":${display_number}"
  clipboard_test=true
  printf 'Rust clipboard smoke uses isolated %s DISPLAY=%s.\n' \
    "${server_kind}" "${clipboard_display}"
}

trap cleanup EXIT
require_visible_capture_commands
verify_cleanup_helper

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
    '[host-rs] snapshot index=0 id=00-resolved.desktop name=Alpha Decoy icon=missing'
    '[host-rs] snapshot index=1 id=10-missing.desktop name=Bravo Decoy icon=missing'
    '[host-rs] snapshot index=2 id=20-third.desktop name=Cobalt Target Fixture icon=present'
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
    '[host-rs] view entered foreground' \
    '[host-rs] metrics logical=1120x760 dpr=1.25 framebuffer-scale=1x1' \
    '[host-rs] Launcher.getApplications resolved 3 applications' \
    '[host-rs] unknown cursor callback probe requested arrow fallback' \
    '[host-rs] runtime core initialized' \
    '[host-rs] runtime core shutdown complete'; do
    if ! grep -Fq -- "${marker}" "${path}"; then
      dump_log "${path}"
      die "${label} did not emit ${marker}"
    fi
  done
  assert_snapshot_trace "${path}" "${label}"
}

assert_input_trace() {
  local path="$1" label="$2" key_down_count key_up_count
  key_down_count="$(grep -Ec -- \
    '\[host-rs\] key dispatch type=down physical=[1-9][0-9]* logical=[0-9]+ synthesized=false' \
    "${path}" || true)"
  key_up_count="$(grep -Ec -- \
    '\[host-rs\] key dispatch type=up physical=[1-9][0-9]* logical=[0-9]+ synthesized=false' \
    "${path}" || true)"
  if ((key_down_count < 6 || key_up_count < 6)); then
    dump_log "${path}"
    die "${label} did not dispatch real key down/up events for the query"
  fi
  for marker in \
    '[host-rs] key dispatch type=repeat physical=458977 logical=8589934850 synthesized=false' \
    '[host-rs] pointer dispatch phase=hover signal=scroll x=250 y=380 scroll-logical-y=-100 scroll-physical-y=-125 buttons=0' \
    '[host-rs] key dispatch type=up physical=458981 logical=8589934851 synthesized=true' \
    '[host-rs] pointer dispatch phase=cancel signal=none x=275 y=200 scroll-logical-y=0 scroll-physical-y=0 buttons=1' \
    '[host-rs] pointer dispatch phase=remove signal=none x=275 y=200 scroll-logical-y=0 scroll-physical-y=0 buttons=0' \
    '[host-rs] view entered background'; do
    if ! grep -Fq -- "${marker}" "${path}"; then
      dump_log "${path}"
      die "${label} did not emit ${marker}"
    fi
  done

  local background_seen=false line
  while IFS= read -r line; do
    if [[ "${line}" == *'[host-rs] view entered background'* ]]; then
      background_seen=true
      continue
    fi
    if [[ "${background_seen}" == true &&
      ("${line}" == *'[host-rs] pointer dispatch '* ||
        "${line}" == *'[host-rs] key dispatch '* ||
        "${line}" == *'[host-rs] character '*) ]]; then
      dump_log "${path}"
      die "${label} dispatched input after entering background: ${line}"
    fi
  done <"${path}"
}

for ((iteration = 1; iteration <= iterations; iteration += 1)); do
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 0' >"${target_helper}"
  chmod +x -- "${target_helper}"
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
    ! wait_for_log '[host-rs] snapshot index=2 id=20-third.desktop name=Cobalt Target Fixture icon=present'; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not render the real launcher"
  fi
  rm -f -- "${target_helper}"

  "${click_driver}" --pid "${host_pid}" expect-popup
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 0 --y 220 --width 1120 --height 300 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 512 --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" expect-cursor-shapes \
    --x 220 --y 160 --other-x 800 --other-y 80 \
    --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" click --x 220 --y 160 --scale 1.25
  sleep 0.2
  "${click_driver}" --pid "${host_pid}" click --x 220 --y 160 --scale 1.25
  sleep 0.1
  "${click_driver}" --pid "${host_pid}" type --text cobalt
  "${click_driver}" --pid "${host_pid}" click --x 220 --y 160 --scale 1.25
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 40 --y 265 --width 76 --height 76 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 512 --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" expect-no-pixel \
    --x 360 --y 270 --width 700 --height 100 \
    --red 241 --green 234 --blue 217 --tolerance 20 \
    --maximum-matches 20 --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" click --x 200 --y 305 --scale 1.25
  if ! wait_for_log \
    '[host-rs] Launcher.launchApplication Promise rejected: No such file or directory (os error 2)'; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not reject the real target spawn failure"
  fi
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 45 --y 205 --width 1030 --height 75 \
    --red 255 --green 107 --blue 61 --tolerance 12 \
    --minimum-matches 10 --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 40 --y 345 --width 90 --height 75 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 128 --timeout-ms 3000 --scale 1.25
  "${click_driver}" --pid "${host_pid}" scroll \
    --x 200 --y 304 --direction up --scale 1.25
  "${click_driver}" --pid "${host_pid}" repeat-key --key left-shift
  "${click_driver}" --pid "${host_pid}" hold-input \
    --key right-shift --x 220 --y 160 --scale 1.25
  "${click_driver}" --pid "${host_pid}" defocus

  if ! wait_for_host_exit; then
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not exit cleanly after focus loss"
  fi
  assert_success_log "${host_log}" "Rust bounded iteration ${iteration}"
  assert_input_trace "${host_log}" "Rust bounded iteration ${iteration}"
  grep -Fq -- '[host-rs] window lost focus; exiting' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust shell iteration ${iteration} did not handle focus loss"
  }
  printf 'Rust first-frame and bounded iteration %s/%s passed.\n' \
    "${iteration}" "${iterations}"
done

host_log="${run_directory}/startup-focus-loss.log"
setsid env "${fixture_environment[@]}" \
  LYNX_LAUNCHER_E2E_STARTUP_FOCUS_WAIT=1 \
  "${rust_host_binary}" --run-for 12 >"${host_log}" 2>&1 &
host_pid=$!
host_identity="$(capture_host_identity)" ||
  die "could not capture the Rust startup-focus process identity"
startup_focus_sent=false
startup_focus_deadline=$((SECONDS + 3))
while ((SECONDS < startup_focus_deadline)); do
  if "${click_driver}" --pid "${host_pid}" defocus \
    >"${run_directory}/startup-focus-driver.log" 2>&1; then
    startup_focus_sent=true
    break
  fi
  [[ "$(process_identity "${host_pid}" 2>/dev/null || true)" == "${host_identity}" ]] || break
  sleep 0.01
done
if [[ "${startup_focus_sent}" != true ]]; then
  dump_log "${host_log}"
  die "could not inject focus loss during the Rust startup event pump"
fi
if ! wait_for_host_exit; then
  dump_log "${host_log}"
  die "Rust startup-focus process did not exit cleanly"
fi
assert_clean_host_log "${host_log}" "Rust startup-focus process"
for marker in \
  '[host-rs] startup window lost focus; closing' \
  '[host-rs] runtime core initialized' \
  '[host-rs] view entered foreground' \
  '[host-rs] view entered background' \
  '[host-rs] runtime core shutdown complete'; do
  if ! grep -Fq -- "${marker}" "${host_log}"; then
    dump_log "${host_log}"
    die "Rust startup-focus process did not emit ${marker}"
  fi
done
printf 'Rust startup focus-loss smoke passed.\n'

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
  --minimum-matches 800 --timeout-ms 3000 --scale 1.25
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

prepare_isolated_clipboard_display
if [[ "${clipboard_test}" == true ]]; then
  clipboard_owner_log="${run_directory}/clipboard-owner.log"
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" clipboard-owner --text cobalt >"${clipboard_owner_log}" 2>&1 &
  clipboard_owner_pid=$!
  clipboard_owner_identity="$(capture_expected_identity "${clipboard_owner_pid}" "${expected_driver}")" ||
    die "could not verify the clipboard owner process identity"
  clipboard_owner_deadline=$((SECONDS + 3))
  while ! grep -Fq -- 'clipboard owner ready' "${clipboard_owner_log}" &&
    ((SECONDS < clipboard_owner_deadline)); do
    kill -0 "${clipboard_owner_pid}" 2>/dev/null || break
    sleep 0.02
  done
  grep -Fq -- 'clipboard owner ready' "${clipboard_owner_log}" ||
    die "isolated X11 clipboard owner did not become ready"
  host_log="${run_directory}/clipboard-integration.log"
  setsid env -u NIRI_SOCKET "${fixture_environment[@]}" \
    DISPLAY="${clipboard_display}" LYNX_LAUNCHER_E2E_IGNORE_FOCUS_LOSS=1 \
    "${rust_host_binary}" --run-for 4 >"${host_log}" 2>&1 &
  host_pid=$!
  host_identity="$(capture_host_identity)" ||
    die "could not capture the Rust clipboard process identity"
  if ! wait_for_log '[host-rs] first screen layout completed' ||
    ! wait_for_log '[host-rs] first GL frame presented'; then
    dump_log "${host_log}"
    die "Rust clipboard process did not render the launcher"
  fi
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" --pid "${host_pid}" click --x 220 --y 160 --scale 1.25
  sleep 0.1
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" --pid "${host_pid}" shortcut --key ctrl-v
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" --pid "${host_pid}" expect-pixel \
    --x 40 --y 265 --width 76 --height 76 \
    --red 0 --green 255 --blue 0 --tolerance 8 \
    --minimum-matches 512 --timeout-ms 3000 --scale 1.25
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" --pid "${host_pid}" shortcut --key ctrl-a
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" --pid "${host_pid}" shortcut --key ctrl-c
  env -u NIRI_SOCKET DISPLAY="${clipboard_display}" \
    "${click_driver}" clipboard-read --text cobalt --timeout-ms 3000
  if ! wait_for_log '[host-rs] clipboard read callback completed' ||
    ! wait_for_log '[host-rs] clipboard write callback completed'; then
    dump_log "${host_log}"
    die "Rust clipboard process did not execute both Lynx clipboard callbacks"
  fi
  stop_clipboard_owner
  if ! wait_for_host_exit; then
    dump_log "${host_log}"
    die "Rust clipboard process did not exit cleanly"
  fi
  assert_success_log "${host_log}" "Rust clipboard process"
  grep -Fq -- '[host-rs] auto-exit after bounded run' "${host_log}" || {
    dump_log "${host_log}"
    die "Rust clipboard process did not complete its bounded run"
  }
  printf 'Rust isolated clipboard read/write smoke passed.\n'
fi

printf 'Rust shell smoke passed. Logs: %s\n' "${run_directory}"
