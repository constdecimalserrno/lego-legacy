//! The C runtime.
//!
//! Rock Raiders links its own copy statically, so nothing is imported from
//! MSVCRT and nothing is written here yet. A title that imports it gets these,
//! and they are cdecl: the caller cleans the stack.

use super::Shim;

pub const SHIMS: &[Shim] = &[];
