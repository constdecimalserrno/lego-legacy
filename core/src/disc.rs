//! Reading a pressing.
//!
//! A rip is an ISO-9660 image beside the cue sheet that describes the whole
//! disc. Nothing here is ever written: the rip stays exactly as the player made
//! it, and everything the guest may change lives in the profile instead.

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
    /// `LEGORR.EXE;1`.
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
            label,
        })
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
        self.read_dir(sector, len)
    }

    /// Read a whole file out of the image.
    ///
    /// # Errors
    /// When the image holds no such file.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, DiscError> {
        let entry = self.entry(path)?;
        if entry.is_dir {
            return Err(DiscError::NoSuchFile {
                iso: self.path.clone(),
                path: path.to_owned(),
            });
        }
        self.extent(entry.sector, entry.len)
    }

    /// Read part of a file out of the image.
    ///
    /// What a large archive is read through: the whole of it need not be in
    /// memory to unpack one entry.
    ///
    /// # Errors
    /// When the image holds no such file.
    pub fn read_range(
        &mut self,
        path: &str,
        offset: u32,
        len: usize,
    ) -> Result<Vec<u8>, DiscError> {
        let entry = self.entry(path)?;
        let start = u64::from(entry.sector) * SECTOR + u64::from(offset);
        let room = (entry.len as usize).saturating_sub(offset as usize);
        let mut bytes = vec![0u8; len.min(room)];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
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
        let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
        let mut here = self.root;
        let mut found = None;
        while let Some(part) = parts.next() {
            let entries = self.read_dir(here.0, here.1)?;
            let hit = entries
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .ok_or_else(|| DiscError::NoSuchFile {
                    iso: self.path.clone(),
                    path: path.to_owned(),
                })?;
            if parts.peek().is_some() {
                here = (hit.sector, hit.len);
            } else {
                found = Some(hit);
            }
        }
        found.ok_or_else(|| DiscError::NoSuchFile {
            iso: self.path.clone(),
            path: path.to_owned(),
        })
    }

    fn locate_dir(&mut self, dir: &str) -> Result<(u32, u32), DiscError> {
        if dir.trim_matches('/').is_empty() {
            return Ok(self.root);
        }
        let entry = self.entry(dir)?;
        Ok((entry.sector, entry.len))
    }

    /// The bytes of one run of sectors, refused when the image is too short
    /// to hold them: a length read from a damaged image can be four gigabytes.
    fn extent(&mut self, sector: u32, len: u32) -> Result<Vec<u8>, DiscError> {
        let start = u64::from(sector) * SECTOR;
        if start + u64::from(len) > self.size {
            return Err(DiscError::Damaged(self.path.clone()));
        }
        let mut bytes = vec![0u8; len as usize];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn read_dir(&mut self, sector: u32, len: u32) -> Result<Vec<Entry>, DiscError> {
        let data = self.extent(sector, len)?;

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
            // parent, spelled as a single zero or one byte.
            if raw != b"\0" && raw != b"\x01" {
                let name = String::from_utf8_lossy(raw);
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
/// `LEGO_LEGACY_GAME_DATA` moves the root; the default is `game-data` beside
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
