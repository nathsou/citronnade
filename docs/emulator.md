# Running the examples in an emulator

The examples are checked headlessly in [Azahar](https://github.com/azahar-emu/azahar)'s
libretro core, driven by a small frontend in `tools/emu/run.c`. It steps a
fixed number of frames, scripts buttons, touch and the Circle Pad, saves
screenshots and prints the program's `log!` output.

Azahar emulates most system services at a high level, so passing here does
not prove a program works on a console: hardware remains the reference.

## Building the core

```bash
git clone --depth 1 --branch 2126.2 https://github.com/azahar-emu/azahar
```

```bash
cd azahar && git submodule update --init --recursive --depth 1
```

```bash
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DENABLE_LIBRETRO=ON -DENABLE_QT=OFF -DENABLE_SDL2=OFF -DENABLE_TESTS=OFF -DENABLE_ROOM=OFF -DENABLE_WEB_SERVICE=OFF -DENABLE_SCRIPTING=OFF -DENABLE_CUBEB=OFF -DENABLE_OPENAL=OFF -DENABLE_LIBUSB=OFF -DENABLE_VULKAN=OFF -DENABLE_OPENGL=OFF -DCITRA_WARNINGS_AS_ERRORS=OFF
```

```bash
ninja -C build azahar_libretro
```

The core is `build/bin/Release/azahar_libretro.so`.

## Building the frontend and running the checks

```bash
AZAHAR_SRC=/path/to/azahar tools/emu/build.sh
```

```bash
AZAHAR_LIBRETRO=/path/to/azahar_libretro.so tools/emu/check.py
```

`check.py` builds the examples, runs each one and compares its log with the
`[package.metadata.citronnade.emulator]` table in its `Cargo.toml`. Logs go
to `target/emu-logs/`, screenshots to `target/emu-shots/`. Pass example names
to run some of them, and `--release` for optimised builds. With `--cia`,
each example is also packaged as a CIA, installed with Azahar's own
installer (`emu-run --install FILE.cia:TITLEID` calls it once the core
runs, and prints where the title's content went), and the installed title
is checked instead of the 3DSX.

To run a program by hand:

```bash
target/emu-run --frames 300 --press 60-70:a --shot 120:shot.ppm program.3dsx
```

It runs `.cxi` files (installable titles' NCCHs) too.

## Network examples

`host_servers = true` in an example's emulator metadata starts the PC-side
servers of `tools/net/hostservers.py` on 127.0.0.1 before the run (TCP echo
on 7777, UDP echo on 7778, an output sink on 7779, HTTP on 7780). Azahar's
socket and HTTP services use the host's network, so the program reaches
them at 127.0.0.1 (the examples read the address from
`sd:/citronnade/net.txt`, which `sd_files` provides). `host_expect` lists
text the output sink must receive, in order, for examples that print to the
PC.

Azahar's sockets are the host's, so a blocking call stalls the emulated
console until it returns; the examples use non-blocking sockets in their
frame loops.
