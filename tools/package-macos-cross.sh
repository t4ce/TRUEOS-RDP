#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly SOURCE_BINARY="${1:-${REPO_DIR}/target/universal2-apple-darwin/release/trueos-rdp}"
readonly DIST_DIR="${REPO_DIR}/dist"
readonly APP_DIR="${DIST_DIR}/TRUEOS-RDP.app"
readonly CONTENTS_DIR="${APP_DIR}/Contents"
readonly MACOS_DIR="${CONTENTS_DIR}/MacOS"
readonly RESOURCES_DIR="${CONTENTS_DIR}/Resources"
readonly ARCHIVE="${DIST_DIR}/trueos-rdp-macos-universal-unsigned.zip"

for command_name in file install sha256sum zip; do
    if ! command -v "${command_name}" >/dev/null 2>&1; then
        printf 'Missing required packaging command: %s\n' "${command_name}" >&2
        exit 1
    fi
done

if [[ ! -f "${SOURCE_BINARY}" ]]; then
    printf 'Universal macOS executable not found: %s\n' "${SOURCE_BINARY}" >&2
    exit 1
fi

readonly BINARY_DESCRIPTION="$(file "${SOURCE_BINARY}")"
if [[ "${BINARY_DESCRIPTION}" != *"Mach-O universal binary with 2 architectures"* ]] ||
    [[ "${BINARY_DESCRIPTION}" != *"x86_64"* ]] ||
    [[ "${BINARY_DESCRIPTION}" != *"arm64"* ]]; then
    printf 'Expected a universal x86_64 + arm64 Mach-O: %s\n' \
        "${BINARY_DESCRIPTION}" >&2
    exit 1
fi

install -d "${DIST_DIR}"
rm -rf -- "${APP_DIR}"
install -d "${MACOS_DIR}" "${RESOURCES_DIR}"
install -m 0755 "${REPO_DIR}/packaging/macos/launcher.sh" \
    "${MACOS_DIR}/trueos-rdp"
install -m 0755 "${SOURCE_BINARY}" "${MACOS_DIR}/trueos-rdp-bin"
install -m 0644 "${REPO_DIR}/packaging/macos/Info.plist" \
    "${CONTENTS_DIR}/Info.plist"
install -m 0644 "${REPO_DIR}/packaging/macos/README.txt" \
    "${RESOURCES_DIR}/README.txt"

rm -f -- "${ARCHIVE}"
(
    cd "${DIST_DIR}"
    zip --quiet --recurse-paths "$(basename "${ARCHIVE}")" \
        "$(basename "${APP_DIR}")"
)

printf 'Unsigned cross-built application: %s\n' "${APP_DIR}"
printf 'Bundle: %s\n' "${ARCHIVE}"
printf '%s\n' \
    'Run tools/build-macos.sh on macOS for Apple code signing and release packaging.'
sha256sum "${ARCHIVE}"
