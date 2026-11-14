# Citronnade: plan

Citronnade is a Nintendo 3DS homebrew toolchain and library written in Rust.
It doesn't depend on devkitPro, a C toolchain or a C library.

The project grows through **versioned examples**. Each release adds a set of
small programs that exercise one area of the toolchain and the hardware:

- screens and 2D graphics;
- input: buttons, touch, motion sensors, the 3D slider;
- files and threads;
- sound;
- GPU shaders and textures;
- 3D graphics and stereoscopy;
- network;
- system applets and the cameras.

The examples are the acceptance tests for their release, and they keep
building on every later version. The long-term goal is a stereoscopic port
of Super Mario Bros. on top of the recompiler in
[nathsou/smb](https://github.com/nathsou/smb). It comes last, outlined at the
end, and is the only part of the plan that lives in another repository.

## Goals

- **Install and build with only rustup.** You need nightly Rust with
  `rust-src` and `cargo install cargo-citronnade`. There is no pacman, no
  `DEVKITPRO` and no system packages.
- **A Rust-native API instead of a libctru clone.** Services are owned values
  instead of global init and exit calls, IPC commands are typed and declared
  in one place, and memory rules live in types. libctru's source (zlib
  licence) is the reference for how the console behaves, not for the API.
- **Each layer is useful on its own.** That means the packaging tools, the
  runtime, raw services and the high-level API.
- **Each 3DS feature gets at least one small example** that shows it working
  on hardware.

Not in scope for v1:
- `std`. Everything is `no_std` + `alloc`. A newlib-compatible shim could be
  added on top later.
- CIA packaging.
- Linking with C libraries built on libctru.

## What has been checked (2026-11-14)

| Check | Result |
|---|---|
| A `no_std` program for the built-in `armv6k-nintendo-3ds` target, linked by `rust-lld` with its own linker script and startup code, on a machine without devkitARM | Links. The output is a hard-float EABI5 ELF at `0x00100000` with relocations kept for 3DSX conversion. |
| A large C program linked in the same way: the SMB recompiler's output (about 22k lines) and its runtime, compiled by clang with no C library | Links into one ELF (137 KB code, about 810 KB RAM). The only unresolved symbols were `memcpy`, `memset` and `__aeabi_*` helpers, all of which Rust's `compiler_builtins` provides. |
| The SMB recompiler with the latest MoonBit (moonc 0.10.20) | Its full test suite passes, including the 7,987-frame trace. |
| Not yet checked | Running anything on a console or emulator, which needs the 3DSX writer from v0.1. Also the startup header field layout, which was written from memory and must be checked against devkitPro's `3dsx_crt0.s` (in the devkitARM specs repo, not libctru). |

## Toolchain decisions

- **Target:** use Rust's built-in `armv6k-nintendo-3ds` target, but override
  its linker. The flags below were checked:
  ```toml
  [target.armv6k-nintendo-3ds]
  linker = "rust-lld"
  rustflags = ["-C", "linker-flavor=ld.lld", "-C", "link-arg=-Tlink.ld", "-C", "link-arg=--emit-relocs"]
  ```
  A custom target JSON was rejected because its schema changes between
  nightlies. One catch: the built-in target sets `target_os = "horizon"` and
  `target_env = "newlib"`. Crates that key off those (`libc`, `getrandom`)
  assume newlib and won't work. That doesn't matter for `no_std`, but it
  should be documented.
- **`--emit-relocs` is required.** The 3DSX format carries its own
  relocation tables. `3dsxtool` builds them from the ELF's `SHT_REL`
  sections: `R_ARM_ABS32` and `R_ARM_TARGET1` become absolute relocations,
  and `R_ARM_REL32`, `R_ARM_TARGET2` and `R_ARM_PREL31` become cross-segment
  relative relocations.
- **Nightly:** `-Zbuild-std=core,alloc` with
  `build-std-features=["compiler-builtins-mem"]`. Pin the nightly in
  `rust-toolchain.toml`, and run a canary CI job on the latest nightly so
  breakage shows up early.
- **Dependency policy (to decide):** the target crates use no third-party
  crates. The host tool is pure Rust and may use a few vetted crates where
  they save real work (PNG decoding, deflate), but nothing that needs a C
  compiler or system library. The alternative is to write those ourselves
  too.

## Repository layout

```
citronnade/
├── Cargo.toml                  # workspace: crates and every example
├── rust-toolchain.toml         # pinned nightly + rust-src
├── crates/
│   ├── citronnade-rt/          # startup code, link.ld, heaps, allocators, panic, svc, per-thread IPC buffer
│   ├── citronnade-ipc/         # Handle, ResultCode, command encoding, the ipc! macro
│   ├── citronnade-services/    # raw typed services (srv, apt, gsp, hid, ir:rst, fs, dsp, mic, cfg, ptm, ac, soc, httpc, cam, y2r, ...)
│   ├── citronnade/             # the user-facing API
│   ├── citronnade-gpu/         # PICA200 command lists, GX transfers, textures, render targets
│   ├── citronnade-shader/      # PICA200 shader assembler (vertex and geometry), as a library and a shader! macro
│   ├── citronnade-assets/      # build-script helpers: images to tiled GPU formats, fonts, audio
│   ├── citronnade-macros/      # #[citronnade::main], shader!
│   ├── citronnade-formats/     # 3DSX, SMDH, RomFS, 3dslink framing (no_std + alloc; used by the tool and on target)
│   └── cargo-citronnade/       # cargo subcommand: new, build, run, test
├── examples/
│   ├── assets/                 # original art, sounds and models (CC0)
│   ├── 00-hello/
│   ├── 01-panic/
│   └── ...                     # one crate per example, numbered in learning order
└── docs/
```

## Example conventions

- **One crate per example,** numbered in learning order, so the list reads as
  a progressive tour of the hardware.
- **Each example declares the version that introduced it** and what it
  covers:
  ```toml
  [package.metadata.citronnade]
  since = "0.3"
  covers = ["hid::touch", "Screen::bottom"]
  title = "Touch paint"          # SMDH title
  ```
- **Each example's README gives:** what it shows, the controls, the
  hardware checklist, and how it degrades on models without the feature
  (2DS: no stereo; Old 2DS: no wide mode; Old 3DS: no C-Stick, ZL or ZR
  without the Circle Pad Pro).
- **CI builds every example on every commit.** A breaking API change updates
  the examples in the same PR, so they always reflect the current API.
  Release notes list the examples each version added.
- **Self-checks where the output is deterministic.** Launched with
  `--selftest` (as an argument through 3dslink), an example runs N frames,
  hashes its framebuffers and reports over the 3dslink socket, and
  `cargo citronnade test` compares the result with a golden value. GPU
  examples compare against a software reference renderer.
- **A release is done when** all of its examples pass their checklists on
  hardware: an Old 3DS (268 MHz, the performance floor), a New 3DS, and a
  2DS for the fallbacks. Azahar is for fast iteration only: it emulates most
  services at a high level, so passing there proves little about the service
  layer.
- **Assets** are original, CC0, and generated procedurally where that's
  natural (gradients, noise, checkerboards, tones).

## Releases

The effort figures assume one experienced developer working part-time, and
are rough. Each release builds on the previous ones.

### v0.1: Boot and deploy (1–2 weeks)
**Library:**
- startup code: the 3DSX header words and clearing `.bss` (the loader
  doesn't);
- heaps through `svcControlMemory` (split rule from libctru's
  `allocateHeaps.c`), a global allocator and a linear-heap allocator;
- the panic path, `svcExitProcess`, and `log!` through
  `svcOutputDebugString`.

**Tool:**
- a 3DSX writer: segments, relocation tables made of skip/patch word runs,
  and the extended header with SMDH and RomFS offsets;
- an SMDH writer: UTF-16 titles, and 24×24 and 48×48 RGB565 icons tiled in
  Morton order;
- `cargo citronnade build` and `new`;
- `cargo citronnade run`. This only needs the host side of 3dslink, because
  the Homebrew Launcher is the receiver:
  - UDP `3dsboot` broadcast on port 17491, answered by `boot3ds`;
  - TCP to 17491 with the name length, name and file size as little-endian
    u32s, then an i32 response (0 is ok);
  - zlib-deflated chunks of up to 16 KiB, each prefixed by its length;
  - then the argument list.

**Oracle:** a CI-only check compares the 3DSX output byte for byte with
`3dsxtool`, using the devkitPro Docker image as a test oracle only.

| # | Example | Exercises |
|---|---|---|
| 00 | hello | startup, linking, 3DSX/SMDH, `log!`, a clean exit to the launcher |
| 01 | panic | panic message in the log, then a clean exit (a visible message from v0.2) |
| 02 | heap | `Box`, `Vec`, `BTreeMap` stress; app and linear heap sizes; out-of-memory handling |

### v0.2: Screens, buttons, lifecycle (3–5 weeks)
**Library:**
- **srv:** connect, register, get service handles.
- **APT:** port libctru `apt.c` behaviour (1,516 lines; the riskiest code in
  the project): init, notifications, the frame loop, HOME, sleep, power
  button, and exit.
- **GSP:** GPU access, the interrupt queue, framebuffers in RGBA8, BGR8,
  RGB565, RGB5A1 and RGBA4; double buffering and VBlank waits. The rotated
  240×400 layout is hidden by the API.
- **HID:** buttons and the Circle Pad.
- **API:** `Screen`, `Input` (held/down/up/repeat), a software 2D canvas
  (pixels, lines, rects, blits with alpha), and a bottom-screen text console
  (public-domain 8×8 font).

| # | Example | Exercises |
|---|---|---|
| 03 | colors | fill both screens, cycle colours, every framebuffer format, double buffering, VBlank |
| 04 | console | text console on the bottom screen, scrolling, `print!`, panics shown on screen |
| 05 | buttons | every button drawn live (held/down/up), Circle Pad vector and dead zone |
| 06 | lifecycle | survives HOME round trips, sleep (lid), the power button and exit; counts and logs every APT event |
| 07 | canvas-2d | software 2D: bouncing sprites (original art), alpha blits, a frame-time graph |

### v0.3: Touch, sensors, 3D slider, system info (2–3 weeks)
**Library:**
- touch with calibration;
- accelerometer and gyroscope;
- `ir:rst` for the C-Stick, ZL and ZR;
- top-screen stereo mode (left and right framebuffers) and wide mode
  (800×240);
- the 3D slider value from the shared config page (`slider_3d`);
- `cfg:u` (model, language, region), `ptm:u` (battery, charging), the volume
  slider, Wi-Fi strength, the RTC clock.

| # | Example | Exercises |
|---|---|---|
| 08 | touch-paint | draw on the bottom screen with the stylus, brushes, clear |
| 09 | stereo-slider | layered parallax scene (original art), depth following the slider, mono at 0 and on 2DS |
| 10 | wide-mode | 800×240 top screen, with a fallback message on Old 2DS |
| 11 | motion | tilt to roll a ball (accelerometer), rotation readout (gyroscope) |
| 12 | new3ds-inputs | C-Stick, ZL and ZR, with model detection |
| 13 | system-info | model, language, region, battery and charging, volume slider, Wi-Fi strength, date and time |

### v0.4: Files, RomFS, threads, time (2–3 weeks)
**Library:**
- **FS:** `fs:USER` and the SD card archive (directories, files,
  read/write).
- **RomFS:** a reader for RomFS appended to the 3DSX. The Homebrew Launcher
  passes the 3DSX path in the argument list. `cargo citronnade build` packs
  a `romfs/` directory.
- **Threads:** creation with a choice of core, mutexes, events, address
  arbiters and the syscore time limit.
- **Time:** system ticks, sleep and frame pacing.

| # | Example | Exercises |
|---|---|---|
| 14 | sd-browser | browse the SD card with buttons and touch, view text files, write a log file |
| 15 | romfs-assets | load images and text from RomFS |
| 16 | threads | worker threads on both cores, synchronisation primitives, a CPU-load display |
| 17 | timing | tick timer, sleep accuracy, frame pacing jitter graph |

### v0.5: Sound (3–4 weeks)
**Library:**
- `dsp::DSP` and a subset of NDSP (libctru `ndsp.c`, about 1,400 lines, as
  the behaviour reference): voices, wave buffers, volume, pan, streaming, and
  sleep and wake.
- This needs the user's `/3ds/dspfirm.cdc`. Without it, examples run silent
  with a clear message.
- `mic:u` for recording.

| # | Example | Exercises |
|---|---|---|
| 18 | tone | synthesised sine, square and noise, with pitch and volume on the Circle Pad |
| 19 | samples | WAV from RomFS, several voices, panning, looping, volume |
| 20 | touch-synth | a keyboard on the bottom screen, polyphony, latency measurement |
| 21 | microphone | record, play back, level meter |

### v0.6: GPU, shaders and 2D (6–8 weeks)
**Library:**
- GX commands through GSP shared memory: `DisplayTransfer`, `TextureCopy`,
  `MemoryFill`.
- PICA200 command lists and state: vertex buffers and attributes, uniforms,
  textures (formats, Morton tiling, ETC1), texture combiners (the PICA200
  has no fragment shaders), blending, depth, viewport and scissor, and
  render targets.

**Shaders:**
- `citronnade-shader` assembles PICA200 vertex shader source into
  SHBIN/DVLB at compile time (`shader!`), with errors reported at the right
  source line. picasso and citro3d are the format references.

**Assets:**
- `citronnade-assets` converts PNG to tiled GPU formats in build scripts, so
  the console does no conversion.

| # | Example | Exercises |
|---|---|---|
| 22 | gpu-clear | clear and display transfer to both screens |
| 23 | triangle | first vertex shader through `shader!`, vertex buffers, uniforms |
| 24 | textured-quad | texture upload with tiling, filtering, wrap modes |
| 25 | texenv | texture combiner showcase: multitexturing, colour operations, alpha test |
| 26 | sprites | thousands of batched 2D sprites with blending (original art) |
| 27 | gpu-text | font atlas text rendering on both screens |

### v0.7: 3D and stereoscopic rendering (6–8 weeks)
**Library:**
- maths (matrices, quaternions);
- stereo rendering helpers: per-eye projection from the slider, interocular
  distance and the convergence plane;
- fragment lighting (lights, LUTs, spotlights), fog, procedural textures,
  stencil;
- geometry shaders in `shader!`.

| # | Example | Exercises |
|---|---|---|
| 28 | cube | perspective, depth buffer, orbit camera on the Circle Pad |
| 29 | stereo-3d | true stereoscopic rendering, slider-driven eye separation, the convergence plane |
| 30 | lighting | per-pixel fragment lighting: diffuse, specular, spotlights, LUT editing on the touch screen |
| 31 | geometry-shader | points expanded into billboards on the GPU |
| 32 | fog | fog and depth cueing |
| 33 | render-to-texture | offscreen target used as a texture; a minimap on the bottom screen |
| 34 | procedural-texture | the procedural texture unit |
| 35 | stencil | stencil reflections and outlines |
| 36 | model | load and draw an original model from RomFS (converted at build time) |

### v0.8: Network (3–4 weeks)
**Library:**
- `ac:u` (Wi-Fi status);
- `soc:U` sockets (TCP and UDP);
- `httpc:C` (HTTP and HTTPS through the system).

**Tool:**
- `cargo citronnade run --server` captures the app's stdout over the 3dslink
  socket;
- `cargo citronnade test` runs examples' self-checks and unit tests on the
  console.

| # | Example | Exercises |
|---|---|---|
| 37 | wifi-status | connection state and signal strength |
| 38 | sockets | TCP echo with a PC, UDP broadcast discovery |
| 39 | stdio | print to the host terminal; the basis of `cargo citronnade test` |
| 40 | http | fetch and show a web page or JSON |

### v0.9: System applets and cameras (3–4 weeks)
**Library:**
- library applets through APT: the software keyboard and the error applet;
- `cam:u` and `y2r:u` (YUV to RGB conversion in hardware).

| # | Example | Exercises |
|---|---|---|
| 41 | software-keyboard | text input through the system keyboard |
| 42 | error-applet | show system error messages |
| 43 | camera | live preview from each camera; the two outer cameras as a stereo pair on the 3D screen |

### v1.0: Release
**Library:** New 3DS speed-up (804 MHz and L2 cache, via `ptm:sysm`),
performance pass, documentation, a template repository, and publishing on
crates.io.

| # | Example | Exercises |
|---|---|---|
| 44 | showcase | a small original game combining 2D and 3D, stereo, touch, sound and high scores on the SD card |

Later ideas, not scheduled:
- local wireless between two consoles (`uds`);
- IR;
- the Mii selector;
- notification LED patterns;
- a `std` shim;
- CIA output.

## API sketch

```rust
// Services are owned. System is handed to main once, and each service is moved
// out of it, so "already initialised" is a compile error instead of a runtime one.
#[citronnade::main]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    let mut screens = gsp.screens(TopMode::Stereo)?;
    let mut input = hid.open()?;
    apt.run(|frame| {                        // handles HOME, sleep, power button and exit
        let pad = input.scan();
        let (left, right) = screens.top_stereo();
        // ... draw ...
        frame.present(&mut screens)          // swaps buffers on VBlank
    })
}

// IPC commands are declared once and the encoding is generated.
ipc! {
    service HidUser = "hid:USER" {
        #[cmd(0x000A)] fn get_ipc_handles() -> (SharedMemory, [Event; 5]);
        #[cmd(0x0011)] fn enable_accelerometer();
    }
}
```

- **Handles** close themselves when dropped. `ResultCode` decodes level,
  summary, module and description, and gives names for the common ones.
- **Memory:** `Box` and `Vec` use the app heap. `Linear<T>` and `LinearVec`
  hold GPU- and DSP-visible memory, and `Vram` holds VRAM. Cache flushes
  happen inside the calls that hand a buffer to the hardware.
- **Async:** v1 is a synchronous frame loop. Keep handles and events
  waitable (`svcWaitSynchronizationN`) so an async layer can be added without
  breaking anything.

## End goal: the SMB port

Status: phases 1 to 5 are done on smb's `3ds` branch (`platforms/3ds/`),
checked in Azahar. The layers are drawn by the GPU (four planes: scenery,
playfield, status bar, sprites) with integer parallax, and the desktop
`--layers` view and the replay of the golden trace on hardware remain.

The port lives in **smb**, at `platforms/3ds/`, next to the desktop and web
front ends. It's mostly smb's own code: the generated game, the C runtime and
the map's names. Its depth layers come from the game's level data, which the
web themes already read. citronnade stays game-agnostic and never touches a
ROM. During development, smb depends on citronnade by git, with a local
`[patch]` override pointing at a sibling checkout.

What's already known:
- The generated C and its runtime compile with clang for the ARM11 without a
  C library and link with a Rust entry point. smb already requires clang.
- The front-end interface is 10 C functions: `cpu_init`,
  `apu_init(size_t)`, `video_init(const uint8_t*)`, `render_init`,
  `cpu_reset_entry`, `update_controller1(uint8_t)`, `next_frame`,
  `render_frame`, `sound_end_frame`, `apu_render(float*, size_t)`.
- The CHR is embedded at build time from the user's ROM. The built `.3dsx`
  therefore contains the game and must never be distributed.

Phases, each gated on a citronnade release:
1. **Mono** (after v0.2): one NES frame per VBlank, centred on the top
   screen, with controls mapped. Replay smb's golden trace on hardware to
   prove the ARM build behaves identically.
2. **Audio** (after v0.5): `apu_render` on a system-core thread into DSP
   wave buffers.
3. **Layers** (any time; desktop-only work): a `codegen/lib/layers.c` that
   splits the frame into planes, using the same material rules as the web
   themes, plus a desktop `--layers` debug view:
   - backdrop;
   - scenery;
   - behind-background sprites;
   - playfield (from the block buffer);
   - sprites;
   - status bar.
4. **Stereo** (after v0.3, GPU path after v0.7): shift planes per eye by the
   slider. Use a software compositor first, then GPU quads with sub-pixel
   parallax.
5. **Polish:** save states on the SD card, depth presets, a bottom-screen
   menu, an original icon.

Optional later: a Rust backend for the recompiler, which would remove clang.
The WebAssembly backend's goto-free structuring maps directly onto Rust's
labelled blocks and `break 'label`.

## Legal hygiene
- Neither repository, nor any CI artefact, contains:
  - the ROM;
  - generated code or anything else extracted from the ROM;
  - built game `.3dsx` files.
- The port's CI uses smb's ROM-free NROM fixture.
- All example assets and icons are original.
- Don't use Nintendo trademarks in names or titles.
- Where behaviour is ported closely from libctru (zlib), add an attribution
  note.

## Risks
| Risk | Mitigation |
|---|---|
| APT lifecycle bugs (hangs on HOME, sleep or exit) | Port libctru's behaviour closely; example 06 runs on hardware every release |
| Nightly breakage | Pinned toolchain plus a canary CI job |
| Emulator hides service bugs | Hardware checklists are the release gate |
| Shader assembler and GPU state are large | v0.6 starts with the smallest path (clear, then a triangle); software references for self-checks |
| Old 3DS performance | Every example's README states its Old 3DS frame time |
| No DSP firmware dump | Documented; examples run silent with a message |
| Model differences (2DS, Old 2DS, Old and New 3DS) | Each example states and handles its fallback |

## First steps
1. v0.1: workspace, `citronnade-rt`, the 3DSX and SMDH writers,
   `cargo citronnade build`, `new` and `run`, and examples 00–02 on hardware
   and Azahar.
2. v0.2, the riskiest library work (APT), with example 06 as its test.
3. Continue release by release. The SMB layer work (phase 3) can happen in
   smb at any time, since it needs no console.
