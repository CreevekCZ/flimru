#!/usr/bin/env python3
"""Measure Flimru's own CPU, wakeups and RSS on macOS with simulated Flutter.

Run: python3 tools/profile_performance.py target/release/flimru
CPU percentages use one core as 100%. No real Flutter SDK or project is used.
Each fixture owns a temporary directory, PTY and children and cleans them up.
"""

import argparse
import ctypes
import errno
import fcntl
import json
import math
import os
from pathlib import Path
import pty
import select
import signal
import struct
import sys
import tempfile
import termios
import time


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint), ("denom", ctypes.c_uint)]


class ProcessUsage:
    def __init__(self):
        self.libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        timebase = Timebase()
        result = ctypes.CDLL(None).mach_timebase_info(ctypes.byref(timebase))
        if result or not timebase.denom:
            raise RuntimeError("Could not read the Mach clock timebase")
        # libproc's CPU counters use Mach absolute ticks on this platform.
        self.nanoseconds_per_tick = timebase.numer / timebase.denom

    def read(self, pid):
        # rusage_info_v2 starts with a 16-byte UUID, followed by uint64 fields.
        buffer = ctypes.create_string_buffer(512)
        if self.libproc.proc_pid_rusage(pid, 2, buffer):
            raise OSError(ctypes.get_errno(), "proc_pid_rusage")
        values = struct.unpack_from("8Q", buffer.raw, 16)
        return (values[0] + values[1], values[2], values[3], values[6], values[7])


def fixture(project, mode):
    (project / ".vscode").mkdir()
    (project / ".vscode/launch.json").write_text(json.dumps({
        "configurations": [{"name": "Perf", "type": "dart", "request": "launch"}]
    }))
    devices = [dict(id=f"d{i}", name=f"Phone {i}", targetPlatform="ios",
                    isSupported=True) for i in range(8)]
    script = project / "flutter"
    setup = """#!/bin/bash
if [ "$1" = devices ]; then
cat <<'EOF'
""" + json.dumps(devices) + "\nEOF\nexit 0\nfi\nstty raw -echo\n"
    history = """for ((i=0;i<12000;i++)); do
  printf 'history %05d: console output for memory measurement\\r\\n' "$i"
done
""" if mode == "history" else ""
    output = """while :; do
  printf 'flutter: [Perf] INFO: steady console output for rendering measurement\\r\\n'
  sleep 0.03
done
""" if mode == "stream" else """while IFS= read -rsn1 key; do
  [ "$key" = q ] && exit 0
done
"""
    script.write_text(setup + history + "printf 'READY\\r\\n'\n" + output)
    script.chmod(0o755)
    return script


def run(binary, count, mode="idle", seconds=8):
    usage = ProcessUsage()
    with tempfile.TemporaryDirectory(prefix="flimru-profile-") as directory:
        project = Path(directory)
        script = fixture(project, mode)
        args = [str(binary), directory, "--flutter", str(script), "--state",
                str(project / "state.json"), "--refresh-interval", "3600"]
        if count > 1:
            args.append("--multi")
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.environ["TERM"] = "xterm-256color"
                os.execv(str(binary), args)
            except OSError:
                os._exit(127)
        output = bytearray()
        terminal_closed = False

        def drain(duration):
            nonlocal terminal_closed
            if terminal_closed:
                return
            deadline = time.monotonic() + duration
            while time.monotonic() < deadline:
                ready, _, _ = select.select(
                    [fd], [], [], min(0.05, max(0, deadline - time.monotonic()))
                )
                if not ready:
                    continue
                try:
                    chunk = os.read(fd, 65536)
                    if not chunk:
                        terminal_closed = True
                        return
                    output.extend(chunk)
                except OSError as error:
                    if error.errno == errno.EIO:
                        terminal_closed = True
                        return
                    if error.errno != errno.EAGAIN:
                        raise
                if len(output) > 2_000_000:
                    del output[:-100_000]

        try:
            os.set_blocking(fd, False)
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("4H", 50, 180, 0, 0))
            drain(0.8)
            deadline = time.monotonic() + 15
            while (b"Phone 0" not in output and not terminal_closed
                   and time.monotonic() < deadline):
                drain(0.2)
            if b"Phone 0" not in output:
                raise RuntimeError("Simulated device discovery did not finish")
            drain(0.2)
            if count:
                if count > 1:
                    for index in range(count):
                        os.write(fd, b" " + (b"\x1b[B" if index + 1 < count else b""))
                    drain(0.4)
                os.write(fd, b"\r")
                drain(0.3)
                os.write(fd, b"\r")
                drain(2)
                deadline = time.monotonic() + 15
                while (b"READY" not in output and not terminal_closed
                       and time.monotonic() < deadline):
                    drain(0.2)
                if b"READY" not in output:
                    raise RuntimeError("Simulated session did not become ready")
                drain(0.5)
            start = usage.read(pid)
            started = time.monotonic()
            output.clear()
            drain(seconds)
            if terminal_closed:
                raise RuntimeError("Launcher exited during measurement")
            wall = time.monotonic() - started
            end = usage.read(pid)
            result = dict(
                binary=Path(binary).name, sessions=count, mode=mode,
                cpu_percent=round((end[0] - start[0]) * usage.nanoseconds_per_tick
                                  / 1e9 / wall * 100, 3),
                idle_wakeups=end[1] - start[1],
                interrupt_wakeups=end[2] - start[2],
                rss_mib=round(end[3] / 2**20, 2),
                footprint_mib=round(end[4] / 2**20, 2),
                rss_growth_mib=round((end[3] - start[3]) / 2**20, 2),
                terminal_bytes=len(output),
            )
            print(json.dumps(result), flush=True)
            return result
        finally:
            try:
                os.write(fd, b"\x11")
                drain(0.2)
                os.write(fd, b"y")
                drain(0.6)
            except OSError:
                pass
            try:
                deadline = time.monotonic() + 5
                while True:
                    done, _ = os.waitpid(pid, os.WNOHANG)
                    if done:
                        break
                    if time.monotonic() > deadline:
                        # Hang up the PTY so Flimru can reap its own sessions.
                        os.close(fd)
                        fd = None
                        time.sleep(0.5)
                        done, _ = os.waitpid(pid, os.WNOHANG)
                        if not done:
                            os.kill(pid, signal.SIGKILL)
                            os.waitpid(pid, 0)
                        raise RuntimeError("Launcher shutdown timed out")
                    drain(0.05)
            finally:
                if fd is not None:
                    os.close(fd)


if __name__ == "__main__":
    if sys.platform != "darwin":
        raise SystemExit("This profiler requires macOS libproc")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda path: Path(path).resolve(strict=True))
    parser.add_argument("--seconds", type=float, default=8)
    options = parser.parse_args()
    if not math.isfinite(options.seconds) or options.seconds <= 0:
        parser.error("--seconds must be finite and positive")
    for count, mode in [(0, "idle"), (1, "idle"), (8, "idle"),
                        (1, "history"), (1, "stream")]:
        run(options.binary, count, mode, options.seconds)
