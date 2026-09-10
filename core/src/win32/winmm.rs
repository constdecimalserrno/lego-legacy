//! The multimedia library: the declared clock as a title of this era reads
//! it, and what it asks of sound hardware.

use super::{Convention, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "WINMM",
        name: "timeGetTime",
        args: 0,
        convention: Convention::Stdcall,
        run: time_get_time,
    },
    Shim {
        dll: "WINMM",
        name: "mixerGetNumDevs",
        args: 0,
        convention: Convention::Stdcall,
        run: mixer_devices,
    },
];

fn time_get_time(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Milliseconds on the declared clock, wrapping as the real counter does
    // (ADR-0003).
    Ok((ctx.declared_millis() & 0xffff_ffff) as u32)
}

fn mixer_devices(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The declared machine has no hardware mixer: what a title plays is mixed
    // on the machine's own clock (ADR-0003), and there are no lines on a card
    // for it to turn up or down. Rock Raiders reads zero as nothing to set.
    Ok(0)
}
