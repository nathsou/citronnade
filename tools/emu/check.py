#!/usr/bin/env python3
"""Runs the examples in Azahar (headless) and checks what they log.

Each example declares its check in Cargo.toml:

    [package.metadata.citronnade.emulator]
    frames = 90                       # frames to run
    expect = ["Hello", "goodbye"]     # log lines containing these, in order
    reject = ["panicked"]             # log lines that must not appear
    exits = true                      # the program must exit by itself
    shots = [30]                      # frames to save as PNG in target/emu-shots
    input = ["10-20:a", "touch 30-40:160,120"]   # see tools/emu/run.c

Usage: tools/emu/check.py [example-name ...] [--release] [--no-build]
Needs target/emu-run (tools/emu/build.sh) and AZAHAR_LIBRETRO.
"""

import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EMU = ROOT / "target" / "emu-run"
SHOTS = ROOT / "target" / "emu-shots"
LOG_LINE = re.compile(r"Debug\.Emulated <Debug> [^:]+:OutputDebugString:\d+: (.*)$")


def examples():
    for manifest in sorted((ROOT / "examples").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text())
        pkg = data["package"]
        meta = pkg.get("metadata", {}).get("citronnade", {})
        yield manifest.parent, pkg["name"], meta


def run_example(name, meta, profile):
    check = meta.get("emulator")
    if check is None:
        return None, "no emulator check declared"
    program = ROOT / "target" / "armv6k-nintendo-3ds" / profile / f"{name}.3dsx"
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
    for bad in check.get("reject", []):
        if any(bad in line for line in lines):
            problems.append(f"unexpected: {bad!r}")
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
        for _, name, _ in selected:
            cmd += ["-p", name]
        subprocess.run(cmd, cwd=ROOT, check=True)

    failures = 0
    for directory, name, meta in selected:
        lines, problems = run_example(name, meta, profile)
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
