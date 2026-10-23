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

A clock spin may be passed in one step (#217). LEGO Chess waits between its movies' frames by
reading `QueryPerformanceCounter` again and again: 99.996 % of a frame's instructions, about 98 %
of its host time, and on the maintainer's MacBook the movies played at 0.72–0.80 of real time
headless, their music stretched to match. Two crossings in a row may be the same clock read from
the same place, in the same declared millisecond, with the processor the same bit for bit, no byte
of memory changed between them, no time stamp counter read, no other thread able to run, and no
quantum ending inside the pass. Then the pass between them is one the guest would make again,
unchanged, until the clock reads the next millisecond, and the machine retires those passes at once
(`core/src/spin.rs`).

Only a loop proven to change nothing but the clock is passed. It lands at exactly the cursor,
declared clock and state that running every pass gives, and is checked against a run that does.
It is never keyed to a title, and a traced or stepped run runs every pass. The guest's own code
still runs the first passes of every millisecond, and the one that sees it change. The maintainer
allowed it on 2026-10-22: "Allow the exact skip".

## Considered options

- **Native override of hot guest functions**: fast, but the emulator would no longer be running the
  game; rejected.
- **JIT from the start**: an earlier, unpublished emulator for these titles measured that its JIT
  gates could not be reached from a 149 MIPS interpreter at any cost, and every optimisation short
  of code generation was still available; deferred behind the seam.
- **A declared cost per clock read** (#217): what a `QueryPerformanceCounter` cost a Windows 98
  Pentium 166 would be a declared value, but no public source gives one, so it cannot be declared;
  and it would move every run that reads the clock.
- **Interpreter speed alone** (#217): the cheap part of a clock read's crossing was worth about
  1.1×, where Chess's movies needed 1.4× or more on the MacBook.
