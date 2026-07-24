#!/bin/sh
set -eu

contents_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PATH="/opt/homebrew/bin:/usr/local/bin:${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}"
export PATH

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
    message='TRUEOS-RDP needs ffmpeg. Install it with “brew install ffmpeg”, then reopen the application.'
    printf '%s\n' "${message}" >&2
    if command -v osascript >/dev/null 2>&1; then
        osascript -e "display alert \"TRUEOS-RDP\" message \"${message}\" as critical"
    fi
    exit 127
fi

exec "${contents_dir}/MacOS/trueos-rdp-bin" "$@"
