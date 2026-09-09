# The emulation boundary is the DirectX and Win32 API

A title's own binaries (its executable and the DLLs it ships) run as guest x86. Microsoft's DLLs
(KERNEL32, USER32, MSVCRT, DDRAW, D3DIM, D3DRM, DSOUND, DINPUT, and vendor audio such as A3D) are
written in Rust as shims at their export tables; no Microsoft binary is ever required or run.
Run-versus-write is priced per binary: a DLL that ships on the disc may run as guest code when
that is cheaper than writing it.

## Considered options

- **Running the DirectX redistributable the discs ship**: pushes the boundary down to the display
  driver and kernel, a wider and undocumented surface; rejected.
