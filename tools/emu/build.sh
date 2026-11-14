#!/usr/bin/env bash
# Builds the headless libretro frontend used to run examples in Azahar.
#   AZAHAR_SRC: an Azahar checkout (for libretro.h), default ../emu/azahar
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
src="${AZAHAR_SRC:-$here/../../../emu/azahar}"
inc="$src/externals/libretro-common/libretro-common/include"
[ -f "$inc/libretro.h" ] || { echo "libretro.h not found under $inc (set AZAHAR_SRC)" >&2; exit 1; }
mkdir -p "$here/../../target"
out="$here/../../target"
cc -O2 -Wall -Wextra -Wno-unused-parameter -I"$inc" -c "$here/run.c" -o "$out/emu-run.o"
c++ -O2 -Wall -std=c++17 -c "$here/install.cpp" -o "$out/emu-install.o"
c++ "$out/emu-run.o" "$out/emu-install.o" -ldl -o "$out/emu-run"
echo "built target/emu-run"
