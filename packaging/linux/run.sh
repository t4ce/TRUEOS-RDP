#!/bin/sh
set -eu

bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

needs_ffmpeg=true
for argument in "$@"; do
    case "$argument" in
        -[Nn][Oo][Vv][Ii][Ee][Ww] | \
            --[Nn][Oo][Vv][Ii][Ee][Ww] | \
            --[Nn][Oo]-[Vv][Ii][Ee][Ww] | \
            -h | --help)
            needs_ffmpeg=false
            ;;
    esac
done

if "${needs_ffmpeg}" && ! command -v ffmpeg >/dev/null 2>&1; then
    printf '%s\n' \
        'TRUEOS-RDP needs ffmpeg for low-latency H.264 decoding.' \
        'Install it with: sudo apt install ffmpeg' >&2
    exit 127
fi

exec "${bundle_dir}/trueos-rdp" "$@"
