#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -n "${WAYLAND_DISPLAY:-}" ]] || die "the native Wayland smoke requires WAYLAND_DISPLAY"
[[ -n "${NIRI_SOCKET:-}" ]] || die "the native Wayland smoke requires NIRI_SOCKET"
require_command cmake
require_command ctest
require_command grep
require_command grim
require_command mktemp
require_command niri
require_command node
require_command readlink
require_command sleep
require_command timeout
ensure_sdk
prepare_ui_toolchain

printf 'Checking the native Wayland Cargo feature\n'
(
  cd -- "${repo_root}"
  cargo fmt --all -- --check
  LYNX_SDK_DIR="${verified_sdk_dir}" \
    cargo clippy --locked -p lynx-launcher-host --all-targets \
      --features native-wayland -- -D warnings
  LYNX_SDK_DIR="${verified_sdk_dir}" \
  LD_LIBRARY_PATH="${verified_sdk_dir}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    cargo test --locked -p lynx-launcher-host --all-targets \
      --features native-wayland
)

(
  cd -- "${ui_dir}"
  "${UI_PNPM[@]}" install --frozen-lockfile
  "${UI_PNPM[@]}" run build
)

wayland_build_dir="${host_dir}/build-wayland"
cmake -S "${host_dir}" -B "${wayland_build_dir}" \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo \
  -DBUILD_TESTING=ON \
  -DLYNX_LAUNCHER_NATIVE_WAYLAND=ON \
  -DLYNX_SDK_DIR="${verified_sdk_dir}"
cmake --build "${wayland_build_dir}" --parallel
ctest --test-dir "${wayland_build_dir}" --build-config RelWithDebInfo --output-on-failure

launcher="${wayland_build_dir}/lynx-launcher-wayland"
[[ -x "${launcher}" ]] || die "native Wayland backend was not staged at ${launcher}"
log_root="${repo_root}/.logs/native-wayland-smoke"
mkdir -p -- "${log_root}"
run_directory="$(mktemp -d "${log_root}/run.XXXXXX")"
outputs_json="${run_directory}/outputs.json"
placement_log="${run_directory}/placement.log"
placement_layers="${run_directory}/placement-layers.json"
placement_ppm="${run_directory}/placement.ppm"
placement_analysis="${run_directory}/placement-analysis.json"
readiness_log="${run_directory}/readiness.log"
clean_wayland_environment=(
  env
  -u DISPLAY
  -u LYNX_LAUNCHER_BUNDLE
  -u LYNX_LAUNCHER_E2E_IGNORE_FOCUS_LOSS
  -u LYNX_LAUNCHER_E2E_STARTUP_FOCUS_WAIT
  -u LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_OUTPUT
  -u LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_PROBE
  -u LYNX_LAUNCHER_ICU
  -u LYNX_LAUNCHER_LYNX_CORE
  -u LYNX_LAUNCHER_WINDOW_BACKEND
)

runner_pid=""
runner_identity=""
process_identity() {
  local pid="$1" stat remainder
  local -a fields
  [[ -r "/proc/${pid}/stat" ]] || return 1
  stat="$(<"/proc/${pid}/stat")"
  remainder="${stat#*) }"
  read -r -a fields <<<"${remainder}"
  [[ "${#fields[@]}" -gt 19 ]] || return 1
  printf '%s:%s\n' "$(readlink -f -- "/proc/${pid}/exe")" "${fields[19]}"
}

cleanup() {
  local current
  if [[ -n "${runner_pid}" ]]; then
    current="$(process_identity "${runner_pid}" 2>/dev/null || true)"
    if [[ -n "${current}" && "${current}" == "${runner_identity}" ]]; then
      kill -TERM "${runner_pid}" 2>/dev/null || true
    fi
    wait "${runner_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

niri msg -j outputs >"${outputs_json}"
probe_output="$(node - "${outputs_json}" <<'NODE'
const fs = require('node:fs');
const outputs = Object.values(JSON.parse(fs.readFileSync(process.argv[2], 'utf8')));
const candidates = outputs.filter((output) => output.logical &&
  output.logical.width >= 1120 && output.logical.height >= 760);
if (candidates.length === 0) process.exit(1);
process.stdout.write(candidates[0].name);
NODE
)" || die "Niri has no output that can contain the 1120x760 placement probe"

"${clean_wayland_environment[@]}" LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_PROBE=1 \
  LYNX_LAUNCHER_E2E_WAYLAND_PLACEMENT_OUTPUT="${probe_output}" \
  timeout --signal=TERM 15s "${launcher}" --window-backend wayland \
  >"${placement_log}" 2>&1 &
runner_pid=$!
runner_identity="$(process_identity "${runner_pid}")" ||
  die "could not capture the placement probe runner identity"

deadline=$((SECONDS + 15))
while ! grep -Fq '[host-rs] E2E Wayland placement probe frame presented' "${placement_log}"; do
  kill -0 "${runner_pid}" 2>/dev/null ||
    die "placement probe exited before presenting its EGL frame; see ${placement_log}"
  ((SECONDS < deadline)) ||
    die "timed out waiting for placement probe frame; see ${placement_log}"
  sleep 0.05
done
grep -Fq 'native Wayland layer-shell configured logical=1120x760' "${placement_log}" ||
  die "compositor configure did not preserve the requested 1120x760 geometry"
if grep -Eq 'first screen layout completed|first GL frame presented|runtime core initialized' "${placement_log}"; then
  die "placement probe emitted a Lynx readiness marker; see ${placement_log}"
fi

niri msg -j layers >"${placement_layers}"
output="$(node - "${placement_layers}" <<'NODE'
const fs = require('node:fs');
const layers = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const matches = layers.filter((entry) => entry.namespace === 'lynx-launcher');
if (matches.length !== 1 || matches[0].layer !== 'Overlay' ||
    matches[0].keyboard_interactivity !== 'OnDemand' || !matches[0].output) {
  process.exit(1);
}
process.stdout.write(matches[0].output);
NODE
)" || die "Niri did not report the expected launcher overlay layer"
[[ "${output}" == "${probe_output}" ]] ||
  die "Niri placed the probe on ${output}, expected focused output ${probe_output}"
sleep 0.5
grim -t ppm -o "${output}" "${placement_ppm}"

node - "${placement_ppm}" "${outputs_json}" "${output}" >"${placement_analysis}" <<'NODE'
const fs = require('node:fs');
const bytes = fs.readFileSync(process.argv[2]);
let offset = 0;
function token() {
  while (offset < bytes.length) {
    if (bytes[offset] === 35) {
      while (offset < bytes.length && bytes[offset] !== 10) offset++;
    } else if (bytes[offset] <= 32) {
      offset++;
    } else {
      break;
    }
  }
  const start = offset;
  while (offset < bytes.length && bytes[offset] > 32 && bytes[offset] !== 35) offset++;
  return bytes.subarray(start, offset).toString('ascii');
}
if (token() !== 'P6') throw new Error('grim did not produce binary PPM');
const width = Number(token());
const height = Number(token());
if (Number(token()) !== 255 || !Number.isInteger(width) || !Number.isInteger(height)) {
  throw new Error('invalid PPM dimensions or maximum channel value');
}
if (bytes[offset] === 13 && bytes[offset + 1] === 10) offset += 2;
else if (bytes[offset] <= 32) offset++;
const pixels = bytes.subarray(offset);
if (pixels.length !== width * height * 3) throw new Error('PPM payload size mismatch');

const outputs = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
const output = outputs[process.argv[4]];
if (!output?.logical || !Number.isInteger(output.current_mode)) {
  throw new Error('selected Niri output lacks logical geometry or current mode');
}
const {scale, transform} = output.logical;
const transforms = new Set(['Normal', '90', '180', '270', 'Flipped',
  'Flipped90', 'Flipped180', 'Flipped270']);
if (!Number.isFinite(scale) || scale <= 0 || !transforms.has(transform)) {
  throw new Error(`unsupported Niri scale/transform: ${scale}/${transform}`);
}
const close = (actual, expected, tolerance = 1) => Math.abs(actual - expected) <= tolerance;
if (!close(width, output.logical.width * scale) ||
    !close(height, output.logical.height * scale)) {
  throw new Error('PPM dimensions disagree with Niri logical geometry and scale');
}
const mode = output.modes[output.current_mode];
const quarterTurn = transform.endsWith('90') || transform.endsWith('270') ||
  transform === '90' || transform === '270';
const modeWidth = quarterTurn ? mode.height : mode.width;
const modeHeight = quarterTurn ? mode.width : mode.height;
if (width !== modeWidth || height !== modeHeight) {
  throw new Error('PPM dimensions disagree with transformed current output mode');
}

const count = width * height;
const mask = new Uint8Array(count);
for (let index = 0; index < count; index++) {
  const pixel = index * 3;
  if (pixels[pixel] >= 250 && pixels[pixel + 1] <= 5 && pixels[pixel + 2] >= 250) {
    mask[index] = 1;
  }
}
const queue = new Int32Array(count);
let largest = null;
for (let start = 0; start < count; start++) {
  if (mask[start] !== 1) continue;
  let read = 0;
  let write = 1;
  queue[0] = start;
  mask[start] = 2;
  let minX = start % width;
  let maxX = minX;
  let minY = Math.floor(start / width);
  let maxY = minY;
  while (read < write) {
    const index = queue[read++];
    const x = index % width;
    const y = Math.floor(index / width);
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
    let neighbor;
    if (x > 0 && mask[neighbor = index - 1] === 1) {
      mask[neighbor] = 2; queue[write++] = neighbor;
    }
    if (x + 1 < width && mask[neighbor = index + 1] === 1) {
      mask[neighbor] = 2; queue[write++] = neighbor;
    }
    if (y > 0 && mask[neighbor = index - width] === 1) {
      mask[neighbor] = 2; queue[write++] = neighbor;
    }
    if (y + 1 < height && mask[neighbor = index + width] === 1) {
      mask[neighbor] = 2; queue[write++] = neighbor;
    }
  }
  if (!largest || write > largest.pixels) {
    largest = {pixels: write, minX, maxX, minY, maxY};
  }
}
if (!largest) throw new Error('placement color was not found in the PPM capture');
const boxWidth = largest.maxX - largest.minX + 1;
const boxHeight = largest.maxY - largest.minY + 1;
const expectedWidth = Math.round(1120 * scale);
const expectedHeight = Math.round(760 * scale);
if (!close(boxWidth, expectedWidth, 2) || !close(boxHeight, expectedHeight, 2)) {
  throw new Error(`placement bbox ${boxWidth}x${boxHeight}, expected ${expectedWidth}x${expectedHeight}`);
}
const centerErrorX = Math.abs((largest.minX + largest.maxX + 1) / 2 - width / 2);
const centerErrorY = Math.abs((largest.minY + largest.maxY + 1) / 2 - height / 2);
if (centerErrorX > 2 || centerErrorY > 2) {
  throw new Error(`placement center error ${centerErrorX},${centerErrorY} exceeds 2 pixels`);
}
if (largest.pixels < boxWidth * boxHeight * 0.99) {
  throw new Error('placement color component is not a solid rectangle');
}
process.stdout.write(JSON.stringify({output: output.name, width, height, scale, transform,
  bbox: largest, boxWidth, boxHeight, centerErrorX, centerErrorY}, null, 2) + '\n');
NODE

set +e
wait "${runner_pid}"
runner_status=$?
set -e
runner_pid=""
[[ "${runner_status}" -eq 0 ]] ||
  die "placement probe exited with status ${runner_status}; see ${placement_log}"
grep -Fq '[host-rs] E2E Wayland placement probe shutdown complete' "${placement_log}" ||
  die "placement probe did not report clean shutdown"
if grep -Eq 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error ' "${placement_log}"; then
  die "placement probe emitted a forbidden lifecycle marker; see ${placement_log}"
fi

niri msg -j layers >"${run_directory}/layers-after-placement.json"
if ! node - "${run_directory}/layers-after-placement.json" <<'NODE'
const fs = require('node:fs');
const layers = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
if (layers.some((entry) => entry.namespace === 'lynx-launcher')) process.exit(1);
NODE
then
  die "Niri retained the placement probe layer after shutdown"
fi

set +e
"${clean_wayland_environment[@]}" timeout --signal=TERM 30s "${launcher}" \
  --window-backend wayland --exit-after-first-frame >"${readiness_log}" 2>&1
readiness_status=$?
set -e
[[ "${readiness_status}" -eq 0 ]] ||
  die "native Wayland readiness invocation exited with status ${readiness_status}; see ${readiness_log}"
for marker in \
  '[host-rs] first screen layout completed' \
  '[host-rs] first GL frame presented' \
  '[host-rs] auto-exit after first rendered frame' \
  '[host-rs] runtime core shutdown complete'; do
  grep -Fq "${marker}" "${readiness_log}" ||
    die "native Wayland readiness invocation missed ${marker}; see ${readiness_log}"
done
node - "${readiness_log}" <<'NODE' || die "native Wayland readiness markers are out of order"
const log = require('node:fs').readFileSync(process.argv[2], 'utf8');
const layout = log.indexOf('[host-rs] first screen layout completed');
const present = log.indexOf('[host-rs] first GL frame presented');
const exit = log.indexOf('[host-rs] auto-exit after first rendered frame');
if (layout < 0 || present < 0 || exit < layout || exit < present) process.exit(1);
NODE
if grep -Eq 'destroyed thread host|Maybe leaked|post an unknown task|LoadJSSource load js error|\[lynx-error ' "${readiness_log}"; then
  die "native Wayland readiness invocation emitted a forbidden lifecycle marker; see ${readiness_log}"
fi

niri msg -j layers >"${run_directory}/layers-after-readiness.json"
if ! node - "${run_directory}/layers-after-readiness.json" <<'NODE'
const fs = require('node:fs');
const layers = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
if (layers.some((entry) => entry.namespace === 'lynx-launcher')) process.exit(1);
NODE
then
  die "Niri retained the launcher layer after readiness shutdown"
fi

printf 'Native Wayland placement and readiness smoke passed on %s; artifacts: %s\n' \
  "${output}" "${run_directory}"
