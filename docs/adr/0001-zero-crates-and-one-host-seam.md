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

## Considered options

- **dlopen SDL3**, as an earlier, unpublished emulator for these titles did: one Homebrew dylib
  away from not running; rejected.
- **Linking crates for windowing/audio**: rejected by the twenty-year rule.

## Linux: X11 and ALSA

Linux's backend (#108) reaches X11 through `libX11.so.6` for the window, input and pointer, and
ALSA through `libasound.so.2` for sound, each opened by its soname so the loader finds it wherever
the distribution put it. Each is one library with a C interface that has held still for decades,
and nearly every desktop distribution installs both. X11 also serves a Wayland desktop, through
XWayland, and ALSA's default output is served by PipeWire or PulseAudio wherever either runs, so
one backend covers the desktops people have. Native Wayland is rejected for now:
`libwayland-client` carries only the core protocol, and a window needs more (xdg-shell to be a
window at all, pointer constraints and relative pointer motion to steer a pointer), whose interface
tables a program normally generates from the protocols' XML and this one would declare by hand;
the pointer protocols are also ones a compositor need not offer. A native Wayland backend can come
later as its own ticket. PipeWire's and PulseAudio's own libraries are rejected for sound: a system
with either has ALSA's plugin for it, and a system with neither still has ALSA.

