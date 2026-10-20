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
    /// Where the executable sits inside that pressing's image: a path on the
    /// disc, or — when `in_cabinet` — the name the pressing's cabinet gives it.
    pub path: &'static str,
    /// Whether the image is inside the pressing's cabinet rather than on the
    /// disc as a file of its own. Alpha Team ships nothing of itself loose
    /// (#53).
    pub in_cabinet: bool,
    /// The bytes that make this title this title.
    pub sha256: &'static str,
    /// Why this pressing and not the other.
    pub why: &'static str,
}

impl EntryImage {
    /// Read this image out of a pressing, wherever it is kept.
    ///
    /// # Errors
    /// When the pressing does not hold it, or its cabinet will not give it up.
    pub fn read(&self, iso: &mut crate::disc::Iso) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if !self.in_cabinet {
            return Ok(iso.read(self.path)?);
        }
        let header = iso.read("DATA1.HDR")?;
        let cabinet = crate::cabinet::Cabinet::open(&header)?;
        let entry = cabinet
            .find(self.path)
            .ok_or_else(|| format!("the cabinet does not hold {}", self.path))?
            .clone();
        let span = iso.read_range(
            &format!("DATA{}.CAB", entry.volume),
            entry.offset,
            entry.compressed as usize,
        )?;
        Ok(cabinet.unpack(&entry, &span)?)
    }
}

/// The machine a title is told it is running on.
///
/// Every field is a declared value: chosen for this title, carried with the
/// reason for that value, and never measured from the host. A run that read
/// the host's own processor or memory would replay differently on another
/// machine.
pub struct Declared {
    /// What `GetVersion` answers with, packed as Windows packs it.
    pub windows: u32,
    /// The processor this machine has, in cycles a second, one instruction to
    /// the cycle. It decides how much a declared second is worth in work,
    /// which is what a title that waits by reading the clock spends.
    pub hz: u64,
    /// Which part that processor is: what `cpuid` tells the title, and which
    /// flags it can toggle (#104).
    pub processor: crate::cpu::Processor,
    /// How much memory the machine says it has.
    pub megabytes: u32,
    /// The screen sizes the machine offers, as width, height and bit depth.
    pub modes: &'static [(u32, u32, u32)],
    /// What the machine calls itself.
    pub computer: &'static str,
    /// Why these values and no others.
    pub why: &'static str,
}

/// Windows 98 Second Edition: major 4, minor 10, build 2222, with the top bit
/// clear to say it is not Windows NT.
pub const WINDOWS_98_SE: u32 = 0x0a04 | (2222 << 16);

/// Windows 95 as it was sold in a box: major 4, minor 0, build 950, packed
/// as [`WINDOWS_98_SE`] is.
pub const WINDOWS_95: u32 = 0x0004 | (950 << 16);

/// How a title's files reached the declared machine's disc.
///
/// A declared value like every other: which of these it is comes from the
/// pressing, not from sniffing the disc for one and falling back to the
/// other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// Unpacked out of the pressing's InstallShield cabinet (#16).
    Cabinet,
    /// Copied whole out of a directory of the disc, which is what an
    /// installer does for a title that ships its files loose.
    Copied(&'static str),
}

/// One supported game, identified by the bytes of its entry image.
///
/// The roster key is the name of the title's binary, and the image is which
/// bytes it runs. Everything else on this struct is a declared value: a chosen
/// input the guest reads as fact, never measured from the host.
pub struct Title {
    /// Roster key, e.g. `lego-rock-raiders`.
    pub key: &'static str,
    /// The version of the crate that declares this title, which is
    /// `env!("CARGO_PKG_VERSION")` written there. A run's log names it beside
    /// core's, so a run directory says what produced it (ADR-0007).
    pub version: &'static str,
    /// What the declared machine charges its clock for presenting one frame.
    pub flip_cost: FlipCost,
    /// The executable this title runs.
    pub image: EntryImage,
    /// The pressing whose audio tracks this title plays, when it has any.
    pub red_book: Option<&'static str>,
    /// The machine this title is told it is running on.
    pub declared: Declared,
    /// How this title's own files got onto that machine's disc.
    pub installed: Installed,
    /// What this title's installer left in the registry for it to read back,
    /// as key, value name and string. A declared value like the rest: the
    /// installer is never run, so what it would have written is written down
    /// here instead.
    pub registry: &'static [(&'static str, &'static str, &'static str)],
    /// Where the declared machine has this title installed.
    ///
    /// A declared value: the guest reads this path back from the machine and
    /// resolves its own files against it, so it has to be the same on every
    /// run and on every host.
    pub install_dir: &'static str,
    /// The key a click is also handed to this title as, while one of its own
    /// movies is playing.
    ///
    /// A declared value, and a translation rather than a decision: a title
    /// that listens for a key to get past its intro and not for the mouse
    /// leaves a person who clicks there with nothing happening, so the
    /// machine hands it the key the person's click meant (#66). The machine
    /// never ends the movie itself — what the title does with the key is
    /// still the title's own rule, including refusing it.
    ///
    /// `None` for a title that takes the click itself, which Rock Raiders
    /// does (#67).
    pub click_in_a_movie: Option<u32>,
    /// How far this machine's pointer goes for how far the mouse went, across
    /// and down. One for one is the pointer a person's own hand is used to.
    ///
    /// A declared value, and the machine's own rather than the title's: on the
    /// machine a title shipped for, how much pointer there is in a push of the
    /// mouse is a setting of the machine, which is why Windows has one. What
    /// the title chooses is what it does with the pointer once it has it, and
    /// a title that takes a fraction of the movement needs the fraction put
    /// back here or a person's hand and the pointer they are watching never
    /// line up (#86).
    ///
    /// It is only ever used where the title draws its own pointer and the host
    /// is steering by how far the mouse went ([`crate::host::pointer::Steer`]);
    /// where the pointer is the person's own, its place is its own and there
    /// is nothing to scale.
    pub pointer_speed: (f64, f64),
}

impl Title {
    /// The title the tests run the stub guest as, which has no disc behind
    /// it. `--stub` on a title's own binary runs the stub guest on that
    /// title's declared machine instead.
    ///
    /// The stub never opens a pressing, so the image this declares is a
    /// description rather than a lookup: it says out loud that the stub is not
    /// a game.
    pub const STUB: Self = Self {
        key: "stub",
        // The stub guest is core's own, so its version is core's.
        version: env!("CARGO_PKG_VERSION"),
        flip_cost: FlipCost::Free,
        image: EntryImage {
            disc_dir: "",
            pressing: "",
            path: "",
            in_cabinet: false,
            sha256: "",
            why: "the stub guest is assembled by hand in this crate and comes off no disc",
        },
        red_book: None,
        declared: Declared {
            windows: WINDOWS_98_SE,
            hz: crate::clock::DECLARED_HZ,
            processor: crate::cpu::Processor::PentiumII,
            megabytes: 128,
            modes: &[(640, 480, 16)],
            computer: "LEGO-LEGACY",
            why: "the stub guest asks the machine nothing, and these are the roster's own \
                  baseline values rather than anything measured for it",
        },
        installed: Installed::Cabinet,
        registry: &[],
        install_dir: "C:\\STUB",
        click_in_a_movie: None,
        // The stub guest has no pointer of its own to draw.
        pointer_speed: (1.0, 1.0),
    };
}
