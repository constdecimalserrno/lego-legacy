//! The movie library.
//!
//! A title opens its own AVI files through this and asks for frames. What it
//! gets depends on whether the emulator can decode what is inside them: Rock
//! Raiders' movies are Indeo Video 5, which nothing here decodes yet, so the
//! library opens and then honestly cannot.

use super::{Convention, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "AVIFIL32",
        name: "AVIFileInit",
        args: 0,
        convention: Convention::Stdcall,
        run: init,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIFileExit",
        args: 0,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIFileOpenA",
        args: 4,
        convention: Convention::Stdcall,
        run: open,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIFileRelease",
        args: 1,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIFileGetStream",
        args: 4,
        convention: Convention::Stdcall,
        run: no_stream,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamRelease",
        args: 1,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamGetFrameOpen",
        args: 2,
        convention: Convention::Stdcall,
        run: no_frames,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamGetFrame",
        args: 2,
        convention: Convention::Stdcall,
        run: no_frames,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamGetFrameClose",
        args: 1,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamInfoA",
        args: 3,
        convention: Convention::Stdcall,
        run: no_stream,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamStart",
        args: 1,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "AVIFIL32",
        name: "AVIStreamLength",
        args: 1,
        convention: Convention::Stdcall,
        run: nothing,
    },
];

fn nothing(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

fn init(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

/// What the library answers when it cannot do something.
const AVIERR_UNSUPPORTED: u32 = 0x8004_4065;
const AVIERR_FILEOPEN: u32 = 0x8004_4070;

fn open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The file is there; what is inside it is Indeo Video 5, and nothing here
    // decodes that yet. Saying so is better than opening it and handing back
    // frames that are not the movie.
    let name = ctx.string(args[1]).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "movie {name} not opened: nothing here decodes Indeo Video 5 yet"
    ));
    ctx.state.movies_refused += 1;
    Ok(AVIERR_FILEOPEN)
}

fn no_stream(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(AVIERR_UNSUPPORTED)
}

fn no_frames(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A null frame: the caller is expected to check, and a title that does not
    // will stop on a null rather than draw something that is not its movie.
    Ok(0)
}
