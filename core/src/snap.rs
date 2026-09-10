//! Snapshots: the machine's state, written down.
//!
//! A snapshot is a cache and never evidence. It exists so a run can be resumed
//! at a cursor instead of replayed from its start, and it is keyed by what
//! produced it: another build, another title or another route cannot resume
//! from it, and a resume that tried would be told which of the three did not
//! match.

use crate::hash::{hex, Sha256};
use std::fmt;

/// The first bytes of every snapshot.
pub const MAGIC: &[u8; 8] = b"LLSNAP\x01\x00";

/// Why a snapshot could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapError {
    NotASnapshot,
    Truncated(&'static str),
    /// The snapshot was made by something else, and which something.
    Mismatch {
        what: &'static str,
        wanted: String,
        found: String,
    },
}

impl fmt::Display for SnapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotASnapshot => write!(f, "not a snapshot"),
            Self::Truncated(what) => write!(f, "the snapshot ends inside its {what}"),
            Self::Mismatch {
                what,
                wanted,
                found,
            } => write!(
                f,
                "this snapshot was made by a different {what}: it says {}, this run is {}",
                &found[..16.min(found.len())],
                &wanted[..16.min(wanted.len())]
            ),
        }
    }
}

impl std::error::Error for SnapError {}

/// What a snapshot was made by, and what it may be resumed into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The emulator that wrote it.
    pub build: String,
    /// The title it was running.
    pub title: String,
    /// The route it was replaying, or the digest of no route at all.
    pub route: String,
}

impl Provenance {
    /// Refuse a snapshot that came from somewhere else, naming which part.
    ///
    /// # Errors
    /// When the build, the title or the route differs.
    pub fn accepts(&self, other: &Self) -> Result<(), SnapError> {
        for (what, wanted, found) in [
            ("build", &self.build, &other.build),
            ("title", &self.title, &other.title),
            ("route", &self.route, &other.route),
        ] {
            if wanted != found {
                return Err(SnapError::Mismatch {
                    what,
                    wanted: wanted.clone(),
                    found: found.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Which way a visit is going.
pub trait Visitor {
    /// Carry four bytes in or out.
    fn u32(&mut self, value: &mut u32);
    /// Carry eight bytes in or out.
    fn u64(&mut self, value: &mut u64);
    /// Carry two bytes in or out.
    fn u16(&mut self, value: &mut u16);
    /// Carry one byte in or out.
    fn u8(&mut self, value: &mut u8);
    /// Carry a truth in or out.
    fn bool(&mut self, value: &mut bool);
    /// Carry a floating-point register in or out, by its bits.
    fn f64(&mut self, value: &mut f64) {
        let mut bits = value.to_bits();
        self.u64(&mut bits);
        *value = f64::from_bits(bits);
    }
    /// Carry a run of bytes in or out. The length is not written: both sides
    /// know it from what they are visiting.
    fn bytes(&mut self, value: &mut [u8]);
}

/// Something a snapshot can carry.
///
/// Every implementation destructures itself completely, with no rest pattern,
/// so a field added later is a compile error here rather than a field silently
/// left out of every snapshot.
pub trait Snap {
    fn snap(&mut self, v: &mut dyn Visitor);
}

/// A visitor that writes.
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn new() -> Self {
        Self { out: Vec::new() }
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visitor for Writer {
    fn u32(&mut self, value: &mut u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: &mut u64) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn u16(&mut self, value: &mut u16) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn u8(&mut self, value: &mut u8) {
        self.out.push(*value);
    }
    fn bool(&mut self, value: &mut bool) {
        self.out.push(u8::from(*value));
    }
    fn bytes(&mut self, value: &mut [u8]) {
        self.out.extend_from_slice(value);
    }
}

/// A visitor that reads.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Set when the snapshot ran out before the state did.
    pub short: bool,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            short: false,
        }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(out)
    }
}

impl Visitor for Reader<'_> {
    fn u32(&mut self, value: &mut u32) {
        match self.take(4) {
            Some(b) => *value = u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => self.short = true,
        }
    }
    fn u64(&mut self, value: &mut u64) {
        match self.take(8) {
            Some(b) => {
                *value = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            }
            None => self.short = true,
        }
    }
    fn u16(&mut self, value: &mut u16) {
        match self.take(2) {
            Some(b) => *value = u16::from_le_bytes([b[0], b[1]]),
            None => self.short = true,
        }
    }
    fn u8(&mut self, value: &mut u8) {
        match self.take(1) {
            Some(b) => *value = b[0],
            None => self.short = true,
        }
    }
    fn bool(&mut self, value: &mut bool) {
        let mut byte = 0;
        self.u8(&mut byte);
        *value = byte != 0;
    }
    fn bytes(&mut self, value: &mut [u8]) {
        match self.take(value.len()) {
            Some(b) => value.copy_from_slice(b),
            None => self.short = true,
        }
    }
}

/// The digest of the emulator that is running.
///
/// A snapshot from another build cannot be resumed into this one, and the only
/// honest way to know which build made it is to hash the binary itself.
#[must_use]
pub fn build_digest() -> String {
    use std::sync::OnceLock;
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST
        .get_or_init(|| {
            let bytes = std::env::current_exe().and_then(std::fs::read);
            match bytes {
                Ok(bytes) => {
                    let mut h = Sha256::new();
                    h.update(&bytes);
                    hex(&h.finish())
                }
                // A build that cannot read itself is still a build, and saying
                // so is better than pretending every build is the same one.
                Err(_) => "unknown-build".to_owned(),
            }
        })
        .clone()
}

impl Snap for crate::cpu::flags::Flags {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let mut bits = self.to_eflags();
        v.u32(&mut bits);
        *self = Self::from_eflags(bits);
    }
}

impl Snap for crate::cpu::Cpu {
    fn snap(&mut self, v: &mut dyn Visitor) {
        // Destructured completely: a register added later stops this
        // compiling rather than quietly staying out of every snapshot.
        let Self {
            r,
            eip,
            seg,
            seg_base,
            flags,
            fpu,
        } = self;
        for value in r {
            v.u32(value);
        }
        v.u32(eip);
        for value in seg {
            v.u16(value);
        }
        for value in seg_base {
            v.u32(value);
        }
        flags.snap(v);
        fpu.snap(v);
    }
}

impl Snap for crate::cpu::x87::X87 {
    fn snap(&mut self, v: &mut dyn Visitor) {
        // The unit's own state travels through its accessors, because its
        // registers are private and its stack top is not where a reader would
        // guess.
        let mut control = self.control();
        v.u16(&mut control);
        self.set_control(control);
        for i in 0..crate::cpu::x87::DEPTH {
            let mut value = self.st(i);
            v.f64(&mut value);
            let mut used = self.occupied(i);
            v.bool(&mut used);
            if used {
                self.set_st(i, value);
            } else {
                self.free(i);
            }
        }
    }
}

impl Snap for crate::clock::Clock {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let mut cursor = self.cursor();
        let mut cycles = self.cycles();
        v.u64(&mut cursor);
        v.u64(&mut cycles);
        *self = Self::resumed(cursor, cycles);
    }
}
