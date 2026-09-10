# Interpreter first, and the machine stays honest

The CPU is a decode-cached interpreter behind a `CpuEngine` seam. No JIT is built until a title
demonstrably cannot reach pace 1.0× any other way. No guest function is ever replaced by a native
Rust override: the machine executes the title's bytes or stops. The software rasteriser may go
multithreaded only through a declared, host-independent decomposition of the frame, and only after
a measurement names it as the wall.

That measurement came on 2026-09-10 (#21): Rock Raiders' rockfall, 1.2 million pixels a frame of
filtered, blended texture, took 26 ms on one thread against a 16.7 ms frame. A draw with enough pixels
is now split into four declared bands of rows (`raster::BANDS`), each pixel drawn by the same
triangles in the same order as on one thread, so the frame is the same on every host.

## Considered options

- **Native override of hot guest functions**: fast, but the emulator would no longer be running the
  game; rejected.
- **JIT from the start**: the previous tree measured that its JIT gates could not be reached from a
  149 MIPS interpreter at any cost, and every optimisation short of code generation was still
  available; deferred behind the seam.
