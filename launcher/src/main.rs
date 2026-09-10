//! `lego-legacy`: the one front door.
//!
//! Detecting pressings and starting a title is #15. Until then this names the
//! roster and the binaries that run it, rather than pretending to more.

use std::process::ExitCode;

fn main() -> ExitCode {
    println!("lego-legacy: the roster, in the order it is being taken:");
    for key in [
        "lego-rock-raiders",
        "hotwheels-stunt-track-driver",
        "lego-alpha-team",
        "lego-racers",
    ] {
        println!("  {key}");
    }
    println!("\nDetecting pressings and starting one is not built yet (#15).");
    println!("Run a title's own binary for now, e.g. `cargo run -p lego-rock-raiders -- --headless --stub`.");
    ExitCode::SUCCESS
}
