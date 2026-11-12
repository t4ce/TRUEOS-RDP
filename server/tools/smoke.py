#!/usr/bin/env python3
"""Exercise real FFmpeg -> UDP -> reassembly -> FFmpeg, on loopback only."""
import argparse
import json
import pathlib
import socket
import struct
import subprocess
import tempfile
import time
import zlib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=pathlib.Path,
                        default=pathlib.Path(__file__).resolve().parents[1] / "target/release/trueos-rdp-server")
    args = parser.parse_args()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    address = ("127.0.0.1", port)
    with tempfile.TemporaryFile() as log:
        server = subprocess.Popen([str(args.server), "--test-source", "--bind", f"127.0.0.1:{port}"],
                                  stdout=subprocess.DEVNULL, stderr=log)
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client, \
                    socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as competitor:
                client.bind(("127.0.0.1", 0))
                client.settimeout(0.2)
                competitor.bind(("127.0.0.1", 0))
                competitor.setblocking(False)
                deadline = time.monotonic() + 5
                next_subscribe = 0
                info = None
                units = {}
                datagrams = 0
                first_video = None
                while time.monotonic() < deadline:
                    now = time.monotonic()
                    if now >= next_subscribe:
                        client.sendto(b"TME1GET1", address)
                        # Competing subscribers may not replace the active peer.
                        if info is not None:
                            competitor.sendto(b"TME1GET1", address)
                        next_subscribe = now + 0.5
                    if server.poll() is not None:
                        raise RuntimeError(f"server exited with {server.returncode}")
                    try:
                        packet, peer = client.recvfrom(65536)
                    except socket.timeout:
                        continue
                    assert peer == address
                    if packet.startswith(b"TME1INFO"):
                        assert len(packet) == 20
                        values = struct.unpack("!IHHHH", packet[8:])
                        assert values[1:] == (1280, 720, 30, 0), values
                        info = values
                        continue
                    assert info is not None, "video arrived before stream metadata"
                    assert 32 < len(packet) <= 1200
                    (magic, version, flags, header_len, session, datagram_seq,
                     unit_seq, index, count, length, reserved, crc) = struct.unpack("!4sBBHIIIHHHHI", packet[:32])
                    assert (magic, version, header_len, session, reserved) == (b"TME1", 1, 32, info[0], 0)
                    assert length == len(packet) - 32
                    assert zlib.crc32(packet[32:]) == crc
                    assert bool(flags & 1) == (index == 0)
                    assert bool(flags & 2) == (index + 1 == count)
                    unit = units.setdefault(unit_seq, {"count": count, "parts": {}, "flags": flags})
                    assert count == unit["count"]
                    unit["parts"][index] = packet[32:]
                    datagrams += 1
                    first_video = first_video or now
                    if now - first_video >= 2.5:
                        break
                assert info is not None and first_video is not None, "no stream within five seconds"
                try:
                    competitor.recvfrom(65536)
                    raise AssertionError("a second subscriber received data before expiration")
                except BlockingIOError:
                    pass
                assert 0 in units and units[0]["flags"] & 4, "stream did not start on a keyframe"
                encoded = bytearray()
                completed = 0
                for sequence in range(max(units) + 1):
                    unit = units[sequence]
                    if len(unit["parts"]) != unit["count"]:
                        assert sequence == max(units), "incomplete AU before final receive boundary"
                        break
                    encoded.extend(b"".join(unit["parts"][i] for i in range(unit["count"])))
                    completed += 1
                decoded = subprocess.run([
                    "ffprobe", "-v", "error", "-f", "h264", "-count_frames",
                    "-show_entries", "stream=width,height,nb_read_frames", "-of", "json", "pipe:0"
                ], input=encoded, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
                stream = json.loads(decoded.stdout)["streams"][0]
                frames = int(stream["nb_read_frames"])
                assert (stream["width"], stream["height"]) == (1280, 720)
                assert frames >= 30, stream
                assert not decoded.stderr, decoded.stderr.decode()

                # After keepalives stop, the waiting second peer gets a new session.
                time.sleep(3.2)
                competitor.settimeout(2)
                competitor.sendto(b"TME1GET1", address)
                second, _ = competitor.recvfrom(65536)
                assert second.startswith(b"TME1INFO")
                assert struct.unpack("!I", second[8:12])[0] != info[0]
                print(f"PASS: {datagrams} CRC-checked datagrams, {completed} complete AUs, "
                      f"{frames} decoded 1280x720 frames; subscriber ownership and expiry verified")
        except BaseException:
            log.seek(0)
            print(log.read().decode(errors="replace"))
            raise
        finally:
            server.terminate()
            try:
                server.wait(timeout=3)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()


if __name__ == "__main__":
    main()
