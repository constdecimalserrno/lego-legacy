//! Consumer-only catalog of all supported titles in the workspace (#407).
//!
//! Each title crate is the single source of truth for its own [`Title`]
//! declaration (identity, display name, image, audio bank, declared machine,
//! and playability guide). `core` stays strictly title-agnostic (Golden Rule 4),
//! while consumer crates such as `lego-legacy-launcher` read [`TITLES`] here.

pub use hotwheels_stunt_track_driver::HOT_WHEELS_STUNT_TRACK_DRIVER;
pub use lego_alpha_team::LEGO_ALPHA_TEAM;
pub use lego_chess::{GM16, LEGO_CHESS};
pub use lego_island::LEGO_ISLAND;
use lego_legacy_core::Title;
pub use lego_racers::LEGO_RACERS;
pub use lego_rock_raiders::ROCK_RAIDERS;
pub use legoland::LEGOLAND;

/// Every supported title in the workspace, in the order they joined:
/// LEGO Rock Raiders, Hot Wheels Stunt Track Driver, LEGO Alpha Team,
/// LEGO Island, LEGO Chess, LEGO Racers, and LEGOLAND.
pub const TITLES: &[&Title] = &[
    &ROCK_RAIDERS,
    &HOT_WHEELS_STUNT_TRACK_DRIVER,
    &LEGO_ALPHA_TEAM,
    &LEGO_ISLAND,
    &LEGO_CHESS,
    &LEGO_RACERS,
    &LEGOLAND,
];
