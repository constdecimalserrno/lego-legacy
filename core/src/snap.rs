//! Snapshots: the machine's whole state, written down (`LLSNAP` v2, #313).
//!
//! A snapshot is a cache and never evidence. It is keyed by what produced it:
//! another build, another title or another route cannot resume from it, and a
//! resume that tried would be told which of the three did not match.
//!
//! Version 2 carries the entire live machine state — CPU + full x87 (`regs`,
//! `top`, `used`, `control`, `status`), declared clock, cooperative schedule
//! & clock-spin tracker, boundary & dynamically loaded modules, sparse non-zero
//! 4 KiB guest arena pages, complete Win32/COM/DirectDraw/Direct3D/D3DRM/
//! DirectSound/DirectInput/MIDI/CD/movie state, audio mixer + DLS synth,
//! recording route, and the profile directory file overlay — so any title on
//! the roster can snapshot and restore bit-for-bit in tens of milliseconds.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// The first bytes of every LLSNAP v2 snapshot.
pub const MAGIC: &[u8; 8] = b"LLSNAP\x02\x00";

/// Why a snapshot could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapError {
    NotASnapshot,
    Truncated(&'static str),
    Corrupt(&'static str),
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
            Self::Corrupt(what) => write!(f, "the snapshot is corrupt inside its {what}"),
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

    /// Refuse a snapshot from a different build or title while allowing live
    /// interactive control sessions to restore across differing pre-roll routes.
    ///
    /// # Errors
    /// When the build or the title differs.
    pub fn accepts_target(&self, other: &Self) -> Result<(), SnapError> {
        for (what, wanted, found) in [
            ("build", &self.build, &other.build),
            ("title", &self.title, &other.title),
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
    /// Whether this visitor is reading from a snapshot (`true`) or writing to one (`false`).
    fn is_reading(&self) -> bool;
    /// Record a semantic corruption error while reading.
    fn mark_corrupt(&mut self, what: &'static str);
    /// Whether at least `min_bytes` remain unread (always `true` when writing).
    fn check_remaining(&mut self, min_bytes: usize) -> bool;
    /// Whether this visitor has already failed (truncated or corrupt).
    fn failed(&self) -> bool;

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

    /// Carry sixteen bytes in or out.
    fn u128(&mut self, value: &mut u128) {
        let mut lo = *value as u64;
        let mut hi = (*value >> 64) as u64;
        self.u64(&mut lo);
        self.u64(&mut hi);
        *value = u128::from(lo) | (u128::from(hi) << 64);
    }

    /// Carry a signed 16-bit integer in or out.
    fn i16(&mut self, value: &mut i16) {
        let mut raw = *value as u16;
        self.u16(&mut raw);
        *value = raw as i16;
    }

    /// Carry a signed 32-bit integer in or out.
    fn i32(&mut self, value: &mut i32) {
        let mut raw = *value as u32;
        self.u32(&mut raw);
        *value = raw as i32;
    }

    /// Carry a signed 64-bit integer in or out.
    fn i64(&mut self, value: &mut i64) {
        let mut raw = *value as u64;
        self.u64(&mut raw);
        *value = raw as i64;
    }

    /// Carry a `usize` in or out as a `u64`.
    fn usize(&mut self, value: &mut usize) {
        let mut raw = *value as u64;
        self.u64(&mut raw);
        if self.is_reading() {
            match usize::try_from(raw) {
                Ok(n) => *value = n,
                Err(_) => self.mark_corrupt("usize"),
            }
        }
    }

    /// Carry a single-precision float in or out, by its exact IEEE-754 bits.
    fn f32(&mut self, value: &mut f32) {
        let mut bits = value.to_bits();
        self.u32(&mut bits);
        *value = f32::from_bits(bits);
    }

    /// Carry a floating-point register in or out, by its exact IEEE-754 bits.
    fn f64(&mut self, value: &mut f64) {
        let mut bits = value.to_bits();
        self.u64(&mut bits);
        *value = f64::from_bits(bits);
    }

    /// Carry a run of bytes in or out. The length is not written: both sides
    /// know it from what they are visiting.
    fn bytes(&mut self, value: &mut [u8]);

    /// Carry a length-prefixed byte buffer in or out.
    fn byte_vec(&mut self, value: &mut Vec<u8>) {
        if self.is_reading() {
            let mut len = 0u32;
            self.u32(&mut len);
            let len = len as usize;
            if !self.check_remaining(len) {
                return;
            }
            value.resize(len, 0);
            self.bytes(value);
        } else {
            let mut len = u32::try_from(value.len()).unwrap_or(u32::MAX);
            self.u32(&mut len);
            self.bytes(value);
        }
    }

    /// Carry a length-prefixed UTF-8 string in or out.
    fn string(&mut self, value: &mut String) {
        if self.is_reading() {
            let mut raw = Vec::new();
            self.byte_vec(&mut raw);
            if self.failed() {
                return;
            }
            match String::from_utf8(raw) {
                Ok(s) => *value = s,
                Err(_) => self.mark_corrupt("utf8 string"),
            }
        } else {
            let mut len = u32::try_from(value.len()).unwrap_or(u32::MAX);
            self.u32(&mut len);
            // Safety: `Writer::bytes` only reads the slice.
            let mut bytes = value.as_bytes().to_vec();
            self.bytes(&mut bytes);
        }
    }
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

    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            out: Vec::with_capacity(cap),
        }
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visitor for Writer {
    fn is_reading(&self) -> bool {
        false
    }
    fn mark_corrupt(&mut self, _what: &'static str) {}
    fn check_remaining(&mut self, _min_bytes: usize) -> bool {
        true
    }
    fn failed(&self) -> bool {
        false
    }
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
    fn string(&mut self, value: &mut String) {
        let len = u32::try_from(value.len()).unwrap_or(u32::MAX);
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(value.as_bytes());
    }
}

/// A visitor that reads.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Set when the snapshot ran out before the state did.
    pub short: bool,
    /// Set when a tag, length, or invariant was invalid.
    pub corrupt: Option<&'static str>,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            short: false,
            corrupt: None,
        }
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    /// Verify that the reader succeeded without truncation or corruption.
    ///
    /// # Errors
    /// Returns [`SnapError::Truncated`] or [`SnapError::Corrupt`].
    pub fn finish(&self, section: &'static str) -> Result<(), SnapError> {
        if let Some(what) = self.corrupt {
            return Err(SnapError::Corrupt(what));
        }
        if self.short {
            return Err(SnapError::Truncated(section));
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.short || self.corrupt.is_some() {
            return None;
        }
        let end = self.at.checked_add(n)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(out)
    }
}

impl Visitor for Reader<'_> {
    fn is_reading(&self) -> bool {
        true
    }
    fn mark_corrupt(&mut self, what: &'static str) {
        if self.corrupt.is_none() {
            self.corrupt = Some(what);
        }
    }
    fn check_remaining(&mut self, min_bytes: usize) -> bool {
        if self.short || self.corrupt.is_some() {
            return false;
        }
        if self.remaining() < min_bytes {
            self.short = true;
            return false;
        }
        true
    }
    fn failed(&self) -> bool {
        self.short || self.corrupt.is_some()
    }
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

/// Visit an `Option<T>` using `T::default()` to construct on read.
pub fn snap_option<T: Default>(
    v: &mut dyn Visitor,
    opt: &mut Option<T>,
    f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    snap_option_with(v, opt, T::default, f);
}

/// Visit an `Option<T>` using a custom constructor on read.
pub fn snap_option_with<T>(
    v: &mut dyn Visitor,
    opt: &mut Option<T>,
    make: impl FnOnce() -> T,
    mut f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    let mut present = opt.is_some();
    v.bool(&mut present);
    if v.failed() {
        return;
    }
    if v.is_reading() {
        if present {
            let mut val = make();
            f(&mut val, v);
            *opt = Some(val);
        } else {
            *opt = None;
        }
    } else if let Some(val) = opt.as_mut() {
        f(val, v);
    }
}

/// Visit a `Vec<T>` using `T::default()` to construct on read.
pub fn snap_vec<T: Default>(
    v: &mut dyn Visitor,
    vec: &mut Vec<T>,
    f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    snap_vec_with(v, vec, T::default, f);
}

/// Visit a `Vec<T>` using a custom element constructor on read.
pub fn snap_vec_with<T>(
    v: &mut dyn Visitor,
    vec: &mut Vec<T>,
    mut make: impl FnMut() -> T,
    mut f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        if !v.check_remaining(len) {
            return;
        }
        vec.clear();
        vec.reserve(len);
        for _ in 0..len {
            let mut item = make();
            f(&mut item, v);
            if v.failed() {
                return;
            }
            vec.push(item);
        }
    } else {
        let mut len = u32::try_from(vec.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        for item in vec.iter_mut() {
            f(item, v);
        }
    }
}

/// Visit a `VecDeque<T>` using `T::default()` to construct on read.
pub fn snap_vec_deque<T: Default>(
    v: &mut dyn Visitor,
    dq: &mut VecDeque<T>,
    f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    snap_vec_deque_with(v, dq, T::default, f);
}

/// Visit a `VecDeque<T>` using a custom element constructor on read.
pub fn snap_vec_deque_with<T>(
    v: &mut dyn Visitor,
    dq: &mut VecDeque<T>,
    mut make: impl FnMut() -> T,
    mut f: impl FnMut(&mut T, &mut dyn Visitor),
) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        if !v.check_remaining(len) {
            return;
        }
        dq.clear();
        dq.reserve(len);
        for _ in 0..len {
            let mut item = make();
            f(&mut item, v);
            if v.failed() {
                return;
            }
            dq.push_back(item);
        }
    } else {
        let mut len = u32::try_from(dq.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        for item in dq.iter_mut() {
            f(item, v);
        }
    }
}

/// Visit a `BTreeMap<K, V>` using `Default` constructors on read.
pub fn snap_btree_map<K: Ord + Clone + Default, V: Default>(
    v: &mut dyn Visitor,
    map: &mut BTreeMap<K, V>,
    fk: impl FnMut(&mut K, &mut dyn Visitor),
    fv: impl FnMut(&mut V, &mut dyn Visitor),
) {
    snap_btree_map_with(v, map, K::default, V::default, fk, fv);
}

/// Visit a `BTreeMap<K, V>` using custom constructors on read.
pub fn snap_btree_map_with<K: Ord + Clone, V>(
    v: &mut dyn Visitor,
    map: &mut BTreeMap<K, V>,
    mut make_k: impl FnMut() -> K,
    mut make_v: impl FnMut() -> V,
    mut fk: impl FnMut(&mut K, &mut dyn Visitor),
    mut fv: impl FnMut(&mut V, &mut dyn Visitor),
) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        if !v.check_remaining(len) {
            return;
        }
        map.clear();
        for _ in 0..len {
            let mut key = make_k();
            fk(&mut key, v);
            let mut val = make_v();
            fv(&mut val, v);
            if v.failed() {
                return;
            }
            map.insert(key, val);
        }
    } else {
        let mut len = u32::try_from(map.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        for (key, val) in map.iter_mut() {
            let mut k = key.clone();
            fk(&mut k, v);
            fv(val, v);
        }
    }
}

/// Visit a `BTreeSet<K>`.
pub fn snap_btree_set<K: Ord + Clone + Default>(
    v: &mut dyn Visitor,
    set: &mut BTreeSet<K>,
    mut fk: impl FnMut(&mut K, &mut dyn Visitor),
) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        if !v.check_remaining(len) {
            return;
        }
        set.clear();
        for _ in 0..len {
            let mut key = K::default();
            fk(&mut key, v);
            if v.failed() {
                return;
            }
            set.insert(key);
        }
    } else {
        let mut len = u32::try_from(set.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        for key in set.iter() {
            let mut k = key.clone();
            fk(&mut k, v);
        }
    }
}

/// Fast bulk visitor for `Vec<u16>`.
pub fn snap_u16_vec(v: &mut dyn Visitor, vec: &mut Vec<u16>) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        let Some(byte_len) = len.checked_mul(2) else {
            v.mark_corrupt("u16 vec length");
            return;
        };
        if !v.check_remaining(byte_len) {
            return;
        }
        vec.resize(len, 0);
        if cfg!(target_endian = "little") {
            // Safety: `vec` is initialized with `len` `u16`s = `byte_len` bytes.
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.u16(item);
            }
        }
    } else {
        let mut len = u32::try_from(vec.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        if cfg!(target_endian = "little") {
            let byte_len = vec.len() * 2;
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.u16(item);
            }
        }
    }
}

/// Fast bulk visitor for `Vec<i16>`.
pub fn snap_i16_vec(v: &mut dyn Visitor, vec: &mut Vec<i16>) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        let Some(byte_len) = len.checked_mul(2) else {
            v.mark_corrupt("i16 vec length");
            return;
        };
        if !v.check_remaining(byte_len) {
            return;
        }
        vec.resize(len, 0);
        if cfg!(target_endian = "little") {
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.i16(item);
            }
        }
    } else {
        let mut len = u32::try_from(vec.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        if cfg!(target_endian = "little") {
            let byte_len = vec.len() * 2;
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.i16(item);
            }
        }
    }
}

/// Fast bulk visitor for `Vec<u32>`.
pub fn snap_u32_vec(v: &mut dyn Visitor, vec: &mut Vec<u32>) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        let Some(byte_len) = len.checked_mul(4) else {
            v.mark_corrupt("u32 vec length");
            return;
        };
        if !v.check_remaining(byte_len) {
            return;
        }
        vec.resize(len, 0);
        if cfg!(target_endian = "little") {
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.u32(item);
            }
        }
    } else {
        let mut len = u32::try_from(vec.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        if cfg!(target_endian = "little") {
            let byte_len = vec.len() * 4;
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.u32(item);
            }
        }
    }
}

/// Fast bulk visitor for `Vec<f32>` (exact IEEE-754 bits).
pub fn snap_f32_vec(v: &mut dyn Visitor, vec: &mut Vec<f32>) {
    if v.is_reading() {
        let mut len = 0u32;
        v.u32(&mut len);
        let len = len as usize;
        let Some(byte_len) = len.checked_mul(4) else {
            v.mark_corrupt("f32 vec length");
            return;
        };
        if !v.check_remaining(byte_len) {
            return;
        }
        vec.resize(len, 0.0);
        if cfg!(target_endian = "little") {
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.f32(item);
            }
        }
    } else {
        let mut len = u32::try_from(vec.len()).unwrap_or(u32::MAX);
        v.u32(&mut len);
        if cfg!(target_endian = "little") {
            let byte_len = vec.len() * 4;
            let raw =
                unsafe { std::slice::from_raw_parts_mut(vec.as_mut_ptr().cast::<u8>(), byte_len) };
            v.bytes(raw);
        } else {
            for item in vec.iter_mut() {
                v.f32(item);
            }
        }
    }
}

/// Visit a `&'static str` resolved through a fixed static lookup table.
pub fn snap_static_str(
    v: &mut dyn Visitor,
    val: &mut &'static str,
    what: &'static str,
    resolve: fn(&str) -> Option<&'static str>,
) {
    if v.is_reading() {
        let mut s = String::new();
        v.string(&mut s);
        if v.failed() {
            return;
        }
        match resolve(&s) {
            Some(found) => *val = found,
            None => v.mark_corrupt(what),
        }
    } else {
        let mut s = (*val).to_owned();
        v.string(&mut s);
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
                Ok(bytes) => crate::hash::sha256_hex(&bytes),
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
            tsc,
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
        v.u64(tsc);
    }
}

impl Snap for crate::raster::PixelFormat {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { bits, masks } = self;
        v.u32(bits);
        for m in masks {
            v.u32(m);
        }
    }
}

impl Snap for crate::host::Button {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let mut tag = match self {
            Self::Left => 0u8,
            Self::Right => 1u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => *self = Self::Left,
                1 => *self = Self::Right,
                _ => v.mark_corrupt("Button tag"),
            }
        }
    }
}

impl Snap for crate::host::Input {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let mut tag = match self {
            Self::Move { .. } => 0u8,
            Self::Down(_) => 1u8,
            Self::Up(_) => 2u8,
            Self::Key { .. } => 3u8,
            Self::Close => 4u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => {
                    let (mut x, mut y) = (0i32, 0i32);
                    v.i32(&mut x);
                    v.i32(&mut y);
                    *self = Self::Move { x, y };
                }
                1 => {
                    let mut b = crate::host::Button::Left;
                    b.snap(v);
                    *self = Self::Down(b);
                }
                2 => {
                    let mut b = crate::host::Button::Left;
                    b.snap(v);
                    *self = Self::Up(b);
                }
                3 => {
                    let (mut code, mut down) = (0u32, false);
                    v.u32(&mut code);
                    v.bool(&mut down);
                    *self = Self::Key { code, down };
                }
                4 => *self = Self::Close,
                _ => v.mark_corrupt("Input tag"),
            }
        } else {
            match self {
                Self::Move { x, y } => {
                    v.i32(x);
                    v.i32(y);
                }
                Self::Down(b) | Self::Up(b) => b.snap(v),
                Self::Key { code, down } => {
                    v.u32(code);
                    v.bool(down);
                }
                Self::Close => {}
            }
        }
    }
}

impl Snap for crate::host::Cursor {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            width,
            height,
            hot,
            pixels,
        } = self;
        v.u32(width);
        v.u32(height);
        v.u32(&mut hot.0);
        v.u32(&mut hot.1);
        v.byte_vec(pixels);
    }
}

impl Snap for crate::pe::Import {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { dll, name, iat } = self;
        v.string(dll);
        v.string(name);
        v.u32(iat);
    }
}

impl Snap for crate::pe::Exports {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            by_name,
            by_ordinal,
            forwards,
            forwards_by_ordinal,
        } = self;
        snap_btree_map(v, by_name, |k, v| v.string(k), |val, v| v.u32(val));
        snap_btree_map(v, by_ordinal, |k, v| v.u16(k), |val, v| v.u32(val));
        snap_btree_map(v, forwards, |k, v| v.string(k), |val, v| v.string(val));
        snap_btree_map(
            v,
            forwards_by_ordinal,
            |k, v| v.u16(k),
            |val, v| v.string(val),
        );
    }
}

impl Snap for crate::avi::Header {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { flags, streams } = self;
        v.u32(flags);
        v.u32(streams);
    }
}

impl Snap for crate::avi::Stream {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            kind,
            handler,
            flags,
            priority,
            language,
            initial_frames,
            scale,
            rate,
            start,
            length,
            suggested_buffer,
            quality,
            sample_size,
            frame,
            name,
            format,
            chunks,
        } = self;
        v.bytes(kind);
        v.bytes(handler);
        v.u32(flags);
        v.u16(priority);
        v.u16(language);
        v.u32(initial_frames);
        v.u32(scale);
        v.u32(rate);
        v.u32(start);
        v.u32(length);
        v.u32(suggested_buffer);
        v.u32(quality);
        v.u32(sample_size);
        for f in frame {
            v.i16(f);
        }
        v.byte_vec(name);
        v.byte_vec(format);
        snap_vec(v, chunks, |(off, len), v| {
            v.usize(off);
            v.usize(len);
        });
    }
}

impl Snap for crate::avi::Avi {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { header, streams } = self;
        header.snap(v);
        snap_vec(v, streams, |s, v| s.snap(v));
    }
}

impl Snap for crate::iv50::Picture {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            width,
            height,
            y,
            u,
            v: vp,
        } = self;
        v.usize(width);
        v.usize(height);
        v.byte_vec(y);
        v.byte_vec(u);
        v.byte_vec(vp);
    }
}
