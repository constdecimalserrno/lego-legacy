//! An InstallShield 3 archive: a title's files in one file of its pressing
//! (#143).
//!
//! LEGO Chess ships its game as `MAIN.Z`, and its setup's own files as
//! `_SETUP.LIB` beside it. Each is a header, every member imploded
//! ([`crate::explode`]) one after another, and at the end a table: each
//! directory with how many files it holds, then every file. Measured on both
//! of the title's pressings, because the format has no specification anyone
//! published.
//!
//! **A file's directory is the index its own record carries**, not its place
//! in the table. The original pressing's `MAIN.Z` lists its executable, a file
//! at its root, last of all, and a reader that deals each directory its count
//! of records in turn puts that file and one of every directory after it in
//! the wrong place: 48 of its 871. The Masterpiece pressing's table happens to
//! be in order, so it reads the same either way.

use crate::explode::explode;
use crate::pe::{u16_at, u32_at};
use std::fmt;

/// The eight bytes every archive starts with.
const MAGIC: [u8; 8] = [0x13, 0x5d, 0x65, 0x8c, 0x3a, 0x01, 0x02, 0x00];

/// How long a file's record is before its name.
const RECORD: usize = 30;

/// What the archive turned out not to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Is3Error {
    NotAnArchive,
    Truncated(&'static str),
    /// A record in the table, among its directories or its files, too short
    /// to hold its own name: the archive is damaged.
    Damaged(&'static str),
    /// A file whose record names a directory the table does not have.
    NoSuchDirectory {
        name: String,
        index: u16,
    },
    /// A member stored some way other than imploded.
    UnknownFlags {
        name: String,
        flags: u8,
    },
    /// A member's own data would not expand, or not to its size.
    Corrupt {
        name: String,
        why: String,
    },
    /// The archive holds no such member.
    NotInArchive {
        path: String,
    },
}

impl fmt::Display for Is3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnArchive => write!(f, "not an InstallShield 3 archive"),
            Self::Truncated(what) => write!(f, "the archive ends inside its {what}"),
            Self::Damaged(what) => write!(
                f,
                "the archive is damaged: a record among its {what} is too short for its own name"
            ),
            Self::NoSuchDirectory { name, index } => write!(
                f,
                "{name}'s record names directory {index}, which the table does not have"
            ),
            Self::UnknownFlags { name, flags } => write!(
                f,
                "{name} is stored with flags {flags:#04x}, which this reader has not been taught; \
                 handing back wrong bytes would be worse than stopping"
            ),
            Self::Corrupt { name, why } => write!(f, "{name} would not expand: {why}"),
            Self::NotInArchive { path } => write!(f, "the archive holds no {path}"),
        }
    }
}

impl std::error::Error for Is3Error {}

/// One file the archive holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// The directory the setup would have put it in, as the table spells it:
    /// `staunton\red`, or nothing at the archive's root.
    pub directory: String,
    /// How big it is once expanded.
    pub expanded: u32,
    /// How much of the archive it takes.
    pub compressed: u32,
    /// Where its data starts in the archive.
    pub offset: u32,
    /// How it is stored. Zero is imploded, which is every member of both of
    /// LEGO Chess's archives and all this reader knows.
    pub flags: u8,
}

impl Member {
    /// Its path in the archive: `staunton\red\chess1.bmp`, or its name alone
    /// at the root.
    #[must_use]
    pub fn path(&self) -> String {
        if self.directory.is_empty() {
            self.name.clone()
        } else {
            format!("{}\\{}", self.directory, self.name)
        }
    }

    /// Expand it from its own data out of the archive.
    ///
    /// # Errors
    /// When it is stored a way this reader does not know, or will not expand
    /// to the size the table gives it.
    pub fn expand(&self, data: &[u8]) -> Result<Vec<u8>, Is3Error> {
        let corrupt = |why: String| Is3Error::Corrupt {
            name: self.path(),
            why,
        };
        if self.flags != 0 {
            return Err(Is3Error::UnknownFlags {
                name: self.path(),
                flags: self.flags,
            });
        }
        let bytes = explode(data, self.expanded as usize).map_err(|e| corrupt(e.to_string()))?;
        if bytes.len() != self.expanded as usize {
            return Err(corrupt(format!(
                "it expands to {} bytes, and the table says {}",
                bytes.len(),
                self.expanded
            )));
        }
        Ok(bytes)
    }
}

/// An archive's table, read.
pub struct Archive {
    members: Vec<Member>,
}

impl Archive {
    /// Read an archive's table out of its bytes.
    ///
    /// # Errors
    /// When the bytes are not an InstallShield 3 archive, or end inside its
    /// table.
    pub fn open(bytes: &[u8]) -> Result<Self, Is3Error> {
        if bytes.get(..MAGIC.len()) != Some(&MAGIC[..]) {
            return Err(Is3Error::NotAnArchive);
        }
        let header = |at| u16_at(bytes, at).ok_or(Is3Error::Truncated("header"));
        let files = header(12)?;
        let directories = header(49)?;
        let mut at = u32_at(bytes, 41).ok_or(Is3Error::Truncated("header"))? as usize;

        // Each directory: how many files it holds, how long its record is, and
        // its name. Which files it holds is theirs to say, not its.
        let mut names = Vec::with_capacity(usize::from(directories));
        for _ in 0..directories {
            let field = |offset| {
                u16_at(bytes, at + offset)
                    .map(usize::from)
                    .ok_or(Is3Error::Truncated("directories"))
            };
            let (len, name_len) = (field(2)?, field(4)?);
            if len < 6 + name_len {
                return Err(Is3Error::Damaged("directories"));
            }
            let name = bytes
                .get(at + 6..at + 6 + name_len)
                .ok_or(Is3Error::Truncated("directories"))?;
            names.push(String::from_utf8_lossy(name).into_owned());
            at += len;
        }

        let mut members = Vec::with_capacity(usize::from(files));
        for _ in 0..files {
            let record = bytes
                .get(at..)
                .filter(|r| r.len() >= RECORD)
                .ok_or(Is3Error::Truncated("files"))?;
            let name = record
                .get(RECORD..RECORD + usize::from(record[29]))
                .ok_or(Is3Error::Truncated("files"))?;
            let name = String::from_utf8_lossy(name).into_owned();
            // The record is long enough for every field before its name.
            let (short, long) = (
                |o| u16_at(record, o).unwrap_or(0),
                |o| u32_at(record, o).unwrap_or(0),
            );
            let len = usize::from(short(23));
            if len < RECORD + usize::from(record[29]) {
                return Err(Is3Error::Damaged("files"));
            }
            let index = short(1);
            let Some(directory) = names.get(usize::from(index)).cloned() else {
                return Err(Is3Error::NoSuchDirectory { name, index });
            };
            members.push(Member {
                name,
                directory,
                expanded: long(3),
                compressed: long(7),
                offset: long(11),
                flags: record[25],
            });
            at += len;
        }
        Ok(Self { members })
    }

    /// Every file, in the order the table lists them.
    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Find a member by its path in the archive, as Windows would find a file:
    /// whatever its case, and whichever way its separators lean.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<&Member> {
        let path = path.replace('/', "\\");
        self.members
            .iter()
            .find(|m| m.path().eq_ignore_ascii_case(&path))
    }

    /// Expand one member, out of the archive's own bytes.
    ///
    /// # Errors
    /// When the archive does not hold it, or it will not expand.
    pub fn read(&self, archive: &[u8], path: &str) -> Result<Vec<u8>, Is3Error> {
        let member = self.find(path).ok_or_else(|| Is3Error::NotInArchive {
            path: path.to_owned(),
        })?;
        let start = member.offset as usize;
        let data = archive
            .get(start..start + member.compressed as usize)
            .ok_or_else(|| Is3Error::Corrupt {
                name: member.path(),
                why: "the archive ends inside it".to_owned(),
            })?;
        member.expand(data)
    }
}

#[cfg(test)]
mod tests {
    use super::{Archive, Is3Error, MAGIC};

    /// Adler's vector: an imploded `AIAIAIAIAIAIA`.
    const AIAI: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];

    /// An archive of a root and one directory under it, `sub`, whose table
    /// lists `sub`'s file first and the root's file last, both pointing at
    /// the same data, as the original pressing's `MAIN.Z` lists its executable.
    fn out_of_order() -> Vec<u8> {
        let mut bytes = vec![0u8; 59];
        bytes[..8].copy_from_slice(&MAGIC);
        bytes[12..14].copy_from_slice(&2u16.to_le_bytes());
        bytes[49..51].copy_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&AIAI);
        let toc = u32::try_from(bytes.len()).expect("small");
        bytes[41..45].copy_from_slice(&toc.to_le_bytes());
        for (files, name) in [(1u16, ""), (1, "sub")] {
            let len = u16::try_from(6 + name.len()).expect("small");
            for field in [files, len, len - 6] {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
            bytes.extend_from_slice(name.as_bytes());
        }
        for (directory, name) in [(1u16, "b.txt"), (0, "a.txt")] {
            let mut record = vec![0u8; 30];
            record[1..3].copy_from_slice(&directory.to_le_bytes());
            record[3..7].copy_from_slice(&13u32.to_le_bytes());
            record[7..11].copy_from_slice(&8u32.to_le_bytes());
            record[11..15].copy_from_slice(&59u32.to_le_bytes());
            let len = u16::try_from(30 + name.len()).expect("small");
            record[23..25].copy_from_slice(&len.to_le_bytes());
            record[29] = u8::try_from(name.len()).expect("small");
            record.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&record);
        }
        bytes
    }

    #[test]
    fn a_file_is_in_the_directory_its_own_record_names_not_where_the_table_lists_it() {
        let bytes = out_of_order();
        let archive = Archive::open(&bytes).expect("this is an archive");
        let paths: Vec<String> = archive.members().iter().map(super::Member::path).collect();
        // Dealt out in turn, the root would get `b.txt` and `sub` would get
        // `a.txt`.
        assert_eq!(paths, ["sub\\b.txt", "a.txt"]);
        for path in ["a.txt", "SUB/B.TXT"] {
            assert_eq!(
                archive.read(&bytes, path),
                Ok(b"AIAIAIAIAIAIA".to_vec()),
                "{path} is found and expands"
            );
        }
        assert_eq!(
            archive.read(&bytes, "sub\\a.txt"),
            Err(Is3Error::NotInArchive {
                path: "sub\\a.txt".to_owned()
            })
        );
    }

    #[test]
    fn a_record_naming_a_directory_the_table_lacks_is_refused() {
        let mut bytes = out_of_order();
        let first = bytes.len() - 2 * 30 - "b.txt".len() - "a.txt".len();
        bytes[first + 1] = 2;
        assert_eq!(
            Archive::open(&bytes).err(),
            Some(Is3Error::NoSuchDirectory {
                name: "b.txt".to_owned(),
                index: 2
            })
        );
    }

    #[test]
    fn a_member_that_expands_to_another_size_or_is_stored_otherwise_is_refused() {
        let bytes = out_of_order();
        let archive = Archive::open(&bytes).expect("this is an archive");
        let mut member = archive.find("a.txt").expect("it is there").clone();
        member.expanded = 12;
        assert!(matches!(
            member.expand(&bytes[59..67]),
            Err(Is3Error::Corrupt { .. })
        ));
        member.flags = 0x10;
        assert!(matches!(
            member.expand(&bytes[59..67]),
            Err(Is3Error::UnknownFlags { flags: 0x10, .. })
        ));
    }

    #[test]
    fn something_that_is_not_an_archive_is_refused() {
        assert_eq!(
            Archive::open(b"not an archive at all").err(),
            Some(Is3Error::NotAnArchive)
        );
        assert_eq!(
            Archive::open(&MAGIC).err(),
            Some(Is3Error::Truncated("header"))
        );
        // A record whose length does not reach past its own name would have
        // the next one read out of it: that archive is damaged, and says so.
        let first = out_of_order().len() - 2 * 30 - "b.txt".len() - "a.txt".len();
        for (at, what) in [(67 + 2, "directories"), (first + 23, "files")] {
            let mut bytes = out_of_order();
            bytes[at..at + 2].copy_from_slice(&0u16.to_le_bytes());
            assert_eq!(
                Archive::open(&bytes).err(),
                Some(Is3Error::Damaged(what)),
                "a record length of 0 at {at}"
            );
        }
    }
}
