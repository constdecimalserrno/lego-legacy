# Zero crates, and one host seam reached by dlopen

The emulator must still build and run twenty years from now, on macOS first and Linux and
Windows later. We decided: no external crates, ever (`Cargo.lock` resolves the workspace's own five
members and nothing else, and a test holds it there), and every touch of the host, window, input,
audio, wall clock, memory mapping, dynamic loading, lives in one `host` module behind one trait,
reaching the operating system's own frameworks by `dlopen`/`dlsym` (and `objc_msgSend` on macOS).
No third-party library is loaded, not even SDL. Porting to a new OS is one new backend under that
module, and the one function in `run.rs` that opens its window. What the standard library already
carries to every OS stays outside the module: files, the rasteriser's threads, and the second a run
directory is named by.

## Windows: user32, gdi32 and waveOut (#107)

The Windows backend reaches the oldest, plainest layer of Windows that does the job: user32 for
the window, its messages and the pointer, gdi32's `StretchDIBits` to put the frame in it, and
winmm's `waveOut` for the sound, each loaded from Windows' own System32 with `LoadLibraryExW` and
read with `GetProcAddress`, and kernel32, which std links already, declared by hand. Every
Windows has them with the same entry points, they are flat C calls with no COM and nothing to
install (ADR-0002), and they are all the declared machine needs: the frame is already finished
in guest memory and only has to be scaled, and `waveOut` is the simplest device a machine clock
can pace. Direct3D or DXGI would buy nothing a finished frame needs; WASAPI is the way past
`waveOut` if its latency is ever too long to hold.

## Considered options

- **dlopen SDL3**, as an earlier, unpublished emulator for these titles did: one Homebrew dylib
  away from not running; rejected.
- **Linking crates for windowing/audio**: rejected by the twenty-year rule.
