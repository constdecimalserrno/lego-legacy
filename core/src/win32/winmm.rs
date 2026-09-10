//! The multimedia library: the declared clock as a title of this era reads
//! it, and what it asks of sound hardware.

use super::{Convention, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[Shim {
    dll: "WINMM",
    name: "timeGetTime",
    args: 0,
    convention: Convention::Stdcall,
    run: time_get_time,
}];

fn time_get_time(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Milliseconds on the declared clock, wrapping as the real counter does
    // (ADR-0003).
    Ok((ctx.declared_millis() & 0xffff_ffff) as u32)
}
