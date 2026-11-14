# Notices

Citronnade is written from scratch, but much of its behaviour follows the
devkitPro homebrew libraries, whose sources document how the system
services and the GPU work. Where Citronnade closely follows them, the
original licences ask for this acknowledgement:

- **libctru** (<https://github.com/devkitPro/libctru>), zlib licence,
  copyright the libctru contributors: the IPC command layouts and the
  behaviour of the system services in `citronnade-services` (APT's
  lifecycle, GSP, HID, fs, DSP, ...), the GX command layouts and the GPU
  register numbers in `citronnade-gpu` (`regs.rs` is generated from
  libctru's `registers.h`), and the shader binary layout read and written by
  `citronnade-shader`.
- **citro3d** (<https://github.com/devkitPro/citro3d>), zlib licence,
  copyright the citro3d contributors: the GPU state handling in
  `citronnade-gpu` (register sequences for render targets, effects,
  attributes, texture combiners, fragment lighting and its lookup tables,
  fog, the procedural texture unit), and the tilted and stereoscopic
  projection matrices.
- **picasso** (<https://github.com/devkitPro/picasso>), MIT licence: the
  shader language that `citronnade-shader` assembles, and its padding and
  register allocation rules.

The zlib licence's conditions: the origin of the software must not be
misrepresented, altered versions must be plainly marked as such, and the
notice may not be removed from source distributions.
