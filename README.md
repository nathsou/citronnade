# Citronnade

A Nintendo 3DS homebrew toolchain and library in Rust, with no dependency on
devkitPro, a C toolchain or a C library. Builds need only rustup (nightly
with `rust-src`) and `cargo-citronnade`.

Status: early development. See [PLAN.md](PLAN.md) for the roadmap and the
versioned examples each release adds.

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
  `Input`, `Console`, the panic screen, `math`.
- `citronnade-formats`: ELF to 3DSX conversion, SMDH, zlib and PNG, shared
  by the host tool and programs.
- `cargo-citronnade`: builds, packages each binary as a `.3dsx` with its
  SMDH (title, description, author and icon from
  `[package.metadata.citronnade]`), and uploads it with the 3dslink
  protocol.

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

Build one with `cargo citronnade build -p hello` from the repository root.

## Testing

Host tests: `cargo test`. The examples run headless in Azahar's libretro
core and check their own output: see [docs/emulator.md](docs/emulator.md).

## License

MIT or Apache-2.0, at your option.
