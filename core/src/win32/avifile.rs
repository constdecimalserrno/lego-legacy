//! The movie library a title reaches movies through by file.
//!
//! Rock Raiders initialises it and plays its movies through DirectShow's
//! multimedia streams instead ([`super::amstream`], #9). Nothing past
//! initialising has been asked for, so every other entry point is an import
//! nobody has written, and calling one is a wall that names it.

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[stdcall("AVIFIL32", "AVIFileInit", 0, init)];

fn init(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}
