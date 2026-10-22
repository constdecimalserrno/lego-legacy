//! Reading a pressing.
//!
//! A rip is an ISO-9660 image of the disc's data track, and, for a pressing
//! whose Red Book audio is played, the raw rip of the whole disc and the cue
//! sheet that describes it. Nothing here is ever written: the rip stays
//! exactly as the player made it, and everything the guest may change lives
//! in the profile instead.

use crate::pe::u32_at;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One sector of an ISO-9660 image.
const SECTOR: u64 = 2048;

/// Where the volume descriptors start.
const FIRST_DESCRIPTOR: u64 = 16;

/// What went wrong reaching a pressing.
#[derive(Debug)]
pub enum DiscError {
    /// No rip where one was expected. Names the path so the answer is
    /// actionable rather than mysterious.
    Missing(PathBuf),
    /// The file is not an ISO-9660 image.
    NotAnIso(PathBuf),
    /// The image records more than it holds: a directory record that runs
    /// past its own length, or a file or directory that runs past the image.
    Damaged(PathBuf),
    /// The image has no such file.
    NoSuchFile {
        iso: PathBuf,
        path: String,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for DiscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(p) => write!(
                f,
                "no rip at {}: the emulator carries no game data, so a pressing has to be \
                 ripped from your own disc",
                p.display()
            ),
            Self::NotAnIso(p) => write!(f, "{} is not an ISO-9660 image", p.display()),
            Self::Damaged(p) => write!(
                f,
                "{} is damaged: it records more than it holds, so the disc has to be ripped again",
                p.display()
            ),
            Self::NoSuchFile { iso, path } => {
                write!(f, "{} holds no {path}", iso.display())
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DiscError {}

impl From<std::io::Error> for DiscError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// One name in a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name with its version suffix removed: `LEGORR.EXE`, not
    /// `LEGORR.EXE;1`. Read through the disc's Joliet tree, its long name:
    /// `Chess Bumper01.bmp`.
    pub name: String,
    pub sector: u32,
    pub len: u32,
    pub is_dir: bool,
}

/// A pressing's data track, open for reading.
pub struct Iso {
    file: File,
    path: PathBuf,
    /// How many bytes the image holds, which nothing it records may run past.
    size: u64,
    root: (u32, u32),
    /// The Joliet tree's root, once the disc is read through it.
    joliet: Option<(u32, u32)>,
    /// A name asked of a disc read through its Joliet tree that only the
    /// primary tree records, until the machine stops at it.
    wall: Option<String>,
    /// The volume's own name for itself.
    pub label: String,
}

impl Iso {
    /// Open an image and read its primary volume descriptor.
    ///
    /// # Errors
    /// When the file is absent, unreadable, or not an ISO-9660 image.
    pub fn open(path: &Path) -> Result<Self, DiscError> {
        if !path.is_file() {
            return Err(DiscError::Missing(path.to_path_buf()));
        }
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut pvd = [0u8; SECTOR as usize];
        file.seek(SeekFrom::Start(FIRST_DESCRIPTOR * SECTOR))?;
        file.read_exact(&mut pvd)?;
        if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
            return Err(DiscError::NotAnIso(path.to_path_buf()));
        }
        let record = &pvd[156..190];
        let root = (
            u32_at(record, 2).unwrap_or(0),
            u32_at(record, 10).unwrap_or(0),
        );
        let label = String::from_utf8_lossy(&pvd[40..72]).trim().to_owned();
        Ok(Self {
            file,
            path: path.to_path_buf(),
            size,
            root,
            joliet: None,
            wall: None,
            label,
        })
    }

    /// Answer the disc's names from its Joliet tree from now on, as Windows
    /// 95's CDFS does, where the disc has one; whether it has (#144).
    ///
    /// The Joliet specification (Microsoft, 1995) records a second tree beside
    /// the primary one, under a supplementary volume descriptor whose escape
    /// sequences are `%/@`, `%/C` or `%/E`, its three levels of UCS-2: names
    /// of up to sixty-four characters, two big-endian bytes each.
    ///
    /// The primary tree's 8.3 names are not answered. Nothing public says
    /// whether Windows 95's CDFS answers them, or 8.3 names of its own making
    /// as NT's does, and the two differ on the roster's own rips: #144's open
    /// question q2. A name only the primary tree records is a wall
    /// ([`Iso::wall`]), not a guess.
    ///
    /// # Errors
    /// When the descriptors run past the image.
    pub fn read_joliet(&mut self) -> Result<bool, DiscError> {
        // The descriptors follow the primary one, a sector each, up to a
        // terminator, type 255 (ECMA-119 6.7.1). A supplementary one is type
        // 2, with its escape sequences from byte 89 and its root directory's
        // record from byte 157, as the primary descriptor has its own (8.5).
        for n in FIRST_DESCRIPTOR + 1.. {
            let d = self.read_at(n * SECTOR, SECTOR as usize)?;
            if &d[1..6] != b"CD001" || d[0] == 255 {
                return Ok(false);
            }
            if d[0] == 2 && matches!(&d[88..91], b"%/@" | b"%/C" | b"%/E") {
                let record = &d[156..190];
                self.joliet = Some((
                    u32_at(record, 2).unwrap_or(0),
                    u32_at(record, 10).unwrap_or(0),
                ));
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The first name asked of a disc read through its Joliet tree that only
    /// its primary tree records, once: what the machine stops at (#144).
    pub fn wall(&mut self) -> Option<String> {
        self.wall.take()
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What a directory holds, in the order the image records it.
    ///
    /// The order is the image's own and is passed through untouched: a guest
    /// that enumerates a directory must see the same sequence on every run,
    /// and the image is the only place that order can honestly come from.
    ///
    /// # Errors
    /// When the image cannot be read.
    pub fn list(&mut self, dir: &str) -> Result<Vec<Entry>, DiscError> {
        let (sector, len) = self.locate_dir(dir)?;
        self.read_dir(sector, len, self.joliet.is_some())
    }

    /// Read a whole file out of the image.
    ///
    /// # Errors
    /// When the image holds no such file.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, DiscError> {
        let (start, len) = self.lies(path)?;
        self.read_at(start, len as usize)
    }

    /// Where a file lies in the image: its first byte, and how long it is.
    /// What a file read where it lies is read through, rather than whole
    /// (#116). A file the image records running past its own end is damaged.
    ///
    /// # Errors
    /// When the image holds no such file, or the file runs past the image.
    pub fn lies(&mut self, path: &str) -> Result<(u64, u32), DiscError> {
        let entry = self.entry(path)?;
        if entry.is_dir {
            return Err(DiscError::NoSuchFile {
                iso: self.path.clone(),
                path: path.to_owned(),
            });
        }
        let start = u64::from(entry.sector) * SECTOR;
        if start + u64::from(entry.len) > self.size {
            return Err(DiscError::Damaged(self.path.clone()));
        }
        Ok((start, entry.len))
    }

    /// Read part of a file out of the image.
    ///
    /// What a large archive is read through: the whole of it need not be in
    /// memory to unpack one entry.
    ///
    /// # Errors
    /// When the image holds no such file, or ends before the span does.
    pub fn read_range(
        &mut self,
        path: &str,
        offset: u32,
        len: usize,
    ) -> Result<Vec<u8>, DiscError> {
        let entry = self.entry(path)?;
        let start = u64::from(entry.sector) * SECTOR + u64::from(offset);
        let room = (entry.len as usize).saturating_sub(offset as usize);
        // A rip cut short is damaged, not a short read (#169).
        self.read_at(start, len.min(room))
    }

    /// Whether the image holds this path.
    ///
    /// # Errors
    /// When the image cannot be read at all.
    pub fn has(&mut self, path: &str) -> Result<bool, DiscError> {
        match self.entry(path) {
            Ok(_) => Ok(true),
            Err(DiscError::NoSuchFile { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// One named entry, found by walking the directories on the way to it.
    ///
    /// # Errors
    /// When no such entry exists.
    pub fn entry(&mut self, path: &str) -> Result<Entry, DiscError> {
        let Some(joliet) = self.joliet else {
            return self.walk(self.root, false, path);
        };
        let found = self.walk(joliet, true, path);
        // ponytail: a path that mixes a long directory name and an 8.3 name
        // is in neither tree, so it is not found rather than a wall; name it
        // too when a title asks for one.
        if matches!(found, Err(DiscError::NoSuchFile { .. }))
            && self.walk(self.root, false, path).is_ok()
        {
            self.wall.get_or_insert_with(|| {
                format!(
                    "CDFS: {path:?} on {}, a name only the primary tree records, on a disc \
                     read through its Joliet tree (#144 q2: whether Windows 95 answers it)",
                    self.path.display()
                )
            });
        }
        found
    }

    /// One entry, walked to from one tree's root. Every name on the way but
    /// the last is a directory's: a path through a file is no path.
    fn walk(&mut self, root: (u32, u32), ucs2: bool, path: &str) -> Result<Entry, DiscError> {
        let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
        let mut here = root;
        let mut found = None;
        while let Some(part) = parts.next() {
            let last = parts.peek().is_none();
            let entries = self.read_dir(here.0, here.1, ucs2)?;
            let hit = entries
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part) && (last || e.is_dir))
                .ok_or_else(|| DiscError::NoSuchFile {
                    iso: self.path.clone(),
                    path: path.to_owned(),
                })?;
            if last {
                found = Some(hit);
            } else {
                here = (hit.sector, hit.len);
            }
        }
        found.ok_or_else(|| DiscError::NoSuchFile {
            iso: self.path.clone(),
            path: path.to_owned(),
        })
    }

    fn locate_dir(&mut self, dir: &str) -> Result<(u32, u32), DiscError> {
        if dir.trim_matches('/').is_empty() {
            return Ok(self.joliet.unwrap_or(self.root));
        }
        let entry = self.entry(dir)?;
        if !entry.is_dir {
            return Err(DiscError::NoSuchFile {
                iso: self.path.clone(),
                path: dir.to_owned(),
            });
        }
        Ok((entry.sector, entry.len))
    }

    /// The bytes from one place in the image, refused when the image is too
    /// short to hold them: a length read from a damaged image can be four
    /// gigabytes.
    ///
    /// # Errors
    /// When the image is too short, or cannot be read.
    pub fn read_at(&mut self, start: u64, len: usize) -> Result<Vec<u8>, DiscError> {
        if start + len as u64 > self.size {
            return Err(DiscError::Damaged(self.path.clone()));
        }
        let mut bytes = vec![0u8; len];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// A directory's records, their names the Joliet tree's UCS-2 where
    /// `ucs2` says so.
    fn read_dir(&mut self, sector: u32, len: u32, ucs2: bool) -> Result<Vec<Entry>, DiscError> {
        let data = self.read_at(u64::from(sector) * SECTOR, len as usize)?;

        let mut out = Vec::new();
        let mut at = 0usize;
        while at < data.len() {
            let record_len = data[at] as usize;
            if record_len == 0 {
                // A directory's records never straddle a sector: the rest of
                // this one is padding.
                at = (at / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            // A record is thirty-three bytes and then its name, and it ends
            // inside its directory: one that does not is a damaged rip.
            let name_len = data.get(at + 32).map_or(0, |n| usize::from(*n));
            let Some(record) = data
                .get(at..at + record_len)
                .filter(|r| r.len() >= 33 + name_len)
            else {
                return Err(DiscError::Damaged(self.path.clone()));
            };
            let raw = &record[33..33 + name_len];
            // The first two entries of every directory are itself and its
            // parent, spelled as a single zero or one byte, in both trees.
            if raw != b"\0" && raw != b"\x01" {
                let name = if ucs2 {
                    let units: Vec<u16> = raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_be_bytes(*c))
                        .collect();
                    String::from_utf16_lossy(&units)
                } else {
                    String::from_utf8_lossy(raw).into_owned()
                };
                let name = name.split(';').next().unwrap_or(&name).to_owned();
                out.push(Entry {
                    name,
                    sector: u32_at(record, 2).unwrap_or(0),
                    len: u32_at(record, 10).unwrap_or(0),
                    is_dir: record[25] & 2 != 0,
                });
            }
            at += record_len;
        }
        Ok(out)
    }
}

/// Where a title's pressings are ripped to.
///
/// `LEGO_LEGACY_GAME_DATA` moves the root; the default is `game-data` inside
/// the working directory, which is where the rips live in a checkout.
#[must_use]
pub fn game_data_root() -> PathBuf {
    std::env::var_os("LEGO_LEGACY_GAME_DATA")
        .map_or_else(|| PathBuf::from("game-data"), PathBuf::from)
}

/// Where a title keeps what its guest writes: `<root>/<disc_dir>/profile`,
/// beside its rips, whichever of its pressings it runs from.
#[must_use]
pub fn profile_dir(title_dir: &str) -> PathBuf {
    game_data_root().join(title_dir).join("profile")
}

/// The image of one pressing: `<root>/<disc_dir>/<pressing>/<pressing>.iso`,
/// where `disc_dir` is the title's own ([`crate::title::EntryImage::disc_dir`]).
#[must_use]
pub fn pressing_path(title: &str, pressing: &str) -> PathBuf {
    game_data_root()
        .join(title)
        .join(pressing)
        .join(format!("{pressing}.iso"))
}

/// Open one pressing of a title.
///
/// # Errors
/// When the rip is absent or unreadable.
pub fn open_pressing(title: &str, pressing: &str) -> Result<Iso, DiscError> {
    Iso::open(&pressing_path(title, pressing))
}

/// One sector of a raw rip. On an audio track it is a seventy-fifth of a
/// second of 44.1 kHz stereo sixteen-bit samples.
const RAW_SECTOR: u64 = 2352;

/// Sectors in a second of Red Book audio.
pub const SECTORS_PER_SECOND: u64 = 75;

// A disc's audio reaches the mix two frames to one.
const _: () = assert!(crate::sound::RATE * 2 == 44_100);

/// One audio track, by the sectors of its rip: where its pregap starts, which
/// a drive already reports as the track, and where its music starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    pub number: u32,
    pub pregap: u64,
    pub start: u64,
}

/// A pressing's Red Book audio: its raw rip, and its audio tracks as its cue
/// sheet lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedBook {
    bin: PathBuf,
    pub tracks: Vec<Track>,
    /// How many sectors the whole rip holds: where the disc ends.
    pub sectors: u64,
}

impl RedBook {
    /// The audio tracks a cue sheet lists, by their `INDEX` lines: `00` where
    /// the pregap starts, `01` where the music does, each as minutes, seconds
    /// and sectors from the start of the rip.
    #[must_use]
    pub fn parse_cue(cue: &str) -> Vec<Track> {
        let at = |msf: &str| -> Option<u64> {
            let parts: Vec<u64> = msf
                .split(':')
                .map(|p| p.parse().ok())
                .collect::<Option<_>>()?;
            match parts[..] {
                [m, s, f] => Some((m * 60 + s) * SECTORS_PER_SECOND + f),
                _ => None,
            }
        };
        let mut tracks: Vec<Track> = Vec::new();
        let mut audio = None;
        for line in cue.lines() {
            match line.split_whitespace().collect::<Vec<_>>()[..] {
                ["TRACK", number, kind] => {
                    audio = (kind == "AUDIO").then(|| number.parse().ok()).flatten();
                }
                ["INDEX", index, msf] => {
                    let (Some(number), Some(sector)) = (audio, at(msf)) else {
                        continue;
                    };
                    if tracks.last().is_none_or(|t| t.number != number) {
                        tracks.push(Track {
                            number,
                            pregap: sector,
                            start: sector,
                        });
                    }
                    let track = tracks.last_mut().expect("pushed");
                    if index == "01" {
                        track.start = sector;
                    }
                }
                _ => {}
            }
        }
        tracks
    }

    /// The track a sector is in, as a drive reports it.
    #[must_use]
    pub fn track_at(&self, sector: u64) -> Option<u32> {
        self.tracks
            .iter()
            .rev()
            .find(|t| sector >= t.pregap)
            .map(|t| t.number)
    }

    /// Every sample from a sector to the end of the disc, stereo at the mix's
    /// rate: each two of the disc's frames averaged into one.
    // ponytail: averaging two frames is the whole of the filter, so what is
    // above the mix's half rate folds back a little; a better one is a few
    // more taps when an ear hears it.
    ///
    /// # Errors
    /// When the rip cannot be read.
    pub fn samples_from(&self, sector: u64) -> Result<Vec<i16>, DiscError> {
        let mut file = File::open(&self.bin)?;
        file.seek(SeekFrom::Start(sector * RAW_SECTOR))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes
            .as_chunks::<8>()
            .0
            .iter()
            .flat_map(|f| {
                let s = |i: usize| i32::from(i16::from_le_bytes([f[i], f[i + 1]]));
                [((s(0) + s(4)) / 2) as i16, ((s(2) + s(6)) / 2) as i16]
            })
            .collect())
    }
}

/// Open one pressing's Red Book audio: the cue sheet beside its image, and
/// the raw rip of the whole disc beside that.
///
/// # Errors
/// When the cue sheet or its rip is absent or unreadable.
pub fn open_red_book(title: &str, pressing: &str) -> Result<RedBook, DiscError> {
    let image = pressing_path(title, pressing);
    let (cue_path, bin) = (image.with_extension("cue"), image.with_extension("bin"));
    // A file not there is a rip not ripped; any other failure is the host's.
    let missing = |path: &Path, e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => DiscError::Missing(path.to_path_buf()),
        _ => DiscError::Io(e),
    };
    let cue = std::fs::read_to_string(&cue_path).map_err(|e| missing(&cue_path, e))?;
    let len = std::fs::metadata(&bin).map_err(|e| missing(&bin, e))?.len();
    Ok(RedBook {
        tracks: RedBook::parse_cue(&cue),
        sectors: len / RAW_SECTOR,
        bin,
    })
}

#[cfg(test)]
mod red_book {
    use super::{RedBook, Track};

    #[test]
    fn a_cue_sheet_gives_each_audio_track_its_pregap_and_its_start() {
        // Rock Raiders' original pressing, as ripped.
        let cue = "FILE \"rock-raiders.bin\" BINARY
  TRACK 01 MODE1/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 00 60:51:09
    INDEX 01 60:53:09
  TRACK 03 AUDIO
    INDEX 00 63:54:05
    INDEX 01 63:56:05
";
        let tracks = RedBook::parse_cue(cue);
        assert_eq!(
            tracks,
            [
                Track {
                    number: 2,
                    pregap: 3651 * 75 + 9,
                    start: 3653 * 75 + 9,
                },
                Track {
                    number: 3,
                    pregap: 3834 * 75 + 5,
                    start: 3836 * 75 + 5,
                },
            ]
        );
        let disc = RedBook {
            bin: "rock-raiders.bin".into(),
            tracks,
            sectors: 400_000,
        };
        assert_eq!(disc.track_at(1000), None, "the data track is no audio");
        assert_eq!(disc.track_at(3653 * 75 + 9), Some(2));
        assert_eq!(
            disc.track_at(3834 * 75 + 5),
            Some(3),
            "a pregap is its track's"
        );
    }
}
