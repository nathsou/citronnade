#!/usr/bin/env bash
# Builds the headless libretro frontend used to run examples in Azahar.
#   AZAHAR_SRC: an Azahar checkout (for libretro.h), default ../emu/azahar
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
src="${AZAHAR_SRC:-$here/../../../emu/azahar}"
inc="$src/externals/libretro-common/libretro-common/include"
[ -f "$inc/libretro.h" ] || { echo "libretro.h not found under $inc (set AZAHAR_SRC)" >&2; exit 1; }
mkdir -p "$here/../../target"
cc -O2 -Wall -Wextra -Wno-unused-parameter -I"$inc" "$here/run.c" -ldl -o "$here/../../target/emu-run"
echo "built target/emu-run"
