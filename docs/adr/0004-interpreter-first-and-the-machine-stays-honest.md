# Interpreter first, and the machine stays honest

The CPU is a decode-cached interpreter behind a `CpuEngine` seam. No JIT is built until a title
demonstrably cannot reach pace 1.0× any other way. No guest function is ever replaced by a native
Rust override: the machine executes the title's bytes or stops. The software rasteriser may go
multithreaded only through a declared, host-independent decomposition of the frame, and only after
a measurement names it as the wall.

## Considered options

- **Native override of hot guest functions**: fast, but the emulator would no longer be running the
  game; rejected.
- **JIT from the start**: the previous tree measured that its JIT gates could not be reached from a
  149 MIPS interpreter at any cost, and every optimisation short of code generation was still
  available; deferred behind the seam.
