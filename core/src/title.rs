//! What a title crate declares about itself.

use crate::clock::FlipCost;

/// One supported game, identified by the bytes of its entry image.
///
/// The roster key is the name of the title's binary and of its data-root
/// directory. Everything else on this struct is a declared value: a chosen
/// input the guest reads as fact, carried with the reason for that value and
/// no other.
pub struct Title {
    /// Roster key, e.g. `lego-rock-raiders`.
    pub key: &'static str,
    /// What the declared machine charges its clock for presenting one frame.
    pub flip_cost: FlipCost,
}
