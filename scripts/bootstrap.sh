#!/usr/bin/env bash
# Habitat setup intentionally mutates HOME independently in two subshells.
# shellcheck disable=SC2030,SC2031
set -euo pipefail

# shellcheck source=scripts/_common.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_command git
require_command curl
require_command python3
require_command tee
require_command sha256sum
require_command unzip
require_command cmp
require_command flock
require_command mktemp

[[ "$(uname -s)" == "Linux" ]] || die "the pinned SDK build currently supports Linux only"
[[ "$(uname -m)" == "x86_64" ]] || die "the pinned SDK build currently supports x86_64 only"

lock_timeout="${LYNX_LAUNCHER_BOOTSTRAP_LOCK_TIMEOUT:-1800}"
[[ "${lock_timeout}" =~ ^[1-9][0-9]*$ ]] ||
  die "LYNX_LAUNCHER_BOOTSTRAP_LOCK_TIMEOUT must be a positive integer"
mkdir -p -- "${build_home}"
bootstrap_lock_file="${build_home}/bootstrap.lock"
exec {bootstrap_lock_fd}>"${bootstrap_lock_file}"
printf 'Waiting up to %ss for bootstrap lock: %s\n' \
  "${lock_timeout}" "${bootstrap_lock_file}"
if ! flock --exclusive --timeout "${lock_timeout}" "${bootstrap_lock_fd}"; then
  die "timed out waiting for bootstrap lock after ${lock_timeout}s: ${bootstrap_lock_file}"
fi
mkdir -p -- "${build_home}/.cache" "${sdk_cache_root}" "${repo_root}/.logs"
exec > >(tee "${repo_root}/.logs/bootstrap.log") 2>&1
printf 'Acquired bootstrap lock: %s\n' "${bootstrap_lock_file}"

expected_sha="$(pinned_lynx_sha)"
printf 'Initializing Lynx submodule at pinned SHA %s\n' "${expected_sha}"
if git -C "${lynx_dir}" rev-parse --git-dir >/dev/null 2>&1; then
  verify_lynx_checkout_clean
fi
git -C "${repo_root}" submodule sync --recursive -- third_party/lynx
git -C "${repo_root}" submodule update --init --recursive --checkout -- third_party/lynx
verify_lynx_checkout
verify_lynx_checkout_clean

applied_patch_count=0
cache_temporary=""

restore_lynx_patches() {
  local status=$? cleanup_failed=0 index patch
  trap - EXIT INT TERM

  if [[ -n "${cache_temporary}" && "${cache_temporary}" == "${sdk_cache_root}/.cache."* ]]; then
    rm -rf -- "${cache_temporary}"
    cache_temporary=""
  fi

  for ((index = applied_patch_count - 1; index >= 0; index -= 1)); do
    patch="${repo_root}/${lynx_patch_relpaths[index]}"
    if git -C "${lynx_dir}" apply --reverse --check "${patch}" &&
      git -C "${lynx_dir}" apply --reverse "${patch}"; then
      printf 'Reversed Lynx integration patch: %s\n' "${lynx_patch_relpaths[index]}"
    else
      printf 'error: could not reverse Lynx integration patch: %s\n' \
        "${lynx_patch_relpaths[index]}" >&2
      cleanup_failed=1
    fi
  done

  if ! lynx_checkout_is_pinned; then
    printf 'error: Lynx submodule no longer points at pinned SHA %s\n' \
      "${expected_sha}" >&2
    cleanup_failed=1
  fi
  if ! lynx_checkout_is_clean; then
    printf 'error: Lynx submodule was not restored to a clean worktree\n' >&2
    git -C "${lynx_dir}" status --short >&2 || true
    cleanup_failed=1
  fi
  if ((cleanup_failed)); then
    status=1
  else
    printf 'Restored clean Lynx submodule at %s\n' "${expected_sha}"
  fi
  exit "${status}"
}

trap restore_lynx_patches EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

printf 'Synchronizing Lynx clay dependencies with isolated HOME %s\n' "${build_home}"
(
  export HOME="${build_home}"
  export XDG_CACHE_HOME="${build_home}/.cache"
  cd -- "${lynx_dir}"
  set +u
  # envsetup is supplied by the pinned Lynx checkout.
  # shellcheck source=/dev/null
  source tools/envsetup.sh
  set -u
  tools/hab sync . --target clay
)

verify_lynx_checkout_clean
printf 'Applying trusted Lynx patch set %s in fixed order\n' \
  "${lynx_patch_set_hash}"
for patch_relative in "${lynx_patch_relpaths[@]}"; do
  patch="${repo_root}/${patch_relative}"
  git -C "${lynx_dir}" apply --check "${patch}"
  git -C "${lynx_dir}" apply --whitespace=error-all "${patch}"
  applied_patch_count=$((applied_patch_count + 1))
  printf 'Applied Lynx integration patch: %s\n' "${patch_relative}"
done
git -C "${lynx_dir}" diff --check

# The upstream package action does not always notice a relinked liblynx.so.
# Remove its static outputs so this source key cannot inherit an older ZIP.
rm -f -- "${sdk_build_archive}" "${sdk_build_checksum}"

printf 'Building the Lynx Linux x64 SDK for source key %s\n' \
  "${lynx_sdk_cache_key}"
(
  export HOME="${build_home}"
  export XDG_CACHE_HOME="${build_home}/.cache"
  cd -- "${lynx_dir}"
  set +u
  # shellcheck source=/dev/null
  source tools/envsetup.sh
  set -u
  python3 platform/linux/build_release.py --target-cpu x64
)

[[ -f "${sdk_build_archive}" && -f "${sdk_build_checksum}" ]] ||
  die "patched Lynx SDK build did not produce the expected archive and checksum"
(
  cd -- "${sdk_build_dir}"
  sha256sum -c "$(basename -- "${sdk_build_checksum}")"
)

built_lib="${sdk_build_dir}/liblynx.so"
[[ -f "${built_lib}" ]] || die "patched Lynx build did not produce ${built_lib}"
read -r built_lib_digest _ < <(sha256sum "${built_lib}")
read -r archive_lib_digest _ < <(unzip -p "${sdk_build_archive}" lib/liblynx.so | sha256sum)
[[ "${built_lib_digest}" == "${archive_lib_digest}" ]] ||
  die "SDK archive liblynx.so does not match the patched build output"

cache_temporary="$(mktemp -d "${sdk_cache_root}/.cache.XXXXXX")"
cp -- "${sdk_build_archive}" "${cache_temporary}/lynx_sdk_linux_x64.zip"
cp -- "${sdk_build_checksum}" "${cache_temporary}/lynx_sdk_linux_x64.zip.sha256"
(
  cd -- "${cache_temporary}"
  sha256sum -c lynx_sdk_linux_x64.zip.sha256
)
printf '%s\n' "${lynx_sdk_cache_key}" >"${cache_temporary}/source.key"
rm -rf -- "${sdk_cache_dir}"
mv -- "${cache_temporary}" "${sdk_cache_dir}"
cache_temporary=""

prepare_verified_sdk
read -r verified_lib_digest _ < <(sha256sum "${verified_sdk_dir}/lib/liblynx.so")
[[ "${verified_lib_digest}" == "${built_lib_digest}" ]] ||
  die "verified SDK liblynx.so does not match the patched build output"

printf 'Patched liblynx.so SHA256: %s\n' "${built_lib_digest}"
printf 'Lynx SDK source key: %s\n' "${lynx_sdk_cache_key}"

printf 'Bootstrap complete. Log: %s\n' "${repo_root}/.logs/bootstrap.log"
