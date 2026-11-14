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

use citronnade::log;

#[citronnade::main]
fn main() {
    log!("Hello from the 3DS!");
}
```

`log!` writes to the debugger: Luma3DS's GDB stub on hardware, or the
emulator's log.

## How it works

- Rust's built-in `armv6k-nintendo-3ds` target, with `core` and `alloc`
  built from source (`-Zbuild-std`) and linked by the `rust-lld` that ships
  with Rust, using Citronnade's own linker script and startup code.
- `citronnade-rt`: startup, heaps and allocators, system calls, logging,
  panics and exit.
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

Build one with `cargo citronnade build -p hello` from the repository root.

## Testing

Host tests: `cargo test`. The examples run headless in Azahar's libretro
core and check their own output: see [docs/emulator.md](docs/emulator.md).

## License

MIT or Apache-2.0, at your option.
