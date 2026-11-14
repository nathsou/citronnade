# Citronnade

A Nintendo 3DS homebrew toolchain and library in Rust, with no dependency on
devkitPro, a C toolchain or a C library. Builds need only rustup (nightly
with `rust-src`) and `cargo-citronnade`.

Status: every release in [PLAN.md](PLAN.md), up to 1.0, is implemented:
45 examples, each checked headless in the Azahar emulator. Testing on
consoles is still to come. [docs/guide.md](docs/guide.md) tours the
library.

## Getting started

```bash
cargo install --git https://github.com/nathsou/citronnade cargo-citronnade
```

```bash
cargo citronnade new hello
```

```bash
cd hello && cargo citronnade build --release
```

This produces `target/armv6k-nintendo-3ds/release/hello.3dsx`. To send it to
a console running the Homebrew Launcher (press Y there to start receiving):

```bash
cargo citronnade run --release
```

A program looks like this:

```rust
#![no_std]
#![no_main]

use citronnade::prelude::*;

#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens()?;
    let mut input = hid.input()?;
    while apt.main_loop() {
        if input.scan().down().contains(Buttons::START) {
            break;
        }
        let mut top = screens.top();
        top.clear(Color::hex(0x204080));
        top.text(10, 10, "Hello from the 3DS!", Color::WHITE, 2);
        screens.present();
    }
    Ok(())
}
```

Services are owned values: `System` is handed to `main` once, and each part
is moved out of it.

`log!` writes to the debugger: Luma3DS's GDB stub on hardware, or the
emulator's log.

## How it works

- Rust's built-in `armv6k-nintendo-3ds` target, with `core` and `alloc`
  built from source (`-Zbuild-std`) and linked by the `rust-lld` that ships
  with Rust, using Citronnade's own linker script and startup code.
- `citronnade-rt`: startup, heaps and allocators, system calls, threads,
  arbiter-based locks and events, logging, panics and exit.
- `citronnade-services`: IPC requests and the system services (srv, APT,
  GSP, HID, ...), following libctru's behaviour.
- `citronnade`: the API programs use: `System`, `Screens` and `Canvas`,
  `Input`, `Console`, the panic screen, `math`, `gpu`, `audio`, `fs` and
  `romfs`, `net`, `applets` (software keyboard, error display) and
  `camera`.
- `citronnade-gpu`: the PICA200: command lists, GX transfers through GSP,
  VRAM, render targets (also as textures), vertex and geometry shaders,
  vertex buffers, textures, texture combiners, fragment lighting, fog, the
  procedural texture unit, blending, depth and stencil, meshes, and the
  maths for them (matrices, quaternions, stereoscopic projections). State
  handling follows citro3d.
- `citronnade-shader`: an assembler for PICA200 shaders with picasso's
  syntax, writing SHBIN files; `gpu::shader!` runs it at compile time and
  reports errors at the shader's line.
- `citronnade-assets`: build-script conversions: PNG images to tiled
  textures (`.ctex`), OBJ models to meshes (`.cmsh`).
- `citronnade-formats`: ELF to 3DSX conversion, SMDH, zlib and PNG, shared
  by the host tool and programs.
- `cargo-citronnade`: builds, packages each binary as a `.3dsx` with its
  SMDH (title, description, author and icon from
  `[package.metadata.citronnade]`) and RomFS (the `romfs` directory named
  there), and uploads it with the 3dslink protocol.

## Examples

| # | Example | Since | Exercises |
|---|---|---|---|
| 00 | [hello](examples/00-hello) | 0.1 | startup, 3DSX/SMDH, `log!`, arguments, a clean exit |
| 01 | [panic](examples/01-panic) | 0.1 | the panic handler and exit hooks |
| 02 | [heap](examples/02-heap) | 0.1 | `alloc` collections, the linear heap, allocation failure |
| 03 | [colors](examples/03-colors) | 0.2 | both screens, every framebuffer format, double buffering, VBlank |
| 04 | [console](examples/04-console) | 0.2 | the text console, `println!`, scrolling, the panic screen |
| 05 | [buttons](examples/05-buttons) | 0.2 | buttons, the Circle Pad, presses and releases |
| 06 | [lifecycle](examples/06-lifecycle) | 0.2 | HOME, sleep, the power button and exit (APT) |
| 07 | [canvas-2d](examples/07-canvas-2d) | 0.2 | software 2D: sprites, alpha blending, frame timing |
| 08 | [touch-paint](examples/08-touch-paint) | 0.3 | the touch screen, off-screen surfaces, thick lines |
| 09 | [stereo-slider](examples/09-stereo-slider) | 0.3 | the stereoscopic top screen and the 3D slider |
| 10 | [wide-mode](examples/10-wide-mode) | 0.3 | the 800x240 wide top screen, model detection |
| 11 | [motion](examples/11-motion) | 0.3 | the accelerometer and gyroscope |
| 12 | [new3ds-inputs](examples/12-new3ds-inputs) | 0.3 | the C-Stick and ZL/ZR (ir:rst) |
| 13 | [system-info](examples/13-system-info) | 0.3 | model, region, language, power, volume, Wi-Fi, versions, the clock |
| 14 | [sd-browser](examples/14-sd-browser) | 0.4 | the SD card: directories, reading, writing, appending |
| 15 | [romfs-assets](examples/15-romfs-assets) | 0.4 | RomFS packed into the 3DSX, PNG decoding on the console |
| 16 | [threads](examples/16-threads) | 0.4 | threads on both cores, `Mutex`, `Event` |
| 17 | [timing](examples/17-timing) | 0.4 | the system tick, `Instant`, sleep accuracy, frame pacing |
| 18 | [tone](examples/18-tone) | 0.5 | the DSP: a synthesiser streaming wave buffers |
| 19 | [samples](examples/19-samples) | 0.5 | WAV files from the RomFS on several channels, panning, looping |
| 20 | [touch-synth](examples/20-touch-synth) | 0.5 | polyphony with looping single-cycle waves, latency |
| 21 | [microphone](examples/21-microphone) | 0.5 | recording from the microphone and playing it back |
| 22 | [gpu-clear](examples/22-gpu-clear) | 0.6 | render targets in VRAM, memory fills, display transfers |
| 23 | [triangle](examples/23-triangle) | 0.6 | a vertex shader through `shader!`, vertex buffers, uniforms |
| 24 | [textured-quad](examples/24-textured-quad) | 0.6 | textures: tiling, orientation, filtering, wrap modes, formats |
| 25 | [texenv](examples/25-texenv) | 0.6 | texture combiners: multitexturing, interpolation, dot3, alpha test |
| 26 | [sprites](examples/26-sprites) | 0.6 | thousands of batched sprites, indexed drawing, blending |
| 27 | [gpu-text](examples/27-gpu-text) | 0.6 | text from a font atlas on both screens |
| 28 | [cube](examples/28-cube) | 0.7 | perspective, the depth buffer, culling, an orbit camera |
| 29 | [stereo-3d](examples/29-stereo-3d) | 0.7 | stereoscopic rendering, slider-driven eye separation, the convergence plane |
| 30 | [lighting](examples/30-lighting) | 0.7 | fragment lighting: directional, spot and point lights, a specular LUT drawn on the touch screen |
| 31 | [geometry-shader](examples/31-geometry-shader) | 0.7 | points expanded into billboards by a geometry shader |
| 32 | [fog](examples/32-fog) | 0.7 | fog and depth cueing |
| 33 | [render-to-texture](examples/33-render-to-texture) | 0.7 | an offscreen target as a texture: a live minimap |
| 34 | [procedural-texture](examples/34-procedural-texture) | 0.7 | the procedural texture unit: rings, marble, bricks, clouds |
| 35 | [stencil](examples/35-stencil) | 0.7 | stencil reflections and outlines |
| 36 | [model](examples/36-model) | 0.7 | an original model converted at build time, loaded from the RomFS |
| 37 | [wifi-status](examples/37-wifi-status) | 0.8 | the Wi-Fi connection, signal strength, the console's address |
| 38 | [sockets](examples/38-sockets) | 0.8 | TCP echo and UDP with a PC, non-blocking sockets, broadcast |
| 39 | [stdio](examples/39-stdio) | 0.8 | printing to the PC's terminal; the basis of `cargo citronnade test` |
| 40 | [http](examples/40-http) | 0.8 | HTTP GET and POST through the system, a JSON response |
| 41 | [software-keyboard](examples/41-software-keyboard) | 0.9 | text input through the system keyboard |
| 42 | [error-applet](examples/42-error-applet) | 0.9 | system error messages |
| 43 | [camera](examples/43-camera) | 0.9 | live preview, the outer cameras as a stereo pair, hardware YUV conversion |
| 44 | [showcase](examples/44-showcase) | 1.0 | Lemon Drop, a small original game: stereo 3D, touch, sound, high scores on the SD card |

Audio needs the console's DSP firmware, which is not distributed: dump your
own with the DSP1 homebrew to `/3ds/dspfirm.cdc` on the SD card.

Build one with `cargo citronnade build -p hello` from the repository root.

Azahar's software renderer, which the headless checks use, ignores texture
filtering (it always samples the nearest texel), so example 24's linear
filtering only shows on hardware or with a hardware renderer.

## Testing

Host tests: `cargo test`. The examples run headless in Azahar's libretro
core and check their own output: see [docs/emulator.md](docs/emulator.md).

On a console, with the Homebrew Launcher's netloader open (Y):

- `cargo citronnade run --server -p hello` sends the program and shows its
  `println!` and `log!` output in the terminal until it exits;
- `cargo citronnade test` builds the workspace's programs, sends each one
  that has a console test and checks its output: the lines listed in
  `[package.metadata.citronnade.test]` (`expect`, `reject`, `timeout` in
  seconds), or, by default, the emulator check of examples that need no
  input. Reopen the netloader between programs if your launcher version
  does not return to it by itself.

The network examples talk to `python3 tools/net/hostservers.py` on the PC;
write the PC's address into `sd:/citronnade/net.txt`.

## License

MIT or Apache-2.0, at your option. See [NOTICE.md](NOTICE.md) for the
libraries whose behaviour Citronnade follows (libctru, citro3d, picasso).
