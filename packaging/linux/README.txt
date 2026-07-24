TRUEOS-RDP portable Linux bundle
================================

Run:

    ./run.sh 192.168.178.94

Or use the default TRUEOS host:

    ./run.sh

The executable targets x86_64 Linux with glibc 2.35 or newer. It deliberately
uses the destination system's Vulkan loader and Intel/other graphics driver;
GPU drivers cannot be safely frozen into an application-level glibc bundle.

Runtime requirements:

  - x86_64 Linux with glibc 2.35 or newer
  - a working Vulkan graphics stack
  - ffmpeg available in PATH for low-latency H.264 decoding (view mode only)

On Ubuntu, ffmpeg can be installed with:

    sudo apt install ffmpeg

Input-only mode remains available:

    ./run.sh 192.168.178.94 --no-view
