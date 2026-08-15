#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly ARM_TARGET="aarch64-apple-darwin"
readonly INTEL_TARGET="x86_64-apple-darwin"
readonly DEPLOYMENT_TARGET="11.0"
readonly APP_NAME="TRUEOS-RDP.app"
readonly ARCHIVE_NAME="trueos-rdp-macos-universal.zip"
readonly DIST_DIR="${REPO_DIR}/dist"
readonly APP_DIR="${DIST_DIR}/${APP_NAME}"
readonly CONTENTS_DIR="${APP_DIR}/Contents"
readonly MACOS_DIR="${CONTENTS_DIR}/MacOS"
readonly RESOURCES_DIR="${CONTENTS_DIR}/Resources"
readonly ARM_BINARY="${REPO_DIR}/target/${ARM_TARGET}/release/trueos-rdp"
readonly INTEL_BINARY="${REPO_DIR}/target/${INTEL_TARGET}/release/trueos-rdp"
readonly UNIVERSAL_BINARY="${MACOS_DIR}/trueos-rdp-bin"

if [[ "$(uname -s)" != "Darwin" ]]; then
    printf '%s\n' \
        'The final macOS application must be linked on macOS with Apple'\''s SDK.' \
        'Run this script on a Mac with Xcode Command Line Tools installed.' >&2
    exit 1
fi

for command_name in cargo codesign ditto lipo rustup shasum xcrun; do
    if ! command -v "${command_name}" >/dev/null 2>&1; then
        printf 'Missing required macOS build command: %s\n' "${command_name}" >&2
        exit 1
    fi
done

if ! xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1; then
    printf '%s\n' \
        'No macOS SDK was found. Install Xcode Command Line Tools with:' \
        '    xcode-select --install' >&2
    exit 1
fi

rustup target add "${ARM_TARGET}" "${INTEL_TARGET}"

(
    cd "${REPO_DIR}"
    MACOSX_DEPLOYMENT_TARGET="${DEPLOYMENT_TARGET}" \
    cargo build --release --target "${ARM_TARGET}"
    MACOSX_DEPLOYMENT_TARGET="${DEPLOYMENT_TARGET}" \
        cargo build --release --target "${INTEL_TARGET}"
)

install -d "${DIST_DIR}"
rm -rf -- "${APP_DIR}"
install -d "${MACOS_DIR}" "${RESOURCES_DIR}"
install -m 0755 "${REPO_DIR}/packaging/macos/launcher.sh" \
    "${MACOS_DIR}/trueos-rdp"
install -m 0644 "${REPO_DIR}/packaging/macos/Info.plist" \
    "${CONTENTS_DIR}/Info.plist"
install -m 0644 "${REPO_DIR}/packaging/macos/README.txt" \
    "${RESOURCES_DIR}/README.txt"

lipo -create \
    "${ARM_BINARY}" \
    "${INTEL_BINARY}" \
    -output "${UNIVERSAL_BINARY}"
lipo -verify_arch arm64 x86_64 "${UNIVERSAL_BINARY}"
chmod 0755 "${UNIVERSAL_BINARY}"

readonly CODESIGN_IDENTITY="${TRUEOS_RDP_CODESIGN_IDENTITY:--}"
if [[ "${CODESIGN_IDENTITY}" == "-" ]]; then
    codesign --force --deep --sign - "${APP_DIR}"
else
    codesign --force --deep --options runtime --timestamp \
        --sign "${CODESIGN_IDENTITY}" "${APP_DIR}"
fi
codesign --verify --deep --strict --verbose=2 "${APP_DIR}"

rm -f -- "${DIST_DIR}/${ARCHIVE_NAME}"
ditto -c -k --sequesterRsrc --keepParent \
    "${APP_DIR}" "${DIST_DIR}/${ARCHIVE_NAME}"

printf 'Application: %s\n' "${APP_DIR}"
printf 'Bundle: %s\n' "${DIST_DIR}/${ARCHIVE_NAME}"
lipo -archs "${UNIVERSAL_BINARY}"
shasum -a 256 "${DIST_DIR}/${ARCHIVE_NAME}"
