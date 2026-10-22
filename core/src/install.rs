//! The declared install tree (ADR-0002's other half).
//!
//! What a title's installer would have put on the machine's disc, declared
//! rather than produced by running the installer. Every file the guest can
//! open is one the pressing carries; nothing on the host is reachable, because
//! the guest is not on the host.
//!
//! What the guest writes is the one exception, and it does not reach the host
//! either: it lands in the title's profile, a directory of its own that the
//! guest sees as part of its disc and nothing else can be read from.

use crate::disc::Iso;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Whether a name matches a pattern with `*` and `?` in it, as a guest's own
/// enumeration spells one. Both are already upper-cased by the caller.
fn matches(name: &str, pattern: &str) -> bool {
    let (n, p): (Vec<char>, Vec<char>) = (name.chars().collect(), pattern.chars().collect());
    // The usual two-pointer walk: remember where the last star was, and go
    // back to it with one more character eaten when the rest stops matching.
    let (mut i, mut j) = (0, 0);
    let (mut star, mut back) = (None, 0);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some(j);
            back = i;
            j += 1;
        } else if let Some(at) = star {
            j = at + 1;
            back += 1;
            i = back;
        } else {
            return false;
        }
    }
    p[j..].iter().all(|c| *c == '*')
}

/// Where one of the guest's files comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file inside the pressing's own image.
    Disc(String),
    /// A file packed into the pressing's cabinet, by its index there.
    Packed(usize),
    /// A file imploded into the pressing's InstallShield 3 archive, by its
    /// index there (#143).
    Imploded(usize),
    /// Bytes the machine makes up, because the installer would have written
    /// them rather than copied them.
    Declared(Vec<u8>),
}

/// The declared machine's disc.
pub struct Tree {
    /// The pressing's cabinet, and the stem its data files are named with: a
    /// cabinet's volume N lives in `<stem>N.CAB` beside the header.
    cabinet: Option<(crate::cabinet::Cabinet, String)>,
    /// The pressing's InstallShield 3 archive, the one `Source::Imploded`
    /// indexes, and its path on the disc (#143).
    archive: Option<(crate::is3::Archive, String)>,
    /// Guest paths, upper-cased and with their separators normalised, to what
    /// stands behind them.
    files: BTreeMap<String, Source>,
    iso: Option<Iso>,
    /// An archive member that would not come out, until the machine stops at
    /// it (#169).
    wall: Option<String>,
    /// Where what the guest writes lands, so that a save outlives the run that
    /// made it. Without one the guest's disc is read-only.
    profile: Option<PathBuf>,
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cabinet: None,
            archive: None,
            files: BTreeMap::new(),
            iso: None,
            wall: None,
            profile: None,
        }
    }

    /// Give the tree the pressing its files come from.
    pub fn set_disc(&mut self, iso: Iso) {
        self.iso = Some(iso);
    }

    /// Where the machine stops, once: an archive member that would not come
    /// out (#169), or a name asked of the disc that only its primary tree
    /// records, read through its Joliet tree (#144).
    pub fn wall(&mut self) -> Option<String> {
        self.wall.take().or_else(|| self.iso.as_mut()?.wall())
    }

    /// Give the tree somewhere to keep what the guest writes.
    pub fn set_profile(&mut self, dir: PathBuf) {
        self.profile = Some(dir);
    }

    /// Where in the profile a guest path lives: one host directory per path
    /// component, the drive among them, and every name as the declared
    /// machine reads it, so a file written as one spelling is found by any
    /// other and two drives never land on one file. Nothing above the profile
    /// is reachable.
    fn profile_path(&self, guest_path: &str) -> Option<PathBuf> {
        let mut path = self.profile.clone()?;
        let mut parts = 0;
        for part in Self::key(guest_path).split('\\') {
            let part = part.trim_end_matches(':');
            if part.is_empty() || part == "." || part == ".." {
                continue;
            }
            path.push(part);
            parts += 1;
        }
        (parts > 0).then_some(path)
    }

    /// Keep what the guest wrote, where the guest will read it back from.
    ///
    /// # Errors
    /// When there is no profile to write to, or the host refuses the write.
    pub fn write(&mut self, guest_path: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        let path = self.profile_path(guest_path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "this run has no profile to write to",
            )
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Take back a file the guest wrote. What the pressing itself holds is
    /// not the guest's to delete, and saying so is honest: a title that
    /// deletes a save sees it go, and one that deletes its own data does not.
    pub fn remove(&mut self, guest_path: &str) -> bool {
        self.profile_path(guest_path)
            .is_some_and(|path| std::fs::remove_file(path).is_ok())
    }

    /// Move a file the guest wrote to another path, where the guest will read
    /// it back from. Nothing for a file it did not write: as with
    /// [`Tree::remove`], the pressing's own files are not the guest's to move.
    ///
    /// # Errors
    /// When the host refuses the move.
    pub fn rename(&mut self, from: &str, to: &str) -> Option<std::io::Result<PathBuf>> {
        let from = self.profile_path(from).filter(|path| path.is_file())?;
        let to = self.profile_path(to)?;
        let moved = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::rename(&from, &to));
        Some(moved.map(|()| to))
    }

    /// The form a guest path is looked up by: case does not matter on the
    /// declared machine, and neither does which way the separators lean.
    fn key(path: &str) -> String {
        path.trim().replace('/', "\\").to_uppercase()
    }

    /// Declare everything the pressing's cabinet holds, under a root.
    ///
    /// A packed entry becomes a file the guest can open; one the header names
    /// but does not pack sits loose on the disc, and is mapped there instead.
    ///
    /// `data_stem` names the cabinet's data files: volume N is `<stem>N.CAB`,
    /// which is what both pressings on the roster that carry a cabinet do.
    /// Rock Raiders has one volume and Alpha Team has two.
    ///
    /// # Errors
    /// When the header is not a cabinet.
    pub fn set_cabinet(
        &mut self,
        header: &[u8],
        data_stem: &str,
        root: &str,
    ) -> Result<usize, crate::cabinet::CabinetError> {
        let cabinet = crate::cabinet::Cabinet::open(header)?;
        let mut declared = 0;
        for (index, entry) in cabinet.entries().iter().enumerate() {
            let guest = entry.guest_path(root);
            // A header may name the same file twice, once per install size:
            // Alpha Team's carries `LMap.pac` as 53 MB of hi-res lightmaps for
            // a Typical install and 3.4 MB of low-res for a Minimum one. The
            // declared machine had the Typical install — the one the
            // installer offers first and a player takes — so the larger of
            // two entries with one name wins.
            if let Some(Source::Packed(before)) = self.files.get(&Self::key(&guest)) {
                if cabinet.entries()[*before].expanded >= entry.expanded {
                    continue;
                }
            }
            if entry.in_cabinet() {
                self.files.insert(Self::key(&guest), Source::Packed(index));
            } else {
                let on_disc = if entry.directory.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", entry.directory.replace('\\', "/"), entry.name)
                };
                self.files.insert(Self::key(&guest), Source::Disc(on_disc));
            }
            declared += 1;
        }
        self.cabinet = Some((cabinet, data_stem.to_owned()));
        Ok(declared)
    }

    /// Declare everything a directory of the pressing holds, under a root:
    /// what an installer that copies files rather than unpacking a cabinet
    /// would have put on the machine's disc.
    ///
    /// # Errors
    /// When the image holds no such directory, or cannot be read.
    pub fn set_copied(
        &mut self,
        iso: &mut Iso,
        from: &str,
        root: &str,
    ) -> Result<usize, crate::disc::DiscError> {
        let mut declared = 0;
        let mut dirs = vec![String::new()];
        while let Some(under) = dirs.pop() {
            let here = if under.is_empty() {
                from.to_owned()
            } else {
                format!("{from}/{under}")
            };
            for entry in iso.list(&here)? {
                let below = if under.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{under}/{}", entry.name)
                };
                if entry.is_dir {
                    dirs.push(below);
                } else {
                    let guest = format!("{root}\\{}", below.replace('/', "\\"));
                    self.files.insert(
                        Self::key(&guest),
                        Source::Disc(format!("{here}/{}", entry.name)),
                    );
                    declared += 1;
                }
            }
        }
        Ok(declared)
    }

    /// Declare everything an InstallShield 3 archive of the pressing holds,
    /// under a root, where its setup expands it (#143). A member is expanded
    /// when it is read, as a cabinet's entry is unpacked.
    ///
    /// `beside` names members of the setup's other archives, by archive and
    /// path, that its script copies to the root on their own. They are small,
    /// and are expanded now.
    ///
    /// # Errors
    /// When an archive is not on the disc or is not an InstallShield 3
    /// archive, or a member named beside it is not in its archive.
    pub fn set_expanded(
        &mut self,
        iso: &mut Iso,
        archive: &str,
        beside: &[(&str, &str)],
        root: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let expanded = crate::is3::Archive::open(&iso.read(archive)?)?;
        for (index, member) in expanded.members().iter().enumerate() {
            let guest = format!("{root}\\{}", member.path());
            self.files
                .insert(Self::key(&guest), Source::Imploded(index));
        }
        let mut declared = expanded.members().len();
        self.archive = Some((expanded, archive.to_owned()));
        for (library, path) in beside {
            let bytes = iso.read(library)?;
            let member = crate::is3::Archive::open(&bytes)?.read(&bytes, path)?;
            self.declare(&format!("{root}\\{path}"), member);
            declared += 1;
        }
        Ok(declared)
    }

    /// Install a title on the declared machine's disc as its setup would have,
    /// from the pressing itself rather than by running the setup (#16), and
    /// put the pressing in the drive. The pressing was opened by its primary
    /// tree's names, as the launcher reads it; from here it is read as the
    /// title declares (#144). How many files it declared.
    ///
    /// The machine and the tests build a title's tree this one way.
    ///
    /// # Errors
    /// When the pressing will not read as the title says it is installed.
    pub fn install(
        &mut self,
        mut iso: Iso,
        title: &crate::Title,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if title.declared.joliet {
            iso.read_joliet()?;
        }
        let installed = match title.installed {
            crate::title::Installed::Cabinet => {
                let header = iso.read("DATA1.HDR")?;
                self.set_cabinet(&header, "DATA", title.install_dir)?
            }
            crate::title::Installed::Copied(from) => {
                self.set_copied(&mut iso, from, title.install_dir)?
            }
            crate::title::Installed::Expanded { archive, beside } => {
                self.set_expanded(&mut iso, archive, beside, title.install_dir)?
            }
        };
        self.set_disc(iso);
        Ok(installed)
    }

    /// Declare a file the installer would have written rather than copied.
    pub fn declare(&mut self, guest_path: &str, bytes: Vec<u8>) {
        self.files
            .insert(Self::key(guest_path), Source::Declared(bytes));
    }

    /// Whether the tree holds a path.
    #[must_use]
    pub fn holds(&self, guest_path: &str) -> bool {
        self.files.contains_key(&Self::key(guest_path))
            || self
                .profile_path(guest_path)
                .is_some_and(|path| path.exists())
    }

    /// Whether [`Tree::read`] would find a file, answered without reading it:
    /// a packed entry is there whether or not anyone unpacks it.
    pub fn readable(&mut self, guest_path: &str) -> bool {
        let key = Self::key(guest_path);
        self.profile_path(&key).is_some_and(|p| p.is_file())
            || matches!(
                self.files.get(&key),
                Some(Source::Declared(_) | Source::Packed(_) | Source::Imploded(_))
            )
            || self.lies(&key).is_some()
    }

    /// Where a file the guest named lies on the pressing's image, for one
    /// [`Tree::read`] would read from there: its first byte in the image and
    /// how long it is. Nothing for a file the guest wrote, which stands in
    /// front of the image, or one the tree unpacks or declares.
    ///
    /// What a file opened only to read is read through, a piece at a time,
    /// rather than held whole: LEGO Island's scripts are 534 MB, and one of
    /// them alone is 157 MB (#116).
    pub fn lies(&mut self, guest_path: &str) -> Option<(u64, u32)> {
        let key = Self::key(guest_path);
        if self.profile_path(&key).is_some_and(|p| p.is_file()) {
            return None;
        }
        let on_disc = match self.files.get(&key) {
            Some(Source::Disc(path)) => path.clone(),
            Some(_) => return None,
            // The pressing is in the declared machine's CD drive.
            None => key.strip_prefix("D:\\")?.replace('\\', "/"),
        };
        self.iso.as_mut()?.lies(&on_disc).ok()
    }

    /// `len` bytes of the pressing's image from `start`, which is somewhere
    /// [`Tree::lies`] said a file lies; nothing when the image cannot be read.
    pub fn read_at(&mut self, start: u64, len: usize) -> Option<Vec<u8>> {
        self.iso.as_mut()?.read_at(start, len).ok()
    }

    /// Read a file the guest named.
    pub fn read(&mut self, guest_path: &str) -> Option<Vec<u8>> {
        let key = Self::key(guest_path);
        // What the guest wrote stands in front of what the pressing holds:
        // writing over a file is how a title keeps its settings.
        if let Some(bytes) = self.profile_path(&key).and_then(|p| std::fs::read(p).ok()) {
            return Some(bytes);
        }
        match self.files.get(&key).cloned() {
            Some(Source::Declared(bytes)) => Some(bytes),
            Some(Source::Packed(index)) => {
                let (cabinet, data_stem) = self.cabinet.as_ref()?;
                let entry = cabinet.entries().get(index)?;
                // An entry names the volume it is in, and the volume names the
                // file: Alpha Team's game is in DATA2.CAB and only its
                // installer is in DATA1.CAB.
                let data_path = format!("{data_stem}{}.CAB", entry.volume);
                let got = self
                    .iso
                    .as_mut()?
                    .read_range(&data_path, entry.offset, entry.compressed as usize)
                    .map_err(|e| format!("{} would not read: {e}", entry.name))
                    .and_then(|span| cabinet.unpack(entry, &span).map_err(|e| e.to_string()));
                self.member(&data_path, got)
            }
            Some(Source::Imploded(index)) => {
                let (archive, path) = self.archive.as_ref()?;
                let member = archive.members().get(index)?;
                let got = self
                    .iso
                    .as_mut()?
                    .read_range(path, member.offset, member.compressed as usize)
                    .map_err(|e| format!("{} would not read: {e}", member.path()))
                    .and_then(|span| member.expand(&span).map_err(|e| e.to_string()));
                let path = path.clone();
                self.member(&path, got)
            }
            Some(Source::Disc(path)) => self.iso.as_mut()?.read(&path).ok(),
            None => {
                // The declared machine has the pressing in its CD drive, so a
                // path on D: is a path on the disc, spelled the way the disc
                // spells it.
                if let Some(rest) = key.strip_prefix("D:\\") {
                    let on_disc = rest.replace('\\', "/");
                    if let Some(iso) = self.iso.as_mut() {
                        if let Ok(bytes) = iso.read(&on_disc) {
                            return Some(bytes);
                        }
                    }
                }
                None
            }
        }
    }

    /// A member of the pressing's archive as it came out of `archive`: the
    /// one place a cabinet's entries and an InstallShield 3 archive's members
    /// are answered (#169).
    ///
    /// One that would not come out, its archive unreadable, its data damaged,
    /// stored a way the reader was not taught, or another size than the
    /// archive's table gives, is a wall naming the archive, the image and the
    /// reader's why, which names the member; and nothing. A member the tree
    /// says is there is never answered as missing, nor as wrong bytes (golden
    /// rules 2 and 4). A good rip never comes here.
    fn member(&mut self, archive: &str, got: Result<Vec<u8>, String>) -> Option<Vec<u8>> {
        got.map_err(|why| {
            let image = self
                .iso
                .as_ref()
                .map_or_else(String::new, |iso| iso.path().display().to_string());
            self.wall
                .get_or_insert_with(|| format!("{archive} on {image}: {why} (#169)"));
        })
        .ok()
    }

    /// Every file the tree holds under a directory whose name matches a
    /// pattern, with how big each one is.
    ///
    /// What `FindFirstFileA` enumerates. The sizes are what the tree already
    /// knows, without unpacking anything: a packed file's from the cabinet's
    /// header, an imploded file's from its archive's table, a declared file's
    /// from its bytes, and a file on the disc
    /// itself is answered as zero bytes long. A title looking for its archives
    /// should not cost four hundred megabytes of inflating to answer.
    #[must_use]
    pub fn matching(&self, dir: &str, pattern: &str) -> Vec<(String, u32)> {
        let dir = Self::key(dir);
        let prefix = if dir.ends_with('\\') {
            dir
        } else {
            format!("{dir}\\")
        };
        let mut out = Vec::new();
        for (path, source) in &self.files {
            let Some(name) = path.strip_prefix(&prefix) else {
                continue;
            };
            if name.contains('\\') || !matches(name, &Self::key(pattern)) {
                continue;
            }
            let size = match source {
                Source::Declared(bytes) => u32::try_from(bytes.len()).unwrap_or(0),
                Source::Packed(index) => self
                    .cabinet
                    .as_ref()
                    .and_then(|(c, _)| c.entries().get(*index))
                    .map_or(0, |e| e.expanded),
                Source::Imploded(index) => self
                    .archive
                    .as_ref()
                    .and_then(|(a, _)| a.members().get(*index))
                    .map_or(0, |m| m.expanded),
                Source::Disc(_) => 0,
            };
            out.push((name.to_owned(), size));
        }
        out
    }
}
