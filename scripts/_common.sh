#!/usr/bin/env bash
# Values in this file are consumed by the scripts that source it.
# shellcheck disable=SC2034

scripts_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${scripts_dir}/.." && pwd)"
lynx_dir="${repo_root}/third_party/lynx"
sdk_build_dir="${lynx_dir}/out/Default"
sdk_build_archive="${sdk_build_dir}/lynx_sdk_linux_x64.zip"
sdk_build_checksum="${sdk_build_archive}.sha256"
build_home="${repo_root}/.build-home"
sdk_cache_root="${build_home}/sdk-cache"
verified_sdk_root="${build_home}/sdk"
verified_sdk_dir=""
ui_dir="${repo_root}/ui"
platform_dir="${repo_root}/platform"
host_dir="${repo_root}/host"
host_build_dir="${host_dir}/build"
host_binary="${host_build_dir}/lynx-launcher"
rust_host_binary="${host_build_dir}/lynx-launcher-rs"
lynx_patch_relpaths=(
  "patches/lynx/0001-linux-windowless-teardown.patch"
  "patches/lynx/0002-linux-fontconfig-fallback.patch"
  "patches/lynx/0003-linux-launcher-size-profile.patch"
)

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

pinned_lynx_sha() {
  local entry mode object stage path
  entry="$(git -C "${repo_root}" ls-files --stage -- third_party/lynx)" ||
    die "could not read the third_party/lynx gitlink"
  read -r mode object stage path <<<"${entry}"
  if [[ "${mode:-}" != "160000" || -z "${object:-}" || "${stage:-}" != "0" || "${path:-}" != "third_party/lynx" ]]; then
    die "third_party/lynx is not a pinned Git submodule in the index"
  fi
  printf '%s\n' "${object}"
}

verify_known_lynx_patch_set() {
  local patch expected index
  local -a actual=()

  for patch in "${repo_root}"/patches/lynx/*.patch; do
    [[ -e "${patch}" || -L "${patch}" ]] || continue
    actual+=("${patch}")
  done

  [[ "${#actual[@]}" -eq "${#lynx_patch_relpaths[@]}" ]] ||
    die "unexpected Lynx patch set; only the explicit integration patch allowlist is accepted"
  for ((index = 0; index < ${#lynx_patch_relpaths[@]}; index += 1)); do
    expected="${repo_root}/${lynx_patch_relpaths[index]}"
    [[ "${actual[index]}" == "${expected}" ]] ||
      die "unexpected Lynx patch or patch order: ${actual[index]}"
    [[ -f "${expected}" && ! -L "${expected}" ]] ||
      die "trusted Lynx patch must be a regular non-symlink file: ${lynx_patch_relpaths[index]}"
  done
}

lynx_patch_set_sha256() {
  local relative digest remainder
  verify_known_lynx_patch_set
  {
    for relative in "${lynx_patch_relpaths[@]}"; do
      read -r digest remainder < <(sha256sum "${repo_root}/${relative}")
      printf '%s  %s\n' "${digest}" "${relative}"
    done
  } | sha256sum | {
    read -r digest remainder
    printf '%s\n' "${digest}"
  }
}

lynx_patch_set_hash="$(lynx_patch_set_sha256)"
lynx_sdk_cache_key="$(pinned_lynx_sha)-${lynx_patch_set_hash}"
sdk_cache_dir="${sdk_cache_root}/${lynx_sdk_cache_key}"
sdk_archive="${sdk_cache_dir}/lynx_sdk_linux_x64.zip"
sdk_checksum="${sdk_archive}.sha256"
sdk_stamp="${sdk_cache_dir}/source.key"

lynx_checkout_is_pinned() {
  local expected actual
  expected="$(pinned_lynx_sha)"
  actual="$(git -C "${lynx_dir}" rev-parse HEAD 2>/dev/null)" || return 1
  [[ "${actual}" == "${expected}" ]]
}

lynx_checkout_is_clean() {
  [[ -z "$(git -C "${lynx_dir}" status --porcelain=v1 --untracked-files=all)" ]]
}

verify_lynx_checkout() {
  local expected actual
  expected="$(pinned_lynx_sha)"
  actual="$(git -C "${lynx_dir}" rev-parse HEAD 2>/dev/null)" ||
    die "Lynx submodule is not initialized; run scripts/bootstrap.sh"
  [[ "${actual}" == "${expected}" ]] ||
    die "Lynx checkout is ${actual}, expected pinned SHA ${expected}; run scripts/bootstrap.sh"
}

verify_lynx_checkout_clean() {
  lynx_checkout_is_clean ||
    die "Lynx submodule has local changes; refusing to overwrite or build them"
}

sdk_archive_is_ready() {
  local expected stamped
  expected="${lynx_sdk_cache_key}"
  [[ -f "${sdk_stamp}" ]] || return 1
  IFS= read -r stamped <"${sdk_stamp}" || return 1
  [[ "${stamped}" == "${expected}" &&
    -f "${sdk_archive}" &&
    -f "${sdk_checksum}" ]]
}

verify_sdk_archive() {
  sdk_archive_is_ready ||
    die "Lynx Linux x64 SDK archive is missing or not built from the pinned SHA and trusted patch set"
  require_command sha256sum
  (
    cd -- "${sdk_cache_dir}" || exit
    sha256sum -c "$(basename -- "${sdk_checksum}")"
  )
}

sdk_archive_sha256() {
  local digest
  read -r digest _ < <(sha256sum "${sdk_archive}")
  [[ "${digest}" =~ ^[0-9a-f]{64}$ ]] || die "could not hash the Lynx SDK archive"
  printf '%s\n' "${digest}"
}

verified_sdk_path() {
  printf '%s/%s\n' "${verified_sdk_root}" "${lynx_sdk_cache_key}"
}

extracted_sdk_has_layout() {
  local directory="$1"
  [[ -f "${directory}/lib/liblynx.so" &&
    -d "${directory}/include" &&
    -f "${directory}/data/icudtl.dat" &&
    -f "${directory}/lynx_core.js" ]]
}

extracted_sdk_matches_archive() {
  local directory="$1" entry
  extracted_sdk_has_layout "${directory}" || return 1
  while IFS= read -r entry; do
    [[ "${entry}" == */ ]] && continue
    [[ -f "${directory}/${entry}" ]] || return 1
    unzip -p "${sdk_archive}" "${entry}" | cmp -s - "${directory}/${entry}" || return 1
  done < <(unzip -Z1 "${sdk_archive}")
}

verified_sdk_is_ready() {
  local directory archive_digest extracted_digest extracted_source_key
  directory="$(verified_sdk_path)"
  extracted_sdk_has_layout "${directory}" || return 1
  [[ -f "${directory}/.archive.sha256" ]] || return 1
  [[ -f "${directory}/.source.key" ]] || return 1
  IFS= read -r extracted_digest <"${directory}/.archive.sha256" || return 1
  IFS= read -r extracted_source_key <"${directory}/.source.key" || return 1
  archive_digest="$(sdk_archive_sha256)"
  [[ "${extracted_digest}" == "${archive_digest}" ]] || return 1
  [[ "${extracted_source_key}" == "${lynx_sdk_cache_key}" ]] || return 1
  extracted_sdk_matches_archive "${directory}"
}

prepare_verified_sdk() {
  local directory archive_digest entry
  verify_sdk_archive
  require_command unzip
  require_command cmp
  require_command mktemp
  directory="$(verified_sdk_path)"
  archive_digest="$(sdk_archive_sha256)"

  if ! verified_sdk_is_ready; then
    while IFS= read -r entry; do
      case "${entry}" in
        /*|../*|*/../*|*/..)
          die "unsafe path in Lynx SDK archive: ${entry}"
          ;;
      esac
    done < <(unzip -Z1 "${sdk_archive}")

    mkdir -p -- "${verified_sdk_root}"
    (
      temporary="$(mktemp -d "${verified_sdk_root}/.extract.XXXXXX")"
      # shellcheck disable=SC2317 # Invoked by the EXIT trap below.
      cleanup_verified_sdk_temporary() {
        if [[ -n "${temporary}" && "${temporary}" == "${verified_sdk_root}/.extract."* ]]; then
          rm -rf -- "${temporary}"
        fi
      }
      trap cleanup_verified_sdk_temporary EXIT
      trap 'exit 130' INT
      trap 'exit 143' TERM

      unzip -q "${sdk_archive}" -d "${temporary}"
      extracted_sdk_matches_archive "${temporary}" ||
        die "extracted Lynx SDK does not match the verified archive in ${temporary}"
      printf '%s\n' "${archive_digest}" >"${temporary}/.archive.sha256"
      printf '%s\n' "${lynx_sdk_cache_key}" >"${temporary}/.source.key"
      rm -rf -- "${directory}"
      mv -- "${temporary}" "${directory}"
      temporary=""
    )
  fi

  verified_sdk_dir="${directory}"
  printf 'Verified Lynx SDK: %s\n' "${verified_sdk_dir}"
}

ensure_sdk() {
  if ! lynx_checkout_is_pinned || ! sdk_archive_is_ready; then
    printf 'Lynx SDK archive is missing or stale; running bootstrap.\n'
    "${scripts_dir}/bootstrap.sh"
  fi
  verify_lynx_checkout
  verify_lynx_checkout_clean
  prepare_verified_sdk
}

prepare_ui_toolchain() {
  local required_node actual_node package_manager expected_pnpm actual_pnpm
  require_command node
  require_command corepack

  IFS= read -r required_node <"${repo_root}/.nvmrc" ||
    die "could not read .nvmrc"
  required_node="v${required_node#v}"
  actual_node="$(node --version)"
  [[ "${actual_node}" == "${required_node}" ]] ||
    die "Node ${required_node} is required (found ${actual_node}); run 'nvm use' or 'fnm use'"

  package_manager="$(node -e '
    const fs = require("node:fs");
    const pkg = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    if (typeof pkg.packageManager !== "string") process.exit(1);
    process.stdout.write(pkg.packageManager);
  ' "${ui_dir}/package.json")" || die "ui/package.json must declare packageManager"
  [[ "${package_manager}" == pnpm@* ]] ||
    die "unsupported UI package manager declaration: ${package_manager}"

  export COREPACK_HOME="${COREPACK_HOME:-${build_home}/.cache/corepack}"
  mkdir -p -- "${COREPACK_HOME}"
  UI_PNPM=(corepack "${package_manager}")
  expected_pnpm="${package_manager#pnpm@}"
  expected_pnpm="${expected_pnpm%%+*}"
  actual_pnpm="$("${UI_PNPM[@]}" --version)"
  [[ "${actual_pnpm}" == "${expected_pnpm}" ]] ||
    die "Corepack resolved pnpm ${actual_pnpm}, expected ${expected_pnpm}"
}
