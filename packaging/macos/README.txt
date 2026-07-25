TRUEOS-RDP for macOS
====================

This application contains both Apple Silicon and Intel executables and
requires macOS 11 or newer.

View mode requires ffmpeg. With Homebrew installed:

    brew install ffmpeg

The launcher recognizes both Apple Silicon Homebrew (/opt/homebrew/bin) and
Intel Homebrew (/usr/local/bin).

Double-click TRUEOS-RDP.app to open the autoconnect panel with the default
TRUEOS IP prefilled. Press Tab or click HOST to switch to trueos.eu. To prefill
a different host from Terminal:

    open TRUEOS-RDP.app --args 192.168.178.94

Input-only mode:

    open TRUEOS-RDP.app --args 192.168.178.94 --no-view

A native build is ad-hoc signed and may need to be opened once with Finder's
Control-click > Open command. A cross-built archive is unsigned; after
extracting it on a Mac, sign it locally before first use:

    codesign --force --deep --sign - TRUEOS-RDP.app

Distribution to other users requires signing with an Apple Developer ID and
notarization.
