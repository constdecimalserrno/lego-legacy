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
    /// disc, or the name the archive it is kept in gives it.
    pub path: &'static str,
    /// Whether it is on the disc as a file of its own or kept in one of the
    /// pressing's archives.
    pub kept: Kept,
    /// The bytes that make this title this title.
    pub sha256: &'static str,
    /// Why this pressing and not the other.
    pub why: &'static str,
}

impl EntryImage {
    /// Read this image out of a pressing, wherever it is kept.
    ///
    /// # Errors
    /// When the pressing does not hold it, or the archive it is kept in will
    /// not give it up.
    pub fn read(&self, iso: &mut crate::disc::Iso) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        match self.kept {
            Kept::Loose => Ok(iso.read(self.path)?),
            Kept::Imploded(archive) => {
                let bytes = iso.read(archive)?;
                Ok(crate::is3::Archive::open(&bytes)?.read(&bytes, self.path)?)
            }
            Kept::Cabinet => {
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
    }
}

/// Where a pressing keeps its entry image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// On the disc as a file of its own.
    Loose,
    /// In the pressing's cabinet, by the name its header gives it. Alpha Team
    /// ships nothing of itself loose (#53).
    Cabinet,
    /// Imploded in the InstallShield 3 archive on the disc named here, by its
    /// path in it. LEGO Chess ships nothing of itself loose either (#143).
    Imploded(&'static str),
}

/// The machine a title is told it is running on.
///
/// Every field is a declared value: chosen for this title, carried with the
/// reason for that value, and never measured from the host. A run that read
/// the host's own processor or memory would replay differently on another
/// machine.
pub struct Declared {
    /// Which Windows the machine runs, as its version calls answer.
    pub windows: Windows,
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
    /// Whether the machine's CD-ROM drive answers the disc's long names from
    /// its Joliet tree, as Windows 95's CDFS does, a name only the primary
    /// tree records being a wall; or the primary tree's names alone (#144).
    pub joliet: bool,
    /// What the machine calls itself.
    pub computer: &'static str,
    /// The libraries the machine does not have, each named with its
    /// extension and with why: a card it has not got, or a file the title
    /// never shipped. `LoadLibraryA` tells a title asking for one that it is
    /// not there, as Windows does; a library that is not on this list, nor
    /// the title's own, nor one core writes, is a wall (#146).
    pub absent_libraries: &'static [(&'static str, &'static str)],
    /// Whether the machine has a floppy drive, A:, with no disk in it: a
    /// drive `GetLogicalDrives` and `GetDriveTypeA` name, and one that
    /// fails what reads it as an empty drive does (#145). Beside it the
    /// machine always has its fixed disk, C:, and its CD-ROM, D:.
    pub floppy: bool,
    /// Why these values and no others.
    pub why: &'static str,
}

/// A Windows, as `GetVersion` and `GetVersionExA` answer for it (#129).
///
/// Windows 9x packs these unlike Windows NT. `GetVersion`'s top bit is set,
/// which is how a title tells it from NT, and the rest of its high word,
/// where NT puts the build, Microsoft documents as reserved. `GetVersionExA`
/// carries the build in the low word of `dwBuildNumber`, with the major and
/// minor versions in its high word.
pub struct Windows {
    /// What `GetVersion` answers.
    pub version: u32,
    /// `OSVERSIONINFOA`'s `dwBuildNumber`.
    pub build: u32,
    /// `OSVERSIONINFOA`'s `szCSDVersion`.
    pub csd: &'static str,
}

/// Windows 98 Second Edition, 4.10.2222: `0xC0000A04`, build `0x040A08AE`
/// and `" A "`, from Wine's table for it (`dlls/ntdll/version.c`; its
/// `misc/version.c` of 2003 gives the `GetVersion` word as a literal).
/// AutoCAD 2000's crash dumps from real Windows 98 machines, posted online,
/// record the same `WinVer=0xC0000A04`, and KernelEx, which runs on Windows
/// 98, compares `GetVersion` against it. MSDN's `OSVERSIONINFO` documents the
/// build's layout and an `A` for Second Edition.
pub const WINDOWS_98_SE: Windows = Windows {
    version: 0xc000_0a04,
    build: 0x040a_08ae,
    csd: " A ",
};

/// Windows 95 as it was sold in a box, 4.00.950: `0xC0000004`, build
/// `0x040003B6` and no CSD string, from Wine's table for it as above, which
/// has given that `GetVersion` word as a literal since 1998
/// (`misc/version.c`, wine-980301). Microsoft's NT compatibility shim for
/// Windows 95, as ReactOS reimplements and tests it, answers `0xC3B60004`
/// with a bare build of 950 instead, a build that contradicts MSDN's layout;
/// Wine's is the answer recorded as Windows 95's own.
pub const WINDOWS_95: Windows = Windows {
    version: 0xc000_0004,
    build: 0x0400_03b6,
    csd: "",
};

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
    /// Expanded out of an InstallShield 3 archive on the disc, every member
    /// where the archive puts it, under the install directory (#143); and at
    /// that directory's root, beside them, each member of the setup's other
    /// archives that its script copies there on its own, as archive and path.
    Expanded {
        archive: &'static str,
        beside: &'static [(&'static str, &'static str)],
    },
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
            kept: Kept::Loose,
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
            // The stub opens no disc.
            joliet: false,
            computer: "LEGO-LEGACY",
            // What core's own tests ask for as a library this machine lacks.
            absent_libraries: &[(
                "glide2x.dll",
                "3dfx's Glide, and the declared machine has no 3dfx card",
            )],
            floppy: false,
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

/// How a title reads and moves the mouse pointer (#311).
///
/// Declared per title in its crate's [`TitleGuide`] so the control plane and
/// external agents know whether to issue absolute coordinates (`move`/`click`),
/// poll-released clicks (`click_poll`), or scaled relative deltas
/// (`mouse_delta`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseMode {
    /// Absolute pointer coordinates kept by the title; standard `move`/`click`
    /// work directly.
    AbsoluteKept,
    /// Title hides the system cursor and draws its own pointer at absolute
    /// `WM_MOUSEMOVE` coordinates; `requires_poll_release` flags titles whose
    /// UI loops wait for button or key release without flipping (`click_poll`).
    SoftwareAbsolute { requires_poll_release: bool },
    /// Title re-centers the pointer each flip and steers a software cursor by
    /// relative deltas mapped onto `virtual_span`; `mouse_delta` scales by
    /// `(virtual_span.0 / resolution.0, virtual_span.1 / resolution.1)`.
    RelativeSteered {
        virtual_span: (u32, u32),
        recenter: (i32, i32),
        start_corner: (i32, i32),
    },
}

/// Recommended observation/action cadence for an agent driving a title (#311).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentCadence {
    /// Pause at flip boundaries while thinking; after acting, wait up to
    /// `settle_ms` declared milliseconds or `settle_flips` flips to settle.
    PauseToThink { settle_ms: u64, settle_flips: u64 },
    /// Run a burst of `burst_ms` declared milliseconds, then pause to observe
    /// and decide the next command.
    BurstThenPause { burst_ms: u64 },
    /// Step tightly in small `flips_per_step` increments for reflex steering
    /// and stunt timing.
    MicroStep { flips_per_step: u64 },
}

/// One documented input control, gesture, or coordinate hotspot (#311).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlHint {
    /// Key name(s), chord, mouse gesture, or hotspot coordinate.
    pub input: &'static str,
    /// Short label of what the control does.
    pub action: &'static str,
    /// Practical note, timing constraint, or hazard warning.
    pub notes: &'static str,
}

/// Coarse progression milestone from `CONTEXT.md` (#311).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Milestone {
    FirstLight,
    Playable,
    Complete,
    Credits,
}

impl Milestone {
    /// Canonical lowercase snake_case name used in JSON metadata.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstLight => "first_light",
            Self::Playable => "playable",
            Self::Complete => "complete",
            Self::Credits => "credits",
        }
    }
}

/// One concrete playable objective for a title (#311).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Goal {
    /// Stable identifier (e.g. `"mission-1"`).
    pub id: &'static str,
    /// Human-readable goal title.
    pub name: &'static str,
    /// Progression milestone this goal corresponds to.
    pub milestone: Milestone,
    /// Concrete description and completion criteria.
    pub description: &'static str,
}

/// Static, first-class per-title playability metadata declared by a title crate
/// (#311).
///
/// Strictly outside guest state (Golden Rules 2 & 4): never alters emulation
/// behaviour, while giving the control plane (`ControlHost`) and external
/// agents/spectators (`/api/meta`, `/api/controls`, `/api/goals`) accurate
/// knowledge of display timing, mouse regime, safety hazards, cadence,
/// controls, goals, and save files with zero title names in core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleGuide {
    /// Human-readable display name of the title (empty in [`Self::EMPTY`]).
    pub display_name: &'static str,
    /// One-line gameplay summary.
    pub summary: &'static str,
    /// Native gameplay framebuffer dimensions `(width, height)`.
    pub resolution: (u32, u32),
    /// Native gameplay colour depth in bits per pixel (`8` or `16`).
    pub bpp: u32,
    /// Intended presentation rate in Hz (e.g. `60` or `30`).
    pub refresh_hz: u32,
    /// Pointer regime and relative-steering parameters.
    pub mouse_mode: MouseMode,
    /// Whether pressing `Escape` immediately quits the title (arms the
    /// `ControlHost` safety guard requiring `force: true` to send `Escape`).
    pub escape_quits: bool,
    /// DirectInput key names that skip intros or cutscenes.
    pub skip_keys: &'static [&'static str],
    /// Recommended agent observation/stepping cadence.
    pub cadence: AgentCadence,
    /// Keybindings, mouse gestures, and timing/hotspot notes.
    pub controls: &'static [ControlHint],
    /// Playability hazards and pitfalls an agent or player should avoid.
    pub hazards: &'static [&'static str],
    /// Concrete playable objectives and milestones.
    pub goals: &'static [Goal],
    /// Profile save file paths (relative to the profile/install tree or full
    /// guest paths).
    pub save_files: &'static [&'static str],
}

impl TitleGuide {
    /// Empty fallback guide for tests or callers that invoke
    /// [`crate::run::main`] without supplying a [`TitleGuide`].
    pub const EMPTY: Self = Self {
        display_name: "",
        summary: "",
        resolution: (640, 480),
        bpp: 16,
        refresh_hz: 60,
        mouse_mode: MouseMode::AbsoluteKept,
        escape_quits: false,
        skip_keys: &[],
        cadence: AgentCadence::PauseToThink {
            settle_ms: 500,
            settle_flips: 15,
        },
        controls: &[],
        hazards: &[],
        goals: &[],
        save_files: &[],
    };

    /// Whether this guide carries no populated metadata (`display_name` empty).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.display_name.is_empty()
    }

    /// Relative pointer scaling hint `(scale_x, scale_y)` derived from
    /// `virtual_span / resolution` when [`Self::mouse_mode`] is
    /// [`MouseMode::RelativeSteered`].
    #[must_use]
    pub const fn relative_steered_scale(&self) -> Option<(f64, f64)> {
        match self.mouse_mode {
            MouseMode::RelativeSteered { virtual_span, .. }
                if self.resolution.0 > 0
                    && self.resolution.1 > 0
                    && virtual_span.0 > 0
                    && virtual_span.1 > 0 =>
            {
                Some((
                    virtual_span.0 as f64 / self.resolution.0 as f64,
                    virtual_span.1 as f64 / self.resolution.1 as f64,
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_guide_empty_and_relative_steered_scale() {
        assert!(TitleGuide::EMPTY.is_empty());
        assert_eq!(TitleGuide::EMPTY.relative_steered_scale(), None);
        const { assert!(!TitleGuide::EMPTY.escape_quits) };
        assert_eq!(Milestone::FirstLight.as_str(), "first_light");
        assert_eq!(Milestone::Playable.as_str(), "playable");
        assert_eq!(Milestone::Complete.as_str(), "complete");
        assert_eq!(Milestone::Credits.as_str(), "credits");

        const STEERED: TitleGuide = TitleGuide {
            display_name: "Steered Fixture",
            summary: "Fixture",
            resolution: (640, 480),
            bpp: 16,
            refresh_hz: 30,
            mouse_mode: MouseMode::RelativeSteered {
                virtual_span: (1280, 1280),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            escape_quits: false,
            skip_keys: &["Space"],
            cadence: AgentCadence::PauseToThink {
                settle_ms: 1000,
                settle_flips: 30,
            },
            controls: &[],
            hazards: &[],
            goals: &[],
            save_files: &[],
        };
        assert!(!STEERED.is_empty());
        let (sx, sy) = STEERED.relative_steered_scale().expect("steered scale");
        assert!((sx - 2.0).abs() < 1e-12);
        assert!((sy - (1280.0 / 480.0)).abs() < 1e-12);
    }

    #[test]
    fn each_declared_windows_answers_what_that_windows_answered() {
        // #129: the words each Windows gave, from the sources beside them.
        for (windows, version, build, csd) in [
            (&WINDOWS_98_SE, 0xc000_0a04, 0x040a_08ae, " A "),
            (&WINDOWS_95, 0xc000_0004, 0x0400_03b6, ""),
        ] {
            assert_eq!(
                (windows.version, windows.build, windows.csd),
                (version, build, csd)
            );
            assert_ne!(windows.version >> 31, 0, "a clear top bit says Windows NT");
            // `GetVersion` packs minor over major; `dwBuildNumber`'s high word
            // packs them the other way round.
            let (major, minor) = (windows.version & 0xff, (windows.version >> 8) & 0xff);
            assert_eq!(windows.build >> 16, major << 8 | minor);
        }
    }
}
