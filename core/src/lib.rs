//! lego-legacy core: the engine shared by every title on the roster.
//!
//! Nothing here is keyed to a title. A title crate supplies a [`Title`] and
//! calls [`run::main`]; everything else in a run happens through this crate.

pub mod cabinet;
pub mod cli;
pub mod clock;
pub mod cpu;
pub mod disc;
pub mod guest;
pub mod hash;
pub mod host;
pub mod inflate;
pub mod install;
pub mod machine;
pub mod pe;
pub mod repl;
pub mod roster;
pub mod route;
pub mod run;
pub mod runlog;
pub mod shim;
pub mod snap;
pub mod stub;
pub mod testkit;
pub mod title;
pub mod win32;

pub use title::Title;
