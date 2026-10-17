//! The C runtime.
//!
//! Rock Raiders links its own copy statically. Alpha Team's disc ships
//! `MSVCRT.DLL` and `MSVCP60.DLL` and the machine runs them, because writing
//! 284 exports is the largest single piece of work that title could ask for
//! (ADR-0002, amended for #51).
//!
//! What is written here is the heap, and only the heap (#80). It is the one
//! part of that runtime whose behaviour a title can observe only through the
//! pointers it gets back, and run as the runtime's own interpreted x86 it is
//! 4.1% of every instruction the guest retires at Alpha Team's front end:
//! 397,000 allocations over the committed menu route, almost all of them
//! through `operator new`, each one walking the runtime's own small-block
//! allocator.
//!
//! **The two heaps are kept apart by the pointer, not by an audit.** The
//! machine's heap answers for blocks it handed out and for nothing else; the
//! runtime's own code calls its own `malloc` directly rather than through an
//! import table, so its own blocks stay in its own small-block heap.
//!
//! One of the machine's blocks reaching the runtime's own `free` is safe by
//! construction: that `free` looks for the block in its small-block regions,
//! does not find it, and falls through to `HeapFree`, which is this machine's
//! heap again. The other direction cannot be answered — there is no way to
//! hand a block back to the runtime's allocator from here — so the run log
//! counts them. Over Alpha Team's whole menu route there are twelve, and the
//! number does not grow with the run, because `_strdup`, the one call that
//! would make it grow, is written here.
//!
//! These are cdecl: the caller cleans the stack.

use super::{cdecl, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    cdecl("MSVCRT", "malloc", 1, malloc),
    // `operator new`. Alpha Team reaches the heap through this and almost
    // never through `malloc`: 396,976 calls against 2,211 over the menu route.
    cdecl("MSVCRT", "??2@YAPAXI@Z", 1, malloc),
    cdecl("MSVCRT", "calloc", 2, calloc),
    cdecl("MSVCRT", "realloc", 2, realloc),
    cdecl("MSVCRT", "free", 1, free),
    // The one entry point of that runtime, other than the heap itself, that
    // hands a caller a block to free: LoadComp asks for it, and its answer
    // would otherwise be one of the runtime's own blocks reaching this `free`.
    cdecl("MSVCRT", "_strdup", 1, strdup),
];

fn malloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // `malloc(0)` gives back a pointer of its own that may be freed, which is
    // what the heap's own `max(1)` already does. It is `HeapAlloc` with no
    // flags.
    super::kernel32::heap_alloc(ctx, &[0, 0, args[0]])
}

fn calloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let Some(size) = args[0].checked_mul(args[1]) else {
        return Ok(0);
    };
    // HEAP_ZERO_MEMORY: a block handed back and given out again is not zero
    // any more.
    super::kernel32::heap_alloc(ctx, &[0, 0x8, size])
}

fn realloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (old, size) = (args[0], args[1]);
    if old == 0 {
        return malloc(ctx, &[size]);
    }
    let Some(previous) = ctx.state.heap.size_of(old) else {
        // Not one of ours: it is the runtime's own, and the runtime's own
        // `realloc` is the only thing that knows how big it was.
        ctx.state.foreign_frees += 1;
        return Ok(0);
    };
    if size == 0 {
        ctx.state.heap.free(old);
        return Ok(0);
    }
    let new = ctx.state.heap.alloc(size);
    if new == 0 {
        return Ok(0);
    }
    let take = previous.min(size) as usize;
    let bytes = ctx.mem.slice(old, take).map_err(Stop::Fault)?.to_vec();
    ctx.mem.write(new, &bytes).map_err(Stop::Fault)?;
    ctx.state.heap.free(old);
    Ok(new)
}

fn free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let addr = args[0];
    // Freeing a null pointer is what the C runtime says it is: nothing.
    if addr == 0 || ctx.state.heap.free(addr) {
        return Ok(0);
    }
    // One of the runtime's own, which its own code allocated by calling its
    // own `malloc` directly rather than through an import table. Giving it back
    // is the runtime's to do, and there is no way to ask it to from here, so
    // the block stays where it is.
    //
    // ponytail: twelve blocks over Alpha Team's whole menu route, and the count
    // does not grow with the run — `_getcwd` is the one entry point left that
    // hands a caller a block, and the rest are the C++ exception machinery's.
    // Writing `_getcwd` would take the twelve to nothing, and it would have to
    // answer with the declared machine's own directory, which is a different
    // question from the heap. The run log carries the count, so a title that
    // makes it grow says so.
    ctx.state.foreign_frees += 1;
    Ok(0)
}

fn strdup(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(0);
    }
    let text = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.intern(&text).map_err(Stop::Fault)
}
