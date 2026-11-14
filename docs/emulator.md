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
to run some of them, and `--release` for optimised builds.

To run a program by hand:

```bash
target/emu-run --frames 300 --press 60-70:a --shot 120:shot.ppm program.3dsx
```
