# Citronnade guide

A tour of the library, one subsystem at a time. Each section names the
examples that show it working; they are the most complete reference.

- [Programs](#programs)
- [Screens and 2D drawing](#screens-and-2d-drawing)
- [Input](#input)
- [Lifecycle](#lifecycle)
- [The GPU](#the-gpu)
- [Shaders](#shaders)
- [3D and stereoscopy](#3d-and-stereoscopy)
- [Assets](#assets)
- [Sound](#sound)
- [Files](#files)
- [Network](#network)
- [Applets and cameras](#applets-and-cameras)
- [Threads and time](#threads-and-time)
- [The tool](#the-tool)
- [Installable titles](#installable-titles)
- [On hardware](#on-hardware)

## Programs

A program is a `no_std`, `no_main` binary whose entry point carries
`#[citronnade::main]`. `main` may take the `System` (the services, handed
over once) and return `()`, an `i32` exit code or a `Result`.

```rust
#![no_std]
#![no_main]

use citronnade::prelude::*;

#[citronnade::main(stack_size = 512 * 1024)]
fn main(sys: System) -> Result<()> {
    let System { apt, gsp, hid, .. } = sys;
    // ...
    Ok(())
}
```

`alloc` works (`Vec`, `String`, `Box`, `Arc`); the heap is split between the
application heap and the linear heap (memory the GPU and DSP can read) as
libctru does, or as `heap_size` and `linear_heap_size` say. A panic shows on
the bottom screen and in the log, then exits. Examples: 00 hello, 01 panic,
02 heap.

`log!` writes a line to the debugger (Luma3DS's GDB stub or the emulator's
log), and to the PC when started with `cargo citronnade run --server`.
`println!` writes to the on-screen [`Console`] when there is one.

## Screens and 2D drawing

`gsp.screens()` returns double-buffered framebuffers in linear memory.
Draw into the back buffers through `Canvas` (screen coordinates, x right, y
down), then `present()` shows them at the next VBlank.

```rust
let mut screens = gsp.screens()?;
let mut top = screens.top();
top.clear(Color::hex(0x204080));
top.fill_rect(10, 10, 100, 40, Color::rgba(255, 255, 255, 128)); // blended
top.text(16, 20, "score 120", Color::WHITE, 2);
screens.present();
```

The top screen has three layouts (`TopMode::Mono`, `Stereo`: one image per
eye, `Wide`: 800x240), and every screen any framebuffer format. Software
renderers can copy whole images with `draw_rgb888` and `draw_rgb565`, or
write the framebuffer directly through `bytes_mut()` (columns of 240
pixels, bottom to top: the screens are mounted sideways). Examples: 03
colors, 04 console, 07 canvas-2d, 09 stereo-slider, 10 wide-mode.

## Input

```rust
let mut input = hid.input()?;
let state = input.scan();                       // once per frame
if state.down().contains(Buttons::A) { /* just pressed */ }
if state.held().contains(Buttons::DPAD_LEFT) { /* held */ }
let (x, y) = state.circle_pad();                // -156..156
if let Some((tx, ty)) = state.touch() { /* stylus on the bottom screen */ }
```

`Buttons::UP` and friends combine the D-pad and the Circle Pad's direction
bits; `contains` needs every bit, so use `DPAD_UP` for the D-pad alone. The
accelerometer, gyroscope, C-Stick and ZL/ZR are there too, and
`slider_3d()` reads the 3D slider. Examples: 05 buttons, 08 touch-paint, 11
motion, 12 new3ds-inputs.

## Lifecycle

`apt.main_loop()` returns false when the program should end (the POWER
button, closing from HOME). It handles HOME, sleep and the system's
requests; `apt.on(hook)` registers a function for suspend, restore, sleep
and wakeup. `apt.set_speedup(true)` runs a New 3DS at 804 MHz with its L2
cache. Example: 06 lifecycle.

## The GPU

`citronnade::gpu` drives the PICA200. A `Gpu` records state and draws into a
command list; clearing a target or showing it runs the list.

```rust
use citronnade::gpu::{self, Gpu, RenderTarget, Shader, VertexLayout, AttribType,
    LinearBuffer, Primitive, TexEnv, Source};

let mut gpu = Gpu::new()?;
let target = RenderTarget::top()?;                 // 400x240, RGBA8, depth + stencil
let shader = Shader::new(VERTEX_SHADER).unwrap();  // from shader! (below)
let projection = shader.uniform("projection").unwrap();
let vertices = LinearBuffer::from_slice(&TRIANGLE)?;
let layout = VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 4);

gpu.clear(&target, gpu::rgba(16, 20, 40, 255), 0)?;
gpu.bind_target(&target);
gpu.bind_shader(&shader);
gpu.set_layout(&layout);
gpu.bind_vertices(&vertices);
gpu.set_texenv(0, TexEnv::replace(Source::PrimaryColor));
gpu.set_mat4(projection, &Mat4::screen_2d(400.0, 240.0));
gpu.draw_arrays(Primitive::Triangles, 0, 3);
screens.show_target(&mut gpu, &target, Screen::Top)?;
screens.present();
```

There are no fragment shaders: six texture combiner stages (`TexEnv`)
compute each pixel from the vertex colour, up to three textures, the
procedural texture unit, fragment lighting and constants. Textures are
power-of-two images in a tiled layout (`Texture::from_rgba` converts, or
`citronnade-assets` does it at build time); render targets can be sampled
too. Blending, the depth and stencil tests, the alpha test, culling,
viewport and scissor are `Gpu` methods. Colours given to the GPU are
0xAABBGGRR (`gpu::rgba`). Examples 22 to 27.

## Shaders

`gpu::shader!` assembles PICA200 assembly at compile time (picasso's
syntax) into a SHBIN, and reports errors at the shader's line:

```rust
const VERTEX_SHADER: &[u8] = shader!(r#"
    .fvec projection[4]
    .constf ones(1.0, 1.0, 1.0, 1.0)
    .alias inpos v0
    .alias inclr v1
    .out outpos position
    .out outclr color
    .proc main
        mov r0.xyz, inpos
        mov r0.w, ones
        dp4 outpos.x, projection[0], r0
        dp4 outpos.y, projection[1], r0
        dp4 outpos.z, projection[2], r0
        dp4 outpos.w, projection[3], r0
        mov outclr, inclr
        end
    .end
"#);
```

`shader!(file = "shaders/thing.pica")` reads a file next to `Cargo.toml`.
Geometry shaders start with `.gsh point c0` (or `variable`, `fixed`) and
use `setemit` and `emit`; bind them with `gpu.bind_program(&vs, Some(&gs))`
and draw `Primitive::Geometry`. Each stage has its own uniforms. Shader
binaries from picasso load with `Shader::new` as well. Examples 23, 31.

## 3D and stereoscopy

`citronnade::math` has `Vec3`, `Vec4`, `Mat4` (rows; shaders compute
`M * v` with `dp4`) and `Quat`. The 3DS's framebuffers are sideways, so
projections come in tilted versions: `Mat4::perspective_tilt`,
`ortho_tilt` and `screen_2d` for the screens, untilted `perspective` and
`ortho` for render targets used as textures. Depth maps to the GPU's 0..-1
range with the near plane largest, so the default depth test is
`TestFunc::Greater` and depth clears to 0.

For stereoscopy, set `TopMode::Stereo`, draw the scene twice with
`Mat4::perspective_stereo_tilt(fovy, aspect, near, far, ±iod, screen)`
(the interocular distance follows `slider_3d()`), and show the results with
`show_target` (left) and `show_target_right`. One render target serves both
eyes: each transfer finishes before the next eye is drawn.

Fragment lighting (`LightEnv`, `Light`, `LightLut`), fog (`FogLut`), the
procedural texture unit (`ProcTex`) and the stencil buffer are set on the
`Gpu` the same way. Examples 28 to 36.

## Assets

Build scripts use `citronnade-assets` so the console loads data ready to
use:

```rust
// build.rs
fn main() {
    citronnade_assets::build_texture("assets/tile.png", citronnade_assets::TexFormat::Rgba5551);
    citronnade_assets::build_mesh("assets/lemon.obj", "romfs");
}
```

`Texture::from_ctex(include_bytes!(concat!(env!("OUT_DIR"), "/tile.ctex")))`
and `Mesh::from_cmsh(&romfs.read("/lemon.cmsh")?)` load them. Files in the
directory named by `romfs = "romfs"` in `[package.metadata.citronnade]` are
packed into the 3DSX and read with `RomFs::mount()`. Examples 15, 24, 36.

## Sound

`Audio::new()` loads the DSP firmware (`sd:/3ds/dspfirm.cdc`, dumped from
the console with the DSP1 homebrew) and gives 24 channels. Queue
`Arc<WaveBuf>` buffers of 16-bit or 8-bit samples; a buffer can loop.

```rust
let audio = Audio::new()?;
let ch = audio.channel(0);
ch.set_format(SampleFormat::Pcm16Mono);
ch.set_sample_rate(OUTPUT_RATE);
ch.queue(&Arc::new(WaveBuf::from_samples(&samples, 1)));
```

`mic::Microphone` records from the microphone. Examples 18 to 21, 44.

## Files

`citronnade::fs` reads and writes the SD card (`read`, `write`, `append`,
`create_dir_all`, directories, `File`). Example 14; 44 keeps its high
scores there.

## Network

```rust
let net = Network::new()?;                                  // sockets + Wi-Fi status
let mut s = TcpStream::connect("192.168.1.20:7777".parse().unwrap())?;
s.set_nonblocking(true)?;                                   // for frame loops
let http = Http::new()?;
let page = http.get("http://example.com/", 64 * 1024)?;     // HTTP and HTTPS
```

`UdpSocket`, `TcpListener` and `net.resolve(host)` are there too. In a frame
loop, use non-blocking sockets and treat `e.would_block()` as "not yet".
`net::stdio` sends `println!` and `log!` lines to a PC. Examples 37 to 40.

## Applets and cameras

```rust
let r = Keyboard::new(KeyboardKind::Normal, 2, 24).hint("Your name").show();
if r.button == Some(KeyboardButton::Right) { name = r.text; }
ErrorDisplay::text("Something went wrong.").show();
```

`Cameras::open(CameraSelect::OuterStereo, PixelFormat::Rgb565)` captures
400x240 frames; `poll()` says when new ones arrived without waiting, and
`Y2r` converts YUYV frames in hardware. Examples 41 to 43.

## Threads and time

`thread::Builder` starts threads on the application core or (after
`apt.set_app_cpu_time_limit`) the system core; `sync::Mutex` and
`sync::Event` are built on the kernel's address arbiter, so a high-priority
thread waiting on a low-priority one does not spin. `time::Instant` and
`svc::sleep_thread` measure and wait. Examples 16, 17.

## The tool

- `cargo citronnade build [--release] [-p name]` builds and packages each
  binary as a `.3dsx` with its SMDH (title, description, author and icon
  from `[package.metadata.citronnade]`) and RomFS. With `--cia` it also
  packages an installable title: a `.cia` (for FBI or another installer)
  and the `.cxi` inside it (which emulators run directly).
- `cargo citronnade run [-p name] [--server]` sends it to the Homebrew
  Launcher's netloader (Y); `--server` shows the program's output.
- `cargo citronnade test` sends each program with a console test and checks
  its output.
- `cargo citronnade serve [-p name] [--release]` builds each binary as a
  `.cia`, serves it over HTTP on the local network and shows a QR code of
  its address: in FBI, Remote Install > Scan QR Code installs it.
  `--port` (default 8000) and `--address` (this computer's, found by
  default) adjust the address. `cargo citronnade qr <text>` shows any QR
  code.
- `cargo citronnade new <path>` starts a project; any other command (check,
  clippy, doc) runs cargo for the 3DS.

## Installable titles

A `.cia` installs the program on the HOME Menu like any title. It is the
same program as the `.3dsx`: the runtime notices that no homebrew loader
started it, reads its RomFS from the title, and closes through the system
when it ends. `[package.metadata.citronnade]` can set:

```toml
[package.metadata.citronnade]
unique_id = "0xF8123"          # the title ID's 20 bits (default: from the name)
product_code = "CTR-H-LEMN"    # default: CTR-H- and the name's first letters
banner = "banner.bnr"          # a HOME Menu banner (made with bannertool)
new3ds_speedup = true          # start at 804 MHz with the L2 cache
new3ds_memory = "124MB"        # "legacy" (default), "124MB" or "178MB"
```

`cargo citronnade serve` sends a title to a console without copying files
(see [the tool](#the-tool)). The title version comes from the package's version. The HOME Menu shows
the homebrew splash logo while it starts the title. Without a banner it
shows the title with its icon only. Nothing is signed, so
consoles install these titles with signature checks patched, as custom
firmware such as Luma3DS does; give each program its own `unique_id` so
titles do not replace each other.

## On hardware

The examples are checked headless in Azahar. On a console:

- sound needs the DSP firmware dump;
- texture filtering shows (Azahar's software renderer samples the nearest
  texel);
- the 3D slider drives the stereo examples, and the outer cameras give a
  real stereo pair;
- the network examples need the PC side, `tools/net/hostservers.py`, and
  the PC's address in `sd:/citronnade/net.txt`.
