//! lego-legacy core: the engine shared by every title on the roster.
//!
//! Nothing here is keyed to a title. A title crate supplies a [`Title`] and
//! calls [`run::main`]; everything else in a run happens through this crate.

pub mod avi;
pub mod cabinet;
pub mod cli;
pub mod clock;
pub mod com;
pub mod cpu;
pub mod disc;
pub mod dls;
pub mod explode;
pub mod guest;
pub mod hash;
pub mod host;
pub mod inflate;
pub mod install;
pub mod is3;
pub mod iv50;
pub mod loader;
pub mod machine;
pub mod pe;
pub mod png;
pub mod raster;
pub mod repl;
pub mod roster;
pub mod route;
pub mod run;
pub mod runlog;
pub mod shim;
pub mod snap;
pub mod sound;
pub mod spin;
pub mod stub;
pub mod synth;
pub mod testkit;
pub mod title;
pub mod win32;

pub use title::Title;
