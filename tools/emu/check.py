#!/usr/bin/env python3
"""Runs the examples in Azahar (headless) and checks what they log.

Each example declares its check in Cargo.toml:

    [package.metadata.citronnade.emulator]
    frames = 90                       # frames to run
    expect = ["Hello", "goodbye"]     # log lines containing these, in order
    reject = ["panicked"]             # log lines that must not appear
    exits = true                      # the program must exit by itself
    shots = [30]                      # frames to save as PNG in target/emu-shots
    sd_files = { "dir/file.txt" = "contents" }    # files put on the virtual SD card first
    audio_peak_min = 2000             # the program must make sound at least this loud
    input = ["10-20:a", "touch 30-40:160,120"]   # see tools/emu/run.c
    host_servers = true               # start tools/net/hostservers.py's servers on 127.0.0.1
    host_expect = ["hello"]           # text the stdio sink (TCP 7779) must receive, in order

Usage: tools/emu/check.py [example-name ...] [--release] [--no-build] [--cia]
Needs target/emu-run (tools/emu/build.sh) and AZAHAR_LIBRETRO.

With --cia, each example is packaged as a CIA too, installed with Azahar's
own installer, and the installed title is checked instead of the 3DSX.
"""

import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "net"))
import hostservers  # noqa: E402

_servers = None


def start_host_servers():
    """Starts the network examples' PC-side servers once."""
    global _servers
    if _servers is None:
        hostservers.tcp_echo("127.0.0.1")
        hostservers.udp_echo("127.0.0.1")
        _, received = hostservers.tcp_sink("127.0.0.1")
        hostservers.http_server("127.0.0.1")
        _servers = received
    return _servers
EMU = ROOT / "target" / "emu-run"
SHOTS = ROOT / "target" / "emu-shots"
LOG_LINE = re.compile(r"Debug\.Emulated <Debug> [^:]+:OutputDebugString:\d+: (.*)$")


def examples():
    for manifest in sorted((ROOT / "examples").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text())
        pkg = data["package"]
        meta = pkg.get("metadata", {}).get("citronnade", {})
        yield manifest.parent, pkg["name"], meta


def install_cia(name, profile):
    """Installs the example's CIA in the emulator; returns the installed
    content's path, or an error."""
    out = ROOT / "target" / "armv6k-nintendo-3ds" / profile
    cia = (out / f"{name}.cia").read_bytes()
    # the TMD's title ID: header, certificate chain and ticket, each on 0x40
    align = lambda x: (x + 0x3F) & ~0x3F
    sizes = [int.from_bytes(cia[o:o + 4], "little") for o in (0, 8, 12)]
    tmd = align(align(align(sizes[0]) + sizes[1]) + sizes[2])
    title = cia[tmd + 0x140 + 0x4C:tmd + 0x140 + 0x54].hex()
    proc = subprocess.run([str(EMU), "--frames", "2", "--install", f"{out / name}.cia:{title}", str(out / f"{name}.cxi")],
                          capture_output=True, text=True, timeout=600)
    found = re.search(r"\[harness\] content 0 of \w+: (.*)", proc.stdout)
    if not found:
        return None, "the CIA did not install: " + " ".join(l for l in proc.stdout.splitlines() if "[harness]" in l)
    return found.group(1), None


def run_example(name, meta, profile, program=None):
    check = meta.get("emulator")
    if check is None:
        return None, "no emulator check declared"
    program = program or ROOT / "target" / "armv6k-nintendo-3ds" / profile / f"{name}.3dsx"
    cmd = [str(EMU), "--frames", str(check.get("frames", 90))]
    for item in check.get("input", []):
        kind, _, spec = item.rpartition(" ")
        cmd += [f"--{kind or 'press'}", spec]
    SHOTS.mkdir(parents=True, exist_ok=True)
    shots = []
    for frame in check.get("shots", []):
        ppm = SHOTS / f"{name}-{frame}.ppm"
        shots.append(ppm)
        cmd += ["--shot", f"{frame}:{ppm}"]
    for key, value in check.get("options", {}).items():
        cmd += ["--option", f"{key}={value}"]
    sd = ROOT / "target" / "emu-data" / "Azahar" / "sdmc"
    for rel, content in check.get("sd_files", {}).items():
        target = sd / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    received = None
    if check.get("host_servers"):
        received = start_host_servers()
        received.clear()
    cmd.append(str(program))
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    output = proc.stdout + proc.stderr
    lines = [m.group(1) for m in map(LOG_LINE.search, output.splitlines()) if m]
    (ROOT / "target" / "emu-logs").mkdir(parents=True, exist_ok=True)
    (ROOT / "target" / "emu-logs" / f"{name}.log").write_text(output)

    problems = []
    if proc.returncode != 0:
        problems.append(f"emu-run exited with {proc.returncode}")
    position = 0
    for want in check.get("expect", []):
        found = next((i for i in range(position, len(lines)) if want in lines[i]), None)
        if found is None:
            problems.append(f"missing (in order): {want!r}")
        else:
            position = found + 1
    if "host_expect" in check:
        text = b"".join(received or []).decode("utf-8", "replace")
        (ROOT / "target" / "emu-logs" / f"{name}.host.txt").write_text(text)
        position = 0
        for want in check["host_expect"]:
            found = text.find(want, position)
            if found < 0:
                problems.append(f"the host did not receive (in order): {want!r}")
            else:
                position = found + len(want)
    for bad in check.get("reject", []):
        if any(bad in line for line in lines):
            problems.append(f"unexpected: {bad!r}")
    peak = re.search(r"audio peak (\d+)", output)
    if "audio_peak_min" in check:
        level = int(peak.group(1)) if peak else 0
        if level < check["audio_peak_min"]:
            problems.append(f"audio peak {level} below {check['audio_peak_min']}")
    exited = "exiting" in output and "TerminateProcess" in output
    if check.get("exits", False) and not exited:
        problems.append("the program did not exit")
    for ppm in shots:
        if ppm.exists():
            png = ppm.with_suffix(".png")
            subprocess.run(["convert", str(ppm), str(png)], check=False)
            if png.exists():
                ppm.unlink()
        else:
            problems.append(f"no screenshot {ppm.name}")
    return lines, problems


def main():
    args = sys.argv[1:]
    profile = "release" if "--release" in args else "debug"
    build = "--no-build" not in args
    cia = "--cia" in args
    wanted = [a for a in args if not a.startswith("--")]
    if not EMU.exists():
        sys.exit("target/emu-run is missing: run tools/emu/build.sh")
    if "AZAHAR_LIBRETRO" not in os.environ:
        sys.exit("set AZAHAR_LIBRETRO to the Azahar libretro core")

    selected = [(d, n, m) for d, n, m in examples() if not wanted or n in wanted or d.name in wanted]
    if build:
        cmd = ["cargo", "run", "-q", "-p", "cargo-citronnade", "--", "build"]
        if profile == "release":
            cmd.append("--release")
        if cia:
            cmd.append("--cia")
        for _, name, _ in selected:
            cmd += ["-p", name]
        subprocess.run(cmd, cwd=ROOT, check=True)

    failures = 0
    for directory, name, meta in selected:
        program = None
        if cia and meta.get("emulator") is not None:
            program, error = install_cia(name, profile)
            if error:
                failures += 1
                print(f"FAIL {directory.name}\n     {error}")
                continue
        lines, problems = run_example(name, meta, profile, program)
        if lines is None:
            print(f"SKIP {directory.name}: {problems}")
            continue
        if problems:
            failures += 1
            print(f"FAIL {directory.name}")
            for p in problems:
                print(f"     {p}")
            for line in lines[-15:]:
                print(f"     | {line}")
        else:
            print(f"ok   {directory.name} ({len(lines)} log lines)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
