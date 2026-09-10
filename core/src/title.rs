//! What a title crate declares about itself.

use crate::clock::FlipCost;

/// The image a title actually runs, and where it comes from.
///
/// A title has one or two pressings and they are not interchangeable
/// (ADR-0006): one may wrap its executable in copy protection while the other
/// ships it plain, and only one may carry the soundtrack.
pub struct EntryImage {
    /// The directory under `game-data/` holding this title's pressings. It is
    /// the rip layout's own name, which is not always the roster key: the
    /// rips were made before the roster was, and renaming somebody's ripped
    /// discs to suit us would be the wrong way round.
    pub disc_dir: &'static str,
    /// The pressing directory under that.
    pub pressing: &'static str,
    /// Where the executable sits inside that pressing's image.
    pub path: &'static str,
    /// The bytes that make this title this title.
    pub sha256: &'static str,
    /// Why this pressing and not the other.
    pub why: &'static str,
}

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
    /// The executable this title runs.
    pub image: EntryImage,
    /// The pressing whose audio tracks this title plays, when it has any.
    pub red_book: Option<&'static str>,
}

impl Title {
    /// The machine's own test guest, which has no disc behind it.
    ///
    /// `--stub` never opens a pressing, so the image this declares is a
    /// description rather than a lookup: it says out loud that the stub is not
    /// a game.
    pub const STUB: Self = Self {
        key: "stub",
        flip_cost: FlipCost::Free,
        image: EntryImage {
            disc_dir: "",
            pressing: "",
            path: "",
            sha256: "",
            why: "the stub guest is assembled by hand in this crate and comes off no disc",
        },
        red_book: None,
    };
}
