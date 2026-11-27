//! What a file says its version is (#60, #394).
//!
//! When the named file is a 32-bit PE image present in the guest tree with an
//! `RT_VERSION` (`VS_VERSION_INFO`) resource, `GetFileVersionInfoSizeA`,
//! `GetFileVersionInfoA` and `VerQueryValueA` answer directly from the image's
//! own resource — supporting `"\\"`, `"\\VarFileInfo\\Translation"`, and
//! `"\\StringFileInfo\\<langcp>\\<key>"` (case-insensitively, as Windows does).
//!
//! For system DLLs not present in the guest tree (such as Alpha Team probing
//! DirectX DLLs for DirectX 7), the declared machine answers with the declared
//! DirectX 7 `VS_FIXEDFILEINFO` (`4.07.00.0700`, #57).

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::pe::u16_at;

pub const SHIMS: &[Shim] = &[
    stdcall("VERSION", "GetFileVersionInfoSizeA", 2, info_size),
    stdcall("VERSION", "GetFileVersionInfoA", 4, info),
    stdcall("VERSION", "VerQueryValueA", 4, query),
];

/// Fallback block size when the queried file is a system library not in the
/// guest tree: one `VS_FIXEDFILEINFO` and nothing else.
const FALLBACK_BLOCK: u32 = 52;

/// `VS_FFI_SIGNATURE` (`0xFEEF04BD`) at offset 0 of a bare `VS_FIXEDFILEINFO`.
const VS_FFI_SIGNATURE: u32 = 0xfeef_04bd;

/// DirectX 7, as its own files number themselves: 4.07.00.0700. A declared
/// value — the declared machine has DirectX 7 because that is what the
/// roster's discs carry and ask for (#57).
const VERSION_MS: u32 = 0x0004_0007;
const VERSION_LS: u32 = 0x0000_02bc;

struct VersionNode<'a> {
    key: String,
    value_offset: usize,
    value_len: usize,
    is_text: bool,
    children_offset: usize,
    end: usize,
    raw: &'a [u8],
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Parse one `VS_VERSIONINFO` / `StringFileInfo` / `StringTable` / `String` /
/// `VarFileInfo` / `Var` node starting at `offset` within `raw[..limit]`.
fn parse_node(raw: &[u8], offset: usize, limit: usize) -> Option<VersionNode<'_>> {
    if offset + 6 > limit || limit > raw.len() {
        return None;
    }
    let total_len = usize::from(u16_at(raw, offset)?);
    if total_len < 6 {
        return None;
    }
    let end = offset.checked_add(total_len)?.min(limit);
    let value_len = usize::from(u16_at(raw, offset + 2)?);
    let is_text = u16_at(raw, offset + 4)? == 1;

    let mut pos = offset + 6;
    let mut units = Vec::new();
    while pos + 2 <= end {
        let u = u16_at(raw, pos)?;
        pos += 2;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    let key = String::from_utf16_lossy(&units);
    let value_offset = align4(pos).min(end);
    let avail = end.saturating_sub(value_offset);
    let value_bytes = if is_text {
        value_len.saturating_mul(2).min(avail)
    } else {
        value_len.min(avail)
    };
    let children_offset = align4(value_offset.saturating_add(value_bytes)).min(end);
    Some(VersionNode {
        key,
        value_offset,
        value_len: if is_text {
            value_bytes / 2
        } else {
            value_bytes
        },
        is_text,
        children_offset,
        end,
        raw,
    })
}

impl<'a> VersionNode<'a> {
    fn children(&self) -> Vec<VersionNode<'a>> {
        let mut out = Vec::new();
        let mut pos = self.children_offset;
        while pos + 6 <= self.end {
            let Some(child) = parse_node(self.raw, pos, self.end) else {
                break;
            };
            let next = align4(child.end);
            out.push(child);
            if next <= pos {
                break;
            }
            pos = next;
        }
        out
    }

    fn text_value_ansi(&self) -> Vec<u8> {
        let byte_len = self
            .value_len
            .saturating_mul(2)
            .min(self.end.saturating_sub(self.value_offset));
        let slice = &self.raw[self.value_offset..self.value_offset + byte_len];
        let mut out = Vec::with_capacity(self.value_len + 1);
        for chunk in slice.as_chunks::<2>().0 {
            let u = u16::from_le_bytes(*chunk);
            if u == 0 {
                break;
            }
            out.push(if u <= 0xff { u as u8 } else { b'?' });
        }
        out.push(0);
        out
    }
}

/// Build the `GetFileVersionInfoA` buffer for a raw PE `RT_VERSION` resource:
/// `rsrc[..root.end]` padded to `align4(root.end)`, immediately followed by
/// NUL-terminated ANSI copies of every `StringFileInfo` entry in traversal order.
fn build_ansi_version_block(rsrc: &[u8]) -> Option<Vec<u8>> {
    let root = parse_node(rsrc, 0, rsrc.len())?;
    let mut ansi_tail = Vec::new();
    for section in root.children() {
        if section.key.eq_ignore_ascii_case("StringFileInfo") {
            for table in section.children() {
                for entry in table.children() {
                    ansi_tail.extend_from_slice(&entry.text_value_ansi());
                }
            }
        }
    }
    let aligned_raw = align4(root.end);
    let mut block = Vec::with_capacity(aligned_raw + ansi_tail.len());
    block.extend_from_slice(&rsrc[..root.end]);
    block.resize(aligned_raw, 0);
    block.extend_from_slice(&ansi_tail);
    Some(block)
}

/// Resolve a `VerQueryValueA` sub-block query against `raw_root` (bytes
/// `0..wLength` of a block built by [`build_ansi_version_block`]). Returns
/// `(offset_in_block, length)` where `offset_in_block` may lie in the ANSI tail
/// starting at `align4(root.end)`.
fn query_version_root(raw_root: &[u8], sub: &str) -> Option<(usize, usize)> {
    let root = parse_node(raw_root, 0, raw_root.len())?;
    if sub == "\\" {
        if root.value_len == 0 || root.is_text {
            return None;
        }
        return Some((root.value_offset, root.value_len));
    }

    let trimmed = sub.strip_prefix('\\')?;
    let parts: Vec<&str> = trimmed.split('\\').collect();

    if parts.len() == 2 && parts[0].eq_ignore_ascii_case("VarFileInfo") {
        for section in root.children() {
            if section.key.eq_ignore_ascii_case("VarFileInfo") {
                for var in section.children() {
                    if var.key.eq_ignore_ascii_case(parts[1]) {
                        return Some((var.value_offset, var.value_len));
                    }
                }
            }
        }
        return None;
    }

    if parts.len() == 3 && parts[0].eq_ignore_ascii_case("StringFileInfo") {
        let mut ansi_pos = align4(root.end);
        for section in root.children() {
            if section.key.eq_ignore_ascii_case("StringFileInfo") {
                for table in section.children() {
                    let table_matches = table.key.eq_ignore_ascii_case(parts[1]);
                    for entry in table.children() {
                        let ansi_len = entry.text_value_ansi().len();
                        if table_matches && entry.key.eq_ignore_ascii_case(parts[2]) {
                            return Some((ansi_pos, ansi_len));
                        }
                        ansi_pos = ansi_pos.saturating_add(ansi_len);
                    }
                }
            }
        }
    }
    None
}

fn fallback_dx7_block() -> Vec<u8> {
    let mut block = vec![0u8; FALLBACK_BLOCK as usize];
    for (off, val) in [
        (0, VS_FFI_SIGNATURE),
        (4, 0x0001_0000),
        (8, VERSION_MS),
        (12, VERSION_LS),
        (16, VERSION_MS),
        (20, VERSION_LS),
    ] {
        block[off..off + 4].copy_from_slice(&val.to_le_bytes());
    }
    block
}

fn version_block(ctx: &mut Ctx<'_>, name: &str) -> Vec<u8> {
    ctx.open_guest_file_or_install(name)
        .and_then(|(_, bytes)| {
            let rsrc = crate::pe::version_resource(&bytes)?;
            build_ansi_version_block(rsrc)
        })
        .unwrap_or_else(fallback_dx7_block)
}

fn info_size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetFileVersionInfoSizeA(lptstrFilename, lpdwHandle).
    if args[1] != 0 {
        ctx.mem.write_u32(args[1], 0).map_err(Stop::Fault)?;
    }
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(0);
    }
    Ok(u32::try_from(version_block(ctx, &name).len()).unwrap_or(0))
}

fn info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetFileVersionInfoA(name, handle, length, data).
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(0);
    }
    let block = version_block(ctx, &name);
    let need = u32::try_from(block.len()).unwrap_or(u32::MAX);
    if args[2] < need {
        // ERROR_INSUFFICIENT_BUFFER.
        ctx.state.last_error = 122;
        return Ok(0);
    }
    ctx.mem.write(args[3], &block).map_err(Stop::Fault)?;
    Ok(1)
}

fn query(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // VerQueryValueA(block, what, out, out length).
    let block_at = args[0];
    let what = ctx.string(args[1]).map_err(Stop::Fault)?;

    let head = ctx.mem.read_u32(block_at).map_err(Stop::Fault)?;
    let (off, len) = if head == VS_FFI_SIGNATURE {
        if what != "\\" {
            ctx.log.line(&format!("version has no {what}"));
            return Ok(0);
        }
        (0usize, FALLBACK_BLOCK as usize)
    } else {
        let raw_len = usize::from(head as u16);
        if raw_len < 6 {
            ctx.log.line(&format!("version block invalid for {what}"));
            return Ok(0);
        }
        let raw_bytes = ctx.mem.slice(block_at, raw_len).map_err(Stop::Fault)?;
        let Some((off, len)) = query_version_root(raw_bytes, &what) else {
            ctx.log.line(&format!("version has no {what}"));
            return Ok(0);
        };
        // Verify the answered range lies inside mapped guest memory.
        ctx.mem
            .slice(block_at.wrapping_add(off as u32), len)
            .map_err(Stop::Fault)?;
        (off, len)
    };

    ctx.mem
        .write_u32(args[2], block_at.wrapping_add(off as u32))
        .map_err(Stop::Fault)?;
    if args[3] != 0 {
        ctx.mem
            .write_u32(args[3], u32::try_from(len).unwrap_or(0))
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::tests::with_test_ctx;

    fn utf16z(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn make_node(
        key: &str,
        w_type: u16,
        value: &[u8],
        value_len_units: u16,
        children: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 6];
        buf.extend_from_slice(&utf16z(key));
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
        buf.extend_from_slice(value);
        for child in children {
            while !buf.len().is_multiple_of(4) {
                buf.push(0);
            }
            buf.extend_from_slice(child);
        }
        let len = u16::try_from(buf.len()).unwrap();
        buf[0..2].copy_from_slice(&len.to_le_bytes());
        buf[2..4].copy_from_slice(&value_len_units.to_le_bytes());
        buf[4..6].copy_from_slice(&w_type.to_le_bytes());
        buf
    }

    fn sample_version_rsrc() -> Vec<u8> {
        let pv = utf16z("V.0.229");
        let pv_node = make_node("ProductVersion", 1, &pv, (pv.len() / 2) as u16, &[]);
        let cn = utf16z("Krisalis Software Ltd.");
        let cn_node = make_node("CompanyName", 1, &cn, (cn.len() / 2) as u16, &[]);
        let table = make_node("080904b0", 1, &[], 0, &[cn_node, pv_node]);
        let sfi = make_node("StringFileInfo", 1, &[], 0, &[table]);
        let trans = make_node("Translation", 0, &[0x09, 0x08, 0xb0, 0x04], 4, &[]);
        let vfi = make_node("VarFileInfo", 1, &[], 0, &[trans]);
        let mut ffi = [0u8; 52];
        ffi[0..4].copy_from_slice(&VS_FFI_SIGNATURE.to_le_bytes());
        ffi[8..12].copy_from_slice(&0x0000_0002u32.to_le_bytes());
        ffi[12..16].copy_from_slice(&0x0002_0009u32.to_le_bytes());
        make_node("VS_VERSION_INFO", 0, &ffi, 52, &[sfi, vfi])
    }

    fn sample_pe_with_rsrc(rsrc: &[u8]) -> Vec<u8> {
        let data_off = 0x280usize;
        let sec_size = (data_off - 0x200 + rsrc.len()).next_multiple_of(0x200);
        let mut bytes = vec![0u8; 0x200 + sec_size];
        let put = |b: &mut Vec<u8>, at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        put(&mut bytes, 0, b"MZ");
        put(&mut bytes, 0x3c, &0x40u32.to_le_bytes());
        put(&mut bytes, 0x40, b"PE\0\0");
        put(&mut bytes, 0x44, &0x014cu16.to_le_bytes());
        put(&mut bytes, 0x46, &1u16.to_le_bytes());
        put(&mut bytes, 0x54, &0xe0u16.to_le_bytes());
        put(&mut bytes, 0x58, &0x010bu16.to_le_bytes());
        put(&mut bytes, 0x58 + 92, &16u32.to_le_bytes());
        put(&mut bytes, 0x58 + 96 + 16, &0x2000u32.to_le_bytes());
        put(&mut bytes, 0x58 + 96 + 20, &(sec_size as u32).to_le_bytes());
        let sec = 0x58 + 0xe0;
        put(&mut bytes, sec, b".rsrc\0\0\0");
        put(&mut bytes, sec + 8, &(sec_size as u32).to_le_bytes());
        put(&mut bytes, sec + 12, &0x2000u32.to_le_bytes());
        put(&mut bytes, sec + 16, &(sec_size as u32).to_le_bytes());
        put(&mut bytes, sec + 20, &0x200u32.to_le_bytes());
        put(&mut bytes, 0x200 + 14, &1u16.to_le_bytes());
        put(&mut bytes, 0x200 + 16, &16u32.to_le_bytes());
        put(&mut bytes, 0x200 + 20, &(0x8000_0020u32).to_le_bytes());
        put(&mut bytes, 0x220 + 14, &1u16.to_le_bytes());
        put(&mut bytes, 0x220 + 16, &1u32.to_le_bytes());
        put(&mut bytes, 0x220 + 20, &(0x8000_0040u32).to_le_bytes());
        put(&mut bytes, 0x240 + 14, &1u16.to_le_bytes());
        put(&mut bytes, 0x240 + 16, &0x409u32.to_le_bytes());
        put(&mut bytes, 0x240 + 20, &0x60u32.to_le_bytes());
        put(&mut bytes, 0x260, &0x2080u32.to_le_bytes());
        put(&mut bytes, 0x260 + 4, &(rsrc.len() as u32).to_le_bytes());
        put(&mut bytes, data_off, rsrc);
        bytes
    }

    #[test]
    fn shims_read_pe_version_strings_case_insensitively_even_with_padded_rsrc_leaf() {
        let mut rsrc = sample_version_rsrc();
        // Trailing padding past root `wLength` must not misalign the ANSI tail.
        rsrc.extend_from_slice(&[0u8; 16]);
        let pe = sample_pe_with_rsrc(&rsrc);

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            ctx.state.current_directory = "C:\\GAME".to_owned();
            ctx.tree.declare("C:\\GAME\\APP.EXE", pe);

            let name = ctx.intern("APP.EXE").unwrap();
            let handle_out = ctx.state.heap.alloc(4);
            ctx.mem.write_u32(handle_out, 0xdead_beef).unwrap();
            let sz = info_size(ctx, &[name, handle_out]).unwrap();
            assert_eq!(ctx.mem.read_u32(handle_out).unwrap(), 0);
            assert!(sz > FALLBACK_BLOCK);

            let buf = ctx.state.heap.alloc(sz);
            assert_eq!(info(ctx, &[name, 0, sz - 1, buf]).unwrap(), 0);
            assert_eq!(ctx.state.last_error, 122);
            assert_eq!(info(ctx, &[name, 0, sz, buf]).unwrap(), 1);

            let out_ptr = ctx.state.heap.alloc(4);
            let out_len = ctx.state.heap.alloc(4);
            let sub = ctx
                .intern("\\StringFileInfo\\080904B0\\ProductVersion")
                .unwrap();
            assert_eq!(query(ctx, &[buf, sub, out_ptr, out_len]).unwrap(), 1);
            let ptr = ctx.mem.read_u32(out_ptr).unwrap();
            assert_eq!(ctx.string(ptr).unwrap(), "V.0.229");
            assert_eq!(ctx.mem.read_u32(out_len).unwrap(), 8);

            let trans = ctx.intern("\\VarFileInfo\\Translation").unwrap();
            assert_eq!(query(ctx, &[buf, trans, out_ptr, out_len]).unwrap(), 1);
            assert_eq!(ctx.mem.read_u32(out_len).unwrap(), 4);

            // System DLL absent from Tree falls back to 52-byte DirectX 7 block.
            let sys = ctx.intern("dsound.dll").unwrap();
            assert_eq!(info_size(ctx, &[sys, 0]).unwrap(), FALLBACK_BLOCK);
        });
    }
}
