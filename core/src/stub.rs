//! The stub guest: the machine's own test guest.
//!
//! Eight hand-assembled x86 instructions that count, write a pixel, present
//! one frame and exit. It exists so the run seam can be tested without a
//! pressing on disk, and so a change to the machine is falsified by something
//! cheaper than a retail title.
//!
//! It reaches the machine through two software interrupts, which no retail
//! title on the roster executes: a real guest reaches the emulator through its
//! imports, and an `int` from one is a wall.

use crate::machine::Surface;

/// Where the stub's code is placed, matching where a Win32 image would be.
pub const ENTRY: u32 = 0x0040_0000;
/// Where the stub's surface lives.
pub const SURFACE: u32 = 0x0010_0000;
/// `int 0x20`: present the declared surface.
pub const INT_PRESENT: u8 = 0x20;
/// `int 0x21`: end the run, with `eax` as the exit code.
pub const INT_EXIT: u8 = 0x21;

/// The surface the stub presents: four by four, 32 bits a pixel.
#[must_use]
pub fn surface() -> Surface {
    Surface {
        addr: SURFACE,
        width: 4,
        height: 4,
        pitch: 16,
    }
}

/// The stub's own instructions.
///
/// ```text
/// 0x400000  b8 00 00 00 00   mov eax, 0
/// 0x400005  b9 03 00 00 00   mov ecx, 3
/// 0x40000a  40               inc eax          <- loop
/// 0x40000b  49               dec ecx
/// 0x40000c  75 fc            jnz loop
/// 0x40000e  a3 00 00 10 00   mov [0x100000], eax
/// 0x400013  cd 20            int 0x20         ; present
/// 0x400015  cd 21            int 0x21         ; exit(eax)
/// ```
///
/// Fourteen instructions retired, three turns of the loop, one frame, exit
/// code 3.
#[must_use]
pub fn program() -> &'static [u8] {
    &[
        0xb8,
        0x00,
        0x00,
        0x00,
        0x00, // mov eax, 0
        0xb9,
        0x03,
        0x00,
        0x00,
        0x00, // mov ecx, 3
        0x40, // inc eax
        0x49, // dec ecx
        0x75,
        0xfc, // jnz -4
        0xa3,
        0x00,
        0x00,
        0x10,
        0x00, // mov [0x100000], eax
        0xcd,
        INT_PRESENT, // int 0x20
        0xcd,
        INT_EXIT, // int 0x21
    ]
}

/// What the stub retires when nothing has gone wrong.
pub const RETIRED: u64 = 14;
/// What the stub exits with.
pub const EXIT_CODE: u32 = 3;
