#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"

run_linux=true
run_macos=true
run_windows=true
failed_targets=0

run_target() {
  local target_name="$1"
  shift

  if "$@"; then
    printf '[OK] %s bundle built\n' "${target_name}"
  else
    local status=$?
    printf '[FAIL] %s bundle failed (code %d)\n' "${target_name}" "${status}" >&2
    failed_targets=$((failed_targets + 1))
  fi

  return 0
}

package_windows_bundle() {
  local repo_dir="$1"

  readonly WINDOWS_TARGET="x86_64-pc-windows-msvc"
  readonly WINDOWS_BINARY="${repo_dir}/target/${WINDOWS_TARGET}/release/trueos-rdp.exe"
  readonly WINDOWS_DIST_DIR="${repo_dir}/dist"
  readonly WINDOWS_ARCHIVE="${WINDOWS_DIST_DIR}/trueos-rdp-windows-x86_64.zip"

  rustup target add "${WINDOWS_TARGET}"
  (
    cd "${repo_dir}"
    cargo build --locked --release --target "${WINDOWS_TARGET}"
  )

  if [[ ! -f "${WINDOWS_BINARY}" ]]; then
    printf 'Windows build did not produce %s\n' "${WINDOWS_BINARY}" >&2
    return 1
  fi

  install -d "${WINDOWS_DIST_DIR}"
  rm -f -- "${WINDOWS_ARCHIVE}"
  (
    cd "${repo_dir}/target/${WINDOWS_TARGET}/release"
    zip --quiet "${WINDOWS_ARCHIVE}" trueos-rdp.exe
  )
  printf 'Bundle: %s\n' "${WINDOWS_ARCHIVE}"
  sha256sum "${WINDOWS_ARCHIVE}"
}

for arg in "$@"; do
  case "${arg}" in
    --linux)
      run_macos=false
      run_windows=false
      ;;
    --macos)
      run_linux=false
      run_windows=false
      ;;
    --windows)
      run_linux=false
      run_macos=false
      ;;
    --no-linux)
      run_linux=false
      ;;
    --no-macos)
      run_macos=false
      ;;
    --no-windows)
      run_windows=false
      ;;
    -h|--help)
      cat <<EOF
usage: package-all-targets.sh [--linux|--macos|--windows] [--no-linux] [--no-macos] [--no-windows]
Build all available platform packages. Defaults to: linux + macOS + windows (best-effort).
EOF
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n' "${arg}" >&2
      exit 1
      ;;
  esac
done

if [[ "${run_linux}" == true ]]; then
  if [[ "$(uname -s)" == Linux ]]; then
    run_target "Linux portable" "${REPO_DIR}/tools/build-portable-linux.sh"
  else
    printf 'Skipping Linux portable bundle (requires Linux host): %s\n' "$(uname -s)"
  fi
fi

if [[ "${run_macos}" == true ]]; then
  if [[ "$(uname -s)" == Darwin ]]; then
    run_target "macOS universal" "${REPO_DIR}/tools/build-macos.sh"
  else
    printf 'Skipping macOS bundle (requires macOS host): %s\n' "$(uname -s)"
  fi
fi

if [[ "${run_windows}" == true ]]; then
  if [[ "$(uname -s)" == *MINGW* || "$(uname -s)" == *NT* ]]; then
    run_target "Windows native" package_windows_bundle "${REPO_DIR}"
  else
    printf 'Skipping Windows bundle (requires Windows host): %s\n' "$(uname -s)"
  fi
fi

if ((failed_targets > 0)); then
  printf '%s target package step(s) failed\n' "${failed_targets}" >&2
  exit 1
fi

printf 'Done.\n'
