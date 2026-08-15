#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_DIR="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"

readonly ZIG_VERSION="0.15.2"
readonly ZIG_SHA256="02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239"
readonly CARGO_ZIGBUILD_VERSION="0.23.0"
readonly RUST_TARGET="x86_64-unknown-linux-gnu"
readonly GLIBC_FLOOR="2.35"
readonly BUILD_TARGET="${RUST_TARGET}.${GLIBC_FLOOR}"
readonly BUNDLE_NAME="trueos-rdp-linux-x86_64-glibc${GLIBC_FLOOR}"

readonly TOOL_DIR="${TRUEOS_RDP_TOOL_DIR:-${REPO_DIR}/.codex_tmp/portable-tools}"
readonly ZIG_DIR="${TOOL_DIR}/zig-${ZIG_VERSION}"
readonly ZIG_ARCHIVE="${TOOL_DIR}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz"
readonly CARGO_ZIGBUILD_DIR="${TOOL_DIR}/cargo-zigbuild-${CARGO_ZIGBUILD_VERSION}"
readonly DIST_DIR="${REPO_DIR}/dist"
readonly BUNDLE_DIR="${DIST_DIR}/${BUNDLE_NAME}"
readonly BUNDLE_ARCHIVE="${DIST_DIR}/${BUNDLE_NAME}.tar.xz"
readonly RELEASE_BINARY="${REPO_DIR}/target/${RUST_TARGET}/release/trueos-rdp"

for command_name in cargo curl readelf rustup sha256sum tar; do
    if ! command -v "${command_name}" >/dev/null 2>&1; then
        printf 'Missing required build command: %s\n' "${command_name}" >&2
        exit 1
    fi
done

install -d "${TOOL_DIR}" "${DIST_DIR}"

if [[ ! -x "${ZIG_DIR}/zig" ]]; then
    if [[ ! -f "${ZIG_ARCHIVE}" ]]; then
        curl --fail --location --output "${ZIG_ARCHIVE}" \
            "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz"
    fi

    printf '%s  %s\n' "${ZIG_SHA256}" "${ZIG_ARCHIVE}" | sha256sum --check

    readonly ZIG_EXTRACT_DIR="${TOOL_DIR}/zig-x86_64-linux-${ZIG_VERSION}"
    rm -rf -- "${ZIG_EXTRACT_DIR}"
    tar --extract --xz --file "${ZIG_ARCHIVE}" --directory "${TOOL_DIR}"
    mv -- "${ZIG_EXTRACT_DIR}" "${ZIG_DIR}"
fi

if [[ ! -x "${CARGO_ZIGBUILD_DIR}/bin/cargo-zigbuild" ]]; then
    cargo install --locked \
        --version "${CARGO_ZIGBUILD_VERSION}" \
        --root "${CARGO_ZIGBUILD_DIR}" \
        cargo-zigbuild
fi

if ! rustup target list --installed | grep -Fxq "${RUST_TARGET}"; then
    rustup target add "${RUST_TARGET}"
fi

(
    cd "${REPO_DIR}"
    PATH="${CARGO_ZIGBUILD_DIR}/bin:${PATH}" \
    CARGO_ZIGBUILD_ZIG_PATH="${ZIG_DIR}/zig" \
    cargo zigbuild --release --target "${BUILD_TARGET}"
)

if [[ ! -x "${RELEASE_BINARY}" ]]; then
    printf 'Portable build did not produce %s\n' "${RELEASE_BINARY}" >&2
    exit 1
fi

readonly REQUIRED_GLIBC="$(
    LANG=C readelf -W --version-info --dyn-syms "${RELEASE_BINARY}" |
        sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\) .*/\1/p' |
        sort --version-sort |
        tail -n 1
)"

if [[ -z "${REQUIRED_GLIBC}" ]]; then
    printf 'Could not determine the executable glibc requirement\n' >&2
    exit 1
fi

if [[ "$(printf '%s\n%s\n' "${GLIBC_FLOOR}" "${REQUIRED_GLIBC}" | sort --version-sort | tail -n 1)" != "${GLIBC_FLOOR}" ]]; then
    printf 'Executable unexpectedly requires glibc %s (requested floor: %s)\n' \
        "${REQUIRED_GLIBC}" "${GLIBC_FLOOR}" >&2
    exit 1
fi

rm -rf -- "${BUNDLE_DIR}"
install -d "${BUNDLE_DIR}"
install -m 0755 "${RELEASE_BINARY}" "${BUNDLE_DIR}/trueos-rdp"
install -m 0755 "${REPO_DIR}/packaging/linux/run.sh" "${BUNDLE_DIR}/run.sh"
install -m 0644 "${REPO_DIR}/packaging/linux/README.txt" "${BUNDLE_DIR}/README.txt"

rm -f -- "${BUNDLE_ARCHIVE}"
tar --create --xz --file "${BUNDLE_ARCHIVE}" --directory "${DIST_DIR}" "${BUNDLE_NAME}"

printf 'Bundle: %s\n' "${BUNDLE_ARCHIVE}"
printf 'Minimum glibc found in executable: %s\n' "${REQUIRED_GLIBC}"
sha256sum "${BUNDLE_ARCHIVE}"
